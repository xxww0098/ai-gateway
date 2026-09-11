//! 上游选择：四级链 + 15 格矩阵。
//!
//! 这一段是 `gw-relay` 的 [`gw_relay::endpoint::upstream`] 与
//! [`gw_relay::endpoint::matrix`] 在 `gw-proxy` 侧的接线，与「怎么转发、
//! 怎么结算」是两件事，所以住在自己的文件里。

use axum::body::{Body, Bytes};
use axum::http::{StatusCode, header};
use axum::response::Response;
use gw_relay::Surface;
use gw_relay::endpoint::matrix::{self, Route};
use gw_relay::endpoint::upstream::{self, ChannelResolver, SelectionLevel};

/// 四级链选出的上游候选，外加它来自哪一级。
///
/// 直接复用 [`gw_relay::endpoint::upstream::Selection`]，本文件不再自己算候选。
pub(crate) type Selection = upstream::Selection;

/// 跑一次四级链。`resolver` 为 `None` 时 L1/L2/L3 全部短路，
/// 行为与收敛前的 `provider_candidates()` **逐字节相同**（灰度回滚开关）。
///
/// 这是 `routes.rs:73-97` 那段 `contains("codex")` / `starts_with("claude-")`
/// 前缀猜测的唯一继任者。根因一句话：**模型名是一个由上游厂商随时变更的字符串，
/// 而路由决策把它当成了一个稳定的枚举。**
pub(crate) fn select_upstreams(
    surface: Surface,
    model: &str,
    resolver: Option<&dyn ChannelResolver>,
) -> Selection {
    upstream::select(surface, Some(model), resolver)
}

/// 按 15 格矩阵把候选切成「能转发的」与「必须 400 的」。
///
/// 返回 `(可转发的 executor 名, 第一条 400 的 (状态码, 信封字节))`。
///
/// # 为什么 `Translate` 也归到 400
///
/// 转义器已经在 `gw-relay` 里写好了，但**接线需要 [`gw_relay::engine::RelayEngine`]
/// 整体取代本文件的转发段**（wave 4）。在那之前，这一格今天的行为是「原样转发 →
/// 上游必 400」（`docs/relay-surface-plan.md` §3.6「今天」一列，7 个转义格全是
/// 「直通 → 上游 400」）。所以换成网关自己的 400 **不破坏任何今天能工作的流量**，
/// 而且客户端能从错误里读到「这是网关的路由问题、该改用哪个入口」，
/// 上游的 400 里读不到这个。
pub(crate) fn partition_routable(
    surface: Surface,
    selection: &Selection,
    model: &str,
) -> (Vec<&'static str>, Option<(StatusCode, Bytes)>) {
    let mut routable = Vec::with_capacity(selection.candidates.len());
    let mut reject: Option<(StatusCode, Bytes)> = None;
    for provider in &selection.candidates {
        match matrix::route(surface, *provider, Some(model)) {
            Route::Passthrough { .. } => routable.push(provider.as_str()),
            Route::Translate { .. } => {
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
///
/// 信封形状由入口决定而不是网关自定：客户端 SDK 只会解析它自己那套结构，
/// 回一个陌生结构，`@anthropic-ai/sdk` 与 OpenAI SDK 都读不到 `error.message`，
/// 用户看到的是一个**无字的红叉**。
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
///
/// 走整体 JSON round-trip：这条路径**只在客户端显式写了 `<channel>/<model>` 时
/// 才会命中**，而那是一个 opt-in 的少数派用法，所以整体重序列化的代价是有界的
/// —— 它不在 `stream_options` 那种「每个流式请求都走一遍」的位置上
/// （那里才必须用 `gw_relay::endpoint::include_usage` 的定点字节插入）。
///
/// 解析失败（body 不是 JSON 对象）时**原样返回**：改不了就不改，
/// 绝不把一个转发不出去的 body 变成另一个转发不出去的 body。
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
