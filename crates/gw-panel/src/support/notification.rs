//! 站内通知。
//!
//! 对应通知相关的 `opsStore` + 四个 `UserNotification*Handler`。
//!
//! # 这是本域唯一不落库的东西，而且是**有意**不落库的
//!
//! 原实现的注释写得很直白：通知是"唯一还没进库的运营态数据"，而且它的已读状态是
//! **进程全局**的，不是按用户分的 —— 一个人点了已读，所有人都变成已读。原实现明确
//! 选择「先不把这个坏味道持久化」，等重新设计。
//!
//! 这里最容易做的两件错事：
//!
//! * **顺手落库**。那会把一个全局已读状态固化进 schema，从此更难改对。
//! * **顺手改成按用户**。那是新功能：前端会突然看到与原实现不同的未读数，而
//!   重写的验收标准是"行为一致"。
//!
//! 所以这里如实复刻：一个进程内的列表 + 一条种子通知，重启即回到初始状态。
//! 真正的修法（每用户已读、落库）是重写之后的独立工作。

use axum::extract::{Path, Query, State};
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard};

use crate::identity::{bad_request, not_found};
use crate::paging::{ListPage, offset, page_params, parse_id};
use crate::{AuthUser, PanelState, ok};

#[cfg(test)]
mod tests;

/// 通知列表的默认页大小。对应 `queryInt(c, "page_size", 10, 1, 100)`。
const NOTIFICATIONS_DEFAULT_PAGE_SIZE: i64 = 10;

/// 对应 `notificationItem`。
#[derive(Debug, Clone, Serialize)]
pub struct NotificationItem {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub is_read: bool,
    pub notification_type: String,
    pub created_at: DateTime<Utc>,
}

/// 进程内的通知表。对应原实现的 `var opsStore struct{ mu; notifications }`。
static NOTIFICATIONS: LazyLock<Mutex<Vec<NotificationItem>>> = LazyLock::new(|| {
    Mutex::new(vec![NotificationItem {
        id: 1,
        title: "欢迎使用 AI-GateWay".to_owned(),
        content: "账户面板、API Key、计费和代理转发功能已就绪。".to_owned(),
        is_read: false,
        notification_type: "system".to_owned(),
        created_at: Utc::now(),
    }])
});

/// 拿锁。持有者 panic 过不该让通知接口从此 500 —— 原实现的 `sync.Mutex` 没有中毒
/// 概念，这里取回内层数据继续用，行为一致。
fn notifications() -> MutexGuard<'static, Vec<NotificationItem>> {
    NOTIFICATIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `GET /user/notifications/unread-count` —— 未读数（进程全局）。
pub async fn unread_count(State(_state): State<PanelState>, _user: AuthUser) -> Response {
    let count = notifications().iter().filter(|n| !n.is_read).count();
    ok(serde_json::json!({ "unread_count": count }))
}

/// `GET /user/notifications` —— 分页列表，最新在前。
///
/// 对应 `UserNotificationsHandler`。切页在内存里做，越界的 `page` 得到一个空
/// 数组而不是错误（原实现把 start/end 夹到长度内）。
pub async fn list(
    State(_state): State<PanelState>,
    _user: AuthUser,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let (page, page_size) = page_params(
        params.get("page").map(String::as_str),
        params.get("page_size").map(String::as_str),
        NOTIFICATIONS_DEFAULT_PAGE_SIZE,
    );

    let mut items = notifications().clone();
    // 最新在前 —— 取负号排序键而不是写比较闭包（clippy::unnecessary_sort_by）。
    items.sort_by_key(|n| std::cmp::Reverse(n.created_at));
    let total = i64::try_from(items.len()).unwrap_or(i64::MAX);
    let page_items = slice_page(&items, page, page_size).to_vec();

    ok(ListPage::new(page_items, total, page, page_size))
}

/// 按 `(page, page_size)` 切一页，越界一律夹到长度内。
///
/// 抽成纯函数是为了能直接测边界：原实现那段 `if start > len { start = len }` 写错
/// 一处就会 panic（切片越界），而它恰好是最少被点到的分页路径。
#[must_use]
pub fn slice_page<T>(items: &[T], page: i64, page_size: i64) -> &[T] {
    let len = items.len();
    let start = usize::try_from(offset(page, page_size))
        .unwrap_or(usize::MAX)
        .min(len);
    let end = usize::try_from(page_size)
        .ok()
        .and_then(|size| start.checked_add(size))
        .unwrap_or(len)
        .min(len);
    &items[start..end]
}

/// `PUT /user/notifications/{id}/read` —— 标记一条已读。
pub async fn mark_read(
    State(_state): State<PanelState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的通知 ID");
    };
    let mut guard = notifications();
    match guard.iter_mut().find(|n| n.id == id) {
        Some(item) => {
            item.is_read = true;
            ok(serde_json::json!({ "ok": true }))
        }
        None => not_found("未找到该通知"),
    }
}

/// `PUT /user/notifications/read-all` —— 全部标记已读。
pub async fn mark_all_read(State(_state): State<PanelState>, _user: AuthUser) -> Response {
    for item in notifications().iter_mut() {
        item.is_read = true;
    }
    ok(serde_json::json!({ "ok": true }))
}
