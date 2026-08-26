#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


def read(path: str) -> str:
    return Path(path).read_text(encoding="utf-8")


def write(path: str, text: str) -> None:
    Path(path).write_text(text, encoding="utf-8")


def replace_once(path: str, old: str, new: str) -> None:
    text = read(path)
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{path}: expected one occurrence, found {count}: {old[:80]!r}")
    write(path, text.replace(old, new, 1))


def replace_between(path: str, start: str, end: str, replacement: str) -> None:
    text = read(path)
    left = text.find(start)
    if left < 0:
        raise RuntimeError(f"{path}: start marker not found")
    right = text.find(end, left)
    if right < 0:
        raise RuntimeError(f"{path}: end marker not found")
    write(path, text[:left] + replacement + text[right:])


def append_once(path: str, marker: str, addition: str) -> None:
    text = read(path)
    if marker in text:
        return
    if not text.endswith("\n"):
        text += "\n"
    write(path, text + "\n" + addition.strip() + "\n")


# ---------------------------------------------------------------------------
# Google SSE framing: HTTP body chunks are not SSE event boundaries.

wire_path = "crates/gw-relay/src/translate/google/wire.rs"
replace_between(
    wire_path,
    "/// 从一段 SSE 字节里取出每个事件块的 `data:` 载荷。",
    "/// 这个 data 载荷值不值得解析成 Google 响应。",
    r'''/// Incrementally splits arbitrary HTTP body chunks into complete SSE events.
///
/// `reqwest` / hyper expose transport chunks, not SSE records. A single Google
/// JSON event may therefore be split at any byte. The decoder keeps only the
/// unterminated tail, emits complete `data:` payloads, and bounds one event so
/// a peer that never sends a blank line cannot grow memory without limit.
const MAX_SSE_EVENT: usize = 8 * 1024 * 1024;

#[derive(Debug, Default)]
pub(super) struct SseDecoder {
    pending: Vec<u8>,
}

impl SseDecoder {
    pub(super) fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, TranslateError> {
        self.pending.extend_from_slice(chunk);
        let mut payloads = Vec::new();
        let mut consumed = 0usize;

        while let Some((relative_end, separator_len)) =
            find_event_end(&self.pending[consumed..])
        {
            let end = consumed + relative_end;
            if end.saturating_sub(consumed) > MAX_SSE_EVENT {
                self.pending.clear();
                return Err(TranslateError::UpstreamShape(
                    "google SSE event exceeds 8 MiB".to_owned(),
                ));
            }
            if end > consumed {
                payloads.extend(data_payloads(&self.pending[consumed..end]));
            }
            consumed = end + separator_len;
        }

        if consumed > 0 {
            self.pending.drain(..consumed);
        }
        if self.pending.len() > MAX_SSE_EVENT {
            self.pending.clear();
            return Err(TranslateError::UpstreamShape(
                "unterminated google SSE event exceeds 8 MiB".to_owned(),
            ));
        }
        Ok(payloads)
    }
}

fn find_event_end(buf: &[u8]) -> Option<(usize, usize)> {
    for index in 0..buf.len() {
        if buf[index] == b'\n' && buf.get(index + 1) == Some(&b'\n') {
            return Some((index, 2));
        }
        if buf[index] == b'\r' && buf[index + 1..].starts_with(b"\n\r\n") {
            return Some((index, 4));
        }
    }
    None
}

/// Extracts every `data:` payload from one or more complete SSE event blocks.
/// Multi-line data fields are joined with `\n` as required by the SSE grammar.
fn data_payloads(buf: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut cur: Option<Vec<u8>> = None;
    for raw in buf.split(|&b| b == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            if let Some(payload) = cur.take() {
                out.push(payload);
            }
            continue;
        }
        let Some(rest) = line.strip_prefix(b"data:") else {
            continue;
        };
        let rest = rest.strip_prefix(b" ").unwrap_or(rest);
        let slot = cur.get_or_insert_with(Vec::new);
        if !slot.is_empty() {
            slot.push(b'\n');
        }
        slot.extend_from_slice(rest);
    }
    if let Some(payload) = cur.take() {
        out.push(payload);
    }
    out
}

''',
)

