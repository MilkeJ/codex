use codex_features::Feature;
use codex_login::CodexAuth;
use core_test_support::responses;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_completed_with_tokens;
use core_test_support::responses::ev_reasoning_item;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::ev_shell_command_call_with_args;
use core_test_support::responses::sse;
use core_test_support::test_codex::TestCodexBuilder;
use core_test_support::test_codex::test_codex;
use serde_json::Value;
use serde_json::json;
pub(super) const AUTO_COMPACT_LIMIT: i64 = 100_000;
pub(super) const LARGE_OUTPUT_CHARS: usize = 36_000;
pub(super) const RECLAIMED_TOOL_OUTPUT_MESSAGE: &str =
    "Tool output retired after it was consumed by the model to reclaim context before compaction.";
pub(super) const CONTEXT_WINDOW_TRUNCATED_OUTPUT_MESSAGE: &str =
    "Output exceeded the available model context and was truncated";

pub(super) fn test_builder(auto_compact_limit: i64) -> TestCodexBuilder {
    test_codex()
        .with_model("gpt-5.4")
        .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
        .with_config(move |config| {
            let _ = config.features.disable(Feature::RemoteCompactionV2);
            config.model_context_window = Some(300_000);
            config.model_auto_compact_token_limit = Some(auto_compact_limit);
            config.tool_output_token_limit = Some(10_000);
        })
}

pub(super) fn large_output_args() -> Value {
    let command = if cfg!(windows) {
        format!("[Console]::Out.Write([string]::new([char]'0', {LARGE_OUTPUT_CHARS}))")
    } else {
        format!("printf '%0{LARGE_OUTPUT_CHARS}d' 0")
    };
    json!({
        "command": command,
        "login": false,
        "timeout_ms": 5_000,
    })
}

pub(super) fn tool_call_events(
    response_id: &str,
    call_ids: &[&str],
    total_tokens: i64,
) -> Vec<Value> {
    let args = large_output_args();
    let mut events = vec![ev_response_created(response_id)];
    events.push(ev_reasoning_item(
        &format!("reasoning-{response_id}"),
        &["retain active reasoning"],
        &["private active state"],
    ));
    events.extend(
        call_ids
            .iter()
            .map(|call_id| ev_shell_command_call_with_args(call_id, &args)),
    );
    events.push(ev_completed_with_tokens(response_id, total_tokens));
    events
}

pub(super) fn tool_call_response(
    response_id: &str,
    call_ids: &[&str],
    total_tokens: i64,
) -> String {
    sse(tool_call_events(response_id, call_ids, total_tokens))
}

pub(super) fn compact_response() -> Value {
    json!({
        "output": [{
            "type": "compaction",
            "encrypted_content": "reclaimed tool output test checkpoint"
        }]
    })
}

pub(super) fn compact_v2_response() -> String {
    sse(vec![
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "compaction",
                "encrypted_content": "reclaimed tool output v2 checkpoint",
            }
        }),
        ev_completed("reclaimed-tool-output-v2-compact"),
    ])
}

pub(super) fn assert_original_output(request: &responses::ResponsesRequest, call_id: &str) {
    let output = request
        .function_call_output_text(call_id)
        .expect("function call output");
    assert_ne!(output, RECLAIMED_TOOL_OUTPUT_MESSAGE);
    assert!(
        output.len() >= LARGE_OUTPUT_CHARS,
        "expected original output for {call_id}, got {} characters",
        output.len()
    );
}
