//! 入口规范：路径 / 方法 / content-type 的校验，流式判定，以及**唯一一次**请求体解析。
//!
//! OWNER: worker `relay-endpoints`。
//!
//! # 根除的缺陷
//!
//! - **缺陷 #15**（`docs/relay-passthrough-audit.md` S3）：今天同一个 body 的 JSON 被解析
//!   两遍（`hold.rs:866` 与 `routes.rs:632` 各调一次 `parse_body_peek`），流式还有第三遍
//!   （`common.rs:252` 的 `ensure_include_usage`）。一个 900 KB 的 body 解析三遍。
//!   这里 [`RequestSpec::parse`] 是**全链路唯一一次**解析，结果进上下文供所有下游复用
//!   —— 包括 [`crate::endpoint::include_usage`] 需要的「顶层有没有 `stream_options`」，
//!   它搭在同一次解析上，不再单独解一遍。
//! - **`/v1/responses` 的 `max_output_tokens` 没被解析**（`docs/relay-surface-plan.md` §3.2 ⚠️1）：
//!   `parse_body_peek`（`hold.rs:765-775`）只认 `max_tokens` / `max_completion_tokens`，
//!   于是每个 `/v1/responses` 请求的 max_tokens 都是 0，预扣退化成保守估算，过度冻结余额。
//! - **非 JSON content-type 静默降级**（`docs/relay-surface-plan.md` §3.0）：
//!   `parse_body_peek`（`hold.rs:757-762`）在 content-type 不含 `json` 子串时**直接放弃解析
//!   并返回全零 peek**，于是 `model=""`、`stream=false`、`max_tokens=0` —— 请求不被拒，
//!   而是带着空模型名走完整个计费与派发链。这里升级为 400。

use http::{HeaderMap, Method, StatusCode, header};

use super::json_peek::parse_top_fields;
use crate::contract::Surface;

/// 入口校验失败的三种形态。每一种对应一个确定的 HTTP 状态码，调用方不要再猜。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceError {
    /// 路径不是三个入口之一 —— 包括被硬删的 `/v1beta/**`、`/v1/completions`、
    /// `/v1/embeddings`、`POST /v1/models/{model}`。**404，不要兜底猜一个入口**。
    UnknownPath,
    /// 路径认得，方法不对。三个入口全部只接受 `POST`。
    MethodNotAllowed,
    /// content-type 不是 JSON。今天这里是静默降级（见模块 doc），现在是 400。
    UnsupportedMediaType,
}

impl SurfaceError {
    /// 对应的 HTTP 状态码。
    #[must_use]
    pub fn status(self) -> StatusCode {
        match self {
            Self::UnknownPath => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Self::UnsupportedMediaType => StatusCode::BAD_REQUEST,
        }
    }
}

/// 三个入口全部只接受这一个方法。
const ALLOWED_METHOD: Method = Method::POST;

/// 校验入站请求的三件套：路径、方法、请求 content-type。
///
/// 顺序是有意的：**先认路径**（不认识就 404，连方法都不必看），
/// 再认方法，最后认 content-type。反过来会让一个打错路径的 GET 拿到 405，
/// 而 405 意味着「路径存在」，那是在向外泄漏一个不存在的路由。
///
/// content-type 的判定：取 `;` 之前的 media type，去空白、转小写，接受
/// `application/json` 与任意 `+json` 结构化后缀（RFC 6839），其余一律
/// [`SurfaceError::UnsupportedMediaType`]。**缺失 content-type 也算不合格** ——
/// 网关不替客户端编一个（`docs/relay-passthrough-audit.md` §4.2 条 3：
/// 「客户端没给就不该替它编一个」），但入站方向必须把它拒掉而不是静默按零处理。
///
/// # Errors
///
/// 见 [`SurfaceError`] 的三个变体。
pub fn validate(method: &Method, path: &str, headers: &HeaderMap) -> Result<Surface, SurfaceError> {
    let surface = Surface::from_path(path).ok_or(SurfaceError::UnknownPath)?;
    if *method != ALLOWED_METHOD {
        return Err(SurfaceError::MethodNotAllowed);
    }
    if !is_json_media_type(headers.get(header::CONTENT_TYPE)) {
        return Err(SurfaceError::UnsupportedMediaType);
    }
    Ok(surface)
}

