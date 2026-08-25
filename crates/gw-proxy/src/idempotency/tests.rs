//! Deduplication: the claim must be exclusive, and entries must never be
//! shareable across tenants.

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use http_body_util::BodyExt;

use super::*;

/// A scope from a literal, for the tests that key on a fixed name.
fn scope(raw: &str) -> IdempotencyScope {
    IdempotencyScope::new(raw)
}
use crate::testsupport::{FakeCrypto, FakeIdempotencyStore};

fn manager() -> (IdempotencyManager, Arc<FakeIdempotencyStore>) {
    let store = FakeIdempotencyStore::shared();
    (
        IdempotencyManager::new(store.clone(), FakeCrypto::shared(), Duration::ZERO),
        store,
    )
}

#[test]
fn a_request_without_a_client_key_opts_out_entirely() {
    let (manager, _) = manager();
    assert_eq!(
        manager
            .scoped_key(1, "POST", "/v1/messages", "")
            .as_str()
            .to_owned(),
        ""
    );
    assert_eq!(
        manager
            .scoped_key(1, "POST", "/v1/messages", "   ")
            .as_str()
            .to_owned(),
        ""
    );
}

#[test]
fn the_same_client_key_never_collides_across_tenants_methods_or_paths() {
    // Blocker B6: two tenants picking the same value must not replay each
    // other's responses.
    let (manager, _) = manager();
    let base = manager
        .scoped_key(1, "POST", "/v1/messages", "k")
        .as_str()
        .to_owned();
    assert_ne!(
        base,
        manager
            .scoped_key(2, "POST", "/v1/messages", "k")
            .as_str()
            .to_owned()
    );
    assert_ne!(
        base,
        manager
            .scoped_key(1, "PUT", "/v1/messages", "k")
            .as_str()
            .to_owned()
    );
    assert_ne!(
        base,
        manager
            .scoped_key(1, "POST", "/v1/responses", "k")
            .as_str()
            .to_owned()
    );
    assert_eq!(
        base,
        manager
            .scoped_key(1, "POST", "/v1/messages", "k")
            .as_str()
            .to_owned()
    );
}

#[tokio::test]
async fn an_unseen_key_has_nothing_to_replay() {
    let (manager, _) = manager();
    assert!(
        manager
            .check(&scope("fresh"))
            .await
            .expect("check")
            .is_none()
    );
}

#[tokio::test]
async fn only_the_first_claimant_owns_the_key() {
    let (manager, _) = manager();
    assert!(manager.claim(&scope("k")).await.expect("claim"));
    assert!(
        !manager.claim(&scope("k")).await.expect("claim"),
        "a concurrent duplicate must lose the race",
    );

    let sentinel = manager
        .check(&scope("k"))
        .await
        .expect("check")
        .expect("entry");
    assert!(
        sentinel.processing,
        "the loser must be able to tell in-flight from completed",
    );
}

#[tokio::test]
async fn storing_the_response_supersedes_the_claim_and_makes_it_replayable() {
    let (manager, _) = manager();
    manager.claim(&scope("k")).await.expect("claim");
    manager
        .store(
            &scope("k"),
            &CachedResponse {
                status_code: 200,
                body: br#"{"ok":true}"#.to_vec(),
                request_id: "req-1".to_owned(),
                ..CachedResponse::default()
            },
        )
        .await
        .expect("store");

    let cached = manager
        .check(&scope("k"))
        .await
        .expect("check")
        .expect("entry");
    assert!(!cached.processing);
    assert_eq!(cached.request_id, "req-1");
}

#[tokio::test]
async fn releasing_a_claim_frees_the_key_for_a_retry() {
    let (manager, _) = manager();
    manager.claim(&scope("k")).await.expect("claim");
    manager.release(&scope("k")).await.expect("release");
    assert!(
        manager.claim(&scope("k")).await.expect("claim"),
        "a failed request must not lock its key until the sentinel expires",
    );
}

#[tokio::test]
async fn entries_are_namespaced_so_they_cannot_collide_with_other_redis_users() {
    let (manager, store) = manager();
    manager.claim(&scope("k")).await.expect("claim");
    let keys: Vec<String> = store.entries.lock().keys().cloned().collect();
    assert_eq!(keys.len(), 1);
    assert!(
        keys[0].starts_with(KEY_PREFIX),
        "unprefixed key {}",
        keys[0]
    );
}

#[tokio::test]
async fn a_replay_reproduces_the_original_response() {
    let cached = CachedResponse {
        status_code: 201,
        headers: [("Content-Type".to_owned(), "application/json".to_owned())]
            .into_iter()
            .collect(),
        body: br#"{"id":"abc"}"#.to_vec(),
        ..CachedResponse::default()
    };
    let response = cached.clone().into_response();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/json"),
    );
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    assert_eq!(bytes.as_ref(), cached.body.as_slice());
}

#[tokio::test]
async fn a_replay_without_a_recorded_content_type_still_announces_json() {
    let response = CachedResponse {
        status_code: 200,
        body: b"{}".to_vec(),
        ..CachedResponse::default()
    }
    .into_response();
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/json"),
    );
}

