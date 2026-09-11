//! Zhipu GLM Coding Plan (Z.ai global + BigModel China).
//!
//! Login is ZCode CLI poll (`provider: zai | bigmodel`), not PKCE. Default
//! chat hop is Anthropic (`/api/anthropic`). Completions `paas/v4` is leftover.
//! Start Plan (`zcode-plan`, captcha 3007) is not supported.

pub mod cache;
mod request;

pub use cache::{apply_anthropic_cache, apply_cache, cache_session_id, session_headers};
pub use request::{
    DEFAULT_MAX_TOKENS, forced_thinking_model, normalize_completions, normalize_request,
};

use std::sync::OnceLock;
use std::time::Duration;

use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use crate::login::StartInput;
use crate::session::{FlowKind, Session, StartOutcome};
use crate::{Error, Family};

#[cfg(test)]
mod tests;

pub const ID: &str = "glm";
pub const CLIENT_ID: &str = "client_P8X5CMWmlaRO9gyO-KSqtg";
/// Official ZCode Desktop, latest stable. Identity for the 150% quota boost.
pub const APP_VERSION: &str = "3.10.1";
pub const USER_AGENT: &str = "ZCode/3.10.1 ai-sdk/anthropic/3.0.81";
/// CLI poll against zcode.z.ai — official CLI shape, not Desktop.
pub const CLI_USER_AGENT: &str = "ZCode/3.10.1";
pub const ZAI_ANTHROPIC: &str = "https://api.z.ai/api/anthropic";
pub const BIGMODEL_ANTHROPIC: &str = "https://open.bigmodel.cn/api/anthropic";
pub const CLI_INIT_URL: &str = "https://zcode.z.ai/api/v1/oauth/cli/init";
pub const CLI_POLL_URL: &str = "https://zcode.z.ai/api/v1/oauth/cli/poll";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const REFERER: &str = "https://zcode.z.ai";
pub const TITLE: &str = "Z Code";
/// Coding Plan keys do not expire.
pub const NEVER_EXPIRES_MS: i64 = 8_640_000_000_000_000;
pub const BOOST_LABEL_ZH: &str = "150%配额";
pub const BOOST_LABEL_EN: &str = "150% quota";

const KEY_NAME: &str = "ai-gateway";
const APP_ACCOUNTS: [&str; 4] = ["zcode", "zai", "bigmodel", "glm"];
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// ZCode welcome-screen region. CLI provider id is the same string; never `zcode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// Global `api.z.ai`. CLI provider `zai`.
    Zai,
    /// China `open.bigmodel.cn`. CLI provider `bigmodel`.
    BigModel,
}

impl Region {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zai => "zai",
            Self::BigModel => "bigmodel",
        }
    }

    /// Posted to `/oauth/cli/init`. `zcode` 500s; China is `bigmodel`.
    #[must_use]
    pub fn cli_provider(self) -> &'static str {
        self.as_str()
    }
}

/// Map a button / body hint onto [`Region`]. `cn` / `zcode` / `china` are China.
#[must_use]
pub fn normalize_region(value: &str) -> Region {
    match value.trim().to_ascii_lowercase().as_str() {
        "bigmodel" | "cn" | "zcode" | "china" => Region::BigModel,
        _ => Region::Zai,
    }
}

/// Anthropic Messages hop for `region`.
#[must_use]
pub fn anthropic_url(region: Region) -> String {
    format!("{}/v1/messages", anthropic_origin(region))
}

/// Completions leftover hop for `region`.
#[must_use]
pub fn coding_url(region: Region) -> String {
    match region {
        Region::Zai => "https://api.z.ai/api/coding/paas/v4/chat/completions".to_owned(),
        Region::BigModel => {
            "https://open.bigmodel.cn/api/coding/paas/v4/chat/completions".to_owned()
        }
    }
}

#[must_use]
pub fn anthropic_origin(region: Region) -> &'static str {
    match region {
        Region::Zai => ZAI_ANTHROPIC,
        Region::BigModel => BIGMODEL_ANTHROPIC,
    }
}

