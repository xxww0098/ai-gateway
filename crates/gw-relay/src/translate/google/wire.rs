//! Google GenerateContent 的 wire 形状 + SSE 帧的收发原语 —— 两个方向共用。
//!
//! OWNER: worker `relay-google`。
//!
//! 这里只放**与目标方言无关**的东西：Google 侧的 JSON 形状、`alt=sse` 帧的
//! 拆装、usage 归并、finishReason 映射表。凡是「OpenAI 长什么样」「Anthropic
//! 长什么样」的知识都不在这个文件里，在 [`super::openai`] 与 [`super::anthropic`]。
//!
//! gemini 与 vertex 共用本文件的全部内容：它们的 wire 协议是同一个
//! GenerateContent，差别只在 endpoint 前缀与鉴权，而那两样归 `engine` 管。

use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::contract::{RelayUsage, TranslateError};

// ============================================================ 响应形状（入）

/// `:generateContent` / `:streamGenerateContent` 的响应体。
///
/// 只声明我们真的会读的字段。Google 侧的 `safetyRatings` / `citationMetadata` /
/// `avgLogprobs` 被 serde 忽略 —— 它们是**纯装饰性**的，丢弃理由见 [`super`]
/// 的模块文档。
#[derive(Debug, Default, Deserialize)]
pub(super) struct GenerateContentResponse {
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    /// 上游真正服务这次请求的模型名。两家方言的响应里都有 `model` 字段，
    /// 客户端拿它做日志对账，丢了就对不上。
    #[serde(default, rename = "modelVersion")]
    pub model_version: Option<String>,
    #[serde(default, rename = "usageMetadata")]
    pub usage_metadata: Option<UsageMetadata>,
    /// prompt 被安全策略整体拦截时，`candidates` 为空、拦截原因只在这里。
    /// 不读它就会把「被拦截」翻译成「模型什么都没说」—— 一个静默的正确性错误。
    #[serde(default, rename = "promptFeedback")]
    pub prompt_feedback: Option<PromptFeedback>,
    /// 上游给的响应 id。有就用它，没有才合成（见 [`synthetic_id`]）。
    #[serde(default, rename = "responseId")]
    pub response_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PromptFeedback {
    #[serde(default, rename = "blockReason")]
    pub block_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct Candidate {
    #[serde(default)]
    pub content: Option<Content>,
    #[serde(default, rename = "finishReason")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct Content {
    #[serde(default)]
    pub parts: Vec<Part>,
}

/// Google 的 part 是一个 union，但 wire 上是「哪个键在就是哪个变体」。
#[derive(Debug, Default, Deserialize)]
pub(super) struct Part {
    #[serde(default)]
    pub text: Option<String>,
    /// `true` 表示这段 text 是模型的思考过程，不是给用户的回答。
    #[serde(default)]
    pub thought: Option<bool>,
    #[serde(default, rename = "functionCall")]
    pub function_call: Option<FunctionCall>,
}

#[derive(Debug, Deserialize)]
pub(super) struct FunctionCall {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

/// 四个计数全是 [`Option`]：**「缺失」与「零」必须能分开**。
/// 上游没给 `candidatesTokenCount` 与上游说「产出了 0 个 token」是两件事，
/// 前者要落 fallback 结算，后者不能。
#[derive(Debug, Default, Deserialize)]
pub(super) struct UsageMetadata {
    #[serde(default, rename = "promptTokenCount")]
    pub prompt: Option<i64>,
    #[serde(default, rename = "candidatesTokenCount")]
    pub candidates: Option<i64>,
    #[serde(default, rename = "cachedContentTokenCount")]
    pub cached: Option<i64>,
    #[serde(default, rename = "thoughtsTokenCount")]
    pub thoughts: Option<i64>,
}

/// 把一帧里的 `usageMetadata` 并进累计值。
///
/// **只覆盖本帧真的给了的字段。** Google 的 `usageMetadata` 是累计量，
/// 但首帧常常只带 `promptTokenCount`、末帧才补齐其余 —— 如果整体替换，
/// 末帧缺 `promptTokenCount` 时首帧那个值就没了，计费直接少算 input。
/// 这就是「转义可以丢字段，绝不能丢 usage」在代码里的样子。
pub(super) fn merge_usage(dst: &mut RelayUsage, src: &UsageMetadata) {
    if src.prompt.is_some() {
        dst.input_tokens = src.prompt;
    }
    if src.candidates.is_some() {
        dst.output_tokens = src.candidates;
    }
    if src.cached.is_some() {
        dst.cached_tokens = src.cached;
    }
    if src.thoughts.is_some() {
        dst.reasoning_tokens = src.thoughts;
    }
}

// ============================================================ finishReason

/// Google 的安全类终止原因。这些值下 `candidates[].content` 通常是空的。
const GOOGLE_BLOCKED: &[&str] = &[
    "SAFETY",
    "RECITATION",
    "BLOCKLIST",
    "PROHIBITED_CONTENT",
    "SPII",
    "IMAGE_SAFETY",
];

/// Google finishReason → OpenAI `finish_reason`。
///
/// 有 functionCall 时 `tool_calls` 压过 `stop`：OpenAI 客户端靠这个值决定
/// 要不要去读 `message.tool_calls`，回 `stop` 会让工具调用被当成空回答。
pub(super) fn openai_finish_reason(google: &str, has_tool_call: bool) -> &'static str {
    if has_tool_call {
        return "tool_calls";
    }
    match google {
        "MAX_TOKENS" => "length",
        g if GOOGLE_BLOCKED.contains(&g) => "content_filter",
        _ => "stop",
    }
}

/// Google finishReason → Anthropic `stop_reason`。
///
/// **与 `docs/relay-surface-plan.md` §3.6 的速写有一处有意偏离**：那里把
/// `SAFETY` 位置对位到 `stop_sequence`，但 Anthropic 的 `stop_sequence` 的
/// 含义是「命中了客户端自己设的 stop sequence」，客户端收到它会去读同一个
/// 响应里的 `stop_sequence` 字段拿那个字符串 —— 而安全拦截下这个字段只能是
/// `null`，等于把「被安全策略拦下」渲染成「命中了一个不存在的停止词」。
/// Anthropic 现行 API 里表达拒答的值是 `refusal`，这里用它。
pub(super) fn anthropic_stop_reason(google: &str, has_tool_call: bool) -> &'static str {
    if has_tool_call {
        return "tool_use";
    }
    match google {
        "MAX_TOKENS" => "max_tokens",
        g if GOOGLE_BLOCKED.contains(&g) => "refusal",
        _ => "end_turn",
    }
}

// ============================================================ SSE 原语

/// Incrementally splits arbitrary HTTP body chunks into complete SSE events.
///
/// `reqwest` / hyper expose transport chunks, not SSE records. A single Google
/// JSON event may therefore be split at any byte. The decoder keeps only the
/// unterminated tail, emits complete `data:` payloads, and bounds one event so
/// a peer that never sends a blank line cannot grow memory without limit.
const MAX_SSE_EVENT: usize = 8 * 1024 * 1024;

#[derive(Debug, Default)]
pub(super) struct SseDecoder {
    pending: Vec<u8>,
    /// Bytes before this index were already proved not to start a separator.
    /// Keep the last three bytes in the next scan because `\r\n\r\n` can
    /// straddle a transport boundary.
    scan_from: usize,
}

impl SseDecoder {
    pub(super) fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, TranslateError> {
        self.pending.extend_from_slice(chunk);
        let mut payloads = Vec::new();
        let mut consumed = 0usize;
        let mut search_from = self.scan_from.min(self.pending.len());

        while let Some((end, separator_len)) = find_event_end(&self.pending, search_from) {
            if end.saturating_sub(consumed) > MAX_SSE_EVENT {
                self.pending.clear();
                self.scan_from = 0;
                return Err(TranslateError::UpstreamShape(
                    "google SSE event exceeds 8 MiB".to_owned(),
                ));
            }
            if end > consumed {
                payloads.extend(data_payloads(&self.pending[consumed..end]));
            }
            consumed = end + separator_len;
            search_from = consumed;
        }

        if consumed > 0 {
            self.pending.drain(..consumed);
        }
        if self.pending.len() > MAX_SSE_EVENT {
            self.pending.clear();
            self.scan_from = 0;
            return Err(TranslateError::UpstreamShape(
                "unterminated google SSE event exceeds 8 MiB".to_owned(),
            ));
        }
        // Everything before this point has already been scanned. Retaining the
        // final three bytes is sufficient for the longest separator prefix.
        self.scan_from = self.pending.len().saturating_sub(3);
        Ok(payloads)
    }
}

fn find_event_end(buf: &[u8], start: usize) -> Option<(usize, usize)> {
    for index in start..buf.len() {
        if buf[index] == b'\n' && buf.get(index + 1) == Some(&b'\n') {
            return Some((index, 2));
        }
        if buf[index] == b'\r' && buf[index + 1..].starts_with(b"\n\r\n") {
            return Some((index, 4));
        }
    }
    None
}

/// Extracts every `data:` payload from one or more complete SSE event blocks.
/// Multi-line data fields are joined with `\n` as required by the SSE grammar.
fn data_payloads(buf: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut cur: Option<Vec<u8>> = None;
    for raw in buf.split(|&b| b == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            if let Some(payload) = cur.take() {
                out.push(payload);
            }
            continue;
        }
        let Some(rest) = line.strip_prefix(b"data:") else {
            continue;
        };
        let rest = rest.strip_prefix(b" ").unwrap_or(rest);
        let slot = cur.get_or_insert_with(Vec::new);
        if !slot.is_empty() {
            slot.push(b'\n');
        }
        slot.extend_from_slice(rest);
    }
    if let Some(payload) = cur.take() {
        out.push(payload);
    }
    out
}

