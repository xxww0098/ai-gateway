//! Response-side protocol translation and its usage side channel.
//!
//! A translated request gets exactly one state machine. It owns both frame
//! conversion and usage extraction, so translated streams never attach the
//! byte-scanning probe a second time.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
use gw_relay::probe::{SseUsageProbe, UsageHandle as ProbeHandle, UsageShape};
use gw_relay::{
    RelayError, RelayResponse, RelayResponseBody, RelayUsage, StreamTranslator, TranslateError,
    Translator, UpstreamDialect, UsageProbe,
};
use http::header::{
    ACCEPT_RANGES, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG,
};
use http::{HeaderMap, HeaderValue};
use http_body::{Body as HttpBody, Frame, SizeHint};
use http_body_util::BodyExt as _;

const MAX_TRANSLATED_UNARY_RESPONSE: usize = 16 * 1024 * 1024;

/// One three-state usage result, regardless of whether it came from the
/// passthrough probe or a protocol translator.
#[derive(Clone)]
pub(super) enum UsageHandle {
    Probe(ProbeHandle),
    Shared(SharedUsage),
}

impl UsageHandle {
    #[must_use]
    pub(super) fn get(&self) -> Option<Option<RelayUsage>> {
        match self {
            Self::Probe(handle) => handle.get(),
            Self::Shared(shared) => shared.get(),
        }
    }

    fn pending() -> (Self, SharedUsage) {
        let shared = SharedUsage::default();
        (Self::Shared(shared.clone()), shared)
    }

    fn completed(usage: Option<RelayUsage>) -> Self {
        let shared = SharedUsage::default();
        shared.set_once(usage);
        Self::Shared(shared)
    }
}

#[derive(Clone, Default)]
pub(super) struct SharedUsage(Arc<Mutex<Option<Option<RelayUsage>>>>);

impl SharedUsage {
    fn get(&self) -> Option<Option<RelayUsage>> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_once(&self, usage: Option<RelayUsage>) {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(usage);
        }
    }
}

/// Passthrough attempts keep the existing side-band parser.
pub(super) fn usage_probe(dialect: UpstreamDialect) -> (Box<dyn UsageProbe>, UsageHandle) {
    let (probe, handle) = SseUsageProbe::new(usage_shape(dialect));
    (Box::new(probe), UsageHandle::Probe(handle))
}

fn usage_shape(dialect: UpstreamDialect) -> UsageShape {
    match dialect {
        UpstreamDialect::OpenAiChat | UpstreamDialect::OpenAiResponses => UsageShape::OpenAi,
        UpstreamDialect::AnthropicMessages => UsageShape::Anthropic,
        UpstreamDialect::GoogleGenerateContent => UsageShape::Google,
    }
}

/// Applies response translation and returns the handle settlement must read
/// after the client-facing body ends.
pub(super) async fn prepare_response(
    mut response: RelayResponse,
    passthrough_handle: Option<UsageHandle>,
    translator: Option<&'static dyn Translator>,
    requested_stream: bool,
    upstream: UpstreamDialect,
) -> Result<(RelayResponse, UsageHandle), TranslateError> {
    let Some(translator) = translator else {
        let handle = passthrough_handle.ok_or_else(|| {
            TranslateError::UpstreamShape("passthrough usage probe handle is missing".to_owned())
        })?;
        return Ok((response, handle));
    };

    if response.status.is_success() && requested_stream && is_event_stream(&response.headers) {
        let (handle, sink) = UsageHandle::pending();
        let body = TranslatedBody::new(
            response.body.into_http_body(),
            translator.stream_translator(),
            sink,
        )
        .boxed();
        response.body = RelayResponseBody::Stream(body);
        rewrite_entity_headers(&mut response.headers, true);
        return Ok((response, handle));
    }

    let original = collect_bounded(response.body).await?;
    let usage = parse_usage(upstream, &original);
    let translated = match translator.translate_response(&original) {
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
}

fn parse_usage(dialect: UpstreamDialect, body: &Bytes) -> Option<RelayUsage> {
    let (mut probe, _) = SseUsageProbe::new(usage_shape(dialect));
    probe.observe(body);
    Box::new(probe).finish()
}

async fn collect_bounded(body: RelayResponseBody) -> Result<Bytes, TranslateError> {
    let mut body = body.into_http_body();
    let mut chunks = Vec::new();
    let mut total = 0usize;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|err| TranslateError::UpstreamShape(err.to_string()))?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        total = total.saturating_add(data.len());
        if total > MAX_TRANSLATED_UNARY_RESPONSE {
            return Err(TranslateError::UpstreamShape(
                "translated unary response exceeds 16 MiB".to_owned(),
            ));
        }
        chunks.push(data);
    }
    if chunks.len() == 1 {
        return Ok(chunks.pop().unwrap_or_default());
    }
    let mut out = BytesMut::with_capacity(total);
    for chunk in chunks {
        out.extend_from_slice(&chunk);
    }
    Ok(out.freeze())
}

