//! 服务面 `/api/service` 的端到端契约测试（ozon-pod 集成契约 §2–§4）。
//!
//! # 夹具是形状的单一事实源
//!
//! `tests/fixtures/service-api.json` 决定**路径、方法、错误码与字段名集合/类型**；
//! 这里逐条从夹具里读出来，而不是把字面量再抄一遍 —— 抄一遍就等于多一份会漂移的
//! 事实源（规则 2.11）。夹具里的**示例值**（邮箱、金额、key id）一律不断言。
//!
//! 不连库的两条（挂载规则与鉴权）没有 `#[ignore]`：它们在任何环境下都必须绿。
//! 其余每条都要真 Postgres，跑法见 [`crate::common`]。

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Method, Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;

use gw_panel::PanelState;

use crate::common::{fresh_db, seed_user, seed_user_with};

/// 契约夹具（编译期嵌进来，路径错就是编译错误）。
const FIXTURE: &str = include_str!("../fixtures/service-api.json");

/// 服务面 token。值本身无意义，能与请求头对上即可。
///
/// 与其他 `pub(crate)` 的辅助一起供 [`crate::service_api_billing`] 复用。
pub(crate) const SERVICE_TOKEN: &str = "service-api-test-token";

pub(crate) fn fixture() -> Value {
    serde_json::from_str(FIXTURE).expect("fixture parses")
}

/// 夹具里的错误码（`errors.<name>.code`）。
pub(crate) fn error_code(fixture: &Value, name: &str) -> i32 {
    i32::try_from(
        fixture["errors"][name]["code"]
            .as_i64()
            .unwrap_or_else(|| panic!("fixture errors.{name}.code")),
    )
    .expect("code fits i32")
}

/// 夹具里的 HTTP 状态（`errors.<name>.http`）。
pub(crate) fn error_http(fixture: &Value, name: &str) -> StatusCode {
    let code = fixture["errors"][name]["http"]
        .as_u64()
        .unwrap_or_else(|| panic!("fixture errors.{name}.http"));
    StatusCode::from_u16(u16::try_from(code).expect("http status fits u16")).expect("status")
}

/// 把夹具路径里的占位符换成可请求的值。
pub(crate) fn endpoint_path(fixture: &Value, name: &str) -> String {
    fixture["endpoints"][name]["path"]
        .as_str()
        .unwrap_or_else(|| panic!("fixture endpoints.{name}.path"))
        .replace("{user_id}", "1")
        .replace("{key}", "k")
}

pub(crate) fn endpoint_method(fixture: &Value, name: &str) -> Method {
    let raw = fixture["endpoints"][name]["method"]
        .as_str()
        .unwrap_or_else(|| panic!("fixture endpoints.{name}.method"));
    Method::from_bytes(raw.as_bytes()).expect("method")
}

/// 每条端点都过一遍夹具的 `method` + `path`。
pub(crate) fn every_endpoint(fixture: &Value) -> Vec<(String, Method, String)> {
    let endpoints = fixture["endpoints"]
        .as_object()
        .expect("fixture endpoints object");
    endpoints
        .keys()
        .map(|name| {
            (
                name.clone(),
                endpoint_method(fixture, name),
                endpoint_path(fixture, name),
            )
        })
        .collect()
}

/// 一个**不会拨号**的池：挂载与鉴权两条用例在任何查询之前就返回了。
pub(crate) fn offline_pool() -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://gw:gw@127.0.0.1:1/gw")
        .expect("connection string parses")
}

/// 在公共面板脚手架上换掉 `service` 配置。
pub(crate) fn state_with_token(pool: &PgPool, token: Option<&str>) -> PanelState {
    let mut state = crate::common::http::panel_state(pool);
    let cfg = gw_config::Config {
        service: gw_config::ServiceConfig {
            token: token.unwrap_or_default().to_owned(),
            ..Default::default()
        },
        ..(*state.cfg).clone()
    };
    state.cfg = Arc::new(cfg);
    state
}

pub(crate) async fn request(
    state: &PanelState,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    let body = match body {
        Some(value) => {
            builder = builder.header(CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&value).expect("serialize body"))
        }
        None => Body::empty(),
    };

    let response = gw_panel::router(state.clone())
        .oneshot(builder.body(body).expect("build request"))
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("response is json")
    };
    (status, json)
}

