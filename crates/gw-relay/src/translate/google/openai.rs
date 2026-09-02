//! `openai-completions × {gemini, vertex}` —— 15 格矩阵里的 P1 两格。
//!
//! OWNER: worker `relay-google`。
//!
//! 入口方言 [`Surface::OpenAiCompletions`] ↔ 上游方言
//! [`UpstreamDialect::GoogleGenerateContent`]。

use std::collections::HashMap;

use bytes::Bytes;
use serde_json::{Map, Value, json};

use super::wire::{self, GenerateContentResponse, GenerationConfig};
use crate::contract::{
    RelayUsage, StreamTranslator, Surface, TranslateError, Translator, UpstreamDialect,
};

/// `POST /v1/chat/completions` ↔ Google GenerateContent 的转义器。
///
/// 根除 `docs/relay-passthrough-audit.md` 缺陷 **#1**（S1）在 gemini/vertex 上的
/// 那一半：今天这两格是**直通**的，OpenAI 形状的 body 被原样 POST 给
/// GenerateContent，上游必 400（`docs/relay-surface-plan.md` §3.6 A×gemini /
/// A×vertex 两格的「今天」列）。协议翻译从此是一次**显式的** [`Translator`]
/// 调用，而不是藏在 provider 内部的隐式改写。
///
/// 同时按缺陷 **#4** 的教训行事：**不做整体 JSON round-trip 后再塞字段**。
/// 这里是按字段重建，客户端没写的东西不会凭空出现 —— 尤其是流式方向
/// **不会**合成一帧客户端没要过的 `usage` chunk（那正是 #4 里让手写
/// `chunk.choices[0]` 的客户端抛 `IndexError` 的那一帧）。usage 走
/// [`StreamTranslator::usage`] 交给计费，不进回写路径。
///
/// # 不变量
///
/// - 未知的顶层字段一律 [`TranslateError::Unsupported`] → 400，绝不静默丢弃。
/// - `usageMetadata` 的四个计数一个都不丢，且「缺失」与「零」可分。
/// - 流式产出的帧序列在 OpenAI 方言里合法：`data: [DONE]` 最后且只有一次。
#[derive(Debug, Default, Clone, Copy)]
pub struct OpenAiToGoogle;

impl Translator for OpenAiToGoogle {
    fn surface(&self) -> Surface {
        Surface::OpenAiCompletions
    }

    fn to_dialect(&self) -> UpstreamDialect {
        UpstreamDialect::GoogleGenerateContent
    }

    fn translate_request(&self, _model: &str, body: &[u8]) -> Result<Bytes, TranslateError> {
        // `model` 走 URL（`…/models/{m}:generateContent`），不进 body —— 塞进去
        // Google 会以 `Unknown name "model"` 拒绝。端点拼接归 engine。
        build_request(body)
    }

    fn translate_response(&self, body: &[u8]) -> Result<Bytes, TranslateError> {
        build_response(body)
    }

    fn stream_translator(&self) -> Box<dyn StreamTranslator> {
        Box::new(OpenAiStream::default())
    }
}

// ===================================================================== 请求

/// 会被翻译过去的顶层字段。
const REQ_MAPPED: &[&str] = &[
    "model",
    "messages",
    "temperature",
    "top_p",
    "max_tokens",
    "max_completion_tokens",
    "stop",
    "n",
    "seed",
    "frequency_penalty",
    "presence_penalty",
    "tools",
    "tool_choice",
    "response_format",
    "parallel_tool_calls",
];

/// **确认可以静默丢弃**的顶层字段，逐条理由见 [`super`] 的模块文档。
const REQ_DROPPED: &[&str] = &[
    "stream",
    "stream_options",
    "user",
    "safety_identifier",
    "prompt_cache_key",
    "metadata",
    "store",
    "service_tier",
];

const MSG_MAPPED: &[&str] = &["role", "content", "tool_calls", "tool_call_id"];
const MSG_DROPPED: &[&str] = &["name", "refusal", "annotations"];

