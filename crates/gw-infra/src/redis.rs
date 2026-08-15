//! Redis bootstrap.

use std::time::Duration;

use ::redis::aio::ConnectionManager;
use ::redis::{Client, ConnectionAddr, ConnectionInfo, RedisConnectionInfo, RedisError};

use crate::Redis;

#[cfg(test)]
mod tests;

/// Startup connectivity budget: the `PING` is given 2 seconds before it is
/// treated as a failure.
pub const PING_TIMEOUT: Duration = Duration::from_secs(2);

/// Port assumed when `redis.addr` carries only a host.
pub const DEFAULT_PORT: u16 = 6379;

/// Connects to Redis and verifies it with a `PING`.
///
/// Fail-soft by design: an empty address, an unparsable address, or an
/// unreachable server all yield `None` (an absent client) after a warning,
/// and the gateway carries on without Redis-backed holds. Only a `Some` may
/// be handed to [`crate::RateLimiter`] / [`crate::CircuitBreaker`] as a live
/// handle.
pub async fn init_redis(addr: &str, password: &str, db: i64) -> Option<Redis> {
    if addr.trim().is_empty() {
        tracing::warn!("Redis disabled: redis.addr is empty");
        return None;
    }

    let (host, port) = match parse_addr(addr) {
        Ok(parsed) => parsed,
        Err(reason) => {
            tracing::warn!(addr, reason, "Redis disabled: unusable redis.addr");
            return None;
        }
    };

    let info = ConnectionInfo {
        addr: ConnectionAddr::Tcp(host, port),
        redis: RedisConnectionInfo {
            db,
            password: (!password.is_empty()).then(|| password.to_owned()),
            ..RedisConnectionInfo::default()
        },
    };

    let client = match Client::open(info) {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(addr, %error, "Redis unavailable; continuing without Redis-backed holds");
            return None;
        }
    };

    match tokio::time::timeout(PING_TIMEOUT, connect_and_ping(client)).await {
        Ok(Ok(conn)) => {
            tracing::info!(addr, db, "Redis connection established");
            Some(conn)
        }
        Ok(Err(error)) => {
            tracing::warn!(addr, %error, "Redis unavailable; continuing without Redis-backed holds");
            None
        }
        Err(_elapsed) => {
            tracing::warn!(
                addr,
                timeout_ms = PING_TIMEOUT.as_millis() as u64,
                "Redis unavailable (ping timed out); continuing without Redis-backed holds"
            );
            None
        }
    }
}

/// Opens the managed connection and round-trips one `PING`.
///
/// `ConnectionManager` reconnects on its own afterwards; this only proves the
/// server was reachable at startup.
async fn connect_and_ping(client: Client) -> Result<Redis, RedisError> {
    let mut conn = ConnectionManager::new(client).await?;
    let _pong: String = ::redis::cmd("PING").query_async(&mut conn).await?;
    Ok(conn)
}

/// Splits a `host:port` address form into the pieces
/// [`ConnectionAddr::Tcp`] wants.
///
/// Accepts `host`, `host:port`, `[v6]:port` and bare `[v6]`/`v6`. A missing
/// port means [`DEFAULT_PORT`]; a missing host means loopback.
fn parse_addr(addr: &str) -> Result<(String, u16), String> {
    let trimmed = addr.trim();
    if trimmed.is_empty() {
        return Err("address is empty".to_owned());
    }

    if let Some(rest) = trimmed.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| format!("unbalanced '[' in {trimmed:?}"))?;
        let port = match tail {
            "" => DEFAULT_PORT,
            _ => parse_port(
                tail.strip_prefix(':')
                    .ok_or_else(|| format!("expected ':port' after ']' in {trimmed:?}"))?,
            )?,
        };
        return Ok((host_or_loopback(host), port));
    }

    match trimmed.rsplit_once(':') {
        // A colon left in the host half means this is a bare IPv6 literal.
        Some((host, _)) if host.contains(':') => Ok((trimmed.to_owned(), DEFAULT_PORT)),
        Some((host, port)) => Ok((host_or_loopback(host), parse_port(port)?)),
        None => Ok((trimmed.to_owned(), DEFAULT_PORT)),
    }
}

fn parse_port(raw: &str) -> Result<u16, String> {
    match raw.parse::<u16>() {
        Ok(0) | Err(_) => Err(format!("invalid port {raw:?}")),
        Ok(port) => Ok(port),
    }
}

fn host_or_loopback(host: &str) -> String {
    if host.is_empty() {
        "127.0.0.1".to_owned()
    } else {
        host.to_owned()
    }
}
