//! Route planning — what this crate produces instead of HTTP.
//!
//! `gw-provider` used to *be* the HTTP client: five executors, five `reqwest`
//! pools, five copies of "read the response, look for usage, turn a 4xx into an
//! error". `gw-relay` is the byte-level pass-through engine, and two engines is
//! one too many — the provider copies lost headers on non-2xx, collapsed
//! multi-valued `set-cookie`, and reported a mid-stream failure as a clean EOF.
//!
//! So the split is: **this crate decides where a request goes and who signs
//! it; `gw-relay` moves the bytes.** A [`RoutePlan`] is that decision, and it
//! is a plain value — no sockets, no futures, no client.
//!
//! ```text
//!   ProviderRequest ──plan()──► RoutePlan ──► gw_relay::RelayEngine::relay()
//!   (model, body, headers,      (endpoint,     (the only inference HTTP
//!    query, surface)             credential,    exit in the workspace)
//!                                headers, body)
//! ```

use bytes::Bytes;
use gw_relay::endpoint::include_usage::Spliced;
use gw_relay::{Credential, RelayTimeouts, UpstreamDialect};
use http::HeaderMap;
use http::uri::PathAndQuery;
use url::Url;

use crate::types::ProviderError;

/// One planned upstream attempt.
///
/// Everything here is decided without touching the network. The relay turns it
/// into exactly one HTTP request.
#[derive(Debug, Clone)]
pub struct RoutePlan {
    /// Registered executor name (`openai`, `claude`, ...), for logs and the
    /// channel-health bookkeeping.
    pub provider: &'static str,
    /// The fully-assembled upstream URL: the account's base URL, the endpoint
    /// the *inbound surface* selects, and the inbound query.
    ///
    /// Assembled here rather than in the relay because "which leaf does this
    /// upstream expose" is provider knowledge; the relay deliberately has none.
    pub endpoint: Url,
    /// The upstream credential. The relay strips whatever the client sent and
    /// sets this instead — one credential carrier, one place.
    pub credential: Credential,
    /// Headers the provider owns: content negotiation, `anthropic-version`,
    /// and so on. Merged over the forwarded inbound headers.
    ///
    /// **No credential header belongs here.** That is [`Self::credential`]'s
    /// job, and the relay is what marks it sensitive.
    pub headers: HeaderMap,
    /// A rewritten request body, or `None` to forward the inbound bytes
    /// untouched. The only rewrite that exists today is the `stream_options.
    /// include_usage` splice on OpenAI-shaped streams.
    pub body: Option<Bytes>,
    pub timeouts: RelayTimeouts,
    /// The wire protocol this upstream speaks.
    pub dialect: UpstreamDialect,
}

impl RoutePlan {
    /// Splits [`Self::endpoint`] into the `(origin, path + query)` pair
    /// `gw-relay` takes.
    ///
    /// The relay assembles `origin + target` verbatim — it does not decode or
    /// re-encode the query — so the split has to preserve the raw bytes. That
    /// is why this reads `url.path()` / `url.query()` rather than iterating
    /// `query_pairs()`.
    ///
    /// # Errors
    /// [`ProviderError::Other`] when the endpoint has no host, or when its
    /// path and query do not form a valid request target.
    pub fn split(&self) -> Result<(Url, PathAndQuery), ProviderError> {
        if self.endpoint.host_str().unwrap_or_default().is_empty() {
            return Err(ProviderError::Other(anyhow::anyhow!(
                "upstream endpoint has no host"
            )));
        }
        let mut origin = self.endpoint.clone();
        origin.set_query(None);
        origin.set_fragment(None);
        origin.set_path("");

        let mut target = String::with_capacity(self.endpoint.path().len() + 16);
        target.push_str(self.endpoint.path());
        if let Some(query) = self.endpoint.query() {
            target.push('?');
            target.push_str(query);
        }
        let target = PathAndQuery::try_from(target).map_err(|err| {
            ProviderError::Other(anyhow::anyhow!(
                "upstream endpoint is not a request target: {err}"
            ))
        })?;
        Ok((origin, target))
    }

    /// Flattens a [`Spliced`] body into the single buffer the relay sends.
    ///
    /// One copy, and only on the streaming OpenAI-shaped path that actually
    /// splices; a plan that rewrites nothing returns `None` and the inbound
    /// `Bytes` are forwarded by refcount.
    #[must_use]
    pub fn splice(spliced: Option<Spliced>) -> Option<Bytes> {
        let spliced = spliced?;
        let mut out = bytes::BytesMut::with_capacity(spliced.len());
        out.extend_from_slice(&spliced.prefix);
        out.extend_from_slice(&spliced.rest);
        Some(out.freeze())
    }
}

/// Everything an upstream account can do that is not "send bytes".
///
/// Replaces the old `Provider` trait. The two methods it lost —
/// `execute` and `execute_stream` — are gone rather than deprecated: a second
/// HTTP path that still works is a second HTTP path that still gets used.
#[async_trait::async_trait]
pub trait RoutePlanner: Send + Sync {
    /// Stable provider key: `openai`, `claude`, `gemini`, `vertex`, `codex`,
    /// `xai`, `kiro`.
    fn name(&self) -> &'static str;

    /// Plans one attempt against `auth`.
    ///
    /// `async` because Vertex mints its access token from a signed assertion
    /// rather than reading a stored one; every other planner resolves without
    /// awaiting anything. It never sends inference bytes — that is the relay's
    /// job, and this returns the plan for it.
    ///
    /// # Errors
    /// [`ProviderError::Credential`] when the account carries no usable
    /// credential, [`ProviderError::Other`] when the endpoint cannot be
    /// assembled (a malformed base URL, a missing model).
    async fn plan(
        &self,
        auth: &gw_authcore::AuthRecord,
        req: &crate::types::ProviderRequest,
    ) -> Result<RoutePlan, ProviderError>;

    /// Plans the upstream's token-counting endpoint.
    ///
    /// Only Anthropic has one. The default **refuses** rather than inventing a
    /// number: a count nobody upstream confirmed is indistinguishable from a
    /// real one once it reaches the caller, and this endpoint used to return
    /// `body.len() / 4`.
    ///
    /// # Errors
    /// [`ProviderError::Other`] for every upstream without such an endpoint.
    async fn plan_count_tokens(
        &self,
        _auth: &gw_authcore::AuthRecord,
        _req: &crate::types::ProviderRequest,
    ) -> Result<RoutePlan, ProviderError> {
        Err(ProviderError::Other(anyhow::anyhow!(
            "{} upstream exposes no token-counting endpoint",
            self.name()
        )))
    }

    /// Refreshes an expiring OAuth credential and returns the updated record.
    ///
    /// This **is** HTTP, and it stays here: it is the credential lifecycle, not
    /// inference. It talks to an identity provider's token endpoint, carries no
    /// tenant payload, and its response never reaches a client.
    async fn refresh(
        &self,
        auth: &gw_authcore::AuthRecord,
    ) -> Result<gw_authcore::AuthRecord, ProviderError>;
}

#[cfg(test)]
mod tests;