fn build_request(body: &[u8]) -> Result<Bytes, TranslateError> {
    let root = wire::as_object(body)?;
    wire::reject_unknown(&root, REQ_MAPPED, REQ_DROPPED, "chat.completions")?;
    wire::reject_non_default_bool(
        root.get("parallel_tool_calls"),
        true,
        "chat.completions.parallel_tool_calls",
    )?;
    // Google 的 `candidateCount` 确实能要多个候选，但流式方向要按
    // `candidates[].index` 把多路候选分发进多个 `choices[]`，代价远高于它的
    // 实际用量；而只在非流式支持、流式静默只回第一个，就是两条路径行为不一致
    // 的静默正确性错误。统一拒掉，`n = 1` 是 Google 的默认值故无需下发。
    if let Some(n) = root.get("n").filter(|v| !v.is_null())
        && n.as_i64() != Some(1)
    {
        return Err(TranslateError::Unsupported(
            "chat.completions.n > 1 is not translated to GenerateContent: the streaming \
             direction would have to fan multiple candidates into multiple choices[]"
                .to_owned(),
        ));
    }

    let messages = root
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            TranslateError::Malformed("`messages` is required and must be an array".to_owned())
        })?;

    let mut contents: Vec<Value> = Vec::with_capacity(messages.len());
    let mut system_parts: Vec<Value> = Vec::new();
    // tool_call_id → function name。按消息顺序增量建表，后写覆盖先写：
    // 命中的永远是**紧邻的上一轮** assistant tool_call，这正是要的语义。
    let mut tool_names: HashMap<String, String> = HashMap::new();

    for message in messages {
        let msg = message.as_object().ok_or_else(|| {
            TranslateError::Malformed("each message must be a JSON object".to_owned())
        })?;
        wire::reject_unknown(msg, MSG_MAPPED, MSG_DROPPED, "chat.completions.messages[]")?;
        let role = msg.get("role").and_then(Value::as_str).ok_or_else(|| {
            TranslateError::Malformed("each message needs a string `role`".to_owned())
        })?;

        match role {
            "system" | "developer" => system_parts.extend(text_parts(msg.get("content"))?),
            "user" => push_content(&mut contents, "user", content_parts(msg.get("content"))?),
            "assistant" => {
                let mut parts = content_parts(msg.get("content"))?;
                parts.extend(assistant_tool_calls(
                    msg.get("tool_calls"),
                    &mut tool_names,
                )?);
                push_content(&mut contents, "model", parts);
            }
            "tool" => push_content(&mut contents, "user", vec![tool_result(msg, &tool_names)?]),
            other => {
                return Err(TranslateError::Unsupported(format!(
                    "chat.completions.messages[].role `{other}` has no GenerateContent equivalent"
                )));
            }
        }
    }

    let mut out = Map::new();
    out.insert("contents".to_owned(), Value::Array(contents));
    if !system_parts.is_empty() {
        out.insert(
            "systemInstruction".to_owned(),
            json!({ "parts": system_parts }),
        );
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

/// Google 不接受 `parts` 为空的 content —— 只有非空才落盘。
fn push_content(contents: &mut Vec<Value>, role: &str, parts: Vec<Value>) {
    if !parts.is_empty() {
        contents.push(json!({ "role": role, "parts": parts }));
    }
}

fn generation_config(root: &Map<String, Value>) -> Result<GenerationConfig, TranslateError> {
    let mut cfg = GenerationConfig::default();
    cfg.set("temperature", root.get("temperature"));
    cfg.set("topP", root.get("top_p"));
    // `max_completion_tokens` 是 `max_tokens` 的继任者，两个都落同一个旋钮，
    // 同时出现时以新的为准（OpenAI 自己也是这么定的）。
    cfg.set(
        "maxOutputTokens",
        root.get("max_completion_tokens")
            .filter(|v| !v.is_null())
            .or_else(|| root.get("max_tokens")),
    );
    cfg.set("seed", root.get("seed"));
    cfg.set("frequencyPenalty", root.get("frequency_penalty"));
    cfg.set("presencePenalty", root.get("presence_penalty"));
    if let Some(stop) = root.get("stop").filter(|v| !v.is_null()) {
        cfg.put("stopSequences", wire::stop_sequences(stop)?);
    }
    if let Some(mime) = response_mime_type(root.get("response_format"))? {
        cfg.put("responseMimeType", Value::String(mime.to_owned()));
    }
    Ok(cfg)
}

/// `response_format` → `generationConfig.responseMimeType`。
///
/// `json_schema` 走 [`TranslateError::Unsupported`]：Google 的 `responseSchema`
/// 只认 OpenAPI 3.0 子集，而 OpenAI 的 strict schema 必带 `additionalProperties:
/// false`（还常带 `$defs` / `$ref`），Google 一律拒。把 schema 悄悄摘掉只留
/// `application/json`，客户端会拿到一个不符合它 schema 的 JSON 并按 schema 去解 ——
/// 一个静默的正确性错误，比一个 400 坏得多。
fn response_mime_type(value: Option<&Value>) -> Result<Option<&'static str>, TranslateError> {
    let Some(fmt) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    match fmt.get("type").and_then(Value::as_str) {
        Some("text") | None => Ok(None),
        Some("json_object") => Ok(Some("application/json")),
        Some(other) => Err(TranslateError::Unsupported(format!(
            "chat.completions.response_format `{other}` cannot be expressed as a \
             GenerateContent responseSchema without changing the schema's meaning"
        ))),
    }
}

