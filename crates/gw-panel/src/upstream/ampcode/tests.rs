//! Unit tests for the Ampcode settings blob's key handling.
//!
//! The five known settings exist under two spellings each. Storing both, or
//! storing the wrong one, makes a setting appear to revert after a save — which
//! is the failure this file is here to prevent.

use super::*;
use serde_json::json;

fn map(raw: Value) -> Map<String, Value> {
    raw.as_object().cloned().expect("object literal")
}

#[test]
fn a_snake_case_input_is_folded_into_the_hyphenated_key() {
    let mut payload = map(json!({"upstream_url": "https://a"}));
    normalize_input(&mut payload);
    assert_eq!(payload.get("upstream-url"), Some(&json!("https://a")));
    // The snake_case key must be gone, or both spellings end up in the blob and
    // the next read has to guess which one is current.
    assert!(!payload.contains_key("upstream_url"));
}

#[test]
fn the_hyphenated_input_wins_when_both_are_sent() {
    let mut payload = map(json!({"upstream-url": "canonical", "upstream_url": "legacy"}));
    normalize_input(&mut payload);
    assert_eq!(payload.get("upstream-url"), Some(&json!("canonical")));
    assert!(!payload.contains_key("upstream_url"));
}

#[test]
fn only_the_known_settings_are_folded() {
    // An unrecognised snake_case key is somebody's own setting; renaming it
    // would lose it.
    let mut payload = map(json!({"some_other_key": 1}));
    normalize_input(&mut payload);
    assert!(payload.contains_key("some_other_key"));
}

#[test]
fn a_response_carries_both_spellings() {
    let response = normalize_response(&map(json!({"upstream-url": "https://a"})));
    assert_eq!(response.get("upstream-url"), Some(&json!("https://a")));
    assert_eq!(response.get("upstream_url"), Some(&json!("https://a")));
}

#[test]
fn a_legacy_row_stored_under_snake_case_is_still_served_hyphenated() {
    // Rows written before input normalisation landed still exist.
    let response = normalize_response(&map(json!({"model_mappings": [1]})));
    assert_eq!(response.get("model-mappings"), Some(&json!([1])));
    assert_eq!(response.get("model_mappings"), Some(&json!([1])));
}

#[test]
fn every_known_pair_is_covered_in_both_directions() {
    for (hyphen, snake) in KNOWN_KEY_PAIRS {
        let from_hyphen = normalize_response(&map(json!({hyphen: "v"})));
        assert_eq!(from_hyphen.get(snake), Some(&json!("v")), "{hyphen}");

        let from_snake = normalize_response(&map(json!({snake: "v"})));
        assert_eq!(from_snake.get(hyphen), Some(&json!("v")), "{snake}");
    }
}

#[test]
fn the_two_spellings_of_a_pair_really_differ() {
    // A typo that made both halves identical would silently disable folding.
    for (hyphen, snake) in KNOWN_KEY_PAIRS {
        assert_ne!(hyphen, snake);
        assert_eq!(hyphen.replace('-', "_"), snake, "{hyphen} / {snake}");
    }
}

#[test]
fn the_singular_and_plural_upstream_keys_are_distinct_settings() {
    // `upstream-api-key` is a string, `upstream-api-keys` a list. One character
    // apart, and confusing them would silently drop a pool.
    let response = normalize_response(&map(json!({
        "upstream-api-key": "single",
        "upstream-api-keys": [{"upstream-api-key": "listed"}],
    })));
    assert_eq!(response.get("upstream-api-key"), Some(&json!("single")));
    assert!(
        response
            .get("upstream-api-keys")
            .is_some_and(Value::is_array)
    );
}

#[test]
fn normalisation_is_idempotent() {
    // The console re-submits what it was served; a second pass must not grow
    // the blob without bound.
    let once = normalize_response(&map(json!({"upstream-url": "https://a"})));
    let twice = normalize_response(&once);
    assert_eq!(once, twice);
}

#[test]
fn an_empty_config_stays_empty() {
    assert!(normalize_response(&Map::new()).is_empty());
    let mut payload = Map::new();
    normalize_input(&mut payload);
    assert!(payload.is_empty());
}