#[test]
fn an_entry_round_trips_through_the_wire_format() {
    let entry = CachedResponse {
        status_code: 200,
        headers: [("Content-Type".to_owned(), "application/json".to_owned())]
            .into_iter()
            .collect(),
        body: br#"{"choices":[]}"#.to_vec(),
        cost: 0.25,
        request_id: "req-1".to_owned(),
        processing: false,
        truncated: true,
    };
    let encoded = serde_json::to_vec(&entry).expect("encodes");
    let decoded: CachedResponse = serde_json::from_slice(&encoded).expect("decodes");
    assert_eq!(decoded, entry);
}

#[test]
fn a_claim_sentinel_is_the_only_thing_that_declares_itself_in_flight() {
    // `processing` is skipped when false, so a stored response can never be
    // mistaken for a claim.
    let stored = serde_json::to_string(&CachedResponse {
        status_code: 200,
        ..CachedResponse::default()
    })
    .expect("encodes");
    assert!(!stored.contains("processing"));

    let claim = serde_json::to_string(&CachedResponse {
        processing: true,
        ..CachedResponse::default()
    })
    .expect("encodes");
    assert!(claim.contains("processing"));
}

// ---------------------------------------------------------------- wire format

// Both old and new binaries read and write the same Redis keys during a
// staged rollout, so the encoding IS the contract. These pin it against the
// legacy `encoding/json` output rather than against this crate's own round
// trip — a symmetric bug would sail straight through a round-trip-only test.
//
// Every literal below is the established wire encoding, not derived by reading
// the code.

#[test]
fn a_body_is_encoded_as_a_standard_alphabet_padded_base64_string() {
    // Standard alphabet, with padding. Not a JSON array, not raw text.
    let encoded = serde_json::to_value(&CachedResponse {
        status_code: 200,
        body: br#"{"ok":true}"#.to_vec(),
        ..CachedResponse::default()
    })
    .expect("encodes");
    assert_eq!(
        encoded["body"].as_str(),
        Some("eyJvayI6dHJ1ZX0="),
        "got {}",
        encoded["body"],
    );
}

#[test]
fn an_entry_written_by_the_legacy_binary_decodes() {
    // Verbatim shape of what `IdempotencyManager.Store` marshals.
    let legacy_json = r#"{
        "status_code": 200,
        "headers": {"Content-Type": "application/json"},
        "body": "eyJvayI6dHJ1ZX0=",
        "cost": 0.25,
        "request_id": "req-1"
    }"#;

    let decoded: CachedResponse =
        serde_json::from_str(legacy_json).expect("decodes the legacy output");
    assert_eq!(decoded.body, br#"{"ok":true}"#.to_vec());
    assert_eq!(decoded.status_code, 200);
    assert_eq!(decoded.cost, 0.25);
    assert_eq!(
        decoded.headers.get("Content-Type").map(String::as_str),
        Some("application/json"),
    );
    assert!(
        !decoded.processing,
        "an absent omitempty flag reads as false"
    );
}

#[test]
fn a_claim_sentinel_decodes_despite_its_null_map_and_body() {
    // `claim` marshals `&CachedResponse{processing: true}`; neither `headers`
    // nor `body` carries `omitempty`, so both keys are emitted as `null`. Plain
    // `#[serde(default)]` only covers an ABSENT key, not an explicit null.
    let legacy_sentinel = r#"{"status_code":0,"headers":null,"body":null,"cost":0,"request_id":"","processing":true}"#;

    let decoded: CachedResponse =
        serde_json::from_str(legacy_sentinel).expect("decodes the legacy sentinel");
    assert!(
        decoded.processing,
        "the loser of the claim race must see this"
    );
    assert!(decoded.headers.is_empty());
    assert!(decoded.body.is_empty());
}

#[test]
fn what_this_crate_writes_stays_within_what_the_legacy_reader_accepts() {
    // The legacy reader tolerates `""` for a byte slice and `{}` for a map, so
    // writing those instead of `null` is safe in the other direction.
    let written = serde_json::to_value(&CachedResponse {
        processing: true,
        ..CachedResponse::default()
    })
    .expect("encodes");
    assert_eq!(written["body"].as_str(), Some(""));
    assert!(written["headers"].is_object());
    assert_eq!(written["processing"].as_bool(), Some(true));
}

#[test]
fn a_binary_body_uses_the_standard_alphabet_and_survives_intact() {
    // The reason the encoding is base64 and not a string: an image, a gzip
    // frame or any other binary reply has to replay byte for byte.
    //
    // This payload encodes to a string containing both `/` and `+`, so it also
    // separates the standard alphabet from the URL-safe one. That distinction
    // round-trips perfectly within this crate and is rejected by the legacy
    // decoder, which is exactly why it is asserted against the legacy output.
    let body = vec![0x00, 0xff, 0xfe, 0x80, 0x7f];
    let entry = CachedResponse {
        status_code: 200,
        body: body.clone(),
        ..CachedResponse::default()
    };

    let encoded = serde_json::to_value(&entry).expect("encodes");
    assert_eq!(encoded["body"].as_str(), Some("AP/+gH8="));

    let decoded: CachedResponse = serde_json::from_value(encoded).expect("decodes");
    assert_eq!(decoded.body, body);
}
