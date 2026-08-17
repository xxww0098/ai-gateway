//! Client OAuth for DeepSeek Harness (`AGW-Oauth`).
//!
//! This is **not** upstream Grok/Kiro OAuth. A plugin starts a device-code
//! session, the user signs into the existing panel, and the plugin polls for
//! an AI-GateWay API key plus the gateway origin.

pub mod session;

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use chrono::Utc;
use serde::Deserialize;

use super::{bad_request, internal, not_found, parse_json_body};
use super::auth::{allow_auth_attempt, rate_limited};
use crate::identity::apikey::generate_api_key;
use crate::identity::oplog::ReqMeta;
use crate::{AuthUser, PanelState, codes, err, ok, ok_empty};

use session::{
    DEFAULT_INTERVAL_SECS, DEFAULT_TTL, DeviceSession, PollOutcome, TransitionError,
    normalize_user_code,
};

/// In-process device-code table. Same shape as the auth IP limiter: one
/// process, short TTL, no second user database.
static SESSIONS: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(Store::default()));

#[derive(Default)]
struct Store {
    by_device: HashMap<String, DeviceSession>,
    by_user: HashMap<String, String>,
}

impl Store {
    fn insert(&mut self, session: DeviceSession) {
        self.by_user
            .insert(normalize_user_code(&session.user_code), session.device_code.clone());
        self.by_device
            .insert(session.device_code.clone(), session);
    }

    fn get_device(&self, device_code: &str) -> Option<DeviceSession> {
        self.by_device.get(device_code).cloned()
    }

    fn get_user(&self, user_code: &str) -> Option<DeviceSession> {
        let device = self.by_user.get(&normalize_user_code(user_code))?;
        self.by_device.get(device).cloned()
    }

    fn put(&mut self, session: DeviceSession) {
        self.by_device
            .insert(session.device_code.clone(), session);
    }

    fn sweep(&mut self, now: chrono::DateTime<Utc>) {
        let stale: Vec<String> = self
            .by_device
            .iter()
            .filter(|(_, s)| now >= s.expires_at + chrono::Duration::seconds(300))
            .map(|(k, _)| k.clone())
            .collect();
        for device in stale {
            if let Some(session) = self.by_device.remove(&device) {
                self.by_user.remove(&normalize_user_code(&session.user_code));
            }
        }
    }
}

fn lock_store() -> std::sync::MutexGuard<'static, Store> {
    match SESSIONS.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Routes under `/api/panel`.
pub fn router() -> axum::Router<PanelState> {
    axum::Router::new()
        .route("/oauth/dsh/device/code", post(start_device))
        .route("/oauth/dsh/device/token", post(poll_token))
        .route("/oauth/dsh/device/approve", post(approve_device))
        .route("/oauth/dsh/device/deny", post(deny_device))
}

