//! Unit tests for the token exchange's pure parts.
//!
//! The three network calls are exercised by the integration suite. Pinned here:
//! parsing a token body, reading an `id_token`, and everything that ends up in
//! the stored credential.

use super::*;

fn body(raw: Value) -> Map<String, Value> {
    raw.as_object().cloned().expect("object literal")
}

fn tokens() -> TokenResponse {
    TokenResponse {
        access_token: "at-1".to_owned(),
        refresh_token: "rt-1".to_owned(),
        id_token: "it-1".to_owned(),
        token_type: "Bearer".to_owned(),
        expires_in: 3600,
        raw: Map::new(),
        email: "ops@example.test".to_owned(),
        account_id: "acct-1".to_owned(),
    }
}

// ---------------------------------------------------------------- claude code

#[test]
fn a_claude_code_carries_its_own_state_after_a_hash() {
    let (code, state) = split_claude_code("abc#state-9");
    assert_eq!(code, "abc");
    assert_eq!(state, "state-9");
}

#[test]
fn a_plain_code_has_no_embedded_state() {
    let (code, state) = split_claude_code("  abc  ");
    assert_eq!(code, "abc");
    assert!(state.is_empty());
}

#[test]
fn only_the_first_hash_splits_the_code() {
    let (code, state) = split_claude_code("abc#s#t");
    assert_eq!(code, "abc");
    assert_eq!(state, "s#t");
}

// ---------------------------------------------------------------- token body

#[test]
fn a_string_expires_in_is_accepted() {
    // Some providers send it as a string; dropping it would make the credential
    // look non-expiring and never get refreshed.
    let numeric = parse_token_body(body(json!({"access_token": "at", "expires_in": 3600})));
    let textual = parse_token_body(body(json!({"access_token": "at", "expires_in": "3600"})));
    assert_eq!(numeric.expires_in, textual.expires_in);
    assert_eq!(numeric.expires_in, 3600);
}

#[test]
fn an_absent_expires_in_is_zero_rather_than_a_guess() {
    let parsed = parse_token_body(body(json!({"access_token": "at"})));
    assert_eq!(parsed.expires_in, 0);
}

#[test]
fn the_whole_token_body_is_kept() {
    // Claude's account email is in a nested object the typed fields miss.
    let parsed = parse_token_body(body(
        json!({"access_token": "at", "account": {"email_address": "a@b.test"}}),
    ));
    assert!(parsed.raw.contains_key("account"));
}

#[test]
fn a_body_with_no_tokens_parses_to_empty_strings() {
    // The caller decides what to do about it; panicking on a provider error
    // body would lose the status code that explains the failure.
    let parsed = parse_token_body(body(json!({"error": "invalid_grant"})));
    assert!(parsed.access_token.is_empty());
    assert!(parsed.raw.contains_key("error"));
}

// ---------------------------------------------------------------- id_token

#[test]
fn jwt_claims_are_read_without_verifying_the_signature() {
    // Deliberate: the token came straight back from the provider's token
    // endpoint over TLS and nothing is authorized on these fields.
    let payload = URL_SAFE_NO_PAD.encode(
        json!({
            "email": "a@b.test",
            "https://api.openai.com/auth": {"account_id": "acct-1"},
        })
        .to_string(),
    );
    let (email, account_id) = claims_from_jwt(&format!("header.{payload}.signature"));
    assert_eq!(email, "a@b.test");
    assert_eq!(account_id, "acct-1");
}

#[test]
fn jwt_claims_fall_back_through_account_id_then_sub() {
    let by_account = URL_SAFE_NO_PAD.encode(json!({"account_id": "acct-2"}).to_string());
    assert_eq!(claims_from_jwt(&format!("h.{by_account}.s")).1, "acct-2");

    let by_sub = URL_SAFE_NO_PAD.encode(json!({"sub": "user-1"}).to_string());
    assert_eq!(claims_from_jwt(&format!("h.{by_sub}.s")).1, "user-1");
}

#[test]
fn the_nested_account_id_outranks_the_flat_one() {
    let payload = URL_SAFE_NO_PAD.encode(
        json!({
            "account_id": "flat",
            "https://api.openai.com/auth": {"account_id": "nested"},
        })
        .to_string(),
    );
    assert_eq!(claims_from_jwt(&format!("h.{payload}.s")).1, "nested");
}

#[test]
fn a_malformed_id_token_yields_empty_claims_rather_than_panicking() {
    for token in ["", "onlyonepart", "a.!!!not-base64!!!.c", "a.e30.c"] {
        let (email, account_id) = claims_from_jwt(token);
        assert!(email.is_empty() && account_id.is_empty(), "{token}");
    }
}

