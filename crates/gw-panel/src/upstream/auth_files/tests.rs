//! Unit tests for the credential inventory.
//!
//! The handlers need a store; the interesting logic does not. Pinned here:
//! which provider an uploaded file is filed under (getting it wrong routes
//! traffic to the wrong upstream), how the filters read, and how a batch target
//! list is flattened.

use super::*;
use chrono::DateTime;
use serde_json::json;

fn payload(raw: Value) -> Map<String, Value> {
    raw.as_object().cloned().expect("object literal")
}

fn upload(raw: Value) -> Result<AuthRecord, &'static str> {
    record_from_upload(
        "cred.json",
        raw.to_string().as_bytes(),
        DateTime::UNIX_EPOCH,
    )
}

// ---------------------------------------------------------------- provider

#[test]
fn a_google_service_account_is_filed_under_vertex() {
    // Recognised by shape before its `type: "service_account"` can be mistaken
    // for a provider name.
    let provider = provider_from_auth_json(&payload(json!({
        "type": "service_account",
        "private_key": "-----BEGIN PRIVATE KEY-----",
        "client_email": "svc@project.iam.gserviceaccount.com",
    })));
    assert_eq!(provider.as_deref(), Ok("vertex"));
}

#[test]
fn a_partial_service_account_is_not_a_service_account() {
    // Missing the key or the email means it cannot authenticate; filing it as
    // vertex would produce a credential that fails on first use.
    let provider = provider_from_auth_json(&payload(json!({
        "type": "service_account",
        "client_email": "svc@project.iam.gserviceaccount.com",
    })));
    assert_eq!(provider.as_deref(), Ok("service_account"));
}

#[test]
fn the_anthropic_alias_is_normalised_to_claude() {
    // Two spellings of one provider in `auth_records` would split the pool.
    assert_eq!(
        provider_from_auth_json(&payload(json!({"provider": "anthropic"}))).as_deref(),
        Ok("claude")
    );
}

#[test]
fn the_openai_compatible_spellings_all_normalise() {
    for spelling in [
        "openai-compatibility",
        "openai_compatibility",
        "openai-compatible",
    ] {
        assert_eq!(
            provider_from_auth_json(&payload(json!({"provider": spelling}))).as_deref(),
            Ok("openai")
        );
    }
}

#[test]
fn a_bare_api_key_file_defaults_to_openai() {
    for key in ["api_key", "api-key", "x-api-key"] {
        assert_eq!(
            provider_from_auth_json(&payload(json!({key: "sk-1"}))).as_deref(),
            Ok("openai")
        );
    }
}

#[test]
fn an_oauth_export_without_a_provider_is_rejected_with_its_own_message() {
    // The fix is different from the generic case — the operator has to say
    // which provider the token belongs to — so the message must differ too.
    let generic = provider_from_auth_json(&payload(json!({}))).expect_err("no provider");
    let oauth = provider_from_auth_json(&payload(json!({"access_token": "at-1"})))
        .expect_err("no provider");
    assert_ne!(generic, oauth);
    assert!(oauth.contains("OAuth"));
}

// ---------------------------------------------------------------- upload

#[test]
fn only_json_files_are_accepted() {
    let body = json!({"provider": "openai", "api_key": "sk-1"}).to_string();
    assert!(record_from_upload("cred.json", body.as_bytes(), DateTime::UNIX_EPOCH).is_ok());
    for name in ["cred.txt", "cred", "cred.json.bak"] {
        assert!(record_from_upload(name, body.as_bytes(), DateTime::UNIX_EPOCH).is_err());
    }
}

#[test]
fn the_json_suffix_check_is_case_insensitive() {
    let body = json!({"provider": "openai", "api_key": "sk-1"}).to_string();
    assert!(record_from_upload("CRED.JSON", body.as_bytes(), DateTime::UNIX_EPOCH).is_ok());
}

#[test]
fn a_file_over_the_size_cap_is_refused() {
    let huge = vec![b'x'; MAX_UPLOAD_BYTES + 1];
    assert!(record_from_upload("cred.json", &huge, DateTime::UNIX_EPOCH).is_err());
}

#[test]
fn malformed_json_is_refused_rather_than_stored_empty() {
    assert!(record_from_upload("cred.json", b"{not json", DateTime::UNIX_EPOCH).is_err());
    // A valid JSON scalar is not a credential either.
    assert!(record_from_upload("cred.json", b"\"text\"", DateTime::UNIX_EPOCH).is_err());
}

#[test]
fn an_unnamed_upload_is_labelled_from_its_filename() {
    let record = upload(json!({"provider": "openai", "api_key": "sk-1"})).expect("uploads");
    assert_eq!(record.label, "cred");
}

