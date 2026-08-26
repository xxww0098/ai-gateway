//! [`crate::UsageProbe`] 的实现 —— 旁路 usage 取数。
//!
//! OWNER: worker `relay-core`。
//!
//! # 根除缺陷 #16
//!
//! 今天 `StreamUsageBuffer` 对每个 chunk 全量 memcpy（tail 每字节必拷 + head 前
//! 32 KiB 再拷一份 + 每 64 KiB 一次 ≤64 KiB 的 memmove ≈ **2× 全流量的额外内存
//! 带宽**，峰值常驻 96 KiB × 并发流数）。
//!
//! 这里用**增量行解析**：SSE 是行式的，一帧里完整的行**零拷贝**地借出去解析，
//! 只有跨帧的那个尾巴（半行）会被复制一次。稳态额外内存 = `O(单行)`，
//! 与流量无关；额外拷贝 ≈ 每帧至多一行，而不是每字节一次。
//!
//! head 窗口整个消失了：增量解析器看得到**每一帧**，Anthropic 的
//! `message_start`（`input_tokens` 在首帧）自然被吃到。
//! [`crate::UsageProbe::needs_head`] 因此退化成一个**说明性**信号 ——
//! [`UsageShape::Anthropic`] 返回 `true` 以如实描述「我要首帧」，
//! 但没有任何人为此付出头窗口的拷贝。
//!
//! # 「缺失」与「零」是两件事
//!
//! [`crate::UsageProbe::finish`] 返回 `None` 表示**上游根本没给** usage ——
//! 计费据此走 fallback / strict 分支。`Some(RelayUsage { input_tokens: Some(0), .. })`
//! 表示上游明确给了 0。两者绝不能合并：合并了 `billing.strict_usage_metadata_mode`
//! 就没有判据了。

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use serde::Deserialize;

use crate::contract::{RelayUsage, UsageProbe};

/// 单行上限。超过就丢弃这一行直到下一个 `\n`。
///
/// 上游字节是信任边界外的输入：没有这个闸门，一个永远不发 `\n` 的上游能把
/// `pending` 撑到 OOM。usage 所在的 `data:` 行实测都在几百字节量级，
/// 这个闸门只会砍掉本来就解析不了的东西。
const MAX_LINE: usize = 256 * 1024;

/// 非流式 JSON body 的累积上限。
///
/// 非流式的 usage 在整份文档的末尾，所以 probe 必须攒齐才解析得动 ——
/// 但这是**旁路**的攒，不在回写路径上（中继早就逐帧发给客户端了）。
/// 超过就放弃解析、计费落 fallback，而不是无上限吃内存。
/// 真实的非流式 LLM 响应远在这个量级之下。
const MAX_JSON: usize = 8 * 1024 * 1024;

/// 上游帧的形状。**不是** provider 名 —— `gemini` 与 `vertex` 的 wire 协议同构，
/// 共用 [`UsageShape::Google`]；`openai` 与 `codex` 共用 [`UsageShape::OpenAi`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageShape {
    /// OpenAI：Chat Completions 的 usage 在末帧顶层；Responses API 的
    /// `response.completed` 则把 usage 放在 `response.usage`。
    OpenAi,
    /// Anthropic：输入与缓存明细在**首帧** `message_start`，
    /// `output_tokens` 在**末帧** `message_delta`。
    Anthropic,
    /// Google GenerateContent：`usageMetadata` 在末帧，且是**累计值**。
    Google,
}

