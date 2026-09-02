//! Streaming relay: framing, settle-on-end, settle-on-drop, usage fallback.

use axum::body::Bytes;

use super::*;

/// 逐跳头留在这一跳 —— 包括这条响应自己 `Connection` **点名**的那些。
///
/// 从前这里断言的是「那张写死的名单里有 `connection`」，即常量等于常量
/// （规范 2.11）：名单漏了什么、`Connection` 的值有没有被读，它一概测不出来 ——
/// 而当时那张名单确实既漏了 `expect`，也从不看 `Connection` 的值。现在断言的是
/// **客户端到底收到了什么**，`x-foo` 是这条测试自己造的名字，生产源码里没有它。
#[tokio::test]
async fn hop_by_hop_headers_are_not_relayed() {
    let harness = Harness::build();
    let mut canned = CannedResponse::sse(&["data: one\n\n"]);
    for (name, value) in [
        // 这条消息把 x-foo 声明成只在这一跳有效。
        ("connection", "close, x-foo"),
        ("x-foo", "hop-scoped"),
        ("transfer-encoding", "chunked"),
        // 上游那份长度描述的是上游那份 body，不是我们发出去的这份。
        ("content-length", "17"),
        ("x-request-id", "req_hop"),
    ] {
        canned
            .headers
            .insert(name, value.parse().expect("测试用的 header 值"));
    }
    harness.transport.queue(Ok(canned));

    let response = {
        use tower::ServiceExt;
        harness
            .router()
            .oneshot(signed_request(
                "/v1/chat/completions",
                stream_body("gpt-4o"),
            ))
            .await
            .expect("router responds")
    };

    assert_eq!(response.status(), StatusCode::OK);
    for name in ["connection", "x-foo", "transfer-encoding", "content-length"] {
        assert!(
            response.headers().get(name).is_none(),
            "{name} 属于这一跳，不该到达客户端",
        );
    }
    assert_eq!(
        response.headers().get("x-request-id").map(|v| v.as_bytes()),
        Some(&b"req_hop"[..]),
        "普通 header 一个都不许丢",
    );
}

// ---------------------------------------------------------------- streaming

fn stream_body(model: &str) -> serde_json::Value {
    let mut body = chat_body(model);
    body["stream"] = serde_json::json!(true);
    body
}

/// Sends a streaming chat request and returns the relayed SSE text.
async fn collect_stream(
    harness: &Harness,
    body: serde_json::Value,
) -> (StatusCode, String, String) {
    collect_sse(harness, signed_request("/v1/chat/completions", body)).await
}

#[tokio::test]
async fn a_streamed_response_relays_every_frame_verbatim() {
    // Including the usage frame. Usage is read on a **side band**; the relay
    // does not filter the byte stream, because filtering it means parsing it,
    // and parsing the write path is what a pass-through must never do.
    let harness = Harness::build();
    harness.transport.queue(Ok(CannedResponse::sse(&[
        "data: one\n\n",
        "data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":20}}\n\n",
        "data: [DONE]\n\n",
    ])));

    let (status, content_type, body) = collect_stream(&harness, stream_body("gpt-4o")).await;

    assert_eq!(status, StatusCode::OK);
    assert!(content_type.contains("event-stream"), "got {content_type}");
    assert_eq!(
        body,
        "data: one\n\ndata: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":20}}\n\ndata: [DONE]\n\n",
    );
}

#[tokio::test]
async fn a_stream_settles_once_it_has_finished_and_uses_the_usage_it_carried() {
    let harness = Harness::build();
    harness
        .transport
        .queue(Ok(CannedResponse::sse(&["data: one\n\n", USAGE_FRAME])));

    collect_stream(&harness, stream_body("gpt-4o")).await;
    harness.wait_idle().await;

    let logs = harness.usage_store.logs.lock();
    assert_eq!(logs.len(), 1, "a stream must settle exactly once");
    assert_eq!(logs[0].input_tokens, 11);
    assert_eq!(logs[0].output_tokens, 22);
    assert!(logs[0].stream, "the usage row must record that it streamed");
}

