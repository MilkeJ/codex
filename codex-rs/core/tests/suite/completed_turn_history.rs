use anyhow::Result;
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
use pretty_assertions::assert_eq;

const DUMMY_FUNCTION_NAME: &str = "test_tool";

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
