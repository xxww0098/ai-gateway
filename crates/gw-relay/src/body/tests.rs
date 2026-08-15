//! OWNER: worker `relay-core`。
//!
//! 规范 2.11：断言的期望值全部来自测试自己造的输入，或是「同一块内存」
//! 「一个字节不丢」「边界被接受/被拒」这类不写死在源码里的性质。

use std::sync::atomic::Ordering;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::BodyExt;

use super::fixtures::{Hanging, fallible, frames};
use crate::contract::{RelayBody, RelayError, RelayResponseBody};

fn blob(byte: u8, len: usize) -> Bytes {
    Bytes::from(vec![byte; len])
}

/// 缺陷 #13 / #14 的守护测试：peek 看到的、转发出去的，必须是**同一块内存**。
///
/// 把 `into_upstream` 的 `Buffered` 分支改成 `Body::from(bytes.to_vec())`
/// 这条就红 —— 字节还相等，但指针不同了。
#[tokio::test]
async fn peek_and_the_forwarded_bytes_are_the_same_allocation() {
    let payload = blob(0x5a, 4096);
    let origin = payload.as_ptr();

    let body = RelayBody::from_body(frames(vec![payload.clone()]), payload.len())
        .await
        .expect("阈值内不会失败");

    let peeked = body.peek().expect("阈值内必须能 peek");
    assert_eq!(peeked.as_ptr(), origin, "peek 必须借用源内存，不是副本");
    assert_eq!(peeked, &payload[..]);

    let upstream = body.into_upstream();
    let sent = upstream
        .as_bytes()
        .expect("Buffered 必须以整块字节交给 reqwest");
    assert_eq!(sent.as_ptr(), origin, "转发出去的必须还是同一块内存");
    assert_eq!(sent, &payload[..]);
}

/// 缺陷 #2（S1）的守护测试：**超阈值必须转发得出去**，而且一个字节都不丢。
///
/// 把超阈值分支改回「返回 Err」（今天的 413）这条就红。
#[tokio::test]
async fn an_oversized_request_is_forwarded_instead_of_rejected() {
    let chunks: Vec<Bytes> = (0..8u8).map(|i| blob(i, 4096)).collect();
    let whole: Vec<u8> = chunks.iter().flat_map(|c| c.iter().copied()).collect();
    let limit = whole.len() / 4;

    let body = RelayBody::from_body(frames(chunks), limit)
        .await
        .expect("超阈值不是错误");

    assert!(body.peek().is_none(), "超阈值时计费必须显式看不到 body");
    assert!(!body.is_replayable(), "流式体不能被 failover 重放");

    let RelayBody::Streaming(inner) = body else {
        panic!("超阈值必须落到 Streaming");
    };
    let forwarded = inner.collect().await.expect("不该失败").to_bytes();
    assert_eq!(
        forwarded.as_ref(),
        whole.as_slice(),
        "已经读出来的前缀必须原样接回流头"
    );
}

/// 阈值语义：`== limit` 仍然是 Buffered，`limit + 1` 才切 Streaming。
#[tokio::test]
async fn the_buffer_threshold_boundary_is_inclusive() {
    let limit = 512;

    let exact = RelayBody::from_body(frames(vec![blob(1, limit)]), limit)
        .await
        .expect("不会失败");
    assert!(exact.peek().is_some(), "正好等于阈值必须仍是 Buffered");

    let over = RelayBody::from_body(frames(vec![blob(1, limit + 1)]), limit)
        .await
        .expect("不会失败");
    assert!(over.peek().is_none(), "阈值 + 1 必须切到 Streaming");
}

/// 多帧且在阈值内时，拼出来的字节必须与逐帧输入完全一致。
#[tokio::test]
async fn multi_frame_bodies_are_joined_byte_for_byte() {
    let chunks = vec![
        Bytes::from_static(b"{\"model\":"),
        Bytes::from_static(b"\"m\",\"stream\":"),
        Bytes::from_static(b"true}"),
    ];
    let whole: Vec<u8> = chunks.iter().flat_map(|c| c.iter().copied()).collect();

    let body = RelayBody::from_body(frames(chunks), whole.len())
        .await
        .expect("不会失败");

    assert_eq!(body.peek().expect("阈值内"), whole.as_slice());
}

/// 空 body 也必须落在 Buffered 上 —— 计费看到的是「空」，不是「看不见」。
#[tokio::test]
async fn an_empty_body_is_buffered_not_streaming() {
    let body = RelayBody::from_body(frames(Vec::new()), 16)
        .await
        .expect("不会失败");
    assert_eq!(body.peek().expect("空 body 仍然可 peek").len(), 0);
}

/// 超阈值的流被 drop 时，**源 body 必须跟着被 drop** ——
/// 否则桥接任务会挂在一个永不产帧的源上，取消传不到上游。
#[tokio::test]
async fn dropping_an_oversized_request_body_drops_the_source() {
    let (source, dropped) = Hanging::new(Some(blob(3, 64)));

    let body = RelayBody::from_body(source, 8)
        .await
        .expect("超阈值不是错误");
    let RelayBody::Streaming(inner) = body else {
        panic!("超阈值必须落到 Streaming");
    };
    drop(inner);

    // 桥接任务要被调度一次才会观察到接收端已关闭。
    for _ in 0..64 {
        if dropped.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("接收端 drop 之后源 body 仍然活着");
}

/// 缺陷 #6 的守护测试（body 层）：中途失败必须以 `Err` 抵达 hyper，
/// 而不是变成一次干净的 EOF。
#[tokio::test]
async fn a_mid_stream_failure_reaches_the_body_error_type() {
    let body = RelayResponseBody::Stream(fallible(vec![
        Ok(Bytes::from_static(b"data: {}\n\n")),
        Err(RelayError::Upstream("upstream went away".to_owned())),
    ]));

    let Err(err) = body.into_http_body().collect().await else {
        panic!("中途失败不能表现为正常结束");
    };
    assert!(matches!(err, RelayError::Upstream(_)));
}

/// 非流式响应体转成 hyper body 之后字节不变。
#[tokio::test]
async fn a_buffered_response_body_round_trips_unchanged() {
    // 故意用**非 UTF-8** 字节：错误体全程 Bytes，不许经过 String（缺陷 #12）。
    let payload = Bytes::from_static(&[0xff, 0xfe, 0x00, 0x41, 0x80]);
    let out = RelayResponseBody::Buffered(payload.clone())
        .into_http_body()
        .collect()
        .await
        .expect("不会失败")
        .to_bytes();
    assert_eq!(out, payload);
}