/// `content` 是 `null` / 字符串 / 多模态数组三态。
fn content_parts(value: Option<&Value>) -> Result<Vec<Value>, TranslateError> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(text)) => Ok(vec![json!({ "text": text })]),
        Some(Value::Array(items)) => items.iter().map(content_part).collect(),
        Some(other) => Err(TranslateError::Malformed(format!(
            "message content must be a string or an array, got {}",
            wire::kind_of(other)
        ))),
    }
}

fn content_part(item: &Value) -> Result<Value, TranslateError> {
    match item.get("type").and_then(Value::as_str) {
        Some("text") => {
            Ok(json!({ "text": item.get("text").and_then(Value::as_str).unwrap_or("") }))
        }
        Some("image_url") => {
            let url = item
                .get("image_url")
                .and_then(|v| v.get("url"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    TranslateError::Malformed("image_url part needs `image_url.url`".to_owned())
                })?;
            wire::data_url_to_inline(url).ok_or_else(|| {
                TranslateError::Unsupported(
                    "chat.completions image_url must be a `data:<mime>;base64,` URL: \
                     GenerateContent has no equivalent for a remote http(s) image"
                        .to_owned(),
                )
            })
        }
        Some(other) => Err(TranslateError::Unsupported(format!(
            "chat.completions content part `{other}` has no GenerateContent equivalent"
        ))),
        None => Err(TranslateError::Malformed(
            "each content part needs a string `type`".to_owned(),
        )),
    }
}

/// system / developer 消息只取文本 —— Google 的 `systemInstruction` 不收图片。
fn text_parts(value: Option<&Value>) -> Result<Vec<Value>, TranslateError> {
    let parts = content_parts(value)?;
    if parts.iter().any(|p| p.get("text").is_none()) {
        return Err(TranslateError::Unsupported(
            "chat.completions system message: GenerateContent's systemInstruction \
             accepts text only"
                .to_owned(),
        ));
    }
    Ok(parts)
}

fn assistant_tool_calls(
    value: Option<&Value>,
    names: &mut HashMap<String, String>,
) -> Result<Vec<Value>, TranslateError> {
    let Some(calls) = value.filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let calls = calls
        .as_array()
        .ok_or_else(|| TranslateError::Malformed("`tool_calls` must be an array".to_owned()))?;
    calls
        .iter()
        .map(|call| {
            let function = call.get("function").ok_or_else(|| {
                TranslateError::Malformed("tool_call needs a `function`".to_owned())
            })?;
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    TranslateError::Malformed("tool_call.function needs a `name`".to_owned())
                })?;
            if let Some(id) = call.get("id").and_then(Value::as_str) {
                names.insert(id.to_owned(), name.to_owned());
            }
            // OpenAI 的 arguments 是一个**JSON 字符串**，Google 的 args 是对象。
            let raw = function
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let args: Value = serde_json::from_str(if raw.trim().is_empty() { "{}" } else { raw })
                .map_err(|err| {
                    TranslateError::Malformed(format!("tool_call.function.arguments: {err}"))
                })?;
            Ok(json!({ "functionCall": { "name": name, "args": args } }))
        })
        .collect()
}

