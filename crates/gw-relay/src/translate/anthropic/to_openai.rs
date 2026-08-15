//! 格 `C×openai` 与 `C×codex`：入口 `anthropic-messages`，上游 OpenAI Chat wire。
//!
//! 一个转义器覆盖两格，因为 `codex` 与 `openai` 的**上游 wire 协议是同一个**
//! （`UpstreamDialect::OpenAiChat`），只是鉴权与 base-url 不同 —— 那两样归
//! [`crate::engine`] 管，不归这里。
//!
//! 方向是**交叉**的，别看混：
//!
//! | 方法 | 输入 | 输出 |
//! | --- | --- | --- |
//! | [`Translator::translate_request`] | Anthropic Messages 请求 | OpenAI Chat 请求 |
//! | [`Translator::translate_response`] | OpenAI Chat 响应 | Anthropic Messages 响应 |
//! | [`Translator::stream_translator`] | OpenAI SSE | Anthropic SSE |

use bytes::Bytes;
use serde_json::{Map, Value, json};

use super::{
    SseSplit, anthropic_frame, parse_object, parse_upstream_object, present, sse_data, str_at,
    to_bytes,
};
use crate::contract::{
    RelayUsage, StreamTranslator, Surface, TranslateError, Translator, UpstreamDialect,
};

/// `anthropic-messages` → `{openai, codex}` 的转义器。
///
/// 根除的缺陷：**#1**（端点由 provider 猜 —— 这里只改 body）、
/// **#4**（为拿 usage 去改请求结构 —— 这里的 usage 从上游帧里顺出来）。
#[derive(Debug, Clone, Copy, Default)]
pub struct AnthropicToOpenAi;

impl Translator for AnthropicToOpenAi {
    fn surface(&self) -> Surface {
        Surface::AnthropicMessages
    }

    fn to_dialect(&self) -> UpstreamDialect {
        UpstreamDialect::OpenAiChat
    }

    fn translate_request(&self, model: &str, body: &[u8]) -> Result<Bytes, TranslateError> {
        request(model, body)
    }

    fn translate_response(&self, body: &[u8]) -> Result<Bytes, TranslateError> {
        response(body)
    }

    fn stream_translator(&self) -> Box<dyn StreamTranslator> {
        Box::new(OpenAiSseToAnthropic::default())
    }
}

// ======================================================= 请求：Anthropic → OpenAI

fn request(model: &str, src: &[u8]) -> Result<Bytes, TranslateError> {
    let src = parse_object(src)?;
    let mut out = Map::new();
    out.insert("model".to_owned(), Value::String(model.to_owned()));

    let mut messages = Vec::new();
    // 顶层 system → messages[] 里的**首条** role: "system"。
    if let Some(system) = present(&src, "system") {
        messages.push(json!({"role": "system", "content": system_content(system)?}));
    }
    messages.extend(convert_messages(&src)?);
    out.insert("messages".to_owned(), Value::Array(messages));

    for (key, value) in &src {
        match key.as_str() {
            "model" | "messages" | "system" => {}
            "max_tokens" => {
                if !value.is_null() {
                    out.insert("max_tokens".to_owned(), value.clone());
                }
            }
            "temperature" | "top_p" | "stream" => {
                if !value.is_null() {
                    out.insert(key.clone(), value.clone());
                }
            }
            "stop_sequences" => {
                if !value.is_null() {
                    out.insert("stop".to_owned(), value.clone());
                }
            }
            "tools" => {
                if !value.is_null() {
                    out.insert("tools".to_owned(), tools(value)?);
                }
            }
            "metadata" => {
                if let Some(user) = value.get("user_id").and_then(Value::as_str) {
                    out.insert("user".to_owned(), json!(user));
                }
            }
            // 循环后统一处理（一个 Anthropic 字段拆成两个 OpenAI 字段）
            "tool_choice" => {}

            // —— 恒等值可丢，非恒等值必须 400 ——
            "top_k" => {
                return Err(TranslateError::Unsupported(
                    "`top_k` 在 OpenAI Chat Completions 里没有对应旋钮，丢了采样分布就变了"
                        .to_owned(),
                ));
            }
            "thinking" => {
                return Err(TranslateError::Unsupported(
                    "`thinking` 无法映射：OpenAI Chat 的 `reasoning_effort` 是档位，\
                     Anthropic 的是 token 预算，两者没有可逆的对应关系"
                        .to_owned(),
                ));
            }
            "service_tier" => {
                if value.as_str() != Some("auto") && !value.is_null() {
                    return Err(TranslateError::Unsupported(
                        "`service_tier` 的非 auto 取值在两侧的取值域不同，不做猜测映射".to_owned(),
                    ));
                }
            }
            "mcp_servers" | "container" | "betas" | "anthropic_beta" => {
                return Err(TranslateError::Unsupported(format!(
                    "`{key}` 是 Anthropic 独有的服务端能力，OpenAI Chat 不会执行它"
                )));
            }

            other => {
                return Err(TranslateError::Unsupported(format!(
                    "Anthropic 请求字段 `{other}` 在 OpenAI Chat Completions 里没有对应表达。\
                     不认识就说不认识 —— 放行会被上游 400，丢掉会静默改语义"
                )));
            }
        }
    }

    if let Some(choice) = present(&src, "tool_choice") {
        let (mapped, parallel) = tool_choice(choice)?;
        out.insert("tool_choice".to_owned(), mapped);
        if let Some(p) = parallel {
            out.insert("parallel_tool_calls".to_owned(), json!(p));
        }
    }

    to_bytes(&Value::Object(out))
}

