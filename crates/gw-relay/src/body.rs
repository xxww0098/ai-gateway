//! `RelayBody` / `RelayResponseBody` 的构造与消费。
//!
//! OWNER: worker `relay-core`。
//!
//! 这里是缺陷 #2（1 MiB 硬上限）的落点：入站 body 在 peek 阈值内走
//! [`crate::RelayBody::Buffered`]（peek 与转发共用同一块内存），超阈值走
//! `Streaming`（边收边转、无上限、计费降级到保守估算）。
//! **计费降级，转发不降级。**
//!
//! # 这里根除的缺陷
//!
//! | 编号 | 内容 |
//! | --- | --- |
//! | #2 (S1) | 1 MiB 打满即 413 → 超阈值改为不缓冲直转，**没有转发上限** |
//! | #13 | `to_vec()` 在 failover 循环体内每次重拷 → 全程 `Bytes`，`clone` 是 refcount |
//! | #14 | `bytes().await?.to_vec()` → `Buffered` 直接把 `Bytes` 交给 reqwest |

use std::convert::Infallible;
use std::fmt::Display;

use bytes::{Bytes, BytesMut};
use futures_util::stream;
use http_body::{Body as HttpBody, Frame};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use tokio::sync::mpsc;

use crate::contract::{RelayBody, RelayError, RelayResponseBody};

impl RelayBody {
    /// 入站 body 的缓冲阈值默认值：4 MiB。
    ///
    /// # 依据
    ///
    /// 这个数**只决定「计费能不能看到 body」，不再决定「请求能不能转发」** ——
    /// 这正是缺陷 #2 的根：今天的 1 MiB 是计费实现细节泄漏成了转发能力限制。
    ///
    /// 取 4 MiB 的理由是实际的大请求形状：Claude Code 一次 `/v1/messages` 带
    /// 完整会话历史（100–500 KiB）+ 一次 `Read` 出来的大文件（几百 KiB）+ 一张
    /// 1920×1080 截图的 base64（约 1.4–4 MiB）。4 MiB 覆盖其中绝大多数，于是
    /// 精确计费**不**降级；再往上的请求本来就罕见，降级成保守估算的代价，远小于
    /// 把常驻内存翻倍的代价。
    ///
    /// 上界是显式的：最坏常驻 ≈ `4 MiB × 在途请求数`（256 并发 ≈ 1 GiB）。
    /// 这就是它不是 16 MiB 的原因。调大之前先算这个乘法。
    pub const DEFAULT_BUFFER_LIMIT: usize = 4 * 1024 * 1024;

    /// 把入站的 [`http_body::Body`] 收成两态之一。
    ///
    /// - 累计 data 字节 **≤ `limit`** 且流已结束 → [`RelayBody::Buffered`]，
    ///   计费 peek 与转发共用同一块内存（单帧时零拷贝，多帧时一次拼接）。
    /// - 累计字节**首次超过 `limit`** → 立刻切到 [`RelayBody::Streaming`]，
    ///   **已经读出来的前缀原样接回流头**，剩下的边收边转。
    ///
    /// # 根除缺陷 #2（S1）
    ///
    /// 超阈值**不是错误**。今天 `to_bytes(body, 1 MiB)` 超限返回 `Err` → 413，
    /// Claude Code 长会话必然撞上，用户唯一出路是 `/compact`。这里超阈值只是
    /// 「计费看不见 body」，转发照常。
    ///
    /// # 关于 `Sync`
    ///
    /// 契约里的 `Streaming` 是 [`BoxBody`]（`Send + Sync`），而 `axum::body::Body`
    /// 底下是 `UnsyncBoxBody`（**不是** `Sync`）。所以这里只要求入参 `Send`，
    /// 超阈值时用一个容量 1 的 channel 把非 `Sync` 的源桥过去。桥接任务在
    /// 接收端被 drop 的**那一刻**就退出（`tx.closed()` 参与 `select`），
    /// 于是客户端断开 → 源 body 被 drop → 取消照常传播到上游，不会有孤儿任务
    /// 卡在一个永远不产帧的源上。
    ///
    /// # Errors
    ///
    /// 缓冲阶段（尚未超阈值）读源 body 失败时返回 [`RelayError::Upstream`]。
    /// 切到 `Streaming` 之后的失败走流内的 `Err` item，不走这里。
    pub async fn from_body<B>(body: B, limit: usize) -> Result<Self, RelayError>
    where
        B: HttpBody<Data = Bytes> + Send + 'static,
        B::Error: Display + Send + 'static,
    {
        let mut body = Box::pin(body);
        let mut prefix: Vec<Bytes> = Vec::new();
        let mut total: usize = 0;

        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|e| RelayError::Upstream(e.to_string()))?;
            // 请求方向的 trailer 没有用武之地，丢弃。
            let Ok(data) = frame.into_data() else {
                continue;
            };
            if data.is_empty() {
                continue;
            }
            total = total.saturating_add(data.len());
            prefix.push(data);
            if total > limit {
                return Ok(Self::Streaming(splice(prefix, body)));
            }
        }

        Ok(Self::Buffered(join(prefix)))
    }

    /// 转发用。**零拷贝**。
    ///
    /// [`RelayBody::Buffered`] 直接把 `Bytes` 交给 reqwest（`Body::from(Bytes)`
    /// 是接管，不是复制），[`RelayBody::Streaming`] 把 body 整个接过去。
    ///
    /// # 根除缺陷 #13 / #14
    ///
    /// 今天这一跳付两次全量堆拷贝（`routes.rs:217` 的 `to_vec()` 在 failover
    /// 循环体内，provider 再 `clone()` 一次），一个 900 KB 的请求在 3 次 failover
    /// 下要 memcpy 约 5.4 MB。这里是 0：`Bytes::clone` 是原子加一。
    #[must_use]
    pub fn into_upstream(self) -> reqwest::Body {
        match self {
            Self::Buffered(bytes) => reqwest::Body::from(bytes),
            Self::Streaming(body) => reqwest::Body::wrap(body),
        }
    }
}

