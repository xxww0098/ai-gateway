//! Endpoint routing, upstream failover, and the middleware order the whole
//! billing pipeline depends on.
//!
//! Split by dialect rather than by phase: `/v1` and `/v1beta` share one
//! pipeline, and the point of most of these cases is that they share it, so the
//! imports and the SSE collector below are common ground both files draw on.

use axum::http::StatusCode;
use gw_provider::types::{StreamChunk, UsageRecord};

use super::*;
use crate::testsupport::{
    Harness, LedgerCall, TEST_API_KEY, anonymous_request, auth_record, chat_body, ok_response,
    ok_response_without_usage, send, signed_get, signed_request,
};

mod dispatch;
mod gemini;

/// A usage chunk carrying `input`/`output` tokens, as an upstream would emit it
/// mid-stream.
fn usage_chunk(input: i64, output: i64) -> StreamChunk {
    StreamChunk::Usage(UsageRecord {
        model: "gpt-4o".to_owned(),
        provider: "openai".to_owned(),
        input_tokens: Some(input),
        output_tokens: Some(output),
        cached_tokens: None,
        reasoning_tokens: None,
    })
}

/// Sends `request` and returns `(status, content-type, relayed body)`.
async fn collect_sse(
    harness: &Harness,
    request: axum::http::Request<axum::body::Body>,
) -> (StatusCode, String, String) {
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let response = harness
        .router()
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (
        status,
        content_type,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}