/// Anthropic 的顶层 `system` 是「字符串或 text block 数组」。
/// OpenAI 的 system 消息 content 恰好也收这两种，所以是**同构映射**，不做拼接。
fn system_content(system: &Value) -> Result<Value, TranslateError> {
    match system {
        Value::String(_) => Ok(system.clone()),
        Value::Array(blocks) => {
            let mut parts = Vec::with_capacity(blocks.len());
            for block in blocks {
                let text = block
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|_| block.get("type").and_then(Value::as_str) == Some("text"))
                    .ok_or_else(|| {
                        TranslateError::Unsupported(
                            "顶层 `system` 只能是文本块 —— OpenAI 的 system 消息不吃图片或文档"
                                .to_owned(),
                        )
                    })?;
                parts.push(json!({"type": "text", "text": text}));
            }
            Ok(collapse_text_parts(parts))
        }
        _ => Err(TranslateError::Malformed(
            "`system` 必须是字符串或内容块数组".to_owned(),
        )),
    }
}

/// 单个 text part 退化成裸字符串。语义不变，但让**往返转义**回到原形，
/// 也少踩一些上游对 content-part 数组的挑剔。
fn collapse_text_parts(mut parts: Vec<Value>) -> Value {
    if parts.len() == 1
        && let Some(text) = parts[0].get("text").and_then(Value::as_str)
    {
        return Value::String(text.to_owned());
    }
    if parts.is_empty() {
        return Value::String(String::new());
    }
    Value::Array(std::mem::take(&mut parts))
}

// ------------------------------------------------------------------ messages

fn convert_messages(src: &Map<String, Value>) -> Result<Vec<Value>, TranslateError> {
    let items = src
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| TranslateError::Malformed("`messages` 缺失或不是数组".to_owned()))?;

    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let msg = item
            .as_object()
            .ok_or_else(|| TranslateError::Malformed(format!("`messages[{i}]` 不是对象")))?;
        let role = msg
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(|| TranslateError::Malformed(format!("`messages[{i}].role` 缺失")))?;
        match role {
            "user" => user_messages(msg.get("content"), i, &mut out)?,
            "assistant" => out.push(assistant_message(msg.get("content"), i)?),
            other => {
                return Err(TranslateError::Unsupported(format!(
                    "`messages[{i}].role = {other}` 不是 Anthropic Messages 认识的 role"
                )));
            }
        }
    }
    Ok(out)
}

