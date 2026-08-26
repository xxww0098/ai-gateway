//! `anthropic-messages × {gemini, vertex}` —— 15 格矩阵里的 P1 另两格。
//!
//! OWNER: worker `relay-google`。
//!
//! 入口方言 [`Surface::AnthropicMessages`] ↔ 上游方言
//! [`UpstreamDialect::GoogleGenerateContent`]。
//!
//! 这一格比 OpenAI 那一格难，难点全在流式：Anthropic 的 SSE 是**带框架的**
//! （`message_start` / `content_block_start` / `content_block_stop` /
//! `message_delta` / `message_stop`），而 Google 侧一个对应物都没有 ——
//! 它只吐 `candidates[].content.parts[]`。这些框架事件必须由
//! [`AnthropicStream`] **合成**，而合成需要知道「现在开着第几个 block、
//! 它是什么类型、message_start 发过没有」，这就是 [`StreamTranslator`]
//! 必须有状态的原因。

use std::collections::HashMap;

use bytes::Bytes;
use serde_json::{Map, Value, json};

use super::wire::{self, GenerateContentResponse, GenerationConfig};
use crate::contract::{
    RelayUsage, StreamTranslator, Surface, TranslateError, Translator, UpstreamDialect,
};

/// `POST /v1/messages` ↔ Google GenerateContent 的转义器。
///
/// 根除 `docs/relay-passthrough-audit.md` 缺陷 **#1**（S1）在 gemini/vertex 上的
/// 另一半：今天 `docs/relay-surface-plan.md` §3.6 的 C×gemini / C×vertex 两格是
/// **直通**的，Anthropic 形状的 body 被原样 POST 给 GenerateContent，上游必 400。
///
/// 流式方向同时是缺陷 **#6**（S2）的一道防线：Google 流在任何位置结束，
/// [`StreamTranslator::finish`] 都会补齐 `content_block_stop` /
/// `message_delta` / `message_stop`，客户端拿到的永远是一个**语法完整**的
/// Anthropic 事件序列；真正的中途失败由 `engine` 用
/// [`crate::RelayError`] 表达（h2 `RST_STREAM`），而不是伪装成一次干净的 EOF。
///
/// usage 走 [`StreamTranslator::usage`] 直接交给计费，**取代**
/// [`crate::UsageProbe`] —— 上游帧已经在这里解析过一次，再解析一遍是纯浪费
/// （缺陷 **#16** 说的那 2× 内存带宽）。
///
/// # 不变量（由 `google/tests.rs` 的序列合法性测试守护）
///
/// - `message_start` 先于任何 `content_block_delta`，且只有一次。
/// - `message_stop` 最后一帧，且只有一次。
/// - 每个 `content_block_delta` 都落在一对 `content_block_start` /
///   `content_block_stop` 之间，`index` 从 0 起严格递增。
/// - `usageMetadata` 的四个计数一个都不丢，且「缺失」与「零」可分。
#[derive(Debug, Default, Clone, Copy)]
pub struct AnthropicToGoogle;

impl Translator for AnthropicToGoogle {
    fn surface(&self) -> Surface {
        Surface::AnthropicMessages
    }

    fn to_dialect(&self) -> UpstreamDialect {
        UpstreamDialect::GoogleGenerateContent
    }

    fn translate_request(&self, _model: &str, body: &[u8]) -> Result<Bytes, TranslateError> {
        // `model` 走 URL，不进 body —— 理由同 [`super::openai`]。
        build_request(body)
    }

    fn translate_response(&self, body: &[u8]) -> Result<Bytes, TranslateError> {
        build_response(body)
    }

    fn stream_translator(&self) -> Box<dyn StreamTranslator> {
        Box::new(AnthropicStream::default())
    }
}

// ===================================================================== 请求

const REQ_MAPPED: &[&str] = &[
    "model",
    "messages",
    "system",
    "max_tokens",
    "temperature",
    "top_p",
    "top_k",
    "stop_sequences",
    "tools",
    "tool_choice",
    "thinking",
];

/// **确认可以静默丢弃**的顶层字段，逐条理由见 [`super`] 的模块文档。
const REQ_DROPPED: &[&str] = &["stream", "metadata"];

