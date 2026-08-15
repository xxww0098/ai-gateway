//! P2 三格：`openai-completions × claude`、`anthropic-messages × {openai, codex}`。
//!
//! OWNER: worker `relay-anthropic`（本文件 + `anthropic/**`）。
//!
//! 真实需求：Claude Code 指向 OpenAI 上游、Cursor 指向 Claude 上游。
//!
//! # 两个转义器覆盖三格
//!
//! | 格 | 入口 | 上游 | 实现 |
//! | --- | --- | --- | --- |
//! | `A×claude` | `openai-completions` | `claude` | [`OpenAiToAnthropic`] |
//! | `C×openai` | `anthropic-messages` | `openai` | [`AnthropicToOpenAi`] |
//! | `C×codex` | `anthropic-messages` | `codex` | [`AnthropicToOpenAi`]（codex 与 openai 共用 OpenAI Chat wire） |
//!
//! `codex` 与 `openai` 之所以共用一个转义器，理由与 `google.rs` 里 gemini/vertex
//! 共用一个转义器完全相同：**分层依据是上游 wire 协议，不是账号 provider 名**。
//!
//! # 这里根除的是审计报告里的哪几条
//!
//! - **缺陷 #1（S1）**：端点由 provider 猜。本模块只做 body 转义，
//!   一个字节的 URL 都不碰 —— 上游 URL 由 [`crate::engine`] 用 origin + 入站 path 拼。
//! - **缺陷 #4（S2）**：`ensure_include_usage()` 为了拿 usage 去改客户端请求的结构。
//!   走转义路径时 usage 由 [`crate::StreamTranslator::usage`] 从**已经解析过一次**的
//!   上游帧里顺出来，请求体一个字节都不为计费而改。
//! - **缺陷 #16（S3）**：`StreamUsageBuffer` 对每个 chunk 全量 memcpy。
//!   转义路径不再挂 [`crate::UsageProbe`]，那 2× 全流量的额外内存带宽直接消失。
//!
//! # 转义可以丢字段，绝不能丢 usage
//!
//! 两个方向的 [`crate::StreamTranslator::usage`] 都必须在流结束时给出非空计数，
//! 否则计费落 fallback，违反「计费语义不变」这条硬约束。取数点：
//!
//! | 上游 | `input_tokens` | `output_tokens` |
//! | --- | --- | --- |
//! | Anthropic | **首帧** `message_start` 的 `usage.input_tokens` | **末帧** `message_delta` 的 `usage.output_tokens` |
//! | OpenAI | 末帧 `usage.prompt_tokens` | 末帧 `usage.completion_tokens` |
//!
//! # 翻不动的字段：逐个判定的结果
//!
//! 判定只有三种结局，**没有第四种「静默丢一个有语义的字段然后假装成功」**。
//!
//! ## 判 [`crate::TranslateError::Unsupported`]（→ 400）
//!
//! | 字段 | 方向 | 为什么不能静默丢 |
//! | --- | --- | --- |
//! | `n > 1` | OpenAI→Anthropic | Anthropic 根本没有这个概念。静默当成 `n=1`，客户端会以为拿到了 5 个候选里的第一个，实际只生成了 1 个 —— 一个看不见的正确性错误 |
//! | `logprobs` / `top_logprobs` | OpenAI→Anthropic | Anthropic 不返回 logprob。客户端拿不到它要的那半个响应，却收到 200 |
//! | `response_format`（非 `text`） | OpenAI→Anthropic | JSON mode / structured output 是一个**输出格式保证**。丢了就是保证没了，客户端 `json.loads()` 炸在下游 |
//! | `seed` | OpenAI→Anthropic | 可复现性保证，Anthropic 无对应物 |
//! | `frequency_penalty` / `presence_penalty`（非 0） | OpenAI→Anthropic | 采样分布被改。0 是恒等值，才允许丢 |
//! | `logit_bias`（非空） | OpenAI→Anthropic | 同上，且它能硬禁某个 token |
//! | `reasoning_effort` | OpenAI→Anthropic | Anthropic 的 extended thinking 要显式 `budget_tokens`，且要求 `max_tokens > budget_tokens`。猜一个预算等于替客户端做决定 |
//! | `thinking` | Anthropic→OpenAI | 同上反向。OpenAI Chat 没有可对齐的预算旋钮 |
//! | `thinking` / `redacted_thinking` content block | Anthropic→OpenAI | 块上的 `signature` 是多轮 extended thinking 的凭据，过一遍 OpenAI 就回不来了 |
//! | `top_k` | Anthropic→OpenAI | OpenAI Chat 没有 top-k，丢了采样分布就变了 |
//! | `document` content block | Anthropic→OpenAI | OpenAI Chat 的 `file` part 走 file-id，不吃内联 PDF |
//! | 服务端工具（`computer_*` / `web_search_*` / `bash_*` / `text_editor_*`） | Anthropic→OpenAI | 上游根本不会执行它 |
//! | **任何未知顶层键** | 双向 | 不认识就说不认识。放行会被上游 400，丢掉会静默改语义 |
//!
//! ## 判「可翻」
//!
//! `system` role ↔ 顶层 `system`、`tool_calls` ↔ `tool_use`、`tool_call_id` ↔ `tool_result`、
//! `stop` ↔ `stop_sequences`、`parallel_tool_calls:false` ↔ `disable_parallel_tool_use:true`、
//! `user` ↔ `metadata.user_id`、data-URI 的 `image_url` ↔ `image.source.base64`、
//! `refusal` → text block（模型确实产出了这段文本，只是被 OpenAI 放在另一个字段里）。
//!
//! ## 判「纯装饰，允许静默丢」—— 全部列在这里，不留暗账
//!
//! | 字段 | 方向 | 判定依据 |
//! | --- | --- | --- |
//! | `cache_control` | Anthropic→OpenAI | 它是**成本提示，不是语义**。OpenAI 自动做前缀缓存并在 `prompt_tokens_details.cached_tokens` 里如实返回，客户端的意图由上游自动兑现 |
//! | `stream_options` | OpenAI→Anthropic | 它只是在讨要 usage。走转义路径 usage 由转义器自己产，讨不讨都给 |
//! | `store` / `metadata`（OpenAI 侧） | OpenAI→Anthropic | 上游侧的留存标签，不参与生成 |
//! | `service_tier: "auto"` | OpenAI→Anthropic | `auto` 是恒等值。非 `auto` 判 `Unsupported` |
//! | `n == 1`、`frequency_penalty == 0` 等恒等值 | OpenAI→Anthropic | 恒等值丢了没有可观测差别 |
//! | `image_url.detail` | OpenAI→Anthropic | Anthropic 不吃分辨率提示，图片照样送到 |
//! | `tool_result.is_error` | Anthropic→OpenAI | OpenAI 的 tool 消息没有这个字段，而**错误信息本身在 `content` 里原样送达** —— 模型照样看得见「这次工具调用失败了」。为它 400 会把每一次失败的工具调用都变成一个 400 |
//! | `metadata` 里 `user_id` 之外的键 | Anthropic→OpenAI | `user_id` 已映射成 OpenAI 的 `user`；其余是滥用归因用的遥测标签，不参与生成 |
//! | `thinking_delta` / `signature_delta` 流事件 | Anthropic→OpenAI | 请求方向已经把 `reasoning_effort` 判成 `Unsupported`，上游不会开 thinking；万一开了也没有 OpenAI 侧落点 |
//!
//! # 两条已知的、有意为之的偏差
//!
//! 1. **usage 挂在带 `finish_reason` 的那一帧上，而不是另起一帧 `choices: []`。**
//!    [`crate::Translator::stream_translator`] 拿不到请求体，因此看不见
//!    `stream_options.include_usage`。缺陷 #4 记在案的客户端崩法正是那一帧空
//!    `choices` —— 手写 `chunk["choices"][0]` 的客户端在它上面抛 `IndexError`。
//!    挂在末帧上是**纯增量**：SDK 读 `chunk.usage` 照样读得到，手写客户端不炸。
//! 2. **`message_start` 的 `usage.input_tokens` 是 0，真值在 `message_delta` 里。**
//!    OpenAI 的 usage 在**末帧**，而 Anthropic 客户端要求 `message_start` 是**首帧** ——
//!    这两条不可能同时满足。所以 `message_delta.usage` 同时带 `input_tokens` 与
//!    `output_tokens`（真实 Anthropic 也这么发），客户端累加后拿到的总数是对的。
//!    网关自己的计费不受影响：它读 [`crate::StreamTranslator::usage`]，不读帧。