/// 一条 Anthropic user 消息可能展开成**多条** OpenAI 消息：
/// N 个 `tool_result` → N 条 `role: "tool"`，剩下的 text/image → 一条 `role: "user"`。
///
/// 顺序是硬要求：OpenAI 要求 tool 消息紧跟在带 `tool_calls` 的 assistant 消息之后，
/// 所以 tool 消息必须排在同一条 user 消息剩余内容的**前面**。
fn user_messages(
    content: Option<&Value>,
    i: usize,
    out: &mut Vec<Value>,
) -> Result<(), TranslateError> {
    let blocks = match content {
        Some(Value::String(s)) => {
            out.push(json!({"role": "user", "content": s}));
            return Ok(());
        }
        Some(Value::Array(blocks)) => blocks,
        _ => {
            return Err(TranslateError::Malformed(format!(
                "`messages[{i}].content` 必须是字符串或内容块数组"
            )));
        }
    };

    let mut parts = Vec::new();
    let mut tool_messages = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => parts.push(json!({
                "type": "text",
                "text": block.get("text").and_then(Value::as_str).unwrap_or_default(),
            })),
            Some("image") => parts.push(image_part(block)?),
            Some("tool_result") => {
                let id = block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        TranslateError::Malformed(format!(
                            "`messages[{i}]` 的 tool_result 缺 tool_use_id"
                        ))
                    })?;
                tool_messages.push(json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": tool_result_content(block.get("content"))?,
                }));
            }
            Some(other @ ("thinking" | "redacted_thinking")) => {
                return Err(TranslateError::Unsupported(format!(
                    "`{other}` 内容块上的 signature 是多轮 extended thinking 的凭据，\
                     过一遍 OpenAI Chat 就回不来了"
                )));
            }
            Some(other) => {
                return Err(TranslateError::Unsupported(format!(
                    "Anthropic 内容块 `{other}` 在 OpenAI Chat Completions 里没有对应表达"
                )));
            }
            None => {
                return Err(TranslateError::Malformed(format!(
                    "`messages[{i}].content[].type` 缺失"
                )));
            }
        }
    }

    out.append(&mut tool_messages);
    if !parts.is_empty() {
        out.push(json!({"role": "user", "content": collapse_text_parts_or_keep(parts)}));
    }
    Ok(())
}

/// 全是 text 才折叠成裸字符串；混了图片就保持 part 数组。
fn collapse_text_parts_or_keep(parts: Vec<Value>) -> Value {
    if parts
        .iter()
        .all(|p| p.get("type").and_then(Value::as_str) == Some("text"))
    {
        collapse_text_parts(parts)
    } else {
        Value::Array(parts)
    }
}

/// Anthropic 的 base64 source → OpenAI 的 data URI；url source → 原样。
fn image_part(block: &Value) -> Result<Value, TranslateError> {
    let source = block
        .get("source")
        .ok_or_else(|| TranslateError::Malformed("image 块缺 source".to_owned()))?;
    let url = match source.get("type").and_then(Value::as_str) {
        Some("base64") => format!(
            "data:{};base64,{}",
            str_at(Some(source), "media_type"),
            str_at(Some(source), "data")
        ),
        Some("url") => str_at(Some(source), "url").to_owned(),
        other => {
            return Err(TranslateError::Unsupported(format!(
                "image source 类型 `{}` 在 OpenAI Chat Completions 里没有对应表达",
                other.unwrap_or("<缺失>")
            )));
        }
    };
    Ok(json!({"type": "image_url", "image_url": {"url": url}}))
}

/// `tool_result.content` → OpenAI tool 消息的 content。
///
/// 丢掉的只有 `is_error` —— OpenAI 的 tool 消息没有这个字段，而**错误信息本身
/// 在 content 里原样送达**，模型照样看得见「这次工具调用失败了」。
fn tool_result_content(content: Option<&Value>) -> Result<Value, TranslateError> {
    match content {
        None | Some(Value::Null) => Ok(Value::String(String::new())),
        Some(Value::String(s)) => Ok(Value::String(s.clone())),
        Some(Value::Array(blocks)) => {
            let mut parts = Vec::with_capacity(blocks.len());
            for block in blocks {
                let text = block
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|_| block.get("type").and_then(Value::as_str) == Some("text"))
                    .ok_or_else(|| {
                        TranslateError::Unsupported(
                            "OpenAI 的 tool 消息只收文本 —— tool_result 里的图片无处可放"
                                .to_owned(),
                        )
                    })?;
                parts.push(json!({"type": "text", "text": text}));
            }
            Ok(collapse_text_parts(parts))
        }
        Some(_) => Err(TranslateError::Malformed(
            "`tool_result.content` 必须是字符串或内容块数组".to_owned(),
        )),
    }
}