openai_path = "crates/gw-relay/src/translate/google/openai.rs"
replace_once(
    openai_path,
    "struct OpenAiStream {\n    id: Option<String>,",
    "struct OpenAiStream {\n    sse: wire::SseDecoder,\n    id: Option<String>,",
)
replace_once(
    openai_path,
    "    finished: bool,\n    usage: RelayUsage,",
    "    finished: bool,\n    done: bool,\n    usage: RelayUsage,",
)
replace_once(
    openai_path,
    "        for payload in wire::data_payloads(upstream_frame) {",
    "        for payload in self.sse.push(upstream_frame)? {",
)
replace_once(
    openai_path,
    '''    fn finish(&mut self) -> Result<Vec<Bytes>, TranslateError> {
        let mut frames = Vec::new();
        if !self.finished {
''',
    '''    fn finish(&mut self) -> Result<Vec<Bytes>, TranslateError> {
        if self.done {
            return Ok(Vec::new());
        }
        // A compliant SSE event ends with a blank line. Appending one at EOF
        // also turns a truncated final JSON event into a visible parse error
        // instead of synthesising a clean stop for an incomplete answer.
        let mut frames = self.push(b"\\n\\n")?;
        if !self.finished {
''',
)
replace_once(
    openai_path,
    '''        frames.push(Bytes::from_static(b"data: [DONE]\\n\\n"));
        Ok(frames)
''',
    '''        self.done = true;
        frames.push(Bytes::from_static(b"data: [DONE]\\n\\n"));
        Ok(frames)
''',
)

anthropic_path = "crates/gw-relay/src/translate/google/anthropic.rs"
replace_once(
    anthropic_path,
    "struct AnthropicStream {\n    id: Option<String>,",
    "struct AnthropicStream {\n    sse: wire::SseDecoder,\n    id: Option<String>,",
)
replace_once(
    anthropic_path,
    "        for payload in wire::data_payloads(upstream_frame) {",
    "        for payload in self.sse.push(upstream_frame)? {",
)
replace_once(
    anthropic_path,
    '''    fn finish(&mut self) -> Result<Vec<Bytes>, TranslateError> {
        let mut frames = Vec::new();
        if self.stopped {
            // 幂等：重复收尾会发出第二个 `message_stop`，客户端会把它当成
            // 第二条消息的开头。
            return Ok(frames);
        }
        self.stopped = true;
''',
    '''    fn finish(&mut self) -> Result<Vec<Bytes>, TranslateError> {
        if self.stopped {
            // 幂等：重复收尾会发出第二个 `message_stop`，客户端会把它当成
            // 第二条消息的开头。
            return Ok(Vec::new());
        }
        // Flush a final event that omitted the trailing blank line. If it is a
        // truncated JSON object, `push` returns an error and the client sees a
        // reset rather than a fabricated clean EOF.
        let mut frames = self.push(b"\\n\\n")?;
        self.stopped = true;
''',
)

