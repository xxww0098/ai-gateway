//! Credential-refresh HTTP — **the only module in this crate that sends.**
//!
//! Inference bytes leave through `gw-relay` and nowhere else. This is the one
//! exception, and it is not an exception to the rule so much as a different
//! kind of traffic:
//!
//! * it talks to an identity provider's token endpoint, not to a model;
//! * it carries no tenant payload — a refresh token in, an access token out;
//! * its response never reaches a client;
//! * it is not on the request path at all (a background credential lifecycle).
//!
//! Gathering it here rather than leaving one copy per provider is what makes
//! "`gw-provider` does not send inference HTTP" *checkable*: the source guard
//! in `route/tests.rs` allows `.send()` in this file and forbids it in every
//! other, so a new executor cannot quietly grow a second inference path.

use std::time::Duration;

use bytes::Bytes;
use http::header::ACCEPT;
use serde::Serialize;

use crate::common::shared_client;
use crate::types::ProviderError;

/// `POST url` with a JSON body, returning the raw response bytes.
///
/// # Errors
/// [`ProviderError::Other`] when the request could not be sent or read, and
/// [`ProviderError::Upstream`] for a >= 400 status, carrying the body so the
/// caller can log which grant was rejected.
pub(crate) async fn post_json<T: Serialize + ?Sized>(
    url: &str,
    timeout: Duration,
    what: &'static str,
    body: &T,
) -> Result<Bytes, ProviderError> {
    read(
        shared_client()
            .post(url)
            .timeout(timeout)
            .header(ACCEPT, "application/json")
            .json(body)
            .send()
            .await
            .map_err(|err| {
                ProviderError::Other(anyhow::anyhow!(
                    "{what} token refresh request failed: {err}"
                ))
            })?,
        what,
    )
    .await
}

/// `POST url` with a form body, returning the raw response bytes.
///
/// # Errors
/// Same as [`post_json`].
pub(crate) async fn post_form(
    url: &str,
    timeout: Duration,
    what: &'static str,
    form: &[(&str, &str)],
) -> Result<Bytes, ProviderError> {
    read(
        shared_client()
            .post(url)
            .timeout(timeout)
            .header(ACCEPT, "application/json")
            .form(form)
            .send()
            .await
            .map_err(|err| {
                ProviderError::Other(anyhow::anyhow!(
                    "{what} token refresh request failed: {err}"
                ))
            })?,
        what,
    )
    .await
}

/// Shared status check + body read.
async fn read(response: reqwest::Response, what: &'static str) -> Result<Bytes, ProviderError> {
    let status = response.status().as_u16();
    let payload = response.bytes().await.map_err(|err| {
        ProviderError::Other(anyhow::anyhow!("reading {what} refresh response: {err}"))
    })?;
    if status >= 400 {
        return Err(ProviderError::Upstream {
            status,
            body: String::from_utf8_lossy(&payload).into_owned(),
        });
    }
    Ok(payload)
}
