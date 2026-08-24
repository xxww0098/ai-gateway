//! The pre-flight's pure decisions: what is billable, what it costs, what the
//! quota allows, and which identifiers a request carries.
//!
//! Split out of `hold.rs` for the 1,000-line ceiling, but the seam is a real
//! one: nothing here touches Redis, Postgres or the network, so every rule
//! below is testable as a function of its arguments. The I/O ordering — reject
//! before reserving — stays next to the middleware in `hold.rs`.

use axum::http::{HeaderMap, Method};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use gw_ledger::ClientTraceId;

use super::{BillingPeek, TRACE_HEADER};
use crate::access::is_proxy_path;
use crate::ports::{Id, PricingCalculator, SubscriptionQuota};

// ---------------------------------------------------------------- pure logic

/// 一个请求要不要走计费 preflight。
///
/// 路径集合来自 [`crate::access::is_proxy_path`]，与鉴权层共用，
/// 所以一条路由**不可能**在没被鉴权的情况下被计费。
///
/// # 三个零成本端点已被移出计费范围（本轮修复，用户已批准）
///
/// `GET /v1/models`、`GET /v1/models/{model}`、`POST /v1/messages/count_tokens`
/// 此前是**按 LLM 价格收钱的**。三者都会带着「没有 usage 信封」抵达结算：
///
/// * 两条 catalogue 读落到 usage 解析的默认分支；
/// * `count_tokens` 命中它的 `/messages` 分支，但 Anthropic 的回复是裸
///   `{"input_tokens": N}`，没有 `usage` 包装，所以 usage 解析器什么也找不到，
///   报 `present = false`。
///
/// 「usage 缺失 + 非 strict」= fallback 结算，于是每次调用被收
/// `max(ActiveHoldAmount, Estimate(model, stream = true, rate_mult))`。
/// 按发布配置（`default_price_per_1k_tokens: 0.001`、`estimatedTokens = 1000`），
/// 一次 catalogue 读要收租户约 $0.004；`count_tokens` 带着真实模型名，
/// 按那个模型的费率计价，**可能贵得多**。而上游对这三者都收 0
/// —— Anthropic 的 token 计数是免费的，catalogue 读根本不出网（纯 DB 读）。
///
/// 此前不敢改的理由是「Go parity：改了会漂移成没人能与移植 bug 区分的差异」。
/// **这个理由已经被证伪**：`docs/relay-surface-plan.md` 证据 C 显示 Go 侧 149 条
/// 路由里 `/v1` 与 `/v1beta` 的匹配数为 **0** —— 整个 `/v1` 面来自先前 SDK 的
/// Builder，根本不在 Go 权威参照内，不存在 A/B 对账时被误判的风险。
///
/// # 这改的是「计费范围」，不是「计费语义」
///
/// Hold / Settle / Release 三段式、partial-debit shortfall、strict-usage-metadata
/// 模式 —— 签名与语义一行未动。变的只是**哪些路径进入这条管线**。
/// 这是 `CONTRACT.md` 硬约束「计费语义不变」允许的那一半。
///
/// `GET /v1/usage` 同样不计费：全部 GET 已被排除，它只读账本与鉴权元数据。
pub fn is_billable(method: &Method, path: &str) -> bool {
    is_proxy_path(path)
        && !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
        && !path.ends_with("/count_tokens")
}

/// Conservative worst case used by the balance gate:
/// `max(hold, EstimateWithMaxTokens, Estimate(stream = true))`.
///
/// `EstimateWithMaxTokens` tightens the bound when the client supplied a cap;
/// the streaming estimate guards the case where the cap is absent or absurd.
/// The upper-bound computation for the balance gate.
pub fn preflight_upper_bound(
    calc: &dyn PricingCalculator,
    model: &str,
    max_tokens: i64,
    stream: bool,
    rate_mult: f64,
    hold_amount: f64,
) -> f64 {
    let with_max = calc.estimate_with_max_tokens(model, max_tokens, stream, rate_mult);
    let streaming = calc.estimate(model, true, rate_mult);
    hold_amount.max(with_max).max(streaming)
}

