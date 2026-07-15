use super::support::*;
use anyhow::Result;
use codex_core::compact::SUMMARIZATION_PROMPT;
use codex_features::Feature;
use codex_login::CodexAuth;
use codex_model_provider_info::built_in_model_providers;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_completed_with_tokens;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_compact_json_once;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use pretty_assertions::assert_eq;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_compaction_trim_scans_past_non_output_items() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let consumed_call_ids = ["trim-scan-1", "trim-scan-2", "trim-scan-3", "trim-scan-4"];
    let current_call_id = "trim-scan-current";
    let _response_mock = mount_sse_sequence(
        &server,
        vec![
            tool_call_response(
                "trim-scan-tools",
                &consumed_call_ids,
                /*total_tokens*/ 10_000,
            ),
            tool_call_response(
                "trim-scan-trigger",
                &[current_call_id],
                AUTO_COMPACT_LIMIT + 30_000,
            ),
        ],
    )
    .await;
    let compact_mock = mount_compact_json_once(&server, compact_response()).await;
    let mut builder = test_builder(AUTO_COMPACT_LIMIT).with_config(|config| {
        config.model_context_window = Some(25_000);
    });
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("trim several separated outputs before remote compaction")
        .await?;

    let compact_request = compact_mock.single_request();
    let truncated_output_count = consumed_call_ids
        .iter()
        .chain(std::iter::once(&current_call_id))
        .filter(|call_id| {
            compact_request.function_call_output_text(call_id)
                == Some(CONTEXT_WINDOW_TRUNCATED_OUTPUT_MESSAGE.to_string())
        })
        .count();
    assert!(
        truncated_output_count >= 2,
        "expected trimming to continue past intervening response items"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_compaction_uses_reclaimed_view_after_second_threshold() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let consumed_call_ids = ["restore-1", "restore-2", "restore-3", "restore-4"];
    let response_mock = mount_sse_sequence(
        &server,
        vec![
            tool_call_response(
                "restore-tools",
                &consumed_call_ids,
                /*total_tokens*/ 10_000,
            ),
            tool_call_response("restore-trigger", &["restore-current"], AUTO_COMPACT_LIMIT),
            tool_call_response(
                "restore-second-threshold",
                &["restore-second-threshold-output"],
                AUTO_COMPACT_LIMIT,
            ),
            sse(vec![
                ev_response_created("restore-local-compact"),
                ev_assistant_message("restore-local-summary", "local reclaimed summary"),
                ev_completed_with_tokens("restore-local-compact", /*total_tokens*/ 1_000),
            ]),
            sse(vec![
                ev_response_created("restore-finished"),
                ev_assistant_message("restore-message", "finished after restored compaction"),
                ev_completed("restore-finished"),
            ]),
        ],
    )
    .await;
    let mut model_provider = built_in_model_providers(/*openai_base_url*/ None)["openai"].clone();
    model_provider.name = "Local compaction test".into();
    model_provider.base_url = Some(format!("{}/v1", server.uri()));
    model_provider.supports_websockets = false;
    let mut builder = test_builder(AUTO_COMPACT_LIMIT)
        .with_auth(CodexAuth::from_api_key("Test API Key"))
        .with_config(move |config| {
            config.model_provider = model_provider;
        });
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("compact after reclaimed context reaches the threshold again")
        .await?;

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 5);
    for call_id in consumed_call_ids {
        assert_eq!(
            requests[2].function_call_output_text(call_id),
            Some(RECLAIMED_TOOL_OUTPUT_MESSAGE.to_string())
        );
    }

    let compact_request = &requests[3];
    assert!(compact_request.body_contains_text(SUMMARIZATION_PROMPT));
    for call_id in consumed_call_ids {
        assert_eq!(
            compact_request.function_call_output_text(call_id),
            Some(RECLAIMED_TOOL_OUTPUT_MESSAGE.to_string())
        );
    }
    assert_eq!(
        compact_request.function_call_output_text("restore-current"),
        Some(RECLAIMED_TOOL_OUTPUT_MESSAGE.to_string())
    );
    assert_original_output(compact_request, "restore-second-threshold-output");

    let rollout_path = test.codex.rollout_path().expect("rollout path");
    let rollout = std::fs::read_to_string(rollout_path)?;
    assert!(!rollout.contains(RECLAIMED_TOOL_OUTPUT_MESSAGE));
    assert!(rollout.contains("restore-second-threshold-output"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_v2_compacts_reclaimed_view_after_second_threshold() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let consumed_call_ids = [
        "v2-consumed-1",
        "v2-consumed-2",
        "v2-consumed-3",
        "v2-consumed-4",
    ];
    let current_call_id = "v2-current";
    let second_threshold_call_id = "v2-second-threshold-output";
    let response_mock = mount_sse_sequence(
        &server,
        vec![
            tool_call_response("v2-tools", &consumed_call_ids, /*total_tokens*/ 10_000),
            tool_call_response("v2-trigger", &[current_call_id], AUTO_COMPACT_LIMIT),
            tool_call_response(
                "v2-second-threshold",
                &[second_threshold_call_id],
                AUTO_COMPACT_LIMIT,
            ),
            compact_v2_response(),
            sse(vec![
                ev_response_created("v2-finished"),
                ev_assistant_message("v2-message", "finished after reclaimed v2 compaction"),
                ev_completed("v2-finished"),
            ]),
        ],
    )
    .await;
    let mut builder = test_builder(AUTO_COMPACT_LIMIT).with_config(|config| {
        let _ = config.features.enable(Feature::RemoteCompactionV2);
    });
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("compact the reclaimed view with remote compaction v2")
        .await?;

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 5);
    let compact_request = &requests[3];
    assert!(
        compact_request
            .body_json()
            .to_string()
            .contains("\"type\":\"compaction_trigger\"")
    );
    for call_id in consumed_call_ids
        .iter()
        .chain(std::iter::once(&current_call_id))
    {
        assert_eq!(
            compact_request.function_call_output_text(call_id),
            Some(RECLAIMED_TOOL_OUTPUT_MESSAGE.to_string())
        );
    }
    assert_original_output(compact_request, second_threshold_call_id);
    Ok(())
}
