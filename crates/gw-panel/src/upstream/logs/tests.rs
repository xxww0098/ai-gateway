//! Unit tests for the log panes' query handling and the built-in model lists.

use super::*;

#[test]
fn an_absent_or_unparseable_limit_uses_the_default() {
    for raw in [None, Some(""), Some("abc"), Some("12x")] {
        assert_eq!(limit_of(raw), DEFAULT_LIMIT);
    }
}

#[test]
fn a_non_positive_limit_falls_back_rather_than_clamping_to_one() {
    // This is where the log routes differ from the panel's usual `queryInt`,
    // which would clamp `0` up to the minimum. 这里回落到默认值。
    for raw in ["0", "-1", "-999"] {
        assert_eq!(limit_of(Some(raw)), DEFAULT_LIMIT);
    }
}

#[test]
fn a_limit_over_the_ceiling_is_capped() {
    assert_eq!(limit_of(Some("100000")), MAX_LIMIT);
    assert_eq!(limit_of(Some("200")), MAX_LIMIT);
}

#[test]
fn a_limit_inside_the_range_is_honoured() {
    assert_eq!(limit_of(Some("1")), 1);
    assert_eq!(limit_of(Some("37")), 37);
}

#[test]
fn only_the_two_named_levels_filter() {
    assert_eq!(level_filter(Some("error")), Some(true));
    assert_eq!(level_filter(Some("info")), Some(false));
    for ignored in [None, Some(""), Some("ERROR"), Some("warn"), Some("all")] {
        assert_eq!(level_filter(ignored), None, "{ignored:?}");
    }
}

#[test]
fn the_five_provider_channels_all_have_a_fallback_list() {
    // The picker has to offer something on a fresh install, before any traffic
    // has populated the catalog.
    for channel in ["openai", "claude", "gemini", "codex", "vertex"] {
        let models = static_models(channel).unwrap_or_default();
        assert!(!models.is_empty(), "{channel} has no fallback models");
    }
}

#[test]
fn an_unknown_channel_has_no_fallback() {
    // It must 404 rather than showing another provider's models.
    for channel in ["", "anthropic", "openai-compatibility", "nope"] {
        assert!(static_models(channel).is_none(), "{channel}");
    }
}

#[test]
fn no_fallback_list_repeats_a_model() {
    for channel in ["openai", "claude", "gemini", "codex", "vertex"] {
        let models = static_models(channel).unwrap_or_default();
        let mut sorted: Vec<&str> = models.to_vec();
        let before = sorted.len();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), before, "{channel} lists a model twice");
    }
}

#[test]
fn no_fallback_model_id_is_blank() {
    for channel in ["openai", "claude", "gemini", "codex", "vertex"] {
        for model in static_models(channel).unwrap_or_default() {
            assert!(!model.trim().is_empty(), "{channel} has a blank model id");
        }
    }
}
