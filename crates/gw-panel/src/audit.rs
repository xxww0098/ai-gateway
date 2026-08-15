//! Tamper-evident operation logging — **shared ground, coordinator-owned**.
//!
//! 对应 `recordOperation`（写）与 `DeriveAuditKey` / `auditCanonical` /
//! `auditEntryHash`（哈希与 canonical）。
//!
//! This lives at the crate root rather than in the `ops` domain because *writing*
//! an operation log is cross-cutting: `identity` writes 8 of them (register,
//! login, logout, user create/update/delete/deposit, order confirm) and
//! `commerce` writes more. Only *reading* them — `/admin/audit-logs` aggregation
//! and `VerifyAuditLog` — is an `ops` concern, and that stays in `ops/`.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// `OperationLog.Source` for panel-originated entries. 对应 `opSourcePanel`。
pub const SOURCE_PANEL: &str = "panel";

/// Who performed the operation. `None` for unauthenticated paths (failed login,
/// register) —— 此时 `ActorID` 留 0。
#[derive(Debug, Clone, Default)]
pub struct Actor {
    pub user_id: i64,
    pub email: String,
    pub role: String,
}

/// Request-derived fields. 旧实现从 `*gin.Context` 读；axum handlers must
/// pass them explicitly.
#[derive(Debug, Clone, Default)]
pub struct RequestMeta {
    pub method: String,
    /// 旧实现用 `c.FullPath()`（匹配到的路由模式），退回原始 URL path。
    /// In axum that is `MatchedPath`, falling back to `uri.path()`.
    pub path: String,
    pub ip_address: String,
    pub request_id: String,
}

/// One row of `operation_logs`, pre-insert.
#[derive(Debug, Clone)]
pub struct OperationEntry {
    pub source: String,
    pub actor_id: i64,
    pub actor_email: String,
    pub actor_role: String,
    pub action: String,
    pub target: String,
    pub method: String,
    pub path: String,
    pub status_code: i32,
    pub ip_address: String,
    pub request_id: String,
    /// Raw JSON bytes, exactly as stored in the `jsonb` column.
    pub metadata: Vec<u8>,
    pub created_at: DateTime<Utc>,
}

/// Derives the audit HMAC key from the credential encryption secret.
///
/// 对应 `DeriveAuditKey`：`sha256(secret + "|cpa-audit-hmac-v1")`。
/// An empty/blank secret disables hashing（此时返回 `nil`）。
pub fn derive_audit_key(secret: &str) -> Option<Vec<u8>> {
    if secret.trim().is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b"|cpa-audit-hmac-v1");
    Some(hasher.finalize().to_vec())
}

/// Deterministic byte representation of an entry's content.
///
/// 对应 `auditCanonical`。**Byte-for-byte compatibility is required** ——
/// 既有实现写入的行必须在 Rust 下仍可通过校验。
/// Field order and the `\0` separator are load-bearing; `created_at` is the UTC
/// nanosecond timestamp, matching `CreatedAt.UTC().UnixNano()`.
fn canonical(entry: &OperationEntry) -> Vec<u8> {
    let actor_id = entry.actor_id.to_string();
    let status = entry.status_code.to_string();
    let created_nanos = entry
        .created_at
        .timestamp_nanos_opt()
        .unwrap_or_default()
        .to_string();
    let parts: [&[u8]; 13] = [
        entry.source.as_bytes(),
        actor_id.as_bytes(),
        entry.actor_email.as_bytes(),
        entry.actor_role.as_bytes(),
        entry.action.as_bytes(),
        entry.target.as_bytes(),
        entry.method.as_bytes(),
        entry.path.as_bytes(),
        status.as_bytes(),
        entry.ip_address.as_bytes(),
        entry.request_id.as_bytes(),
        &entry.metadata,
        created_nanos.as_bytes(),
    ];
    parts.join(&0u8)
}

/// Hex HMAC-SHA256 of the entry under `key`; empty string when hashing is off.
///
/// 对应 `auditEntryHash`。
pub fn entry_hash(key: Option<&[u8]>, entry: &OperationEntry) -> String {
    let Some(key) = key.filter(|k| !k.is_empty()) else {
        return String::new();
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(&canonical(entry));
    hex::encode(mac.finalize().into_bytes())
}