/// ZCode CLI poll. Region comes from `StartInput.method` or body `mode`/`region`/`provider`.
///
/// # Errors
/// Start Plan / import, vendor HTTP, or a CLI init body missing `flow_id`/`authorize_url`.
pub async fn start(input: &StartInput) -> Result<StartOutcome, Error> {
    start_with(input, &http_client(), &Hosts::production()).await
}

/// One GET of `/oauth/cli/poll/{flow_id}`.
///
/// # Errors
/// Vendor HTTP, envelope failure, or a `ready` poll with no access token.
pub async fn poll(input: &PollInput) -> Result<PollStatus, Error> {
    poll_with(input, &http_client(), &Hosts::production()).await
}

/// Turn a ready CLI poll into a Coding Plan session.
///
/// Z.ai mints `id.secret` through biz login. BigModel uses the poll JWT as bearer.
///
/// # Errors
/// Mint HTTP, missing org/project/key, or a BigModel poll with no token.
pub async fn exchange(ready: &ReadyTokens) -> Result<Session, Error> {
    exchange_with(ready, &http_client(), &Hosts::production()).await
}

/// Coding Plan keys do not expire. Keeps [`NEVER_EXPIRES_MS`].
///
/// # Errors
/// Empty access token.
pub async fn refresh(session: &Session) -> Result<Session, Error> {
    if session.access_token.is_empty() {
        return Err(Error::Payload(
            "glm session needs an access token".to_owned(),
        ));
    }
    let mut next = session.clone();
    next.expires_at_ms = NEVER_EXPIRES_MS;
    if next.refresh_token.is_empty() {
        next.refresh_token.clone_from(&next.access_token);
    }
    Ok(next)
}

/// Refresh never invalidates a stored GLM row; the operator does not re-login.
#[must_use]
pub fn is_permanent_refresh_error(_error: &Error) -> bool {
    false
}

/// Inputs the panel stored after [`start`].
#[derive(Debug, Clone)]
pub struct PollInput {
    pub flow_id: String,
    pub poll_token: String,
    pub region: Region,
}

impl PollInput {
    /// Recover poll fields from a [`StartOutcome`] produced by [`start`].
    #[must_use]
    pub fn from_start(outcome: &StartOutcome) -> Option<Self> {
        let extra = match outcome {
            StartOutcome::Browser { extra, .. } | StartOutcome::Device { extra, .. } => extra,
            StartOutcome::Ready(_) => return None,
        };
        let flow_id = extra
            .get("flow_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())?;
        let poll_token = extra
            .get("poll_token")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or(match outcome {
                StartOutcome::Browser { verifier, .. } => Some(verifier.as_str()),
                _ => None,
            })
            .filter(|s| !s.is_empty())?;
        let region = extra
            .get("region")
            .and_then(Value::as_str)
            .map(normalize_region)
            .unwrap_or(Region::Zai);
        Some(Self {
            flow_id: flow_id.to_owned(),
            poll_token: poll_token.to_owned(),
            region,
        })
    }
}

/// Result of one CLI poll.
#[derive(Debug, Clone)]
pub enum PollStatus {
    Pending { status: String },
    Ready(ReadyTokens),
}

/// Tokens from a `status: ready` CLI poll, before [`exchange`].
#[derive(Debug, Clone)]
pub struct ReadyTokens {
    pub oauth_access: String,
    pub zcode_jwt: Option<String>,
    pub email: Option<String>,
    pub region: Region,
}

/// Desktop fingerprint for Coding Plan hops. UA is the 150% identity, not the protocol.
#[must_use]
pub fn desktop_headers(session_id: Option<&str>) -> HeaderMap {
    let session = cache_session_id(session_id).unwrap_or_else(|| process_session_id().to_owned());
    let mut headers = session_headers(Some(&session));
    insert_header(&mut headers, "user-agent", USER_AGENT);
    insert_header(&mut headers, "X-ZCode-App-Version", APP_VERSION);
    insert_header(&mut headers, "X-ZCode-Agent", ID);
    insert_header(&mut headers, "x-zcode-trace-id", &random_hex_fallback(16));
    insert_header(&mut headers, "x-request-id", &random_hex_fallback(16));
    insert_header(&mut headers, "x-query-id", &random_hex_fallback(16));
    insert_header(&mut headers, "HTTP-Referer", REFERER);
    insert_header(&mut headers, "referer", REFERER);
    insert_header(&mut headers, "X-Title", TITLE);
    headers
}

/// Bearer + Desktop + `anthropic-version`.
#[must_use]
pub fn anthropic_headers(access_token: &str, session_id: Option<&str>) -> HeaderMap {
    let mut headers = desktop_headers(session_id);
    insert_header(
        &mut headers,
        "authorization",
        &format!("Bearer {access_token}"),
    );
    insert_header(&mut headers, "accept", "application/json");
    insert_header(&mut headers, "anthropic-version", ANTHROPIC_VERSION);
    headers
}

/// Site ids, poll `user.id`, JWT `sub` / numeric uid. Never a card title.
#[must_use]
pub fn is_opaque_account(value: &str) -> bool {
    let raw = value.trim();
    if raw.is_empty() {
        return false;
    }
    if is_app_account(raw) {
        return true;
    }
    if raw.contains('@') {
        return false;
    }
    if is_formatted_phone(raw) {
        return false;
    }
    if raw.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    if is_uuid(raw) {
        return true;
    }
    if raw.len() >= 16 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    raw.len() >= 2
        && raw.len() <= 24
        && raw.chars().all(|c| c.is_ascii_alphanumeric())
        && raw.chars().any(|c| c.is_ascii_alphabetic())
        && raw.chars().any(|c| c.is_ascii_digit())
}

/// First non-opaque candidate (email, then phone, then a human name).
#[must_use]
pub fn pick_human_account(candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty() && !is_opaque_account(value))
        .map(str::to_owned)
}