/// 从 peek 一次算出预扣额与 floor 上界，避免 middleware 重复调 estimator。
pub fn compute_reservation(
    peek: &BillingPeek,
    rate_mult: f64,
    calc: &dyn PricingCalculator,
) -> (f64, f64) {
    let hold_amount = calc.estimate_with_tokens(
        &peek.price_key,
        peek.input_tokens,
        peek.max_tokens,
        peek.stream,
        rate_mult,
    );
    let upper_bound = preflight_upper_bound(
        calc,
        &peek.price_key,
        peek.max_tokens,
        peek.stream,
        rate_mult,
        hold_amount,
    );
    (hold_amount, upper_bound)
}

/// Returns the rejection reason when `estimated` would push any period over its
/// limit.
pub fn evaluate_quota(quota: &SubscriptionQuota, estimated: f64) -> Option<&'static str> {
    let periods = [
        (
            quota.daily_limit_usd,
            quota.daily_usage_usd,
            "subscription daily quota exceeded",
        ),
        (
            quota.weekly_limit_usd,
            quota.weekly_usage_usd,
            "subscription weekly quota exceeded",
        ),
        (
            quota.monthly_limit_usd,
            quota.monthly_usage_usd,
            "subscription monthly quota exceeded",
        ),
    ];
    for (limit, used, reason) in periods {
        if let Some(limit) = limit
            && used + estimated > limit
        {
            return Some(reason);
        }
    }
    None
}

/// Zeroes any period counter whose reset boundary has passed and advances that
/// boundary. Returns whether anything changed.
///
/// `pub` so a [`SubscriptionQuotaStore`] implementation can apply the identical
/// rotation inside its `SELECT ... FOR UPDATE` transaction instead of
/// re-deriving the rule in SQL.
pub fn rotate_counters(quota: &mut SubscriptionQuota, now: DateTime<Utc>) -> bool {
    let mut dirty = false;
    if let Some(at) = quota.daily_reset_at
        && now > at
    {
        quota.daily_usage_usd = 0.0;
        quota.daily_reset_at = Some(next_daily_reset_after(now));
        dirty = true;
    }
    if let Some(at) = quota.weekly_reset_at
        && now > at
    {
        quota.weekly_usage_usd = 0.0;
        quota.weekly_reset_at = Some(next_weekly_reset_after(now));
        dirty = true;
    }
    if let Some(at) = quota.monthly_reset_at
        && now > at
    {
        quota.monthly_usage_usd = 0.0;
        quota.monthly_reset_at = Some(next_monthly_reset_after(now));
        dirty = true;
    }
    dirty
}

fn midnight(date: NaiveDate) -> DateTime<Utc> {
    date.and_hms_opt(0, 0, 0)
        .expect("midnight is always a valid time")
        .and_utc()
}

/// Next UTC midnight strictly after `t`.
pub fn next_daily_reset_after(t: DateTime<Utc>) -> DateTime<Utc> {
    midnight(t.date_naive() + chrono::Duration::days(1))
}

/// Next UTC Monday 00:00 strictly after `t` (ISO weeks start on Monday),
/// including the "today is Monday midnight counts as past" rule.
pub fn next_weekly_reset_after(t: DateTime<Utc>) -> DateTime<Utc> {
    let day = t.date_naive();
    let iso = i64::from(day.weekday().number_from_monday()); // Mon=1 .. Sun=7
    midnight(day + chrono::Duration::days(8 - iso))
}

/// First day of the next UTC month at 00:00.
pub fn next_monthly_reset_after(t: DateTime<Utc>) -> DateTime<Utc> {
    let d = t.date_naive();
    let (year, month) = if d.month() == 12 {
        (d.year() + 1, 1)
    } else {
        (d.year(), d.month() + 1)
    };
    midnight(NaiveDate::from_ymd_opt(year, month, 1).expect("first of month is always valid"))
}

/// Maps a model name onto the circuit-breaker key (NOT the dispatch registry —
/// 派发用的是 `gw_relay::endpoint::upstream::select` 的四级链，见
/// [`crate::routes::select_upstreams`]).
pub fn infer_provider(model: &str) -> Option<&'static str> {
    // 不分配：热路径上每个请求都会走到这里，to_ascii_lowercase 只为几个前缀。
    if starts_ignore_ascii(model, "gpt-")
        || starts_ignore_ascii(model, "o1")
        || starts_ignore_ascii(model, "o3")
        || starts_ignore_ascii(model, "o4")
    {
        Some("openai")
    } else if starts_ignore_ascii(model, "claude-") {
        Some("anthropic")
    } else if starts_ignore_ascii(model, "gemini-") {
        Some("google")
    } else if contains_ignore_ascii(model, "codex") {
        Some("codex")
    } else {
        None
    }
}

