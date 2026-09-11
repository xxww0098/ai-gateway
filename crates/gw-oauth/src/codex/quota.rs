//! ChatGPT Codex quota JSON (wham/usage and reset-credits). No network.

use chrono::Utc;
use serde_json::{Map, Value};

#[cfg(test)]
mod tests;

const USED_RESET_STATUS: [&str; 4] = ["redeemed", "used", "consumed", "expired"];

/// One usage window from `wham/usage`.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageRow {
    pub key: String,
    pub kind: String,
    pub used_percent: i64,
    pub remaining_percent: i64,
    pub window_minutes: Option<i64>,
    pub reset_at_ms: Option<i64>,
}

/// Parsed Codex usage payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Usage {
    pub plan_type: Option<String>,
    pub rows: Vec<UsageRow>,
}

/// One rate-limit reset credit.
#[derive(Debug, Clone, PartialEq)]
pub struct ResetCredit {
    pub id: Option<String>,
    pub status: Option<String>,
    pub expires_at_ms: Option<i64>,
}

/// Parsed reset-credits payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ResetCredits {
    pub available_count: i64,
    pub credits: Vec<ResetCredit>,
    pub next_expires_at_ms: Option<i64>,
}

/// `primary_window` → 5-hour `primary`; `secondary_window` → `weekly`.
#[must_use]
pub fn parse_usage(payload: &Value) -> Usage {
    let Some(obj) = payload.as_object() else {
        return Usage {
            plan_type: None,
            rows: Vec::new(),
        };
    };
    let rate = obj
        .get("rate_limit")
        .or_else(|| obj.get("rateLimit"))
        .and_then(Value::as_object);
    let mut rows = Vec::new();
    if let Some(primary) = rate.and_then(|rate| {
        parse_window(
            rate.get("primary_window")
                .or_else(|| rate.get("primaryWindow")),
        )
    }) {
        rows.push(UsageRow {
            key: "primary".to_owned(),
            kind: "primary".to_owned(),
            used_percent: primary.used_percent,
            remaining_percent: primary.remaining_percent,
            window_minutes: primary.window_minutes,
            reset_at_ms: primary.reset_at_ms,
        });
    }
    if let Some(secondary) = rate.and_then(|rate| {
        parse_window(
            rate.get("secondary_window")
                .or_else(|| rate.get("secondaryWindow")),
        )
    }) {
        rows.push(UsageRow {
            key: "weekly".to_owned(),
            kind: "weekly".to_owned(),
            used_percent: secondary.used_percent,
            remaining_percent: secondary.remaining_percent,
            window_minutes: secondary.window_minutes,
            reset_at_ms: secondary.reset_at_ms,
        });
    }
    let plan_type = string_field(obj, &["plan_type", "planType"]);
    Usage { plan_type, rows }
}

/// Reset-credit inventory. Missing `available_count` is derived from rows.
#[must_use]
pub fn parse_reset_credits(payload: &Value) -> ResetCredits {
    let Some(obj) = payload.as_object() else {
        return ResetCredits {
            available_count: 0,
            credits: Vec::new(),
            next_expires_at_ms: None,
        };
    };
    let nested = obj.get("data").and_then(Value::as_object);
    let raw_credits = obj
        .get("credits")
        .or_else(|| nested.and_then(|data| data.get("credits")));
    let credits = match raw_credits.and_then(Value::as_array) {
        Some(items) => items.iter().filter_map(parse_reset_credit).collect(),
        None => Vec::new(),
    };
    let listed = as_number(
        obj.get("available_count")
            .or_else(|| obj.get("availableCount"))
            .or_else(|| nested.and_then(|data| data.get("available_count")))
            .or_else(|| nested.and_then(|data| data.get("availableCount"))),
    );
    let available_count = match listed {
        Some(n) => n.round().max(0.0) as i64,
        None => credits
            .iter()
            .filter(|row| is_available_reset_credit(row))
            .count() as i64,
    };
    let listed_expiry = stamp_of(
        obj.get("expires_at")
            .or_else(|| obj.get("expire_at"))
            .or_else(|| obj.get("next_expire_at"))
            .or_else(|| obj.get("nextExpiresAt"))
            .or_else(|| nested.and_then(|data| data.get("expires_at")))
            .or_else(|| nested.and_then(|data| data.get("next_expire_at"))),
    );
    let from_credits = credits
        .iter()
        .filter(|row| is_available_reset_credit(row))
        .filter_map(|row| row.expires_at_ms)
        .min();
    ResetCredits {
        available_count,
        credits,
        next_expires_at_ms: from_credits.or(listed_expiry),
    }
}