struct Hosts {
    cli_init: String,
    cli_poll: String,
    zai_biz: String,
    bigmodel_biz: String,
    zai_userinfo: String,
    bigmodel_userinfo: String,
}

impl Hosts {
    fn production() -> Self {
        Self {
            cli_init: CLI_INIT_URL.to_owned(),
            cli_poll: CLI_POLL_URL.to_owned(),
            zai_biz: "https://api.z.ai".to_owned(),
            bigmodel_biz: "https://open.bigmodel.cn".to_owned(),
            zai_userinfo: "https://chat.z.ai/api/oauth/userinfo".to_owned(),
            bigmodel_userinfo: "https://open.bigmodel.cn/api/biz/customer/getCustomerInfo"
                .to_owned(),
        }
    }

    fn biz(&self, region: Region) -> &str {
        match region {
            Region::Zai => &self.zai_biz,
            Region::BigModel => &self.bigmodel_biz,
        }
    }

    #[cfg(test)]
    fn local(base: &str) -> Self {
        Self {
            cli_init: format!("{base}/api/v1/oauth/cli/init"),
            cli_poll: format!("{base}/api/v1/oauth/cli/poll"),
            zai_biz: base.to_owned(),
            bigmodel_biz: base.to_owned(),
            zai_userinfo: format!("{base}/api/oauth/userinfo"),
            bigmodel_userinfo: format!("{base}/api/biz/customer/getCustomerInfo"),
        }
    }
}

struct CliInit {
    flow_id: String,
    authorize_url: String,
    interval_secs: i64,
    expires_at_ms: i64,
}

