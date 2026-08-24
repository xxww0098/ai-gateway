//! Shared provider contract: the planning *input* and the usage *output*.
//!
//! What is deliberately absent is a response type. `gw-provider` no longer
//! sees upstream responses — [`crate::RoutePlanner`] hands a plan to
//! `gw-relay`, and the bytes come back through the relay's own
//! [`gw_relay::RelayResponse`]. A `ProviderResponse` here would be the seam a
//! second HTTP path grows back through.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A normalised upstream request. `payload` is the raw JSON body in the
/// *caller's* dialect; each provider translates it to its own wire format.
#[derive(Debug, Clone, Default)]
pub struct ProviderRequest {
    pub model: String,
    /// WAVE 3 接缝：`Vec<u8>` → [`bytes::Bytes`]。
    ///
    /// `Bytes::clone` 是 refcount，所以 failover 重试不再重复 memcpy 整个请求体。
    /// 实测代价见 `docs/relay-perf-baseline.md` §3 第 1 条：分配字节达载荷的
    /// **4.20 倍**（下界 1.40 倍），一个 900 KB 的请求在 3 次 failover 下要
    /// memcpy 约 5.4 MB。
    pub payload: bytes::Bytes,
    pub stream: bool,
    pub metadata: HashMap<String, String>,

    /// Inbound headers. Forwarded by `gw-relay`, which owns the hop-by-hop
    /// denylist and the credential strip — this crate must not filter them a
    /// second time, or the two lists drift.
    pub headers: http::HeaderMap,

    /// Inbound query parameters, **appended** to the provider's endpoint. A
    /// `Vec` of pairs rather than a map because order and duplicate keys are
    /// both significant.
    /// A provider may still `set` a key it owns afterwards — Gemini and Vertex
    /// force `alt=sse` on streaming that way.
    pub query: Vec<(String, String)>,
}

/// Token accounting extracted from an upstream response. `None` fields mean the
/// upstream omitted the value — the billing pipeline distinguishes "absent"
/// from "zero" (strict-usage-metadata mode depends on it).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageRecord {
    pub model: String,
    pub provider: String,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("upstream {status}: {body}")]
    Upstream { status: u16, body: String },
    #[error("credential unusable: {0}")]
    Credential(String),
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