fn starts_ignore_ascii(hay: &str, prefix: &str) -> bool {
    hay.len() >= prefix.len()
        && hay.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn contains_ignore_ascii(hay: &str, needle: &str) -> bool {
    hay.as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Approximates a token count from a byte length (`ceil(size / 4)`).
pub fn approximate_tokens_from_bytes(size: usize) -> i64 {
    if size == 0 {
        return 0;
    }
    size.div_ceil(4) as i64
}

/// The request's **observability** id: an inbound `X-Trace-ID`, else a
/// process-local one.
///
/// # It is not the money key
///
/// It used to be. A client-supplied header keyed the Redis hold, the settle,
/// the reconcile scan and the usage row, so two callers picking the same value
/// — by replay or by accident — landed on one ledger row. The money key is now
/// minted by the server ([`BillingOperationId::mint`]) and this value only
/// reaches logs, the response header and `usage_logs.request_id`. The return
/// type is [`ClientTraceId`] precisely so it cannot be passed where an
/// operation id belongs.
///
/// # 为什么不是 `Uuid::new_v4()`
///
/// 基线实测（`docs/relay-perf-baseline.md` 热点 #6）：`Uuid::new_v4()` 每请求
/// 一次 `getentropy` 系统调用，占有效 CPU **1.93%**。trace id 不是密码学材料
/// —— 它只需要在**这个进程的生命周期内**不重复，供账本 hold 键与日志关联。
///
/// 所以换成「**进程随机前缀 + 单调原子计数**」：前缀在进程启动时取一次熵
/// （整个进程一次，不是每请求一次），计数器保证同进程内唯一，
/// 前缀保证跨进程/跨副本不碰撞。`getentropy` 的每请求调用次数归 **0**（验收目标 T12）。
///
/// 形状仍是十六进制文本，长度固定，对既有的 `usage_logs.request_id` 兼容。
pub fn client_trace_from(headers: &HeaderMap) -> ClientTraceId {
    headers
        .get(TRACE_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(|| ClientTraceId::new(new_trace_id()), ClientTraceId::new)
}

/// Identifies *which request* an operation is for.
///
/// Compared on a re-hold: same operation id + different fingerprint is a
/// conflict, not an overwrite. It does not have to be unguessable — nothing
/// is authorised by it — only stable for one request and different for two,
/// so it is assembled from facts the pre-flight already has rather than
/// hashing the body a second time.
pub(super) fn request_fingerprint(
    user_id: Id,
    method: &Method,
    path: &str,
    peek: &BillingPeek,
) -> String {
    format!(
        "{user_id}:{method}:{path}:{}:{}:{}:{}",
        peek.model, peek.stream, peek.max_tokens, peek.input_tokens
    )
}

/// 进程级随机前缀。`LazyLock` 保证整个进程**只取一次**熵；
/// 复用 `uuid` 的 v4 生成器只是为了不多引一个熵源，那一次 `getentropy`
/// 摊到进程生命周期上等于零。
static TRACE_PREFIX: std::sync::LazyLock<u64> =
    std::sync::LazyLock::new(|| uuid::Uuid::new_v4().as_u64_pair().0);

/// 同进程内的单调计数器。`Relaxed` 足够：这里只要求**唯一**，不要求跨线程有序。
static TRACE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn new_trace_id() -> String {
    let n = TRACE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{:016x}{n:016x}", *TRACE_PREFIX)
}

/// Client IP: `X-Forwarded-For` (first entry) -> `X-Real-IP` -> nothing.
///
/// The `RemoteAddr` fallback is the caller's job here because axum surfaces the
/// peer address as a `ConnectInfo` extension rather than on the request itself.
pub fn extract_ip_address(headers: &HeaderMap) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let first = xff.split(',').next().unwrap_or("").trim();
        if !first.is_empty() {
            return first.to_owned();
        }
    }
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_owned())
        .unwrap_or_default()
}

/// Reads `Idempotency-Key`, then `X-Idempotency-Key`.
pub fn extract_idempotency_key(headers: &HeaderMap) -> String {
    for name in ["idempotency-key", "x-idempotency-key"] {
        if let Some(v) = headers.get(name).and_then(|v| v.to_str().ok()) {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_owned();
            }
        }
    }
    String::new()
}
