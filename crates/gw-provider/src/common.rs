//! Plumbing shared by every provider executor.
//!
//! OWNER: worker `provider-openai` (but written to be called by all five
//! executors — `claude.rs` / `gemini.rs` / `vertex.rs` are welcome here).
//!
//! Header forwarding lives in [`crate::types`] (`copy_outbound_headers` /
//! `is_skipped_proxy_header`), next to the trait it serves.

use crate::streambuf::StreamUsageBuffer;
use crate::types::{ProviderError, ProviderRequest, StreamChunk, StreamResponse};
use crate::usage::UsageTokens;
use bytes::Bytes;
use futures::Stream;
use futures_util::StreamExt;
use serde_json::Value;
use std::borrow::Cow;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Duration;

/// Shared provider identifiers.
pub const PROVIDER_OPENAI: &str = "openai";
/// See [`PROVIDER_OPENAI`].
pub const PROVIDER_CLAUDE: &str = "claude";
/// See [`PROVIDER_OPENAI`].
pub const PROVIDER_GEMINI: &str = "gemini";
/// See [`PROVIDER_OPENAI`].
pub const PROVIDER_CODEX: &str = "codex";
/// See [`PROVIDER_OPENAI`].
pub const PROVIDER_VERTEX: &str = "vertex";

/// Whole-request cap for non-streaming calls.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Bounds how long a streaming read may stall — receiving no bytes from
/// upstream — before the request is aborted.
pub const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// [`ProviderRequest::metadata`] key holding the client-facing model name when
/// the router already rewrote [`ProviderRequest::model`] to an upstream alias.
///
/// `gw-proxy` is the only writer.
pub const REQUESTED_MODEL_METADATA_KEY: &str = "requested_model";

/// Provider-specific upstream settings.
///
/// Deliberately local rather than a `gw_config` re-export: the executors need
/// exactly these three fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderConfig {
    pub base_url: String,
    pub api_key: String,
    pub enabled: bool,
}

/// Connection-pool settings shared by every executor client.
///
/// Idle conns are capped at 100/host because the default of 2 forces repeated
/// TCP+TLS handshakes on a relay's hot path. `reqwest` reads `HTTP_PROXY` /
/// `HTTPS_PROXY` / `NO_PROXY` from the environment by default.
fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .pool_max_idle_per_host(100)
        .pool_idle_timeout(Duration::from_secs(90))
}

fn build_or_default(builder: reqwest::ClientBuilder) -> reqwest::Client {
    builder.build().unwrap_or_else(|err| {
        tracing::warn!(error = %err, "falling back to the default reqwest client");
        reqwest::Client::new()
    })
}

/// Process-wide HTTP client with **no** whole-request timeout, and therefore a
/// process-wide connection pool.
///
/// A whole-request timeout also bounds *reading the response body*, so any
/// non-zero value silently truncates long-but-healthy streams (an extended o1 /
/// Claude answer). Stall protection comes from [`with_stream_idle_timeout`]
/// instead.
///
/// Callers that want a bounded non-streaming request can either use
/// [`new_http_client`] or scope the cap per request with
/// `RequestBuilder::timeout` — the latter keeps everything on this one pool
/// (`reqwest` has no API for sharing a pool between two `Client`s).
pub fn shared_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| build_or_default(client_builder()))
        .clone()
}

/// Alias for [`shared_client`]: a client with no whole-request timeout.
pub fn streaming_client() -> reqwest::Client {
    shared_client()
}

/// An executor HTTP client whose `timeout` caps the whole request, including
/// reading the response body. For non-streaming calls only.
///
/// This cannot reuse [`shared_client`]'s pool — `reqwest` binds the pool to the
/// `Client` — so prefer `shared_client().post(..).timeout(..)` when the timeout
/// is the only reason you would build a second client.
pub fn new_http_client(timeout: Duration) -> reqwest::Client {
    build_or_default(client_builder().timeout(timeout))
}

