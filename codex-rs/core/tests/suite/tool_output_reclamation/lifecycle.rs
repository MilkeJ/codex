use super::support::*;
use anyhow::Result;
use codex_features::Feature;
use codex_protocol::AgentPath;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_completed_with_tokens;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::ev_shell_command_call_with_args;
use core_test_support::responses::sse;
use core_test_support::skip_if_no_network;
use core_test_support::streaming_sse::StreamingSseChunk;
use core_test_support::streaming_sse::start_streaming_sse_server;
use core_test_support::test_codex::test_codex;
use core_test_support::test_codex::turn_permission_fields;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_terminal_input_invalidates_reclamation_without_resurrecting_output() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let consumed_call_ids = ["queued-consumed"];
    let initial_chunks = tool_call_events(
        "queued-tools",
        &consumed_call_ids,
        /*total_tokens*/ 10_000,
    )
    .into_iter()
    .map(|event| StreamingSseChunk {
        gate: None,
        body: sse(vec![event]),
    })
    .collect();
    let trigger_chunks = tool_call_events(
        "queued-trigger",
        &["queued-current"],
        /*total_tokens*/ 16_000,
    )
    .into_iter()
    .map(|event| StreamingSseChunk {
        gate: None,
        body: sse(vec![event]),
    })
    .collect();
    let (complete_terminal_tx, complete_terminal_rx) = tokio::sync::oneshot::channel();
    let terminal_chunks = vec![
        StreamingSseChunk {
            gate: None,
            body: sse(vec![ev_response_created("queued-terminal")]),
        },
        StreamingSseChunk {
            gate: None,
            body: sse(vec![ev_assistant_message(
                "queued-terminal-message",
                "first answer",
            )]),
        },
        StreamingSseChunk {
            gate: Some(complete_terminal_rx),
            body: sse(vec![ev_completed("queued-terminal")]),
        },
    ];
    let follow_up_chunks = vec![StreamingSseChunk {
        gate: None,
        body: sse(vec![
            ev_response_created("queued-follow-up"),
            ev_assistant_message("queued-follow-up-message", "second answer"),
            ev_completed("queued-follow-up"),
        ]),
    }];
    let (server, _completions) = start_streaming_sse_server(vec![
        initial_chunks,
        trigger_chunks,
        terminal_chunks,
        follow_up_chunks,
    ])
    .await;
    let mut builder = test_codex().with_model("gpt-5.4").with_config(|config| {
        let _ = config.features.disable(Feature::RemoteCompactionV2);
        config.model_context_window = Some(300_000);
        config.model_auto_compact_token_limit = Some(20_000);
        config.tool_output_token_limit = Some(10_000);
    });
    let test = builder.build_with_streaming_server(&server).await?;
    let (sandbox_policy, permission_profile) =
        turn_permission_fields(PermissionProfile::Disabled, test.config.cwd.as_path());

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "first prompt".to_string(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
            additional_context: Default::default(),
            thread_settings: codex_protocol::protocol::ThreadSettingsOverrides {
                approval_policy: Some(AskForApproval::Never),
                sandbox_policy: Some(sandbox_policy),
                permission_profile,
                ..Default::default()
            },
        })
        .await?;
    wait_for_event(
        &test.codex,
        |event| matches!(event, EventMsg::AgentMessage(message) if message.message == "first answer"),
    )
    .await;
    test.codex
        .steer_input(
            vec![UserInput::Text {
                text: "second prompt".to_string(),
                text_elements: Vec::new(),
            }],
            /*additional_context*/ Default::default(),
            /*expected_turn_id*/ None,
            /*client_user_message_id*/ None,
            /*responsesapi_client_metadata*/ None,
        )
        .await
        .expect("queue terminal input");
    let _ = complete_terminal_tx.send(());
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let requests = server.requests().await;
    assert_eq!(requests.len(), 4);
    let reclaimed_request: Value =
        serde_json::from_slice(&requests[2]).expect("parse reclaimed request");
    for call_id in consumed_call_ids {
        assert_eq!(
            reclaimed_request["input"]
                .as_array()
                .expect("reclaimed input")
                .iter()
                .find(|item| {
                    item["type"] == "function_call_output" && item["call_id"] == call_id
                })
                .and_then(|item| item["output"].as_str()),
            Some(RECLAIMED_TOOL_OUTPUT_MESSAGE)
        );
    }

    let queued_follow_up: Value =
        serde_json::from_slice(&requests[3]).expect("parse queued follow-up request");
    assert!(
        queued_follow_up["input"]
            .as_array()
            .expect("queued follow-up input")
            .iter()
            .all(|item| item["type"] != "function_call_output"),
        "the completed-turn boundary should remove raw and reclaimed tool outputs"
    );
    let queued_follow_up_body = queued_follow_up.to_string();
    assert!(!queued_follow_up_body.contains(RECLAIMED_TOOL_OUTPUT_MESSAGE));
    assert!(queued_follow_up_body.contains("first answer"));
    assert!(queued_follow_up_body.contains("second prompt"));

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_queue_only_mail_waits_for_next_turn_after_reclaimed_final_answer() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let consumed_call_ids = ["mail-consumed"];
    let initial_chunks = tool_call_events(
        "mail-tools",
        &consumed_call_ids,
        /*total_tokens*/ 10_000,
    )
    .into_iter()
    .map(|event| StreamingSseChunk {
        gate: None,
        body: sse(vec![event]),
    })
    .collect();
    let trigger_chunks = tool_call_events(
        "mail-trigger",
        &["mail-current"],
        /*total_tokens*/ 16_000,
    )
    .into_iter()
    .map(|event| StreamingSseChunk {
        gate: None,
        body: sse(vec![event]),
    })
    .collect();
    let (complete_terminal_tx, complete_terminal_rx) = tokio::sync::oneshot::channel();
    let terminal_chunks = vec![
        StreamingSseChunk {
            gate: None,
            body: sse(vec![ev_response_created("mail-terminal")]),
        },
        StreamingSseChunk {
            gate: None,
            body: sse(vec![json!({
                "type": "response.output_item.done",
                "item": {
                    "type": "message",
                    "role": "assistant",
                    "id": "mail-terminal-message",
                    "content": [{"type": "output_text", "text": "mail first answer"}],
                    "phase": "final_answer",
                }
            })]),
        },
        StreamingSseChunk {
            gate: Some(complete_terminal_rx),
            body: sse(vec![ev_completed("mail-terminal")]),
        },
    ];
    let next_turn_tool_args = if cfg!(windows) {
        json!({"command": "Write-Output next-turn-tool", "login": false})
    } else {
        json!({"command": "printf next-turn-tool", "login": false})
    };
    let next_turn_chunks = vec![StreamingSseChunk {
        gate: None,
        body: sse(vec![
            ev_response_created("mail-next-turn"),
            ev_shell_command_call_with_args("mail-next-tool", &next_turn_tool_args),
            ev_completed_with_tokens("mail-next-turn", /*total_tokens*/ 1_000),
        ]),
    }];
    let mail_follow_up_chunks = vec![StreamingSseChunk {
        gate: None,
        body: sse(vec![
            ev_response_created("mail-follow-up"),
            ev_assistant_message("mail-follow-up-message", "mail second answer"),
            ev_completed("mail-follow-up"),
        ]),
    }];
    let (server, _completions) = start_streaming_sse_server(vec![
        initial_chunks,
        trigger_chunks,
        terminal_chunks,
        next_turn_chunks,
        mail_follow_up_chunks,
    ])
    .await;
    let mut builder = test_codex().with_model("gpt-5.4").with_config(|config| {
        let _ = config.features.disable(Feature::RemoteCompactionV2);
        config.model_context_window = Some(300_000);
        config.model_auto_compact_token_limit = Some(20_000);
        config.tool_output_token_limit = Some(10_000);
    });
    let test = builder.build_with_streaming_server(&server).await?;
    let (sandbox_policy, permission_profile) =
        turn_permission_fields(PermissionProfile::Disabled, test.config.cwd.as_path());

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "mail first prompt".to_string(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
            additional_context: Default::default(),
            thread_settings: codex_protocol::protocol::ThreadSettingsOverrides {
                approval_policy: Some(AskForApproval::Never),
                sandbox_policy: Some(sandbox_policy),
                permission_profile,
                ..Default::default()
            },
        })
        .await?;
    wait_for_event(
        &test.codex,
        |event| matches!(event, EventMsg::AgentMessage(message) if message.message == "mail first answer"),
    )
    .await;

    test.codex
        .submit(Op::InterAgentCommunication {
            communication: InterAgentCommunication::new(
                AgentPath::try_from("/root/worker").expect("worker path should parse"),
                AgentPath::root(),
                Vec::new(),
                "late queued child update".to_string(),
                /*trigger_turn*/ false,
            ),
        })
        .await?;
    test.codex
        .submit(Op::RealtimeConversationListVoices)
        .await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::RealtimeConversationListVoicesResponse(_))
    })
    .await;
    let _ = complete_terminal_tx.send(());
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;

    let first_turn_requests = server.requests().await;
    assert_eq!(first_turn_requests.len(), 3);
    let reclaimed_request: Value =
        serde_json::from_slice(&first_turn_requests[2]).expect("parse reclaimed request");
    assert_eq!(
        reclaimed_request["input"]
            .as_array()
            .expect("reclaimed input")
            .iter()
            .find(|item| {
                item["type"] == "function_call_output" && item["call_id"] == consumed_call_ids[0]
            })
            .and_then(|item| item["output"].as_str()),
        Some(RECLAIMED_TOOL_OUTPUT_MESSAGE)
    );

    test.submit_turn("mail second prompt").await?;

    let requests = server.requests().await;
    assert_eq!(requests.len(), 5);
    let next_turn: Value = serde_json::from_slice(&requests[3]).expect("parse next-turn request");
    let next_turn_input = next_turn["input"].as_array().expect("next-turn input");
    assert!(
        next_turn_input
            .iter()
            .all(|item| item["type"] != "function_call_output"),
        "the next turn must not resurrect raw or reclaimed tool outputs"
    );
    assert!(
        next_turn_input
            .iter()
            .all(|item| item["type"] != "agent_message"),
        "fresh user input is intentionally sampled before pending mail"
    );
    let mail_request: Value =
        serde_json::from_slice(&requests[4]).expect("parse mail follow-up request");
    let mail_request_input = mail_request["input"].as_array().expect("mail input");
    assert!(
        mail_request_input.iter().any(|item| {
            item["type"] == "agent_message"
                && item["content"]
                    == json!([{"type": "input_text", "text": "late queued child update"}])
        }),
        "late queue-only mail should remain pending for the next user turn: {mail_request}"
    );
    for request in [&next_turn, &mail_request] {
        let request_body = request.to_string();
        assert!(!request_body.contains(RECLAIMED_TOOL_OUTPUT_MESSAGE));
        assert!(request_body.contains("mail first answer"));
        assert!(request_body.contains("mail second prompt"));
        for old_call_id in [consumed_call_ids[0], "mail-current"] {
            assert!(
                request["input"]
                    .as_array()
                    .expect("request input")
                    .iter()
                    .all(|item| {
                        item["type"] != "function_call_output" || item["call_id"] != old_call_id
                    })
            );
        }
    }

    server.shutdown().await;
    Ok(())
}
