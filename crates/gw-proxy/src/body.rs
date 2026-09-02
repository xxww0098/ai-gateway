//! 入站请求体的一生：**看得见吗、改得动吗、还能再发一次吗**。
//!
//! 三个问题各有一道闸，而**三道闸是分开的** —— 这就是本模块存在的全部理由。
//!
//! 收敛前只有一道：`to_bytes(body, 1 MiB)`，超了就 413。于是「计费看不看得见
//! 这份 JSON」直接决定了「这个请求转不转得出去」。Claude Code 一次带完整会话
//! 历史（100–500 KiB）+ 一个 `Read` 出来的大文件 + 一张截图 base64 的
//! `/v1/messages` 必然撞上它，用户唯一的出路是 `/compact`。
//! **计费降级，转发不降级** —— 这句话要成立，两个上限就必须是两个数。
//!
//! # 三道闸
//!
//! | 闸 | 越过它会怎样 | 谁在守 |
//! | --- | --- | --- |
//! | [`BILLING_PEEK_LIMIT`] | 计费降级成保守估算，**转发照旧** | `read_inbound` |
//! | [`TRANSLATION_BUFFER_LIMIT`] | 必须整体改写 body 的那条路径明确 400 | `rewritable` |
//! | 传输硬上限 | **不存在**，见下 | [`gw_relay::RelayTimeouts`] 三档超时 |
//!
//! ## 为什么第三道闸不是一个常数
//!
//! 转发方向**故意没有字节上限**。一个纯字节中继没有立场替上游决定「多大算大」，
//! 而 [`gw_relay::RelayTimeouts`] 的 connect / request / stream_idle 三档已经是
//! 真正的传输闸门：它们挡的是「连不上」「发不完」「卡住了」，那才是转发方向
//! 真实的失败模式。再补一个字节上限，等于把刚拆掉的那道 413 换个数字装回去。
//!
//! # 两个搬运类型
//!
//! [`InboundBody`] 把 hold 层读到的体交给 handler（一次交接，不重读）；
//! `Outbound` 回答 failover 那个问题：**这份体还能不能再发一次**。

use axum::body::{Body, Bytes};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use gw_relay::RelayBody;
use parking_lot::Mutex;
use std::sync::Arc;

/// 计费能看见多少字节。超过它 → 计费降级成保守估算，**转发不受影响**。
///
/// 取的就是 [`RelayBody::DEFAULT_BUFFER_LIMIT`]（4 MiB）：不是抄一个相同的数，
/// 是**引用同一个数**。`gw_relay::engine::RelayOptions::request_buffer_limit`
/// 的默认值也是它，网关这侧再声明一个自己的值，只会让两边悄悄漂开。
/// 4 MiB 的依据（Claude Code 的真实请求形状，以及 `4 MiB × 在途请求数` 这条
/// 常驻内存上界）写在 gw-relay 那个常量上，这里不复述。
pub const BILLING_PEEK_LIMIT: usize = RelayBody::DEFAULT_BUFFER_LIMIT;

/// 网关必须**整体解析并重写** body 的那条路径的上限。
///
/// 今天与 [`BILLING_PEEK_LIMIT`] 同值，而且不是巧合：改写的前提是看得见，
/// 而 [`RelayBody::Buffered`] 的长度按构造不超过 peek 上限，所以真正的判据是
/// 「body 可见吗」，第二个数字不会独立生效。它仍然是一个**独立的名字**，
/// 因为它答的是另一个问题；把它调小＝「计费能看见但不许改写」，
/// 那是一个还没有人需要的决定，不预先替未来做。
pub const TRANSLATION_BUFFER_LIMIT: usize = BILLING_PEEK_LIMIT;

/// 把入站 [`Body`] 收成 [`RelayBody`] 的两态之一。
///
/// 阈值内 → [`RelayBody::Buffered`]，peek 与转发共用同一块内存；
/// 超阈值 → [`RelayBody::Streaming`]，已读出的前缀接回流头，剩下的边收边转。
/// **超阈值不是错误**，它只意味着计费看不见 body。
///
/// # Errors
///
/// 只有一种：缓冲阶段读客户端连接失败。那是**客户端**没把 body 发完，
/// 400 而不是 413 —— 413 说的是「你发得太大了」，而这里网关根本没有大小意见。
pub(crate) async fn read_inbound(body: Body) -> Result<RelayBody, Response> {
    RelayBody::from_body(body, BILLING_PEEK_LIMIT)
        .await
        .map_err(|err| {
            tracing::debug!(%err, "读入站 body 失败");
            (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({
                    "error": "Bad Request",
                    "message": "request body could not be read",
                })),
            )
                .into_response()
        })
}