async fn start_with(
    input: &StartInput,
    client: &reqwest::Client,
    hosts: &Hosts,
) -> Result<StartOutcome, Error> {
    if is_unsupported_start(input) {
        return Err(Error::UnsupportedFlow);
    }
    let region = region_from_input(input);
    let poll_token = crate::random_hex(32).unwrap_or_else(|_| random_hex_fallback(32));
    let response = client
        .post(&hosts.cli_init)
        .headers(cli_headers(&poll_token))
        .json(&json!({ "provider": region.cli_provider() }))
        .send()
        .await?;
    let started = parse_cli_init(read_json(response, "glm cli init").await?)?;
    let mut extra = Map::new();
    extra.insert("flow_id".to_owned(), json!(started.flow_id));
    extra.insert("region".to_owned(), json!(region.as_str()));
    extra.insert("poll_token".to_owned(), json!(poll_token));
    extra.insert("interval_secs".to_owned(), json!(started.interval_secs));
    extra.insert("expires_at_ms".to_owned(), json!(started.expires_at_ms));
    extra.insert("authorize_url".to_owned(), json!(started.authorize_url));
    Ok(StartOutcome::Browser {
        authorize_url: started.authorize_url,
        state: input.state.clone(),
        verifier: poll_token,
        redirect_uri: input.redirect_uri.clone(),
        flow: FlowKind::CliPoll,
        extra,
    })
}

async fn poll_with(
    input: &PollInput,
    client: &reqwest::Client,
    hosts: &Hosts,
) -> Result<PollStatus, Error> {
    let url = join_poll(&hosts.cli_poll, &input.flow_id)?;
    let response = client
        .get(url)
        .headers(cli_headers(&input.poll_token))
        .send()
        .await?;
    parse_cli_poll(read_json(response, "glm cli poll").await?, input.region)
}

async fn exchange_with(
    ready: &ReadyTokens,
    client: &reqwest::Client,
    hosts: &Hosts,
) -> Result<Session, Error> {
    let access = match ready.region {
        Region::BigModel => {
            let token = ready
                .zcode_jwt
                .as_deref()
                .filter(|s| !s.is_empty())
                .or_else(|| Some(ready.oauth_access.as_str()).filter(|s| !s.is_empty()))
                .ok_or_else(|| {
                    Error::Payload("glm BigModel poll ready without a token".to_owned())
                })?;
            token.to_owned()
        }
        Region::Zai => mint_api_key(&ready.oauth_access, client, hosts).await?,
    };
    let account = resolve_identity(ready, &access, client, hosts).await;
    Ok(glm_session(
        &access,
        account,
        ready.region,
        ready.zcode_jwt.clone(),
    ))
}

fn glm_session(
    access: &str,
    account: Option<String>,
    region: Region,
    zcode_jwt: Option<String>,
) -> Session {
    let mut session = Session::new(Family::Glm, access, access);
    session.expires_at_ms = NEVER_EXPIRES_MS;
    if let Some(account) = account {
        session.account = account;
    }
    session
        .extra
        .insert("region".to_owned(), json!(region.as_str()));
    if let Some(jwt) = zcode_jwt.filter(|s| !s.is_empty()) {
        session.extra.insert("zcode_jwt".to_owned(), json!(jwt));
    }
    session
}

async fn mint_api_key(
    oauth_access: &str,
    client: &reqwest::Client,
    hosts: &Hosts,
) -> Result<String, Error> {
    let biz_token = business_login(oauth_access, client, hosts, Region::Zai).await?;
    let auth = bearer_desktop(&biz_token, None);
    let base = hosts.biz(Region::Zai);
    let customer = unwrap_envelope(
        get_json(
            &format!("{base}/api/biz/customer/getCustomerInfo"),
            &auth,
            client,
        )
        .await?,
        "customer lookup",
    )?;
    let (organization_id, project_id) = org_project(&customer)?;
    let keys_url =
        format!("{base}/api/biz/v1/organization/{organization_id}/projects/{project_id}/api_keys");
    let listed = unwrap_envelope(get_json(&keys_url, &auth, client).await?, "api key list")?;
    let existing = as_key_array(&listed)
        .into_iter()
        .find(|key| key.get("name").and_then(Value::as_str) == Some(KEY_NAME));
    let key_record = match existing {
        Some(row) => row,
        None => unwrap_envelope(
            post_json(&keys_url, json!({ "name": KEY_NAME }), &auth, client).await?,
            "api key create",
        )?,
    };
    let api_key = trimmed(key_record.get("apiKey"))
        .ok_or_else(|| Error::Payload("glm key provisioning returned no apiKey".to_owned()))?;
    let copied = unwrap_envelope(
        get_json(&format!("{keys_url}/copy/{api_key}"), &auth, client).await?,
        "api key copy",
    )?;
    let secret = trimmed(copied.get("secretKey"))
        .ok_or_else(|| Error::Payload("glm key provisioning returned no secretKey".to_owned()))?;
    Ok(format!("{api_key}.{secret}"))
}

