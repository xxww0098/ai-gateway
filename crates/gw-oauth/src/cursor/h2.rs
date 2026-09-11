//! HTTP/2 Connect transport for Cursor unary RPCs.
//!
//! Run is bidirectional (`application/connect+proto`); unary models use
//! `application/proto`. Cursor is HTTP/2 only — an ALPN-stripping TLS proxy
//! fails with `h2 is not supported`.

use http::Version;
use reqwest::Client;

use super::chat_headers;
use super::proto::{
    AvailableModel, UsableModel, decode_available_models_response,
    decode_get_usable_models_response, encode_available_models_request,
    encode_get_usable_models_request,
};
use super::{API2_URL, AVAILABLE_MODELS_PATH, MODELS_PATH, agent_url};
use crate::{Error, Session};

#[cfg(test)]
mod tests;

const UNARY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// Human-readable HTTP/2 negotiation failure.
#[must_use]
pub fn describe_h2_transport_error(error: &str, base_url: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("h2 is not supported")
        || (error.contains("ERR_HTTP2_ERROR") && lower.contains("h2"))
    {
        format!(
            "Cursor transport could not negotiate HTTP/2 with {base_url}: \"h2 is not supported\". \
             Cursor RPCs are HTTP/2 only; an ALPN-stripping TLS proxy usually causes this."
        )
    } else {
        error.to_owned()
    }
}

fn map_h2_error(err: reqwest::Error, base_url: &str) -> Error {
    let described = describe_h2_transport_error(&err.to_string(), base_url);
    if described == err.to_string() {
        Error::Transport(err)
    } else {
        Error::Payload(described)
    }
}

/// HTTP/2 unary Connect RPC. `unary` selects `application/proto`.
///
/// # Errors
/// Transport, HTTP/2 negotiation, or non-success status.
pub async fn unary_rpc(
    client: &Client,
    session: &Session,
    url: &str,
    path: &str,
    body: &[u8],
) -> Result<Vec<u8>, Error> {
    let base = url.trim_end_matches('/');
    let full = format!("{base}{path}");
    let headers = chat_headers(&session.access_token, None, true);
    let response = client
        .post(&full)
        .version(Version::HTTP_2)
        .headers(headers)
        .timeout(UNARY_TIMEOUT)
        .body(body.to_vec())
        .send()
        .await
        .map_err(|err| map_h2_error(err, url))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|err| map_h2_error(err, url))?;
    if !status.is_success() {
        return Err(Error::TokenEndpoint {
            status: status.as_u16(),
            body: String::from_utf8_lossy(&bytes).into_owned(),
        });
    }
    Ok(bytes.to_vec())
}

/// `AgentService/GetUsableModels`.
///
/// # Errors
/// Transport or decode failure.
pub async fn fetch_usable_models(
    client: &Client,
    session: &Session,
) -> Result<Vec<UsableModel>, Error> {
    let raw = unary_rpc(
        client,
        session,
        &agent_url(),
        MODELS_PATH,
        &encode_get_usable_models_request(&[]),
    )
    .await?;
    Ok(decode_get_usable_models_response(&raw))
}

/// `AiService/AvailableModels`.
///
/// # Errors
/// Transport or decode failure.
pub async fn fetch_available_models(
    client: &Client,
    session: &Session,
) -> Result<Vec<AvailableModel>, Error> {
    let raw = unary_rpc(
        client,
        session,
        API2_URL,
        AVAILABLE_MODELS_PATH,
        &encode_available_models_request(),
    )
    .await?;
    Ok(decode_available_models_response(&raw))
}