fn is_json_media_type(value: Option<&http::HeaderValue>) -> bool {
    let Some(raw) = value.and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let media = raw.split(';').next().unwrap_or_default().trim();
    media.eq_ignore_ascii_case("application/json")
        || media
            .rsplit_once('+')
            .is_some_and(|(_, suffix)| suffix.eq_ignore_ascii_case("json"))
}

/// 网关**必须**解析的字段，以及它们的解析结果。这是网关与请求体之间的**全部**接触面。
///
/// 边界（`docs/relay-surface-plan.md` §3.1–§3.3）：
///
/// | | 字段 |
/// | --- | --- |
/// | **必须解析** | `model`、`stream`、`max_tokens` / `max_completion_tokens` / `max_output_tokens` |
/// | **只看在不在** | `stream_options`（决定要不要定点注入，见 [`super::include_usage`]） |
/// | **绝不碰** | `messages`、`input`、`system`、`instructions`、`tools`、`tool_choice`、`response_format`、`text`、`reasoning`、`previous_response_id`、`store`、`include`、`temperature`、`top_p`、`top_k`、`seed`、`n`、`logprobs`、`stop_sequences`、`thinking`、`metadata`、`user`，以及**任何未来新增字段** |
///
/// 「透传」的定义就是：未列入上表前两行的字段，网关既不读也不写，更不做 schema 校验。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestSpec {
    /// 请求落在哪个入口。
    pub surface: Surface,
    /// 客户端要的模型名，**原样**（不 trim、不小写化、不剥前缀）。
    ///
    /// `None` 有两种成因，都由 [`Self::body_visible`] 区分：body 看不见（流式请求体，
    /// 缺陷 #2 的降级路径），或者 body 看得见但没写 `model`。
    pub model: Option<String>,
    /// 流式判定的**唯一**结果。规则见 [`Self::parse`] 与 [`accept_conflicts_with_body`]。
    pub stream: bool,
    /// 输出上限，三个方言字段收敛成一个。`None` 与 `Some(0)` **必须能分开**：
    /// 前者是「客户端没说」（预扣按估算），后者是「客户端说了 0」。
    pub max_tokens: Option<i64>,
    /// 顶层有没有 `stream_options` **键**。有 → 一个字节都不动（尊重客户端）。
    ///
    /// 判的是**键在不在**，不是值是不是 `null`：客户端写了
    /// `"stream_options": null` 也算写过。再插一个同名键会产出重复键的 JSON ——
    /// 后一个赢是 serde 的行为，不是 JSON 规范的保证，不同上游的解析器不一定同意。
    pub stream_options_present: bool,
    /// body 有没有被成功解析过。`false` 时上面四个字段全是缺省值，**不是**「客户端没写」。
    ///
    /// 计费必须显式处理这一支：走最保守的估算，而**不是**把它当成一个空 body。
    /// 转发不受影响 —— 计费降级，转发不降级。
    pub body_visible: bool,
}

/// 测试用的 serde 对照物。生产路径走 [`parse_top_fields`]，不再把
/// `messages` 整棵扫进 serde。对照测试见 `tests.rs` 的 corpus。
#[cfg(test)]
#[derive(Debug, serde::Deserialize)]
pub(super) struct RawPeek {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub max_tokens: Option<i64>,
    #[serde(default)]
    pub max_completion_tokens: Option<i64>,
    #[serde(default)]
    pub max_output_tokens: Option<i64>,
    #[serde(default, deserialize_with = "present")]
    pub stream_options: Option<serde::de::IgnoredAny>,
}

#[cfg(test)]
fn present<'de, D>(de: D) -> Result<Option<serde::de::IgnoredAny>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(de).map(Some)
}

