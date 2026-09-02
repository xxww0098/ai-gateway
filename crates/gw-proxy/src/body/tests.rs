//! 三道闸的性质，以及两个搬运类型的合同。
//!
//! 这里不复述阈值的字面量 —— 测的是「网关这侧的 peek 上限就是中继默认的那个数」
//! 这条**关系**，以及越过它之后的行为。

use http_body_util::BodyExt as _;

use super::*;

/// 把 [`RelayBody`] 抽干成字节，用来断言「转发出去的是完整的原始 body」。
async fn drain(body: RelayBody) -> Bytes {
    match body {
        RelayBody::Buffered(bytes) => bytes,
        RelayBody::Streaming(body) => body.collect().await.expect("流不该失败").to_bytes(),
    }
}

/// 一份长度为 `len` 的可辨认载荷：全 `x` 会让「拼错顺序」这类错误看不出来。
fn payload(len: usize) -> Bytes {
    Bytes::from((0..len).map(|i| (i % 251) as u8).collect::<Vec<u8>>())
}

#[test]
fn the_billing_peek_limit_is_the_relay_default_not_a_second_number() {
    // 文档里承诺的是「引用同一个数」。抄一份相同的字面量过来，两边会悄悄漂开。
    assert_eq!(BILLING_PEEK_LIMIT, RelayBody::DEFAULT_BUFFER_LIMIT);
    assert_eq!(
        BILLING_PEEK_LIMIT,
        gw_relay::engine::RelayOptions::default().request_buffer_limit,
        "中继引擎默认缓冲多少，网关就得按多少去 peek",
    );
}

#[tokio::test]
async fn a_body_within_the_peek_limit_is_visible_to_billing() {
    let bytes = payload(1024);
    let body = read_inbound(Body::from(bytes.clone()))
        .await
        .expect("读 body 不该失败");

    assert_eq!(body.peek(), Some(bytes.as_ref()), "阈值内计费必须看得见");
    assert!(body.is_replayable(), "看得见的体必须能被 failover 重发");
    assert_eq!(drain(body).await, bytes);
}

#[tokio::test]
async fn a_body_over_the_peek_limit_is_invisible_but_forwarded_whole() {
    // 缺陷 #2：超阈值**不是错误**。计费看不见它，转发一个字节都不能少。
    let bytes = payload(BILLING_PEEK_LIMIT + 1);
    let body = read_inbound(Body::from(bytes.clone()))
        .await
        .expect("超阈值不是错误");

    assert_eq!(body.peek(), None, "计费必须显式面对『我看不见 body』");
    assert!(!body.is_replayable(), "流式体只能发一次");
    assert_eq!(drain(body).await, bytes, "转发出去的必须是完整的原始字节");
}

#[test]
fn only_a_visible_body_is_rewritable() {
    let bytes = payload(64);
    assert_eq!(
        rewritable(&RelayBody::Buffered(bytes.clone())),
        Some(&bytes)
    );

    let streaming = RelayBody::Streaming(
        http_body_util::Full::new(bytes)
            .map_err(|never: std::convert::Infallible| match never {})
            .boxed(),
    );
    assert!(
        rewritable(&streaming).is_none(),
        "看不见的 body 改不动，调用方必须明确拒绝而不是原样转发",
    );
}

#[tokio::test]
async fn a_replayable_body_can_be_sent_once_per_failover_attempt() {
    let bytes = payload(128);
    let mut outbound = Outbound::new(RelayBody::Buffered(bytes.clone()));
    assert_eq!(outbound.payload(), bytes, "planner 看得见可重放的体");

    for attempt in 0..3 {
        let body = outbound
            .next(None)
            .unwrap_or_else(|| panic!("第 {attempt} 次重试没有 body"));
        assert_eq!(drain(body).await, bytes);
    }
}

#[tokio::test]
async fn a_rewritten_body_replaces_the_bytes_the_planner_was_given() {
    let original = payload(32);
    let spliced = payload(48);
    let mut outbound = Outbound::new(RelayBody::Buffered(original));
    let body = outbound.next(Some(spliced.clone())).expect("有 body");
    assert_eq!(drain(body).await, spliced);
}

#[tokio::test]
async fn a_streaming_body_is_handed_out_exactly_once() {
    // failover 拿不到第二份，就必须停下 —— 而不是发一个空 body 重试。
    let bytes = payload(BILLING_PEEK_LIMIT + 1);
    let mut outbound = Outbound::new(
        read_inbound(Body::from(bytes.clone()))
            .await
            .expect("超阈值不是错误"),
    );
    assert!(
        outbound.payload().is_empty(),
        "planner 看不见流式体，拿到的是空 payload",
    );

    let first = outbound.next(None).expect("第一次尝试必须拿得到 body");
    assert_eq!(drain(first).await, bytes);
    assert!(outbound.next(None).is_none(), "第二个账号没有第二份可发");
}