/// 这个 data 载荷值不值得解析成 Google 响应。
pub(super) fn is_parseable(payload: &[u8]) -> bool {
    let trimmed = payload.trim_ascii();
    !trimmed.is_empty() && trimmed != b"[DONE]"
}

/// 解析一个 Google 响应载荷。上游给了不认识的形状是**上游或转义器**的 bug，
/// 不是客户端的错 —— 所以是 `UpstreamShape` 而不是 `Malformed`。
pub(super) fn parse_response(payload: &[u8]) -> Result<GenerateContentResponse, TranslateError> {
    serde_json::from_slice(payload)
        .map_err(|err| TranslateError::UpstreamShape(format!("google response: {err}")))
}

/// OpenAI 方言的一帧：`data: {json}\n\n`，没有 `event:` 行。
pub(super) fn openai_frame(value: &Value) -> Result<Bytes, TranslateError> {
    let mut buf = Vec::with_capacity(64);
    buf.extend_from_slice(b"data: ");
    serialize_into(&mut buf, value)?;
    buf.extend_from_slice(b"\n\n");
    Ok(Bytes::from(buf))
}

/// Anthropic 方言的一帧：`event: <name>\ndata: {json}\n\n`。
///
/// `event:` 行不是可选的 —— Anthropic 的官方 SDK 按 `event` 字段分派事件类型，
/// 只给 `data:` 的流会被它当成未知事件全部丢掉。
pub(super) fn anthropic_frame(event: &str, value: &Value) -> Result<Bytes, TranslateError> {
    let mut buf = Vec::with_capacity(64 + event.len());
    buf.extend_from_slice(b"event: ");
    buf.extend_from_slice(event.as_bytes());
    buf.extend_from_slice(b"\ndata: ");
    serialize_into(&mut buf, value)?;
    buf.extend_from_slice(b"\n\n");
    Ok(Bytes::from(buf))
}