/// True when the credit can still be redeemed.
#[must_use]
pub fn is_available_reset_credit(credit: &ResetCredit) -> bool {
    let status = credit
        .status
        .as_deref()
        .unwrap_or("available")
        .trim()
        .to_ascii_lowercase();
    if USED_RESET_STATUS.contains(&status.as_str()) {
        return false;
    }
    match credit.expires_at_ms {
        Some(stamp) => stamp > now_ms(),
        None => true,
    }
}

/// Body for `POST .../rate-limit-reset-credits/consume`.
#[must_use]
pub fn consume_reset_body(request_id: &str) -> Value {
    Value::Object(Map::from_iter([
        (
            "redeem_request_id".to_owned(),
            Value::String(request_id.to_owned()),
        ),
        (
            "idempotencyKey".to_owned(),
            Value::String(request_id.to_owned()),
        ),
    ]))
}

struct Window {
    used_percent: i64,
    remaining_percent: i64,
    window_minutes: Option<i64>,
    reset_at_ms: Option<i64>,
}

fn parse_window(window: Option<&Value>) -> Option<Window> {
    let obj = window.and_then(Value::as_object)?;
    let used_percent = clamp_pct(
        as_number(obj.get("used_percent"))
            .or_else(|| as_number(obj.get("usedPercent")))
            .unwrap_or(0.0),
    );
    let seconds = as_number(obj.get("limit_window_seconds"))
        .or_else(|| as_number(obj.get("limitWindowSeconds")));
    let window_minutes = seconds
        .filter(|value| *value > 0.0)
        .map(|value| ((value + 59.0) / 60.0).floor() as i64);
    Some(Window {
        used_percent,
        remaining_percent: 100 - used_percent,
        window_minutes,
        reset_at_ms: reset_at_of(obj),
    })
}

fn parse_reset_credit(item: &Value) -> Option<ResetCredit> {
    let obj = item.as_object()?;
    let raw_status = obj
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| obj.get("state").and_then(Value::as_str));
    let expires_at_ms = stamp_of(
        obj.get("expires_at")
            .or_else(|| obj.get("expire_at"))
            .or_else(|| obj.get("expiresAt")),
    );
    let mut status = raw_status.map(|value| value.trim().to_ascii_lowercase());
    if status.as_ref().is_none_or(|value| value.is_empty())
        && expires_at_ms.is_some_and(|stamp| stamp <= now_ms())
    {
        status = Some("expired".to_owned());
    }
    let id = obj
        .get("id")
        .or_else(|| obj.get("credit_id"))
        .or_else(|| obj.get("creditId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Some(ResetCredit {
        id,
        status,
        expires_at_ms,
    })
}

fn reset_at_of(window: &Map<String, Value>) -> Option<i64> {
    if let Some(stamp) = stamp_of(
        window
            .get("reset_at")
            .or_else(|| window.get("resetAt"))
            .or_else(|| window.get("resets_at"))
            .or_else(|| window.get("resetsAt"))
            .or_else(|| window.get("reset_time"))
            .or_else(|| window.get("resetTime")),
    ) {
        return Some(stamp);
    }
    let after = as_number(window.get("reset_after_seconds"))
        .or_else(|| as_number(window.get("resetAfterSeconds")))
        .or_else(|| as_number(window.get("seconds_until_reset")))
        .or_else(|| as_number(window.get("secondsUntilReset")))
        .or_else(|| as_number(window.get("reset_after")))
        .or_else(|| as_number(window.get("resetAfter")))?;
    if after >= 0.0 {
        Some(now_ms().saturating_add((after * 1000.0).round() as i64))
    } else {
        None
    }
}

fn as_number(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    match value {
        Value::Number(number) => number.as_f64().filter(|n| n.is_finite()),
        Value::String(text) if !text.trim().is_empty() => {
            text.trim().parse::<f64>().ok().filter(|n| n.is_finite())
        }
        Value::Object(map) => as_number(map.get("val")).or_else(|| as_number(map.get("value"))),
        _ => None,
    }
}

fn clamp_pct(value: f64) -> i64 {
    value.round().clamp(0.0, 100.0) as i64
}

fn stamp_of(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    if let Some(n) = as_number(Some(value)).filter(|n| *n > 0.0) {
        return Some(if n > 1e12 {
            n.round() as i64
        } else {
            (n * 1000.0).round() as i64
        });
    }
    let Value::String(text) = value else {
        return None;
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(trimmed)
        .ok()
        .map(|stamp| stamp.timestamp_millis())
}

fn string_field(obj: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        obj.get(*name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    })
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}