/// Public HTML consent page, mounted at the application root.
pub fn public_router() -> axum::Router<PanelState> {
    axum::Router::new().route("/oauth/dsh", get(approve_page))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StartRequest {
    origin: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TokenRequest {
    device_code: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DecideRequest {
    user_code: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PageQuery {
    #[serde(default)]
    user_code: String,
}

/// `POST /oauth/dsh/device/code`
pub async fn start_device(
    State(state): State<PanelState>,
    ReqMeta(meta): ReqMeta,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !allow_auth_attempt(&meta.ip_address) {
        return rate_limited();
    }
    let req: StartRequest = if body.is_empty() {
        StartRequest::default()
    } else {
        match parse_json_body(&body, "请求格式无效") {
            Ok(req) => req,
            Err(response) => return response,
        }
    };
    let origin = if req.origin.trim().is_empty() {
        public_origin(&headers, &state.cfg.server.host, state.cfg.server.port)
    } else {
        req.origin.trim().trim_end_matches('/').to_owned()
    };
    if !origin.starts_with("http://") && !origin.starts_with("https://") {
        return bad_request("origin 必须是 http(s) URL");
    }

    let now = Utc::now();
    let session = session::start(now, DEFAULT_TTL, origin.clone());
    let device_code = session.device_code.clone();
    let user_code = session.user_code.clone();
    let expires_in = (session.expires_at - now).num_seconds().max(0);
    {
        let mut store = lock_store();
        store.sweep(now);
        store.insert(session);
    }

    let verification_uri = format!("{origin}/oauth/dsh");
    let verification_uri_complete = format!("{verification_uri}?user_code={user_code}");
    ok(serde_json::json!({
        "device_code": device_code,
        "user_code": user_code,
        "verification_uri": verification_uri,
        "verification_uri_complete": verification_uri_complete,
        "expires_in": expires_in,
        "interval": DEFAULT_INTERVAL_SECS,
    }))
}

/// `POST /oauth/dsh/device/token`
pub async fn poll_token(body: Bytes) -> Response {
    let req: TokenRequest = match parse_json_body(&body, "请求格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    if req.device_code.trim().is_empty() {
        return bad_request("device_code 不能为空");
    }
    let now = Utc::now();
    let session = {
        let store = lock_store();
        store.get_device(req.device_code.trim())
    };
    let Some(session) = session else {
        return err(
            StatusCode::BAD_REQUEST,
            codes::BAD_REQUEST,
            "expired_token",
        );
    };
    match session::poll(&session, now) {
        PollOutcome::Pending => ok(serde_json::json!({ "status": "pending" })),
        PollOutcome::Denied => ok(serde_json::json!({ "status": "denied" })),
        PollOutcome::Expired => ok(serde_json::json!({ "status": "expired" })),
        PollOutcome::Approved { api_key, origin } => ok(serde_json::json!({
            "status": "approved",
            "api_key": api_key,
            "origin": origin,
        })),
    }
}

/// `POST /oauth/dsh/device/approve`
pub async fn approve_device(
    State(state): State<PanelState>,
    user: AuthUser,
    body: Bytes,
) -> Response {
    let req: DecideRequest = match parse_json_body(&body, "请求格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    if req.user_code.trim().is_empty() {
        return bad_request("user_code 不能为空");
    }
    let now = Utc::now();
    let session = {
        let store = lock_store();
        store.get_user(req.user_code.trim())
    };
    let Some(session) = session else {
        return not_found("未找到该授权码");
    };

    let (plaintext, _row) = match generate_api_key(
        &state.pg,
        user.user_id,
        "AGW-Oauth",
        None,
    )
    .await
    {
        Ok(pair) => pair,
        Err(error) => {
            tracing::warn!(event = "dsh_oauth_key_failed", user_id = user.user_id, error = %error);
            return internal("创建 API Key 失败，请稍后重试");
        }
    };

    match session::approve(&session, now, user.user_id, plaintext) {
        Ok(next) => {
            lock_store().put(next);
            ok(serde_json::json!({ "status": "approved" }))
        }
        Err(TransitionError::Expired) => err(
            StatusCode::BAD_REQUEST,
            codes::BAD_REQUEST,
            "授权码已过期",
        ),
        Err(TransitionError::AlreadyResolved) => err(
            StatusCode::CONFLICT,
            super::ERR_CONFLICT,
            "该授权码已处理",
        ),
    }
}

/// `POST /oauth/dsh/device/deny`
pub async fn deny_device(user: AuthUser, body: Bytes) -> Response {
    let _ = user;
    let req: DecideRequest = match parse_json_body(&body, "请求格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    if req.user_code.trim().is_empty() {
        return bad_request("user_code 不能为空");
    }
    let now = Utc::now();
    let session = {
        let store = lock_store();
        store.get_user(req.user_code.trim())
    };
    let Some(session) = session else {
        return not_found("未找到该授权码");
    };
    match session::deny(&session, now) {
        Ok(next) => {
            lock_store().put(next);
            ok_empty()
        }
        Err(TransitionError::Expired) => err(
            StatusCode::BAD_REQUEST,
            codes::BAD_REQUEST,
            "授权码已过期",
        ),
        Err(TransitionError::AlreadyResolved) => err(
            StatusCode::CONFLICT,
            super::ERR_CONFLICT,
            "该授权码已处理",
        ),
    }
}

/// `GET /oauth/dsh` — self-contained consent page (does not touch `frontend/`).
pub(crate) async fn approve_page(Query(query): Query<PageQuery>) -> impl IntoResponse {
    Html(consent_html(&query.user_code))
}

/// Build the public origin the plugin should open.
#[must_use]
pub fn public_origin(headers: &HeaderMap, fallback_host: &str, fallback_port: u16) -> String {
    if let Ok(from_env) = std::env::var("AGW_PUBLIC_ORIGIN") {
        let trimmed = from_env.trim().trim_end_matches('/');
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return trimmed.to_owned();
        }
    }
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| *s == "http" || *s == "https")
        .unwrap_or("http");
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(axum::http::header::HOST))
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            if fallback_port == 80 || fallback_port == 443 {
                fallback_host.to_owned()
            } else {
                format!("{fallback_host}:{fallback_port}")
            }
        });
    format!("{proto}://{host}")
}