/// `data` 信封：成功一定是 `code:0 / message:ok / data`。
pub(crate) fn data(body: &Value) -> &Value {
    assert_eq!(body["code"].as_i64(), Some(0), "envelope: {body}");
    assert_eq!(body["message"].as_str(), Some("ok"), "envelope: {body}");
    body.get("data").unwrap_or_else(|| panic!("no data: {body}"))
}

/// 一个 JSON 对象的键集合。
pub(crate) fn keys_of(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("not an object: {value}"))
        .keys()
        .map(String::as_str)
        .collect()
}

/// JSON 值的类型名，用来断言夹具里的类型。
pub(crate) fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// 实际对象的键集合与类型必须与夹具逐键一致（键集合完全相等）。
/// 实际对象的键集合与类型必须与夹具逐键一致（键集合完全相等）。
pub(crate) fn assert_shape(actual: &Value, expected: &Value, context: &str) {
    assert_eq!(keys_of(actual), keys_of(expected), "{context}: 字段名集合");
    for (key, want) in expected.as_object().expect("fixture object") {
        assert_eq!(kind(&actual[key]), kind(want), "{context}.{key}: 类型");
    }
}

/// 夹具里的模型条目只有必填字段，实际实现可以多给价格列，但必填字段一个都不能少。
/// 夹具里的模型条目只有必填字段，实际实现可以多给价格列，但必填字段一个都不能少。
pub(crate) fn assert_shape_superset(actual: &Value, expected: &Value, context: &str) {
    let actual_keys = keys_of(actual);
    for (key, want) in expected.as_object().expect("fixture object") {
        assert!(actual_keys.contains(key.as_str()), "{context}: 缺字段 {key}");
        assert_eq!(kind(&actual[key]), kind(want), "{context}.{key}: 类型");
    }
}

// ---------------------------------------------------------------------------
// 挂载与鉴权（不需要 Postgres）
// ---------------------------------------------------------------------------

