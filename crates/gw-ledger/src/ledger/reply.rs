//! Lua 调用与**回复的读法**：redis-rs 的错误类型 ↔ 账本的结局。
//!
//! 分出来是因为它是一件独立的事：脚本回什么、怎么把 `error_reply` 的标记
//! 认出来、怎么从 `INSUFFICIENT_BALANCE:<n>` 里把那个数抠出来。
//! 标记既查解析出来的错误码**又**查渲染后的消息 —— redis-rs 对自定义
//! `error_reply` 的分类方式一旦变化，不能让一次可重试的缓存未命中
//! 静默变成一次硬失败。

use redis::aio::ConnectionManager;

use crate::keys::{balance_key, holds_key, holds_ts_key};
use crate::scripts::{CACHE_MISS, GET_BALANCE_SCRIPT, HOLD_SCRIPT, INSUFFICIENT_BALANCE};

/// Runs [`GET_BALANCE_SCRIPT`] and returns its raw string reply.
pub(crate) async fn run_get_balance(
    conn: &mut ConnectionManager,
    user_id: i64,
    now: i64,
    hold_ttl_secs: i64,
) -> Result<String, redis::RedisError> {
    let mut inv = GET_BALANCE_SCRIPT.prepare_invoke();
    inv.key(balance_key(user_id));
    inv.key(holds_key(user_id));
    inv.key(holds_ts_key(user_id));
    inv.arg(now);
    inv.arg(hold_ttl_secs);
    inv.invoke_async(conn).await
}

/// Runs [`HOLD_SCRIPT`] and returns its raw string reply (`OK` on admission).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_hold(
    conn: &mut ConnectionManager,
    user_id: i64,
    amount: &str,
    request_id: &str,
    now: i64,
    hold_ttl_secs: i64,
    key_ttl_secs: i64,
    min_available: &str,
) -> Result<String, redis::RedisError> {
    let mut inv = HOLD_SCRIPT.prepare_invoke();
    inv.key(balance_key(user_id));
    inv.key(holds_key(user_id));
    inv.key(holds_ts_key(user_id));
    inv.arg(amount);
    inv.arg(request_id);
    inv.arg(now);
    inv.arg(hold_ttl_secs);
    inv.arg(""); // idempotency key (unused for now, kept for parity)
    inv.arg(key_ttl_secs);
    inv.arg(min_available);
    inv.invoke_async(conn).await
}

/// Whether a Lua reply was the `CACHE_MISS` marker, meaning the user's balance
/// is not cached and must be loaded from Postgres first.
///
/// Checks the parsed error code *and* the rendered message, so a change in
/// how redis-rs classifies a custom `error_reply` cannot silently turn a
/// retryable miss into a hard failure.
pub(crate) fn is_cache_miss(err: &redis::RedisError) -> bool {
    marks(err, CACHE_MISS)
}

/// Whether a Lua reply was the `INSUFFICIENT_BALANCE:<available>` refusal.
pub(crate) fn is_insufficient_balance(err: &redis::RedisError) -> bool {
    marks(err, INSUFFICIENT_BALANCE)
}

/// Available-balance payload of `INSUFFICIENT_BALANCE:<n>`, if present.
pub(crate) fn parse_insufficient_available(err: &redis::RedisError) -> Option<f64> {
    err.code()
        .and_then(available_after_marker)
        .or_else(|| available_after_marker(&err.to_string()))
}

fn available_after_marker(s: &str) -> Option<f64> {
    let rest = s.split_once(INSUFFICIENT_BALANCE)?.1;
    let rest = rest.strip_prefix(':').unwrap_or(rest).trim();
    let token = rest
        .split(|c: char| c.is_whitespace() || c == ',' || c == ')' || c == '/')
        .find(|part| !part.is_empty())?;
    token.parse().ok()
}

pub(crate) fn marks(err: &redis::RedisError, marker: &str) -> bool {
    err.code().is_some_and(|c| c.contains(marker)) || err.to_string().contains(marker)
}