const MSG_MAPPED: &[&str] = &["role", "content"];

fn build_request(body: &[u8]) -> Result<Bytes, TranslateError> {
    let root = wire::as_object(body)?;
    wire::reject_unknown(&root, REQ_MAPPED, REQ_DROPPED, "messages")?;

    let messages = root
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            TranslateError::Malformed("`messages` is required and must be an array".to_owned())
        })?;

    let mut contents: Vec<Value> = Vec::with_capacity(messages.len());
    // tool_use id → 工具名。同 [`super::openai`]：按顺序增量建表，
    // 命中的永远是紧邻的上一轮 assistant tool_use。
    let mut tool_names: HashMap<String, String> = HashMap::new();

    for message in messages {
        let msg = message.as_object().ok_or_else(|| {
            TranslateError::Malformed("each message must be a JSON object".to_owned())
        })?;
        wire::reject_unknown(msg, MSG_MAPPED, &[], "messages[]")?;
        let role = match msg.get("role").and_then(Value::as_str) {
            Some("user") => "user",
            Some("assistant") => "model",
            Some(other) => {
                return Err(TranslateError::Unsupported(format!(
                    "messages[].role `{other}` has no GenerateContent equivalent"
                )));
            }
            None => {
                return Err(TranslateError::Malformed(
                    "each message needs a string `role`".to_owned(),
                ));
            }
        };
        let parts = content_parts(msg.get("content"), &mut tool_names)?;
        if !parts.is_empty() {
            contents.push(json!({ "role": role, "parts": parts }));
        }
    }

    let mut out = Map::new();
    out.insert("contents".to_owned(), Value::Array(contents));
    if let Some(parts) = system_instruction(root.get("system"))? {
        out.insert("systemInstruction".to_owned(), json!({ "parts": parts }));
    }
    if let Some(cfg) = generation_config(&root)?.into_value() {
        out.insert("generationConfig".to_owned(), cfg);
    }
    if let Some(tools) = wire::function_declarations(tool_declarations(root.get("tools"))?) {
        out.insert("tools".to_owned(), tools);
    }
    if let Some(cfg) = tool_config(root.get("tool_choice"))? {
        out.insert("toolConfig".to_owned(), cfg);
    }

    serde_json::to_vec(&Value::Object(out))
        .map(Bytes::from)
        .map_err(|err| TranslateError::Malformed(err.to_string()))
}

fn generation_config(root: &Map<String, Value>) -> Result<GenerationConfig, TranslateError> {
    let mut cfg = GenerationConfig::default();
    cfg.set("temperature", root.get("temperature"));
    cfg.set("topP", root.get("top_p"));
    cfg.set("topK", root.get("top_k"));
    cfg.set("maxOutputTokens", root.get("max_tokens"));
    if let Some(stop) = root.get("stop_sequences").filter(|v| !v.is_null()) {
        cfg.put("stopSequences", wire::stop_sequences(stop)?);
    }
    if let Some(thinking) = thinking_config(root.get("thinking"))? {
        cfg.put("thinkingConfig", thinking);
    }
    Ok(cfg)
}

/// `thinking` → `generationConfig.thinkingConfig`。两家的旋钮语义正好对得上：
/// Anthropic 给 token 预算，Google 也给 token 预算。
fn thinking_config(value: Option<&Value>) -> Result<Option<Value>, TranslateError> {
    let Some(thinking) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    match thinking.get("type").and_then(Value::as_str) {
        Some("disabled") => Ok(Some(json!({ "thinkingBudget": 0 }))),
        Some("enabled") => {
            let mut cfg = Map::new();
            cfg.insert("includeThoughts".to_owned(), Value::Bool(true));
            if let Some(budget) = thinking.get("budget_tokens").filter(|v| !v.is_null()) {
                cfg.insert("thinkingBudget".to_owned(), budget.clone());
            }
            Ok(Some(Value::Object(cfg)))
        }
        Some(other) => Err(TranslateError::Unsupported(format!(
            "messages.thinking.type `{other}` has no GenerateContent equivalent"
        ))),
        None => Err(TranslateError::Malformed(
            "`thinking` needs a string `type`".to_owned(),
        )),
    }
}