fn consent_html(user_code: &str) -> String {
    let escaped = user_code
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    format!(
        r##"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>AI-GateWay · 授权 DeepSeek Harness</title>
<style>
body {{ font-family: ui-sans-serif, system-ui, sans-serif; margin: 0; background: #0f1419; color: #e7ecf3; }}
main {{ max-width: 28rem; margin: 4rem auto; padding: 1.5rem; background: #1a222c; border-radius: 12px; }}
h1 {{ font-size: 1.25rem; margin: 0 0 0.5rem; }}
p {{ color: #9aa7b5; line-height: 1.5; }}
label {{ display: block; margin: 0.75rem 0 0.25rem; font-size: 0.85rem; }}
input {{ width: 100%; box-sizing: border-box; padding: 0.6rem 0.7rem; border-radius: 8px; border: 1px solid #2c3846; background: #0f1419; color: #e7ecf3; }}
button {{ margin-top: 1rem; margin-right: 0.5rem; padding: 0.55rem 1rem; border: 0; border-radius: 8px; cursor: pointer; }}
.ok {{ background: #3d8bfd; color: #fff; }}
.no {{ background: #2c3846; color: #e7ecf3; }}
.err {{ color: #ff8a8a; min-height: 1.2rem; }}
code {{ font-size: 1.1rem; letter-spacing: 0.08em; }}
</style>
</head>
<body>
<main>
<h1>授权 AGW-Oauth</h1>
<p>DeepSeek Harness 想使用你的 <strong>AI-GateWay</strong> 账号调用模型。登录后批准，插件会拿到一把 API Key，无需手写模型配置。</p>
<label>授权码</label>
<input id="user_code" value="{escaped}" autocomplete="off"/>
<div id="login">
<label>邮箱</label>
<input id="email" type="email" autocomplete="username"/>
<label>密码</label>
<input id="password" type="password" autocomplete="current-password"/>
<button class="ok" id="signin" type="button">登录</button>
</div>
<div id="decide" hidden>
<p>已登录。批准后将为 DeepSeek Harness 创建一把 API Key。</p>
<button class="ok" id="approve" type="button">批准</button>
<button class="no" id="deny" type="button">拒绝</button>
</div>
<p class="err" id="err"></p>
</main>
<script>
const err = (m) => document.getElementById('err').textContent = m || '';
const tokenKey = 'agw_dsh_jwt';
const showDecide = () => {{
  document.getElementById('login').hidden = true;
  document.getElementById('decide').hidden = false;
}};
if (sessionStorage.getItem(tokenKey)) showDecide();
document.getElementById('signin').onclick = async () => {{
  err('');
  const res = await fetch('/api/panel/auth/login', {{
    method: 'POST',
    headers: {{ 'content-type': 'application/json' }},
    body: JSON.stringify({{
      email: document.getElementById('email').value,
      password: document.getElementById('password').value,
    }}),
  }});
  const json = await res.json();
  if (!res.ok || json.code !== 0 || !json.data || !json.data.token) {{
    err(json.message || '登录失败');
    return;
  }}
  sessionStorage.setItem(tokenKey, json.data.token);
  showDecide();
}};
async function decide(path) {{
  err('');
  const token = sessionStorage.getItem(tokenKey);
  if (!token) {{ err('请先登录'); return; }}
  const res = await fetch('/api/panel' + path, {{
    method: 'POST',
    headers: {{
      'content-type': 'application/json',
      'authorization': 'Bearer ' + token,
    }},
    body: JSON.stringify({{ user_code: document.getElementById('user_code').value }}),
  }});
  const json = await res.json();
  if (!res.ok || json.code !== 0) {{
    err(json.message || '操作失败');
    return;
  }}
  document.querySelector('main').innerHTML = '<h1>完成</h1><p>可以回到 DeepSeek Harness 了。</p>';
}}
document.getElementById('approve').onclick = () => decide('/oauth/dsh/device/approve');
document.getElementById('deny').onclick = () => decide('/oauth/dsh/device/deny');
</script>
</main>
</body>
</html>
"##
    )
}

#[cfg(test)]
mod tests;