/// 旁路 usage probe：增量行解析，稳态零字节拷贝。
///
/// 同一个类型同时吃流式（SSE 行）与非流式（完整 JSON body）——
/// 非流式时中继把 body 的数据帧依次交给 `observe`，probe 在旁路有界累积。
pub struct SseUsageProbe {
    shape: UsageShape,
    /// 跨帧的半行。稳态长度 = `O(单行)`。
    pending: Vec<u8>,
    /// 当前这一行已经超长，丢弃到下一个 `\n` 为止；
    /// 非流式路径上复用为「JSON 已超 [`MAX_JSON`]，放弃解析」。
    discarding: bool,
    /// 非流式 JSON body 的旁路累积。`None` = 还没认定是 JSON。
    json: Option<Vec<u8>>,
    /// 是否已经看到第一个非空白帧并完成 JSON/SSE 判型。
    classified: bool,
    tally: RelayUsage,
    /// 有没有看到过**任何**一个 usage 字段。`false` → `finish()` 返回 `None`。
    seen: bool,
    sink: UsageHandle,
}

impl SseUsageProbe {
    /// 建一个 probe 和它的取数句柄。
    ///
    /// [`crate::Relay::relay`] 吃掉 `Box<dyn UsageProbe>` 之后没有返回通道，
    /// 所以结果走这个句柄：中继在流结束（正常、失败、或客户端断开）时调
    /// [`UsageProbe::finish`] 恰好一次，值同时写进句柄。
    #[must_use]
    pub fn new(shape: UsageShape) -> (Self, UsageHandle) {
        let sink = UsageHandle::default();
        let probe = Self {
            shape,
            pending: Vec::new(),
            discarding: false,
            json: None,
            classified: false,
            tally: RelayUsage::default(),
            seen: false,
            sink: sink.clone(),
        };
        (probe, sink)
    }

    /// 当前跨帧缓冲的字节数。
    ///
    /// 这是缺陷 #16 的**可观测量**：不管流过多少字节，它都只在一行的量级上。
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.pending.len()
    }

    /// 非流式 JSON 的有界累积。超过 [`MAX_JSON`] 就放弃，计费落 fallback；
    /// 客户端回写不受影响。
    fn accumulate_json(&mut self, chunk: &[u8]) {
        if self.discarding {
            return;
        }
        let current = self.json.as_ref().map_or(0, Vec::len);
        if current.saturating_add(chunk.len()) > MAX_JSON {
            self.json = Some(Vec::new());
            self.discarding = true;
            return;
        }
        self.json
            .get_or_insert_with(|| Vec::with_capacity(chunk.len()))
            .extend_from_slice(chunk);
    }

    /// 一行走完了（遇到 `\n`）。
    fn end_line(&mut self, line: &[u8]) {
        if self.discarding {
            self.discarding = false;
            self.pending.clear();
            return;
        }
        if self.pending.is_empty() {
            // 整行都在这一帧里 —— **零拷贝**，直接借切片。
            self.take_line(line);
            return;
        }
        self.pending.extend_from_slice(line);
        let buf = std::mem::take(&mut self.pending);
        self.take_line(&buf);
        // 把容量还回去，下一次跨帧不用重新分配。
        self.pending = buf;
        self.pending.clear();
    }

    /// 帧尾的半行，留到下一帧。
    fn hold(&mut self, tail: &[u8]) {
        if tail.is_empty() || self.discarding {
            return;
        }
        if self.pending.len().saturating_add(tail.len()) > MAX_LINE {
            self.pending.clear();
            self.discarding = true;
            return;
        }
        self.pending.extend_from_slice(tail);
    }

    /// SSE 语义：只有 `data:` 行带载荷。`event:` / `id:` / `retry:` / 注释行
    /// （`:` 开头）/ 空行一律跳过。
    fn take_line(&mut self, line: &[u8]) {
        let line = trim(line);
        let Some(rest) = line.strip_prefix(b"data:") else {
            return;
        };
        let payload = trim(rest);
        if payload.is_empty() || payload == b"[DONE]" {
            return;
        }
        self.absorb(payload);
    }

    /// 解析一段 JSON，命中就并进 tally。
    fn absorb(&mut self, json: &[u8]) {
        let parsed = match self.shape {
            UsageShape::OpenAi => parse_openai(json),
            UsageShape::Anthropic => parse_anthropic(json),
            UsageShape::Google => parse_google(json),
        };
        let Some(found) = parsed else { return };
        self.seen = true;
        // 后到的帧是权威的。Anthropic 首尾分账时，后帧缺失的列由旧值保留。
        overwrite(&mut self.tally.input_tokens, found.input_tokens);
        overwrite(&mut self.tally.output_tokens, found.output_tokens);
        overwrite(&mut self.tally.cached_tokens, found.cached_tokens);
        overwrite(&mut self.tally.reasoning_tokens, found.reasoning_tokens);
    }
}

