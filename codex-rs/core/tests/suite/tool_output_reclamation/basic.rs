use super::support::*;
use anyhow::Result;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_compact_json_once;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::responses::start_websocket_server;
use core_test_support::skip_if_no_network;
use pretty_assertions::assert_eq;
use serde_json::json;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reclaims_consumed_outputs_and_avoids_compaction_when_turn_finishes() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let consumed_call_ids = [
        "consumed-1",
        "consumed-2",
        "consumed-3",
        "consumed-4",
        "consumed-5",
    ];
    let current_call_id = "current-output";
    let mut initial_response = tool_call_events(
        "initial-tool",
        &[consumed_call_ids[0]],
        /*total_tokens*/ 10_000,
    );
    initial_response.insert(
        0,
        json!({
            "type": "response.metadata",
            "headers": {"x-codex-turn-state": "reclaim-turn-state"}
        }),
    );
    let server = start_websocket_server(vec![vec![
        initial_response,
        tool_call_events(
            "second-tool",
            &[consumed_call_ids[1]],
            /*total_tokens*/ 20_000,
        ),
        tool_call_events(
            "third-tool",
            &[consumed_call_ids[2]],
            /*total_tokens*/ 30_000,
        ),
        tool_call_events(
            "fourth-tool",
            &[consumed_call_ids[3]],
            /*total_tokens*/ 40_000,
        ),
        tool_call_events(
            "fifth-tool",
            &[consumed_call_ids[4]],
            /*total_tokens*/ 50_000,
        ),
        tool_call_events("limit-trigger", &[current_call_id], AUTO_COMPACT_LIMIT),
        vec![
            ev_response_created("reclaimed-finished"),
            ev_assistant_message("reclaimed-message", "finished after reclaiming outputs"),
            ev_completed("reclaimed-finished"),
        ],
    ]])
    .await;
    let mut builder = test_builder(AUTO_COMPACT_LIMIT);
    let test = builder.build_with_websocket_server(&server).await?;

    test.submit_turn("finish after several large tool results")
        .await?;

    let requests = server.single_connection();
    assert_eq!(requests.len(), 7);
    let reclaimed_request = requests[6].body_json();
    assert_eq!(reclaimed_request.get("previous_response_id"), None);
    assert_eq!(
        reclaimed_request["client_metadata"]["x-codex-turn-state"],
        "reclaim-turn-state"
    );
    let reclaimed_input = reclaimed_request["input"]
        .as_array()
        .expect("reclaimed input");
    let consumed_outputs = consumed_call_ids
        .iter()
        .filter_map(|call_id| {
            reclaimed_input.iter().find_map(|item| {
                (item["type"] == "function_call_output" && item["call_id"] == **call_id)
                    .then(|| item["output"].as_str())
                    .flatten()
            })
        })
        .collect::<Vec<_>>();
    // The WebSocket fixture retains the responses after its synthetic connection root.
    assert!(consumed_outputs.len() >= 4);
    assert!(
        consumed_outputs
            .iter()
            .all(|output| *output == RECLAIMED_TOOL_OUTPUT_MESSAGE)
    );
    assert!(
        reclaimed_input
            .iter()
            .filter(|item| item["type"] == "reasoning")
            .count()
            >= 5
    );
    let current_output = reclaimed_input
        .iter()
        .find_map(|item| {
            (item["type"] == "function_call_output" && item["call_id"] == current_call_id)
                .then(|| item["output"].as_str())
                .flatten()
        })
        .expect("current output");
    assert!(current_output.len() >= LARGE_OUTPUT_CHARS);

    let rollout_path = test.codex.rollout_path().expect("rollout path");
    let rollout = std::fs::read_to_string(rollout_path)?;
    assert!(!rollout.contains(RECLAIMED_TOOL_OUTPUT_MESSAGE));
    assert!(
        consumed_call_ids
            .iter()
            .filter(|call_id| rollout.contains(**call_id))
            .count()
            >= 4
    );
    assert!(rollout.contains(current_call_id));
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reclaimed_outputs_remain_active_across_tool_follow_ups() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let consumed_call_ids = ["active-1", "active-2", "active-3", "active-4"];
    let response_mock = mount_sse_sequence(
        &server,
        vec![
            tool_call_response(
                "active-tools",
                &consumed_call_ids,
                /*total_tokens*/ 10_000,
            ),
            tool_call_response("active-trigger", &["active-current"], AUTO_COMPACT_LIMIT),
            tool_call_response(
                "active-follow-up-1",
                &["active-follow-up-output-1"],
                /*total_tokens*/ 50_000,
            ),
            tool_call_response(
                "active-follow-up-2",
                &["active-follow-up-output-2"],
                /*total_tokens*/ 60_000,
            ),
            sse(vec![
                ev_response_created("active-finished"),
                ev_assistant_message("active-message", "finished with reclaimed context"),
                ev_completed("active-finished"),
            ]),
        ],
    )
    .await;
    let compact_mock = mount_compact_json_once(&server, compact_response()).await;
    let mut builder = test_builder(AUTO_COMPACT_LIMIT);
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("continue across several tools after reclaiming")
        .await?;

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 5);
    for reclaimed_request in &requests[2..] {
        for call_id in consumed_call_ids {
            assert_eq!(
                reclaimed_request.function_call_output_text(call_id),
                Some(RECLAIMED_TOOL_OUTPUT_MESSAGE.to_string())
            );
        }
    }
    assert_original_output(&requests[2], "active-current");
    assert_original_output(&requests[3], "active-follow-up-output-1");
    assert_original_output(&requests[4], "active-follow-up-output-2");
    assert_eq!(compact_mock.requests().len(), 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_threshold_extends_reclamation_instead_of_compacting() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let first_consumed_call_ids = [
        "repeat-first-1",
        "repeat-first-2",
        "repeat-first-3",
        "repeat-first-4",
    ];
    let second_consumed_call_ids = [
        "repeat-second-1",
        "repeat-second-2",
        "repeat-second-3",
        "repeat-second-4",
    ];
    let first_current_call_id = "repeat-first-current";
    let second_current_call_id = "repeat-second-current";
    let response_mock = mount_sse_sequence(
        &server,
        vec![
            tool_call_response(
                "repeat-first-tools",
                &first_consumed_call_ids,
                /*total_tokens*/ 10_000,
            ),
            tool_call_response(
                "repeat-first-trigger",
                &[first_current_call_id],
                AUTO_COMPACT_LIMIT,
            ),
            tool_call_response(
                "repeat-second-tools",
                &second_consumed_call_ids,
                /*total_tokens*/ 50_000,
            ),
            tool_call_response(
                "repeat-second-trigger",
                &[second_current_call_id],
                AUTO_COMPACT_LIMIT,
            ),
            sse(vec![
                ev_response_created("repeat-finished"),
                ev_assistant_message("repeat-message", "finished after repeated reclamation"),
                ev_completed("repeat-finished"),
            ]),
        ],
    )
    .await;
    let compact_mock = mount_compact_json_once(&server, compact_response()).await;
    let mut builder = test_builder(AUTO_COMPACT_LIMIT);
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("reclaim another batch at the next threshold")
        .await?;

    assert_eq!(compact_mock.requests().len(), 0);
    let requests = response_mock.requests();
    assert_eq!(requests.len(), 5);
    let twice_reclaimed_request = &requests[4];
    for call_id in first_consumed_call_ids
        .iter()
        .chain(second_consumed_call_ids.iter())
        .chain(std::iter::once(&first_current_call_id))
    {
        assert_eq!(
            twice_reclaimed_request.function_call_output_text(call_id),
            Some(RECLAIMED_TOOL_OUTPUT_MESSAGE.to_string())
        );
    }
    assert_original_output(twice_reclaimed_request, second_current_call_id);
    Ok(())
}
