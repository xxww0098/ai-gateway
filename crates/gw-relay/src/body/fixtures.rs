//! 测试夹具。OWNER: worker `relay-core`。
//!
//! 放在 `body/` 下是因为它们全是 [`http_body::Body`] 的替身；`engine/tests.rs`
//! 也用它们（`crate::body` 在 crate 内可见）—— 同一个夹具只写一份。

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::stream;
use http_body::{Body as HttpBody, Frame};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, StreamBody};

use crate::contract::RelayError;

/// 一个把 `chunks` 逐帧吐出来然后结束的 body。
pub(crate) fn frames(chunks: Vec<Bytes>) -> BoxBody<Bytes, RelayError> {
    StreamBody::new(stream::iter(chunks.into_iter().map(|c| Ok(Frame::data(c))))).boxed()
}

/// 逐帧吐出 `items`，其中的 `Err` 会中断流 —— 用来演「中途失败」。
pub(crate) fn fallible(items: Vec<Result<Bytes, RelayError>>) -> BoxBody<Bytes, RelayError> {
    StreamBody::new(stream::iter(
        items.into_iter().map(|item| item.map(Frame::data)),
    ))
    .boxed()
}

/// 先吐 `head`（如果有），之后**永远 `Pending`**；被 drop 时把标志位置 `true`。
///
/// 用来验证两件事：帧间空闲看门狗真的会触发；客户端断开时源 body 真的被 drop
/// （取消传播到上游，不会有孤儿任务挂在一个永不产帧的源上）。
pub(crate) struct Hanging {
    head: Option<Bytes>,
    dropped: Arc<AtomicBool>,
}

impl Hanging {
    /// 返回 body 与一个「我被 drop 了吗」的标志位。
    pub(crate) fn new(head: Option<Bytes>) -> (Self, Arc<AtomicBool>) {
        let dropped = Arc::new(AtomicBool::new(false));
        (
            Self {
                head,
                dropped: Arc::clone(&dropped),
            },
            dropped,
        )
    }
}

impl Drop for Hanging {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl HttpBody for Hanging {
    type Data = Bytes;
    type Error = RelayError;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        match self.head.take() {
            Some(head) => Poll::Ready(Some(Ok(Frame::data(head)))),
            None => Poll::Pending,
        }
    }
}

/// 把一个 body 抽干成「帧序列」。保留帧边界与顺序 —— 断言 SSE 不被重新分帧
/// 就靠它。遇到 `Err` 就收进结果并停。
pub(crate) async fn drain(mut body: BoxBody<Bytes, RelayError>) -> Vec<Result<Bytes, RelayError>> {
    let mut out = Vec::new();
    while let Some(frame) = body.frame().await {
        match frame {
            Ok(frame) => {
                if let Ok(data) = frame.into_data() {
                    out.push(Ok(data));
                }
            }
            Err(err) => {
                out.push(Err(err));
                break;
            }
        }
    }
    out
}

/// 只留 `Ok` 的那一半，方便和输入逐帧比。
pub(crate) fn payloads(frames: &[Result<Bytes, RelayError>]) -> Vec<Bytes> {
    frames
        .iter()
        .filter_map(|f| f.as_ref().ok().cloned())
        .collect()
}