fn serialize_into(buf: &mut Vec<u8>, value: &Value) -> Result<(), TranslateError> {
    serde_json::to_writer(&mut *buf, value)
        .map_err(|err| TranslateError::UpstreamShape(format!("cannot serialize frame: {err}")))
}

// ============================================================ 杂项

/// 上游没给 `responseId` 时合成一个。客户端只把它当不透明字符串做日志关联，
/// 唯一的要求是别在同一个进程里撞车。
pub(super) fn synthetic_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{prefix}{nanos}")
}

/// unix 秒。OpenAI 的 `created` 字段要它。
pub(super) fn unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

// ============================================================ 请求侧共用

/// 请求体必须是一个 JSON 对象。这是客户端的错，所以是 `Malformed`。
pub(super) fn as_object(body: &[u8]) -> Result<Map<String, Value>, TranslateError> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(other) => Err(TranslateError::Malformed(format!(
            "request body must be a JSON object, got {}",
            kind_of(&other)
        ))),
        Err(err) => Err(TranslateError::Malformed(err.to_string())),
    }
}

pub(super) fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// 未知键一律拒绝。
///
/// 这是本模块最重要的一条策略：**不要静默丢弃一个有语义的字段然后假装成功**。
/// `mapped` 是会被翻译过去的键，`dropped` 是确认纯装饰性、丢了不改变上游行为的键
/// （每一个的理由写在 [`super`] 的模块文档里）。两张表都没有的键 → 400，
/// 客户端至少知道自己发的东西没生效。
pub(super) fn reject_unknown(
    obj: &Map<String, Value>,
    mapped: &[&str],
    dropped: &[&str],
    ctx: &str,
) -> Result<(), TranslateError> {
    for key in obj.keys() {
        let k = key.as_str();
        if mapped.contains(&k) || dropped.contains(&k) {
            continue;
        }
        return Err(TranslateError::Unsupported(format!(
            "{ctx}: `{k}` has no GenerateContent equivalent"
        )));
    }
    Ok(())
}

