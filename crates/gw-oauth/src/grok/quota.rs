//! CLI billing JSON + grok.com credits-frame merge.

use serde_json::Value;

use super::credits::{CreditsSnapshot, EMPTY_FRAME, decode_credits_frame};
use super::{BILLING_URL, CLI_USER_URL, CREDITS_URL, credits_headers, upstream_headers};
use crate::{Error, Session};

#[cfg(test)]
mod tests;

/// Production quota endpoints.
#[derive(Debug, Clone, Copy)]
pub struct QuotaUrls<'a> {
    pub billing: &'a str,
    pub user: &'a str,
    pub credits: &'a str,
}

impl QuotaUrls<'static> {
    pub const PRODUCTION: Self = Self {
        billing: BILLING_URL,
        user: CLI_USER_URL,
        credits: CREDITS_URL,
    };
}

/// Operator-facing Grok quota card.
#[derive(Debug, Clone, PartialEq)]
pub struct Quota {
    pub plan_type: String,
    pub subscription_status: String,
    pub has_grok_code_access: Option<bool>,
    pub rows: Vec<QuotaRow>,
}

/// One bar / product / prepaid line on the card.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaRow {
    pub key: String,
    pub kind: String,
    pub product: String,
    pub used_percent: Option<i64>,
    pub remaining_percent: Option<i64>,
    pub used: Option<f64>,
    pub total: Option<f64>,
    pub remaining: Option<f64>,
    pub reset_at_ms: Option<i64>,
    pub period_type: String,
    pub period_start: String,
    pub period_end: String,
}