#[test]
fn a_named_upload_keeps_its_own_label() {
    let record =
        upload(json!({"provider": "openai", "api_key": "sk-1", "label": "prod"})).expect("uploads");
    assert_eq!(record.label, "prod");
}

#[test]
fn the_x_api_key_header_spelling_folds_into_api_key() {
    let record = upload(json!({"provider": "openai", "x-api-key": "sk-1"})).expect("uploads");
    assert_eq!(super::super::record::api_key(&record), "sk-1");
}

#[test]
fn a_claude_code_credentials_file_is_filed_under_claude() {
    let record = upload(json!({
        "claudeAiOauth": {
            "accessToken": "sk-ant-oat-test",
            "refreshToken": "sk-ant-ort-test",
            "expiresAt": 1_800_000_000_000_i64
        }
    }))
    .expect("claude code file");
    assert_eq!(record.provider, "claude");
    assert!(super::super::record::has_metadata(&record, "access_token"));
    assert!(super::super::record::has_metadata(&record, "refresh_token"));
}

#[test]
fn a_codex_cli_auth_file_is_filed_under_codex() {
    let record = upload(json!({
        "tokens": {
            "access_token": "codex-at",
            "refresh_token": "codex-rt",
            "id_token": "codex-id"
        },
        "last_refresh": "2026-01-01T00:00:00Z"
    }))
    .expect("codex cli file");
    assert_eq!(record.provider, "codex");
    assert!(super::super::record::has_metadata(&record, "access_token"));
    assert!(super::super::record::has_metadata(&record, "refresh_token"));
}

#[test]
fn the_same_cli_token_is_not_imported_twice() {
    let first = upload(json!({
        "claudeAiOauth": {
            "accessToken": "same-at",
            "refreshToken": "same-rt"
        }
    }))
    .expect("first");
    let cred = gw_provider::local_oauth::LocalOauthCred {
        provider: "claude",
        source: std::path::PathBuf::from("/tmp/.claude/.credentials.json"),
        access_token: "same-at".to_owned(),
        refresh_token: "same-rt".to_owned(),
        id_token: String::new(),
        expires_at: String::new(),
        email: String::new(),
    };
    assert!(already_imported(&[first], &cred));
}

#[test]
fn nested_oauth_tokens_are_lifted_to_the_top_level() {
    // The executors read the flat keys; leaving them nested makes a valid
    // credential look like it has no token at all.
    let record = upload(json!({
        "provider": "claude",
        "token_data": {"access_token": "at-1", "refresh_token": "rt-1", "email": "a@b.test"},
    }))
    .expect("uploads");

    assert!(super::super::record::has_metadata(&record, "access_token"));
    assert!(super::super::record::has_metadata(&record, "refresh_token"));
    assert_eq!(
        super::super::record::metadata_string(&record, "email"),
        "a@b.test"
    );
}

#[test]
fn a_top_level_token_is_not_overwritten_by_the_nested_one() {
    let record = upload(json!({
        "provider": "claude",
        "access_token": "flat",
        "token_data": {"access_token": "nested"},
    }))
    .expect("uploads");
    assert_eq!(
        super::super::record::metadata_string(&record, "access_token"),
        "flat"
    );
}

#[test]
fn a_service_account_file_keeps_the_whole_document() {
    // The private key lives inside; flattening it to a string would break the
    // credential.
    let record = upload(json!({
        "type": "service_account",
        "private_key": "-----BEGIN PRIVATE KEY-----",
        "client_email": "svc@project.iam.gserviceaccount.com",
        "project_id": "proj-1",
    }))
    .expect("uploads");

    assert_eq!(record.provider, "vertex");
    assert!(matches!(
        super::super::record::metadata(&record, "service_account"),
        Some(Value::Object(_))
    ));
    assert_eq!(record.attribute("project_id"), Some("proj-1"));
}

#[test]
fn the_proxy_and_prefix_fields_reach_their_columns() {
    // They are attributes *and* columns; writing only one leaves the routing
    // layer reading a stale value.
    let record = upload(json!({
        "provider": "openai",
        "api_key": "sk-1",
        "proxy-url": "http://proxy",
        "prefix": "eu",
    }))
    .expect("uploads");

    assert_eq!(record.proxy_url, "http://proxy");
    assert_eq!(record.prefix, "eu");
    assert_eq!(record.attribute("proxy_url"), Some("http://proxy"));
}

// ---------------------------------------------------------------- filters

fn stored(provider: &str, label: &str, disabled: bool) -> AuthRecord {
    let mut record = AuthRecord::new("auth-1", provider, DateTime::UNIX_EPOCH);
    record.label = label.to_owned();
    record.disabled = disabled;
    if disabled {
        record.status = AuthStatus::Disabled;
    }
    record
}

