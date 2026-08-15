//! 工单：用户提单/追问、管理员回复/分派/状态流转、图片上传、快捷回复。
//!
//! 对应 ticket handlers（`User*TicketHandler` / `Admin*TicketHandler`）。
//!
//! # 状态流转是回复的副作用，不是一个独立端点
//!
//! 原实现把它藏在 `createTicketReply` 里，只有两条规则：
//!
//! * 管理员回复且当前是 `open` → `answered`（"我们答了，等你"）；
//! * 用户回复且当前是 `answered` → `open`（"我又问了，等你们"）。
//!
//! 其余状态（`closed` 等）**不因回复而改变** —— 关掉的单被追问不会自己弹回来，
//! 那是 `PUT /admin/tickets/{id}/status` 的事。
//!
//! # 图片是 data URI，不是文件
//!
//! 上传接口不落盘、不进对象存储：把图片 base64 进一个 `data:` URI 直接回给前端，
//! 由它塞进 Markdown。好处是没有静态资源生命周期要管，代价是 4 MiB 的硬上限和
//! 撑大的工单正文 —— 原实现就是这么做的，这里保持原样。

use axum::extract::{Multipart, Path, Query, State};
use axum::response::Response;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::identity::{
    bad_request, db_failure, first_non_empty, internal, not_found, parse_json_body,
};
use crate::paging::{ListPage, offset, page_params, parse_id};
use crate::{AdminUser, AuthUser, PanelState, ok};

#[cfg(test)]
mod tests;

/// 工单图片的硬上限。对应 `maxTicketImageBytes`（4 MiB）。
///
/// 图片会被 base64 进 data URI（膨胀 ~33%）再存进回复正文，所以这个上限同时是
/// **数据库行大小**的上限，不只是上传大小。
pub const MAX_IMAGE_BYTES: usize = 4 << 20;

/// 列表默认页大小。对应 `queryInt(c, "page_size", 20, 1, 100)`（用户与管理员同）。
const TICKETS_DEFAULT_PAGE_SIZE: i64 = 20;

const STATUS_OPEN: &str = "open";
const STATUS_ANSWERED: &str = "answered";

/// 新工单缺省的标题/分类/优先级。对应 `firstNonEmpty(req.X, "…")` 里的兜底值。
const DEFAULT_TITLE: &str = "在线咨询";
const DEFAULT_CATEGORY: &str = "other";
const DEFAULT_PRIORITY: &str = "medium";