async fn business_login(
    oauth_access: &str,
    client: &reqwest::Client,
    hosts: &Hosts,
    region: Region,
) -> Result<String, Error> {
    let url = format!("{}/api/auth/z/login", hosts.biz(region));
    let data = unwrap_envelope(
        post_json(
            &url,
            json!({ "token": oauth_access }),
            &desktop_headers(None),
            client,
        )
        .await?,
        "business login",
    )?;
    trimmed(data.get("access_token"))
        .or_else(|| trimmed(data.get("accessToken")))
        .ok_or_else(|| Error::Payload("glm business login returned no access token".to_owned()))
}

fn org_project(customer: &Value) -> Result<(String, String), Error> {
    let orgs = customer
        .get("organizations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let org = orgs
        .iter()
        .find(|row| row.get("isDefault").and_then(Value::as_bool) == Some(true))
        .or(orgs.first())
        .ok_or_else(|| {
            Error::Payload(
                "glm key provisioning failed: no organization/project on account".to_owned(),
            )
        })?;
    let projects = org
        .get("projects")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let project = projects
        .iter()
        .find(|row| row.get("isDefault").and_then(Value::as_bool) == Some(true))
        .or(projects.first())
        .ok_or_else(|| {
            Error::Payload(
                "glm key provisioning failed: no organization/project on account".to_owned(),
            )
        })?;
    let organization_id = trimmed(org.get("organizationId")).ok_or_else(|| {
        Error::Payload("glm key provisioning failed: no organization/project on account".to_owned())
    })?;
    let project_id = trimmed(project.get("projectId")).ok_or_else(|| {
        Error::Payload("glm key provisioning failed: no organization/project on account".to_owned())
    })?;
    Ok((organization_id, project_id))
}

fn as_key_array(value: &Value) -> Vec<Value> {
    if let Some(array) = value.as_array() {
        return array.clone();
    }
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    for field in ["list", "keys", "apiKeys", "records"] {
        if let Some(array) = obj.get(field).and_then(Value::as_array) {
            return array.clone();
        }
    }
    Vec::new()
}

async fn resolve_identity(
    ready: &ReadyTokens,
    access: &str,
    client: &reqwest::Client,
    hosts: &Hosts,
) -> Option<String> {
    let from_email = ready.email.clone().unwrap_or_default();
    let from_jwt =
        account_from_jwt(ready.zcode_jwt.as_deref().unwrap_or_default()).unwrap_or_default();
    let from_access = account_from_jwt(access).unwrap_or_default();
    let from_oauth = account_from_jwt(&ready.oauth_access).unwrap_or_default();
    if let Some(human) = pick_human_account(&[&from_email, &from_jwt, &from_access, &from_oauth]) {
        return Some(human);
    }
    fetch_userinfo(ready, access, client, hosts).await
}

async fn fetch_userinfo(
    ready: &ReadyTokens,
    access: &str,
    client: &reqwest::Client,
    hosts: &Hosts,
) -> Option<String> {
    let bearer = ready
        .zcode_jwt
        .as_deref()
        .filter(|s| !s.is_empty())
        .or_else(|| Some(ready.oauth_access.as_str()).filter(|s| !s.is_empty()))
        .filter(|s| !s.is_empty())
        .unwrap_or(access);
    if bearer.is_empty() {
        return None;
    }
    let zai_extra = format!("{}/api/biz/customer/getCustomerInfo", hosts.zai_biz);
    let urls: Vec<&str> = match ready.region {
        Region::BigModel => vec![hosts.bigmodel_userinfo.as_str()],
        Region::Zai => vec![hosts.zai_userinfo.as_str(), zai_extra.as_str()],
    };
    let headers = bearer_desktop(bearer, None);
    for url in urls {
        let Ok(body) = get_json(url, &headers, client).await else {
            continue;
        };
        let data = unwrap_envelope(body.clone(), "userinfo").unwrap_or(body.clone());
        if let Some(human) = human_from_object(&data)
            .or_else(|| data.get("user").and_then(human_from_object))
            .or_else(|| data.get("profile").and_then(human_from_object))
            .or_else(|| human_from_object(&body))
        {
            return Some(human);
        }
    }
    None
}

fn human_from_object(value: &Value) -> Option<String> {
    let obj = value.as_object()?;
    let email = pick_human_account(&[
        str_field(obj, "email"),
        str_field(obj, "mail"),
        str_field(obj, "preferred_username"),
        str_field(obj, "preferredUsername"),
    ]);
    if email.is_some() {
        return email;
    }
    if let Some(phone) = pick_phone(&[str_field(obj, "phone"), str_field(obj, "mobile")]) {
        return Some(phone);
    }
    pick_human_account(&[
        str_field(obj, "customerName"),
        str_field(obj, "nickName"),
        str_field(obj, "nickname"),
        str_field(obj, "displayName"),
        str_field(obj, "name"),
        str_field(obj, "username"),
        str_field(obj, "userName"),
    ])
}

fn str_field<'a>(obj: &'a Map<String, Value>, key: &str) -> &'a str {
    obj.get(key).and_then(Value::as_str).unwrap_or("")
}