tests_path = "crates/gw-relay/src/translate/google/tests.rs"
replace_once(
    tests_path,
    '''    let zero = r#"data: {"usageMetadata":{"promptTokenCount":0,"candidatesTokenCount":0}}"#;
    let absent = r#"data: {"usageMetadata":{"promptTokenCount":0}}"#;
''',
    '''    let zero = concat!(
        r#"data: {"usageMetadata":{"promptTokenCount":0,"candidatesTokenCount":0}}"#,
        "\\n\\n"
    );
    let absent = concat!(
        r#"data: {"usageMetadata":{"promptTokenCount":0}}"#,
        "\\n\\n"
    );
''',
)
append_once(
    tests_path,
    "fn google_stream_translation_is_independent_of_network_chunking()",
    r'''
/// HTTP transport chunks are unrelated to SSE event boundaries. Every possible
/// single split — plus the one-byte-at-a-time adversarial framing — must produce
/// the same spoken text and the same billing usage for both target dialects.
#[test]
fn google_stream_translation_is_independent_of_network_chunking() {
    let wire: Vec<u8> = google_sse_frames().into_iter().flatten().collect();

    let translated_text = |frames: &[(Option<String>, Value)]| -> String {
        frames
            .iter()
            .filter_map(|(_, value)| {
                value
                    .pointer("/choices/0/delta/content")
                    .and_then(Value::as_str)
                    .or_else(|| value.pointer("/delta/text").and_then(Value::as_str))
            })
            .collect()
    };

    for translator in [&OpenAiToGoogle as &dyn Translator, &AnthropicToGoogle] {
        let (_, expected_usage) = run_stream(translator);

        for cut in 1..wire.len() {
            let mut stream = translator.stream_translator();
            let mut output = stream.push(&wire[..cut]).expect("first transport chunk");
            output.extend(
                stream
                    .push(&wire[cut..])
                    .expect("second transport chunk"),
            );
            output.extend(stream.finish().expect("finish"));
            let decoded = decode(&output);
            assert_eq!(
                translated_text(&decoded),
                SPOKEN,
                "SSE split at byte {cut} changed the response"
            );
            assert_eq!(
                stream.usage(),
                expected_usage,
                "SSE split at byte {cut} changed billing usage"
            );
        }

        let mut stream = translator.stream_translator();
        let mut output = Vec::new();
        for byte in &wire {
            output.extend(
                stream
                    .push(std::slice::from_ref(byte))
                    .expect("one-byte transport chunk"),
            );
        }
        output.extend(stream.finish().expect("finish"));
        assert_eq!(translated_text(&decode(&output)), SPOKEN);
        assert_eq!(stream.usage(), expected_usage);
    }
}

/// A final partial JSON event is an upstream truncation, not a successful empty
/// turn. `finish` must surface it instead of fabricating a normal stop frame.
#[test]
fn truncated_google_event_is_not_reported_as_a_clean_eof() {
    for translator in [&OpenAiToGoogle as &dyn Translator, &AnthropicToGoogle] {
        let mut stream = translator.stream_translator();
        assert!(
            stream
                .push(br#"data: {"candidates":[{"content":{"parts":[{"text":"half"#)
                .expect("the incomplete event is buffered")
                .is_empty()
        );
        assert!(matches!(
            stream.finish().expect_err("truncated JSON must fail"),
            TranslateError::UpstreamShape(_)
        ));
    }
}
''',
)

translation_path = "crates/gw-proxy/src/routes/translation.rs"
replace_once(
    translation_path,
    '''    let translated = match translator.translate_response(&original) {
        Ok(body) => body,
        // Infrastructure error pages are often HTML. Preserve their real
        // status/body rather than replacing a useful 429/503 with a gateway 502.
        Err(_) if !response.status.is_success() => original,
        Err(err) => return Err(err),
    };
    response.body = RelayResponseBody::Buffered(translated);
    rewrite_entity_headers(&mut response.headers, false);
    Ok((response, UsageHandle::completed(usage)))
''',
    '''    let translated = match translator.translate_response(&original) {
        Ok(body) => body,
        // Infrastructure error pages are often HTML. Preserve the status,
        // bytes *and entity headers*; labelling HTML as application/json is
        // another form of corruption and breaks SDK diagnostics.
        Err(_) if !response.status.is_success() => {
            response.body = RelayResponseBody::Buffered(original);
            return Ok((response, UsageHandle::completed(usage)));
        }
        Err(err) => return Err(err),
    };
    response.body = RelayResponseBody::Buffered(translated);
    rewrite_entity_headers(&mut response.headers, false);
    Ok((response, UsageHandle::completed(usage)))
''',
)