fn system_instruction(value: Option<&Value>) -> Result<Option<Vec<Value>>, TranslateError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(vec![json!({ "text": text })])),
        Some(Value::Array(blocks)) => {
            let parts = blocks
                .iter()
                .map(|block| match block.get("type").and_then(Value::as_str) {
                    Some("text") => Ok(json!({
                        "text": block.get("text").and_then(Value::as_str).unwrap_or("")
                    })),
                    Some(other) => Err(TranslateError::Unsupported(format!(
                        "messages.system block `{other}`: GenerateContent's \
                         systemInstruction accepts text only"
                    ))),
                    None => Err(TranslateError::Malformed(
                        "each system block needs a string `type`".to_owned(),
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((!parts.is_empty()).then_some(parts))
        }
        Some(other) => Err(TranslateError::Malformed(format!(
            "`system` must be a string or an array of blocks, got {}",
            wire::kind_of(other)
        ))),
    }
}

fn content_parts(
    value: Option<&Value>,
    names: &mut HashMap<String, String>,
) -> Result<Vec<Value>, TranslateError> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(text)) => Ok(vec![json!({ "text": text })]),
        Some(Value::Array(blocks)) => {
            let mut parts = Vec::with_capacity(blocks.len());
            for block in blocks {
                if let Some(part) = content_block(block, names)? {
                    parts.push(part);
                }
            }
            Ok(parts)
        }
        Some(other) => Err(TranslateError::Malformed(format!(
            "message content must be a string or an array of blocks, got {}",
            wire::kind_of(other)
        ))),
    }
}

/// 返回 `None` 表示这个 block **确认可以丢**（见 [`super`] 的模块文档）。
fn content_block(
    block: &Value,
    names: &mut HashMap<String, String>,
) -> Result<Option<Value>, TranslateError> {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => Ok(Some(json!({
            "text": block.get("text").and_then(Value::as_str).unwrap_or("")
        }))),
        Some("image") => image_block(block).map(Some),
        Some("tool_use") => {
            let name = block.get("name").and_then(Value::as_str).ok_or_else(|| {
                TranslateError::Malformed("tool_use block needs a `name`".to_owned())
            })?;
            if let Some(id) = block.get("id").and_then(Value::as_str) {
                names.insert(id.to_owned(), name.to_owned());
            }
            let args = block.get("input").cloned().unwrap_or_else(|| json!({}));
            Ok(Some(
                json!({ "functionCall": { "name": name, "args": args } }),
            ))
        }
        Some("tool_result") => tool_result(block, names).map(Some),
        // 思考块是上一轮 assistant 输出的回放。Google 没有「回放思考」这个
        // 概念（`thoughtSignature` 只在同一次 Google 会话内有意义），
        // 丢掉不改变本轮的模型行为。
        Some("thinking" | "redacted_thinking") => Ok(None),
        Some(other) => Err(TranslateError::Unsupported(format!(
            "messages content block `{other}` has no GenerateContent equivalent"
        ))),
        None => Err(TranslateError::Malformed(
            "each content block needs a string `type`".to_owned(),
        )),
    }
}

fn image_block(block: &Value) -> Result<Value, TranslateError> {
    let source = block
        .get("source")
        .ok_or_else(|| TranslateError::Malformed("image block needs a `source`".to_owned()))?;
    match source.get("type").and_then(Value::as_str) {
        Some("base64") => {
            let mime = source
                .get("media_type")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    TranslateError::Malformed("image source needs `media_type`".to_owned())
                })?;
            let data = source
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| TranslateError::Malformed("image source needs `data`".to_owned()))?;
            Ok(json!({ "inlineData": { "mimeType": mime, "data": data } }))
        }
        Some(other) => Err(TranslateError::Unsupported(format!(
            "messages image source `{other}`: GenerateContent only accepts inline \
             base64 bytes, and fetching a remote image on the client's behalf is a \
             side effect the gateway must not take silently"
        ))),
        None => Err(TranslateError::Malformed(
            "image source needs a string `type`".to_owned(),
        )),
    }
}