#[test]
fn an_empty_query_matches_everything() {
    assert!(matches_query(
        &stored("openai", "prod", false),
        &AuthFileQuery::default(),
        0
    ));
}

#[test]
fn the_provider_filter_is_case_insensitive() {
    let record = stored("openai", "prod", false);
    for spelling in ["openai", "OpenAI", " OPENAI "] {
        let query = AuthFileQuery {
            provider: Some(spelling.to_owned()),
            ..AuthFileQuery::default()
        };
        assert!(matches_query(&record, &query, 0));
    }
}

#[test]
fn a_malformed_disabled_filter_matches_nothing() {
    // 解析失败时返回 false。An operator who typed `disabled=yes`
    // is better served by an empty list than by a silently unfiltered one.
    let query = AuthFileQuery {
        disabled: Some("yes".to_owned()),
        ..AuthFileQuery::default()
    };
    assert!(!matches_query(&stored("openai", "prod", true), &query, 0));
    assert!(!matches_query(&stored("openai", "prod", false), &query, 0));
}

#[test]
fn the_disabled_filter_accepts_boolean_spellings() {
    for (raw, want_disabled) in [("1", true), ("true", true), ("0", false), ("f", false)] {
        let query = AuthFileQuery {
            disabled: Some(raw.to_owned()),
            ..AuthFileQuery::default()
        };
        assert!(matches_query(
            &stored("openai", "p", want_disabled),
            &query,
            0
        ));
        assert!(!matches_query(
            &stored("openai", "p", !want_disabled),
            &query,
            0
        ));
    }
}

#[test]
fn free_text_search_reaches_the_fields_an_operator_would_type() {
    let mut record = stored("openai", "prod-eu", false);
    record
        .metadata
        .as_object_mut()
        .expect("object")
        .insert("email".to_owned(), json!("ops@example.test"));
    record.set_attribute("base_url", "https://eu.example.test");

    for needle in ["prod", "OPS@EXAMPLE", "eu.example", "openai", "auth-1"] {
        let query = AuthFileQuery {
            q: Some(needle.to_owned()),
            ..AuthFileQuery::default()
        };
        assert!(matches_query(&record, &query, 0), "{needle} did not match");
    }
}

#[test]
fn free_text_search_does_not_match_across_field_boundaries() {
    // The fields are joined with newlines; joining with nothing would let
    // "prodopenai" match, which is a false positive an operator cannot explain.
    let record = stored("openai", "prod", false);
    let query = AuthFileQuery {
        q: Some("prodopenai".to_owned()),
        ..AuthFileQuery::default()
    };
    assert!(!matches_query(&record, &query, 0));
}

// ---------------------------------------------------------------- targets

#[test]
fn a_batch_target_list_accepts_scalars_and_arrays() {
    let targets = payload_string_slice(
        &payload(json!({"names": ["a", "b"], "id": "c"})),
        &["names", "id"],
    );
    assert_eq!(targets, ["a", "b", "c"]);
}

#[test]
fn duplicate_and_blank_targets_are_dropped() {
    let targets = payload_string_slice(
        &payload(json!({"names": ["a", "a", "", "  "], "name": "a"})),
        &["names", "name"],
    );
    assert_eq!(targets, ["a"]);
}

#[test]
fn a_null_target_is_not_the_string_nil() {
    // 旧实现的 `fmt.Sprint(nil)` 是 "<nil>"，再按名字过滤掉。
    // 这里 null 干脆就不是标量。
    let targets = payload_string_slice(&payload(json!({"names": [null, "a"]})), &["names"]);
    assert_eq!(targets, ["a"]);
}

#[test]
fn a_numeric_target_is_accepted() {
    // Credential ids are strings, but the console has sent numbers for
    // positional targets before.
    let targets = payload_string_slice(&payload(json!({"ids": [1, 2]})), &["ids"]);
    assert_eq!(targets, ["1", "2"]);
}

#[test]
fn a_target_resolves_by_id_label_or_display_name() {
    let mut unlabelled = AuthRecord::new("auth-2", "openai", DateTime::UNIX_EPOCH);
    unlabelled.label = String::new();
    let records = vec![stored("openai", "prod", false), unlabelled];

    assert_eq!(
        find_by_name(&records, "prod").map(|r| r.id.as_str()),
        Some("auth-1")
    );
    assert_eq!(
        find_by_name(&records, "auth-2").map(|r| r.id.as_str()),
        Some("auth-2")
    );
    assert!(find_by_name(&records, "").is_none());
    assert!(find_by_name(&records, "nope").is_none());
}

#[test]
fn a_tombstoned_credential_is_not_a_valid_target() {
    let mut deleted = stored("openai", "prod", true);
    deleted.set_attribute(super::super::record::DELETED_ATTRIBUTE, "true");
    assert!(find_by_name(&[deleted], "prod").is_none());
}