#[tokio::test]
async fn an_upstream_error_status_releases_instead_of_charging() {
    let harness = Harness::build();
    harness.transport.queue(Ok(CannedResponse::status(500)));
    harness.transport.queue(Ok(CannedResponse::status(500)));
    harness.transport.queue(Ok(CannedResponse::status(500)));

    collect_stream(&harness, stream_body("gpt-4o")).await;
    harness.wait_idle().await;

    assert!(
        harness
            .ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
    );
    assert!(harness.usage_store.settled_costs().is_empty());
}

#[tokio::test]
async fn a_stream_without_a_usage_frame_falls_back_rather_than_billing_zero() {
    let harness = Harness::build();
    harness
        .transport
        .queue(Ok(CannedResponse::sse(&["data: one\n\n"])));

    collect_stream(&harness, stream_body("gpt-4o")).await;
    harness.wait_idle().await;

    let costs = harness.usage_store.settled_costs();
    assert_eq!(costs.len(), 1);
    assert!(costs[0] > 0.0);
}

#[tokio::test]
async fn abandoning_a_stream_mid_flight_settles_through_the_shutdown_tracker() {
    // A client that hangs up drops the body without the stream ever ending, so
    // the settlement is detached. It MUST go to the composition root's tracker:
    // a bare `tokio::spawn` is aborted when the runtime is dropped, which turns
    // a disconnect during shutdown into a lost charge and a leaked hold.
    //
    // The deferred finalizer must run on the server even when the client
    // disconnects, so shutdown drains it.
    use tower::ServiceExt;

    let harness = Harness::build();
    harness.transport.queue(Ok(CannedResponse::sse(&[
        "data: one\n\n",
        USAGE_FRAME,
        "data: two\n\n",
    ])));

    let response = harness
        .router()
        .oneshot(signed_request(
            "/v1/chat/completions",
            stream_body("gpt-4o"),
        ))
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        harness.usage_store.logs.lock().is_empty(),
        "nothing should have settled while the stream was still live",
    );

    drop(response); // the client goes away without reading a byte

    assert_eq!(
        harness.drain.len(),
        1,
        "the settlement must be registered on the tracker the composition root \
         drains, not spawned loose onto the runtime",
    );

    // Exactly what `gw_server::drain_settlements` does after graceful shutdown.
    harness.drain.close();
    harness.drain.wait().await;

    assert_eq!(
        harness.usage_store.logs.lock().len(),
        1,
        "draining the tracker must have run the settlement to completion",
    );
    assert_eq!(harness.drain.len(), 0);
}

#[tokio::test]
async fn a_disconnect_that_lands_during_the_drain_is_still_waited_on() {
    // `TaskTracker::close` does not block later spawns, which is why
    // `StreamSettler::drop` needs no "already closing?" check: a body dropped
    // after the drain began is still tracked, and `wait()` still covers it.
    use tower::ServiceExt;

    let harness = Harness::build();
    harness
        .transport
        .queue(Ok(CannedResponse::sse(&["data: one\n\n", USAGE_FRAME])));

    let response = harness
        .router()
        .oneshot(signed_request(
            "/v1/chat/completions",
            stream_body("gpt-4o"),
        ))
        .await
        .expect("router responds");

    harness.drain.close(); // shutdown starts while the client is still attached
    drop(response); // ...and only then does the client go away
    harness.drain.wait().await;

    assert_eq!(
        harness.usage_store.logs.lock().len(),
        1,
        "a settlement spawned after close() must still be drained",
    );
}