fn tool_result(block: &Value, names: &HashMap<String, String>) -> Result<Value, TranslateError> {
    let id = block
        .get("tool_use_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            TranslateError::Malformed("tool_result block needs `tool_use_id`".to_owned())
        })?;
    // 同 [`super::openai::tool_result`]：Google 按名字匹配，名字找不回来只能 400。
    let name = names.get(id).ok_or_else(|| {
        TranslateError::Unsupported(format!(
            "messages tool_result references tool_use_id `{id}` that no preceding \
             assistant message declared; GenerateContent matches function responses \
             by name, so the name cannot be recovered"
        ))
    })?;
    let payload = match block.get("content") {
        Some(Value::String(text)) => Value::String(text.clone()),
        Some(other) if !other.is_null() => other.clone(),
        _ => Value::Null,
    };
    // `is_error` 不能丢：模型看到「工具报错了」和看到「工具返回了这段文本」
    // 会做出完全不同的下一步。Google 的 functionResponse 没有错误标志位，
    // 只能把它编码进 response 对象的键名里。
    let key = if block.get("is_error").and_then(Value::as_bool) == Some(true) {
        "error"
    } else {
        "content"
    };
    let mut response = Map::new();
    if !payload.is_null() {
        response.insert(key.to_owned(), payload);
    }
    Ok(json!({
        "functionResponse": { "name": name, "response": Value::Object(response) }
    }))
}

fn tool_declarations(value: Option<&Value>) -> Result<Vec<Value>, TranslateError> {
    let Some(tools) = value.filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let tools = tools
        .as_array()
        .ok_or_else(|| TranslateError::Malformed("`tools` must be an array".to_owned()))?;
    tools
        .iter()
        .map(|tool| {
            // Anthropic 的服务端工具（`web_search_*` / `computer_*` / `bash_*`）
            // 由 Anthropic 自己执行，Google 侧没有任何对应物。
            if let Some(kind) = tool
                .get("type")
                .and_then(Value::as_str)
                .filter(|k| *k != "custom")
            {
                return Err(TranslateError::Unsupported(format!(
                    "messages tool type `{kind}` is server-side at Anthropic and \
                     has no GenerateContent equivalent"
                )));
            }
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| TranslateError::Malformed("tool needs a `name`".to_owned()))?;
            let mut decl = Map::new();
            decl.insert("name".to_owned(), Value::String(name.to_owned()));
            if let Some(desc) = tool.get("description").filter(|v| !v.is_null()) {
                decl.insert("description".to_owned(), desc.clone());
            }
            if let Some(schema) = tool.get("input_schema").filter(|v| !v.is_null()) {
                decl.insert("parameters".to_owned(), schema.clone());
            }
            Ok(Value::Object(decl))
        })
        .collect()
}

fn tool_config(value: Option<&Value>) -> Result<Option<Value>, TranslateError> {
    let Some(choice) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    wire::reject_non_default_bool(
        choice.get("disable_parallel_tool_use"),
        false,
        "messages.tool_choice.disable_parallel_tool_use",
    )?;
    let mut cfg = Map::new();
    match choice.get("type").and_then(Value::as_str) {
        Some("auto") => {
            cfg.insert("mode".to_owned(), Value::String("AUTO".to_owned()));
        }
        Some("any") => {
            cfg.insert("mode".to_owned(), Value::String("ANY".to_owned()));
        }
        Some("none") => {
            cfg.insert("mode".to_owned(), Value::String("NONE".to_owned()));
        }
        Some("tool") => {
            let name = choice.get("name").and_then(Value::as_str).ok_or_else(|| {
                TranslateError::Malformed("tool_choice `tool` needs a `name`".to_owned())
            })?;
            cfg.insert("mode".to_owned(), Value::String("ANY".to_owned()));
            cfg.insert("allowedFunctionNames".to_owned(), json!([name]));
        }
        Some(other) => {
            return Err(TranslateError::Unsupported(format!(
                "messages.tool_choice `{other}` has no GenerateContent equivalent"
            )));
        }
        None => {
            return Err(TranslateError::Malformed(
                "`tool_choice` needs a string `type`".to_owned(),
            ));
        }
    }
    Ok(Some(json!({ "functionCallingConfig": Value::Object(cfg) })))
}

// ============================================================ 响应（非流式）

