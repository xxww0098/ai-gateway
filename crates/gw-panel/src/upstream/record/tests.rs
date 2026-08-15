//! Unit tests for credential serialisation.
//!
//! Everything here is a pure function over an [`AuthRecord`], so all of it is
//! testable without a store. The claims worth pinning are the ones an operator
//! would notice: a secret that leaked, a preview that overwrote a real key, a
//! name that changed between a list and a delete.

use super::*;
use gw_authcore::AuthStatus;
use serde_json::json;

fn record() -> AuthRecord {
    AuthRecord::new("auth-1", "openai", DateTime::UNIX_EPOCH)
}

fn with_metadata(pairs: &[(&str, Value)]) -> AuthRecord {
    let mut record = record();
    let object = record.metadata.as_object_mut().expect("object");
    for (key, value) in pairs {
        object.insert((*key).to_owned(), value.clone());
    }
    record
}

// ---------------------------------------------------------------- masking

#[test]
fn a_mask_never_reveals_a_short_secret() {
    // The whole point: what comes back must not be enough to authenticate.
    for secret in ["a", "ab", "abcdefgh", "abcdefghi", "sk-0123456789abcdef"] {
        let masked = mask_secret(secret);
        assert_ne!(masked, secret);
        assert!(masked.contains("..."));
        assert!(masked.len() < secret.len() + 4 || secret.len() <= 8);
    }
}

#[test]
fn masking_keeps_enough_to_tell_two_keys_apart() {
    // An operator picks the right row by its preview; masking everything would
    // make a pool of five keys indistinguishable.
    let left = mask_secret("sk-aaaaaaaaaaaa1111");
    let right = mask_secret("sk-bbbbbbbbbbbb2222");
    assert_ne!(left, right);
}

#[test]
fn an_absent_secret_masks_to_nothing() {
    assert!(mask_secret("").is_empty());
    assert!(mask_secret("   ").is_empty());
}

#[test]
fn masking_a_multibyte_secret_does_not_panic() {
    // 旧实现按字节切片会产生 mojibake；Rust 里真正的风险是
    // a panic on a char boundary, which would 500 the whole credential list.
    let masked = mask_secret("密钥密钥密钥密钥密钥");
    assert!(masked.contains("..."));
}

#[test]
fn a_mask_is_recognised_as_a_mask() {
    // This is what stops "load the form, press save" from writing the preview
    // back over a live credential.
    for secret in ["sk-live-key-000000", "short"] {
        assert!(looks_masked(&mask_secret(secret)));
    }
    assert!(looks_masked("••••1234"));
    assert!(looks_masked("sk-***"));
}

#[test]
fn a_real_key_is_not_mistaken_for_a_mask() {
    for secret in ["sk-0123456789abcdef", "AIzaSyA-abcdefghijklmnop", "k"] {
        assert!(!looks_masked(secret));
    }
}

// ---------------------------------------------------------------- attributes

#[test]
fn an_absent_attribute_is_null_not_false() {
    // The console distinguishes "websockets off" from "not configured"; a
    // `false` here would silently turn every credential into an explicit off.
    let record = record();
    assert_eq!(attr_bool(&record, "websockets"), Value::Null);
    assert_eq!(attr_number(&record, "priority"), Value::Null);
}

#[test]
fn an_integral_priority_stays_integral() {
    // `1.0` in the UI where the operator typed `1` reads as a bug.
    let mut record = record();
    record.set_attribute("priority", "1");
    assert_eq!(attr_number(&record, "priority"), json!(1));
}

#[test]
fn a_fractional_priority_survives() {
    let mut record = record();
    record.set_attribute("priority", "1.5");
    assert_eq!(attr_number(&record, "priority"), json!(1.5));
}

#[test]
fn an_unparseable_number_is_shown_rather_than_dropped() {
    // An operator who typed something wrong needs to see it to fix it.
    let mut record = record();
    record.set_attribute("priority", "high");
    assert_eq!(attr_number(&record, "priority"), json!("high"));
}

#[test]
fn attributes_are_read_under_either_spelling() {
    let mut hyphen = record();
    hyphen.set_attribute("base-url", "https://a");
    let mut snake = record();
    snake.set_attribute("base_url", "https://a");

    assert_eq!(attr(&hyphen, &["base_url", "base-url"]), "https://a");
    assert_eq!(attr(&snake, &["base_url", "base-url"]), "https://a");
}

#[test]
fn the_first_listed_spelling_wins() {
    let mut record = record();
    record.set_attribute("base_url", "canonical");
    record.set_attribute("base-url", "legacy");
    assert_eq!(attr(&record, &["base_url", "base-url"]), "canonical");
}

#[test]
fn a_blank_attribute_falls_through_to_the_next_spelling() {
    let mut record = record();
    record.set_attribute("base_url", "  ");
    record.set_attribute("base-url", "legacy");
    assert_eq!(attr(&record, &["base_url", "base-url"]), "legacy");
}

#[test]
fn the_proxy_column_outranks_the_attribute() {
    let mut record = record();
    record.set_attribute("proxy_url", "http://stale");
    record.proxy_url = "http://current".to_owned();
    assert_eq!(proxy_url(&record), "http://current");
}

