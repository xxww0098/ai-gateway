//! Doubles for the upstream side: credential store, providers, catalogue.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::Utc;
use gw_authcore::{AuthRecord, AuthStore};
use gw_provider::route::{RoutePlan, RoutePlanner};
use gw_provider::types::{ProviderError, ProviderRequest};
use gw_relay::engine::{Transport, UpstreamHead, UpstreamRequest};
use gw_relay::{Credential, RelayError, RelayTimeouts, RelayTransportError, UpstreamDialect};
use parking_lot::Mutex;

use crate::ports::{ModelCatalog, ModelEntry};

#[derive(Default)]
pub(crate) struct FakeAuthStore {
    pub(crate) records: Mutex<Vec<AuthRecord>>,
    /// `list()` 被调用了多少次。真实后端上这一次是「全表 SELECT + 每行一次
    /// AES-GCM 解密」，所以「每请求几次」是热点 #5 的可量化代理指标。
    list_calls: AtomicUsize,
}

impl FakeAuthStore {
    pub(crate) fn with(records: Vec<AuthRecord>) -> Arc<Self> {
        Arc::new(Self {
            records: Mutex::new(records),
            list_calls: AtomicUsize::new(0),
        })
    }

    pub(crate) fn list_calls(&self) -> usize {
        self.list_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl AuthStore for FakeAuthStore {
    async fn list(&self) -> anyhow::Result<Vec<AuthRecord>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.records.lock().clone())
    }

    async fn get(&self, id: &str) -> anyhow::Result<Option<AuthRecord>> {
        Ok(self.records.lock().iter().find(|r| r.id == id).cloned())
    }

    async fn save(&self, record: &AuthRecord) -> anyhow::Result<()> {
        self.records.lock().push(record.clone());
        Ok(())
    }

    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.records.lock().retain(|r| r.id != id);
        Ok(())
    }
}

/// Builds an active credential for `provider`.
pub(crate) fn auth_record(id: &str, provider: &str) -> AuthRecord {
    AuthRecord::new(id, provider, Utc::now())
}

/// A planner that always points at one fixed test origin.
///
/// It records what the dispatcher handed over — the Gemini dialect carries its
/// model and its streaming flag in the URL and neither in the body, and the
/// tenant credential rides on headers the kernel is supposed to consume, none
/// of which is observable anywhere else.
pub(crate) struct FakePlanner {
    pub(crate) name: &'static str,
    pub(crate) seen_auth_ids: Mutex<Vec<String>>,
    pub(crate) seen_requests: Mutex<Vec<ProviderRequest>>,
    /// When set, `plan` fails instead of producing a plan — a broken account.
    pub(crate) plan_fails: Mutex<bool>,
}

impl FakePlanner {
    pub(crate) fn new(name: &'static str) -> Arc<Self> {
        Arc::new(Self {
            name,
            seen_auth_ids: Mutex::new(Vec::new()),
            seen_requests: Mutex::new(Vec::new()),
            plan_fails: Mutex::new(false),
        })
    }

    pub(crate) fn call_count(&self) -> usize {
        self.seen_requests.lock().len()
    }

    /// `(model, stream)` per call, which is what most routing assertions want.
    pub(crate) fn dispatched(&self) -> Vec<(String, bool)> {
        self.seen_requests
            .lock()
            .iter()
            .map(|r| (r.model.clone(), r.stream))
            .collect()
    }

    /// The single request this planner was handed. Panics if it was not
    /// called exactly once, so a test cannot silently assert against the
    /// wrong attempt.
    pub(crate) fn only_request(&self) -> ProviderRequest {
        let seen = self.seen_requests.lock();
        assert_eq!(seen.len(), 1, "expected exactly one upstream call");
        seen[0].clone()
    }
}

/// The origin every [`FakePlanner`] points at. Nothing resolves it — the fake
/// transport answers before DNS would.
pub(crate) const FAKE_UPSTREAM: &str = "https://upstream.test";

