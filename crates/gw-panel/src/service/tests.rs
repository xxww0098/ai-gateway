//! 服务面的纯函数单测：常量时间比较、金额/时间格式、时间窗校验，以及鉴权提取器
//! 本身。
//!
//! 需要真 Postgres 的那一档在 `tests/panel/service_api.rs`（走真 router），这里只
//! 覆盖"不碰库就能判定"的那部分 —— 提取器用 `connect_lazy` 的池，断言都在任何查询
//! 之前就返回了。

use std::sync::Arc;

use axum::extract::FromRequestParts as _;
use axum::http::StatusCode;
use axum::http::request::Parts;
use chrono::{DateTime, Duration, Utc};

use super::usage::{UsageWindow, parse_window};
use super::{ServiceCaller, amount_str, constant_time_eq, timestamp};
use crate::PanelState;

/// 契约 §2 的两种 401 共用体：`401` + 业务码 `1001`。
const UNAUTHORIZED_CODE: i32 = 1001;

/// 一个不会拨号的 `PanelState`：断言在提取器里完成，池只需要存在。
fn state_with_token(token: &str) -> PanelState {
    let cfg = gw_config::Config {
        service: gw_config::ServiceConfig {
            token: token.to_owned(),
            ..Default::default()
        },
        ..Default::default()
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://gw:gw@127.0.0.1:1/gw")
        .expect("connection string parses");
    let price_cache = Arc::new(gw_pricing::ModelPriceCache::empty());
    PanelState {
        pg: pool.clone(),
        cfg: Arc::new(cfg),
        calc: Arc::new(gw_pricing::Calculator::new(
            Some(Arc::clone(&price_cache)),
            0.0,
        )),
        price_cache,
        ledger: Arc::new(gw_ledger::Ledger::new(pool.clone(), None)),
        auth_store: Arc::new(
            gw_authcore::PostgresAuthStore::new(pool, "").expect("build auth store"),
        ),
        user_status_cache: Arc::new(gw_infra::UserStatusCache::new()),
        api_key_cache: Arc::new(gw_infra::ApiKeyCache::new()),
        audit_hmac_key: None,
        stripe_webhook_secret: None,
    }
}

fn parts_with(bearer: Option<&str>) -> Parts {
    let mut builder = axum::http::Request::builder().uri("/api/service/users");
    if let Some(token) = bearer {
        builder = builder.header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}"),
        );
    }
    builder.body(()).expect("build request").into_parts().0
}

/// 提取失败时返回 `(HTTP 状态, 业务码)`。
async fn rejection(state: &PanelState, bearer: Option<&str>) -> (StatusCode, i32) {
    let mut parts = parts_with(bearer);
    let response = ServiceCaller::from_request_parts(&mut parts, state)
        .await
        .expect_err("request must be rejected");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("envelope is json");
    let code = body["code"].as_i64().expect("code is an integer");
    (status, i32::try_from(code).expect("code fits"))
}

#[tokio::test]
async fn the_exact_token_passes_and_everything_else_is_401() {
    let state = state_with_token("shared-secret");

    // `bearer_token` 会把头部值两侧的空白裁掉，所以带空白的写法与不带等价。
    for bearer in ["shared-secret", " shared-secret", "shared-secret "] {
        let mut parts = parts_with(Some(bearer));
        assert!(
            ServiceCaller::from_request_parts(&mut parts, &state).await.is_ok(),
            "bearer={bearer:?}"
        );
    }

    for bearer in [
        None,
        Some(""),
        Some("wrong"),
        Some("shared-secre"),
        Some("shared-secrets"),
        Some("Shared-Secret"),
        // token 内部的空白是秘密的一部分。
        Some("shared secret"),
    ] {
        let (status, code) = rejection(&state, bearer).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "bearer={bearer:?}");
        assert_eq!(code, UNAUTHORIZED_CODE, "bearer={bearer:?}");
    }
}

