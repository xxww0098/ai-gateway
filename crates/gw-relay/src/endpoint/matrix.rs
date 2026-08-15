//! 3 入口 × 5 上游 = **15 格显式表**，每格三选一：直通 / 转义 / 400。
//!
//! OWNER: worker `relay-endpoints`。
//!
//! 表在 [`cell`] 里，是一个穷尽的 `match`。写成表而不是 if 链，是因为
//! 「哪一格今天还没实现」必须是**一眼可数**的，而不是散在五个 provider 的
//! 实现里靠读代码推断 —— 后者正是缺陷 #1 能藏三个版本没人发现的原因。
//!
//! # 根除的缺陷
//!
//! - **缺陷 #1（S1）**：`/v1/responses` 的 body 被 POST 到上游的
//!   `/v1/chat/completions`。根因是端点由 provider 猜（`openai.rs:103` 只会构造
//!   chat/completions），而不是由入口决定。[`upstream_dialect`] 把这件事收回入口层：
//!   入口 B × {openai, codex} 得到 [`UpstreamDialect::OpenAiResponses`]，
//!   引擎据此拼 `/v1/responses`。三个保留入口之一从 100% 不可用变成直通。
//!
//! # 15 格
//!
//! | | openai | codex | claude | gemini | vertex |
//! | --- | --- | --- | --- | --- | --- |
//! | **A** `/v1/chat/completions` | 直通 | 直通 | 转义 P2 | 转义 P1 | 转义 P1 |
//! | **B** `/v1/responses` | 直通 | 直通 | **400** | **400** | **400** |
//! | **C** `/v1/messages` | 转义 P2 | 转义 P2 | 直通 | 转义 P1 | 转义 P1 |
//!
//! **P0 直通 5 格的正确性是本项目的第一性原理**，其余 10 格都可以后置。

use bytes::Bytes;
use http::StatusCode;

use crate::contract::{Dialect, Surface, Translator, UpstreamDialect};

/// 上游 executor 名。**不是** wire 协议 —— `gemini` 与 `vertex` 是两套鉴权与端点
/// 前缀，wire 协议却是同一个 GenerateContent，所以转义器按
/// [`UpstreamDialect`] 分（2 个覆盖 4 格），而矩阵按这个枚举分（5 列）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    OpenAi,
    Codex,
    Claude,
    Gemini,
    Vertex,
}

impl Provider {
    /// 矩阵的五列。有了它，「15 格是不是真的都覆盖了」才能被测试穷举，
    /// 而不是靠人数格子。
    pub const ALL: [Self; 5] = [
        Self::OpenAi,
        Self::Codex,
        Self::Claude,
        Self::Gemini,
        Self::Vertex,
    ];

    /// `gw-provider` 里 executor 的注册名。`gw-relay` 不依赖 `gw-provider`，
    /// 这个字符串是两边之间唯一的约定。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
            Self::Vertex => "vertex",
        }
    }

    /// executor 注册名 → provider。不认识返回 `None`（配置里写错了渠道映射时
    /// 必须报出来，不能兜底猜一个）。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == name)
    }
}

/// 三个入口。与 [`Surface`] 一一对应，存在的意义是让测试能穷举三行。
pub(crate) const ALL_SURFACES: [Surface; 3] = [
    Surface::OpenAiCompletions,
    Surface::OpenAiResponses,
    Surface::AnthropicMessages,
];

/// 一格的判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    /// 恒等转发：body 原样、query 原样、非逐跳头原样、响应原样。
    Passthrough,
    /// 需要协议翻译，交给 [`crate::translate`]。
    Translate,
    /// 直接 400。**理由是语义会真丢，不是嫌麻烦** —— 见 [`RejectReason`]。
    Reject,
}