fn build_response(body: &[u8]) -> Result<Bytes, TranslateError> {
    let resp = wire::parse_response(body)?;
    let mut usage = RelayUsage::default();
    if let Some(meta) = resp.usage_metadata.as_ref() {
        wire::merge_usage(&mut usage, meta);
    }

    let candidate = resp.candidates.first();
    let mut blocks: Vec<Value> = Vec::new();
    if let Some(content) = candidate.and_then(|c| c.content.as_ref()) {
        for (idx, part) in content.parts.iter().enumerate() {
            if let Some(call) = part.function_call.as_ref() {
                blocks.push(json!({
                    "type": "tool_use",
                    "id": format!("toolu_{idx}"),
                    "name": call.name,
                    "input": call.args,
                }));
            }
            if let Some(text) = part.text.as_deref() {
                blocks.push(text_block(text, part.thought.unwrap_or(false)));
            }
        }
    }
    let has_tool_call = blocks
        .iter()
        .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"));

    let raw_finish = candidate
        .and_then(|c| c.finish_reason.as_deref())
        .or_else(|| {
            resp.prompt_feedback
                .as_ref()
                .and_then(|f| f.block_reason.as_deref())
                .map(|_| "SAFETY")
        })
        .unwrap_or("STOP");

    let mut out = Map::new();
    out.insert(
        "id".to_owned(),
        Value::String(
            resp.response_id
                .unwrap_or_else(|| wire::synthetic_id("msg_")),
        ),
    );
    out.insert("type".to_owned(), Value::String("message".to_owned()));
    out.insert("role".to_owned(), Value::String("assistant".to_owned()));
    if let Some(model) = resp.model_version {
        out.insert("model".to_owned(), Value::String(model));
    }
    out.insert("content".to_owned(), Value::Array(blocks));
    out.insert(
        "stop_reason".to_owned(),
        Value::String(wire::anthropic_stop_reason(raw_finish, has_tool_call).to_owned()),
    );
    out.insert("stop_sequence".to_owned(), Value::Null);
    out.insert("usage".to_owned(), usage_value(&usage));

    serde_json::to_vec(&Value::Object(out))
        .map(Bytes::from)
        .map_err(|err| TranslateError::UpstreamShape(err.to_string()))
}

/// Anthropic 的 `thinking` block 在多个官方 SDK 里 `signature` 是必填字段，
/// 缺了会在客户端侧抛解析错误。Google 的 `thoughtSignature` 换个上游就失效，
/// 转过去只会误导 —— 所以给一个空串占位，而不是把这段思考文本整个丢掉。
fn text_block(text: &str, thought: bool) -> Value {
    if thought {
        json!({ "type": "thinking", "thinking": text, "signature": "" })
    } else {
        json!({ "type": "text", "text": text })
    }
}

/// [`RelayUsage`] → Anthropic 的 `usage` 信封。
///
/// 两处刻意的换算，都是为了对上 Anthropic 客户端的语义：
/// - `input_tokens` **不含**缓存命中（Anthropic 把缓存读单列），
///   而 Google 的 `promptTokenCount` **含** —— 所以要减掉。
/// - `output_tokens` **含**思考 token（Anthropic 这么算），
///   而 Google 的 `candidatesTokenCount` 不含 —— 所以要加上。
///
/// 计费拿到的 [`RelayUsage`] 是**原始值**，不做这两个换算（见 [`super`]）。
fn usage_value(usage: &RelayUsage) -> Value {
    let mut out = Map::new();
    let cached = usage.cached_tokens;
    if let Some(prompt) = usage.input_tokens {
        out.insert(
            "input_tokens".to_owned(),
            json!((prompt - cached.unwrap_or(0)).max(0)),
        );
    }
    let output = match (usage.output_tokens, usage.reasoning_tokens) {
        (Some(o), Some(r)) => Some(o + r),
        (Some(o), None) => Some(o),
        (None, r) => r,
    };
    if let Some(v) = output {
        out.insert("output_tokens".to_owned(), json!(v));
    }
    if let Some(v) = cached {
        out.insert("cache_read_input_tokens".to_owned(), json!(v));
    }
    Value::Object(out)
}

// ============================================================== 响应（流式）

/// 当前打开的 content block 的类型。切类型必须先 `content_block_stop`。
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum BlockKind {
    Text,
    Thinking,
}

