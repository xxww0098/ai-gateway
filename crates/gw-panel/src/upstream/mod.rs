//! 上游域：上游凭证与运行时管理面。
//!
//! Owner: worker `panel-upstream`。上游凭证与运行时管理面的五个业务域，
//! 外加一条挂在组外的 `PATCH /admin/sdk-config`。
//!
//! # 为什么拆成七个文件
//!
//! 规则 1.10：单文件超 1,000 行就该停下来看一眼。这一域按**它服务的东西**
//! 切开，删掉一个功能仍然等于删掉一个文件：
//!
//! | 模块 | 管什么 | 对应的 handler/helper |
//! | --- | --- | --- |
//! | [`record`] | [`AuthRecord`](gw_authcore::AuthRecord) 的读取与三种序列化 | `sdkMgmtAttr*` / `sdkMgmtSerialize*` |
//! | [`auth_files`] | 凭证清单：列表 / 上传 / 启停 / 删除 / 配额 / 模型 | `SDKMgmtAuthFiles*Handler` |
//! | [`providers`] | 五个 provider 的 API Key 池 CRUD + 用量桶 | `SDKMgmtProvider*Handler` |
//! | [`oauth`] | Gemini / Claude / Codex / xAI / Kiro 的 OAuth 起止与会话 | `SDKMgmtOAuth*` |
//! | [`ampcode`] | Ampcode 上游的配置块 | `SDKMgmtAmpcode*Handler` |
//! | [`runtime_config`] | 运行时设置块与十八条便捷键路由 | `SDKMgmtConfig*` |
//! | [`logs`] | 日志面板与模型目录 | `SDKMgmtLogs*` / `SDKMgmtModelDefinitions*` |
//!
//! # 路由注册顺序是有意义的
//!
//! `/{provider}` 是一条通配路由，`/auth-files`、`/oauth-sessions`、`/config` 等等
//! 都会被它吃掉。gin 靠 radix 树把静态路由排在参数路由前面；axum 也一样，但
//! **前提是静态路由真的注册了** —— 少写一条，它就会静默落到 `/{provider}` 上并
//! 得到一个 `unknown provider` 404。这里的顺序照 API 面分组，别乱动。
//!
//! # 两块已无后端的能力
//!
//! `POST /oauth-callback`（不带 provider）与 `antigravity-` / `kimi-auth-url`
//! 不再有可转发的目标，这里保留了「没接上」分支
//! （503 / 404），现在走的就是那两条，不是新造的错误。详见 [`oauth`]。
//!
//! 另一处：凭证的 `success` / `failed` / `recent_requests` 是 SDK manager 的
//! 进程内计数器，`model.AuthRecord` 明确不落库，所以恒为 0 / 0 / `[]` ——
//! 重启后到有流量之前也是这个值。详见 [`record`]。

pub mod ampcode;
pub mod auth_files;
pub mod logs;
pub mod oauth;
pub mod providers;
pub mod record;
pub mod runtime_config;

use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post, put};
use serde_json::Value;

use crate::{AdminUser, PanelState};

/// Prefix every route in this domain sits under, except the one deliberately
/// mounted outside it (`PATCH /admin/sdk-config`).
const BASE: &str = "/admin/sdk-management";

