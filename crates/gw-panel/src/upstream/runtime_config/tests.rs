//! Unit tests for the settings blob's key handling.
//!
//! Reads and writes need Postgres; the normalisation does not, and that is
//! where the bugs are — a key stored under one spelling and read under another
//! makes a setting silently revert.

use super::*;
use serde_json::json;

fn map(raw: Value) -> Map<String, Value> {
    raw.as_object().cloned().expect("object literal")
}

// ---------------------------------------------------------------- shapes

#[test]
fn camel_case_becomes_hyphen_case() {
    assert_eq!(camel_to_hyphen("proxyUrl"), "proxy-url");
    assert_eq!(
        camel_to_hyphen("logsMaxTotalSizeMb"),
        "logs-max-total-size-mb"
    );
}

#[test]
fn a_leading_capital_does_not_produce_a_leading_hyphen() {
    // `-proxy-url` would be a key nothing ever reads.
    assert_eq!(camel_to_hyphen("ProxyUrl"), "proxy-url");
    assert!(!camel_to_hyphen("Debug").starts_with('-'));
}

#[test]
fn an_already_hyphenated_key_is_left_alone() {
    for key in ["proxy-url", "debug", "ws-auth"] {
        assert_eq!(normalize_key(key), key);
    }
}

#[test]
fn snake_case_becomes_hyphen_case() {
    assert_eq!(normalize_key("proxy_url"), "proxy-url");
    assert_eq!(
        normalize_key("logs_max_total_size_mb"),
        "logs-max-total-size-mb"
    );
}

#[test]
fn the_three_spellings_normalise_to_one() {
    // This is the whole point: whichever spelling arrives, one key is stored.
    let canonical = normalize_key("proxy-url");
    for spelling in ["proxyUrl", "proxy_url", "proxy-url"] {
        assert_eq!(normalize_key(spelling), canonical);
    }
}

#[test]
fn hyphen_to_camel_inverts_camel_to_hyphen() {
    for key in ["proxyUrl", "logsMaxTotalSizeMb", "wsAuth", "debug"] {
        assert_eq!(hyphen_to_camel(&camel_to_hyphen(key)), key);
    }
}

#[test]
fn normalisation_reaches_nested_settings() {
    let normalised = normalize_config(&map(json!({
        "outerKey": {"innerKey": 1},
    })));
    let inner = normalised
        .get("outer-key")
        .and_then(Value::as_object)
        .expect("nested object survives");
    assert!(inner.contains_key("inner-key"));
}

#[test]
fn normalisation_does_not_touch_values() {
    // Only keys are settings names; a value that happens to look camelCase is
    // data.
    let normalised = normalize_config(&map(json!({"routingStrategy": "roundRobin"})));
    assert_eq!(
        normalised.get("routing-strategy"),
        Some(&json!("roundRobin"))
    );
}

// ---------------------------------------------------------------- aliases

#[test]
fn a_hyphenated_key_is_echoed_under_all_three_spellings() {
    let expanded = expand_aliases(&map(json!({"proxy-url": "http://p"})));
    for spelling in ["proxy-url", "proxyUrl", "proxy_url"] {
        assert_eq!(
            expanded.get(spelling),
            Some(&json!("http://p")),
            "{spelling} missing"
        );
    }
}

#[test]
fn a_single_word_key_gains_no_aliases() {
    let expanded = expand_aliases(&map(json!({"debug": true})));
    assert_eq!(expanded.len(), 1);
}

#[test]
fn expansion_never_drops_a_stored_key() {
    let stored = map(json!({"a-b": 1, "c": 2, "d_e": 3}));
    let expanded = expand_aliases(&stored);
    for key in stored.keys() {
        assert!(expanded.contains_key(key), "{key} vanished");
    }
}

#[test]
fn an_empty_config_expands_to_an_empty_config() {
    assert!(expand_aliases(&Map::new()).is_empty());
}

