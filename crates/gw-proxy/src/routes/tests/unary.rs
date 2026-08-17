//! Unary settle is off the request path: same StreamSettler + drain tracker
//! a hung-up stream already uses.

use http_body_util::BodyExt;
use tower::ServiceExt;

use super::*;

/// The handler has returned (status + body in hand) while the ledger write
/// is still parked. That is the whole point of moving unary settle off-path.
#[tokio::test]
async fn a_unary_response_is_not_blocked_on_ledger_io() {
    let harness = Harness::build();
    let release = harness.usage_store.hold_commits();
    harness.provider.queue(Ok(ok_response(10, 20)));

    let response = harness
        .router()
        .oneshot(signed_request("/v1/chat/completions", chat_body("gpt-4o")))
        .await
        .expect("router responds");

    assert!(
        response.status().is_success(),
        "the client must see the upstream status before settle runs"
    );
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert!(
        body.is_object(),
        "the client must receive the upstream body before settle runs"
    );
    assert!(
        harness.usage_store.logs.lock().is_empty(),
        "ledger I/O was still gated; the response must not have waited for it",
    );
    assert_eq!(
        harness.drain.len(),
        1,
        "the settlement must already be on the tracker the composition root drains",
    );

    release.send(()).expect("settlement is waiting");
    harness.wait_idle().await;

    let logs = harness.usage_store.logs.lock();
    assert_eq!(logs.len(), 1, "settle must still pair with the hold");
    assert_eq!(logs[0].input_tokens, 10);
    assert_eq!(logs[0].output_tokens, 20);
    assert!(!logs[0].failed);
}

/// Dropping the response after the handler returns must not lose the charge:
/// unary already spawned onto the drain tracker, same as a stream hang-up.
#[tokio::test]
async fn dropping_a_unary_response_still_settles_through_the_shutdown_tracker() {
    let harness = Harness::build();
    let release = harness.usage_store.hold_commits();
    harness.provider.queue(Ok(ok_response(5, 7)));

    let response = harness
        .router()
        .oneshot(signed_request("/v1/chat/completions", chat_body("gpt-4o")))
        .await
        .expect("router responds");
    assert!(response.status().is_success());
    drop(response);

    assert_eq!(harness.drain.len(), 1);
    assert!(harness.usage_store.logs.lock().is_empty());

    // Exactly what `gw_server::drain_settlements` does after graceful shutdown.
    harness.drain.close();
    release.send(()).expect("settlement is waiting");
    harness.drain.wait().await;

    assert_eq!(harness.usage_store.logs.lock().len(), 1);
    assert_eq!(harness.drain.len(), 0);
}

/// A late drop (tracker already closed) is still waited on — `close` does
/// not reject later spawns, and `wait` covers them. Unary spawn happens
/// before the handler returns, so this is the "spawn during drain" cousin
/// of the stream test.
#[tokio::test]
async fn a_unary_settlement_spawned_before_close_is_still_waited_on() {
    let harness = Harness::build();
    harness.provider.queue(Ok(ok_response(3, 4)));

    let response = harness
        .router()
        .oneshot(signed_request("/v1/chat/completions", chat_body("gpt-4o")))
        .await
        .expect("router responds");
    drop(response);

    harness.drain.close();
    harness.drain.wait().await;

    assert_eq!(harness.usage_store.logs.lock().len(), 1);
    assert_eq!(harness.drain.len(), 0);
}

#[tokio::test]
async fn collecting_a_unary_body_does_not_double_settle() {
    let harness = Harness::build();
    harness.provider.queue(Ok(ok_response(11, 13)));

    send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(harness.usage_store.settled_costs().len(), 1);
    assert_eq!(harness.usage_store.logs.lock().len(), 1);
    assert_eq!(
        harness.drain.len(),
        0,
        "a finished detached settle must not leave a second task behind",
    );
}

#[tokio::test]
async fn a_unary_error_status_releases_instead_of_charging() {
    let harness = Harness::build();
    harness.provider.queue(Ok(gw_provider::types::ProviderResponse {
        status: 400,
        headers: http::HeaderMap::new(),
        body: bytes::Bytes::from_static(b"{\"error\":\"bad\"}"),
        usage: None,
    }));

    let (status, _) = send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        harness
            .ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
        "a non-success unary must release: {:?}",
        harness.ledger.calls(),
    );
    assert!(harness.usage_store.settled_costs().is_empty());
}

#[tokio::test]
async fn a_unary_without_usage_falls_back_rather_than_billing_zero() {
    let harness = Harness::build();
    harness.provider.queue(Ok(ok_response_without_usage()));

    send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    let costs = harness.usage_store.settled_costs();
    assert_eq!(costs.len(), 1);
    assert!(costs[0] > 0.0, "upstream output must never be free");
}

#[tokio::test]
async fn strict_mode_neither_settles_nor_releases_a_usage_less_unary() {
    let harness = Harness::build();
    harness.settlement.set_strict_usage_metadata(true);
    harness.provider.queue(Ok(ok_response_without_usage()));

    send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert!(
        harness.usage_store.settled_costs().is_empty(),
        "strict mode must not settle",
    );
    assert!(
        !harness
            .ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
        "strict mode must not release either — the hold expires on its TTL: {:?}",
        harness.ledger.calls(),
    );
    let logs = harness.usage_store.logs.lock();
    assert_eq!(logs.len(), 1);
    assert!(logs[0].failed);
}