#[async_trait]
impl RoutePlanner for FakePlanner {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn plan(
        &self,
        auth: &AuthRecord,
        req: &ProviderRequest,
    ) -> Result<RoutePlan, ProviderError> {
        self.seen_auth_ids.lock().push(auth.id.clone());
        self.seen_requests.lock().push(req.clone());
        if *self.plan_fails.lock() {
            return Err(ProviderError::Credential("planner refused".to_owned()));
        }
        Ok(RoutePlan {
            provider: self.name,
            endpoint: url::Url::parse(&format!("{FAKE_UPSTREAM}/v1/chat/completions"))
                .expect("a valid endpoint"),
            credential: Credential::Bearer("test-token".to_owned()),
            headers: http::HeaderMap::new(),
            body: None,
            timeouts: RelayTimeouts::default(),
            dialect: match self.name {
                "claude" => UpstreamDialect::AnthropicMessages,
                "gemini" | "vertex" => UpstreamDialect::GoogleGenerateContent,
                _ => UpstreamDialect::OpenAiChat,
            },
        })
    }

    async fn plan_count_tokens(
        &self,
        auth: &AuthRecord,
        req: &ProviderRequest,
    ) -> Result<RoutePlan, ProviderError> {
        let mut plan = self.plan(auth, req).await?;
        plan.endpoint = url::Url::parse(&format!("{FAKE_UPSTREAM}/v1/messages/count_tokens"))
            .expect("a valid endpoint");
        Ok(plan)
    }

    async fn refresh(&self, auth: &AuthRecord) -> Result<AuthRecord, ProviderError> {
        Ok(auth.clone())
    }
}

/// One canned upstream answer.
pub(crate) struct CannedResponse {
    pub(crate) status: u16,
    pub(crate) headers: http::HeaderMap,
    /// Frames, in order. A single-element vec is the non-streaming shape.
    pub(crate) frames: Vec<bytes::Bytes>,
}

impl CannedResponse {
    /// A 200 whose body carries an OpenAI usage envelope, so the *real*
    /// side-band probe extracts the counts exactly as it would in production.
    pub(crate) fn ok(input: i64, output: i64) -> Self {
        Self {
            status: 200,
            headers: http::HeaderMap::new(),
            frames: vec![bytes::Bytes::from(format!(
                r#"{{"ok":true,"usage":{{"prompt_tokens":{input},"completion_tokens":{output}}}}}"#
            ))],
        }
    }