/// 15 格表。**穷尽 `match`**：新增入口或新增 provider 时编译器会点名每一格，
/// 不会有一格靠 `_ =>` 静默落到某个默认行为上。
#[must_use]
pub fn cell(surface: Surface, provider: Provider) -> Cell {
    use Provider as P;
    use Surface as S;
    match (surface, provider) {
        // P0 直通 5 格
        (S::OpenAiCompletions, P::OpenAi | P::Codex) => Cell::Passthrough,
        (S::OpenAiResponses, P::OpenAi | P::Codex) => Cell::Passthrough,
        (S::AnthropicMessages, P::Claude) => Cell::Passthrough,
        // P1 转义 4 格：{A, C} × {gemini, vertex} → crate::translate::google
        (S::OpenAiCompletions | S::AnthropicMessages, P::Gemini | P::Vertex) => Cell::Translate,
        // P2 转义 3 格：A×claude、C×{openai, codex} → crate::translate::anthropic
        (S::OpenAiCompletions, P::Claude) => Cell::Translate,
        (S::AnthropicMessages, P::OpenAi | P::Codex) => Cell::Translate,
        // P3 直接 400 三格：B × {claude, gemini, vertex}
        (S::OpenAiResponses, P::Claude | P::Gemini | P::Vertex) => Cell::Reject,
    }
}

/// 上游说哪种 wire 协议。填进 [`crate::UpstreamTarget::dialect`]。
///
/// **缺陷 #1 的根除点**：入口 B 打到 openai / codex 时得到
/// [`UpstreamDialect::OpenAiResponses`]，而不是像今天那样被 provider
/// 一律猜成 chat/completions。端点由入口决定，不由 provider 猜。
///
/// 入口 C 转义到 openai / codex 时目标是 Chat Completions
/// （[`UpstreamDialect::OpenAiChat`]）—— Anthropic Messages 在 Responses API
/// 里没有对应表达，硬翻会掉进和 P3 同一个坑。
#[must_use]
pub fn upstream_dialect(surface: Surface, provider: Provider) -> UpstreamDialect {
    use Provider as P;
    match (surface, provider) {
        (_, P::Claude) => UpstreamDialect::AnthropicMessages,
        (_, P::Gemini | P::Vertex) => UpstreamDialect::GoogleGenerateContent,
        (Surface::OpenAiResponses, P::OpenAi | P::Codex) => UpstreamDialect::OpenAiResponses,
        (_, P::OpenAi | P::Codex) => UpstreamDialect::OpenAiChat,
    }
}

/// 400 的两种理由。两者共用同一个错误信封形状，只是 `message` 不同。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// P3 三格：翻过去会**静默丢语义**。
    ///
    /// Responses API 的核心是有状态的 item 模型 —— `previous_response_id`
    /// 服务端会话续接、`store: true` 服务端留存、`include` 按需回填、
    /// reasoning item 的加密留存。Anthropic 与 Google 都没有对应概念。
    /// 翻过去只能翻文本部分，而客户端会以为 `previous_response_id` 生效了：
    /// **一个静默的跨轮次正确性错误，比一个 400 坏得多。**
    SemanticsWouldBeLost,
    /// P2 三格在转义器落地之前的过渡态：这一格**将来**能翻，现在还没有实现。
    /// 未做之前返回 400 而不是转发 —— 转发过去只会拿一个上游 400，
    /// 而客户端从上游的错误里读不出「这是网关的路由问题」。
    TranslatorUnavailable,
}

/// 一次派发的结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// 恒等转发。`upstream` 决定引擎拼哪个端点。
    Passthrough { upstream: UpstreamDialect },
    /// 交给转义器。`upstream` 决定去 [`crate::translate`] 的哪个模块取
    /// [`crate::Translator`]：[`UpstreamDialect::GoogleGenerateContent`] → `translate::google`，
    /// [`UpstreamDialect::OpenAiChat`] / [`UpstreamDialect::AnthropicMessages`] → `translate::anthropic`。
    ///
    /// 取不到（P2 尚未实现）时，调用方用
    /// [`reject_body`] + [`RejectReason::TranslatorUnavailable`] 回 400，
    /// **不要**把请求原样转发过去。
    Translate { upstream: UpstreamDialect },
    /// HTTP **400**，`content-type: application/json`，body 就是这里的字节。
    /// body 已经是**入口自身方言**的错误信封，调用方原样写出即可。
    ///
    /// **不计费** —— 走 `UsageOutcome::failed()` 的释放路径而不是结算。
    Reject(Bytes),
}