fn assistant_message(content: Option<&Value>, i: usize) -> Result<Value, TranslateError> {
    let mut msg = Map::new();
    msg.insert("role".to_owned(), json!("assistant"));

    let blocks = match content {
        Some(Value::String(s)) => {
            msg.insert("content".to_owned(), json!(s));
            return Ok(Value::Object(msg));
        }
        Some(Value::Array(blocks)) => blocks,
        _ => {
            return Err(TranslateError::Malformed(format!(
                "`messages[{i}].content` 必须是字符串或内容块数组"
            )));
        }
    };

    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => text.push_str(block.get("text").and_then(Value::as_str).unwrap_or("")),
            Some("tool_use") => tool_calls.push(json!({
                "id": block.get("id").cloned().unwrap_or(Value::Null),
                "type": "function",
                "function": {
                    "name": block.get("name").cloned().unwrap_or(Value::Null),
                    // OpenAI 把工具参数编码成**字符串里的 JSON**，Anthropic 直接就是对象。
                    "arguments": serde_json::to_string(block.get("input").unwrap_or(&json!({})))
                        .unwrap_or_else(|_| "{}".to_owned()),
                },
            })),
            Some(other @ ("thinking" | "redacted_thinking")) => {
                return Err(TranslateError::Unsupported(format!(
                    "`{other}` 内容块上的 signature 是多轮 extended thinking 的凭据，\
                     过一遍 OpenAI Chat 就回不来了"
                )));
            }
            Some(other) => {
                return Err(TranslateError::Unsupported(format!(
                    "Anthropic 内容块 `{other}` 在 OpenAI Chat Completions 里没有对应表达"
                )));
            }
            None => {
                return Err(TranslateError::Malformed(format!(
                    "`messages[{i}].content[].type` 缺失"
                )));
            }
        }
    }

    msg.insert(
        "content".to_owned(),
        if text.is_empty() && !tool_calls.is_empty() {
            Value::Null
        } else {
            Value::String(text)
        },
    );
    if !tool_calls.is_empty() {
        msg.insert("tool_calls".to_owned(), Value::Array(tool_calls));
    }
    Ok(Value::Object(msg))
}

// ---------------------------------------------------------------------- tools

/// Anthropic `input_schema` 形状 → OpenAI function 形状。
fn tools(value: &Value) -> Result<Value, TranslateError> {
    let items = value
        .as_array()
        .ok_or_else(|| TranslateError::Malformed("`tools` 必须是数组".to_owned()))?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let tool = item
            .as_object()
            .ok_or_else(|| TranslateError::Malformed("`tools[]` 必须是对象".to_owned()))?;
        // 服务端工具（computer_* / bash_* / text_editor_* / web_search_*）由 Anthropic
        // **自己执行**，OpenAI 上游根本不会跑它。静默丢等于让工具凭空消失。
        match tool.get("type").and_then(Value::as_str) {
            None | Some("custom") => {}
            Some(other) => {
                return Err(TranslateError::Unsupported(format!(
                    "Anthropic 服务端工具 `{other}` 在 OpenAI Chat Completions 上不会被执行"
                )));
            }
        }
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| TranslateError::Malformed("`tools[].name` 缺失".to_owned()))?;
        let mut func = Map::new();
        func.insert("name".to_owned(), json!(name));
        if let Some(desc) = present(tool, "description") {
            func.insert("description".to_owned(), desc.clone());
        }
        func.insert(
            "parameters".to_owned(),
            present(tool, "input_schema")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
        );
        out.push(json!({"type": "function", "function": func}));
    }
    Ok(Value::Array(out))
}

