//! The unified JSON envelope.
//!
//! Only the endpoints this crate owns (`/api/health`, `/metrics`) use it —
//! `gw-panel` carries the same shape for `/api/panel/**`. The field ORDER is
//! `code`, `message`, `data` and `data` is omitted when absent, mirroring
//! `json:"data,omitempty"`.

use axum::Json;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// The unified JSON envelope type.
#[derive(Debug, Clone, Serialize)]
pub struct ApiResponse<T> {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

impl<T> ApiResponse<T> {
    /// HTTP 200 with `code: 0, message: "ok"`.
    pub fn ok(data: T) -> Self {
        Self {
            code: 0,
            message: "ok".to_owned(),
            data: Some(data),
        }
    }
}

/// A successful envelope response.
pub fn success<T: Serialize>(data: T) -> Response {
    Json(ApiResponse::ok(data)).into_response()
}