/// 请求头两侧的空白与配置侧按同一规则裁掉：配置 `"  padded  "` + 头 `"padded"` 通过，
/// 否则一个尾部带空格的 token 会永远匹配不上，而写它的人看不出来。
#[tokio::test]
async fn surrounding_whitespace_is_trimmed_on_both_sides() {
    let state = state_with_token("  padded  ");
    let mut parts = parts_with(Some("padded"));
    assert!(ServiceCaller::from_request_parts(&mut parts, &state).await.is_ok());

    // token **内部**的空白仍是秘密的一部分：这不是同一个秘密。
    let (status, code) = rejection(&state, Some("pa dded")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(code, UNAUTHORIZED_CODE);
}

#[tokio::test]
async fn a_blank_configured_token_never_authenticates() {
    // 路由本来就不该挂载；这是纵深防御，所以连"空 token 对空 token"也必须拒。
    let state = state_with_token("   ");
    let (status, code) = rejection(&state, Some("   ")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(code, UNAUTHORIZED_CODE);
}

#[test]
fn constant_time_comparison_is_an_equality_test() {
    assert!(constant_time_eq(b"", b""));
    assert!(constant_time_eq(b"abc", b"abc"));
    assert!(!constant_time_eq(b"abc", b"abd"));
    assert!(!constant_time_eq(b"abc", b"zbc"));
    assert!(!constant_time_eq(b"abc", b"ab"));
    assert!(!constant_time_eq(b"ab", b"abc"));
    assert!(!constant_time_eq(b"abc", b""));
}

#[test]
fn amounts_are_eight_decimal_places() {
    assert_eq!(amount_str(0.0), "0.00000000");
    // 负零不是金额：它必须渲染成零，而不是 "-0.00000000"。
    assert_eq!(amount_str(-0.0), "0.00000000");
    assert_eq!(amount_str(12.34), "12.34000000");
    assert_eq!(amount_str(0.00015), "0.00015000");
    assert_eq!(amount_str(100_000.0), "100000.00000000");
    // 超过 8 位的部分四舍五入，不会漏出一串浮点尾数。
    assert_eq!(amount_str(0.1 + 0.2), "0.30000000");
}

#[test]
fn timestamps_are_second_precision_rfc3339_utc() {
    let at = DateTime::parse_from_rfc3339("2026-09-11T10:00:00.123456789Z")
        .expect("parse")
        .with_timezone(&Utc);
    assert_eq!(timestamp(at), "2026-09-11T10:00:00Z");

    let offset = DateTime::parse_from_rfc3339("2026-09-11T18:00:00+08:00")
        .expect("parse")
        .with_timezone(&Utc);
    assert_eq!(timestamp(offset), "2026-09-11T10:00:00Z");
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-09-11T10:00:00Z")
        .expect("parse")
        .with_timezone(&Utc)
}

#[test]
fn an_absent_window_is_the_last_seven_days() {
    let window = parse_window(None, None, now()).expect("default window");
    assert_eq!(window.end, now());
    assert_eq!(window.start, now() - Duration::days(7));
}

#[test]
fn a_missing_end_uses_the_callers_now() {
    let start = now() - Duration::days(3);
    let window = parse_window(Some(&start.to_rfc3339()), None, now()).expect("window");
    assert_eq!(window.start, start);
    assert_eq!(window.end, now());
}

#[test]
fn the_window_ceiling_is_ninety_days_inclusive() {
    let end = now();
    let exactly = end - Duration::days(90);
    assert_eq!(
        parse_window(Some(&exactly.to_rfc3339()), Some(&end.to_rfc3339()), now())
        .expect("90 days is allowed"),
        UsageWindow { start: exactly, end }
    );

    let too_wide = end - Duration::days(90) - Duration::seconds(1);
    assert!(
        parse_window(Some(&too_wide.to_rfc3339()), Some(&end.to_rfc3339()), now()).is_err(),
        "91 days must be refused"
    );
}

#[test]
fn an_inverted_or_unparseable_window_is_refused() {
    let end = now();
    let later = end + Duration::hours(1);
    assert!(parse_window(Some(&later.to_rfc3339()), Some(&end.to_rfc3339()), now()).is_err());
    assert!(parse_window(Some("not-a-time"), None, now()).is_err());
    assert!(parse_window(None, Some("2026-09-11"), now()).is_err());
    // 空串按"没传"处理，与 query_int 的语义一致。
    assert!(parse_window(Some("  "), Some(""), now()).is_ok());
}
