//! Hashing and JWT verification.

use super::*;

#[test]
fn an_api_key_hash_is_stable_and_collision_free_across_keys() {
    let crypto = AuthcoreCrypto::new("");
    assert_eq!(
        crypto.hash_api_key("agw-abc"),
        crypto.hash_api_key("agw-abc"),
        "the same key must always resolve to the same row",
    );
    assert_ne!(
        crypto.hash_api_key("agw-abc"),
        crypto.hash_api_key("agw-abd")
    );
}

#[test]
fn the_hash_is_the_lowercase_hex_digest_the_column_stores() {
    // 32 bytes of SHA-256 as lowercase hex. A different encoding here silently
    // stops matching the `api_keys.key_hash` values already persisted.
    let digest = AuthcoreCrypto::new("").hash_api_key("agw-abc");
    assert_eq!(digest.len(), 64, "got {digest}");
    assert!(
        digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
}

#[test]
fn the_idempotency_scope_digest_shares_the_key_hash_primitive() {
    let crypto = AuthcoreCrypto::new("");
    assert_eq!(
        crypto.sha256_hex("7\0POST\0/v1/messages\0k"),
        crypto.hash_api_key("7\0POST\0/v1/messages\0k")
    );
    assert_ne!(crypto.sha256_hex("a"), crypto.sha256_hex("b"));
}

#[test]
fn a_token_this_secret_signed_verifies_and_carries_its_subject() {
    let secret = "0123456789abcdef0123456789abcdef";
    let token = gw_authcore::generate_jwt(42, "user@example.test", secret, 1)
        .expect("the secret is long enough to sign with");

    let claims = AuthcoreCrypto::new(secret)
        .verify_jwt(&token)
        .expect("a token we just signed must verify");
    assert_eq!(claims.user_id, 42);
}

#[test]
fn a_token_signed_by_a_different_secret_is_rejected() {
    let token = gw_authcore::generate_jwt(
        42,
        "user@example.test",
        "0123456789abcdef0123456789abcdef",
        1,
    )
    .expect("signs");
    assert!(
        AuthcoreCrypto::new("fedcba9876543210fedcba9876543210")
            .verify_jwt(&token)
            .is_none(),
    );
}

#[test]
fn an_empty_secret_verifies_nothing_rather_than_accepting_everything() {
    // Fail closed: a misconfigured deployment must reject tokens, not honour
    // unsigned ones.
    let token = gw_authcore::generate_jwt(
        42,
        "user@example.test",
        "0123456789abcdef0123456789abcdef",
        1,
    )
    .expect("signs");
    assert!(AuthcoreCrypto::new("").verify_jwt(&token).is_none());
    assert!(AuthcoreCrypto::new("").verify_jwt("").is_none());
}

#[test]
fn garbage_is_rejected_without_panicking() {
    let crypto = AuthcoreCrypto::new("0123456789abcdef0123456789abcdef");
    for token in ["", "..", "not-a-jwt", "a.b.c", "\u{feff}"] {
        assert!(crypto.verify_jwt(token).is_none(), "accepted {token:?}");
    }
}
