//! 分组：用户可绑定的清单 + 管理员的套餐 CRUD。
//!
//! 对应既有实现的 `handler_user` 的 `AvailableGroupsHandler` + `handler_admin_expanded`
//! 的 `AdminGroups*Handler` / `saveAdminGroup` / `groupPayload`。
//!
//! # 两个「分组」不是一张表
//!
//! 这是本域最容易踩的坑，旧实现里也一样：
//!
//! * `GET /user/available-groups` 读的是 **`groups`**（`id` / `name` /
//!   `rate_multiplier`）—— 它是 API Key 真正绑定的那张表；
//! * `GET|POST|PUT|DELETE /admin/groups` 读写的是 **`subscription_packages`** ——
//!   运营编辑的是「套餐」，`groups` 行由别处维护。
//!
//! 两者靠 `subscription_packages.group_id` 关联。名字撞车是历史遗留，别在移植时
//! "顺手统一"，那会让管理员页面开始编辑鉴权用的分组表。

use axum::extract::{Path, State};
use axum::response::Response;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::{bad_request, db_failure, not_found, parse_json_body};
use crate::paging::parse_id;
use crate::{AdminUser, AuthUser, PanelState, ok};

#[cfg(test)]
mod tests;

/// 旧实现在 `availableGroupItem` 里写死的两个常量。用户可绑分组没有套餐概念，
/// 所以类型恒为 `standard`、有效期恒为 30 —— 前端拿它做展示，不是业务事实。
const AVAILABLE_SUBSCRIPTION_TYPE: &str = "standard";
const AVAILABLE_DEFAULT_VALIDITY_DAYS: i64 = 30;

/// 管理员套餐视图里写死的类型标签。
const PACKAGE_SUBSCRIPTION_TYPE: &str = "subscription";

/// `rate_multiplier` 缺省/非法时兜底成 1（不打折）。对应 `if pkg.RateMultiplier <= 0 { = 1 }`。
const DEFAULT_RATE_MULTIPLIER: f64 = 1.0;

/// `default_validity_days` 缺省/非法时兜底成 30 天。
const DEFAULT_VALIDITY_DAYS: i64 = 30;

/// 对应 `availableGroupItem`。
///
/// 三个 `*_limit_usd` **没有** `omitempty`：它们恒为 `null`，键必须在。
#[derive(Debug, Serialize)]
pub struct AvailableGroup {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub subscription_type: String,
    pub rate_multiplier: f64,
    pub daily_limit_usd: Option<f64>,
    pub weekly_limit_usd: Option<f64>,
    pub monthly_limit_usd: Option<f64>,
    pub default_validity_days: i64,
}

/// 对应 `groupPayload`（`subscription_packages` 的一行）。
#[derive(Debug, Serialize)]
pub struct PackagePayload {
    pub id: i64,
    pub name: String,
    pub subscription_type: String,
    pub rate_multiplier: f64,
    pub daily_limit_usd: Option<f64>,
    pub weekly_limit_usd: Option<f64>,
    pub monthly_limit_usd: Option<f64>,
    pub default_validity_days: i64,
    pub subscription_price_usd: f64,
}

/// 旧实现 `saveAdminGroup` 的请求体。
///
/// **三个 `*_limit_usd` 的键名是有意写成旧实现字段名原样的**，不是笔误：
/// 旧实现那三个字段没有 json tag，`encoding/json` 只做**字段名的大小写不敏感**匹配，
/// 不做 snake_case 转换，所以前端发的 `daily_limit_usd` 在旧实现侧**根本绑不上**，
/// 三个额度一直是 nil。这里逐字复刻这个行为（连同 alias 覆盖大小写变体），
/// 免得重写悄悄改变配额语义；这属于已知的旧实现缺陷，修不修由上游决定。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SavePackageRequest {
    name: String,
    rate_multiplier: f64,
    #[serde(rename = "DailyLimitUSD", alias = "dailylimitusd")]
    daily_limit_usd: Option<f64>,
    #[serde(rename = "WeeklyLimitUSD", alias = "weeklylimitusd")]
    weekly_limit_usd: Option<f64>,
    #[serde(rename = "MonthlyLimitUSD", alias = "monthlylimitusd")]
    monthly_limit_usd: Option<f64>,
    default_validity_days: i64,
    subscription_price_usd: f64,
}

