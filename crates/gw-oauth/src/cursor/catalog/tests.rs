use super::{
    infer_context_window, infer_max_output_tokens, is_internal_model, merge_static_floor,
    picker_family_id, source_is_fast, static_models, to_picker_models,
};
use crate::cursor::proto::{AvailableModel, UsableModel};

fn usable(id: &str, name: &str) -> UsableModel {
    UsableModel {
        id: id.to_owned(),
        display_id: id.to_owned(),
        name: name.to_owned(),
        max_mode: false,
    }
}

#[test]
fn static_catalog_has_no_fast_or_auto_rows() {
    let models = static_models();
    assert!(!models.is_empty());
    assert!(models.iter().all(|m| !m.id.ends_with("-fast")));
    assert!(models.iter().all(|m| m.id != "default" && m.id != "auto"));
}

#[test]
fn empty_live_keeps_the_static_floor() {
    let merged = merge_static_floor(&[]);
    let floor = static_models();
    assert_eq!(
        merged.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        floor.iter().map(|m| m.id.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn non_empty_live_is_trusted() {
    let live = to_picker_models(
        &[
            usable("composer-2", "Composer 2"),
            usable("gpt-5.5", "GPT-5.5"),
        ],
        &[],
    );
    let merged = merge_static_floor(&live);
    assert_eq!(merged.len(), live.len());
    assert!(!merged.iter().any(|m| m.id == "composer-2.5"));
}

#[test]
fn picker_collapses_effort_fast_thinking_and_hides_tab() {
    let rows = to_picker_models(
        &[
            usable("default", "Auto"),
            usable("default-fast", "Auto Fast"),
            usable("gpt-5.5-none", "GPT-5.5 272K None"),
            usable("gpt-5.5-high-fast", "GPT-5.5 272K High Fast"),
            usable("gpt-5.5-1m-extra-high", "GPT-5.5 1M Extra High"),
            usable("claude-4.6-opus-max-thinking", "Opus 4.6 1M Max Thinking"),
            usable("claude-4.6-opus-high", "Opus 4.6 1M"),
            usable("gpt-5.1-codex-max-high-fast", "GPT-5.1 Codex Max High Fast"),
            usable("composer-2-fast", "Composer 2 Fast"),
            usable("cursor-small", "Tab"),
            usable("tab-completion", "Tab completion"),
            usable("cursor-chat", "Chat"),
        ],
        &[],
    );
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert!(ids.contains(&"default"));
    assert_eq!(
        rows.iter()
            .find(|r| r.id == "default")
            .map(|r| r.name.as_str()),
        Some("Cursor Auto")
    );
    assert!(ids.contains(&"gpt-5.5"));
    assert!(!ids.contains(&"gpt-5.5-high-fast"));
    assert!(!ids.contains(&"gpt-5.5-none"));
    assert!(ids.contains(&"claude-4.6-opus"));
    assert!(ids.contains(&"gpt-5.1-codex-max"));
    assert!(ids.contains(&"composer-2"));
    assert!(ids.contains(&"gpt-5.5-fast"));
    assert!(ids.contains(&"composer-2-fast"));
    assert!(ids.contains(&"gpt-5.1-codex-max-fast"));
    assert!(!ids.contains(&"claude-4.6-opus-fast"));
    assert!(!ids.contains(&"default-fast"));
    assert!(
        !ids.iter()
            .any(|id| id.contains("tab") || *id == "cursor-small" || *id == "cursor-chat")
    );
    assert!(is_internal_model("cursor-small", "Tab"));
    assert_eq!(picker_family_id("gpt-5.5-max-extra-high-fast"), "gpt-5.5");
    assert!(source_is_fast("gpt-5.5-high-fast"));
    assert!(source_is_fast("composer-2-fast"));
    assert!(!source_is_fast("gpt-5.5-high"));
    assert!(!source_is_fast("default"));
}

#[test]
fn inferred_windows_match_family_not_codex_tier() {
    assert_eq!(infer_context_window("grok-4.5", "Grok 4.5"), 256_000);
    assert_eq!(infer_context_window("grok-4.6", "Grok 4.6"), 256_000);
    assert_eq!(
        infer_context_window("claude-opus-5", "Claude Opus 5"),
        300_000
    );
    assert_eq!(infer_max_output_tokens("gpt-5.5", "GPT-5.5"), 128_000);
    assert_eq!(
        infer_max_output_tokens("claude-sonnet-5", "Claude Sonnet 5"),
        128_000
    );
}

#[test]
fn available_models_can_seed_a_family_without_inventing_fast() {
    let from_available = to_picker_models(
        &[],
        &[
            AvailableModel {
                name: "composer-2-fast".into(),
                supports_images: false,
                supports_max_mode: false,
                context_token_limit: None,
                context_token_limit_for_max_mode: None,
                client_display_name: Some("Composer 2 Fast".into()),
                server_model_name: None,
                supports_non_max_mode: false,
                variants: Vec::new(),
            },
            AvailableModel {
                name: "kimi-k2.5".into(),
                supports_images: false,
                supports_max_mode: false,
                context_token_limit: None,
                context_token_limit_for_max_mode: None,
                client_display_name: Some("Kimi K2.5".into()),
                server_model_name: None,
                supports_non_max_mode: false,
                variants: Vec::new(),
            },
        ],
    );
    assert!(from_available.iter().any(|r| r.id == "composer-2-fast"));
    assert!(!from_available.iter().any(|r| r.id == "kimi-k2.5-fast"));
}