routes_path = "crates/gw-proxy/src/routes.rs"
replace_between(
    routes_path,
    "//! # 已知缺口（本轮**未**根除，需要 `gw-provider` 配合）",
    "//! Dispatch 选出上游候选",
    r'''//! # 已根除的协议接缝
//!
//! `/v1/responses` 的端点由入口元数据决定；7 个 Translate 格现在在 proxy
//! 中显式调用对应 [`gw_relay::Translator`]。请求、普通响应和 SSE 都只翻译一次，
//! translated stream 的 usage 直接取自同一个状态机，不再额外挂 probe 重复解析。
//! 无法等价表达的 Responses→非 OpenAI 三格仍按矩阵明确返回 400。
//!
//! ''',
)
replace_once(
    routes_path,
    '''endpoint!(
    /// 入口 B · `POST /v1/responses` —— OpenAI Responses 方言。
    ///
    /// ⚠️ **打到 openai / codex 时今天仍然是坏的**：矩阵已经把它判成
    /// [`gw_relay::UpstreamDialect::OpenAiResponses`]，但 `gw-provider` 的
    /// executor 只会构造 `{base}/v1/chat/completions`。见模块 doc 的「已知缺口」。
    responses,
''',
    '''endpoint!(
    /// 入口 B · `POST /v1/responses` —— OpenAI Responses 方言。
    ///
    /// OpenAI / Codex 原生直通；Claude / Google 三格因有状态 item 语义无法
    /// 等价表达，按 15 格矩阵明确返回入口方言的 400。
    responses,
''',
)

stream_path = "crates/gw-proxy/src/routes/stream.rs"
replace_between(
    stream_path,
    "//! # One response path",
    "//! # Settling",
    r'''//! # One response path
//!
//! Passthrough frames remain byte-for-byte identical. Translate cells wrap the
//! same upstream body with one request-scoped state machine before this module
//! sees it; that state machine also owns usage extraction. Both paths still
//! share this single header-copy, disconnect and settlement implementation.
//!
//! ''',
)

proxy_stream_tests = "crates/gw-proxy/src/routes/tests/stream.rs"
append_once(
    proxy_stream_tests,
    "fn a_translated_google_stream_survives_transport_fragmentation_and_bills_once()",
    r'''
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
    let harness = Harness::build_routed(
        vec![auth_record("acct-google", "gemini")],
        Some(resolver),
    );

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

    let (status, content_type, body) =
        collect_stream(&harness, stream_body("house-model")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.contains("event-stream"));
    assert!(body.contains("hello"), "translated output lost text: {body}");
    assert!(body.ends_with("data: [DONE]\n\n"));

    harness.wait_idle().await;
    let logs = harness.usage_store.logs.lock();
    assert_eq!(logs.len(), 1, "translated stream must settle exactly once");
    assert_eq!(logs[0].input_tokens, 9);
    assert_eq!(logs[0].output_tokens, 4);
    assert!(!logs[0].failed);
}
''',
)

proxy_dispatch_tests = "crates/gw-proxy/src/routes/tests/dispatch.rs"
append_once(
    proxy_dispatch_tests,
    "fn an_untranslatable_upstream_error_keeps_its_original_entity_headers()",
    r'''
/// When a translated provider returns a non-JSON infrastructure page, the
/// gateway cannot change its dialect. It must preserve the useful status, bytes
/// and content type rather than claiming the HTML body is JSON.
#[tokio::test]
async fn an_untranslatable_upstream_error_keeps_its_original_entity_headers() {
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    let harness = Harness::build_routed(
        vec![auth_record("acct-1", "gemini")],
        Some(gemini_only_resolver()),
    );
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    let original = Bytes::from_static(b"<html><body>temporarily unavailable</body></html>");
    harness.transport.queue(Ok(CannedResponse {
        status: 503,
        headers,
        frames: vec![original.clone()],
    }));

    let response = harness
        .router()
        .oneshot(signed_request(
            "/v1/chat/completions",
            chat_body("house-model"),
        ))
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/html; charset=utf-8")
    );
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    assert_eq!(body, original);
}
''',
)

print("google stream framing hardening applied")