/// Routes owned by this domain. Paths are relative to the panel mount point.
pub fn router() -> Router<PanelState> {
    Router::new()
        // ── static routes: must be registered so `/{provider}` cannot eat them ──
        .route(&path("/api-key-usage"), get(providers::api_key_usage))
        .route(
            &path("/auth-files"),
            get(auth_files::list)
                .post(auth_files::create)
                .put(auth_files::update)
                .delete(auth_files::remove),
        )
        .route(&path("/auth-files/quota"), get(auth_files::quota))
        .route(&path("/auth-files/models"), get(auth_files::models))
        .route(
            &path("/auth-files/import-local"),
            post(auth_files::import_local),
        )
        .route(&path("/oauth-sessions"), get(oauth::list_sessions))
        .route(&path("/get-auth-status"), get(oauth::auth_status))
        .route(&path("/oauth-callback"), post(oauth::sdk_callback))
        .route(&path("/oauth-callback/{provider}"), post(oauth::callback))
        .route(
            &path("/oauth-device-poll/{provider}"),
            post(oauth::device_poll),
        )
        // ── ampcode ──
        .route(&path("/ampcode"), get(ampcode::get).put(ampcode::put))
        .route(
            &path("/ampcode/model-mappings"),
            get(ampcode::get_model_mappings)
                .put(ampcode::put_model_mappings)
                .delete(ampcode::delete_model_mappings),
        )
        .route(
            &path("/ampcode/upstream-api-keys"),
            get(ampcode::get_upstream_api_keys)
                .put(ampcode::put_upstream_api_keys)
                .delete(ampcode::delete_upstream_api_keys),
        )
        .route(
            &path("/ampcode/upstream-url"),
            put(ampcode::put_upstream_url).delete(ampcode::delete_upstream_url),
        )
        .route(
            &path("/ampcode/upstream-api-key"),
            put(ampcode::put_upstream_api_key).delete(ampcode::delete_upstream_api_key),
        )
        // ── the settings blob ──
        .route(
            &path("/config"),
            get(runtime_config::get_config).put(runtime_config::put_config),
        )
        .route(
            &path("/routing/strategy"),
            get(get_routing_strategy).put(set_routing_strategy),
        )
        .route(
            &path("/force-model-prefix"),
            get(get_force_model_prefix).put(set_force_model_prefix),
        )
        .route(
            &path("/logs-max-total-size-mb"),
            get(get_logs_max_size).put(set_logs_max_size),
        )
        // ── logs ──
        .route(&path("/logs"), get(logs::list).delete(logs::clear))
        .route(
            &path("/request-error-logs"),
            get(logs::list_errors).delete(logs::clear_errors),
        )
        .route(
            &path("/model-definitions/{channel}"),
            get(logs::model_definitions),
        )
        // ── the plain config keys, plus the write-only one ──
        .merge(plain_config_routes())
        .route(
            &path(&format!("/{}", runtime_config::PROXY_URL_KEY)),
            put(set_proxy_url).delete(delete_proxy_url),
        )
        // ── the wildcard, registered last ──
        .route(
            &path("/{provider}"),
            get(providers::get)
                .post(providers::post)
                .put(providers::put)
                .delete(providers::delete),
        )
        // This one lives on the authed group, NOT under sdk-management.
        // Same handler as `PUT /config`.
        .route(
            "/admin/sdk-config",
            axum::routing::patch(runtime_config::put_config),
        )
}

fn path(suffix: &str) -> String {
    format!("{BASE}{suffix}")
}

/// The keys whose GET/PUT pair carries no aliases and no default.
///
/// One macro-free table beats eight near-identical handler pairs: axum needs a
/// distinct `fn` per route, so each key gets a tiny closure rather than a
/// hand-written copy of the same six lines.
fn plain_config_routes() -> Router<PanelState> {
    let mut router = Router::new();
    for key in runtime_config::PLAIN_KEYS {
        let spec = runtime_config::plain(key);
        let route = get(
            move |State(state): State<PanelState>, _admin: AdminUser| async move {
                runtime_config::get_key(&state, spec).await
            },
        )
        .put(
            move |State(state): State<PanelState>,
                  _admin: AdminUser,
                  body: Option<axum::Json<Value>>| async move {
                runtime_config::set_key(&state, key, body).await
            },
        );
        router = router.route(&path(&format!("/{key}")), route);
    }
    router
}

/// `PUT /proxy-url`. 对应 `sdkMgmtConfigSetHandlerFn("proxy-url")`。
async fn set_proxy_url(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<Value>>,
) -> Response {
    runtime_config::set_key(&state, runtime_config::PROXY_URL_KEY, body).await
}

/// `DELETE /proxy-url`. 对应 `sdkMgmtConfigDeleteHandlerFn("proxy-url")` ——
/// only config key with a delete route.
async fn delete_proxy_url(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    runtime_config::delete_key(&state, runtime_config::PROXY_URL_KEY).await
}

// The three keys with aliases or defaults need named handlers, because their
// GET response shape differs from the plain ones.

async fn get_routing_strategy(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    runtime_config::get_key(&state, runtime_config::ROUTING_STRATEGY).await
}

async fn set_routing_strategy(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<Value>>,
) -> Response {
    runtime_config::set_key(&state, runtime_config::ROUTING_STRATEGY.key, body).await
}

async fn get_force_model_prefix(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    runtime_config::get_key(&state, runtime_config::FORCE_MODEL_PREFIX).await
}

async fn set_force_model_prefix(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<Value>>,
) -> Response {
    runtime_config::set_key(&state, runtime_config::FORCE_MODEL_PREFIX.key, body).await
}

async fn get_logs_max_size(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    runtime_config::get_key(&state, runtime_config::LOGS_MAX_TOTAL_SIZE_MB).await
}

async fn set_logs_max_size(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<Value>>,
) -> Response {
    runtime_config::set_key(&state, runtime_config::LOGS_MAX_TOTAL_SIZE_MB.key, body).await
}