use bytes::Bytes;
use serde_json::{Map, Value};

use crate::contract::TranslateError;

mod to_anthropic;
mod to_openai;

pub use to_anthropic::{DEFAULT_MAX_TOKENS, OpenAiToAnthropic};
pub use to_openai::AnthropicToOpenAi;

// ============================================================ 共享的 JSON 入口

/// 把请求/响应体解析成顶层 JSON 对象。
///
/// 非对象一律 [`TranslateError::Malformed`] —— 三个入口的 body 在协议上都必须是
/// 一个 JSON object，数组或裸标量不是「我们不支持」，是客户端发错了。
fn parse_object(body: &[u8]) -> Result<Map<String, Value>, TranslateError> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(other) => Err(TranslateError::Malformed(format!(
            "顶层必须是 JSON object，收到 {}",
            kind_of(&other)
        ))),
        Err(e) => Err(TranslateError::Malformed(e.to_string())),
    }
}

/// 同上，但用于**上游响应** —— 上游给了不认识的形状是上游或转义器的 bug，
/// 不是客户端的错，所以错误类型不同（[`TranslateError::UpstreamShape`]）。
fn parse_upstream_object(body: &[u8]) -> Result<Map<String, Value>, TranslateError> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(other) => Err(TranslateError::UpstreamShape(format!(
            "顶层必须是 JSON object，收到 {}",
            kind_of(&other)
        ))),
        Err(e) => Err(TranslateError::UpstreamShape(e.to_string())),
    }
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// `Some` 当且仅当键存在且不是 `null`。
///
/// 客户端 SDK 普遍把「没设」序列化成 `null`（Python 侧 `NOT_GIVEN` 之外的
/// 默认值、Go 侧的 `omitempty` 漏网），把 `null` 当成「设了一个值」会让一堆
/// 本可放行的请求撞上 `Unsupported`。
fn present<'a>(src: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    src.get(key).filter(|v| !v.is_null())
}