#[test]
fn write_then_read_finds_the_value_under_every_spelling() {
    // The round trip that matters: the console PUTs camelCase and GETs
    // whichever spelling its `configValue()` asks for.
    let stored = normalize_config(&map(json!({"proxyUrl": "http://p"})));
    let served = expand_aliases(&stored);
    for spelling in ["proxyUrl", "proxy_url", "proxy-url"] {
        assert_eq!(served.get(spelling), Some(&json!("http://p")));
    }
}

// ---------------------------------------------------------------- the table

#[test]
fn every_plain_key_is_already_canonical() {
    // A route registered under a non-canonical key would write one key and read
    // another.
    for key in PLAIN_KEYS {
        assert_eq!(normalize_key(key), key, "{key} is not canonical");
    }
}

#[test]
fn the_three_special_keys_are_canonical_too() {
    for spec in [ROUTING_STRATEGY, FORCE_MODEL_PREFIX, LOGS_MAX_TOTAL_SIZE_MB] {
        assert_eq!(normalize_key(spec.key), spec.key);
    }
}

#[test]
fn no_key_is_registered_twice() {
    let mut keys: Vec<&str> = PLAIN_KEYS.to_vec();
    keys.extend([
        PROXY_URL_KEY,
        ROUTING_STRATEGY.key,
        FORCE_MODEL_PREFIX.key,
        LOGS_MAX_TOTAL_SIZE_MB.key,
    ]);
    let before = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), before, "a config key is registered twice");
}

#[test]
fn an_alias_never_collides_with_a_real_key() {
    // An alias that shadowed another setting's key would make one of them
    // unreadable.
    let real: Vec<&str> = PLAIN_KEYS
        .iter()
        .copied()
        .chain([
            PROXY_URL_KEY,
            ROUTING_STRATEGY.key,
            FORCE_MODEL_PREFIX.key,
            LOGS_MAX_TOTAL_SIZE_MB.key,
        ])
        .collect();
    for spec in [ROUTING_STRATEGY, FORCE_MODEL_PREFIX, LOGS_MAX_TOTAL_SIZE_MB] {
        for alias in spec.aliases {
            assert!(!real.contains(alias), "{alias} shadows a real key");
        }
    }
}

#[test]
fn a_plain_key_carries_no_default() {
    // Only the three special getters substitute a value; inventing one for the
    // rest would make "never configured" indistinguishable from a real setting.
    for key in PLAIN_KEYS {
        assert!(plain(key).default.is_none());
        assert!(plain(key).aliases.is_empty());
    }
}

#[test]
fn the_defaults_are_the_ones_the_console_expects() {
    // 这些默认值须与控制台期望的一致。
    match ROUTING_STRATEGY.default {
        Some(ConfigDefault::Text(value)) => assert_eq!(value, "round-robin"),
        other => panic!("routing strategy default is {other:?}"),
    }
    match LOGS_MAX_TOTAL_SIZE_MB.default {
        Some(ConfigDefault::Number(value)) => assert_eq!(value, 100),
        other => panic!("log size default is {other:?}"),
    }
    assert!(
        FORCE_MODEL_PREFIX.default.is_none(),
        "an unset prefix is null, not empty"
    );
}

#[test]
fn the_routing_strategy_answers_under_the_key_the_console_reads() {
    // `strategy` is neither spelling of the stored key, and it is the one the
    // console actually looks at.
    assert!(ROUTING_STRATEGY.aliases.contains(&"strategy"));
}

#[test]
fn the_write_only_key_has_no_reader_route() {
    // `proxy-url` 只注册了 PUT 和 DELETE，没有 GET。把它折进普通表
    // 会多出一条本不该有的路由 —— 即便只是新增，也是一次契约变化。
    assert!(!PLAIN_KEYS.contains(&PROXY_URL_KEY));
    assert_eq!(normalize_key(PROXY_URL_KEY), PROXY_URL_KEY);
}