#[tokio::test]
async fn stream_headers_drop_hop_by_hop_and_keep_repeated_values() {
    // The relay used to `insert` each header, which collapses set-cookie to
    // the last value, and cloned the whole map. Moving the map keeps every
    // value and still strips hop-by-hop names.
    let harness = Harness::build();
    let mut canned = CannedResponse::sse(&["data: one\n\n"]);
    canned
        .headers
        .append("set-cookie", "a=1".parse().expect("cookie"));
    canned
        .headers
        .append("set-cookie", "b=2".parse().expect("cookie"));
    canned
        .headers
        .insert("transfer-encoding", "chunked".parse().expect("te"));
    canned
        .headers
        .insert("x-request-id", "req_stream".parse().expect("id"));
    harness.transport.queue(Ok(canned));

    let response = {
        use tower::ServiceExt;
        harness
            .router()
            .oneshot(signed_request(
                "/v1/chat/completions",
                stream_body("gpt-4o"),
            ))
            .await
            .expect("router responds")
    };

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers().get("transfer-encoding").is_none(),
        "hop-by-hop headers must not leak to the client",
    );
    let cookies: Vec<_> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.as_bytes())
        .collect();
    assert_eq!(cookies, vec![&b"a=1"[..], &b"b=2"[..]]);
    assert_eq!(
        response.headers().get("x-request-id").map(|v| v.as_bytes()),
        Some(&b"req_stream"[..]),
    );
}

#[tokio::test]
async fn a_stream_that_runs_to_completion_settles_exactly_once() {
    // Completion and disconnect take the same path — the settler drops either
    // way — so what matters is that only *one* settlement results, and that the
    // drain has nothing left once it has run.
    let harness = Harness::build();
    harness
        .transport
        .queue(Ok(CannedResponse::sse(&["data: one\n\n", USAGE_FRAME])));

    collect_stream(&harness, stream_body("gpt-4o")).await;
    harness.wait_idle().await;

    assert_eq!(harness.usage_store.logs.lock().len(), 1);
    assert_eq!(harness.usage_store.settled_costs().len(), 1);
    assert_eq!(
        harness.drain.len(),
        0,
        "a finished settlement must not leave a second task behind",
    );
}

/// Production wiring must feed arbitrary HTTP chunks into the incremental SSE
/// decoder. Splitting one Google JSON object in the middle must neither lose
/// visible text nor force fallback billing.
#[tokio::test]
async fn a_translated_google_stream_survives_transport_fragmentation_and_bills_once() {
    let resolver: std::sync::Arc<dyn gw_relay::endpoint::upstream::ChannelResolver> =
        std::sync::Arc::new(
            gw_relay::endpoint::upstream::InMemoryChannelResolver::new()
                .with_model("house-model", ["house"])
                .with_channel("house", gw_relay::endpoint::matrix::Provider::Gemini),
        );
    let harness = Harness::build_routed(vec![auth_record("acct-google", "gemini")], Some(resolver));

    let wire = concat!(
        r#"data: {"responseId":"google-stream-1","modelVersion":"gemini-test","#,
        r#""candidates":[{"content":{"parts":[{"text":"hello"}]},"finishReason":"STOP"}],"#,
        r#""usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":4}}"#,
        "\n\n"
    )
    .as_bytes();
    let hello = wire
        .windows(b"hello".len())
        .position(|window| window == b"hello")
        .expect("fixture contains text");
    let cut = hello + 2;

    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("text/event-stream"),
    );
    harness.transport.queue(Ok(CannedResponse {
        status: 200,
        headers,
        frames: vec![
            Bytes::copy_from_slice(&wire[..cut]),
            Bytes::copy_from_slice(&wire[cut..]),
        ],
    }));

    let (status, content_type, body) = collect_stream(&harness, stream_body("house-model")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.contains("event-stream"));
    assert!(
        body.contains("hello"),
        "translated output lost text: {body}"
    );
    assert!(body.ends_with("data: [DONE]\n\n"));

    harness.wait_idle().await;
    let logs = harness.usage_store.logs.lock();
    assert_eq!(logs.len(), 1, "translated stream must settle exactly once");
    assert_eq!(logs[0].input_tokens, 9);
    assert_eq!(logs[0].output_tokens, 4);
    assert!(!logs[0].failed);
}
