//! Stripe 回调：签名校验 + 幂等结算。
//!
//! 对应 `handler_stripe_webhook`。
//!
//! # 签名是这条路由唯一的身份证明
//!
//! 这个端点挂在 `/api/payment/stripe/webhook`（panel 组之外），**不带 bearer
//! token** —— Stripe 不会带。`Stripe-Signature` 头的 HMAC 就是认证本身，所以
//! 校验失败必须在**读取事件内容之前**结束请求。
//!
//! 手写而不是引 Stripe SDK：Stripe 的方案就是
//! `HMAC-SHA256(secret, "<timestamp>.<payload>")`，用 `hmac` + `sha2` 三十行写完，
//! 比为一个校验函数拖进整套 SDK 依赖更合适（CONTRACT 也明确要求不加 stripe 依赖）。
//!
//! # 三处安全细节
//!
//! * **定长比较**。用 `hmac` 的 `verify_slice`，不是 `==` —— 字节比较会因为提前
//!   返回而泄露前缀信息。
//! * **时间戳容差**。签名有效的事件被截获后仍可重放；5 分钟容差把窗口关上。
//! * **响应体不是统一信封**。旧实现这里发的是裸 `{"received":true}`，Stripe 只看
//!   HTTP 状态，但形状照抄。

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

use crate::identity::{bad_request, internal};
use crate::{PanelState, codes, err};

#[cfg(test)]
mod tests;

/// 缓冲上限。Ports `stripeWebhookMaxBody`（1 MiB）——签名校验必须先把整个
/// body 读进来，所以它必须有界，否则一个超长请求就能吃掉内存。
pub const MAX_BODY_BYTES: usize = 1 << 20;

/// 事件时间戳允许偏离现在多久。Ports `stripeSignatureTolerance`。
pub const SIGNATURE_TOLERANCE_SECONDS: i64 = 5 * 60;

/// Stripe 送来的签名头名。
const SIGNATURE_HEADER: &str = "Stripe-Signature";

/// 只有这两类事件会触发结算；其余一律 ack 掉，免得 Stripe 反复重投。
const SETTLING_EVENTS: [&str; 2] = ["payment_intent.succeeded", "checkout.session.completed"];

/// 我们关心的那几个字段。其余一概忽略 —— Stripe 的事件体很大且会演进。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StripeEvent {
    #[serde(rename = "type")]
    kind: String,
    data: StripeEventData,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StripeEventData {
    object: StripeEventObject,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StripeEventObject {
    metadata: std::collections::HashMap<String, String>,
}

/// 签名校验失败的原因。只用于日志 —— 对外一律是同一句 `invalid signature`。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SignatureError {
    /// 头缺失、或者没有 `t=` / `v1=`。
    #[error("malformed signature header")]
    Malformed,
    /// `t=` 不是整数。
    #[error("bad timestamp")]
    BadTimestamp,
    /// 时间戳偏离太远，按重放处理。
    #[error("timestamp outside tolerance")]
    OutsideTolerance,
    /// 没有任何一个 `v1=` 对得上。
    #[error("no matching v1 signature")]
    NoMatch,
}

