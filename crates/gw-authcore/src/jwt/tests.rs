//! The golden tokens below were produced **outside this crate** (a stdlib-only
//! Python HMAC-SHA256 script following RFC 7519), so they check interoperability
//! with an independent issuer rather than restating our own implementation.

use super::{
    Claims, DEFAULT_EXPIRY_HOURS, generate_jwt, generate_jwt_with_version, sign_at, validate_jwt,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{TimeDelta, Utc};
use jsonwebtoken::errors::ErrorKind;

const GOLDEN_SECRET: &str = "golden-jwt-secret";

/// `{"user_id":42,"email":"golden@example.com","tv":3,"exp":4102444800,
///   "iat":1700000000,"nbf":1700000000,"iss":"ai-gateway","sub":"42"}`
const GOLDEN_TOKEN: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJ1c2VyX2lkIjo0MiwiZW1haWwiOiJnb2xkZW5AZXhhbXBsZS5jb20iLCJ0diI6MywiZXhwIjo0MTAyNDQ0ODAwLCJpYXQiOjE3MDAwMDAwMDAsIm5iZiI6MTcwMDAwMDAwMCwiaXNzIjoiYWktZ2F0ZXdheSIsInN1YiI6IjQyIn0.HBbZDnFfyTWGUPjqlVU51uMJdidC_abSrEjYXTB7nI8";

/// Same shape, but minted before the `tv` claim existed (user 7).
const GOLDEN_TOKEN_WITHOUT_TV: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJ1c2VyX2lkIjo3LCJlbWFpbCI6ImxlZ2FjeUBleGFtcGxlLmNvbSIsImV4cCI6NDEwMjQ0NDgwMCwiaWF0IjoxNzAwMDAwMDAwLCJuYmYiOjE3MDAwMDAwMDAsImlzcyI6ImFpLWdhdGV3YXkiLCJzdWIiOiI3In0.3yRa5DTKw3rQqAdEZYbXhlpiUFtai54eM7goW8Bo0e0";

/// Well-formed, correctly signed, but `exp` is 2023-11-14.
const GOLDEN_TOKEN_EXPIRED: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJ1c2VyX2lkIjo0MiwiZW1haWwiOiJnb2xkZW5AZXhhbXBsZS5jb20iLCJ0diI6MCwiZXhwIjoxNzAwMDAzNjAwLCJpYXQiOjE3MDAwMDAwMDAsIm5iZiI6MTcwMDAwMDAwMCwiaXNzIjoiYWktZ2F0ZXdheSIsInN1YiI6IjQyIn0.eX84byXc2sdNZmGxCqC7rdZ2wMcCCj5PDk5VUBCpCvM";

/// The unsigned `alg: none` variant of [`GOLDEN_TOKEN`].
const GOLDEN_TOKEN_ALG_NONE: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.eyJ1c2VyX2lkIjo0MiwiZW1haWwiOiJnb2xkZW5AZXhhbXBsZS5jb20iLCJ0diI6MywiZXhwIjo0MTAyNDQ0ODAwLCJpYXQiOjE3MDAwMDAwMDAsIm5iZiI6MTcwMDAwMDAwMCwiaXNzIjoiYWktZ2F0ZXdheSIsInN1YiI6IjQyIn0.";

fn payload_json(token: &str) -> serde_json::Value {
    let segment = token
        .split('.')
        .nth(1)
        .expect("token has a payload segment");
    let raw = URL_SAFE_NO_PAD
        .decode(segment)
        .expect("payload is base64url");
    serde_json::from_slice(&raw).expect("payload is JSON")
}

#[test]
fn token_issued_elsewhere_validates_here() {
    let claims = validate_jwt(GOLDEN_TOKEN, GOLDEN_SECRET).expect("golden token validates");

    assert_eq!(claims.user_id, 42);
    assert_eq!(claims.email, "golden@example.com");
    assert_eq!(claims.token_version, 3);
    assert_eq!(claims.sub.as_deref(), Some("42"));
    assert_eq!(claims.iss.as_deref(), Some("ai-gateway"));
}

#[test]
fn token_minted_before_the_tv_claim_decodes_as_version_zero() {
    let claims = validate_jwt(GOLDEN_TOKEN_WITHOUT_TV, GOLDEN_SECRET).expect("legacy token");

    assert_eq!(claims.user_id, 7);
    assert_eq!(claims.token_version, 0);
    assert!(
        !claims.is_revoked(0),
        "never-revoked user keeps their session"
    );
}

#[test]
fn expired_token_is_rejected_without_leeway() {
    let err = validate_jwt(GOLDEN_TOKEN_EXPIRED, GOLDEN_SECRET).expect_err("exp is in the past");

    assert!(
        matches!(&err, crate::AuthError::InvalidJwt(e) if matches!(e.kind(), ErrorKind::ExpiredSignature)),
        "expected an expiry failure, got {err}"
    );
}

#[test]
fn unsigned_alg_none_token_is_rejected() {
    validate_jwt(GOLDEN_TOKEN_ALG_NONE, GOLDEN_SECRET)
        .expect_err("algorithm confusion must not authenticate anybody");
}

#[test]
fn signature_is_bound_to_the_secret() {
    validate_jwt(GOLDEN_TOKEN, "some-other-secret").expect_err("wrong secret must not validate");
}

#[test]
fn roundtrip_preserves_identity_and_version() {
    let token = generate_jwt_with_version(99, "user@example.com", "s3cret", 1, 7)
        .expect("issuing succeeds");
    let claims = validate_jwt(&token, "s3cret").expect("our own token validates");

    assert_eq!(claims.user_id, 99);
    assert_eq!(claims.email, "user@example.com");
    assert_eq!(claims.token_version, 7);
    assert_eq!(claims.sub.as_deref(), Some("99"));
}

#[test]
fn version_zero_is_omitted_from_the_wire_like_gos_omitempty() {
    let with_version =
        generate_jwt_with_version(1, "a@b.c", "s3cret", 1, 4).expect("issuing succeeds");
    let without_version = generate_jwt(1, "a@b.c", "s3cret", 1).expect("issuing succeeds");

    assert_eq!(payload_json(&with_version)["tv"], 4);
    assert!(
        payload_json(&without_version).get("tv").is_none(),
        "tv must be absent (not 0) so omitempty round-trips byte-for-byte"
    );
}

#[test]
fn positive_expiry_hours_are_honoured_exactly() {
    for hours in [1_i64, 2, 24, 168, 720] {
        let token = generate_jwt(1, "a@b.c", "s3cret", hours).expect("issuing succeeds");
        let claims: Claims =
            serde_json::from_value(payload_json(&token)).expect("payload decodes as Claims");

        assert_eq!(
            claims.exp - claims.iat,
            hours * 3600,
            "exp-iat must equal the requested window for {hours}h"
        );
    }
}

#[test]
fn non_positive_expiry_hours_fall_back_to_the_default_window() {
    for hours in [0_i64, -1, -720] {
        let token = generate_jwt(1, "a@b.c", "s3cret", hours).expect("issuing succeeds");
        let claims: Claims =
            serde_json::from_value(payload_json(&token)).expect("payload decodes as Claims");

        assert_eq!(claims.exp - claims.iat, DEFAULT_EXPIRY_HOURS * 3600);
    }
}

#[test]
fn a_token_whose_window_already_closed_is_rejected() {
    let long_ago = Utc::now() - TimeDelta::try_days(30).expect("30 days is a valid delta");
    let token = sign_at(long_ago, 1, "a@b.c", "s3cret", 1, 0).expect("issuing succeeds");

    validate_jwt(&token, "s3cret").expect_err("a token that expired 30 days ago must not validate");
}

#[test]
fn a_token_that_is_not_yet_valid_is_rejected() {
    let tomorrow = Utc::now() + TimeDelta::try_days(1).expect("one day is a valid delta");
    let token = sign_at(tomorrow, 1, "a@b.c", "s3cret", 48, 0).expect("issuing succeeds");

    let err = validate_jwt(&token, "s3cret").expect_err("nbf is in the future");
    assert!(
        matches!(&err, crate::AuthError::InvalidJwt(e) if matches!(e.kind(), ErrorKind::ImmatureSignature)),
        "expected a not-before failure, got {err}"
    );
}

#[test]
fn an_unconfigured_secret_fails_closed_on_both_sides() {
    assert!(matches!(
        generate_jwt(1, "a@b.c", "", 1),
        Err(crate::AuthError::MissingJwtSecret)
    ));
    assert!(matches!(
        validate_jwt(GOLDEN_TOKEN, ""),
        Err(crate::AuthError::MissingJwtSecret)
    ));
}

#[test]
fn revocation_rejects_only_tokens_older_than_the_current_epoch() {
    let claims = validate_jwt(GOLDEN_TOKEN, GOLDEN_SECRET).expect("golden token validates");

    assert!(!claims.is_revoked(claims.token_version - 1));
    assert!(!claims.is_revoked(claims.token_version));
    assert!(claims.is_revoked(claims.token_version + 1));
}