#[derive(Debug, sqlx::FromRow)]
struct PackageRow {
    id: i64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    name: String,
    #[sqlx(try_from = "gw_model::compat::Money")]
    rate_multiplier: f64,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    daily_limit_usd: Option<f64>,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    weekly_limit_usd: Option<f64>,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    monthly_limit_usd: Option<f64>,
    #[sqlx(try_from = "gw_model::compat::Int")]
    default_validity_days: i64,
    #[sqlx(try_from = "gw_model::compat::Money")]
    subscription_price_usd: f64,
}

impl From<PackageRow> for PackagePayload {
    fn from(row: PackageRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            subscription_type: PACKAGE_SUBSCRIPTION_TYPE.to_owned(),
            rate_multiplier: row.rate_multiplier,
            daily_limit_usd: row.daily_limit_usd,
            weekly_limit_usd: row.weekly_limit_usd,
            monthly_limit_usd: row.monthly_limit_usd,
            default_validity_days: row.default_validity_days,
            subscription_price_usd: row.subscription_price_usd,
        }
    }
}

const PACKAGE_COLUMNS: &str = "id, name, rate_multiplier, daily_limit_usd, weekly_limit_usd, \
     monthly_limit_usd, default_validity_days, subscription_price_usd";

// ── handlers ─────────────────────────────────────────────────────────────────

/// `GET /user/available-groups` —— 调用者当前能绑的分组。
///
/// Ports `AvailableGroupsHandler`（Requirement 3.3）。非管理员只看得到
/// **基线分组 + 自己有未过期订阅的分组**；管理员看全量，运营界面还要列出每一档。
///
/// 响应是**裸数组**，不是分页信封。
pub async fn available(State(state): State<PanelState>, user: AuthUser) -> Response {
    let rows: Result<Vec<(i64, String, f64)>, _> = if user.is_admin() {
        sqlx::query_as(
            "SELECT id, COALESCE(name,''), COALESCE(rate_multiplier,0)::float8 \
             FROM groups ORDER BY id ASC",
        )
        .fetch_all(&state.pg)
        .await
    } else {
        sqlx::query_as(
            "SELECT id, COALESCE(name,''), COALESCE(rate_multiplier,0)::float8 FROM groups \
             WHERE rate_multiplier = 1.0 OR id IN (\
                 SELECT group_id FROM subscriptions \
                 WHERE user_id = $1 AND status = 'active' AND expires_at > $2\
             ) \
             ORDER BY id ASC",
        )
        .bind(user.user_id)
        .bind(Utc::now())
        .fetch_all(&state.pg)
        .await
    };

    match rows {
        Ok(rows) => ok(rows
            .into_iter()
            .map(|(id, name, rate_multiplier)| AvailableGroup {
                id,
                name,
                description: String::new(),
                subscription_type: AVAILABLE_SUBSCRIPTION_TYPE.to_owned(),
                rate_multiplier,
                daily_limit_usd: None,
                weekly_limit_usd: None,
                monthly_limit_usd: None,
                default_validity_days: AVAILABLE_DEFAULT_VALIDITY_DAYS,
            })
            .collect::<Vec<_>>()),
        Err(error) => db_failure("available_groups", &error, "获取分组失败，请稍后重试"),
    }
}

/// `GET /admin/groups` —— 全部套餐，裸数组。Ports `AdminGroupsListHandler`。
pub async fn admin_list(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    let rows: Result<Vec<PackageRow>, _> = sqlx::query_as(&format!(
        "SELECT {PACKAGE_COLUMNS} FROM subscription_packages ORDER BY id ASC"
    ))
    .fetch_all(&state.pg)
    .await;
    match rows {
        Ok(rows) => ok(rows
            .into_iter()
            .map(PackagePayload::from)
            .collect::<Vec<_>>()),
        Err(error) => db_failure("admin_list_groups", &error, "获取分组失败，请稍后重试"),
    }
}