// ---------------------------------------------------------------- tombstones

#[test]
fn a_tombstone_is_recognised_from_either_place() {
    let mut by_attribute = record();
    by_attribute.set_attribute(DELETED_ATTRIBUTE, "true");
    assert!(is_deleted(&by_attribute));

    assert!(is_deleted(&with_metadata(&[("deleted", json!(true))])));
    assert!(is_deleted(&with_metadata(&[("deleted", json!("TRUE"))])));
}

#[test]
fn a_live_credential_is_not_a_tombstone() {
    assert!(!is_deleted(&record()));
    assert!(!is_deleted(&with_metadata(&[("deleted", json!(false))])));
    assert!(!is_deleted(&with_metadata(&[("deleted", json!("no"))])));
}

// ---------------------------------------------------------------- metadata

#[test]
fn a_nested_secret_never_stringifies_into_a_scalar_field() {
    // `service_account` is a whole private key. If `metadata_string` rendered
    // objects, it would end up in `email`.
    let record = with_metadata(&[("email", json!({"private_key": "-----BEGIN"}))]);
    assert!(metadata_string(&record, "email").is_empty());
}

#[test]
fn presence_and_emptiness_are_different_questions() {
    assert!(!has_metadata(&record(), "api_key"));
    assert!(!has_metadata(
        &with_metadata(&[("api_key", json!(""))]),
        "api_key"
    ));
    assert!(!has_metadata(
        &with_metadata(&[("api_key", json!(null))]),
        "api_key"
    ));
    assert!(has_metadata(
        &with_metadata(&[("api_key", json!("sk-1"))]),
        "api_key"
    ));
    // A non-string value counts as present: a service account is an object.
    assert!(has_metadata(
        &with_metadata(&[("service_account", json!({"a": 1}))]),
        "service_account"
    ));
}

#[test]
fn a_missing_api_key_is_empty_rather_than_a_rendered_nil() {
    // 旧实现的 `fmt.Sprint(nil)` 产生字面量 "<nil>"，再遮蔽成 "<...>"，
    // 在控制台里看起来就像 OAuth 凭证带着一个 API key。钉死它不让这个产物回归。
    let record = record();
    assert!(api_key(&record).is_empty());
    let entry = serialize_pool_entry(&record, 0);
    assert_eq!(entry["api-key"], json!(""));
}

// ---------------------------------------------------------------- naming

#[test]
fn the_stable_name_prefers_the_operator_visible_label() {
    let mut record = record();
    record.label = "prod-key".to_owned();
    assert_eq!(stable_name(&record, 3), "prod-key");
}

#[test]
fn the_stable_name_falls_back_through_email_then_id() {
    let mut by_email = with_metadata(&[("email", json!("ops@example.test"))]);
    by_email.label = String::new();
    assert_eq!(stable_name(&by_email, 3), "ops@example.test");

    let mut by_id = record();
    by_id.label = String::new();
    assert_eq!(stable_name(&by_id, 3), by_id.id);
}

#[test]
fn the_stable_name_does_not_depend_on_position() {
    // `PUT`/`DELETE` on /auth-files address rows by this string; if it moved
    // when another credential was added, a delete would hit the wrong row.
    let mut record = record();
    record.label = String::new();
    assert_eq!(stable_name(&record, 0), stable_name(&record, 99));
}

#[test]
fn the_pool_display_name_does_depend_on_position() {
    // Different function, different job: the pool list numbers unnamed channels
    // for the operator's benefit.
    let mut record = record();
    record.label = String::new();
    assert_ne!(display_name(&record, 0), display_name(&record, 1));
}

// ---------------------------------------------------------------- time

#[test]
fn a_never_set_timestamp_renders_as_empty() {
    // 旧实现的零值是公元 1 年；Rust 实体把 NULL 解码成 epoch。
    // 两者都表示「从未」，都不该显示成一个日期。
    assert!(time_string(DateTime::UNIX_EPOCH).is_empty());
    assert!(opt_time_string(None).is_empty());
}

#[test]
fn a_real_timestamp_renders_as_rfc3339_utc() {
    let at = DateTime::from_timestamp(1_700_000_000, 0).expect("in range");
    let rendered = time_string(at);
    assert!(rendered.ends_with('Z'), "not UTC: {rendered}");
    assert_eq!(
        DateTime::parse_from_rfc3339(&rendered)
            .expect("round-trips")
            .timestamp(),
        at.timestamp()
    );
}

// ---------------------------------------------------------------- models

#[test]
fn models_merge_both_sources_without_duplicates() {
    let mut record = with_metadata(&[("models", json!(["b", "a"]))]);
    record.model_states = json!({"a": {}, "c": {}});
    assert_eq!(models(&record), ["a", "b", "c"]);
}

#[test]
fn a_comma_separated_model_list_is_accepted() {
    let record = with_metadata(&[("models", json!(" a , b ,, a "))]);
    assert_eq!(models(&record), ["a", "b"]);
}