fn is_event_stream(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .trim_start()
                .get(.."text/event-stream".len())
                .is_some_and(|head| head.eq_ignore_ascii_case("text/event-stream"))
        })
}

fn rewrite_entity_headers(headers: &mut HeaderMap, stream: bool) {
    for name in [
        CONTENT_LENGTH,
        CONTENT_ENCODING,
        ETAG,
        CONTENT_RANGE,
        ACCEPT_RANGES,
    ] {
        headers.remove(name);
    }
    for name in ["content-md5", "digest"] {
        headers.remove(name);
    }
    headers.insert(
        CONTENT_TYPE,
        if stream {
            HeaderValue::from_static("text/event-stream")
        } else {
            HeaderValue::from_static("application/json")
        },
    );
}

struct TranslatedBody {
    state: Mutex<TranslatedState>,
}

struct TranslatedState {
    inner: http_body_util::combinators::BoxBody<Bytes, RelayError>,
    translator: Box<dyn StreamTranslator>,
    pending: VecDeque<Bytes>,
    usage: SharedUsage,
    done: bool,
}

impl TranslatedBody {
    fn new(
        inner: http_body_util::combinators::BoxBody<Bytes, RelayError>,
        translator: Box<dyn StreamTranslator>,
        usage: SharedUsage,
    ) -> Self {
        Self {
            state: Mutex::new(TranslatedState {
                inner,
                translator,
                pending: VecDeque::new(),
                usage,
                done: false,
            }),
        }
    }
}

impl HttpBody for TranslatedBody {
    type Data = Bytes;
    type Error = RelayError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let mut state = self
            .get_mut()
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        loop {
            if let Some(bytes) = state.pending.pop_front() {
                return Poll::Ready(Some(Ok(Frame::data(bytes))));
            }
            if state.done {
                return Poll::Ready(None);
            }

            match Pin::new(&mut state.inner).poll_frame(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(frame))) => match frame.into_data() {
                    Ok(data) => match state.translator.push(&data) {
                        Ok(frames) => {
                            state.pending.extend(frames);
                        }
                        Err(err) => {
                            let usage = state.translator.usage();
                            state.usage.set_once(usage);
                            state.done = true;
                            return Poll::Ready(Some(Err(RelayError::Translate(err.to_string()))));
                        }
                    },
                    Err(frame) => return Poll::Ready(Some(Ok(frame))),
                },
                Poll::Ready(Some(Err(err))) => {
                    let usage = state.translator.usage();
                    state.usage.set_once(usage);
                    state.done = true;
                    return Poll::Ready(Some(Err(err)));
                }
                Poll::Ready(None) => {
                    let frames = match state.translator.finish() {
                        Ok(frames) => frames,
                        Err(err) => {
                            let usage = state.translator.usage();
                            state.usage.set_once(usage);
                            state.done = true;
                            return Poll::Ready(Some(Err(RelayError::Translate(err.to_string()))));
                        }
                    };
                    let usage = state.translator.usage();
                    state.usage.set_once(usage);
                    state.pending.extend(frames);
                    state.done = true;
                }
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.done && state.pending.is_empty()
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}

impl Drop for TranslatedBody {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        state.usage.set_once(state.translator.usage());
    }
}