/// 返回 `(OpenAI tool_choice, Option<parallel_tool_calls>)` ——
/// Anthropic 把「选哪个工具」和「能不能并行」塞在同一个对象里，OpenAI 是两个字段。
fn tool_choice(choice: &Value) -> Result<(Value, Option<bool>), TranslateError> {
    let parallel = choice
        .get("disable_parallel_tool_use")
        .and_then(Value::as_bool)
        .map(|disabled| !disabled);
    let mapped = match choice.get("type").and_then(Value::as_str) {
        Some("auto") => json!("auto"),
        Some("none") => json!("none"),
        Some("any") => json!("required"),
        Some("tool") => {
            let name = choice
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| TranslateError::Malformed("`tool_choice.name` 缺失".to_owned()))?;
            json!({"type": "function", "function": {"name": name}})
        }
        other => {
            return Err(TranslateError::Unsupported(format!(
                "`tool_choice.type = {}` 在 OpenAI Chat Completions 里没有对应表达",
                other.unwrap_or("<缺失>")
            )));
        }
    };
    Ok((mapped, parallel))
}

// ====================================================== 响应：OpenAI → Anthropic

fn response(body: &[u8]) -> Result<Bytes, TranslateError> {
    let src = parse_upstream_object(body)?;

    // 上游的错误体也要换方言：客户端 SDK 只会解析它自己那套结构。
    if let Some(err) = src.get("error").and_then(Value::as_object) {
        return to_bytes(&json!({
            "type": "error",
            "error": {
                "type": err.get("type").cloned().unwrap_or(Value::Null),
                "message": err.get("message").cloned().unwrap_or(Value::Null),
            },
        }));
    }

    let choice = src
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .ok_or_else(|| TranslateError::UpstreamShape("OpenAI 响应缺少 choices[0]".to_owned()))?;
    let message = choice.get("message");

    let mut content = Vec::new();
    if let Some(text) = message
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        content.push(json!({"type": "text", "text": text}));
    }
    // `refusal` 是模型确实产出的文本，只是 OpenAI 把它放在另一个字段里。
    if let Some(refusal) = message
        .and_then(|m| m.get("refusal"))
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        content.push(json!({"type": "text", "text": refusal}));
    }
    for call in message
        .and_then(|m| m.get("tool_calls"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let raw = call
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let input: Value = if raw.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(raw).map_err(|e| {
                TranslateError::UpstreamShape(format!("tool_calls[].arguments 不是合法 JSON：{e}"))
            })?
        };
        content.push(json!({
            "type": "tool_use",
            "id": call.get("id").cloned().unwrap_or(Value::Null),
            "name": call.get("function").and_then(|f| f.get("name")).cloned().unwrap_or(Value::Null),
            "input": input,
        }));
    }

    let stop = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map_or(Value::Null, |r| json!(stop_reason(r)));

    let mut out = Map::new();
    out.insert(
        "id".to_owned(),
        src.get("id").cloned().unwrap_or(Value::Null),
    );
    out.insert("type".to_owned(), json!("message"));
    out.insert("role".to_owned(), json!("assistant"));
    out.insert(
        "model".to_owned(),
        src.get("model").cloned().unwrap_or(Value::Null),
    );
    out.insert("content".to_owned(), Value::Array(content));
    out.insert("stop_reason".to_owned(), stop);
    out.insert("stop_sequence".to_owned(), Value::Null);
    if let Some(usage) = src.get("usage") {
        out.insert("usage".to_owned(), anthropic_usage(usage));
    }
    to_bytes(&Value::Object(out))
}

/// OpenAI `finish_reason` → Anthropic `stop_reason`。
fn stop_reason(finish: &str) -> &'static str {
    match finish {
        "length" => "max_tokens",
        "tool_calls" | "function_call" => "tool_use",
        "content_filter" => "refusal",
        // stop 与未知取值都落 end_turn：编一个新值会让 Anthropic SDK 的枚举解析炸掉。
        _ => "end_turn",
    }
}

fn anthropic_usage(usage: &Value) -> Value {
    let mut out = Map::new();
    // 「缺失」与「零」必须能分开：上游没给的字段就不写，不要补 0。
    if let Some(v) = usage.get("prompt_tokens").and_then(Value::as_i64) {
        out.insert("input_tokens".to_owned(), json!(v));
    }
    if let Some(v) = usage.get("completion_tokens").and_then(Value::as_i64) {
        out.insert("output_tokens".to_owned(), json!(v));
    }
    if let Some(v) = usage
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_i64)
    {
        out.insert("cache_read_input_tokens".to_owned(), json!(v));
    }
    Value::Object(out)
}

// ================================================= 流式：OpenAI SSE → Anthropic SSE