/// 被拒绝时的 HTTP 状态码。三格 P3 与未实现期的三格 P2 共用。
pub const REJECT_STATUS: StatusCode = StatusCode::BAD_REQUEST;

/// 查表 + 备好 400 的字节。这是本模块唯一需要被调用的入口。
///
/// `model` 只用来把模型名写进错误 message（可执行指引），不参与判定。
#[must_use]
pub fn route(surface: Surface, provider: Provider, model: Option<&str>) -> Route {
    let upstream = upstream_dialect(surface, provider);
    match cell(surface, provider) {
        Cell::Passthrough => Route::Passthrough { upstream },
        Cell::Translate => Route::Translate { upstream },
        Cell::Reject => Route::Reject(reject_body(
            surface,
            provider,
            model,
            RejectReason::SemanticsWouldBeLost,
        )),
    }
}

/// 这一格该用哪个 [`Translator`]。[`Cell::Translate`] 之外的格子返回 `None`。
///
/// 本 worker **不实现**转义器本身（那是 `relay-google` / `relay-anthropic` 的活），
/// 只负责按矩阵把请求送到正确的那一个门口。转义器按**上游 wire 协议**分，
/// 所以 gemini 与 vertex 共用同一个实现（它们的 wire 协议是同一个
/// GenerateContent，只是鉴权与端点前缀不同），2 个实现覆盖 P1 四格。
///
/// 返回 `None` 而这一格却是 [`Cell::Translate`] 时，调用方用
/// [`reject_body`] + [`RejectReason::TranslatorUnavailable`] 回 400，
/// **不要**把请求原样转发过去 —— 转发过去只会拿一个上游 400，
/// 而客户端从上游的错误里读不出「这是网关的路由问题」。
#[must_use]
pub fn translator_for(
    surface: Surface,
    upstream: UpstreamDialect,
) -> Option<&'static dyn Translator> {
    use crate::translate::{anthropic as p2, google as p1};
    use UpstreamDialect as U;

    static OPENAI_TO_GOOGLE: p1::OpenAiToGoogle = p1::OpenAiToGoogle;
    static ANTHROPIC_TO_GOOGLE: p1::AnthropicToGoogle = p1::AnthropicToGoogle;
    static OPENAI_TO_ANTHROPIC: p2::OpenAiToAnthropic = p2::OpenAiToAnthropic;
    static ANTHROPIC_TO_OPENAI: p2::AnthropicToOpenAi = p2::AnthropicToOpenAi;

    match (surface, upstream) {
        // P1 四格
        (Surface::OpenAiCompletions, U::GoogleGenerateContent) => Some(&OPENAI_TO_GOOGLE),
        (Surface::AnthropicMessages, U::GoogleGenerateContent) => Some(&ANTHROPIC_TO_GOOGLE),
        // P2 三格
        (Surface::OpenAiCompletions, U::AnthropicMessages) => Some(&OPENAI_TO_ANTHROPIC),
        (Surface::AnthropicMessages, U::OpenAiChat) => Some(&ANTHROPIC_TO_OPENAI),
        _ => None,
    }
}

