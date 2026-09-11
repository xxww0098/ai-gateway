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

#[test]
fn force_model_mappings_snake_case_is_folded_into_the_hyphenated_key() {
    let mut payload_true = map(json!({"force_model_mappings": true}));
    normalize_input(&mut payload_true);
    assert_eq!(
        payload_true.get("force-model-mappings"),
        Some(&Value::Bool(true))
    );
    assert!(!payload_true.contains_key("force_model_mappings"));

    let mut payload_false = map(json!({"force_model_mappings": false}));
    normalize_input(&mut payload_false);
    assert_eq!(
        payload_false.get("force-model-mappings"),
        Some(&Value::Bool(false))
    );
    assert!(!payload_false.contains_key("force_model_mappings"));
}

#[test]
fn force_model_mappings_hyphenated_input_wins_when_both_are_sent() {
    let mut payload = map(json!({
        "force-model-mappings": true,
        "force_model_mappings": false,
    }));
    normalize_input(&mut payload);
    assert_eq!(
        payload.get("force-model-mappings"),
        Some(&Value::Bool(true))
    );
    assert!(!payload.contains_key("force_model_mappings"));
}

#[test]
fn force_model_mappings_response_carries_both_spellings_with_boolean_type() {
    let res_true = normalize_response(&map(json!({"force-model-mappings": true})));
    assert_eq!(
        res_true.get("force-model-mappings"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        res_true.get("force_model_mappings"),
        Some(&Value::Bool(true))
    );

    let res_false = normalize_response(&map(json!({"force-model-mappings": false})));
    assert_eq!(
        res_false.get("force-model-mappings"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        res_false.get("force_model_mappings"),
        Some(&Value::Bool(false))
    );

    // Legacy row stored under snake_case
    let res_legacy = normalize_response(&map(json!({"force_model_mappings": true})));
    assert_eq!(
        res_legacy.get("force-model-mappings"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        res_legacy.get("force_model_mappings"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn parse_bool_value_handles_various_formats() {
    // Bare booleans
    assert_eq!(parse_bool_value(&json!(true)), Some(true));
    assert_eq!(parse_bool_value(&json!(false)), Some(false));

    // Bare strings
    assert_eq!(parse_bool_value(&json!("true")), Some(true));
    assert_eq!(parse_bool_value(&json!("false")), Some(false));
    assert_eq!(parse_bool_value(&json!("1")), Some(true));
    assert_eq!(parse_bool_value(&json!("0")), Some(false));
    assert_eq!(parse_bool_value(&json!("TRUE")), Some(true));
    assert_eq!(parse_bool_value(&json!("FALSE")), Some(false));

    // Bare numbers
    assert_eq!(parse_bool_value(&json!(1)), Some(true));
    assert_eq!(parse_bool_value(&json!(0)), Some(false));
    assert_eq!(parse_bool_value(&json!(2)), None);

    // Objects with "value"
    assert_eq!(parse_bool_value(&json!({"value": true})), Some(true));
    assert_eq!(parse_bool_value(&json!({"value": false})), Some(false));
    assert_eq!(parse_bool_value(&json!({"value": "true"})), Some(true));
    assert_eq!(parse_bool_value(&json!({"value": "false"})), Some(false));

    // Objects with "force-model-mappings" / "force_model_mappings"
    assert_eq!(
        parse_bool_value(&json!({"force-model-mappings": true})),
        Some(true)
    );
    assert_eq!(
        parse_bool_value(&json!({"force-model-mappings": false})),
        Some(false)
    );
    assert_eq!(
        parse_bool_value(&json!({"force-model-mappings": "true"})),
        Some(true)
    );
    assert_eq!(
        parse_bool_value(&json!({"force_model_mappings": true})),
        Some(true)
    );
    assert_eq!(
        parse_bool_value(&json!({"force_model_mappings": false})),
        Some(false)
    );

    // Wrapped in "ampcode"
    assert_eq!(
        parse_bool_value(&json!({"ampcode": {"force-model-mappings": true}})),
        Some(true)
    );
    assert_eq!(
        parse_bool_value(&json!({"ampcode": {"value": false}})),
        Some(false)
    );

    // Invalid / unparseable formats
    assert_eq!(parse_bool_value(&json!({"value": "invalid"})), None);
    assert_eq!(parse_bool_value(&json!({})), None);
    assert_eq!(parse_bool_value(&json!([])), None);
    assert_eq!(parse_bool_value(&json!(null)), None);
    assert_eq!(parse_bool_value(&json!("unknown")), None);
}

#[test]
fn get_force_model_mappings_value_returns_defaults_and_parsed_values() {
    assert!(!get_force_model_mappings_value(&Map::new()));

    let config_true = map(json!({"force-model-mappings": true}));
    assert!(get_force_model_mappings_value(&config_true));

    let config_false = map(json!({"force-model-mappings": false}));
    assert!(!get_force_model_mappings_value(&config_false));

    let config_snake = map(json!({"force_model_mappings": true}));
    assert!(get_force_model_mappings_value(&config_snake));

    let config_str = map(json!({"force-model-mappings": "true"}));
    assert!(get_force_model_mappings_value(&config_str));

    let config_invalid = map(json!({"force-model-mappings": "invalid"}));
    assert!(!get_force_model_mappings_value(&config_invalid));
}

#[test]
fn boolean_types_are_preserved_when_saving_and_normalizing() {
    let mut config = Map::new();
    let parsed = parse_bool_value(&json!({"value": "true"})).expect("parsed bool");
    assert!(parsed);

    // Saving as Value::Bool(parsed)
    config.remove("force_model_mappings");
    config.insert("force-model-mappings".to_owned(), Value::Bool(parsed));

    let normalized = normalize_response(&config);
    assert_eq!(
        normalized.get("force-model-mappings"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        normalized.get("force_model_mappings"),
        Some(&Value::Bool(true))
    );

    // Now test with false
    let parsed_false = parse_bool_value(&json!({"value": false})).expect("parsed bool");
    assert!(!parsed_false);
    config.insert("force-model-mappings".to_owned(), Value::Bool(parsed_false));
    let normalized_false = normalize_response(&config);
    assert_eq!(
        normalized_false.get("force-model-mappings"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        normalized_false.get("force_model_mappings"),
        Some(&Value::Bool(false))
    );
}
