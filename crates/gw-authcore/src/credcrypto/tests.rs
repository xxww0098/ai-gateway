//! `REF_ENVELOPE` was sealed by an independent AES-GCM implementation, with a
//! fixed nonce so the vector is reproducible. If this test ever fails, every
//! upstream credential in the existing database has become unreadable.

use super::{CRED_ENC_ENVELOPE_KEY, CredentialCipher};
use crate::AuthError;
use serde_json::{Value, json};

const KEY_HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const KEY_BASE64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const OTHER_KEY_HEX: &str = "ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100";
const REF_ENVELOPE_B64: &str = "AAECAwQFBgcICQoLPCC3eKaAsWjSNfjg1IdaV6Gx6FiUHjFRTAiO4HNLLJBgYMejxKRruk6GDIal4EdUijwOrycDvYuYOc+/gc8iEJdCrLRw";

fn ref_envelope() -> Value {
    json!({ CRED_ENC_ENVELOPE_KEY: REF_ENVELOPE_B64 })
}

fn ref_plaintext() -> Value {
    json!({"access_token": "golden-token", "api_key": "sk-golden"})
}

#[test]
fn credentials_sealed_by_an_older_binary_still_decrypt() {
    let cipher = CredentialCipher::new(KEY_HEX).expect("hex key is accepted");

    assert_eq!(
        cipher.decrypt(&ref_envelope()).expect("the envelope opens"),
        ref_plaintext()
    );
}

#[test]
fn the_same_key_may_be_supplied_as_hex_or_base64() {
    let from_hex = CredentialCipher::new(KEY_HEX).expect("hex key is accepted");
    let from_base64 = CredentialCipher::new(KEY_BASE64).expect("base64 key is accepted");

    assert_eq!(
        from_base64.decrypt(&ref_envelope()).expect("opens"),
        from_hex.decrypt(&ref_envelope()).expect("opens"),
    );
}

#[test]
fn round_trip_returns_the_original_metadata() {
    let cipher = CredentialCipher::new(KEY_HEX).expect("hex key is accepted");
    let metadata = json!({
        "api_key": "sk-live-secret",
        "token_data": {"access_token": "at", "expires_at": 1_700_000_000_i64},
        "scopes": ["a", "b"],
    });

    let sealed = cipher.encrypt(&metadata).expect("sealing succeeds");
    assert_eq!(cipher.decrypt(&sealed).expect("opening succeeds"), metadata);
}

#[test]
fn the_sealed_row_does_not_contain_the_secret() {
    let cipher = CredentialCipher::new(KEY_HEX).expect("hex key is accepted");
    let sealed = cipher
        .encrypt(&json!({"api_key": "sk-live-secret"}))
        .expect("sealing succeeds");

    let as_text = sealed.to_string();
    assert!(!as_text.contains("sk-live-secret"));
    assert!(!as_text.contains("api_key"));
    assert!(sealed.get(CRED_ENC_ENVELOPE_KEY).is_some());
}

#[test]
fn each_seal_uses_a_fresh_nonce() {
    let cipher = CredentialCipher::new(KEY_HEX).expect("hex key is accepted");
    let metadata = json!({"api_key": "sk-live-secret"});

    let first = cipher.encrypt(&metadata).expect("sealing succeeds");
    let second = cipher.encrypt(&metadata).expect("sealing succeeds");

    assert_ne!(
        first, second,
        "a reused nonce would leak plaintext relationships across rows"
    );
}

#[test]
fn the_wrong_key_fails_loudly_instead_of_returning_garbage() {
    let cipher = CredentialCipher::new(OTHER_KEY_HEX).expect("hex key is accepted");

    assert!(matches!(
        cipher.decrypt(&ref_envelope()),
        Err(AuthError::CredentialDecrypt)
    ));
}

#[test]
fn an_encrypted_row_without_a_configured_key_is_refused() {
    let disabled = CredentialCipher::new("").expect("an empty key disables encryption");

    assert!(!disabled.enabled());
    assert!(matches!(
        disabled.decrypt(&ref_envelope()),
        Err(AuthError::CredentialKeyMissing)
    ));
}

#[test]
fn legacy_plaintext_rows_pass_through_unchanged() {
    let enabled = CredentialCipher::new(KEY_HEX).expect("hex key is accepted");
    let disabled = CredentialCipher::new("   ").expect("a blank key disables encryption");
    let plaintext = json!({"api_key": "sk-legacy"});

    assert_eq!(enabled.decrypt(&plaintext).expect("passthrough"), plaintext);
    assert_eq!(
        disabled.decrypt(&plaintext).expect("passthrough"),
        plaintext
    );
    assert_eq!(
        disabled.encrypt(&plaintext).expect("passthrough"),
        plaintext
    );
}

#[test]
fn a_row_that_merely_mentions_the_envelope_key_is_not_an_envelope() {
    let cipher = CredentialCipher::new(KEY_HEX).expect("hex key is accepted");
    let lookalike = json!({CRED_ENC_ENVELOPE_KEY: REF_ENVELOPE_B64, "api_key": "sk-plain"});

    assert_eq!(cipher.decrypt(&lookalike).expect("passthrough"), lookalike);
}

#[test]
fn nothing_secret_means_nothing_to_encrypt() {
    let cipher = CredentialCipher::new(KEY_HEX).expect("hex key is accepted");

    assert_eq!(
        cipher.encrypt(&Value::Null).expect("passthrough"),
        Value::Null
    );
    assert_eq!(cipher.encrypt(&json!({})).expect("passthrough"), json!({}));
}

#[test]
fn a_misconfigured_key_is_rejected_at_construction() {
    assert!(matches!(
        CredentialCipher::new("00112233"),
        Err(AuthError::CredentialKeyLength(4))
    ));
    assert!(matches!(
        CredentialCipher::new("this is not a key!"),
        Err(AuthError::CredentialKeyEncoding)
    ));
}

#[test]
fn a_corrupt_envelope_is_reported_rather_than_ignored() {
    let cipher = CredentialCipher::new(KEY_HEX).expect("hex key is accepted");

    assert!(matches!(
        cipher.decrypt(&json!({CRED_ENC_ENVELOPE_KEY: "!!!not base64!!!"})),
        Err(AuthError::CredentialEnvelopeDecode(_))
    ));
    assert!(matches!(
        cipher.decrypt(&json!({CRED_ENC_ENVELOPE_KEY: "AAEC"})),
        Err(AuthError::CredentialEnvelopeTooShort)
    ));
    assert!(matches!(
        cipher.decrypt(&json!({CRED_ENC_ENVELOPE_KEY: &REF_ENVELOPE_B64[..40]})),
        Err(AuthError::CredentialDecrypt)
    ));
}