fn pick_phone(candidates: &[&str]) -> Option<String> {
    for value in candidates {
        let raw = value.trim();
        if raw.is_empty() || is_app_account(raw) {
            continue;
        }
        if is_formatted_phone(raw) || {
            let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
            (7..=15).contains(&digits.len())
                && raw
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '(' | ')' | '.' | '-'))
        } {
            return Some(raw.to_owned());
        }
    }
    None
}

fn account_from_jwt(token: &str) -> Option<String> {
    let claims = crate::decode_payload(token)?;
    pick_human_account(&[
        str_field(&claims, "email"),
        str_field(&claims, "preferred_username"),
        str_field(&claims, "preferredUsername"),
        str_field(&claims, "username"),
        str_field(&claims, "userName"),
        str_field(&claims, "name"),
    ])
}

fn is_app_account(value: &str) -> bool {
    let raw = value.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return false;
    }
    if APP_ACCOUNTS.contains(&raw.as_str()) {
        return true;
    }
    let Some((head, tail)) = raw.rsplit_once('@') else {
        return false;
    };
    if head.is_empty() {
        return false;
    }
    if matches!(tail, "zai" | "bigmodel" | "zcode" | "glm") && APP_ACCOUNTS.contains(&head) {
        return true;
    }
    APP_ACCOUNTS.contains(&head) && matches!(tail, "zai" | "bigmodel")
}

fn is_formatted_phone(value: &str) -> bool {
    let raw = value.trim();
    if raw.is_empty() {
        return false;
    }
    if !raw
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '(' | ')' | '.' | '-'))
    {
        return false;
    }
    if !raw
        .chars()
        .any(|c| matches!(c, '+' | ' ' | '(' | ')' | '.' | '-'))
    {
        return false;
    }
    let digits = raw.chars().filter(char::is_ascii_digit).count();
    (7..=15).contains(&digits)
}

fn is_uuid(value: &str) -> bool {
    let raw = value.trim();
    let parts: Vec<&str> = raw.split('-').collect();
    parts.len() == 5
        && parts[0].len() == 8
        && parts[1].len() == 4
        && parts[2].len() == 4
        && parts[3].len() == 4
        && parts[4].len() == 12
        && parts
            .iter()
            .all(|p| p.chars().all(|c| c.is_ascii_hexdigit()))
}

fn is_unsupported_start(input: &StartInput) -> bool {
    let method = input.method.trim().to_ascii_lowercase();
    if matches!(
        method.as_str(),
        "import" | "start-plan" | "startplan" | "zcode-plan" | "zcode_plan" | "captcha"
    ) {
        return true;
    }
    body_looks_like_start_plan(&input.body)
}

