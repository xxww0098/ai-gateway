//! JSON / form POST used by Kiro login and refresh. Not a public surface.

use std::time::Duration;

use serde_json::{Value, json};

use crate::Error;

pub(crate) const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub(crate) enum Body {
    Json(Value),
    Form(String),
}

#[derive(Debug, Clone)]
pub(crate) struct Posted {
    pub status: u16,
    pub json: Value,
    pub text: String,
    pub retry_after: Option<String>,
}

impl Posted {
    #[must_use]
    pub(crate) fn is_ok(&self) -> bool {
        self.status < 400
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn ok(body: Value) -> Self {
        let text = body.to_string();
        Self {
            status: 200,
            json: body,
            text,
            retry_after: None,
        }
    }
}

pub(crate) async fn send(
    url: &str,
    headers: &[(&str, &str)],
    body: &Body,
) -> Result<Posted, Error> {
    let mut builder = reqwest::Client::new().post(url).timeout(HTTP_TIMEOUT);
    for (key, value) in headers {
        builder = builder.header(*key, *value);
    }
    builder = match body {
        Body::Json(value) => builder.json(value),
        Body::Form(form) => builder
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form.clone()),
    };
    let response = builder.send().await?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let text = response.text().await.unwrap_or_default();
    let json = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&text).unwrap_or_else(|_| json!({"error": text}))
    };
    Ok(Posted {
        status,
        json,
        text,
        retry_after,
    })
}

pub(crate) fn token_error(posted: &Posted, label: &str) -> Error {
    let snippet: String = posted.text.chars().take(240).collect();
    let mut body = if snippet.is_empty() {
        format!("{label} failed (HTTP {})", posted.status)
    } else {
        format!("{label} failed (HTTP {}): {snippet}", posted.status)
    };
    if let Some(retry) = &posted.retry_after {
        body.push_str(&format!(" retry-after={retry}"));
    }
    let lower = body.to_ascii_lowercase();
    if lower.contains("invalid_grant") || lower.contains("invalid refresh token") {
        return Error::Permanent;
    }
    Error::TokenEndpoint {
        status: posted.status,
        body,
    }
}

#[cfg(test)]
pub(crate) struct Scripted {
    pub calls: std::sync::Mutex<Vec<(String, Body)>>,
    responses: std::sync::Mutex<std::collections::VecDeque<Posted>>,
}

#[cfg(test)]
impl Scripted {
    pub(crate) fn new(responses: Vec<Posted>) -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            responses: std::sync::Mutex::new(std::collections::VecDeque::from(responses)),
        }
    }

    pub(crate) async fn post(
        &self,
        url: String,
        _headers: Vec<(String, String)>,
        body: Body,
    ) -> Result<Posted, Error> {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((url, body));
        self.responses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
            .ok_or_else(|| Error::Payload("kiro test poster exhausted".into()))
    }
}
