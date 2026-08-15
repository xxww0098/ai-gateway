//! Doubles for the Redis-backed collaborators and the metrics sink.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::ports::{
    ChannelPolicy, ChannelPolicyStore, CircuitBreaker, Id, IdempotencyStore, RateLimiter,
};

#[derive(Default)]
pub(crate) struct FakeRateLimiter {
    pub(crate) allow: Mutex<bool>,
    pub(crate) released: Mutex<Vec<String>>,
    pub(crate) errors: Mutex<bool>,
}

impl FakeRateLimiter {
    pub(crate) fn allowing() -> Arc<Self> {
        let rl = Self::default();
        *rl.allow.lock() = true;
        Arc::new(rl)
    }
}

#[async_trait]
impl RateLimiter for FakeRateLimiter {
    async fn allow(
        &self,
        _identity: &str,
        _tokens: i64,
        _model: &str,
        _group_id: Option<Id>,
    ) -> anyhow::Result<(bool, Option<String>)> {
        if *self.errors.lock() {
            anyhow::bail!("redis down");
        }
        Ok((*self.allow.lock(), Some("slot-1".to_owned())))
    }

    async fn release_concurrency(&self, _identity: &str, release_id: &str) -> anyhow::Result<()> {
        self.released.lock().push(release_id.to_owned());
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct FakeCircuitBreaker {
    pub(crate) allow: Mutex<bool>,
    pub(crate) recorded: Mutex<Vec<(String, bool)>>,
}

impl FakeCircuitBreaker {
    pub(crate) fn closed() -> Arc<Self> {
        let cb = Self::default();
        *cb.allow.lock() = true;
        Arc::new(cb)
    }
}

#[async_trait]
impl CircuitBreaker for FakeCircuitBreaker {
    async fn allow(&self, _provider: &str) -> anyhow::Result<bool> {
        Ok(*self.allow.lock())
    }

    async fn record(&self, provider: &str, success: bool) {
        self.recorded.lock().push((provider.to_owned(), success));
    }
}

#[derive(Default)]
pub(crate) struct FakeIdempotencyStore {
    pub(crate) entries: Mutex<HashMap<String, Vec<u8>>>,
}

impl FakeIdempotencyStore {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait]
impl IdempotencyStore for FakeIdempotencyStore {
    async fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(self.entries.lock().get(key).cloned())
    }

    async fn set(&self, key: &str, value: Vec<u8>, _ttl: Duration) -> anyhow::Result<()> {
        self.entries.lock().insert(key.to_owned(), value);
        Ok(())
    }

    async fn set_nx(&self, key: &str, value: Vec<u8>, _ttl: Duration) -> anyhow::Result<bool> {
        let mut entries = self.entries.lock();
        if entries.contains_key(key) {
            return Ok(false);
        }
        entries.insert(key.to_owned(), value);
        Ok(true)
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.entries.lock().remove(key);
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct FakePolicyStore {
    pub(crate) policies: Mutex<Vec<ChannelPolicy>>,
}

#[async_trait]
impl ChannelPolicyStore for FakePolicyStore {
    async fn list_channel_policies(&self) -> anyhow::Result<Vec<ChannelPolicy>> {
        Ok(self.policies.lock().clone())
    }
}

/// Records the gauges the crate publishes, standing in for
/// `gw_server::Metrics`.
#[derive(Debug, Default)]
pub(crate) struct RecordingMetrics {
    pub(crate) channel_benched: AtomicI64,
    pub(crate) orphaned_holds: AtomicI64,
}

impl RecordingMetrics {
    pub(crate) fn benched(&self) -> i64 {
        self.channel_benched.load(Ordering::SeqCst)
    }

    pub(crate) fn orphaned(&self) -> i64 {
        self.orphaned_holds.load(Ordering::SeqCst)
    }
}

impl crate::ports::MetricsSink for RecordingMetrics {
    fn set_channel_benched(&self, count: i64) {
        self.channel_benched.store(count, Ordering::SeqCst);
    }

    fn set_orphaned_holds(&self, count: i64) {
        self.orphaned_holds.store(count, Ordering::SeqCst);
    }
}