fn tool_result(
    msg: &Map<String, Value>,
    names: &HashMap<String, String>,
) -> Result<Value, TranslateError> {
    let id = msg
        .get("tool_call_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            TranslateError::Malformed("a `tool` message needs `tool_call_id`".to_owned())
        })?;
    // Google 的 functionResponse 按**名字**匹配 functionCall，没有 id 这个概念。
    // 匹配不上就只能 400：随便编一个名字，模型会拿到一个它没调用过的工具的结果。
    let name = names.get(id).ok_or_else(|| {
        TranslateError::Unsupported(format!(
            "chat.completions tool message references tool_call_id `{id}` that no \
             preceding assistant message declared; GenerateContent matches function \
             responses by name, so the name cannot be recovered"
        ))
    })?;
    let response = match msg.get("content") {
        Some(Value::String(text)) => json!({ "content": text }),
        Some(Value::Array(items)) => json!({ "content": Value::Array(items.clone()) }),
        None | Some(Value::Null) => json!({}),
        Some(other) => json!({ "content": other.clone() }),
    };
    Ok(json!({ "functionResponse": { "name": name, "response": response } }))
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
        .map(|tool| match tool.get("type").and_then(Value::as_str) {
            Some("function") | None => {
                let function = tool.get("function").ok_or_else(|| {
                    TranslateError::Malformed("tool needs a `function`".to_owned())
                })?;
                let mut decl = Map::new();
                let name = function
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        TranslateError::Malformed("tool.function needs a `name`".to_owned())
                    })?;
                decl.insert("name".to_owned(), Value::String(name.to_owned()));
                if let Some(desc) = function.get("description").filter(|v| !v.is_null()) {
                    decl.insert("description".to_owned(), desc.clone());
                }
                if let Some(params) = function.get("parameters").filter(|v| !v.is_null()) {
                    decl.insert("parameters".to_owned(), params.clone());
                }
                Ok(Value::Object(decl))
            }
            Some(other) => Err(TranslateError::Unsupported(format!(
                "chat.completions tool type `{other}` has no GenerateContent equivalent"
            ))),
        })
        .collect()
}

fn tool_config(value: Option<&Value>) -> Result<Option<Value>, TranslateError> {
    let Some(choice) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let mode = match choice {
        Value::String(s) => match s.as_str() {
            "auto" => "AUTO",
            "none" => "NONE",
            "required" => "ANY",
            other => {
                return Err(TranslateError::Unsupported(format!(
                    "chat.completions.tool_choice `{other}` has no GenerateContent equivalent"
                )));
            }
        },
        Value::Object(_) => {
            let name = choice
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    TranslateError::Malformed("object tool_choice needs `function.name`".to_owned())
                })?;
            return Ok(Some(json!({
                "functionCallingConfig": { "mode": "ANY", "allowedFunctionNames": [name] }
            })));
        }
        other => {
            return Err(TranslateError::Malformed(format!(
                "tool_choice must be a string or an object, got {}",
                wire::kind_of(other)
            )));
        }
    };
    Ok(Some(json!({ "functionCallingConfig": { "mode": mode } })))
}

// ============================================================ 响应（非流式）

fn build_response(body: &[u8]) -> Result<Bytes, TranslateError> {
    let resp = wire::parse_response(body)?;
    let mut usage = RelayUsage::default();
    if let Some(meta) = resp.usage_metadata.as_ref() {
        wire::merge_usage(&mut usage, meta);
    }

    let candidate = resp.candidates.first();
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    if let Some(content) = candidate.and_then(|c| c.content.as_ref()) {
        for (idx, part) in content.parts.iter().enumerate() {
            if let Some(call) = part.function_call.as_ref() {
                tool_calls.push(json!({
                    "index": tool_calls.len(),
                    "id": format!("call_{idx}"),
                    "type": "function",
                    "function": {
                        "name": call.name,
                        "arguments": serde_json::to_string(&call.args)
                            .unwrap_or_else(|_| "{}".to_owned()),
                    }
                }));
            }
            if let Some(chunk) = part.text.as_deref() {
                if part.thought.unwrap_or(false) {
                    reasoning.push_str(chunk);
                } else {
                    text.push_str(chunk);
                }
            }
        }
    }

    let raw_finish = candidate
        .and_then(|c| c.finish_reason.as_deref())
        .or_else(|| {
            resp.prompt_feedback
                .as_ref()
                .and_then(|f| f.block_reason.as_deref())
                .map(|_| "SAFETY")
        })
        .unwrap_or("STOP");

    let mut message = Map::new();
    message.insert("role".to_owned(), Value::String("assistant".to_owned()));
    message.insert(
        "content".to_owned(),
        if text.is_empty() && !tool_calls.is_empty() {
            Value::Null
        } else {
            Value::String(text)
        },
    );
    if !reasoning.is_empty() {
        // OpenAI 方言没有标准的思考字段。`reasoning_content` 是事实标准，
        // 不认识它的客户端会忽略未知字段 —— 加一个键的风险远小于把模型
        // 真的产出的一段文本丢掉。
        message.insert("reasoning_content".to_owned(), Value::String(reasoning));
    }
    let has_tool_call = !tool_calls.is_empty();
    if has_tool_call {
        message.insert("tool_calls".to_owned(), Value::Array(tool_calls));
    }

    let mut out = Map::new();
    out.insert(
        "id".to_owned(),
        Value::String(
            resp.response_id
                .unwrap_or_else(|| wire::synthetic_id("chatcmpl-")),
        ),
    );
    out.insert(
        "object".to_owned(),
        Value::String("chat.completion".to_owned()),
    );
    out.insert("created".to_owned(), json!(wire::unix_secs()));
    if let Some(model) = resp.model_version {
        out.insert("model".to_owned(), Value::String(model));
    }
    out.insert(
        "choices".to_owned(),
        json!([{
            "index": 0,
            "message": Value::Object(message),
            "finish_reason": wire::openai_finish_reason(raw_finish, has_tool_call),
        }]),
    );
    if let Some(u) = usage_value(&usage) {
        out.insert("usage".to_owned(), u);
    }

    serde_json::to_vec(&Value::Object(out))
        .map(Bytes::from)
        .map_err(|err| TranslateError::UpstreamShape(err.to_string()))
}