/// Turns a configured timeout in seconds into a [`Duration`], falling back to
/// [`DEFAULT_TIMEOUT`] for non-positive values.
#[must_use]
pub fn resolve_timeout(timeout_seconds: i64) -> Duration {
    if timeout_seconds > 0 {
        Duration::from_secs(timeout_seconds as u64)
    } else {
        DEFAULT_TIMEOUT
    }
}

/// Estimates token count from a payload byte size.
#[must_use]
pub fn approximate_tokens_from_bytes(size: usize) -> i64 {
    if size == 0 {
        return 0;
    }
    size.div_ceil(4) as i64
}

/// Clips a failure payload so a persisted error body stays bounded. 4 KiB is
/// more than enough for provider error envelopes.
#[must_use]
pub fn truncate_failure_body(payload: &[u8]) -> String {
    const MAX: usize = 4 * 1024;
    let end = payload.len().min(MAX);
    // Never split a UTF-8 code point; `from_utf8_lossy` also tolerates the
    // non-UTF-8 bodies some upstreams return on infrastructure errors.
    String::from_utf8_lossy(&payload[..end]).into_owned()
}

/// Resolves the upstream-facing model name for a request.
///
/// Prefers the translated [`ProviderRequest::model`] and falls back to the
/// [`REQUESTED_MODEL_METADATA_KEY`] hint the router stored.
#[must_use]
pub fn requested_model(req: &ProviderRequest) -> String {
    let model = req.model.trim();
    if !model.is_empty() {
        return model.to_owned();
    }
    req.metadata
        .get(REQUESTED_MODEL_METADATA_KEY)
        .map(|v| v.trim().to_owned())
        .unwrap_or_default()
}

// --- credential lookup helpers ----------------------------------------------

/// Reads `values[key]` as a string, coercing scalar JSON values. Two
/// deliberate deviations, both narrowing what counts as a credential:
///
/// - Absent is `None` (rather than an empty string that every caller had to
///   compare against), so the check cannot be forgotten.
/// - A JSON `null` is absent (rather than the literal string `"null"`), so it
///   cannot be accepted as a usable credential.
#[must_use]
pub fn string_from_map(values: &Value, key: &str) -> Option<String> {
    let rendered = match values.get(key)? {
        Value::Null => return None,
        Value::String(s) => s.trim().to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            // An integral float is rendered as an integer.
            Some(f) if n.as_i64().is_none() && n.as_u64().is_none() && f.fract() == 0.0 => {
                format!("{}", f as i64)
            }
            _ => n.to_string(),
        },
        other => serde_json::to_string(other).ok()?,
    };
    (!rendered.is_empty()).then_some(rendered)
}

/// Reads `values[parent][key]` as a string, tolerating a `parent` that is a
/// nested object *or* a JSON document embedded in a string.
#[must_use]
pub fn nested_string(values: &Value, parent: &str, key: &str) -> Option<String> {
    match values.get(parent)? {
        v @ Value::Object(_) => string_from_map(v, key),
        Value::String(raw) => {
            let parsed: Value = serde_json::from_str(raw.trim()).ok()?;
            string_from_map(&parsed, key)
        }
        _ => None,
    }
}

// --- include_usage rewrite ---------------------------------------------------