/// 契约 §2：`service.token` 为空时整组路由不存在（404），不是"无凭证放行"。
#[tokio::test]
async fn the_whole_group_is_404_without_a_configured_token() {
    let fixture = fixture();
    let state = state_with_token(&offline_pool(), None);

    for (name, method, path) in every_endpoint(&fixture) {
        let (status, _) = request(&state, method, &path, Some(SERVICE_TOKEN), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{name} {path} must not exist");
    }
}

/// 契约 §2：缺凭证与凭证错误都是 401 + `errors.unauthorized.code`。
#[tokio::test]
async fn every_endpoint_rejects_a_missing_or_wrong_token() {
    let fixture = fixture();
    let state = state_with_token(&offline_pool(), Some(SERVICE_TOKEN));
    let want = error_http(&fixture, "unauthorized");
    let want_code = error_code(&fixture, "unauthorized");

    for (name, method, path) in every_endpoint(&fixture) {
        for token in [None, Some("not-the-token"), Some("")] {
            let (status, body) = request(&state, method.clone(), &path, token, None).await;
            assert_eq!(status, want, "{name} {path} token={token:?}: {body}");
            assert_eq!(
                body["code"].as_i64(),
                Some(i64::from(want_code)),
                "{name} {path} token={token:?}: {body}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 建号
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn provisioning_is_idempotent_by_email() {
    let fixture = fixture();
    let pool = fresh_db("service_provision_idempotent").await;
    let state = state_with_token(&pool, Some(SERVICE_TOKEN));
    let path = endpoint_path(&fixture, "provision_user");
    let method = endpoint_method(&fixture, "provision_user");

    // 大小写与空白都要规范化：这是幂等身份的一部分。
    let (status, body) = request(
        &state,
        method.clone(),
        &path,
        Some(SERVICE_TOKEN),
        Some(json!({"email": "  Owner+WS1@Example.COM  ", "display_name": "Workspace 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let created = data(&body);
    assert_shape(
        created,
        &fixture["endpoints"]["provision_user"]["response"],
        "provision_user",
    );
    assert_eq!(created["created"], json!(true));
    assert_eq!(created["email"], json!("owner+ws1@example.com"));
    let user_id = created["user_id"].as_i64().expect("user_id");
    assert_eq!(created["status"].as_str(), Some("active"));

    // 行本身：role/status/concurrency/username 与"没有可用口令"。
    let (role, status_col, concurrency, username, hash, balance): (
        String,
        String,
        i64,
        String,
        String,
        f64,
    ) = sqlx::query_as(
        "SELECT COALESCE(role, ''), COALESCE(status, ''), COALESCE(concurrency, 0), \
                COALESCE(username, ''), COALESCE(password_hash, ''), \
                COALESCE(balance, 0)::float8 \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("read provisioned user");
    assert_eq!(role, "user");
    assert_eq!(status_col, "active");
    assert_eq!(concurrency, 1);
    assert_eq!(username, "Workspace 1");
    assert!(
        hash.starts_with("$2"),
        "供给账号的口令必须是 bcrypt 哈希: {hash}"
    );
    assert!(
        !body.to_string().contains(&hash),
        "哈希绝不回传（明文与哈希都不出现）"
    );
    assert_eq!(balance, 0.0, "契约要求不赠注册额度");

    // 不赠 `initial_register_credit`：这个账号在账本里一条流水都没有。
    let logs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM balance_logs WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("count logs");
    assert_eq!(logs, 0);

    // 第二次：created=false、同一个 user_id、不改余额也不改 username。
    let (status, body) = request(
        &state,
        method,
        &path,
        Some(SERVICE_TOKEN),
        Some(json!({"email": "OWNER+WS1@example.com", "display_name": "别的名字"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let again = data(&body);
    assert_eq!(again["created"], json!(false));
    assert_eq!(again["user_id"].as_i64(), Some(user_id));
    assert_eq!(again["email"], json!("owner+ws1@example.com"));

    let (username, balance): (String, f64) = sqlx::query_as(
        "SELECT COALESCE(username, ''), COALESCE(balance, 0)::float8 FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("read user again");
    assert_eq!(username, "Workspace 1", "幂等路径不许改资料");
    assert_eq!(balance, 0.0, "幂等路径不许动余额");

    // 邮箱不含 `@` 是 4000。
    let (status, body) = request(
        &state,
        endpoint_method(&fixture, "provision_user"),
        &endpoint_path(&fixture, "provision_user"),
        Some(SERVICE_TOKEN),
        Some(json!({"email": "not-an-email"})),
    )
    .await;
    assert_eq!(status, error_http(&fixture, "bad_request"), "{body}");
    assert_eq!(
        body["code"].as_i64(),
        Some(i64::from(error_code(&fixture, "bad_request")))
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn provisioning_refuses_an_account_that_is_not_active() {
    let fixture = fixture();
    let pool = fresh_db("service_provision_suspended").await;
    let state = state_with_token(&pool, Some(SERVICE_TOKEN));
    seed_user_with(&pool, "frozen@example.com", 5.0, "user", "suspended").await;

    let (status, body) = request(
        &state,
        endpoint_method(&fixture, "provision_user"),
        &endpoint_path(&fixture, "provision_user"),
        Some(SERVICE_TOKEN),
        Some(json!({"email": "Frozen@Example.com"})),
    )
    .await;
    assert_eq!(status, error_http(&fixture, "conflict"), "{body}");
    assert_eq!(
        body["code"].as_i64(),
        Some(i64::from(error_code(&fixture, "conflict")))
    );

    // 状态与余额都没被动过。
    let (status_col, balance): (String, f64) = sqlx::query_as(
        "SELECT COALESCE(status, ''), COALESCE(balance, 0)::float8 FROM users \
         WHERE email = 'frozen@example.com'",
    )
    .fetch_one(&pool)
    .await
    .expect("read frozen user");
    assert_eq!(status_col, "suspended");
    assert_eq!(balance, 5.0);
}

// ---------------------------------------------------------------------------
// Key 轮换
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn rotating_a_key_revokes_the_same_name_and_kills_the_old_secret() {
    let fixture = fixture();
    let pool = fresh_db("service_key_rotation").await;
    let state = state_with_token(&pool, Some(SERVICE_TOKEN));
    let user = seed_user(&pool, "owner@example.com", 0.0).await;
    let path = format!("/api/service/users/{user}/keys");
    let name = "ozon-pod:workspace:1";

    let (status, body) = request(
        &state,
        Method::POST,
        &path,
        Some(SERVICE_TOKEN),
        Some(json!({"name": name})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let first = data(&body);
    assert_shape(
        first,
        &fixture["endpoints"]["rotate_key"]["response"],
        "rotate_key",
    );
    assert_eq!(first["rotated"], json!(false), "第一次没有旧 key 可撤");
    assert_eq!(first["name"].as_str(), Some(name));
    let first_key = first["key"].as_str().expect("plaintext key").to_owned();
    let first_prefix = first["key_prefix"].as_str().expect("prefix").to_owned();
    assert!(
        first_key.starts_with(&first_prefix),
        "前缀必须是明文的开头"
    );

    // 旧 key 现在真的能用 —— 这一步会把 active 状态灌进 APIKeyCache。
    let (status, _) = request(
        &state,
        Method::GET,
        "/api/panel/user/profile",
        Some(&first_key),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 同名重发 = 轮换。
    let (status, body) = request(
        &state,
        Method::POST,
        &path,
        Some(SERVICE_TOKEN),
        Some(json!({"name": name})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let second = data(&body);
    assert_eq!(second["rotated"], json!(true));
    let second_key = second["key"].as_str().expect("plaintext key").to_owned();
    assert_ne!(first_key, second_key);

    // 契约的"超时重试不会留下两个 active key"：库里同名的 active 只剩一条。
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM api_keys \
         WHERE user_id = $1 AND name = $2 AND status = 'active'",
    )
    .bind(user)
    .bind(name)
    .fetch_one(&pool)
    .await
    .expect("count active keys");
    assert_eq!(active, 1);

    // 明文绝不落库。
    let (hash, prefix): (String, String) = sqlx::query_as(
        "SELECT COALESCE(key_hash, ''), COALESCE(key_prefix, '') FROM api_keys \
         WHERE id = $1",
    )
    .bind(second["key_id"].as_i64().expect("key_id"))
    .fetch_one(&pool)
    .await
    .expect("read new key row");
    assert_ne!(hash, second_key);
    assert!(second_key.starts_with(&prefix));

    // 轮换之后旧 key 立刻不通：DB 状态 + 已失效的 L1 缓存两条路都要挡住。
    let (status, _) = request(
        &state,
        Method::GET,
        "/api/panel/user/profile",
        Some(&first_key),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "旧 key 必须立即失效");
    let (status, _) = request(
        &state,
        Method::GET,
        "/api/panel/user/profile",
        Some(&second_key),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "新 key 必须可用");

    // 未知 user 是 404/4004；名字越界是 4000。
    let (status, body) = request(
        &state,
        Method::POST,
        "/api/service/users/999999/keys",
        Some(SERVICE_TOKEN),
        Some(json!({"name": name})),
    )
    .await;
    assert_eq!(status, error_http(&fixture, "not_found"), "{body}");
    assert_eq!(
        body["code"].as_i64(),
        Some(i64::from(error_code(&fixture, "not_found")))
    );

    for bad_name in [json!(""), json!("x".repeat(65))] {
        let (status, body) = request(
            &state,
            Method::POST,
            &path,
            Some(SERVICE_TOKEN),
            Some(json!({"name": bad_name})),
        )
        .await;
        assert_eq!(status, error_http(&fixture, "bad_request"), "{body}");
    }
}

// ---------------------------------------------------------------------------
// 余额
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn balance_is_a_decimal_string_in_the_configured_currency() {
    let fixture = fixture();
    let pool = fresh_db("service_balance_shape").await;
    let state = state_with_token(&pool, Some(SERVICE_TOKEN));
    let user = seed_user(&pool, "rich@example.com", 12.34).await;

    let (status, body) = request(
        &state,
        Method::GET,
        &format!("/api/service/users/{user}/balance"),
        Some(SERVICE_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let view = data(&body);
    assert_shape(
        view,
        &fixture["endpoints"]["balance"]["response"],
        "balance",
    );
    assert_eq!(view["balance"].as_str(), Some("12.34000000"));
    assert_eq!(view["currency"].as_str(), Some("USD"));

    let (status, body) = request(
        &state,
        Method::GET,
        "/api/service/users/999999/balance",
        Some(SERVICE_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, error_http(&fixture, "not_found"), "{body}");
    assert_eq!(
        body["code"].as_i64(),
        Some(i64::from(error_code(&fixture, "not_found")))
    );
}