/// Anthropic 方向的流式状态机 —— 本任务里最难的一块。
///
/// Google 只吐 parts，Anthropic 要一整套框架事件。这个结构体记住的三件事
/// 正是「无状态翻不出来」的那三件：
///
/// - `started`：`message_start` 发过没有（只能有一次，且必须最先）；
/// - `open`：现在开着第几个 block、是什么类型（决定要不要先 stop 再 start）；
/// - `usage` / `stop_reason`：收尾的 `message_delta` 要用，而它们分散在
///   上游的若干帧里。
#[derive(Default)]
struct AnthropicStream {
    sse: wire::SseDecoder,
    id: Option<String>,
    model: Option<String>,
    started: bool,
    stopped: bool,
    open: Option<(usize, BlockKind)>,
    next_index: usize,
    stop_reason: Option<String>,
    /// 本轮出过 functionCall 没有。Google 在有工具调用时 `finishReason` 仍然是
    /// `STOP`，不单独记就没法在收尾时把 `stop_reason` 定成 `tool_use` ——
    /// 而客户端正是靠这个值决定要不要去执行工具。
    saw_tool_call: bool,
    usage: RelayUsage,
}

impl AnthropicStream {
    fn message_start(&mut self, resp: &GenerateContentResponse) -> Result<Bytes, TranslateError> {
        self.started = true;
        if self.id.is_none() {
            self.id = Some(
                resp.response_id
                    .clone()
                    .unwrap_or_else(|| wire::synthetic_id("msg_")),
            );
        }
        if self.model.is_none() {
            self.model.clone_from(&resp.model_version);
        }
        let mut message = Map::new();
        message.insert(
            "id".to_owned(),
            Value::String(self.id.clone().unwrap_or_default()),
        );
        message.insert("type".to_owned(), Value::String("message".to_owned()));
        message.insert("role".to_owned(), Value::String("assistant".to_owned()));
        if let Some(model) = self.model.as_ref() {
            message.insert("model".to_owned(), Value::String(model.clone()));
        }
        message.insert("content".to_owned(), Value::Array(Vec::new()));
        message.insert("stop_reason".to_owned(), Value::Null);
        message.insert("stop_sequence".to_owned(), Value::Null);
        message.insert("usage".to_owned(), usage_value(&self.usage));
        wire::anthropic_frame(
            "message_start",
            &json!({ "type": "message_start", "message": Value::Object(message) }),
        )
    }

    fn close_open(&mut self, frames: &mut Vec<Bytes>) -> Result<(), TranslateError> {
        if let Some((index, _)) = self.open.take() {
            frames.push(wire::anthropic_frame(
                "content_block_stop",
                &json!({ "type": "content_block_stop", "index": index }),
            )?);
        }
        Ok(())
    }

    /// 打开一个新 block（必要时先关掉旧的），返回它的 index。
    fn open_block(
        &mut self,
        kind: BlockKind,
        frames: &mut Vec<Bytes>,
    ) -> Result<usize, TranslateError> {
        if let Some((index, open_kind)) = self.open
            && open_kind == kind
        {
            return Ok(index);
        }
        self.close_open(frames)?;
        let index = self.next_index;
        self.next_index += 1;
        self.open = Some((index, kind));
        let block = match kind {
            BlockKind::Text => json!({ "type": "text", "text": "" }),
            BlockKind::Thinking => json!({ "type": "thinking", "thinking": "" }),
        };
        frames.push(wire::anthropic_frame(
            "content_block_start",
            &json!({ "type": "content_block_start", "index": index, "content_block": block }),
        )?);
        Ok(index)
    }
}