fn body_looks_like_start_plan(body: &Value) -> bool {
    let blob = body.to_string().to_ascii_lowercase();
    blob.contains("zcode-plan") || blob.contains("start-plan")
}

fn region_from_input(input: &StartInput) -> Region {
    let method = input.method.trim();
    if !method.is_empty() && is_region_hint(method) {
        return normalize_region(method);
    }
    for key in ["mode", "region", "provider", "site"] {
        if let Some(value) = input
            .body
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return normalize_region(value);
        }
    }
    if method.is_empty() {
        Region::Zai
    } else {
        normalize_region(method)
    }
}

fn is_region_hint(method: &str) -> bool {
    let lower = method.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "zai" | "bigmodel" | "cn" | "zcode" | "china" | "global"
    )
}

fn parse_cli_init(body: Value) -> Result<CliInit, Error> {
    let data = unwrap_envelope(body, "cli init")?;
    let flow_id = trimmed(data.get("flow_id"))
        .or_else(|| trimmed(data.get("flowId")))
        .ok_or_else(|| {
            Error::Payload("glm cli init is missing flow_id/authorize_url".to_owned())
        })?;
    let authorize_url = trimmed(data.get("authorize_url"))
        .or_else(|| trimmed(data.get("authorizeUrl")))
        .ok_or_else(|| {
            Error::Payload("glm cli init is missing flow_id/authorize_url".to_owned())
        })?;
    let interval = data
        .get("poll_interval_sec")
        .or_else(|| data.get("interval"))
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n > 0.0)
        .unwrap_or(2.0);
    let expires_at = data
        .get("expires_at")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let now = chrono::Utc::now().timestamp_millis();
    let expires_at_ms = if expires_at.is_finite() && expires_at > 1e12 {
        expires_at as i64
    } else {
        now.saturating_add(300_000)
    };
    Ok(CliInit {
        flow_id,
        authorize_url,
        interval_secs: interval.round() as i64,
        expires_at_ms,
    })
}

fn parse_cli_poll(body: Value, region: Region) -> Result<PollStatus, Error> {
    let data = unwrap_envelope(body, "cli poll")?;
    let status = trimmed(data.get("status")).unwrap_or_else(|| "pending".to_owned());
    if status != "ready" {
        return Ok(PollStatus::Pending { status });
    }
    let oauth_access = token_from(data.get("zai"))
        .or_else(|| token_from(data.get("zcode")))
        .or_else(|| token_from(data.get("bigmodel")))
        .or_else(|| trimmed(data.get("access_token")))
        .ok_or_else(|| Error::Payload("glm cli poll ready without access token".to_owned()))?;
    let zcode_jwt = trimmed(data.get("token"));
    let from_jwt = account_from_jwt(zcode_jwt.as_deref().unwrap_or_default()).unwrap_or_default();
    let from_oauth = account_from_jwt(&oauth_access).unwrap_or_default();
    let email = pick_human_account(&[
        data.get("user")
            .and_then(|u| u.get("email"))
            .and_then(Value::as_str)
            .unwrap_or(""),
        data.get("user")
            .and_then(|u| u.get("preferred_username"))
            .and_then(Value::as_str)
            .unwrap_or(""),
        data.get("user")
            .and_then(|u| u.get("preferredUsername"))
            .and_then(Value::as_str)
            .unwrap_or(""),
        data.get("email").and_then(Value::as_str).unwrap_or(""),
        from_jwt.as_str(),
        from_oauth.as_str(),
    ]);
    Ok(PollStatus::Ready(ReadyTokens {
        oauth_access,
        zcode_jwt,
        email,
        region,
    }))
}

fn token_from(value: Option<&Value>) -> Option<String> {
    let obj = value.and_then(Value::as_object)?;
    trimmed(obj.get("access_token")).or_else(|| trimmed(obj.get("accessToken")))
}