#[test]
fn models_are_sorted_so_the_list_does_not_reshuffle() {
    let record = with_metadata(&[("models", json!(["z", "m", "a"]))]);
    let first = models(&record);
    let second = models(&record);
    assert_eq!(first, second);
    let mut sorted = first.clone();
    sorted.sort();
    assert_eq!(first, sorted);
}

// ---------------------------------------------------------------- quota

#[test]
fn quota_is_read_under_either_key_casing() {
    let mut snake = record();
    snake.quota = json!({"exceeded": true, "backoff_level": 3});
    let mut pascal = record();
    pascal.quota = json!({"Exceeded": true, "BackoffLevel": 3});

    for record in [&snake, &pascal] {
        assert!(quota_exceeded(record));
        assert_eq!(quota_backoff_level(record), 3);
    }
}

#[test]
fn an_absent_quota_block_is_not_exceeded() {
    // Defaulting the other way would bench every credential on a fresh install.
    assert!(!quota_exceeded(&record()));
    assert_eq!(quota_backoff_level(&record()), 0);
    assert!(quota_next_recover_at(&record()).is_empty());
}

// ---------------------------------------------------------------- payloads

#[test]
fn the_auth_file_row_never_carries_a_raw_secret() {
    // The single most important property in this file.
    let record = with_metadata(&[
        ("api_key", json!("sk-super-secret-value")),
        ("access_token", json!("at-super-secret-value")),
        ("refresh_token", json!("rt-super-secret-value")),
    ]);
    let body = serde_json::to_string(&serialize_auth_file(&record, 0)).expect("serializes");
    for secret in [
        "sk-super-secret-value",
        "at-super-secret-value",
        "rt-super-secret-value",
    ] {
        assert!(!body.contains(secret), "{secret} leaked into /auth-files");
    }
    assert!(body.contains("api_key_preview"));
}

#[test]
fn the_pool_entry_never_carries_a_raw_secret() {
    let record = with_metadata(&[("api_key", json!("sk-super-secret-value"))]);
    let body = serde_json::to_string(&serialize_pool_entry(&record, 0)).expect("serializes");
    assert!(!body.contains("sk-super-secret-value"));
}

#[test]
fn preview_keys_are_absent_rather_than_empty() {
    // Their absence is how the console knows there is no such secret at all.
    let body = serialize_auth_file(&record(), 0);
    let object = body.as_object().expect("object");
    for key in [
        "api_key_preview",
        "access_token_preview",
        "refresh_token_preview",
        "account_id",
        "chatgpt_account_id",
    ] {
        assert!(!object.contains_key(key), "{key} should be absent");
    }
}

#[test]
fn an_account_id_is_published_under_both_names() {
    let record = with_metadata(&[("account_id", json!("acct-1"))]);
    let body = serialize_auth_file(&record, 0);
    assert_eq!(body["account_id"], json!("acct-1"));
    assert_eq!(body["chatgpt_account_id"], json!("acct-1"));
}

#[test]
fn the_disabled_flag_covers_both_ways_of_being_disabled() {
    let mut by_flag = record();
    by_flag.disabled = true;
    let mut by_status = record();
    by_status.status = AuthStatus::Disabled;

    for record in [&by_flag, &by_status] {
        assert_eq!(serialize_pool_entry(record, 0)["disabled"], json!(true));
    }
    assert_eq!(serialize_pool_entry(&record(), 0)["disabled"], json!(false));
}

#[test]
fn the_quota_row_publishes_both_key_casings() {
    // Not a mistake in the port: 这里同时发 `exceeded` 和 `Exceeded`，因为
    // 两个不同的 UI 都对着这个端点写过。
    let mut record = record();
    record.quota = json!({"exceeded": true});
    let body = serialize_quota(&record, 0);
    assert_eq!(body["exceeded"], body["Exceeded"]);
    assert_eq!(body["next_recover_at"], body["NextRecoverAt"]);
}

#[test]
fn a_model_the_proxy_discovered_still_appears() {
    // A model that only ever showed up in `model_states` — because the proxy
    // was rejected for it — must still be listed, or an operator debugging a
    // dead model sees nothing at all.
    let mut record = record();
    record.model_states = json!({"gpt-4o": {"status": "error", "unavailable": true}});
    let rows = serialize_models(&record, 0);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["model"], json!("gpt-4o"));
}

#[test]
fn a_model_is_never_listed_twice() {
    // `models()` already unions the declared list with the state keys, so a
    // model present in both must not produce two rows.（这也意味着第二个
    // 遍历 `ModelStates` 的循环对任何非空白 key 实际都到不了——它要加的那行
    // 早就发出来了。）
    let mut record = with_metadata(&[("models", json!(["gpt-4o"]))]);
    record.model_states = json!({"gpt-4o": {"status": "error"}});
    let rows = serialize_models(&record, 0);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["status"], json!(record.status.as_str()));
}

#[test]
fn model_state_rows_are_ordered_deterministically() {
    let mut record = record();
    record.model_states = json!({"z": {}, "a": {}, "m": {}});
    let names: Vec<String> = serialize_models(&record, 0)
        .iter()
        .map(|row| row["model"].as_str().unwrap_or_default().to_owned())
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}