/// OpenAI 侧**没有**的框架事件全部由这里合成：`message_start`、
/// `content_block_start` / `content_block_stop`、`message_delta`、`message_stop`。
///
/// 这就是 [`StreamTranslator`] 必须有状态的原因 —— 要记住已经开了哪个
/// content block、发过 `message_start` 没有、下一个 block 该是几号。
#[derive(Debug, Default)]
struct OpenAiSseToAnthropic {
    split: SseSplit,
    id: String,
    model: String,
    /// `message_start` 已经发过。**必须先于任何 `content_block_delta`**。
    started: bool,
    /// 当前打开的 content block：`(index, 是不是 tool_use)`。
    /// 每个 `content_block_start` 都要配一个 `content_block_stop`。
    open: Option<(i64, bool)>,
    /// 下一个 content block 用的 index。
    next_index: i64,
    /// 当前打开的 tool block 对应 OpenAI 的哪个 `tool_calls[].index`。
    tool_slot: Option<i64>,
    stop_reason: Option<&'static str>,
    usage: RelayUsage,
    /// `message_stop` 已经发过。**必须最后且只有一次**。
    done: bool,
}

impl StreamTranslator for OpenAiSseToAnthropic {
    fn push(&mut self, upstream_frame: &[u8]) -> Result<Vec<Bytes>, TranslateError> {
        let mut out = Vec::new();
        for event in self.split.push(upstream_frame) {
            self.handle(&event, &mut out)?;
        }
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<Bytes>, TranslateError> {
        let mut out = Vec::new();
        if let Some(rest) = self.split.flush() {
            self.handle(&rest, &mut out)?;
        }
        self.finalize(&mut out);
        Ok(out)
    }

    fn usage(&self) -> Option<RelayUsage> {
        (!self.usage.is_empty()).then(|| self.usage.clone())
    }
}

impl OpenAiSseToAnthropic {
    fn handle(&mut self, event: &[u8], out: &mut Vec<Bytes>) -> Result<(), TranslateError> {
        let Some(data) = sse_data(event) else {
            return Ok(()); // 注释帧 / 只有 event: 的帧
        };
        if data.trim_ascii() == b"[DONE]" {
            self.finalize(out);
            return Ok(());
        }
        let Ok(Value::Object(chunk)) = serde_json::from_slice::<Value>(&data) else {
            return Ok(()); // 心跳与任何非 JSON 载荷：跳过，不打断在途的流
        };

        if let Some(err) = chunk.get("error").filter(|v| !v.is_null()) {
            // 中途失败必须让客户端**察觉**（缺陷 #6）：上抛 → RST_STREAM / 掐连接。
            return Err(TranslateError::UpstreamShape(format!(
                "上游流中途报错：{err}"
            )));
        }

        if !self.started {
            self.id = chunk
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            self.model = chunk
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            self.started = true;
            out.push(anthropic_frame(
                "message_start",
                &json!({
                    "type": "message_start",
                    "message": {
                        "id": self.id,
                        "type": "message",
                        "role": "assistant",
                        "model": self.model,
                        "content": [],
                        "stop_reason": null,
                        "stop_sequence": null,
                        // 真值在 message_delta 里 —— OpenAI 的 usage 在**末帧**，
                        // 而 message_start 必须是**首帧**，这两条不可能同时满足。
                        "usage": {"input_tokens": 0, "output_tokens": 0},
                    },
                }),
            ));
        }

        if let Some(usage) = chunk.get("usage").filter(|v| !v.is_null()) {
            self.absorb_usage(usage);
        }

        let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
        else {
            return Ok(()); // usage-only 帧（`choices: []`）
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.stop_reason = Some(stop_reason(reason));
        }
        let delta = choice.get("delta");

        // `content` 与 `refusal` 都是模型产出的文本，只是 OpenAI 分了两个字段。
        for key in ["content", "refusal"] {
            let text = str_at(delta, key);
            if text.is_empty() {
                continue;
            }
            self.open_text(out);
            out.push(anthropic_frame(
                "content_block_delta",
                &json!({
                    "type": "content_block_delta",
                    "index": self.open.map_or(0, |(i, _)| i),
                    "delta": {"type": "text_delta", "text": text},
                }),
            ));
        }

        for call in delta
            .and_then(|d| d.get("tool_calls"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            self.tool_call_delta(call, out);
        }
        Ok(())
    }

    /// OpenAI 的 `tool_calls[].index` 是**工具调用序号**，与 Anthropic 的
    /// content block index 不是一回事 —— 序号一变就要换一个 content block。
    fn tool_call_delta(&mut self, call: &Value, out: &mut Vec<Bytes>) {
        let slot = call.get("index").and_then(Value::as_i64).unwrap_or(0);
        if self.tool_slot != Some(slot) {
            self.close_open(out);
            let index = self.next_index;
            self.next_index += 1;
            self.open = Some((index, true));
            self.tool_slot = Some(slot);
            out.push(anthropic_frame(
                "content_block_start",
                &json!({
                    "type": "content_block_start",
                    "index": index,
                    "content_block": {
                        "type": "tool_use",
                        "id": str_at(Some(call), "id"),
                        "name": call.get("function").and_then(|f| f.get("name"))
                            .and_then(Value::as_str).unwrap_or_default(),
                        "input": {},
                    },
                }),
            ));
        }
        let args = call
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !args.is_empty() {
            out.push(anthropic_frame(
                "content_block_delta",
                &json!({
                    "type": "content_block_delta",
                    "index": self.open.map_or(0, |(i, _)| i),
                    // 分片原样转发：拼接是客户端的事，网关插手只会丢字节。
                    "delta": {"type": "input_json_delta", "partial_json": args},
                }),
            ));
        }
    }

    fn open_text(&mut self, out: &mut Vec<Bytes>) {
        if matches!(self.open, Some((_, false))) {
            return;
        }
        self.close_open(out);
        let index = self.next_index;
        self.next_index += 1;
        self.open = Some((index, false));
        out.push(anthropic_frame(
            "content_block_start",
            &json!({
                "type": "content_block_start",
                "index": index,
                "content_block": {"type": "text", "text": ""},
            }),
        ));
    }

    fn close_open(&mut self, out: &mut Vec<Bytes>) {
        if let Some((index, _)) = self.open.take() {
            self.tool_slot = None;
            out.push(anthropic_frame(
                "content_block_stop",
                &json!({"type": "content_block_stop", "index": index}),
            ));
        }
    }

    /// 收尾三帧：关掉在开的 block、`message_delta`（**usage 在这里**）、`message_stop`。
    fn finalize(&mut self, out: &mut Vec<Bytes>) {
        // 一帧都没收到就别凭空造一个流：让上层看见「上游什么都没给」。
        if !self.started || self.done {
            return;
        }
        self.close_open(out);
        let mut usage = Map::new();
        if let Some(v) = self.usage.input_tokens {
            usage.insert("input_tokens".to_owned(), json!(v));
        }
        if let Some(v) = self.usage.output_tokens {
            usage.insert("output_tokens".to_owned(), json!(v));
        }
        if let Some(v) = self.usage.cached_tokens {
            usage.insert("cache_read_input_tokens".to_owned(), json!(v));
        }
        out.push(anthropic_frame(
            "message_delta",
            &json!({
                "type": "message_delta",
                "delta": {
                    "stop_reason": self.stop_reason.map_or(Value::Null, |r| json!(r)),
                    "stop_sequence": null,
                },
                "usage": usage,
            }),
        ));
        out.push(anthropic_frame(
            "message_stop",
            &json!({"type": "message_stop"}),
        ));
        self.done = true;
    }

    /// **绝不能丢 usage**：OpenAI 把全部计数放在末帧。
    fn absorb_usage(&mut self, usage: &Value) {
        if let Some(v) = usage.get("prompt_tokens").and_then(Value::as_i64) {
            self.usage.input_tokens = Some(v);
        }
        if let Some(v) = usage.get("completion_tokens").and_then(Value::as_i64) {
            self.usage.output_tokens = Some(v);
        }
        if let Some(v) = usage
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(Value::as_i64)
        {
            self.usage.cached_tokens = Some(v);
        }
        if let Some(v) = usage
            .get("completion_tokens_details")
            .and_then(|d| d.get("reasoning_tokens"))
            .and_then(Value::as_i64)
        {
            self.usage.reasoning_tokens = Some(v);
        }
    }
}