/// `value[key]` 当字符串读，读不到就是空串。
///
/// 流式路径专用：一帧里少一个可选字段不该打断整条在途的流（缺陷 #6 的反面 ——
/// 该失败时必须失败，不该失败时也绝不能失败）。
fn str_at<'a>(value: Option<&'a Value>, key: &str) -> &'a str {
    value
        .and_then(|v| v.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn to_bytes(value: &Value) -> Result<Bytes, TranslateError> {
    serde_json::to_vec(value)
        .map(Bytes::from)
        .map_err(|e| TranslateError::Malformed(e.to_string()))
}

// ================================================================= SSE 帧切分

/// SSE 事件切分器：把**任意分块**的上游字节流切成完整事件。
///
/// [`crate::StreamTranslator::push`] 的契约是「喂入上游的一个 SSE 帧」，但网络
/// 分块与 SSE 事件边界没有任何关系 —— 一个 `content_block_delta` 完全可能横跨两个
/// TCP 段。这里内部缓一个半事件，`push` 只处理**已经完整**的事件，剩下的留到下次。
/// 于是「一次喂一帧」和「一次喂半帧」两种调用方式都对。
#[derive(Debug, Default)]
struct SseSplit {
    buf: Vec<u8>,
}

impl SseSplit {
    /// 吃进一段字节，吐出其中所有**完整**的 SSE 事件（不含事件间的空行）。
    fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some((end, skip)) = find_event_end(&self.buf) {
            let event: Vec<u8> = self.buf.drain(..end).collect();
            self.buf.drain(..skip);
            if !event.is_empty() {
                out.push(event);
            }
        }
        out
    }

    /// 流结束时缓冲区里剩下的那半个事件。上游正常收尾时通常只剩换行，返回 `None`。
    fn flush(&mut self) -> Option<Vec<u8>> {
        let rest = std::mem::take(&mut self.buf);
        rest.iter()
            .any(|b| !matches!(b, b'\r' | b'\n'))
            .then_some(rest)
    }
}

/// 找到第一个事件分隔符（空行）。返回 `(事件内容长度, 分隔符长度)`。
fn find_event_end(buf: &[u8]) -> Option<(usize, usize)> {
    for i in 0..buf.len() {
        if buf[i] == b'\n' && buf.get(i + 1) == Some(&b'\n') {
            return Some((i, 2));
        }
        if buf[i] == b'\r' && buf[i + 1..].starts_with(b"\n\r\n") {
            return Some((i, 4));
        }
    }
    None
}

/// 取出一个 SSE 事件里的 `data:` 载荷（多行 `data:` 按 SSE 规范用 `\n` 拼接）。
///
/// 没有 `data:` 字段返回 `None` —— 注释帧（`: keep-alive`）与只有 `event:` 的帧
/// 都走这条路，调用方据此跳过。
fn sse_data(event: &[u8]) -> Option<Vec<u8>> {
    let mut data: Option<Vec<u8>> = None;
    for line in event.split(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(rest) = line.strip_prefix(b"data:") else {
            continue;
        };
        // SSE 规范：字段值前的**一个**空格是分隔符，不是数据。
        let rest = rest.strip_prefix(b" ").unwrap_or(rest);
        let slot = data.get_or_insert_with(Vec::new);
        if !slot.is_empty() {
            slot.push(b'\n');
        }
        slot.extend_from_slice(rest);
    }
    data
}

// ================================================================= SSE 帧构造

/// Anthropic 的 SSE 帧：`event:` 与 `data:` **都要**。
///
/// Anthropic 官方 SDK 按 `event:` 行分派事件类型，只发 `data:` 会让它把每一帧
/// 都当成未知事件丢掉 —— 表现为「流跑完了但一个字都没显示」。
fn anthropic_frame(event: &str, data: &Value) -> Bytes {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(b"event: ");
    out.extend_from_slice(event.as_bytes());
    out.extend_from_slice(b"\ndata: ");
    // Value 序列化不会失败（不存在非字符串 map key / 非有限浮点的来源）。
    serde_json::to_writer(&mut out, data).unwrap_or_default();
    out.extend_from_slice(b"\n\n");
    Bytes::from(out)
}

/// OpenAI 的 SSE 帧：只有 `data:`，没有 `event:`。
fn openai_frame(data: &Value) -> Bytes {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(b"data: ");
    serde_json::to_writer(&mut out, data).unwrap_or_default();
    out.extend_from_slice(b"\n\n");
    Bytes::from(out)
}

/// OpenAI 流的终止帧。**必须最后且只有一次**。
const OPENAI_DONE: &[u8] = b"data: [DONE]\n\n";

#[cfg(test)]
mod tests;