    /// A 200 with no usage envelope, driving the fallback / strict paths.
    pub(crate) fn ok_without_usage() -> Self {
        Self {
            status: 200,
            headers: http::HeaderMap::new(),
            frames: vec![bytes::Bytes::from_static(br#"{"ok":true}"#)],
        }
    }

    /// A non-2xx the upstream *answered* with — still a response, headers and
    /// all.
    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            headers: http::HeaderMap::new(),
            frames: vec![bytes::Bytes::from_static(br#"{"error":"upstream"}"#)],
        }
    }

    pub(crate) fn sse(frames: &[&'static str]) -> Self {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("text/event-stream"),
        );
        Self {
            status: 200,
            headers,
            frames: frames
                .iter()
                .map(|f| bytes::Bytes::from_static(f.as_bytes()))
                .collect(),
        }
    }
}

/// 一次出网请求里测试需要断言的东西。
pub(crate) struct SeenRequest {
    pub(crate) headers: http::HeaderMap,
    /// 上游最终收到的**全部**字节。
    pub(crate) body: bytes::Bytes,
    /// 这份 body 是边收边转的（[`gw_relay::RelayBody::Streaming`]）还是已缓冲的。
    /// 「超阈值的请求有没有被网关重新缓冲一遍」只能在这里看出来。
    pub(crate) streamed: bool,
}

/// A scripted [`Transport`], so dispatch tests drive the **real**
/// [`RelayEngine`] — probe guard, frame forwarding, idle watchdog and all —
/// without a socket.
#[derive(Default)]
pub(crate) struct FakeTransport {
    pub(crate) outcomes: Mutex<Vec<Result<CannedResponse, String>>>,
    pub(crate) calls: AtomicUsize,
    /// Every outbound request, for header, URL and body assertions.
    pub(crate) seen: Mutex<Vec<SeenRequest>>,
}

impl FakeTransport {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The engine takes ownership of its transport, so the shared handle is
    /// wrapped in a local newtype (the orphan rule forbids implementing
    /// `Transport` for `Arc<T>` directly).
    pub(crate) fn wired(self: &Arc<Self>) -> WiredTransport {
        WiredTransport(Arc::clone(self))
    }

    /// Queues one answer. Calls beyond the queue get a default 200 with usage.
    pub(crate) fn queue(self: &Arc<Self>, outcome: Result<CannedResponse, String>) {
        self.outcomes.lock().push(outcome);
    }

    pub(crate) fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// The headers of the single outbound request. Panics unless there was
    /// exactly one.
    pub(crate) fn only_headers(&self) -> http::HeaderMap {
        let seen = self.seen.lock();
        assert_eq!(seen.len(), 1, "expected exactly one upstream request");
        seen[0].headers.clone()
    }

    /// `(body, streamed)` of the single outbound request. Panics unless there
    /// was exactly one.
    pub(crate) fn only_body(&self) -> (bytes::Bytes, bool) {
        let seen = self.seen.lock();
        assert_eq!(seen.len(), 1, "expected exactly one upstream request");
        (seen[0].body.clone(), seen[0].streamed)
    }
}

/// [`FakeTransport`] as the engine's owned transport.
pub(crate) struct WiredTransport(Arc<FakeTransport>);

#[async_trait]
impl Transport for WiredTransport {
    async fn send(&self, req: UpstreamRequest) -> Result<UpstreamHead, RelayTransportError> {
        let this = &self.0;
        this.calls.fetch_add(1, Ordering::SeqCst);
        let headers = req.headers.clone();
        // 先把 body 抽干再上锁：跨 await 持有 `parking_lot` 的守卫会让这个
        // future 不再是 `Send`，引擎就挂不上去了。
        let streamed = !matches!(req.body, gw_relay::RelayBody::Buffered(_));
        let body = match req.body {
            gw_relay::RelayBody::Buffered(bytes) => bytes,
            gw_relay::RelayBody::Streaming(body) => http_body_util::BodyExt::collect(body)
                .await
                .expect("出网 body 不该失败")
                .to_bytes(),
        };
        this.seen.lock().push(SeenRequest {
            headers,
            body,
            streamed,
        });
        let outcome = {
            let mut outcomes = this.outcomes.lock();
            if outcomes.is_empty() {
                Ok(CannedResponse::ok(10, 20))
            } else {
                outcomes.remove(0)
            }
        };
        let canned = match outcome {
            Ok(canned) => canned,
            // The transport could not reach an upstream at all.
            Err(_) => return Err(RelayTransportError::BadTarget("fake transport".to_owned())),
        };
        let frames = futures_util::stream::iter(
            canned
                .frames
                .into_iter()
                .map(|f| Ok::<_, RelayError>(http_body::Frame::data(f))),
        );
        Ok(UpstreamHead {
            status: http::StatusCode::from_u16(canned.status).expect("a valid status"),
            headers: canned.headers,
            body: http_body_util::BodyExt::boxed(http_body_util::StreamBody::new(frames)),
        })
    }
}

#[derive(Default)]
pub(crate) struct FakeCatalog {
    pub(crate) models: Mutex<Vec<ModelEntry>>,
    /// How many times the listing was walked. A detail request that increments
    /// this is scanning the whole catalogue to find one id.
    list_calls: AtomicUsize,
    get_calls: AtomicUsize,
}

impl FakeCatalog {
    pub(crate) fn list_calls(&self) -> usize {
        self.list_calls.load(Ordering::SeqCst)
    }

    pub(crate) fn get_calls(&self) -> usize {
        self.get_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ModelCatalog for FakeCatalog {
    async fn list_models(&self) -> anyhow::Result<Vec<ModelEntry>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.models.lock().clone())
    }

    async fn get_model(&self, id: &str) -> anyhow::Result<Option<ModelEntry>> {
        self.get_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .models
            .lock()
            .iter()
            .find(|model| model.id == id)
            .cloned())
    }
}
