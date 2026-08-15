//! 客服域：工单与回复、工单图片、公告、站内通知。
//!
//! Owner: worker `panel-identity`。工单与回复、工单图片、公告、站内通知。
//!
//! # 用户与管理员两侧住在一起
//!
//! 工单在这里有六条用户路由和五条管理员路由，读写的是同两张表；公告有一条用户
//! 只读路由和四条管理员 CRUD。按角色劈开会让"工单"这个功能横跨两个目录
//! （规则 1.6）。

pub mod announcement;
pub mod notification;
pub mod ticket;

use axum::Router;
use axum::routing::{get, post, put};

use crate::PanelState;

/// 客服域的路由表（挂在 `/api/panel` 下）。
pub fn router() -> Router<PanelState> {
    Router::new()
        // ── 工单 ──
        .route(
            "/user/tickets",
            get(ticket::list_own).post(ticket::create_own),
        )
        .route("/user/tickets/{id}", get(ticket::get_own))
        .route("/user/tickets/{id}/replies", post(ticket::reply_own))
        .route("/user/ticket-images", post(ticket::upload_image))
        .route("/admin/tickets", get(ticket::admin_list))
        .route("/admin/tickets/{id}", get(ticket::admin_get))
        .route("/admin/tickets/{id}/replies", post(ticket::admin_reply))
        .route("/admin/tickets/{id}/status", put(ticket::admin_set_status))
        .route("/admin/tickets/{id}/assign", put(ticket::admin_assign))
        .route(
            "/admin/ticket-quick-replies",
            get(ticket::admin_quick_replies).post(ticket::admin_save_quick_replies),
        )
        // ── 公告 ──
        .route("/user/announcements", get(announcement::list_active))
        .route(
            "/admin/announcements",
            get(announcement::admin_list).post(announcement::admin_create),
        )
        .route(
            "/admin/announcements/{id}",
            put(announcement::admin_update).delete(announcement::admin_delete),
        )
        // ── 站内通知 ──
        .route(
            "/user/notifications/unread-count",
            get(notification::unread_count),
        )
        .route("/user/notifications", get(notification::list))
        .route(
            "/user/notifications/{id}/read",
            put(notification::mark_read),
        )
        .route(
            "/user/notifications/read-all",
            put(notification::mark_all_read),
        )
}
