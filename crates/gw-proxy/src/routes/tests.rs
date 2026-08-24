//! Endpoint routing, upstream failover, and the middleware order the whole
//! billing pipeline depends on.
//!
//! 三面收敛之后只剩一个前缀，所以此前按方言切出去的 `gemini.rs` 整个消失了。
//! 它里面 17 条用例中的 15 条测的是已被硬删的 `/v1beta` 面；另外 2 条测的是
//! `/v1` 面上的凭证语义（`x-goog-api-key` 不是凭据、`x-api-key` 是 Anthropic
//! 的上游头），已迁到 [`dispatch`] 保命 —— 那两条是删 `/v1beta` 时最容易被顺手
//! 带走的不变量。

use axum::http::StatusCode;

use super::*;
use crate::testsupport::{
    CannedResponse, Harness, LedgerCall, TEST_API_KEY, anonymous_request, auth_record, chat_body,
    send, send_settled, signed_get, signed_request,
};

mod dispatch;
mod stream;
mod unary;

/// The terminal SSE frame an OpenAI-shaped upstream ends a stream with.
///
/// The counts travel in the frame itself, so the *real* side-band probe is
/// what extracts them — the same code production runs.
const USAGE_FRAME: &str = "data: {\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":22}}\n\n";

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

/// `request_metadata` 写进去的东西，executor 必须认得回来。
///
/// # 为什么这条要单独存在
///
/// 这是缺陷 #1（S1）的最后一公里，而且它**失败时是静默的**：
/// `gw_provider::common::request_surface` 在键缺失或路径不认识时回落到
/// `Surface::OpenAiCompletions`（那是本键存在之前的既有行为，回落本身是对的）。
/// 于是一旦 `request_metadata` 忘了写、或者写了个 executor 认不出的值，
/// `POST /v1/responses` 就会**悄悄**被打回 chat/completions —— 上游必 400，
/// 三个保留入口之一 100% 不可用，而整个测试套件照样全绿。
///
/// 所以这里断言的不是「map 里有某个字面量」，而是**跨 crate 的往返**：
/// gw-proxy 写 → gw-provider 读 → 拿回同一个入口。
#[test]
fn every_surface_survives_the_metadata_round_trip_to_the_executor() {
    for surface in [
        Surface::OpenAiCompletions,
        Surface::OpenAiResponses,
        Surface::AnthropicMessages,
    ] {
        let request = gw_provider::types::ProviderRequest {
            metadata: request_metadata(surface, None, None),
            ..Default::default()
        };
        assert_eq!(
            gw_provider::common::request_surface(&request),
            surface,
            "{surface:?} 经 metadata 交给 executor 后认不回来了 —— \
             executor 会静默回落到 chat/completions"
        );
    }
}