/// [`RelayUsage`] → OpenAI 的 `usage` 信封。
///
/// `completion_tokens` **加上** `reasoning_tokens`：OpenAI 的约定是
/// `completion_tokens` 含推理 token（`reasoning_tokens` 只是它的明细），
/// 而 Google 的 `candidatesTokenCount` **不含** `thoughtsTokenCount`。
/// 不加，思考型模型在客户端侧看到的输出量就是少的。
/// 注意计费拿到的 [`RelayUsage`] 是**原始值**，不做这个加法（见 [`super`]）。
fn usage_value(usage: &RelayUsage) -> Option<Value> {
    if usage.is_empty() {
        return None;
    }
    let mut out = Map::new();
    let completion = match (usage.output_tokens, usage.reasoning_tokens) {
        (Some(o), Some(r)) => Some(o + r),
        (Some(o), None) => Some(o),
        (None, r) => r,
    };
    if let Some(v) = usage.input_tokens {
        out.insert("prompt_tokens".to_owned(), json!(v));
    }
    if let Some(v) = completion {
        out.insert("completion_tokens".to_owned(), json!(v));
    }
    if let (Some(p), Some(c)) = (usage.input_tokens, completion) {
        out.insert("total_tokens".to_owned(), json!(p + c));
    }
    if let Some(v) = usage.cached_tokens {
        out.insert(
            "prompt_tokens_details".to_owned(),
            json!({ "cached_tokens": v }),
        );
    }
    if let Some(v) = usage.reasoning_tokens {
        out.insert(
            "completion_tokens_details".to_owned(),
            json!({ "reasoning_tokens": v }),
        );
    }
    Some(Value::Object(out))
}

// ============================================================== 响应（流式）

/// OpenAI 方向的流式状态机。
///
/// 状态很轻（只有 id / model / 是否发过 role / 是否发过 finish_reason），
/// 但**必须**有：`delta.role` 只能出现在第一帧，`data: [DONE]` 只能出现一次
/// 且必须最后 —— 这两条都是跨帧的性质，无状态翻不出来。
#[derive(Default)]
struct OpenAiStream {
    sse: wire::SseDecoder,
    id: Option<String>,
    created: i64,
    model: Option<String>,
    role_sent: bool,
    finished: bool,
    done: bool,
    usage: RelayUsage,
}

impl OpenAiStream {
    fn envelope(&mut self, resp: &GenerateContentResponse) -> Map<String, Value> {
        if self.id.is_none() {
            self.id = Some(
                resp.response_id
                    .clone()
                    .unwrap_or_else(|| wire::synthetic_id("chatcmpl-")),
            );
            self.created = wire::unix_secs();
        }
        if self.model.is_none() {
            self.model.clone_from(&resp.model_version);
        }
        let mut out = Map::new();
        out.insert(
            "id".to_owned(),
            Value::String(self.id.clone().unwrap_or_default()),
        );
        out.insert(
            "object".to_owned(),
            Value::String("chat.completion.chunk".to_owned()),
        );
        out.insert("created".to_owned(), json!(self.created));
        if let Some(model) = self.model.as_ref() {
            out.insert("model".to_owned(), Value::String(model.clone()));
        }
        out
    }
}