fn unwrap_envelope(body: Value, operation: &str) -> Result<Value, Error> {
    let Some(obj) = body.as_object() else {
        return Ok(body);
    };
    if !obj.contains_key("code") && !obj.contains_key("success") {
        return Ok(body);
    }
    if obj.get("success") == Some(&Value::Bool(false)) || !is_success_code(obj.get("code")) {
        let msg = obj.get("msg").and_then(Value::as_str).map_or_else(
            || format!("code {}", display_code(obj.get("code"))),
            str::to_owned,
        );
        return Err(Error::Payload(format!("glm {operation} failed: {msg}")));
    }
    Ok(obj.get("data").cloned().unwrap_or(body))
}

fn is_success_code(code: Option<&Value>) -> bool {
    match code {
        None | Some(Value::Null) => true,
        Some(Value::Number(n)) => {
            n.as_i64() == Some(0)
                || n.as_i64() == Some(200)
                || n.as_u64() == Some(0)
                || n.as_u64() == Some(200)
        }
        Some(Value::String(s)) => s == "0" || s == "200",
        Some(_) => false,
    }
}

fn display_code(code: Option<&Value>) -> String {
    match code {
        Some(v) => v.to_string(),
        None => "unknown".to_owned(),
    }
}

fn trimmed(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn join_poll(base: &str, flow_id: &str) -> Result<String, Error> {
    let mut url = url::Url::parse(base).map_err(|err| Error::Payload(err.to_string()))?;
    {
        let mut segs = url
            .path_segments_mut()
            .map_err(|()| Error::Payload("glm poll url is not a path".to_owned()))?;
        segs.pop_if_empty();
        segs.push(flow_id);
    }
    Ok(url.to_string())
}

async fn read_json(response: reqwest::Response, label: &str) -> Result<Value, Error> {
    let status = response.status().as_u16();
    let text = response.text().await?;
    if !(200..300).contains(&status) {
        let body: String = text.chars().take(240).collect();
        return Err(Error::TokenEndpoint { status, body });
    }
    if text.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|err| Error::Payload(format!("{label}: {err}")))
}

async fn get_json(
    url: &str,
    headers: &HeaderMap,
    client: &reqwest::Client,
) -> Result<Value, Error> {
    let response = client.get(url).headers(headers.clone()).send().await?;
    read_json(response, url).await
}

async fn post_json(
    url: &str,
    body: Value,
    headers: &HeaderMap,
    client: &reqwest::Client,
) -> Result<Value, Error> {
    let mut headers = headers.clone();
    insert_header(&mut headers, "accept", "application/json");
    insert_header(&mut headers, "content-type", "application/json");
    let response = client.post(url).headers(headers).json(&body).send().await?;
    read_json(response, url).await
}

fn cli_headers(poll_token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    insert_header(&mut headers, "accept", "application/json");
    insert_header(&mut headers, "content-type", "application/json");
    insert_header(&mut headers, "user-agent", CLI_USER_AGENT);
    insert_header(
        &mut headers,
        "authorization",
        &format!("Bearer {poll_token}"),
    );
    headers
}

fn bearer_desktop(token: &str, session_id: Option<&str>) -> HeaderMap {
    let mut headers = desktop_headers(session_id);
    insert_header(&mut headers, "authorization", &format!("Bearer {token}"));
    insert_header(&mut headers, "accept", "application/json");
    headers
}

fn insert_header(headers: &mut HeaderMap, name: &str, value: &str) {
    let Ok(header) = HeaderName::from_bytes(name.as_bytes()) else {
        return;
    };
    let Ok(header_value) = HeaderValue::from_str(value) else {
        return;
    };
    headers.insert(header, header_value);
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn process_session_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| format!("sess_{}", random_hex_fallback(12)))
}

fn random_hex_fallback(bytes: usize) -> String {
    crate::random_hex(bytes).unwrap_or_else(|_| {
        let uuid = uuid::Uuid::new_v4().simple().to_string();
        uuid.chars()
            .cycle()
            .take(bytes.saturating_mul(2).max(1))
            .collect()
    })
}