impl UsageProbe for SseUsageProbe {
    fn needs_head(&self) -> bool {
        matches!(self.shape, UsageShape::Anthropic)
    }

    fn observe(&mut self, frame: &Bytes) {
        let bytes = frame.as_ref();

        if self.json.is_some() {
            self.accumulate_json(bytes);
            return;
        }

        // 空白-only 的首帧不参与判型：某些代理会先吐 CRLF。此前它会把随后真正
        // 以 `{` 开头的 JSON 错判成 SSE，导致每个非流式请求都落 fallback。
        if !self.classified {
            let first = trim(bytes);
            if first.is_empty() {
                return;
            }
            self.classified = true;
            if first.first() == Some(&b'{') {
                self.accumulate_json(bytes);
                return;
            }
        }

        let mut rest = bytes;
        while let Some(idx) = rest.iter().position(|&b| b == b'\n') {
            let (line, tail) = rest.split_at(idx);
            rest = &tail[1..];
            self.end_line(line);
        }
        self.hold(rest);
    }

    fn finish(self: Box<Self>) -> Option<RelayUsage> {
        let mut me = *self;
        // 非流式：整份 JSON 攒齐了才解析得动。超限时 `discarding` 为 true，
        // 不再尝试解析空缓冲。
        if !me.discarding
            && let Some(buf) = me.json.take()
        {
            me.absorb(&buf);
        }
        let Self {
            tally, seen, sink, ..
        } = me;
        let outcome = seen.then_some(tally);
        sink.set(outcome.clone());
        outcome
    }
}

/// [`SseUsageProbe`] 的取数句柄。`Clone` 是 `Arc` 克隆。
#[derive(Debug, Clone, Default)]
pub struct UsageHandle(Arc<Mutex<Option<Option<RelayUsage>>>>);

impl UsageHandle {
    /// 三态，**「缺失」与「零」必须能分开**：
    ///
    /// - `None` —— 流还没结束，`finish()` 没被调过；
    /// - `Some(None)` —— 结束了，上游没给 usage；
    /// - `Some(Some(u))` —— 结束了，上游给了。
    #[must_use]
    pub fn get(&self) -> Option<Option<RelayUsage>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn set(&self, outcome: Option<RelayUsage>) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
    }
}

/// 后者非 `None` 就覆盖。`Some(0)` 是一个**有效值**，会覆盖掉 `None`。
fn overwrite(slot: &mut Option<i64>, found: Option<i64>) {
    if found.is_some() {
        *slot = found;
    }
}

fn trim(mut s: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = s {
        if first.is_ascii_whitespace() {
            s = rest;
        } else {
            break;
        }
    }
    while let [rest @ .., last] = s {
        if last.is_ascii_whitespace() {
            s = rest;
        } else {
            break;
        }
    }
    s
}

// ============================================================= 三种帧形状

#[derive(Deserialize)]
struct OpenAiEnvelope {
    usage: Option<OpenAiUsage>,
    /// Responses API 的终局 SSE：`response.completed.response.usage`。
    response: Option<OpenAiResponse>,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    // Chat Completions
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    prompt_tokens_details: Option<CachedDetails>,
    completion_tokens_details: Option<ReasoningDetails>,
    // Responses API
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    input_tokens_details: Option<CachedDetails>,
    output_tokens_details: Option<ReasoningDetails>,
}