impl RelayResponseBody {
    /// 转成 hyper / axum 能直接用的 body。
    ///
    /// 错误类型保持 [`RelayError`] 而**不是** `Infallible` —— 这是缺陷 #6 的解药：
    /// 流式中途失败必须能交给 hyper（h2 发 `RST_STREAM`，h1 掐连接），
    /// 否则客户端拿到的是一次**干净的 EOF**，Claude Code 不报错不重试。
    ///
    /// `axum::body::Body::new()` 直接吃这个类型；本 crate 不依赖 axum，
    /// 所以停在 [`BoxBody`] 这个公共货币上。
    #[must_use]
    pub fn into_http_body(self) -> BoxBody<Bytes, RelayError> {
        match self {
            Self::Buffered(bytes) => Full::new(bytes)
                .map_err(|never: Infallible| match never {})
                .boxed(),
            Self::Stream(body) => body,
        }
    }
}

/// 把前缀帧拼回流头，后面接原 body。
///
/// channel 容量 1：只做「反压 + 跨 `Sync` 边界」，不做缓冲。
fn splice<B>(prefix: Vec<Bytes>, rest: B) -> BoxBody<Bytes, RelayError>
where
    B: HttpBody<Data = Bytes> + Send + Unpin + 'static,
    B::Error: Display + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<Result<Frame<Bytes>, RelayError>>(1);
    drop(tokio::spawn(pump(prefix, rest, tx)));
    StreamBody::new(stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }))
    .boxed()
}

/// 桥接任务：前缀帧 → 源 body 的剩余帧 → 结束。
///
/// 接收端一被 drop 就退出（`tx.closed()` 在 `select` 里，且 `biased`），
/// 源 body 随之 drop，取消信号继续往上游传。
async fn pump<B>(
    prefix: Vec<Bytes>,
    mut rest: B,
    tx: mpsc::Sender<Result<Frame<Bytes>, RelayError>>,
) where
    B: HttpBody<Data = Bytes> + Send + Unpin + 'static,
    B::Error: Display + Send + 'static,
{
    for chunk in prefix {
        if tx.send(Ok(Frame::data(chunk))).await.is_err() {
            return;
        }
    }
    loop {
        let next = tokio::select! {
            biased;
            () = tx.closed() => return,
            frame = rest.frame() => frame,
        };
        let Some(frame) = next else { return };
        match frame {
            Ok(frame) => {
                if tx.send(Ok(frame)).await.is_err() {
                    return;
                }
            }
            Err(err) => {
                drop(tx.send(Err(RelayError::Upstream(err.to_string()))).await);
                return;
            }
        }
    }
}

/// 单帧时零拷贝，多帧时一次拼接（这一次是聚合成连续内存的必要代价）。
///
/// 请求方向与响应方向共用 —— 「一个概念只声明一处」。
pub(crate) fn join(mut parts: Vec<Bytes>) -> Bytes {
    if parts.len() == 1 {
        return parts.pop().unwrap_or_default();
    }
    let mut buf = BytesMut::with_capacity(parts.iter().map(Bytes::len).sum());
    for part in parts {
        buf.extend_from_slice(&part);
    }
    buf.freeze()
}

#[cfg(test)]
pub(crate) mod fixtures;
#[cfg(test)]
mod tests;