impl RequestSpec {
    /// **全链路唯一一次**请求体解析（缺陷 #15）。
    ///
    /// `peek` 来自 [`crate::RelayBody::peek`]：`None` 表示请求体是
    /// [`crate::RelayBody::Streaming`]，网关看不见它 —— 此时返回的 spec
    /// [`Self::body_visible`] 为 `false`，**转发照常进行**。
    ///
    /// # 流式判定（三入口统一，`docs/relay-surface-plan.md` §3.4）
    ///
    /// - **规则 S1**：`stream` 只由请求体顶层的 `stream` 字段决定。字段缺失、
    ///   非布尔、或 body 无法解析为 JSON 对象，一律视为 `false`。
    /// - **规则 S2**：`Accept` 头**永不**参与流式判定（见 [`accept_conflicts_with_body`]）。
    ///
    /// 三个入口用的是**同一个** `stream` 字段、**同一段**代码，没有 per-surface 分支
    /// —— `anthropic-messages` 与 `openai-responses` 的判定规则因此必然一致，
    /// 不存在「不一致要写明为什么」的情况。这是有意的：上游（OpenAI / Anthropic / Google）
    /// 自己就是只看 body 里的 `stream` 来决定响应框架的，判据统一才能让转发是恒等变换。
    ///
    /// # 输出上限的回落链
    ///
    /// `max_tokens` → `max_completion_tokens` → `max_output_tokens`，第一个
    /// `Some` 即止。三个入口共用同一条链：入口不同只是字段名不同，语义是同一个。
    #[must_use]
    pub fn parse(surface: Surface, peek: Option<&[u8]>) -> Self {
        let Some(raw) = peek.and_then(parse_top_fields) else {
            return Self {
                surface,
                model: None,
                stream: false,
                max_tokens: None,
                stream_options_present: false,
                body_visible: false,
            };
        };
        Self {
            surface,
            model: raw.model,
            stream: raw.stream.unwrap_or(false),
            max_tokens: raw
                .max_tokens
                .or(raw.max_completion_tokens)
                .or(raw.max_output_tokens),
            stream_options_present: raw.stream_options_present,
            body_visible: true,
        }
    }

    /// 模型名的只读借用，给上游选择链用。
    #[must_use]
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }
}

/// 规则 S3：`Accept` 与 body 冲突时**以 body 为准**，并打一条 `warn!` 供运维定位
/// 客户端配置错误。返回是否冲突（`true` = 已告警）。
///
/// 冲突的两个方向：
/// - `Accept: text/event-stream` 而 body 没说 `stream: true`；
/// - body 说了 `stream: true` 而 `Accept` 只接受 `application/json`。
///
/// **为什么是 body 赢**（`docs/relay-surface-plan.md` §3.4）：body 是要**原样送到上游**
/// 的那个东西，而上游只看 body 的 `stream` 决定响应形状。若按 `Accept` 判定为流式却把
/// `stream: false` 的 body 原样送上去，上游返回一次性 JSON，而网关已经回了
/// `text/event-stream` 头 —— 网关将被迫自己把 JSON 切成 SSE 帧，那就是凭空发明了一次
/// 转义。反过来若按 body 判定，网关的判断与上游的判断**永远一致**，转发始终是恒等变换。
///
/// 同理也**不**改写 body 去迎合 `Accept` —— 那会让「唯一被授权的 body 变异是
/// `stream_options` 定点注入」这条合同破防。
///
/// 日志字段是 `path` / `body_stream` / `accept`；`request_id` 由调用方的
/// tracing span 带入（本 crate 不认识请求 ID，那是 `gw-proxy` 的概念）。
pub fn accept_conflicts_with_body(spec: &RequestSpec, headers: &HeaderMap, path: &str) -> bool {
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let wants_sse = contains_ignore_ascii(accept, "text/event-stream");
    let wants_json = contains_ignore_ascii(accept, "application/json");

    let conflict = if spec.stream {
        wants_json && !wants_sse
    } else {
        wants_sse
    };
    if conflict {
        tracing::warn!(
            path,
            body_stream = spec.stream,
            accept,
            "Accept 与请求体的 stream 字段冲突，以 body 为准（规则 S3）"
        );
    }
    conflict
}

fn contains_ignore_ascii(hay: &str, needle: &str) -> bool {
    hay.as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

#[cfg(test)]
mod tests;