/// `POST /admin/groups` —— 新建套餐。Ports `AdminGroupsCreateHandler` + `saveAdminGroup(id=0)`。
pub async fn admin_create(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: axum::body::Bytes,
) -> Response {
    save_package(&state, None, &body).await
}

/// `PUT /admin/groups/{id}` —— 改套餐。Ports `AdminGroupsUpdateHandler`。
pub async fn admin_update(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的分组 ID");
    };
    save_package(&state, Some(id), &body).await
}

/// `DELETE /admin/groups/{id}`。Ports `AdminGroupsDeleteHandler`。
///
/// 注意旧实现这里**不检查影响行数** —— 删一个不存在的 id 也回 `{"deleted": true}`。
/// 照抄，别顺手加 404，前端把非 200 当失败弹窗。
pub async fn admin_delete(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的分组 ID");
    };
    match sqlx::query("DELETE FROM subscription_packages WHERE id = $1")
        .bind(id)
        .execute(&state.pg)
        .await
    {
        Ok(_) => ok(serde_json::json!({ "deleted": true })),
        Err(error) => db_failure("admin_delete_group", &error, "删除分组失败，请稍后重试"),
    }
}

/// 新建/更新共用的落库逻辑。Ports `saveAdminGroup`。
async fn save_package(state: &PanelState, id: Option<i64>, body: &[u8]) -> Response {
    let req: SavePackageRequest = match parse_json_body(body, "分组信息格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    let name = req.name.trim();
    if name.is_empty() {
        return bad_request("分组信息格式无效");
    }

    let rate_multiplier = if req.rate_multiplier <= 0.0 {
        DEFAULT_RATE_MULTIPLIER
    } else {
        req.rate_multiplier
    };
    let validity_days = if req.default_validity_days <= 0 {
        DEFAULT_VALIDITY_DAYS
    } else {
        req.default_validity_days
    };
    let now = Utc::now();

    let saved: Result<Option<PackageRow>, sqlx::Error> = match id {
        Some(id) => {
            sqlx::query_as(&format!(
                "UPDATE subscription_packages SET name = $2, rate_multiplier = $3, \
                     default_validity_days = $4, daily_limit_usd = $5, weekly_limit_usd = $6, \
                     monthly_limit_usd = $7, subscription_price_usd = $8, updated_at = $9 \
                 WHERE id = $1 RETURNING {PACKAGE_COLUMNS}"
            ))
            .bind(id)
            .bind(name)
            .bind(rate_multiplier)
            .bind(validity_days)
            .bind(req.daily_limit_usd)
            .bind(req.weekly_limit_usd)
            .bind(req.monthly_limit_usd)
            .bind(req.subscription_price_usd)
            .bind(now)
            .fetch_optional(&state.pg)
            .await
        }
        None => {
            // 对应 `SELECT COALESCE(MAX(group_id),0)` 然后 +1。新套餐默认启用。
            sqlx::query_as(&format!(
                "INSERT INTO subscription_packages \
                     (name, description, group_id, rate_multiplier, default_validity_days, \
                      daily_limit_usd, weekly_limit_usd, monthly_limit_usd, \
                      subscription_price_usd, enabled, created_at, updated_at) \
                 VALUES ($1, '', \
                     (SELECT COALESCE(MAX(group_id), 0) + 1 FROM subscription_packages), \
                     $2, $3, $4, $5, $6, $7, TRUE, $8, $8) \
                 RETURNING {PACKAGE_COLUMNS}"
            ))
            .bind(name)
            .bind(rate_multiplier)
            .bind(validity_days)
            .bind(req.daily_limit_usd)
            .bind(req.weekly_limit_usd)
            .bind(req.monthly_limit_usd)
            .bind(req.subscription_price_usd)
            .bind(now)
            .fetch_optional(&state.pg)
            .await
        }
    };

    match saved {
        // UPDATE 没命中任何行 = 这个套餐不存在。旧实现在更新前先 First 了一次，
        // 读不到就 404「未找到该分组」，这里靠 RETURNING 的空结果得到同一个答案。
        Ok(None) => not_found("未找到该分组"),
        Ok(Some(row)) => ok(PackagePayload::from(row)),
        Err(error) => db_failure("save_group", &error, "保存分组失败，请稍后重试"),
    }
}
