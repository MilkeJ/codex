use super::support::*;
use anyhow::Result;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
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
