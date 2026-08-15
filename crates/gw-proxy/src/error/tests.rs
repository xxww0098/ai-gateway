//! The rejection bodies are a cross-implementation contract: client SDKs branch
//! on the status and the `error` code, so those are what these tests pin.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use http_body_util::BodyExt;

use super::*;

async fn body_of(response: axum::response::Response) -> serde_json::Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

fn every_rejection() -> Vec<HoldRejection> {
    vec![
        HoldRejection::MissingAccessContext,
        HoldRejection::InvalidUserId,
        HoldRejection::RateLimited,
        HoldRejection::CircuitOpen,
        HoldRejection::OutstandingDebt,
        HoldRejection::InsufficientBalance {
            current_balance: 0.5,
            required_amount: 2.0,
        },
        HoldRejection::QuotaExceeded("subscription daily quota exceeded".to_owned()),
        HoldRejection::PaymentRequired,
        HoldRejection::IdempotencyConflict,
        HoldRejection::IdempotencyReplayUnavailable,
    ]
}

#[tokio::test]
async fn every_rejection_is_a_client_or_server_error_with_a_code() {
    for rejection in every_rejection() {
        let status = rejection.status();
        assert!(
            status.is_client_error() || status.is_server_error(),
            "{rejection:?} produced a non-error status {status}",
        );
        assert!(!rejection.code().is_empty(), "{rejection:?} has no code");

        let body = body_of(rejection.clone().into_response()).await;
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some(rejection.code()),
            "{rejection:?} did not surface its code in the body",
        );
    }
}

#[tokio::test]
async fn outstanding_debt_carries_only_the_error_code() {
    // The one rejection emitted without a `message`; clients that switch on
    // body shape would break if a message appeared.
    let body = body_of(HoldRejection::OutstandingDebt.into_response()).await;
    assert_eq!(
        body.as_object().map(|o| o.len()),
        Some(1),
        "outstanding_debt gained extra fields: {body}",
    );
}

#[tokio::test]
async fn insufficient_balance_reports_the_gap_and_where_to_fix_it() {
    let rejection = HoldRejection::InsufficientBalance {
        current_balance: 0.25,
        required_amount: 3.0,
    };
    assert_eq!(rejection.status(), StatusCode::PAYMENT_REQUIRED);

    let body = body_of(rejection.into_response()).await;
    let current = body["current_balance"].as_f64().expect("current_balance");
    let required = body["required_amount"].as_f64().expect("required_amount");
    assert!(
        current < required,
        "a 402 must describe a shortfall, got {current} >= {required}",
    );
    assert_eq!(body["top_up_url"].as_str(), Some(TOP_UP_URL));
}

#[tokio::test]
async fn quota_rejections_pass_the_reason_through() {
    let reason = "subscription weekly quota exceeded";
    let body = body_of(HoldRejection::QuotaExceeded(reason.to_owned()).into_response()).await;
    assert_eq!(body["message"].as_str(), Some(reason));
}

#[test]
fn credential_failures_are_unauthorized_and_infrastructure_failures_are_not() {
    assert_eq!(AuthError::NoCredentials.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        AuthError::InvalidCredential.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        AuthError::Internal("boom".to_owned()).status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn an_upstream_body_is_relayed_rather_than_rewrapped() {
    // The upstream already speaks the caller's dialect; re-wrapping it would
    // break clients that parse provider-specific error shapes.
    let upstream = r#"{"error":{"type":"rate_limit_error"}}"#;
    let response = DispatchError::Upstream {
        status: StatusCode::TOO_MANY_REQUESTS,
        body: upstream.to_owned(),
    }
    .into_response();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        body_of(response).await,
        serde_json::json!({"error":{"type":"rate_limit_error"}})
    );
}

#[test]
fn dispatch_failures_map_onto_distinguishable_statuses() {
    assert_eq!(
        DispatchError::NoUpstream("openai".to_owned()).status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        DispatchError::UnknownModel("nope".to_owned()).status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        DispatchError::Internal(anyhow::anyhow!("boom")).status(),
        StatusCode::BAD_GATEWAY
    );
}
