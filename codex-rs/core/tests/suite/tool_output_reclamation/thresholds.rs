use super::support::*;
use anyhow::Result;
use codex_protocol::config_types::AutoCompactTokenLimitScope;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_compact_json_once;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use pretty_assertions::assert_eq;
#[derive(Clone, Copy)]
enum FullContextHardCapOutcome {
    Reclaim,
    Compact,
}

async fn run_full_context_hard_cap_case(outcome: FullContextHardCapOutcome) -> Result<()> {
    let server = start_mock_server().await;
    let all_consumed_call_ids = [
        "hard-cap-consumed-1",
        "hard-cap-consumed-2",
        "hard-cap-consumed-3",
        "hard-cap-consumed-4",
    ];
    let consumed_call_ids = match outcome {
        // Two 36K-character outputs save about 18K estimated tokens: enough for 10% of the
        // 100K hard cap, but not 10% of the larger 200K body-after-prefix limit.
        FullContextHardCapOutcome::Reclaim => &all_consumed_call_ids[..2],
        FullContextHardCapOutcome::Compact => &all_consumed_call_ids[..],
    };
    let trigger_tokens = match outcome {
        // The newest raw output takes active usage past 100K. Reclaiming the two older outputs
        // then leaves between 10K and 20K of projected hard-cap headroom.
        FullContextHardCapOutcome::Reclaim => 98_000,
        FullContextHardCapOutcome::Compact => 125_000,
    };
    let response_mock = mount_sse_sequence(
        &server,
        vec![
            tool_call_response(
                "hard-cap-tools",
                consumed_call_ids,
                /*total_tokens*/ 10_000,
            ),
            tool_call_response("hard-cap-trigger", &["hard-cap-current"], trigger_tokens),
            sse(vec![
                ev_response_created("hard-cap-finished"),
                ev_assistant_message("hard-cap-message", "finished after the hard cap"),
                ev_completed("hard-cap-finished"),
            ]),
        ],
    )
    .await;
    let compact_mock = mount_compact_json_once(&server, compact_response()).await;
    let mut builder = test_builder(/*auto_compact_limit*/ 200_000)
        .with_model_info_override("gpt-5.4", |model_info| {
            model_info.effective_context_window_percent = 100;
        })
        .with_config(|config| {
            config.model_context_window = Some(100_000);
            config.model_auto_compact_token_limit_scope =
                AutoCompactTokenLimitScope::BodyAfterPrefix;
        });
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("respect the full context hard cap")
        .await?;

    let expected_compaction_requests = match outcome {
        FullContextHardCapOutcome::Reclaim => 0,
        FullContextHardCapOutcome::Compact => 1,
    };
    assert_eq!(compact_mock.requests().len(), expected_compaction_requests);
    let requests = response_mock.requests();
    assert_eq!(requests.len(), 3);
    if matches!(outcome, FullContextHardCapOutcome::Reclaim) {
        for call_id in consumed_call_ids {
            assert_eq!(
                requests[2].function_call_output_text(call_id),
                Some(RECLAIMED_TOOL_OUTPUT_MESSAGE.to_string())
            );
        }
        assert_original_output(&requests[2], "hard-cap-current");
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reclamation_threshold_scales_with_auto_compact_limit() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let scaled_auto_compact_limit = 20_000;
    let response_mock = mount_sse_sequence(
        &server,
        vec![
            tool_call_response(
                "scaled-threshold-tool",
                &["scaled-threshold-consumed"],
                /*total_tokens*/ 10_000,
            ),
            tool_call_response(
                "scaled-threshold-trigger",
                &["scaled-threshold-current"],
                /*total_tokens*/ scaled_auto_compact_limit - 4_000,
            ),
            sse(vec![
                ev_response_created("scaled-threshold-finished"),
                ev_assistant_message("scaled-threshold-message", "finished after reclamation"),
                ev_completed("scaled-threshold-finished"),
            ]),
        ],
    )
    .await;
    let compact_mock = mount_compact_json_once(&server, compact_response()).await;
    let mut builder = test_builder(scaled_auto_compact_limit);
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("scale reclamation threshold with compaction limit")
        .await?;

    assert_eq!(compact_mock.requests().len(), 0);
    let requests = response_mock.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[2].function_call_output_text("scaled-threshold-consumed"),
        Some(RECLAIMED_TOOL_OUTPUT_MESSAGE.to_string())
    );
    assert_original_output(&requests[2], "scaled-threshold-current");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compacts_when_reclamation_savings_are_below_percentage_threshold() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let consumed_call_ids = ["small-savings-1"];
    let response_mock = mount_sse_sequence(
        &server,
        vec![
            tool_call_response(
                "small-savings-tools",
                &consumed_call_ids,
                /*total_tokens*/ 10_000,
            ),
            tool_call_response(
                "small-savings-trigger",
                &["small-savings-current"],
                AUTO_COMPACT_LIMIT,
            ),
            sse(vec![
                ev_response_created("small-savings-finished"),
                ev_assistant_message("small-savings-message", "finished after compaction"),
                ev_completed("small-savings-finished"),
            ]),
        ],
    )
    .await;
    let compact_mock = mount_compact_json_once(&server, compact_response()).await;
    let mut builder = test_builder(AUTO_COMPACT_LIMIT);
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("compact when too little output can be reclaimed")
        .await?;

    assert_eq!(compact_mock.requests().len(), 1);
    assert_eq!(response_mock.requests().len(), 3);
    for call_id in consumed_call_ids {
        assert_original_output(&compact_mock.single_request(), call_id);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compacts_when_reclamation_does_not_create_percentage_headroom() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let consumed_call_ids = [
        "low-headroom-1",
        "low-headroom-2",
        "low-headroom-3",
        "low-headroom-4",
    ];
    let response_mock = mount_sse_sequence(
        &server,
        vec![
            tool_call_response(
                "low-headroom-tools",
                &consumed_call_ids,
                /*total_tokens*/ 10_000,
            ),
            tool_call_response(
                "low-headroom-trigger",
                &["low-headroom-current"],
                AUTO_COMPACT_LIMIT + 30_000,
            ),
            sse(vec![
                ev_response_created("low-headroom-finished"),
                ev_assistant_message("low-headroom-message", "finished after compaction"),
                ev_completed("low-headroom-finished"),
            ]),
        ],
    )
    .await;
    let compact_mock = mount_compact_json_once(&server, compact_response()).await;
    let mut builder = test_builder(AUTO_COMPACT_LIMIT);
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("compact when reclamation leaves too little headroom")
        .await?;

    assert_eq!(compact_mock.requests().len(), 1);
    assert_eq!(response_mock.requests().len(), 3);
    for call_id in consumed_call_ids {
        assert_original_output(&compact_mock.single_request(), call_id);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_token_budget_reclamation_respects_full_context_hard_cap() -> Result<()> {
    skip_if_no_network!(Ok(()));

    run_full_context_hard_cap_case(FullContextHardCapOutcome::Reclaim).await?;
    run_full_context_hard_cap_case(FullContextHardCapOutcome::Compact).await?;
    Ok(())
}