/// 布尔开关：等于默认值就当没写（丢弃无副作用），不等于默认值就是客户端
/// 真的要改行为 —— Google 没这个旋钮，只能 400。
///
/// 这条规则让 `parallel_tool_calls: true`（OpenAI 的默认值）与
/// `disable_parallel_tool_use: false`（Anthropic 的默认值）这种 SDK 无脑带上的
/// 字段不会白白打客户端一个 400，同时又不放过真正要关掉并行工具调用的请求。
pub(super) fn reject_non_default_bool(
    value: Option<&Value>,
    default: bool,
    ctx: &str,
) -> Result<(), TranslateError> {
    match value {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Bool(b)) if *b == default => Ok(()),
        Some(_) => Err(TranslateError::Unsupported(format!(
            "{ctx}: GenerateContent cannot express a non-default value here"
        ))),
    }
}

/// `data:<mime>;base64,<payload>` → Google `inlineData`。
///
/// 非 data URL（`https://…`）返回 `None`：Google 的 `fileData.fileUri` 只收
/// GCS / Files API 的 URI，网关替客户端去下载再转 base64 是一次静默的、
/// 会失败得莫名其妙的副作用。调用方据此回 400。
pub(super) fn data_url_to_inline(url: &str) -> Option<Value> {
    let rest = url.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let mime = meta.strip_suffix(";base64")?;
    Some(serde_json::json!({
        "inlineData": { "mimeType": mime, "data": payload }
    }))
}

/// `generationConfig` 的累加器。两个方向的采样参数名不同，但落点相同。
#[derive(Default)]
pub(super) struct GenerationConfig(Map<String, Value>);

impl GenerationConfig {
    /// 存在且非 null 才写入 —— 显式的 `null` 在两家方言里都表示「用默认值」，
    /// 原样转过去会让 Google 报 `INVALID_ARGUMENT`。
    pub(super) fn set(&mut self, key: &str, value: Option<&Value>) {
        if let Some(v) = value.filter(|v| !v.is_null()) {
            self.0.insert(key.to_owned(), v.clone());
        }
    }

    pub(super) fn put(&mut self, key: &str, value: Value) {
        self.0.insert(key.to_owned(), value);
    }

    pub(super) fn into_value(self) -> Option<Value> {
        (!self.0.is_empty()).then_some(Value::Object(self.0))
    }
}

/// `stop` / `stop_sequences` → `generationConfig.stopSequences`（Google 只收数组）。
pub(super) fn stop_sequences(value: &Value) -> Result<Value, TranslateError> {
    match value {
        Value::String(s) => Ok(Value::Array(vec![Value::String(s.clone())])),
        Value::Array(items) if items.iter().all(Value::is_string) => {
            Ok(Value::Array(items.clone()))
        }
        other => Err(TranslateError::Malformed(format!(
            "stop sequences must be a string or an array of strings, got {}",
            kind_of(other)
        ))),
    }
}

/// `[{name, description, parameters}]` → Google 的 `tools[0].functionDeclarations`。
///
/// Google 只认 OpenAPI 3.0 子集的 schema。`additionalProperties` / `$schema` /
/// `$ref` 这些 JSON Schema 关键字它会直接 400 —— 这里**不清洗**，原样传过去，
/// 让 Google 自己拒。清洗等于替客户端改工具契约，那才是静默的正确性错误。
pub(super) fn function_declarations(decls: Vec<Value>) -> Option<Value> {
    (!decls.is_empty()).then(|| serde_json::json!([{ "functionDeclarations": decls }]))
}