// ---------------------------------------------------------------- record

#[test]
fn an_oauth_credential_is_marked_as_one() {
    let record = oauth_record(Provider::Claude, &tokens(), Utc::now());
    assert_eq!(record.attribute("oauth"), Some("true"));
    assert_eq!(record.provider, Provider::Claude.as_str());
    assert!(record.last_refreshed_at.is_some());
}

#[test]
fn the_tokens_are_stored_flat_and_nested() {
    // The executors read the flat keys; an exported auth file carries the
    // nested shape. Both are needed for export/re-import to round-trip.
    let record = oauth_record(Provider::Codex, &tokens(), Utc::now());
    let metadata = record.metadata.as_object().expect("object");
    let nested = metadata
        .get("token_data")
        .and_then(Value::as_object)
        .expect("token_data");

    for key in ["access_token", "refresh_token", "id_token"] {
        assert_eq!(metadata.get(key), nested.get(key), "{key} disagrees");
    }
}

#[test]
fn an_absent_refresh_token_is_omitted_rather_than_stored_empty() {
    // `has_refresh_token` drives a console action; an empty string would offer
    // a rotate button for a token that does not exist.
    let record = oauth_record(
        Provider::Claude,
        &TokenResponse {
            refresh_token: String::new(),
            ..tokens()
        },
        Utc::now(),
    );
    assert!(
        !record
            .metadata
            .as_object()
            .expect("object")
            .contains_key("refresh_token")
    );
}

#[test]
fn a_lifetime_free_token_gets_no_expiry() {
    // A wrong expiry would make the refresher discard a working token.
    let record = oauth_record(
        Provider::Claude,
        &TokenResponse {
            expires_in: 0,
            ..tokens()
        },
        Utc::now(),
    );
    let metadata = record.metadata.as_object().expect("object");
    assert!(!metadata.contains_key("expires_at"));
    assert!(!metadata.contains_key("expired"));
}

#[test]
fn an_expiry_is_published_under_both_names() {
    let record = oauth_record(Provider::Claude, &tokens(), Utc::now());
    let metadata = record.metadata.as_object().expect("object");
    assert_eq!(metadata.get("expires_at"), metadata.get("expired"));
    assert!(metadata.contains_key("expires_at"));
}

#[test]
fn the_expiry_is_in_the_future() {
    let now = Utc::now();
    let record = oauth_record(Provider::Claude, &tokens(), now);
    let expires_at = record
        .metadata
        .get("expires_at")
        .and_then(Value::as_str)
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .expect("an rfc3339 expiry");
    assert!(expires_at.with_timezone(&Utc) > now);
}

#[test]
fn the_label_names_the_account_when_there_is_one() {
    let named = oauth_record(Provider::Gemini, &tokens(), Utc::now());
    assert!(named.label.contains("ops@example.test"));

    let anonymous = oauth_record(
        Provider::Gemini,
        &TokenResponse {
            email: String::new(),
            ..tokens()
        },
        Utc::now(),
    );
    assert!(!anonymous.label.is_empty());
    assert!(!anonymous.label.contains('('));
}

#[test]
fn only_gemini_gets_the_google_token_blob() {
    let gemini = oauth_record(Provider::Gemini, &tokens(), Utc::now());
    assert!(
        gemini
            .metadata
            .as_object()
            .expect("object")
            .contains_key("token")
    );

    for provider in [Provider::Claude, Provider::Codex] {
        let record = oauth_record(provider, &tokens(), Utc::now());
        assert!(
            !record
                .metadata
                .as_object()
                .expect("object")
                .contains_key("token")
        );
    }
}

#[test]
fn the_google_token_blob_lists_the_scopes_as_an_array() {
    // A Google credential file carries `scopes` as a list; a single
    // space-joined string is rejected by the client libraries that read it.
    let record = oauth_record(Provider::Gemini, &tokens(), Utc::now());
    let scopes = record
        .metadata
        .get("token")
        .and_then(|token| token.get("scopes"))
        .and_then(Value::as_array)
        .expect("scopes array");
    assert!(scopes.len() > 1);
}

#[test]
fn two_oauth_credentials_never_share_an_id() {
    let first = oauth_record(Provider::Claude, &tokens(), Utc::now());
    let second = oauth_record(Provider::Claude, &tokens(), Utc::now());
    assert_ne!(first.id, second.id);
}
