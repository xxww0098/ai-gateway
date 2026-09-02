//! 上游选择：四级链 + 15 格矩阵。
//!
//! 这一段是 `gw-relay` 的 [`gw_relay::endpoint::upstream`] 与
//! [`gw_relay::endpoint::matrix`] 在 `gw-proxy` 侧的接线，与「怎么转发、
//! 怎么结算」是两件事，所以住在自己的文件里。

use axum::body::{Body, Bytes};
use axum::http::{StatusCode, header};
use axum::response::Response;
use gw_relay::endpoint::matrix::{self, Route};
use gw_relay::endpoint::upstream::{self, ChannelResolver, SelectionLevel};
use gw_relay::{Surface, Translator, UpstreamDialect};

/// 四级链选出的上游候选，外加它来自哪一级。
pub(crate) type Selection = upstream::Selection;

/// 一个能执行的矩阵格。
#[derive(Clone, Copy)]
pub(crate) struct RoutedProvider {
    pub(crate) name: &'static str,
    pub(crate) upstream: UpstreamDialect,
    pub(crate) translator: Option<&'static dyn Translator>,
}

impl RoutedProvider {
    /// planner 需要知道目标 wire 的端点族，而不是客户端入口。
    #[must_use]
    pub(crate) fn planner_surface(self, inbound: Surface) -> Surface {
        match self.upstream {
            UpstreamDialect::OpenAiChat => Surface::OpenAiCompletions,
            UpstreamDialect::OpenAiResponses => Surface::OpenAiResponses,
            UpstreamDialect::AnthropicMessages => Surface::AnthropicMessages,
            UpstreamDialect::GoogleGenerateContent => inbound,
        }
    }
}

/// 跑一次四级链。`resolver` 为 `None` 时 L1/L2/L3 全部短路，
/// 行为与收敛前的 `provider_candidates()` **逐字节相同**（灰度回滚开关）。
pub(crate) fn select_upstreams(
    surface: Surface,
    model: &str,
    resolver: Option<&dyn ChannelResolver>,
) -> Selection {
    upstream::select(surface, Some(model), resolver)
}

/// 按 15 格矩阵把候选切成「能执行的」与「必须 400 的」。
///
/// Translate 格只有在矩阵能返回一个静态 translator 时才进入执行集合；
/// 不存在的 translator 仍然 fail-closed，而不是把异方言 body 原样发给上游。
pub(crate) fn partition_routable(
    surface: Surface,
    selection: &Selection,
    model: &str,
) -> (Vec<RoutedProvider>, Option<(StatusCode, Bytes)>) {
    let mut routable = Vec::with_capacity(selection.candidates.len());
    let mut reject: Option<(StatusCode, Bytes)> = None;
    for provider in &selection.candidates {
        match matrix::route(surface, *provider, Some(model)) {
            Route::Passthrough { upstream } => routable.push(RoutedProvider {
                name: provider.as_str(),
                upstream,
                translator: None,
            }),
            Route::Translate { upstream } => {
                if let Some(translator) = matrix::translator_for(surface, upstream) {
                    routable.push(RoutedProvider {
                        name: provider.as_str(),
                        upstream,
                        translator: Some(translator),
                    });
                } else {
                    reject.get_or_insert_with(|| {
                        (
                            matrix::REJECT_STATUS,
                            matrix::reject_body(
                                surface,
                                *provider,
                                Some(model),
                                matrix::RejectReason::TranslatorUnavailable,
                            ),
                        )
                    });
                }
            }
            Route::Reject(body) => {
                reject.get_or_insert((matrix::REJECT_STATUS, body));
            }
        }
    }
    if selection.level == SelectionLevel::PrefixGuess {
        tracing::debug!(
            model,
            surface = ?surface,
            "上游选择落到 L4 前缀猜测（gw-relay 已打点，见 upstream::prefix_guess_hits）"
        );
    }
    (routable, reject)
}

/// 把 [`matrix::reject_body`] 已经备好的**入口方言**错误信封写成一个响应。
pub(crate) fn dialect_error(status: StatusCode, body: Bytes) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    response
}

/// 改写请求体顶层的 `model` 值。**只在 L1 剥掉了渠道前缀时调用。**
pub(crate) fn rewrite_model(body: &Bytes, model: &str) -> Bytes {
    let Ok(serde_json::Value::Object(mut map)) = serde_json::from_slice(body) else {
        return body.clone();
    };
    map.insert(
        "model".to_owned(),
        serde_json::Value::String(model.to_owned()),
    );
    serde_json::to_vec(&serde_json::Value::Object(map)).map_or_else(|_| body.clone(), Bytes::from)
}