/// 对应 `ticketPayload`。
///
/// `content` 是**列表里恒为空串**的展示字段：工单正文其实住在第一条回复里，
/// 只有刚创建那次响应会把它填上（传的就是刚提交的正文）。
#[derive(Debug, Serialize)]
pub struct TicketPayload {
    pub id: i64,
    pub user_id: i64,
    pub title: String,
    pub category: String,
    pub priority: String,
    pub status: String,
    pub assignee_id: Option<i64>,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 工单详情 = [`TicketPayload`] 再加一条 `replies`。
#[derive(Debug, Serialize)]
pub struct TicketDetail {
    #[serde(flatten)]
    pub ticket: TicketPayload,
    pub replies: Vec<ReplyPayload>,
}

/// 对应 `replyPayload`。
#[derive(Debug, Serialize)]
pub struct ReplyPayload {
    pub id: i64,
    pub ticket_id: i64,
    pub user_id: i64,
    pub is_admin: bool,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct TicketRow {
    id: i64,
    user_id: i64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    title: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    category: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    priority: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    status: String,
    assignee_id: Option<i64>,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    created_at: DateTime<Utc>,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    updated_at: DateTime<Utc>,
}

const TICKET_COLUMNS: &str =
    "id, user_id, title, category, priority, status, assignee_id, created_at, updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
struct ReplyRow {
    id: i64,
    ticket_id: i64,
    user_id: i64,
    #[sqlx(try_from = "gw_model::compat::Bool")]
    is_admin: bool,
    #[sqlx(try_from = "gw_model::compat::Text")]
    content: String,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    created_at: DateTime<Utc>,
}

const REPLY_COLUMNS: &str = "id, ticket_id, user_id, is_admin, content, created_at";

impl TicketRow {
    fn into_payload(self, content: &str) -> TicketPayload {
        TicketPayload {
            id: self.id,
            user_id: self.user_id,
            title: self.title,
            category: self.category,
            priority: self.priority,
            status: self.status,
            assignee_id: self.assignee_id,
            content: content.to_owned(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

impl From<ReplyRow> for ReplyPayload {
    fn from(row: ReplyRow) -> Self {
        Self {
            id: row.id,
            ticket_id: row.ticket_id,
            user_id: row.user_id,
            is_admin: row.is_admin,
            content: row.content,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CreateTicketRequest {
    title: String,
    category: String,
    priority: String,
    content: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ReplyRequest {
    content: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StatusRequest {
    status: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AssignRequest {
    assignee_id: Option<i64>,
}

/// 一条回复应当把工单推到哪个状态。对应 `createTicketReply` 里的那两个 if。
///
/// 抽成纯函数是因为它是这个模块唯一有分支的业务规则，而它的两条边（管理员回复
/// 已关闭的单、用户回复已关闭的单）都必须**保持原状**。
#[must_use]
pub fn status_after_reply(current: &str, is_admin: bool) -> String {
    match (is_admin, current) {
        (true, STATUS_OPEN) => STATUS_ANSWERED.to_owned(),
        (false, STATUS_ANSWERED) => STATUS_OPEN.to_owned(),
        _ => current.to_owned(),
    }
}

/// 对应 `html.EscapeString`。
///
/// 用在图片的 Markdown alt 文本上：文件名是用户输入，会被原样拼进
/// `![alt](data:…)`。不转义的话一个叫 `x")![](javascript:…` 的文件名就能在渲染
/// 工单的页面里注入标记。**五个字符全都要转，且 `&` 必须第一个转**，否则会把
/// 自己生成的实体再转一遍。
#[must_use]
pub fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '\'' => out.push_str("&#39;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&#34;"),
            other => out.push(other),
        }
    }
    out
}

// ── 用户侧 ───────────────────────────────────────────────────────────────────

/// `GET /user/tickets` —— 我的工单，分页 + 状态过滤。
///
/// 对应 `UserListTicketsHandler`。`status=all` 与 `status=`（缺省）等价，
/// 都表示不过滤。
pub async fn list_own(
    State(state): State<PanelState>,
    user: AuthUser,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    list_tickets(&state, Some(user.user_id), &params).await
}

/// `POST /user/tickets` —— 建单。
///
/// 对应 `UserCreateTicketHandler`。工单行与它的**第一条回复**（即正文）在同一个
/// 事务里写入：分开写会出现一个没有正文的空工单。
pub async fn create_own(
    State(state): State<PanelState>,
    user: AuthUser,
    body: axum::body::Bytes,
) -> Response {
    let req: CreateTicketRequest = match parse_json_body(&body, "工单内容无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    let content = req.content.trim();
    if content.is_empty() {
        return bad_request("工单内容无效");
    }

    let title = first_non_empty(&[&req.title, DEFAULT_TITLE]).to_owned();
    let category = first_non_empty(&[&req.category, DEFAULT_CATEGORY]).to_owned();
    let priority = first_non_empty(&[&req.priority, DEFAULT_PRIORITY]).to_owned();

    match create_ticket_tx(&state, user.user_id, &title, &category, &priority, content).await {
        Ok(row) => ok(row.into_payload(content)),
        Err(error) => db_failure("create_ticket", &error, "工单创建失败，请稍后重试"),
    }
}

/// `GET /user/tickets/{id}` —— 详情（含全部回复）。
///
/// 对应 `UserGetTicketHandler` + `loadUserTicket`：别人的工单返回 **404**，
/// 与"不存在"不可区分。
pub async fn get_own(
    State(state): State<PanelState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Response {
    let ticket = match load_ticket(&state, &id, Some(user.user_id)).await {
        Ok(ticket) => ticket,
        Err(response) => return response,
    };
    detail_response(&state, ticket).await
}

/// `POST /user/tickets/{id}/replies` —— 用户追问。
pub async fn reply_own(
    State(state): State<PanelState>,
    user: AuthUser,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let ticket = match load_ticket(&state, &id, Some(user.user_id)).await {
        Ok(ticket) => ticket,
        Err(response) => return response,
    };
    create_reply(&state, &ticket, user.user_id, false, &body).await
}

/// `POST /user/ticket-images` —— 上传一张图，得到 data URI 与现成的 Markdown。
///
/// 对应 `UserUploadTicketImageHandler`。三道闸门缺一不可：字段必须叫 `image`、
/// `Content-Type` 必须是 `image/*`、大小不超过 [`MAX_IMAGE_BYTES`]。
pub async fn upload_image(_user: AuthUser, mut multipart: Multipart) -> Response {
    let field = loop {
        match multipart.next_field().await {
            Ok(Some(field)) if field.name() == Some("image") => break field,
            // 忽略其他字段，继续找 —— 前端可能还带了别的表单项。
            Ok(Some(_)) => {}
            Ok(None) => return bad_request("请上传图片"),
            Err(_) => return bad_request("请上传图片"),
        }
    };

    let content_type = field.content_type().unwrap_or_default().to_owned();
    let filename = field.file_name().unwrap_or_default().to_owned();
    if !content_type.starts_with("image/") {
        return bad_request("上传文件必须是图片");
    }

    let Ok(data) = field.bytes().await else {
        return internal("读取图片失败，请稍后重试");
    };
    if data.len() > MAX_IMAGE_BYTES {
        return bad_request("图片大小不能超过 4MB");
    }

    let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
    let url = format!("data:{content_type};base64,{encoded}");
    let alt = escape_html(&filename);
    ok(serde_json::json!({
        "url": url,
        "markdown": format!("![{alt}]({url})"),
    }))
}

// ── 管理员侧 ─────────────────────────────────────────────────────────────────

/// `GET /admin/tickets` —— 全量工单，分页 + 状态过滤。
pub async fn admin_list(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    list_tickets(&state, None, &params).await
}

/// `GET /admin/tickets/{id}` —— 任意工单的详情。
pub async fn admin_get(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(id): Path<String>,
) -> Response {
    let ticket = match load_ticket(&state, &id, None).await {
        Ok(ticket) => ticket,
        Err(response) => return response,
    };
    detail_response(&state, ticket).await
}

/// `POST /admin/tickets/{id}/replies` —— 客服回复。
pub async fn admin_reply(
    State(state): State<PanelState>,
    admin: AdminUser,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let ticket = match load_ticket(&state, &id, None).await {
        Ok(ticket) => ticket,
        Err(response) => return response,
    };
    create_reply(&state, &ticket, admin.0.user_id, true, &body).await
}

/// `PUT /admin/tickets/{id}/status` —— 显式改状态。
///
/// 对应 `AdminUpdateTicketStatusHandler`。**不校验状态取值** —— 原实现只要求非空，
/// 什么字符串都收。这里照旧，前端的下拉框是唯一的约束。
pub async fn admin_set_status(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let ticket = match load_ticket(&state, &id, None).await {
        Ok(ticket) => ticket,
        Err(response) => return response,
    };
    let req: StatusRequest = match parse_json_body(&body, "状态无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    let status = req.status.trim();
    if status.is_empty() {
        return bad_request("状态无效");
    }

    let updated: Result<Option<TicketRow>, _> = sqlx::query_as(&format!(
        "UPDATE tickets SET status = $2, updated_at = $3 WHERE id = $1 RETURNING {TICKET_COLUMNS}"
    ))
    .bind(ticket.id)
    .bind(status)
    .bind(Utc::now())
    .fetch_optional(&state.pg)
    .await;

    match updated {
        Ok(Some(row)) => ok(row.into_payload("")),
        Ok(None) => not_found("未找到该工单"),
        Err(error) => db_failure("update_ticket_status", &error, "更新工单失败，请稍后重试"),
    }
}

/// `PUT /admin/tickets/{id}/assign` —— 分派。
///
/// 对应 `AdminAssignTicketHandler`。请求体可以整个省掉：**不带 `assignee_id`
/// 就是把单派给自己**（原实现 `_ = c.ShouldBindJSON(&req)` 后 `req.AssigneeID == nil`
/// 走的就是这条），所以这里的解析失败也不能报错。
pub async fn admin_assign(
    State(state): State<PanelState>,
    admin: AdminUser,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let ticket = match load_ticket(&state, &id, None).await {
        Ok(ticket) => ticket,
        Err(response) => return response,
    };
    // 解析失败被吞掉：原实现忽略了 ShouldBindJSON 的错误，空 body 是合法的"派给我"。
    let req: AssignRequest = parse_json_body(&body, "").unwrap_or_default();
    let assignee = req.assignee_id.unwrap_or(admin.0.user_id);

    let updated: Result<Option<TicketRow>, _> = sqlx::query_as(&format!(
        "UPDATE tickets SET assignee_id = $2, updated_at = $3 WHERE id = $1 \
         RETURNING {TICKET_COLUMNS}"
    ))
    .bind(ticket.id)
    .bind(assignee)
    .bind(Utc::now())
    .fetch_optional(&state.pg)
    .await;

    match updated {
        Ok(Some(row)) => ok(row.into_payload("")),
        Ok(None) => not_found("未找到该工单"),
        Err(error) => db_failure("assign_ticket", &error, "工单指派失败，请稍后重试"),
    }
}

/// `GET /admin/ticket-quick-replies` —— 快捷回复模板。
///
/// 对应 `AdminTicketQuickRepliesGetHandler`：原实现回的是一条**写死的**模板，
/// 没有存储。这里保持原样（把它做成一张表是新功能，不是既有行为）。
pub async fn admin_quick_replies(_admin: AdminUser) -> Response {
    ok(serde_json::json!({
        "items": [{
            "id": 1,
            "title": "收到",
            "content": "您好，我们已收到您的反馈，将尽快处理。",
        }],
    }))
}

/// `POST /admin/ticket-quick-replies` —— 保存模板。
///
/// 对应 `AdminTicketQuickRepliesSaveHandler`：原实现直接回 `{"ok": true}` 而**不落库**
/// （与上面的写死模板是一对）。照旧，别偷偷加一张表 —— 那会让前端以为保存成功了
/// 却在下次 GET 时看不到。
pub async fn admin_save_quick_replies(_admin: AdminUser) -> Response {
    ok(serde_json::json!({ "ok": true }))
}

// ── 内部 ─────────────────────────────────────────────────────────────────────

async fn list_tickets(
    state: &PanelState,
    owner: Option<i64>,
    params: &HashMap<String, String>,
) -> Response {
    let (page, page_size) = page_params(
        params.get("page").map(String::as_str),
        params.get("page_size").map(String::as_str),
        TICKETS_DEFAULT_PAGE_SIZE,
    );
    // 原实现把 "all" 和空串都当成"不过滤"。
    let status = params
        .get("status")
        .map(|s| s.trim())
        .filter(|s| *s != "all")
        .unwrap_or_default();
    // owner 为 None（管理员视图）时用 0 表示"不限用户"。
    let owner = owner.unwrap_or(0);
    let filter = "($1 = 0 OR user_id = $1) AND ($2 = '' OR status = $2)";

    let total: Result<i64, _> = sqlx::query_scalar(&format!(
        "SELECT COUNT(*)::bigint FROM tickets WHERE {filter}"
    ))
    .bind(owner)
    .bind(status)
    .fetch_one(&state.pg)
    .await;
    let total = match total {
        Ok(total) => total,
        Err(error) => return db_failure("count_tickets", &error, "查询工单失败，请稍后重试"),
    };

    let rows: Result<Vec<TicketRow>, _> = sqlx::query_as(&format!(
        "SELECT {TICKET_COLUMNS} FROM tickets WHERE {filter} \
         ORDER BY updated_at DESC, id DESC LIMIT $3 OFFSET $4"
    ))
    .bind(owner)
    .bind(status)
    .bind(page_size)
    .bind(offset(page, page_size))
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(ListPage::new(
            // 列表里 content 恒为空串（正文在第一条回复里）。
            rows.into_iter().map(|r| r.into_payload("")).collect(),
            total,
            page,
            page_size,
        )),
        Err(error) => db_failure("list_tickets", &error, "获取工单列表失败，请稍后重试"),
    }
}

/// 载入一张工单；`owner` 为 `Some` 时同时要求它属于该用户。
///
/// 错误路径直接返回构造好的响应，调用方 `match` 一下即可。
async fn load_ticket(
    state: &PanelState,
    raw_id: &str,
    owner: Option<i64>,
) -> Result<TicketRow, Response> {
    let Some(id) = parse_id(raw_id) else {
        return Err(bad_request("无效的工单 ID"));
    };
    let row: Result<Option<TicketRow>, _> = sqlx::query_as(&format!(
        "SELECT {TICKET_COLUMNS} FROM tickets WHERE id = $1 LIMIT 1"
    ))
    .bind(id)
    .fetch_optional(&state.pg)
    .await;

    match row {
        Ok(Some(row)) => {
            // 越权访问与"不存在"给同一个 404，不泄露工单是否存在。
            if owner.is_some_and(|owner| owner != row.user_id) {
                return Err(not_found("未找到该工单"));
            }
            Ok(row)
        }
        Ok(None) => Err(not_found("未找到该工单")),
        Err(error) => Err(db_failure(
            "load_ticket",
            &error,
            "加载工单失败，请稍后重试",
        )),
    }
}

async fn detail_response(state: &PanelState, ticket: TicketRow) -> Response {
    // 原实现忽略这次查询的错误（`_ = ...Find(&replies)`），读不到就给一个空数组 ——
    // 详情页至少还能显示工单本身。
    let replies: Vec<ReplyRow> = sqlx::query_as(&format!(
        "SELECT {REPLY_COLUMNS} FROM ticket_replies WHERE ticket_id = $1 \
         ORDER BY created_at ASC, id ASC"
    ))
    .bind(ticket.id)
    .fetch_all(&state.pg)
    .await
    .unwrap_or_default();

    ok(TicketDetail {
        ticket: ticket.into_payload(""),
        replies: replies.into_iter().map(ReplyPayload::from).collect(),
    })
}

async fn create_reply(
    state: &PanelState,
    ticket: &TicketRow,
    user_id: i64,
    is_admin: bool,
    body: &[u8],
) -> Response {
    let req: ReplyRequest = match parse_json_body(body, "回复内容无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    let content = req.content.trim();
    if content.is_empty() {
        return bad_request("回复内容无效");
    }

    let status = status_after_reply(&ticket.status, is_admin);
    match create_reply_tx(state, ticket.id, user_id, is_admin, content, &status).await {
        Ok(row) => ok(ReplyPayload::from(row)),
        Err(error) => db_failure("create_reply", &error, "回复创建失败，请稍后重试"),
    }
}

/// 工单 + 第一条回复，一个事务。
async fn create_ticket_tx(
    state: &PanelState,
    user_id: i64,
    title: &str,
    category: &str,
    priority: &str,
    content: &str,
) -> Result<TicketRow, sqlx::Error> {
    let now = Utc::now();
    let mut tx = state.pg.begin().await?;

    let ticket: TicketRow = sqlx::query_as(&format!(
        "INSERT INTO tickets \
             (user_id, title, category, priority, status, assignee_id, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, NULL, $6, $6) RETURNING {TICKET_COLUMNS}"
    ))
    .bind(user_id)
    .bind(title)
    .bind(category)
    .bind(priority)
    .bind(STATUS_OPEN)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO ticket_replies (ticket_id, user_id, is_admin, content, created_at) \
         VALUES ($1, $2, FALSE, $3, $4)",
    )
    .bind(ticket.id)
    .bind(user_id)
    .bind(content)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(ticket)
}

/// 回复 + 状态流转，一个事务。
async fn create_reply_tx(
    state: &PanelState,
    ticket_id: i64,
    user_id: i64,
    is_admin: bool,
    content: &str,
    status: &str,
) -> Result<ReplyRow, sqlx::Error> {
    let now = Utc::now();
    let mut tx = state.pg.begin().await?;

    let reply: ReplyRow = sqlx::query_as(&format!(
        "INSERT INTO ticket_replies (ticket_id, user_id, is_admin, content, created_at) \
         VALUES ($1, $2, $3, $4, $5) RETURNING {REPLY_COLUMNS}"
    ))
    .bind(ticket_id)
    .bind(user_id)
    .bind(is_admin)
    .bind(content)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query("UPDATE tickets SET status = $2, updated_at = $3 WHERE id = $1")
        .bind(ticket_id)
        .bind(status)
        .bind(now)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(reply)
}