#[derive(Deserialize)]
struct CachedDetails {
    cached_tokens: Option<i64>,
}

#[derive(Deserialize)]
struct ReasoningDetails {
    reasoning_tokens: Option<i64>,
}

fn parse_openai(json: &[u8]) -> Option<RelayUsage> {
    let env = serde_json::from_slice::<OpenAiEnvelope>(json).ok()?;
    let usage = env.usage.or_else(|| env.response.and_then(|r| r.usage))?;
    let found = RelayUsage {
        input_tokens: usage.prompt_tokens.or(usage.input_tokens),
        output_tokens: usage.completion_tokens.or(usage.output_tokens),
        cached_tokens: usage
            .prompt_tokens_details
            .or(usage.input_tokens_details)
            .and_then(|d| d.cached_tokens),
        reasoning_tokens: usage
            .completion_tokens_details
            .or(usage.output_tokens_details)
            .and_then(|d| d.reasoning_tokens),
    };
    (!found.is_empty()).then_some(found)
}

#[derive(Deserialize)]
struct AnthropicEnvelope {
    /// `message_delta` 与非流式响应把 usage 放在顶层。
    usage: Option<AnthropicUsage>,
    /// `message_start` 把它放在 `message` 里。
    message: Option<AnthropicNested>,
    /// 部分兼容实现把 usage 放在 `delta` 里。
    delta: Option<AnthropicNested>,
}

#[derive(Deserialize)]
struct AnthropicNested {
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    /// Anthropic 的这一列是**未缓存输入**，不是含缓存的总输入。
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_creation_input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
}

/// 把 Anthropic 的三列输入折成网关内部不变量：
/// `input_tokens` = 全部输入，`cached_tokens` = 其中命中读取缓存的子集。
/// cache creation 仍按普通输入价进入 uncached 部分；当前价格模型没有独立的
/// cache-write 单价，至少不能把它误当 cache-read 或从总输入里再减一次。
fn anthropic_input(usage: &AnthropicUsage) -> Option<Option<i64>> {
    let columns = [
        usage.input_tokens,
        usage.cache_creation_input_tokens,
        usage.cache_read_input_tokens,
    ];
    if columns.iter().all(Option::is_none) {
        return Some(None);
    }
    let total = columns
        .into_iter()
        .flatten()
        .try_fold(0_i64, i64::checked_add)?;
    Some(Some(total))
}

fn parse_anthropic(json: &[u8]) -> Option<RelayUsage> {
    let env = serde_json::from_slice::<AnthropicEnvelope>(json).ok()?;
    let mut found = RelayUsage::default();
    for usage in [
        env.message.and_then(|m| m.usage),
        env.delta.and_then(|d| d.usage),
        env.usage,
    ]
    .into_iter()
    .flatten()
    {
        overwrite(&mut found.input_tokens, anthropic_input(&usage)?);
        overwrite(&mut found.output_tokens, usage.output_tokens);
        overwrite(&mut found.cached_tokens, usage.cache_read_input_tokens);
    }
    (!found.is_empty()).then_some(found)
}

#[derive(Deserialize)]
struct GoogleEnvelope {
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<GoogleUsage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleUsage {
    prompt_token_count: Option<i64>,
    candidates_token_count: Option<i64>,
    thoughts_token_count: Option<i64>,
    cached_content_token_count: Option<i64>,
}

fn parse_google(json: &[u8]) -> Option<RelayUsage> {
    let meta = serde_json::from_slice::<GoogleEnvelope>(json)
        .ok()?
        .usage_metadata?;
    let found = RelayUsage {
        input_tokens: meta.prompt_token_count,
        output_tokens: meta.candidates_token_count,
        cached_tokens: meta.cached_content_token_count,
        reasoning_tokens: meta.thoughts_token_count,
    };
    (!found.is_empty()).then_some(found)
}

#[cfg(test)]
mod tests;