/// 400 的统一形状：HTTP 400 + **入口自身方言**的错误信封。
///
/// | 入口方言 | 信封 |
/// | --- | --- |
/// | OpenAI | `{"error":{"message":…,"type":"invalid_request_error","code":"unsupported_upstream"}}` |
/// | Anthropic | `{"type":"error","error":{"type":"invalid_request_error","message":…}}` |
///
/// **为什么用入口方言而不是网关自有格式**：客户端 SDK 只会解析它自己那套错误结构。
/// 回一个陌生结构，`@anthropic-ai/sdk` 的 `APIStatusError` 构造拿不到 `error.message`，
/// OpenAI SDK 的 `error.message` 同理 —— 客户端会把它渲染成一个**无字的红叉**，
/// 用户看到一次失败却读不到任何原因。
///
/// `message` 写的是**可执行指引**：模型属于哪个渠道、该改用哪个入口。
/// 建议的入口是从 [`cell`] 反查出来的（[`surfaces_serving`]），所以它不会与矩阵漂移。
#[must_use]
pub fn reject_body(
    surface: Surface,
    provider: Provider,
    model: Option<&str>,
    reason: RejectReason,
) -> Bytes {
    let message = reject_message(surface, provider, model, reason);
    let envelope = match surface.dialect() {
        Dialect::OpenAi => serde_json::json!({
            "error": {
                "message": message,
                "type": "invalid_request_error",
                "code": "unsupported_upstream",
            }
        }),
        Dialect::Anthropic => serde_json::json!({
            "type": "error",
            "error": {
                "type": "invalid_request_error",
                "message": message,
            }
        }),
    };
    // `serde_json::Value` 的序列化不可能失败（没有自定义 Serialize、没有非字符串键）。
    // 兜底分支存在只是为了不在中继路径上 panic。
    serde_json::to_vec(&envelope).map_or_else(|_| Bytes::from_static(b"{}"), Bytes::from)
}

/// 哪些入口能承接这个 provider（[`Cell::Reject`] 之外的都算）。
///
/// 从 [`cell`] 反查，不是第二张手抄表 —— 矩阵改了，指引跟着改。
#[must_use]
pub fn surfaces_serving(provider: Provider) -> Vec<Surface> {
    ALL_SURFACES
        .into_iter()
        .filter(|s| cell(*s, provider) != Cell::Reject)
        .collect()
}

/// 入口 → 路径。[`Surface::from_path`] 的逆映射。
///
/// 正映射住在 `contract.rs`（协调者独占），这里是它唯一的逆。两者一致性由
/// `tests.rs` 的往返测试钉死：`Surface::from_path(path_of(s)) == Some(s)`。
#[must_use]
pub fn path_of(surface: Surface) -> &'static str {
    match surface {
        Surface::OpenAiCompletions => "/v1/chat/completions",
        Surface::OpenAiResponses => "/v1/responses",
        Surface::AnthropicMessages => "/v1/messages",
    }
}

fn reject_message(
    surface: Surface,
    provider: Provider,
    model: Option<&str>,
    reason: RejectReason,
) -> String {
    let model = model.unwrap_or("(未指定)");
    let alternatives: Vec<String> = surfaces_serving(provider)
        .into_iter()
        .filter(|s| *s != surface)
        .map(|s| format!("POST {}", path_of(s)))
        .collect();
    let guidance = if alternatives.is_empty() {
        format!("本网关没有任何入口能承接渠道 {}。", provider.as_str())
    } else {
        format!("请改用 {}。", alternatives.join(" 或 "))
    };
    let why = match reason {
        RejectReason::SemanticsWouldBeLost => concat!(
            "Responses API 的有状态 item 模型（previous_response_id 服务端会话续接、",
            "store 服务端留存、include 按需回填、reasoning item 的加密留存）",
            "在该上游没有对应概念，翻译过去只能翻文本部分，",
            "而客户端会以为跨轮次续接生效了 —— 网关拒绝制造这种静默错误。",
        ),
        RejectReason::TranslatorUnavailable => {
            "本网关尚未实现这两种方言之间的协议转义，转发过去只会拿到一个上游错误。"
        }
    };
    format!(
        "模型 {model} 属于渠道 {channel}，{path} 入口无法承接它：{why}{guidance}",
        channel = provider.as_str(),
        path = path_of(surface),
    )
}

#[cfg(test)]
mod tests;