impl StreamTranslator for AnthropicStream {
    fn push(&mut self, upstream_frame: &[u8]) -> Result<Vec<Bytes>, TranslateError> {
        let mut frames = Vec::new();
        for payload in self.sse.push(upstream_frame)? {
            if !wire::is_parseable(&payload) {
                continue;
            }
            let resp = wire::parse_response(&payload)?;
            // usage 先并 —— `message_start` 要带 `input_tokens`，而它通常就在
            // 本帧的 `usageMetadata` 里。
            if let Some(meta) = resp.usage_metadata.as_ref() {
                wire::merge_usage(&mut self.usage, meta);
            }
            if !self.started {
                let frame = self.message_start(&resp)?;
                frames.push(frame);
            }

            let candidate = resp.candidates.first();
            if let Some(content) = candidate.and_then(|c| c.content.as_ref()) {
                for (idx, part) in content.parts.iter().enumerate() {
                    if let Some(call) = part.function_call.as_ref() {
                        // Google 的 functionCall 是**整块**到达的，没有增量。
                        // Anthropic 方言里 tool_use 仍然必须走
                        // start → input_json_delta → stop 三帧，客户端的
                        // 累加器是照这个序列写的。
                        self.close_open(&mut frames)?;
                        self.saw_tool_call = true;
                        let index = self.next_index;
                        self.next_index += 1;
                        frames.push(wire::anthropic_frame(
                            "content_block_start",
                            &json!({
                                "type": "content_block_start",
                                "index": index,
                                "content_block": {
                                    "type": "tool_use",
                                    "id": format!("toolu_{index}_{idx}"),
                                    "name": call.name,
                                    "input": {},
                                }
                            }),
                        )?);
                        frames.push(wire::anthropic_frame(
                            "content_block_delta",
                            &json!({
                                "type": "content_block_delta",
                                "index": index,
                                "delta": {
                                    "type": "input_json_delta",
                                    "partial_json": serde_json::to_string(&call.args)
                                        .unwrap_or_else(|_| "{}".to_owned()),
                                }
                            }),
                        )?);
                        frames.push(wire::anthropic_frame(
                            "content_block_stop",
                            &json!({ "type": "content_block_stop", "index": index }),
                        )?);
                    }
                    let Some(text) = part.text.as_deref() else {
                        continue;
                    };
                    let (kind, delta) = if part.thought.unwrap_or(false) {
                        (
                            BlockKind::Thinking,
                            json!({ "type": "thinking_delta", "thinking": text }),
                        )
                    } else {
                        (
                            BlockKind::Text,
                            json!({ "type": "text_delta", "text": text }),
                        )
                    };
                    let index = self.open_block(kind, &mut frames)?;
                    frames.push(wire::anthropic_frame(
                        "content_block_delta",
                        &json!({
                            "type": "content_block_delta", "index": index, "delta": delta
                        }),
                    )?);
                }
            }

            if let Some(reason) = candidate
                .and_then(|c| c.finish_reason.as_deref())
                .or_else(|| {
                    resp.prompt_feedback
                        .as_ref()
                        .and_then(|f| f.block_reason.as_deref())
                        .map(|_| "SAFETY")
                })
            {
                self.stop_reason = Some(reason.to_owned());
            }
        }
        Ok(frames)
    }

    fn finish(&mut self) -> Result<Vec<Bytes>, TranslateError> {
        if self.stopped {
            // 幂等：重复收尾会发出第二个 `message_stop`，客户端会把它当成
            // 第二条消息的开头。
            return Ok(Vec::new());
        }
        // Flush a final event that omitted the trailing blank line. If it is a
        // truncated JSON object, `push` returns an error and the client sees a
        // reset rather than a fabricated clean EOF.
        let mut frames = self.push(b"\n\n")?;
        self.stopped = true;
        if !self.started {
            // 一帧内容都没产出的流仍然要给客户端一个语法完整的信封，
            // 否则就是缺陷 #6 那种「干净的 EOF」。
            let frame = self.message_start(&GenerateContentResponse::default())?;
            frames.push(frame);
        }
        self.close_open(&mut frames)?;

        let raw = self.stop_reason.as_deref().unwrap_or("STOP");
        let stop_reason = wire::anthropic_stop_reason(raw, self.saw_tool_call);
        frames.push(wire::anthropic_frame(
            "message_delta",
            &json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop_reason, "stop_sequence": Value::Null },
                "usage": usage_value(&self.usage),
            }),
        )?);
        frames.push(wire::anthropic_frame(
            "message_stop",
            &json!({ "type": "message_stop" }),
        )?);
        Ok(frames)
    }

    fn usage(&self) -> Option<RelayUsage> {
        (!self.usage.is_empty()).then(|| self.usage.clone())
    }
}
