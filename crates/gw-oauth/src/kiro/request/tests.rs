use serde_json::{Value, json};

use super::{
    SYSTEM_ACK, StreamEvent, classify_hop_error, client_error_status, collect_events,
    encode_event_stream, normalize_tool_use_id, openai_to_kiro, parse_event_stream, to_openai,
};
use crate::kiro::cache::reset_system_pins;
use crate::kiro::catalog::CONTEXT_WINDOW;

fn tool_call(id: &str, name: &str) -> Value {
    json!({"id": id, "type": "function", "function": {"name": name, "arguments": "{}"}})
}

fn history(body: &Value) -> &[Value] {
    body["conversationState"]["history"]
        .as_array()
        .expect("history")
}

fn current(body: &Value) -> &Value {
    &body["conversationState"]["currentMessage"]["userInputMessage"]
}

fn tool_use_ids(row: &Value) -> Vec<&str> {
    row["assistantResponseMessage"]["toolUses"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|item| item["toolUseId"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

fn tool_result_ids(row: &Value) -> Vec<&str> {
    let results = row
        .get("userInputMessage")
        .and_then(|u| u.get("userInputMessageContext"))
        .and_then(|c| c.get("toolResults"))
        .or_else(|| {
            row.get("userInputMessageContext")
                .and_then(|c| c.get("toolResults"))
        });
    results
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|item| item["toolUseId"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

fn is_ack(row: &Value) -> bool {
    row["assistantResponseMessage"]["content"].as_str() == Some(SYSTEM_ACK)
}

fn assert_result_follows_use(history: &[Value], current: &Value, tool_use_id: &str) {
    let idx = history
        .iter()
        .position(|row| tool_use_ids(row).contains(&tool_use_id));
    let idx = idx.expect("missing tool_use");
    let after = history.get(idx + 1);
    if let Some(after) = after {
        assert!(!is_ack(after), "ack between tool_use and tool_result");
    }
    let result_ids = after
        .map(tool_result_ids)
        .unwrap_or_else(|| tool_result_ids(current));
    assert!(
        result_ids.contains(&tool_use_id),
        "missing tool_result {tool_use_id}"
    );
}

#[test]
fn system_is_first_history_pair_not_current_content() {
    reset_system_pins();
    let body = openai_to_kiro(
        &json!({
            "model": "deepseek-3.2",
            "session_id": "session-dsh-1",
            "messages": [
                {"role": "developer", "content": "You are DSH."},
                {"role": "system", "content": "Be brief."},
                {"role": "user", "content": "hello"}
            ]
        }),
        None,
        None,
    )
    .expect("translate");
    let user = current(&body);
    assert_eq!(user["content"], "hello");
    assert!(
        !user["content"]
            .as_str()
            .unwrap_or("")
            .contains("You are DSH.")
    );
    assert_eq!(
        body["conversationState"]["conversationId"],
        "session-dsh-1:deepseek-3.2"
    );
    assert!(
        !body["conversationState"]["conversationId"]
            .as_str()
            .unwrap_or("")
            .chars()
            .all(|c| c.is_ascii_digit())
    );
    let hist = history(&body);
    assert_eq!(hist.len(), 2);
    assert_eq!(
        hist[0]["userInputMessage"]["content"],
        "You are DSH.\nBe brief."
    );
    assert_eq!(hist[1]["assistantResponseMessage"]["content"], SYSTEM_ACK);
    assert!(
        hist[0]["userInputMessage"]["modelId"]
            .as_str()
            .unwrap_or("")
            .contains('.')
    );
}

#[test]
fn later_snapshot_parks_at_suffix_and_skips_tool_gap() {
    reset_system_pins();
    openai_to_kiro(
        &json!({
            "model": "claude-sonnet-5",
            "session_id": "session-dsh-snap",
            "messages": [
                {"role": "system", "content": "You are DSH."},
                {"role": "user", "content": "hello"}
            ]
        }),
        None,
        None,
    )
    .expect("first");
    let second = openai_to_kiro(
        &json!({
            "model": "claude-sonnet-5",
            "session_id": "session-dsh-snap",
            "messages": [
                {"role": "system", "content": "You are DSH.\nThis snapshot supersedes the previous context."},
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": "Hi!"},
                {"role": "user", "content": "again"}
            ]
        }),
        None,
        None,
    )
    .expect("second");
    let hist = history(&second);
    assert_eq!(hist[0]["userInputMessage"]["content"], "You are DSH.");
    assert!(is_ack(&hist[1]));
    assert_eq!(
        hist[hist.len() - 2]["userInputMessage"]["content"],
        "This snapshot supersedes the previous context."
    );
    assert!(is_ack(hist.last().expect("ack")));
    assert_eq!(current(&second)["content"], "again");
    let dump = second.to_string();
    assert!(!dump.contains("prompt_cache_key"));
    assert!(!dump.contains("x-grok-conv-id"));
    assert!(!dump.contains("session-id"));
}

#[test]
fn tools_live_on_current_context_and_results_follow_uses() {
    reset_system_pins();
    let ez5 = "toolu_bdrk_01Ez5MSML7fNdsjMvkPUTeCd";
    let ez5_wire = "tooluse_bdrk_01Ez5MSML7fNdsjMvkPUTeCd";
    openai_to_kiro(
        &json!({
            "model": "claude-sonnet-5",
            "session_id": "session-dsh-tool-extra",
            "messages": [
                {"role": "system", "content": "You are DSH."},
                {"role": "user", "content": "use the tool"}
            ]
        }),
        None,
        None,
    )
    .expect("pin");
    let extra = "This snapshot supersedes the previous context.";
    let body = openai_to_kiro(
        &json!({
            "model": "claude-sonnet-5",
            "session_id": "session-dsh-tool-extra",
            "tools": [{"type": "function", "function": {"name": "Read", "parameters": {"type": "object", "properties": {}}}}],
            "messages": [
                {"role": "system", "content": format!("You are DSH.\n{extra}")},
                {"role": "user", "content": "use the tool"},
                {"role": "assistant", "content": "", "tool_calls": [tool_call(ez5, "Read")]},
                {"role": "tool", "tool_call_id": ez5, "content": "{\"ok\":true}"}
            ]
        }),
        None,
        None,
    )
    .expect("tool turn");
    let hist = history(&body);
    let cur = current(&body);
    assert_eq!(
        cur["userInputMessageContext"]["tools"][0]["toolSpecification"]["name"],
        "Read"
    );
    assert!(body["conversationState"].get("tools").is_none());
    assert_result_follows_use(hist, cur, ez5_wire);
    assert_eq!(tool_use_ids(hist.last().expect("last")), vec![ez5_wire]);
    assert_eq!(tool_result_ids(cur), vec![ez5_wire]);
    assert!(!is_ack(hist.last().expect("last")));
    assert!(
        hist.iter()
            .any(|row| row["userInputMessage"]["content"].as_str() == Some(extra))
    );
}

#[test]
fn tools_array_is_on_current_not_conversation_root() {
    let body = openai_to_kiro(
        &json!({
            "model": "deepseek-3.2",
            "tools": [{"type": "function", "function": {"name": "run_code", "description": "run", "parameters": {"type": "object", "properties": {"code": {"type": "string"}}}}}],
            "messages": [
                {"role": "user", "content": "run it"},
                {"role": "assistant", "content": "", "tool_calls": [tool_call("call_1", "run_code")]},
                {"role": "tool", "tool_call_id": "call_1", "content": "{\"ok\":true}"},
                {"role": "user", "content": "thanks"}
            ]
        }),
        None,
        None,
    )
    .expect("translate");
    let cur = current(&body);
    assert_eq!(
        cur["userInputMessageContext"]["tools"][0]["toolSpecification"]["name"],
        "run_code"
    );
    assert!(body["conversationState"].get("tools").is_none());
    let dump = body.to_string();
    assert!(!dump.contains("Date.now"));
}

#[test]
fn piped_tool_ids_remap_stably_on_use_and_result() {
    let piped = "call_abc123|fc_this_is_an_openai_responses_compound_id_over_sixty_four_chars_xx";
    assert!(piped.contains('|'));
    assert!(piped.len() > 64);
    let first = normalize_tool_use_id(Some(piped)).expect("id");
    let second = normalize_tool_use_id(Some(piped)).expect("id");
    assert_eq!(first, second);
    assert!(first.starts_with("tooluse_"));
    assert!(!first.contains('|'));
    assert!(first.len() <= 64);
    assert_eq!(
        normalize_tool_use_id(Some("call_1")).as_deref(),
        Some("tooluse_1")
    );
    let body = openai_to_kiro(
        &json!({
            "model": "claude-opus-5",
            "messages": [
                {"role": "user", "content": "run it"},
                {"role": "assistant", "content": "", "tool_calls": [tool_call(piped, "run_code")]},
                {"role": "tool", "tool_call_id": piped, "content": "{\"ok\":true}"}
            ]
        }),
        None,
        None,
    )
    .expect("translate");
    let hist = history(&body);
    let use_id = hist
        .iter()
        .find_map(|row| tool_use_ids(row).into_iter().next())
        .expect("use");
    assert_eq!(use_id, first);
    assert_eq!(tool_result_ids(current(&body)), vec![first.as_str()]);
}

#[test]
fn interleaved_results_relocate_without_fabricating_text() {
    let body = openai_to_kiro(
        &json!({
            "model": "claude-sonnet-5",
            "messages": [
                {"role": "user", "content": "start"},
                {"role": "assistant", "content": "", "tool_calls": [tool_call("call_A", "alpha")]},
                {"role": "user", "content": "meanwhile"},
                {"role": "assistant", "content": "", "tool_calls": [tool_call("call_B", "bravo")]},
                {"role": "tool", "tool_call_id": "call_A", "content": "{\"a\":1}"},
                {"role": "tool", "tool_call_id": "call_B", "content": "{\"b\":2}"}
            ]
        }),
        None,
        None,
    )
    .expect("translate");
    let hist = history(&body);
    let a_idx = hist
        .iter()
        .position(|row| tool_use_ids(row).contains(&"tooluse_A"))
        .expect("A");
    assert_eq!(tool_result_ids(&hist[a_idx + 1]), vec!["tooluse_A"]);
    assert_eq!(hist[a_idx + 1]["userInputMessage"]["content"], "");
    assert!(
        hist.iter()
            .any(|row| row["userInputMessage"]["content"].as_str() == Some("meanwhile"))
    );
    assert_eq!(tool_result_ids(current(&body)), vec!["tooluse_B"]);
    assert!(!body.to_string().contains("Tool results provided."));
}

#[test]
fn switching_models_does_not_reuse_conversation_id() {
    reset_system_pins();
    let a = openai_to_kiro(
        &json!({"model": "claude-opus-5", "session_id": "session-shared", "messages": [{"role": "user", "content": "hello"}]}),
        None,
        None,
    )
    .expect("a");
    let b = openai_to_kiro(
        &json!({"model": "glm-5", "session_id": "session-shared", "messages": [{"role": "user", "content": "hello"}]}),
        None,
        None,
    )
    .expect("b");
    assert_ne!(
        a["conversationState"]["conversationId"],
        b["conversationState"]["conversationId"]
    );
}

#[test]
fn monthly_quota_is_a_client_error_not_a_retryable_429() {
    let body =
        json!({"reason": "MONTHLY_REQUEST_COUNT", "message": "monthly request count exceeded"});
    let classified = classify_hop_error(429, &body, &body.to_string(), None);
    assert_eq!(classified.status, 400);
    assert_ne!(classified.status, 429);
    assert_eq!(classified.code, "kiro_quota");
    assert_eq!(client_error_status(429, &body, &body.to_string()), 400);
    assert_eq!(
        classify_hop_error(
            429,
            &json!({"reason": "USER_REQUEST_RATE_EXCEEDED"}),
            "",
            Some("2")
        )
        .status,
        429
    );
    assert_eq!(
        classify_hop_error(
            503,
            &json!({"reason": "INSUFFICIENT_MODEL_CAPACITY"}),
            "",
            None
        )
        .status,
        503
    );
    assert_eq!(client_error_status(403, &json!({}), ""), 400);
}

#[test]
fn context_usage_event_estimates_tokens_and_thinking_stays_out_of_content() {
    let frames = encode_event_stream(&[
        StreamEvent {
            type_name: "assistantResponseEvent".into(),
            message_type: "event".into(),
            payload: json!({"content": "ALPHA", "modelId": "claude-haiku-4.5"}),
        },
        StreamEvent {
            type_name: "contextUsageEvent".into(),
            message_type: "event".into(),
            payload: json!({"contextUsagePercentage": 1.2}),
        },
        StreamEvent {
            type_name: "meteringEvent".into(),
            message_type: "event".into(),
            payload: json!({"unit": "credit", "usage": 0.016954}),
        },
        StreamEvent {
            type_name: "thinkingEvent".into(),
            message_type: "event".into(),
            payload: json!({"text": "I will plan the edit"}),
        },
    ]);
    let events = parse_event_stream(&frames);
    assert!(events.iter().any(|e| e.type_name == "contextUsageEvent"));
    assert!(events.iter().any(|e| e.type_name == "thinkingEvent"));
    let collected = collect_events(&events);
    assert_eq!(collected.text, "ALPHA");
    assert_eq!(collected.thinking, "I will plan the edit");
    assert!(!collected.text.contains("<thinking>"));
    let openai = to_openai(&events, "claude-haiku-4.5", "chatcmpl-live");
    assert_eq!(openai["choices"][0]["message"]["content"], "ALPHA");
    assert_eq!(
        openai["usage"]["prompt_tokens"],
        json!(((CONTEXT_WINDOW as f64) * 1.2 / 100.0).round() as i64)
    );
    assert_ne!(openai["usage"]["prompt_tokens"], json!(0.016954));
}
