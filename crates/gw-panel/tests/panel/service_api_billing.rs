//! 服务面的用量、入账与模型目录契约测试（ozon-pod 集成契约 §3.4–§3.7）。
//!
//! 与 [`crate::service_api`]（挂载、鉴权、建号、发 key、余额）同属服务面那一组
//! 用例；拆成两个文件只为守住 xtask 的 1,000 行上限（规则 1.10），公共脚手架仍
//! 从 `crate::service_api` 复用，不复制一份。

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{balance_of, fresh_db, seed_user};
use crate::service_api::{
    SERVICE_TOKEN, assert_shape, assert_shape_superset, data, endpoint_path, error_code,
    error_http, fixture, request, state_with_token,
};

// ---------------------------------------------------------------------------
// 用量
// ---------------------------------------------------------------------------

/// 插一条 `usage_logs`；只填端点会读的那几列，其余走表默认值。
#[allow(clippy::too_many_arguments)]
async fn seed_usage(
    pool: &PgPool,
    user: i64,
    idempotency_key: Option<&str>,
    model: &str,
    failed: bool,
    cost: f64,
    created_at: chrono::DateTime<chrono::Utc>,
) {
    sqlx::query(
        "INSERT INTO usage_logs \
             (user_id, api_key_id, request_id, idempotency_key, model, provider, \
              tokens_in, tokens_out, input_tokens, output_tokens, \
              total_cost, actual_cost, cost, failed, duration_ms, created_at) \
         VALUES ($1, 0, $2, $3, $4, 'openai', \
                 120, 30, 120, 30, $5, $6, $5, $7, 812, $8)",
    )
    .bind(user)
    .bind(format!("req-{model}-{}", created_at.timestamp()))
    .bind(idempotency_key)
    .bind(model)
    .bind(cost)
    .bind(if failed { 0.0 } else { cost })
    .bind(failed)
    .bind(created_at)
    .execute(pool)
    .await
    .expect("seed usage log");
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn usage_rows_match_the_fixture_shape() {
    let fixture = fixture();
    let pool = fresh_db("service_usage_shape").await;
    let state = state_with_token(&pool, Some(SERVICE_TOKEN));
    let user = seed_user(&pool, "consumer@example.com", 0.0).await;
    let now = chrono::Utc::now();

    seed_usage(&pool, user, Some("ozon:req-1"), "gpt-4o", false, 0.00015, now).await;
    seed_usage(
        &pool,
        user,
        None,
        "gpt-4o-mini",
        true,
        0.5,
        now - chrono::Duration::hours(1),
    )
    .await;
    // 窗口之外：默认最近 7 天，它不该被算进来。
    seed_usage(
        &pool,
        user,
        None,
        "ancient",
        false,
        1.0,
        now - chrono::Duration::days(8),
    )
    .await;

    let (status, body) = request(
        &state,
        Method::GET,
        &format!("/api/service/users/{user}/usage"),
        Some(SERVICE_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let page = data(&body);
    assert_shape(
        page,
        &fixture["endpoints"]["usage"]["response"],
        "usage",
    );
    assert_eq!(page["total"].as_i64(), Some(2), "8 天前的那条在默认窗口之外");
    assert_eq!(page["page"].as_i64(), Some(1));
    assert_eq!(page["page_size"].as_i64(), Some(50));

    let rows = page["rows"].as_array().expect("rows array");
    assert_eq!(rows.len(), 2);
    let expected_row = &fixture["endpoints"]["usage"]["response"]["rows"][0];
    for row in rows {
        assert_shape(row, expected_row, "usage.rows[]");
        assert_eq!(row["tokens"].as_i64(), row["tokens_in"].as_i64().map(|v| v + 30));
        let cost = row["cost"].as_str().expect("cost is a string");
        let actual = row["actual_cost"].as_str().expect("actual_cost is a string");
        assert_eq!(cost.len(), cost.find('.').expect("decimal point") + 9);
        assert_eq!(actual.len(), actual.find('.').expect("decimal point") + 9);
    }

    // 最新的一条排在最前；失败请求的 actual_cost 是 0（写入方三列金额都清零）。
    let newest = &rows[0];
    assert_eq!(newest["idempotency_key"].as_str(), Some("ozon:req-1"));
    assert_eq!(newest["cost"].as_str(), Some("0.00015000"));
    assert_eq!(newest["actual_cost"].as_str(), Some("0.00015000"));
    assert_eq!(newest["tokens_in"].as_i64(), Some(120));
    assert_eq!(newest["tokens_out"].as_i64(), Some(30));
    assert_eq!(newest["tokens"].as_i64(), Some(150));
    assert_eq!(newest["failed"], json!(false));
    assert_eq!(newest["duration_ms"].as_i64(), Some(812));
    let created_at = newest["created_at"].as_str().expect("created_at string");
    assert!(
        created_at.ends_with('Z') && created_at.len() == 20,
        "秒精度 RFC3339 UTC: {created_at}"
    );

    let failed_row = &rows[1];
    assert_eq!(failed_row["failed"], json!(true));
    assert_eq!(failed_row["actual_cost"].as_str(), Some("0.00000000"));

    // 窗口上限 90 天：超一天就是 4000。
    let start = (now - chrono::Duration::days(91)).to_rfc3339();
    let (status, body) = request(
        &state,
        Method::GET,
        &format!("/api/service/users/{user}/usage?start={start}"),
        Some(SERVICE_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, error_http(&fixture, "bad_request"), "{body}");
    assert_eq!(
        body["code"].as_i64(),
        Some(i64::from(error_code(&fixture, "bad_request")))
    );

    // 页大小上限 200（契约 §3.4），夹紧而不是报错。
    let (status, body) = request(
        &state,
        Method::GET,
        &format!("/api/service/users/{user}/usage?page_size=1000&page=2"),
        Some(SERVICE_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(data(&body)["page_size"].as_i64(), Some(200));
    assert_eq!(data(&body)["page"].as_i64(), Some(2));

    // 未知 user 是 404；时间格式非法是 4000。
    let (status, body) = request(
        &state,
        Method::GET,
        "/api/service/users/999999/usage",
        Some(SERVICE_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, error_http(&fixture, "not_found"), "{body}");
    let (status, body) = request(
        &state,
        Method::GET,
        &format!("/api/service/users/{user}/usage?start=yesterday"),
        Some(SERVICE_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, error_http(&fixture, "bad_request"), "{body}");
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn usage_by_idempotency_key_reports_found_and_not_found() {
    let fixture = fixture();
    let pool = fresh_db("service_usage_by_key").await;
    let state = state_with_token(&pool, Some(SERVICE_TOKEN));
    let user = seed_user(&pool, "recon@example.com", 0.0).await;
    seed_usage(
        &pool,
        user,
        Some("ozon:req-1"),
        "gpt-4o",
        false,
        0.00015,
        chrono::Utc::now(),
    )
    .await;

    // `:` 百分号编码后由 axum 解回原值。
    let (status, body) = request(
        &state,
        Method::GET,
        &format!("/api/service/users/{user}/usage/by-idempotency-key/ozon%3Areq-1"),
        Some(SERVICE_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let lookup = data(&body);
    assert_shape(
        lookup,
        &fixture["endpoints"]["usage_by_idempotency_key"]["response"],
        "usage_by_idempotency_key",
    );
    assert_eq!(lookup["found"], json!(true));
    assert_shape(
        &lookup["row"],
        &fixture["endpoints"]["usage_by_idempotency_key"]["response"]["row"],
        "usage_by_idempotency_key.row",
    );
    assert_eq!(lookup["row"]["idempotency_key"].as_str(), Some("ozon:req-1"));

    // 未命中不是 404：200 + found:false + row:null。
    let (status, body) = request(
        &state,
        Method::GET,
        &format!("/api/service/users/{user}/usage/by-idempotency-key/ozon%3Areq-404"),
        Some(SERVICE_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let lookup = data(&body);
    assert_eq!(lookup["found"], json!(false));
    assert_eq!(lookup["row"], Value::Null);

    // 未知 user 才是 404/4004。
    let (status, body) = request(
        &state,
        Method::GET,
        "/api/service/users/999999/usage/by-idempotency-key/ozon%3Areq-1",
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

// ---------------------------------------------------------------------------
// 入账
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn credits_are_idempotent_and_conflicts_fail_closed() {
    let fixture = fixture();
    let pool = fresh_db("service_credits").await;
    let state = state_with_token(&pool, Some(SERVICE_TOKEN));
    let user = seed_user(&pool, "payer@example.com", 0.0).await;
    let other = seed_user(&pool, "other@example.com", 0.0).await;
    let path = format!("/api/service/users/{user}/credits");
    let key = "ozon-order:PO-1";

    let (status, body) = request(
        &state,
        Method::POST,
        &path,
        Some(SERVICE_TOKEN),
        Some(json!({"amount": "10.00000000", "idempotency_key": key, "note": "充值"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let applied = data(&body);
    assert_shape(
        applied,
        &fixture["endpoints"]["credit"]["response"],
        "credit",
    );
    assert_eq!(applied["applied"], json!(true));
    assert_eq!(applied["duplicate"], json!(false));
    assert_eq!(applied["balance"].as_str(), Some("10.00000000"));

    // 重放：不重复入账，余额原样返回。
    let (status, body) = request(
        &state,
        Method::POST,
        &path,
        Some(SERVICE_TOKEN),
        Some(json!({"amount": "10.00000000", "idempotency_key": key})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let again = data(&body);
    assert_eq!(again["applied"], json!(false));
    assert_eq!(again["duplicate"], json!(true));
    assert_eq!(again["balance"].as_str(), Some("10.00000000"));
    assert!((balance_of(&pool, user).await - 10.0).abs() < 1e-9);

    let reference = format!("service_credit:{key}");
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM balance_logs WHERE user_id = $1 AND reference = $2",
    )
    .bind(user)
    .bind(&reference)
    .fetch_one(&pool)
    .await
    .expect("count credit rows");
    assert_eq!(rows, 1, "同一 reference 只许有一条入账流水");

    // 同 reference 换金额 → 409。
    let (status, body) = request(
        &state,
        Method::POST,
        &path,
        Some(SERVICE_TOKEN),
        Some(json!({"amount": "5.00000000", "idempotency_key": key})),
    )
    .await;
    assert_eq!(status, error_http(&fixture, "conflict"), "{body}");
    assert_eq!(
        body["code"].as_i64(),
        Some(i64::from(error_code(&fixture, "conflict")))
    );

    // 同 reference 换 user → 409，且另一个人分文未进。
    let (status, body) = request(
        &state,
        Method::POST,
        &format!("/api/service/users/{other}/credits"),
        Some(SERVICE_TOKEN),
        Some(json!({"amount": "10.00000000", "idempotency_key": key})),
    )
    .await;
    assert_eq!(status, error_http(&fixture, "conflict"), "{body}");
    assert!((balance_of(&pool, other).await - 0.0).abs() < 1e-9);

    // 金额与幂等键的边界：全部 4000。
    for payload in [
        json!({"amount": "0", "idempotency_key": "k-zero"}),
        json!({"amount": "-1.00000000", "idempotency_key": "k-negative"}),
        json!({"amount": "100000.00000001", "idempotency_key": "k-over-max"}),
        json!({"amount": "NaN", "idempotency_key": "k-nan"}),
        json!({"idempotency_key": "k-missing-amount"}),
        json!({"amount": "1.00000000", "idempotency_key": ""}),
        json!({"amount": "1.00000000", "idempotency_key": "x".repeat(129)}),
    ] {
        let (status, body) = request(
            &state,
            Method::POST,
            &path,
            Some(SERVICE_TOKEN),
            Some(payload.clone()),
        )
        .await;
        assert_eq!(
            status,
            error_http(&fixture, "bad_request"),
            "payload={payload}: {body}"
        );
        assert_eq!(
            body["code"].as_i64(),
            Some(i64::from(error_code(&fixture, "bad_request"))),
            "payload={payload}"
        );
    }

    // 未知 user 是 404（账本先锁行，锁不到就是 UserNotFound）。
    let (status, body) = request(
        &state,
        Method::POST,
        "/api/service/users/999999/credits",
        Some(SERVICE_TOKEN),
        Some(json!({"amount": "1.00000000", "idempotency_key": "k-ghost"})),
    )
    .await;
    assert_eq!(status, error_http(&fixture, "not_found"), "{body}");

    // 另一笔正常入账：余额是可加的，且 JSON 数字金额（超集写法）也接受。
    let (status, body) = request(
        &state,
        Method::POST,
        &path,
        Some(SERVICE_TOKEN),
        Some(json!({"amount": 2.5, "idempotency_key": "ozon-order:PO-2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(data(&body)["balance"].as_str(), Some("12.50000000"));
    assert!((balance_of(&pool, user).await - 12.5).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// 模型目录
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn models_mirror_the_visible_catalog_and_current_prices() {
    let fixture = fixture();
    let pool = fresh_db("service_models").await;
    let state = state_with_token(&pool, Some(SERVICE_TOKEN));

    for (channel, model, visible) in [
        ("openai", "gpt-4o", true),
        ("azure", "gpt-4o", true),
        ("openai", "hidden-model", false),
        // 设置行：渠道的模型列表地址，不是模型。
        ("openai", "__models_url__", false),
    ] {
        sqlx::query(
            "INSERT INTO model_catalog_entries (channel_key, model_id, visible, created_at, updated_at) \
             VALUES ($1, $2, $3, NOW(), NOW()) \
             ON CONFLICT (channel_key, model_id) DO UPDATE SET visible = EXCLUDED.visible",
        )
        .bind(channel)
        .bind(model)
        .bind(visible)
        .execute(&pool)
        .await
        .expect("seed catalog entry");
    }
    sqlx::query(
        "INSERT INTO model_prices (model_id, input_price_per1_m, output_price_per1_m, \
                                   cached_input_price_per1_m, reasoning_price_per1_m, \
                                   created_at, updated_at) \
         VALUES ('gpt-4o', 0.00015, 0.0006, 0.000075, 0.0006, NOW(), NOW()) \
         ON CONFLICT (model_id) DO UPDATE SET input_price_per1_m = EXCLUDED.input_price_per1_m",
    )
    .execute(&pool)
    .await
    .expect("seed price");

    let (status, body) = request(
        &state,
        Method::GET,
        &endpoint_path(&fixture, "models"),
        Some(SERVICE_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let view = data(&body);
    assert_shape(
        view,
        &fixture["endpoints"]["models"]["response"],
        "models",
    );

    let models = view["models"].as_array().expect("models array");
    assert_eq!(models.len(), 1, "隐藏模型与设置行都不出现: {models:?}");
    let model = &models[0];
    assert_shape_superset(
        model,
        &fixture["endpoints"]["models"]["response"]["models"][0],
        "models[0]",
    );
    assert_eq!(model["id"].as_str(), Some("gpt-4o"));
    // 同一模型被多个渠道提供时，与 /v1/models 一样取渠道字典序最小的那个。
    assert_eq!(model["owned_by"].as_str(), Some("azure"));

    // 价格字段名就是 gw-pricing 的列名，金额是 decimal 字符串。
    for field in [
        "input_price_per1_m",
        "output_price_per1_m",
        "cached_input_price_per1_m",
        "reasoning_price_per1_m",
    ] {
        let raw = model[field]
            .as_str()
            .unwrap_or_else(|| panic!("{field} must be a decimal string: {model}"));
        assert_eq!(raw.len(), raw.find('.').expect("decimal point") + 9, "{field}");
    }
    assert_eq!(model["input_price_per1_m"].as_str(), Some("0.00015000"));
    assert_eq!(model["output_price_per1_m"].as_str(), Some("0.00060000"));
}
