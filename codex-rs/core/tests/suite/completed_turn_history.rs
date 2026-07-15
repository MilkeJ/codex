use anyhow::Result;
use codex_core::CodexThread;
use codex_core::ForkSnapshot;
use codex_core::NewThread;
use codex_core::config::TokenBudgetConfig;
use codex_features::Feature;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_completed_with_tokens;
use core_test_support::responses::ev_function_call;
use core_test_support::responses::ev_reasoning_item;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use std::sync::Arc;

const DUMMY_FUNCTION_NAME: &str = "test_tool";

async fn submit_thread(thread: &Arc<CodexThread>, prompt: &str) -> Result<()> {
    thread
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: prompt.to_string(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
            additional_context: Default::default(),
            thread_settings: Default::default(),
        })
        .await?;
    wait_for_event(thread, |event| matches!(event, EventMsg::TurnComplete(_))).await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_turn_prunes_transient_items_and_rebases_usage() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let call_id = "completed-turn-call";
    let response_mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("resp-1"),
                ev_reasoning_item("reason-1", &["using a tool"], &["private detail"]),
                ev_function_call(call_id, DUMMY_FUNCTION_NAME, "{}"),
                ev_completed("resp-1"),
            ]),
            sse(vec![
                ev_response_created("resp-2"),
                ev_reasoning_item("reason-2", &["finishing"], &["more private detail"]),
                ev_assistant_message("message-1", "first turn complete"),
                ev_completed_with_tokens("resp-2", /*total_tokens*/ 500_000),
            ]),
            sse(vec![
                ev_response_created("resp-3"),
                ev_assistant_message("message-2", "second turn complete"),
                ev_completed("resp-3"),
            ]),
        ],
    )
    .await;

    let mut builder = test_codex().with_model("gpt-5.4").with_config(|config| {
        config.model_auto_compact_token_limit = Some(200_000);
    });
    let test = builder.build_with_auto_env(&server).await?;
    test.submit_turn("first prompt").await?;
    test.submit_turn("second prompt").await?;

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 3);

    let tool_follow_up = &requests[1];
    assert_eq!(tool_follow_up.inputs_of_type("reasoning").len(), 1);
    assert_eq!(tool_follow_up.inputs_of_type("function_call").len(), 1);
    assert_eq!(
        tool_follow_up.function_call_output(call_id)["call_id"],
        call_id
    );

    let next_turn = &requests[2];
    for item_type in [
        "reasoning",
        "function_call",
        "function_call_output",
        "custom_tool_call",
        "custom_tool_call_output",
        "tool_search_call",
        "tool_search_output",
        "web_search_call",
        "image_generation_call",
        "local_shell_call",
    ] {
        assert!(
            next_turn.inputs_of_type(item_type).is_empty(),
            "completed {item_type} items should be pruned"
        );
    }
    let next_turn_body = next_turn.body_json().to_string();
    assert!(next_turn_body.contains("first prompt"));
    assert!(next_turn_body.contains("first turn complete"));
    assert!(next_turn_body.contains("second prompt"));

    let rollout_path = test.codex.rollout_path().expect("rollout path");
    let rollout = std::fs::read_to_string(rollout_path)?;
    for item_type in ["reasoning", "function_call", "function_call_output"] {
        assert!(rollout.contains(&format!(r#""type":"{item_type}""#)));
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_and_fork_prune_reconstructed_completed_items() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let response_mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("initial-response"),
                ev_reasoning_item("initial-reasoning", &["initial"], &["private"]),
                ev_assistant_message("initial-message", "initial answer"),
                ev_completed("initial-response"),
            ]),
            sse(vec![
                ev_response_created("resumed-response"),
                ev_assistant_message("resumed-message", "resumed answer"),
                ev_completed("resumed-response"),
            ]),
            sse(vec![
                ev_response_created("forked-response"),
                ev_assistant_message("forked-message", "forked answer"),
                ev_completed("forked-response"),
            ]),
        ],
    )
    .await;

    let mut builder = test_codex().with_model("gpt-5.4");
    let initial = builder.build_with_auto_env(&server).await?;
    initial.submit_turn("initial prompt").await?;
    let rollout_path = initial.codex.rollout_path().expect("rollout path");

    let resumed = builder
        .resume(&server, Arc::clone(&initial.home), rollout_path.clone())
        .await?;
    resumed.submit_turn("resumed prompt").await?;

    let NewThread { thread: forked, .. } = initial
        .thread_manager
        .fork_thread(
            ForkSnapshot::Interrupted,
            initial.config.clone(),
            rollout_path,
            /*thread_source*/ None,
            /*parent_trace*/ None,
        )
        .await?;
    submit_thread(&forked, "forked prompt").await?;

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 3);
    for (request, prompt) in [
        (&requests[1], "resumed prompt"),
        (&requests[2], "forked prompt"),
    ] {
        assert!(request.inputs_of_type("reasoning").is_empty());
        let body = request.body_json().to_string();
        assert!(body.contains("initial prompt"));
        assert!(body.contains("initial answer"));
        assert!(body.contains(prompt));
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn responses_lite_preserves_active_reasoning_and_prunes_completed_items() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let call_id = "lite-completed-turn-call";
    let response_mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("lite-response-1"),
                ev_reasoning_item("lite-reasoning", &["using a tool"], &["active detail"]),
                ev_function_call(call_id, DUMMY_FUNCTION_NAME, "{}"),
                ev_completed("lite-response-1"),
            ]),
            sse(vec![
                ev_response_created("lite-response-2"),
                ev_assistant_message("lite-message-1", "lite first answer"),
                ev_completed("lite-response-2"),
            ]),
            sse(vec![
                ev_response_created("lite-response-3"),
                ev_assistant_message("lite-message-2", "lite second answer"),
                ev_completed("lite-response-3"),
            ]),
        ],
    )
    .await;

    let mut builder = test_codex().with_model_info_override("gpt-5.4", |model_info| {
        model_info.use_responses_lite = true;
    });
    let test = builder.build_with_auto_env(&server).await?;
    test.submit_turn("lite first prompt").await?;
    test.submit_turn("lite second prompt").await?;

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 3);
    for request in &requests {
        assert_eq!(request.body_json()["reasoning"]["context"], "all_turns");
    }

    let tool_follow_up = &requests[1];
    assert_eq!(tool_follow_up.inputs_of_type("reasoning").len(), 1);
    assert_eq!(tool_follow_up.inputs_of_type("function_call").len(), 1);
    assert_eq!(
        tool_follow_up.function_call_output(call_id)["call_id"],
        call_id
    );

    let next_turn = &requests[2];
    for item_type in ["reasoning", "function_call", "function_call_output"] {
        assert!(
            next_turn.inputs_of_type(item_type).is_empty(),
            "completed {item_type} items should be pruned"
        );
    }
    let next_turn_body = next_turn.body_json().to_string();
    assert!(next_turn_body.contains("lite first prompt"));
    assert!(next_turn_body.contains("lite first answer"));
    assert!(next_turn_body.contains("lite second prompt"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn token_budget_fallback_survives_completed_turn_pruning() -> Result<()> {
    skip_if_no_network!(Ok(()));

    const FALLBACK_PROMPT: &str = "Save important state before rollover.";
    let server = start_mock_server().await;
    let call_id = "fallback-trigger";
    let response_mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("fallback-trigger-response"),
                ev_reasoning_item(
                    "fallback-trigger-reasoning",
                    &["checking budget"],
                    &["private detail"],
                ),
                ev_function_call(call_id, "get_context_remaining", "{}"),
                ev_completed_with_tokens("fallback-trigger-response", /*total_tokens*/ 9_500),
            ]),
            sse(vec![
                ev_response_created("fallback-terminal-response"),
                ev_assistant_message("fallback-terminal-message", "first turn complete"),
                ev_completed_with_tokens(
                    "fallback-terminal-response",
                    /*total_tokens*/ 10_000,
                ),
            ]),
            sse(vec![
                ev_response_created("post-prune-response"),
                ev_assistant_message("post-prune-message", "second turn complete"),
                ev_completed("post-prune-response"),
            ]),
        ],
    )
    .await;

    let mut builder = test_codex().with_model("gpt-5.4").with_config(|config| {
        config.model_context_window = Some(50_000);
        config.model_auto_compact_token_limit = Some(9_000);
        config.token_budget = Some(TokenBudgetConfig {
            auto_compact_fallback_prompt: Some(FALLBACK_PROMPT.to_string()),
            auto_compact_fallback_buffer_tokens: Some(4_000),
            ..TokenBudgetConfig::default()
        });
        config
            .features
            .enable(Feature::TokenBudget)
            .expect("test config should allow token budget");
    });
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("trigger the fallback").await?;
    test.submit_turn("continue after completed-turn pruning")
        .await?;

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 3);
    let post_prune_request = &requests[2];
    assert!(
        post_prune_request
            .message_input_texts("developer")
            .iter()
            .any(|text| text == FALLBACK_PROMPT),
        "the fallback developer message should remain model-visible after pruning"
    );
    for item_type in ["reasoning", "function_call", "function_call_output"] {
        assert!(
            post_prune_request.inputs_of_type(item_type).is_empty(),
            "completed {item_type} machinery should be pruned"
        );
    }
    assert!(
        post_prune_request
            .body_json()
            .to_string()
            .contains("continue after completed-turn pruning")
    );
    Ok(())
}