/// 校验 `Stripe-Signature`。Ports `verifyStripeSignature`。
///
/// 头的格式是 `t=<unix>,v1=<hex>[,v1=<hex>…]`，签名内容是 `"<t>.<payload>"`。
/// 多个 `v1` 是 Stripe 轮换密钥时的常态，**任意一个**对上即通过。
///
/// `tolerance_seconds <= 0` 关闭时间戳检查（测试用）；`now` 注入是为了让容差
/// 边界可测而不必等时钟。
///
/// # Errors
/// 见 [`SignatureError`]。
pub fn verify_signature(
    payload: &[u8],
    signature_header: &str,
    secret: &str,
    tolerance_seconds: i64,
    now: DateTime<Utc>,
) -> Result<(), SignatureError> {
    let mut timestamp: Option<&str> = None;
    let mut candidates: Vec<&str> = Vec::new();
    for part in signature_header.split(',') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        match key.trim() {
            "t" => timestamp = Some(value.trim()),
            "v1" => candidates.push(value.trim()),
            _ => {}
        }
    }

    let timestamp = timestamp
        .filter(|t| !t.is_empty())
        .ok_or(SignatureError::Malformed)?;
    if candidates.is_empty() {
        return Err(SignatureError::Malformed);
    }
    let seconds: i64 = timestamp
        .parse()
        .map_err(|_| SignatureError::BadTimestamp)?;

    if tolerance_seconds > 0 && (now.timestamp() - seconds).abs() > tolerance_seconds {
        return Err(SignatureError::OutsideTolerance);
    }

    for candidate in candidates {
        // 定长比较：先把十六进制解开，再交给 `verify_slice`（内部走 subtle 的
        // 常数时间等值），**不要**写成 `expected_hex == candidate` —— 字节比较
        // 提前返回，会把匹配的前缀长度泄露给攻击者。
        let Ok(bytes) = hex::decode(candidate) else {
            continue;
        };
        let mut mac = <Hmac<Sha256>>::new_from_slice(secret.as_bytes())
            .map_err(|_| SignatureError::NoMatch)?;
        mac.update(timestamp.as_bytes());
        mac.update(b".");
        mac.update(payload);
        if mac.verify_slice(&bytes).is_ok() {
            return Ok(());
        }
    }
    Err(SignatureError::NoMatch)
}

/// `POST /api/payment/stripe/webhook` —— 公网入口，无鉴权层。
///
/// Ports `PaymentStripeWebhookHandler`。流程：没配密钥 → 503；读 body（有界）→
/// 验签 → 解事件 → 非结算类事件直接 ack → 取 `metadata.order_id` → 走
/// [`super::payment::settle_payment_order`]。
///
/// 结算失败回 **500**，好让 Stripe 重投 —— 这是安全的，因为结算本身幂等。
pub async fn webhook(State(state): State<PanelState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(secret) = state
        .stripe_webhook_secret
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            codes::INTERNAL,
            "stripe webhook not configured",
        );
    };

    // 旧实现用 io.LimitReader **截断**而不是报错：超长 body 的签名自然对不上，
    // 于是走到下面同一个 400。照抄这个形状。
    let body = if body.len() > MAX_BODY_BYTES {
        body.slice(0..MAX_BODY_BYTES)
    } else {
        body
    };

    let signature = headers
        .get(SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if let Err(error) = verify_signature(
        &body,
        signature,
        secret,
        SIGNATURE_TOLERANCE_SECONDS,
        Utc::now(),
    ) {
        tracing::warn!(event = "stripe_webhook_bad_signature", reason = %error);
        return bad_request("invalid signature");
    }

    let Ok(event) = serde_json::from_slice::<StripeEvent>(&body) else {
        return bad_request("malformed event");
    };

    if !SETTLING_EVENTS.contains(&event.kind.as_str()) {
        // 非结算类事件也要 200，否则 Stripe 会一直重投这些用不上的通知。
        return (
            StatusCode::OK,
            axum::Json(serde_json::json!({ "received": true, "ignored": event.kind })),
        )
            .into_response();
    }

    let order_id = event
        .data
        .object
        .metadata
        .get("order_id")
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|id| *id != 0)
        .and_then(|id| i64::try_from(id).ok());
    let Some(order_id) = order_id else {
        return bad_request("event missing metadata.order_id");
    };

    if let Err(error) =
        super::payment::settle_payment_order(&state.pg, &state.ledger, order_id).await
    {
        tracing::warn!(event = "stripe_webhook_settle_failed", order_id, error = %error);
        // 500 → Stripe 重投。幂等结算让重投是安全的。
        return internal("settlement failed");
    }

    (
        StatusCode::OK,
        axum::Json(serde_json::json!({ "received": true })),
    )
        .into_response()
}