/// body 可见、且在 [`TRANSLATION_BUFFER_LIMIT`] 之内时的只读视图。
///
/// `None` = **网关改不动这份 body**。调用方必须据此明确拒绝，
/// 不许原样往上游转 —— 一个被静默丢掉的改写，客户端只会看到上游的
/// 「模型不存在」，从那里读不出「是网关没能改写请求体」。
pub(crate) fn rewritable(body: &RelayBody) -> Option<&Bytes> {
    match body {
        RelayBody::Buffered(bytes) if bytes.len() <= TRANSLATION_BUFFER_LIMIT => Some(bytes),
        _ => None,
    }
}

/// hold 层读到的入站 body，经请求扩展交给 handler。
///
/// # 为什么是 `Arc<Mutex<Option<..>>>` 而不是直接放 [`RelayBody`]
///
/// 请求扩展要求 `Clone`，而 [`RelayBody`] **故意不是** `Clone`：
/// 一个还在流的 body 只能被消费一次，这正是 [`RelayBody::is_replayable`]
/// 说的那件事。所以这里放的是一个**一次性交接槽** —— hold 放进去，
/// handler [`take`](Self::take) 出来，第二次拿到 `None`。
///
/// 交接而不是「让 handler 自己再读一遍」：hold 已经把前缀读走了，
/// 再读一遍要么读到半截，要么把同样的 4 MiB 再缓冲一次。
#[derive(Debug, Clone)]
pub struct InboundBody(Arc<Mutex<Option<RelayBody>>>);

impl InboundBody {
    #[must_use]
    pub fn new(body: RelayBody) -> Self {
        Self(Arc::new(Mutex::new(Some(body))))
    }

    /// 取走这份 body。第二次调用返回 `None`。
    #[must_use]
    pub fn take(self) -> Option<RelayBody> {
        self.0.lock().take()
    }
}

/// 一次派发里「要发给上游的那份 body」，按**还能不能再发一次**分岔。
///
/// 分岔的依据是 [`RelayBody::is_replayable`]：可重放的体每次 failover 都能再发
/// 一遍（`Bytes::clone` 是 refcount，三次重试零拷贝）；流式体**只能发一次**
/// —— 字节已经在往上游流了，没有第二份。第二个账号必须就此停下，
/// 而不是拿一个空 body 去重试。
pub(crate) enum Outbound {
    /// 看得见、可重放。
    Replayable(Bytes),
    /// 看不见、只有一次机会。`None` = 已经交给某个账号了。
    Once(Option<RelayBody>),
}

impl Outbound {
    pub(crate) fn new(body: RelayBody) -> Self {
        match body {
            RelayBody::Buffered(bytes) => Self::Replayable(bytes),
            streaming => Self::Once(Some(streaming)),
        }
    }

    /// planner 看到的 `ProviderRequest::payload`。
    ///
    /// 流式体看不见，给空 —— `ensure_include_usage` 对空 body 返回
    /// 「一个字节都不动」，于是流式直通天然走「不改写」那一支，
    /// 不需要在 planner 里加一个「body 可见吗」的分支。
    pub(crate) fn payload(&self) -> Bytes {
        match self {
            Self::Replayable(bytes) => bytes.clone(),
            Self::Once(_) => Bytes::new(),
        }
    }

    /// 这一次尝试要发的 body。`None` = 流式体已经交给上一个账号了。
    ///
    /// `rewritten` 是 [`gw_provider::route::RoutePlan::body`]（今天唯一的改写是
    /// `stream_options.include_usage` 的定点插入）。它只对可重放的体有意义：
    /// planner 拿到的流式 payload 是空的，改不出任何东西来。
    pub(crate) fn next(&mut self, rewritten: Option<Bytes>) -> Option<RelayBody> {
        match self {
            Self::Replayable(bytes) => Some(RelayBody::Buffered(
                rewritten.unwrap_or_else(|| bytes.clone()),
            )),
            Self::Once(slot) => slot.take(),
        }
    }
}

#[cfg(test)]
mod tests;