/// Parse CLI `/v1/billing?format=credits` plus optional `/v1/user`.
///
/// Prepaid `0` and product shells with no numbers are dropped (unified billing).
#[must_use]
pub fn parse_billing(billing: &Value, cli_user: Option<&Value>) -> Quota {
    if !billing.is_object() {
        return Quota {
            plan_type: String::new(),
            subscription_status: String::new(),
            has_grok_code_access: None,
            rows: Vec::new(),
        };
    }
    let config = billing
        .get("config")
        .filter(|v| v.is_object())
        .unwrap_or(billing);
    let period = config
        .get("currentPeriod")
        .or_else(|| config.get("current_period"))
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));
    let user = user_payload(cli_user);
    let subscription = user
        .get("subscription")
        .cloned()
        .or_else(|| cli_user.and_then(|v| v.get("subscription").cloned()))
        .or_else(|| config.get("subscription").cloned());
    let plan_type = plan_label(pick_plan_raw(&[
        config.get("subscription_tier"),
        config.get("subscriptionTier"),
        billing.get("subscription_tier"),
        billing.get("subscriptionTier"),
        subscription.as_ref().and_then(|s| s.get("tier")),
        user.get("subscriptionTier"),
        user.get("subscription_tier"),
    ]));
    let subscription_status = subscription
        .as_ref()
        .and_then(|s| s.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let has_grok_code_access = user
        .get("hasGrokCodeAccess")
        .or_else(|| user.get("has_grok_code_access"))
        .or_else(|| cli_user.and_then(|v| v.get("hasGrokCodeAccess")))
        .and_then(Value::as_bool);

    let mut used_percent = as_number(
        config
            .get("creditUsagePercent")
            .or_else(|| config.get("credit_usage_percent")),
    )
    .and_then(clamp_pct);
    let on_demand = on_demand_bag(billing, config);
    let monthly = monthly_bag(billing, config);
    let amounts = credit_usage_sources(billing, config)
        .into_iter()
        .find_map(|source| {
            let bag = credit_bag_amounts(source)?;
            if bag.used.is_some() || bag.total.is_some() {
                Some(bag)
            } else {
                None
            }
        })
        .or(on_demand)
        .or(monthly);
    if used_percent.is_none() {
        used_percent = amounts.as_ref().and_then(credit_bag_used_percent);
    }
    let remaining_percent = used_percent.map(|used| 100 - used);
    let period_type = period
        .get("type")
        .or_else(|| period.get("periodType"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let period_start = period
        .get("start")
        .and_then(Value::as_str)
        .or_else(|| config.get("billingPeriodStart").and_then(Value::as_str))
        .unwrap_or("")
        .to_owned();
    let period_end = period
        .get("end")
        .and_then(Value::as_str)
        .or_else(|| config.get("billingPeriodEnd").and_then(Value::as_str))
        .unwrap_or("")
        .to_owned();
    let reset_at_ms = period_reset_at(
        period
            .get("end")
            .or_else(|| config.get("billingPeriodEnd"))
            .or_else(|| config.get("billing_period_end")),
    );

    let mut rows = Vec::new();
    if used_percent.is_some()
        || amounts.as_ref().and_then(|b| b.used).is_some()
        || amounts.as_ref().and_then(|b| b.total).is_some()
    {
        let kind = window_kind(&period_type);
        rows.push(QuotaRow {
            key: if kind == "weekly" {
                "weekly".to_owned()
            } else {
                "cycle".to_owned()
            },
            kind,
            product: String::new(),
            used_percent,
            remaining_percent,
            used: amounts.as_ref().and_then(|b| b.used),
            total: amounts.as_ref().and_then(|b| b.total),
            remaining: amounts.as_ref().and_then(|b| b.remaining),
            reset_at_ms,
            period_type: period_type.clone(),
            period_start: period_start.clone(),
            period_end: period_end.clone(),
        });
    }
    let prepaid = as_number(
        config
            .get("prepaidBalance")
            .or_else(|| config.get("prepaid_balance"))
            .or_else(|| billing.get("prepaidBalance"))
            .or_else(|| billing.get("prepaid_balance")),
    );
    if let Some(prepaid) = prepaid.filter(|n| *n > 0.0) {
        rows.push(QuotaRow {
            key: "prepaid".to_owned(),
            kind: "prepaid".to_owned(),
            product: String::new(),
            used_percent: None,
            remaining_percent: None,
            used: None,
            total: None,
            remaining: Some(prepaid),
            reset_at_ms: None,
            period_type: String::new(),
            period_start: String::new(),
            period_end: String::new(),
        });
    }
    let products = config
        .get("productUsage")
        .or_else(|| config.get("product_usage"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in products.into_iter().take(4) {
        if let Some(row) = product_row(&item) {
            rows.push(row);
        }
    }

    Quota {
        plan_type,
        subscription_status,
        has_grok_code_access,
        rows,
    }
}

/// Fill a missing weekly percent from the grok.com credits frame.
/// JSON percent wins; snapshot only supplies what billing omitted.
#[must_use]
pub fn apply_credits_snapshot(parsed: Quota, snapshot: Option<&CreditsSnapshot>) -> Quota {
    let mut rows = parsed.rows;
    let Some(snapshot) = snapshot else {
        return Quota { rows, ..parsed };
    };
    let idx = rows
        .iter()
        .position(|row| row.kind == "cycle" || row.kind == "weekly");
    if let Some(idx) = idx
        && rows[idx].used_percent.is_some()
    {
        if rows[idx].reset_at_ms.is_none() {
            rows[idx].reset_at_ms = snapshot.reset_at_ms;
        }
        return Quota { rows, ..parsed };
    }
    if snapshot.used_percent.is_none() && snapshot.reset_at_ms.is_none() {
        return Quota { rows, ..parsed };
    }
    let used_percent = snapshot.used_percent;
    let current = idx.and_then(|i| rows.get(i)).cloned();
    let next = QuotaRow {
        key: "weekly".to_owned(),
        kind: "weekly".to_owned(),
        product: String::new(),
        used_percent,
        remaining_percent: used_percent.map(|used| 100 - used),
        used: current.as_ref().and_then(|c| c.used),
        total: current.as_ref().and_then(|c| c.total),
        remaining: current.as_ref().and_then(|c| c.remaining),
        reset_at_ms: snapshot
            .reset_at_ms
            .or_else(|| current.as_ref().and_then(|c| c.reset_at_ms)),
        period_type: current
            .as_ref()
            .map(|c| c.period_type.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "USAGE_PERIOD_TYPE_WEEKLY".to_owned()),
        period_start: current
            .as_ref()
            .map(|c| c.period_start.clone())
            .unwrap_or_default(),
        period_end: current
            .as_ref()
            .map(|c| c.period_end.clone())
            .unwrap_or_default(),
    };
    if let Some(idx) = idx {
        if let Some(current) = rows.get_mut(idx) {
            *current = QuotaRow {
                product: current.product.clone(),
                ..next
            };
        }
    } else {
        rows.insert(0, next);
    }
    Quota { rows, ..parsed }
}

/// GET billing + user, POST grok.com credits. User 404 does not fail the card.
/// Billing and credits both failing is an error.
///
/// # Errors
/// Transport, or both billing and credits failed.
pub async fn fetch_quota(session: &Session) -> Result<Quota, Error> {
    let client = super::http_client()?;
    fetch_quota_at(session, &client, QuotaUrls::PRODUCTION).await
}

pub(crate) async fn fetch_quota_at(
    session: &Session,
    client: &reqwest::Client,
    urls: QuotaUrls<'_>,
) -> Result<Quota, Error> {
    let billing_headers = quota_headers(session);
    let billing = get_json(client, urls.billing, &billing_headers).await;
    let user = get_json(client, urls.user, &billing_headers).await.ok();
    let credits = post_credits(client, urls.credits, session).await;
    match (billing, credits) {
        (Err(billing_err), Err(_)) => Err(billing_err),
        (billing, credits) => {
            let parsed = parse_billing(
                &billing.unwrap_or(Value::Object(serde_json::Map::new())),
                user.as_ref(),
            );
            Ok(apply_credits_snapshot(parsed, credits.ok().as_ref()))
        }
    }
}

fn quota_headers(session: &Session) -> reqwest::header::HeaderMap {
    let mut headers = upstream_headers(session);
    if let Ok(value) = reqwest::header::HeaderValue::from_str(super::CLIENT_VERSION) {
        headers.insert("x-grok-client-version", value.clone());
        headers.insert("x-grok-cli-version", value);
    }
    if let Ok(value) = reqwest::header::HeaderValue::from_str("grok-cli") {
        headers.insert("x-grok-client-surface", value);
    }
    headers
}

async fn get_json(
    client: &reqwest::Client,
    url: &str,
    headers: &reqwest::header::HeaderMap,
) -> Result<Value, Error> {
    let response = client.get(url).headers(headers.clone()).send().await?;
    let status = response.status().as_u16();
    let body = response.text().await?;
    if !(200..300).contains(&status) {
        return Err(Error::TokenEndpoint { status, body });
    }
    serde_json::from_str(&body)
        .map_err(|_| Error::Payload("grok quota response is not JSON".to_owned()))
}

async fn post_credits(
    client: &reqwest::Client,
    url: &str,
    session: &Session,
) -> Result<CreditsSnapshot, Error> {
    let response = client
        .post(url)
        .headers(credits_headers(session))
        .body(EMPTY_FRAME.to_vec())
        .send()
        .await?;
    let status = response.status().as_u16();
    let bytes = response.bytes().await?;
    if !(200..300).contains(&status) {
        return Err(Error::TokenEndpoint {
            status,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        });
    }
    decode_credits_frame(&bytes)
        .ok_or_else(|| Error::Payload("grok credits returned no usage".to_owned()))
}

#[derive(Clone, Copy)]
struct CreditBag {
    used: Option<f64>,
    total: Option<f64>,
    remaining: Option<f64>,
}

pub(crate) fn as_number(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) if !s.trim().is_empty() => s.trim().parse().ok(),
        Value::Object(obj) => as_number(obj.get("val")).or_else(|| as_number(obj.get("value"))),
        _ => None,
    }
}

fn clamp_pct(n: f64) -> Option<i64> {
    if !n.is_finite() {
        return None;
    }
    Some(n.round().clamp(0.0, 100.0) as i64)
}

fn credit_bag_amounts(value: Option<&Value>) -> Option<CreditBag> {
    let value = value?;
    if let Value::Array(items) = value {
        return items.iter().find_map(|item| credit_bag_amounts(Some(item)));
    }
    let Value::Object(obj) = value else {
        return None;
    };
    let total = as_number(obj.get("total"))
        .or_else(|| as_number(obj.get("limit")))
        .or_else(|| as_number(obj.get("cap")))
        .or_else(|| as_number(obj.get("allocation")))
        .or_else(|| as_number(obj.get("amount")));
    let used = as_number(obj.get("used"))
        .or_else(|| as_number(obj.get("spent")))
        .or_else(|| as_number(obj.get("consumed")))
        .or_else(|| as_number(obj.get("usage")));
    let remaining = as_number(obj.get("remaining"))
        .or_else(|| as_number(obj.get("balance")))
        .or_else(|| as_number(obj.get("left")));
    if total.is_none() && used.is_none() && remaining.is_none() {
        return credit_bag_amounts(obj.get("bags"))
            .or_else(|| credit_bag_amounts(obj.get("items")));
    }
    let resolved_used = used.or_else(|| match (total, remaining) {
        (Some(total), Some(remaining)) => Some((total - remaining).max(0.0)),
        _ => None,
    });
    let resolved_remaining = remaining.or_else(|| match (total, resolved_used) {
        (Some(total), Some(used)) => Some((total - used).max(0.0)),
        _ => None,
    });
    Some(CreditBag {
        used: resolved_used,
        total,
        remaining: resolved_remaining,
    })
}

fn credit_bag_used_percent(bag: &CreditBag) -> Option<i64> {
    let total = bag.total.filter(|t| *t > 0.0)?;
    let used = bag.used?;
    clamp_pct((used / total) * 100.0)
}

fn credit_usage_sources<'a>(billing: &'a Value, config: &'a Value) -> Vec<Option<&'a Value>> {
    vec![
        billing.get("credits"),
        billing.get("creditBalance"),
        billing.get("usage"),
        config.get("credits"),
        config.get("includedCredits"),
        config.get("subscriptionCredits"),
        config.get("weeklyCredits"),
        config.get("sharedPool"),
    ]
}

fn on_demand_bag(billing: &Value, config: &Value) -> Option<CreditBag> {
    let used = as_number(config.get("onDemandUsed"))
        .or_else(|| as_number(config.get("on_demand_used")))
        .or_else(|| as_number(billing.get("onDemandUsed")))
        .or_else(|| as_number(billing.get("on_demand_used")));
    let total = as_number(config.get("onDemandCap"))
        .or_else(|| as_number(config.get("on_demand_cap")))
        .or_else(|| as_number(billing.get("onDemandCap")))
        .or_else(|| as_number(billing.get("on_demand_cap")));
    if used.is_none() && total.is_none() {
        return None;
    }
    let remaining = match (total, used) {
        (Some(total), Some(used)) => Some((total - used).max(0.0)),
        _ => None,
    };
    Some(CreditBag {
        used,
        total,
        remaining,
    })
}

fn monthly_bag(billing: &Value, config: &Value) -> Option<CreditBag> {
    let used = as_number(config.get("used"))
        .or_else(|| as_number(billing.pointer("/usage/includedUsed")))
        .or_else(|| as_number(billing.pointer("/usage/totalUsed")))
        .or_else(|| as_number(billing.get("includedUsed")));
    let total = as_number(config.get("monthlyLimit"))
        .or_else(|| as_number(config.get("monthly_limit")))
        .or_else(|| as_number(billing.get("monthlyLimit")))
        .or_else(|| as_number(billing.get("monthly_limit")));
    let total = total.filter(|t| *t > 0.0)?;
    let remaining = used.map(|used| (total - used).max(0.0));
    Some(CreditBag {
        used,
        total: Some(total),
        remaining,
    })
}

fn window_kind(period_type: &str) -> String {
    if period_type.to_ascii_lowercase().contains("month") {
        "cycle".to_owned()
    } else {
        "weekly".to_owned()
    }
}

fn product_row(item: &Value) -> Option<QuotaRow> {
    let product = item
        .get("product")
        .or_else(|| item.get("name"))
        .or_else(|| item.get("productName"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?
        .to_owned();
    let bag = credit_bag_amounts(Some(item)).unwrap_or(CreditBag {
        used: None,
        total: None,
        remaining: None,
    });
    let used_percent = as_number(item.get("usagePercent"))
        .or_else(|| as_number(item.get("usedPercent")))
        .or_else(|| as_number(item.get("usage_percent")))
        .and_then(clamp_pct)
        .or_else(|| match (bag.total, bag.used) {
            (Some(total), Some(used)) if total > 0.0 => clamp_pct((used / total) * 100.0),
            _ => None,
        });
    if used_percent.is_none() && bag.used.is_none() && bag.total.is_none() {
        return None;
    }
    Some(QuotaRow {
        key: format!("product:{product}"),
        kind: "product".to_owned(),
        product,
        remaining_percent: used_percent.map(|used| 100 - used),
        used_percent,
        used: bag.used,
        total: bag.total,
        remaining: bag.remaining,
        reset_at_ms: None,
        period_type: String::new(),
        period_start: String::new(),
        period_end: String::new(),
    })
}

fn user_payload(cli_user: Option<&Value>) -> Value {
    let Some(cli_user) = cli_user else {
        return Value::Object(serde_json::Map::new());
    };
    cli_user
        .get("user")
        .or_else(|| cli_user.get("profile"))
        .cloned()
        .unwrap_or_else(|| cli_user.clone())
}

fn pick_plan_raw(values: &[Option<&Value>]) -> Option<Value> {
    for value in values.iter().flatten().copied() {
        if is_int(value) {
            return Some(value.clone());
        }
        if let Some(s) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            return Some(Value::String(s.to_owned()));
        }
    }
    None
}

fn is_int(value: &Value) -> bool {
    match value {
        Value::Number(n) => {
            n.as_i64().is_some()
                || n.as_u64().is_some()
                || n.as_f64()
                    .is_some_and(|f| f.is_finite() && f.fract() == 0.0)
        }
        _ => false,
    }
}

fn plan_label(raw: Option<Value>) -> String {
    let Some(raw) = raw else {
        return String::new();
    };
    let Some(named) = super::tier_from_value(&raw) else {
        return String::new();
    };
    crate::format_plan_label(&named, crate::Family::Grok)
}

fn period_reset_at(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::String(s) if !s.is_empty() => chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.timestamp_millis()),
        other => {
            let n = as_number(Some(other))?;
            if n > 1e12 {
                Some(n.round() as i64)
            } else if n > 0.0 {
                Some((n * 1000.0).round() as i64)
            } else {
                None
            }
        }
    }
}