impl StreamTranslator for OpenAiStream {
    fn push(&mut self, upstream_frame: &[u8]) -> Result<Vec<Bytes>, TranslateError> {
        let mut frames = Vec::new();
        for payload in self.sse.push(upstream_frame)? {
            if !wire::is_parseable(&payload) {
                continue;
            }
            let resp = wire::parse_response(&payload)?;
            if let Some(meta) = resp.usage_metadata.as_ref() {
                wire::merge_usage(&mut self.usage, meta);
            }

            let mut delta = Map::new();
            let mut text = String::new();
            let mut reasoning = String::new();
            let mut tool_calls: Vec<Value> = Vec::new();
            let candidate = resp.candidates.first();
            if let Some(content) = candidate.and_then(|c| c.content.as_ref()) {
                for (idx, part) in content.parts.iter().enumerate() {
                    if let Some(call) = part.function_call.as_ref() {
                        tool_calls.push(json!({
                            "index": tool_calls.len(),
                            "id": format!("call_{idx}"),
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": serde_json::to_string(&call.args)
                                    .unwrap_or_else(|_| "{}".to_owned()),
                            }
                        }));
                    }
                    if let Some(chunk) = part.text.as_deref() {
                        if part.thought.unwrap_or(false) {
                            reasoning.push_str(chunk);
                        } else {
                            text.push_str(chunk);
                        }
                    }
                }
            }

            let raw_finish = candidate
                .and_then(|c| c.finish_reason.as_deref())
                .or_else(|| {
                    resp.prompt_feedback
                        .as_ref()
                        .and_then(|f| f.block_reason.as_deref())
                        .map(|_| "SAFETY")
                });
            let has_tool_call = !tool_calls.is_empty();
            if text.is_empty()
                && reasoning.is_empty()
                && !has_tool_call
                && raw_finish.is_none()
                && resp.candidates.is_empty()
            {
                // 上游的纯 usage 心跳帧在 OpenAI 方言里没有对应物 —— 零帧合法。
                continue;
            }

            if !self.role_sent {
                delta.insert("role".to_owned(), Value::String("assistant".to_owned()));
                self.role_sent = true;
            }
            if !text.is_empty() {
                delta.insert("content".to_owned(), Value::String(text));
            }
            if !reasoning.is_empty() {
                delta.insert("reasoning_content".to_owned(), Value::String(reasoning));
            }
            if has_tool_call {
                delta.insert("tool_calls".to_owned(), Value::Array(tool_calls));
            }

            let finish_reason = raw_finish.map(|r| {
                self.finished = true;
                Value::String(wire::openai_finish_reason(r, has_tool_call).to_owned())
            });
            let mut chunk = self.envelope(&resp);
            chunk.insert(
                "choices".to_owned(),
                json!([{
                    "index": 0,
                    "delta": Value::Object(delta),
                    "finish_reason": finish_reason.unwrap_or(Value::Null),
                }]),
            );
            frames.push(wire::openai_frame(&Value::Object(chunk))?);
        }
        Ok(frames)
    }

    fn finish(&mut self) -> Result<Vec<Bytes>, TranslateError> {
        if self.done {
            return Ok(Vec::new());
        }
        // A compliant SSE event ends with a blank line. Appending one at EOF
        // also turns a truncated final JSON event into a visible parse error
        // instead of synthesising a clean stop for an incomplete answer.
        let mut frames = self.push(b"\n\n")?;
        if !self.finished {
            // 上游流结束却没给过 finishReason。OpenAI 客户端在等一个非 null 的
            // `finish_reason` 才会认为本轮结束，不补就是缺陷 #6 那种「干净的
            // EOF」—— 客户端拿到截断回答却不报错。
            self.finished = true;
            let mut chunk = self.envelope(&GenerateContentResponse::default());
            chunk.insert(
                "choices".to_owned(),
                json!([{ "index": 0, "delta": {}, "finish_reason": "stop" }]),
            );
            frames.push(wire::openai_frame(&Value::Object(chunk))?);
        }
        self.done = true;
        frames.push(Bytes::from_static(b"data: [DONE]\n\n"));
        Ok(frames)
    }

    fn usage(&self) -> Option<RelayUsage> {
        (!self.usage.is_empty()).then(|| self.usage.clone())
    }
}
