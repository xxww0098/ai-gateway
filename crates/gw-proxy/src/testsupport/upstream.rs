//! Doubles for the upstream side: credential store, providers, catalogue.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::Utc;
use gw_authcore::{AuthRecord, AuthStore};
use gw_provider::types::{
    Provider, ProviderError, ProviderRequest, ProviderResponse, StreamChunk, StreamResponse,
    UsageRecord,
};
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

/// Scripted upstream. Each call pops the next queued outcome, so a test can
/// stage "first account fails, second succeeds" for the failover assertions.
pub(crate) struct FakeProvider {
    pub(crate) name: &'static str,
    pub(crate) outcomes: Mutex<Vec<Result<ProviderResponse, ProviderError>>>,
    pub(crate) stream_chunks: Mutex<Vec<StreamChunk>>,
    pub(crate) stream_headers: Mutex<http::HeaderMap>,
    pub(crate) calls: AtomicUsize,
    pub(crate) seen_auth_ids: Mutex<Vec<String>>,
    /// Every request as the dispatcher handed it over. The Gemini dialect
    /// carries its model and its streaming flag in the URL and neither in the
    /// body, and the tenant credential rides on headers the kernel is supposed
    /// to consume — none of which is observable anywhere else.
    pub(crate) seen_requests: Mutex<Vec<ProviderRequest>>,
}

impl FakeProvider {
    pub(crate) fn new(name: &'static str) -> Arc<Self> {
        Arc::new(Self {
            name,
            outcomes: Mutex::new(Vec::new()),
            stream_chunks: Mutex::new(Vec::new()),
            stream_headers: Mutex::new(http::HeaderMap::new()),
            calls: AtomicUsize::new(0),
            seen_auth_ids: Mutex::new(Vec::new()),
            seen_requests: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn queue(self: &Arc<Self>, outcome: Result<ProviderResponse, ProviderError>) {
        self.outcomes.lock().push(outcome);
    }

    pub(crate) fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// `(model, stream)` per call, which is what most routing assertions want.
    pub(crate) fn dispatched(&self) -> Vec<(String, bool)> {
        self.seen_requests
            .lock()
            .iter()
            .map(|r| (r.model.clone(), r.stream))
            .collect()
    }

    /// The single request this provider was handed. Panics if it was not
    /// called exactly once, so a test cannot silently assert against the
    /// wrong attempt.
    pub(crate) fn only_request(&self) -> ProviderRequest {
        let seen = self.seen_requests.lock();
        assert_eq!(seen.len(), 1, "expected exactly one upstream call");
        seen[0].clone()
    }
}

/// A 200 response carrying the given token counts.
pub(crate) fn ok_response(input: i64, output: i64) -> ProviderResponse {
    ProviderResponse {
        status: 200,
        headers: http::HeaderMap::new(),
        body: bytes::Bytes::from_static(br#"{"ok":true}"#),
        usage: Some(UsageRecord {
            model: "test-model".to_owned(),
            provider: "openai".to_owned(),
            input_tokens: Some(input),
            output_tokens: Some(output),
            cached_tokens: None,
            reasoning_tokens: None,
        }),
    }
}

/// A 200 response with no usage envelope, driving the fallback / strict paths.
pub(crate) fn ok_response_without_usage() -> ProviderResponse {
    ProviderResponse {
        status: 200,
        headers: http::HeaderMap::new(),
        body: bytes::Bytes::from_static(br#"{"ok":true}"#),
        usage: None,
    }
}

#[async_trait]
impl Provider for FakeProvider {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn execute(
        &self,
        auth: &AuthRecord,
        req: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen_auth_ids.lock().push(auth.id.clone());
        self.seen_requests.lock().push(req);
        let mut outcomes = self.outcomes.lock();
        if outcomes.is_empty() {
            return Ok(ok_response(10, 20));
        }
        outcomes.remove(0)
    }

    async fn execute_stream(
        &self,
        auth: &AuthRecord,
        req: ProviderRequest,
    ) -> Result<StreamResponse, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen_auth_ids.lock().push(auth.id.clone());
        self.seen_requests.lock().push(req);
        {
            let mut outcomes = self.outcomes.lock();
            if !outcomes.is_empty()
                && let Err(err) = outcomes.remove(0)
            {
                return Err(err);
            }
        }
        let chunks = self.stream_chunks.lock().clone();
        let headers = self.stream_headers.lock().clone();
        Ok(StreamResponse {
            status: 200,
            headers,
            chunks: Box::pin(futures_util::stream::iter(chunks)),
        })
    }

    async fn refresh(&self, auth: &AuthRecord) -> Result<AuthRecord, ProviderError> {
        Ok(auth.clone())
    }

    async fn count_tokens(
        &self,
        _auth: &AuthRecord,
        _req: ProviderRequest,
    ) -> Result<i64, ProviderError> {
        Ok(42)
    }
}

#[derive(Default)]
pub(crate) struct FakeCatalog {
    pub(crate) models: Mutex<Vec<ModelEntry>>,
}

#[async_trait]
impl ModelCatalog for FakeCatalog {
    async fn list_models(&self) -> anyhow::Result<Vec<ModelEntry>> {
        Ok(self.models.lock().clone())
    }
}
