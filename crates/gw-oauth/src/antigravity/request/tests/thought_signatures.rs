use serde_json::json;

use super::{
    antigravity_to_openai, contents_of, convert, google_event, isolated, reset_thought_signatures,
};

#[test]
fn thought_signature_round_trips_on_the_matching_part() {
    let _lock = isolated();
    let google = google_event(
        json!([{
            "functionCall": {"name": "default_api:run_code", "args": {"code": "print(1)"}},
            "thoughtSignature": "sig-run-code-A"
        }]),
        None,
        Some("STOP"),
    );
    let openai = antigravity_to_openai(
        &google,
        Some("gemini-3.7-flash-high"),
        Some("chatcmpl-1"),
        None,
    );
    let call = &openai["choices"][0]["message"]["tool_calls"][0];
    assert_eq!(call["thoughtSignature"], "sig-run-code-A");
    assert_eq!(
        call["extra_content"]["google"]["thought_signature"],
        "sig-run-code-A"
    );

    reset_thought_signatures();
    let back = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-inband-1",
            "messages": [
                {"role": "user", "content": "run"},
                {"role": "assistant", "content": null, "tool_calls": [call]},
                {"role": "tool", "tool_call_id": call["id"], "name": "default_api:run_code", "content": "1"}
            ]
        }),
        "p",
    );
    let model = contents_of(&back)
        .iter()
        .find(|c| c["role"] == "model")
        .expect("model");
    assert_eq!(model["parts"][0]["thoughtSignature"], "sig-run-code-A");
    assert!(
        model["parts"][0]["functionCall"]
            .get("thoughtSignature")
            .is_none()
    );
}

#[test]
fn session_map_reattaches_signature_when_tool_call_keys_are_stripped() {
    let _lock = isolated();
    let google = google_event(
        json!([{
            "functionCall": {"name": "default_api:run_code", "args": {"code": "print(1)"}},
            "thoughtSignature": "sig-map-A"
        }]),
        None,
        None,
    );
    let _ = antigravity_to_openai(
        &google,
        Some("gemini-3.7-flash-high"),
        None,
        Some("session-sig-1"),
    );
    let back = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-sig-1",
            "messages": [
                {"role": "user", "content": "run"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "default_api:run_code", "arguments": "{\"code\":\"print(1)\"}"}
                    }]
                },
                {"role": "tool", "tool_call_id": "call_1", "name": "default_api:run_code", "content": "1"}
            ]
        }),
        "p",
    );
    assert_eq!(
        contents_of(&back)[1]["parts"][0]["thoughtSignature"],
        "sig-map-A"
    );
}

#[test]
fn thought_only_part_signature_moves_onto_the_following_unsigned_call() {
    let _lock = isolated();
    let google = google_event(
        json!([
            {"thought": true, "text": "planning run_code", "thoughtSignature": "sig-thought"},
            {"functionCall": {"name": "default_api:run_code", "args": {"code": "1+1"}}}
        ]),
        None,
        None,
    );
    let openai = antigravity_to_openai(&google, Some("gemini-3.7-flash-high"), None, None);
    assert!(openai["choices"][0]["message"]["content"].is_null());
    assert_eq!(
        openai["choices"][0]["message"]["tool_calls"][0]["thoughtSignature"],
        "sig-thought"
    );
}