/// Ensures an OpenAI-compatible `chat.completions` payload requests the
/// terminal usage envelope on streaming responses.
///
/// Behaviour (idempotent, additive):
///
/// - If `payload` is not a JSON **object**, it is returned unchanged.
///   `UsagePlugin`'s fallback path compensates when upstream usage cannot be
///   observed, so silently falling through keeps malformed-but-accepted
///   payloads working end-to-end.
/// - If the body does not set `stream` to literal `true`, it is returned
///   unchanged. Non-streaming requests always carry a top-level `usage`
///   envelope in their response, so no hint is necessary.
/// - Otherwise `stream_options.include_usage = true` is set while every other
///   key already under `stream_options` (e.g. `include_input_tokens`) is
///   preserved.
///
/// Defense in depth: even when the caller guarantees
/// [`ProviderRequest::stream`], this helper still verifies the JSON body
/// carries `stream: true` before mutating, so a mis-set flag can never force
/// `include_usage` onto a non-streaming payload.
///
/// A second invocation on the same bytes is a byte-level no-op: `serde_json`'s
/// object representation is a `BTreeMap`, so re-serialising is stable.
#[must_use]
pub fn ensure_include_usage(payload: &[u8]) -> Cow<'_, [u8]> {
    if payload.is_empty() {
        return Cow::Borrowed(payload);
    }
    let Ok(Value::Object(mut body)) = serde_json::from_slice::<Value>(payload) else {
        return Cow::Borrowed(payload);
    };
    if body.get("stream") != Some(&Value::Bool(true)) {
        return Cow::Borrowed(payload);
    }
    let mut opts = match body.remove("stream_options") {
        Some(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    opts.insert("include_usage".to_owned(), Value::Bool(true));
    body.insert("stream_options".to_owned(), Value::Object(opts));
    match serde_json::to_vec(&Value::Object(body)) {
        Ok(out) => Cow::Owned(out),
        Err(_) => Cow::Borrowed(payload),
    }
}

// --- endpoint construction ---------------------------------------------------

/// Builds the `chat/completions` endpoint for an OpenAI-compatible base URL and
/// appends the inbound query parameters.
///
/// The base URL may already point at the full path, at `/v1`, or at the bare
/// origin; all three converge on the same endpoint.
pub fn chat_completions_endpoint(
    base_url: &str,
    query: &[(String, String)],
) -> Result<String, ProviderError> {
    let base = base_url.trim().trim_end_matches('/');
    // Validate the BASE, not the assembled endpoint. `url` is lenient about
    // slashes for special schemes, so a hostless `https://` would otherwise
    // become `https:/v1/chat/completions` and re-parse with `v1` as the host.
    let base_parsed = url::Url::parse(base)
        .map_err(|err| ProviderError::Other(anyhow::anyhow!("invalid base_url: {err}")))?;
    if base_parsed.host_str().unwrap_or_default().is_empty() {
        return Err(ProviderError::Other(anyhow::anyhow!(
            "invalid base_url: missing host"
        )));
    }

    let endpoint = if base.ends_with("/v1/chat/completions") {
        base.to_owned()
    } else if base.ends_with("/v1") {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    };
    let mut parsed = url::Url::parse(&endpoint)
        .map_err(|err| ProviderError::Other(anyhow::anyhow!("invalid base_url: {err}")))?;
    if !query.is_empty() {
        let mut pairs = parsed.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
        drop(pairs);
    }
    Ok(parsed.to_string())
}

// --- stream idle watchdog ----------------------------------------------------

/// The upstream stalled for longer than the idle window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("upstream stream idle timeout")]
pub struct StreamIdleElapsed;

/// Wraps a stream so that a gap longer than `idle` between items terminates it
/// with [`StreamIdleElapsed`].
///
/// A stream *is* the sequence of reads, so the reset is implicit — every
/// yielded item restarts the window, and a zero-length item cannot occur
/// (empty reads are not progress).
///
/// A zero `idle` disables the watchdog and passes the inner stream through
/// untouched.
pub fn with_stream_idle_timeout<S>(
    inner: S,
    idle: Duration,
) -> impl Stream<Item = Result<S::Item, StreamIdleElapsed>> + Send
where
    S: Stream + Send + 'static,
    S::Item: Send,
{
    futures_util::stream::unfold(
        (Box::pin(inner), idle, false),
        |(mut inner, idle, expired)| async move {
            if expired {
                return None;
            }
            if idle.is_zero() {
                return inner.next().await.map(|it| (Ok(it), (inner, idle, false)));
            }
            match tokio::time::timeout(idle, inner.next()).await {
                Ok(Some(item)) => Some((Ok(item), (inner, idle, false))),
                Ok(None) => None,
                Err(_) => Some((Err(StreamIdleElapsed), (inner, idle, true))),
            }
        },
    )
}

// --- streamed usage accumulation ---------------------------------------------

/// Assembles a [`StreamResponse`] from an accepted upstream response.
///
/// The single reason this is a function rather than three lines inlined five
/// times: the header clone **must** happen before the body is consumed, and
/// `bytes_stream()` takes the response by value. Doing it here makes that
/// ordering unrepeatable-by-accident, and gives the five executors one tested
/// seam. `build` receives the response and the status it reported, so error
/// chunks can carry the status the relay actually ran under.
///
/// Callers must have rejected a failing status already — see
/// [`crate::types::Provider::execute_stream`].
pub fn stream_response<F>(response: reqwest::Response, build: F) -> StreamResponse
where
    F: FnOnce(reqwest::Response, u16) -> Pin<Box<dyn Stream<Item = StreamChunk> + Send>>,
{
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    StreamResponse {
        status,
        headers,
        chunks: build(response, status),
    }
}

enum UsagePhase {
    Streaming,
    Trailing,
    Done,
}

/// The per-stream constants [`usage_stream`] threads through every chunk.
///
/// A struct rather than four more tuple slots: the `unfold` state already
/// carries the reader, the buffer and the phase, and a six-field tuple stops
/// being readable at the destructuring site.
struct UsageStreamMeta {
    model: String,
    provider: &'static str,
    parse: fn(&[u8]) -> Option<UsageTokens>,
    /// Status of the response being relayed, stamped onto any error chunk.
    upstream_status: u16,
}

/// Turns a raw upstream byte stream into the [`StreamChunk`] sequence the
/// [`crate::types::Provider`] trait promises: every payload is forwarded
/// verbatim, and a terminal [`StreamChunk::Usage`] is emitted iff `parse` found
/// a usage envelope in the accumulated body.
///
/// Two behavioural notes:
///
/// - The *absence* of a `StreamChunk::Usage` carries the "nothing parsed"
///   signal, so `strict_usage_metadata_mode` still has its "absent, not zero"
///   input.
/// - Memory stays bounded by [`StreamUsageBuffer`] regardless of body length.
pub fn usage_stream<S>(
    body: S,
    idle: Duration,
    model: String,
    provider: &'static str,
    parse: fn(&[u8]) -> Option<UsageTokens>,
    upstream_status: u16,
) -> Pin<Box<dyn Stream<Item = StreamChunk> + Send>>
where
    S: Stream<Item = reqwest::Result<Bytes>> + Send + 'static,
{
    let guarded: Pin<Box<dyn Stream<Item = _> + Send>> =
        Box::pin(with_stream_idle_timeout(body, idle));
    let state = (
        guarded,
        StreamUsageBuffer::new(),
        UsagePhase::Streaming,
        UsageStreamMeta {
            model,
            provider,
            parse,
            upstream_status,
        },
    );

    Box::pin(futures_util::stream::unfold(
        state,
        |(mut inner, mut buf, mut phase, meta)| async move {
            loop {
                match phase {
                    UsagePhase::Done => return None,
                    UsagePhase::Trailing => {
                        let record = (meta.parse)(&buf.bytes())
                            .map(|t| t.to_record(&meta.model, meta.provider));
                        let state = (inner, buf, UsagePhase::Done, meta);
                        return record.map(|rec| (StreamChunk::Usage(rec), state));
                    }
                    UsagePhase::Streaming => match inner.next().await {
                        Some(Ok(Ok(chunk))) => {
                            buf.write(&chunk);
                            let state = (inner, buf, UsagePhase::Streaming, meta);
                            return Some((StreamChunk::Payload(chunk), state));
                        }
                        // Transport error mid-stream: surface it, then still try
                        // to bill whatever usage the truncated body carried.
                        Some(Ok(Err(err))) => {
                            let chunk = StreamChunk::Error {
                                status: Some(meta.upstream_status),
                                message: err.to_string(),
                            };
                            return Some((chunk, (inner, buf, UsagePhase::Trailing, meta)));
                        }
                        Some(Err(elapsed)) => {
                            let chunk = StreamChunk::Error {
                                status: Some(meta.upstream_status),
                                message: elapsed.to_string(),
                            };
                            return Some((chunk, (inner, buf, UsagePhase::Trailing, meta)));
                        }
                        None => {
                            phase = UsagePhase::Trailing;
                        }
                    },
                }
            }
        },
    ))
}

#[cfg(test)]
#[path = "common_tests.rs"]
mod tests;
