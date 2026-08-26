//! 格 `A×claude`：入口 `openai-completions`，上游 `claude`。
//!
//! 方向是**交叉**的，别看混：
//!
//! | 方法 | 输入 | 输出 |
//! | --- | --- | --- |
//! | [`Translator::translate_request`] | OpenAI Chat 请求 | Anthropic Messages 请求 |
//! | [`Translator::translate_response`] | Anthropic Messages 响应 | OpenAI Chat 响应 |
//! | [`Translator::stream_translator`] | Anthropic SSE | OpenAI SSE |

use bytes::Bytes;
use serde_json::{Map, Value, json};

use super::{
    OPENAI_DONE, SseSplit, openai_frame, parse_object, parse_upstream_object, present, sse_data,
    str_at, to_bytes,
};
use crate::contract::{
    RelayUsage, StreamTranslator, Surface, TranslateError, Translator, UpstreamDialect,
};

/// Anthropic Messages 的 `max_tokens` 是**必填**，OpenAI Chat 的是选填。
/// 客户端没写时补这个值。
///
/// # 为什么是 4096，不是更大也不是更小
///
/// 它是**在售 Claude 模型 `max_tokens` 上限的下确界**：`claude-3-opus` 与
/// `claude-3-haiku` 的上限就是 4096。取更大的值（比如 8192）会在这两个模型上
/// 被上游直接 400 `max_tokens: 8192 > 4096` —— 而客户端**根本没写过这个字段**，
/// 它会收到一个自己既无法解释、也无法规避的错误。取更小的值则会无谓地截断回答。
///
/// 客户端写了 `max_tokens` 或 `max_completion_tokens` 时这个值不参与任何计算，
/// 原样透传（包括超过模型上限的值 —— 那是客户端与上游之间的事，网关不代为裁剪）。
pub const DEFAULT_MAX_TOKENS: i64 = 4096;

/// `openai-completions` → `claude` 的转义器。
///
/// 根除的缺陷：**#1**（端点由 provider 猜 —— 这里只改 body，URL 由
/// [`crate::engine`] 用 origin + 入站 path 拼）、**#4**（为拿 usage 去改请求结构 ——
/// 这里的 usage 从上游帧里顺出来，请求体一个字节都不为计费而改）。
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenAiToAnthropic;

impl Translator for OpenAiToAnthropic {
    fn surface(&self) -> Surface {
        Surface::OpenAiCompletions
    }

    fn to_dialect(&self) -> UpstreamDialect {
        UpstreamDialect::AnthropicMessages
    }

    fn translate_request(&self, model: &str, body: &[u8]) -> Result<Bytes, TranslateError> {
        request(model, body)
    }

    fn translate_response(&self, body: &[u8]) -> Result<Bytes, TranslateError> {
        response(body)
    }

    fn stream_translator(&self) -> Box<dyn StreamTranslator> {
        Box::new(AnthropicSseToOpenAi::default())
    }
}

// ======================================================= 请求：OpenAI → Anthropic

fn request(model: &str, src: &[u8]) -> Result<Bytes, TranslateError> {
    let src = parse_object(src)?;
    let mut out = Map::new();
    out.insert("model".to_owned(), Value::String(model.to_owned()));

    let (system, messages) = split_system(&src)?;
    if !system.is_empty() {
        out.insert("system".to_owned(), Value::Array(system));
    }
    out.insert("messages".to_owned(), Value::Array(messages));

    // Anthropic 必填。`max_completion_tokens` 是 OpenAI 的新名字，优先级更高。
    let max_tokens = present(&src, "max_completion_tokens")
        .or_else(|| present(&src, "max_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(DEFAULT_MAX_TOKENS);
    out.insert("max_tokens".to_owned(), json!(max_tokens));

    for (key, value) in &src {
        match key.as_str() {
            // 上面已经处理过
            "model" | "messages" | "max_tokens" | "max_completion_tokens" => {}
            // 同名同义，直译
            "temperature" | "top_p" | "stream" => {
                if !value.is_null() {
                    out.insert(key.clone(), value.clone());
                }
            }
            "stop" => {
                if let Some(seqs) = stop_sequences(value)? {
                    out.insert("stop_sequences".to_owned(), seqs);
                }
            }
            "tools" => {
                if !value.is_null() {
                    out.insert("tools".to_owned(), tools(value)?);
                }
            }
            "user" => {
                if let Some(id) = value.as_str() {
                    out.insert("metadata".to_owned(), json!({ "user_id": id }));
                }
            }
            // 循环后统一处理（两个字段合成一个 Anthropic 字段）
            "tool_choice" | "parallel_tool_calls" => {}

            // —— 恒等值可丢，非恒等值必须 400 ——
            "n" => reject_unless(
                value,
                |v| v.as_i64() == Some(1),
                || {
                    "`n > 1` 在 Anthropic Messages 里没有对应表达。\
                 静默当成 n=1 会让你以为拿到了多个候选里的第一个，实际只生成了 1 个"
                        .to_owned()
                },
            )?,
            "logprobs" => reject_unless(
                value,
                |v| v.as_bool() == Some(false),
                || {
                    "`logprobs` 在 Anthropic Messages 里没有对应表达（上游不返回 logprob）"
                        .to_owned()
                },
            )?,
            "top_logprobs" => reject_unless(
                value,
                |_| false,
                || "`top_logprobs` 在 Anthropic Messages 里没有对应表达".to_owned(),
            )?,
            "response_format" => reject_unless(
                value,
                |v| v.get("type").and_then(Value::as_str) == Some("text"),
                || {
                    "`response_format` 是一个输出格式保证，Anthropic Messages 无对应物。\
                     丢掉它等于保证消失，客户端会炸在自己的 json 解析上"
                        .to_owned()
                },
            )?,
            "seed" => reject_unless(
                value,
                |_| false,
                || "`seed` 的可复现性保证在 Anthropic Messages 里没有对应表达".to_owned(),
            )?,
            "frequency_penalty" | "presence_penalty" => {
                let name = key.clone();
                reject_unless(
                    value,
                    |v| v.as_f64() == Some(0.0),
                    move || {
                        format!(
                            "`{name}` 会改变采样分布，Anthropic Messages 无对应旋钮（0 是恒等值，可省略）"
                        )
                    },
                )?;
            }
            "logit_bias" => reject_unless(
                value,
                |v| v.as_object().is_none_or(Map::is_empty),
                || "`logit_bias` 会改变采样分布，Anthropic Messages 无对应旋钮".to_owned(),
            )?,
            "reasoning_effort" => reject_unless(
                value,
                |_| false,
                || {
                    "`reasoning_effort` 无法映射：Anthropic 的 extended thinking 要显式的 \
                 `budget_tokens`，且要求 `max_tokens > budget_tokens`。\
                 替你猜一个预算等于替你做决定"
                        .to_owned()
                },
            )?,
            "service_tier" => reject_unless(
                value,
                |v| v.as_str() == Some("auto"),
                || "`service_tier` 的非 auto 取值在 Anthropic Messages 里没有对应表达".to_owned(),
            )?,

            // —— 纯装饰，静默丢（模块文档已逐条列出，不留暗账）——
            // `stream_options` 只是在讨要 usage：走转义路径 usage 由转义器自己产。
            // `store` / `metadata` 是 OpenAI 侧的留存标签，不参与生成。
            "stream_options" | "store" | "metadata" => {}

            other => {
                return Err(TranslateError::Unsupported(format!(
                    "OpenAI 请求字段 `{other}` 在 Anthropic Messages 里没有对应表达。\
                     不认识就说不认识 —— 放行会被上游 400，丢掉会静默改语义"
                )));
            }
        }
    }

    if let Some(choice) = tool_choice(&src)? {
        out.insert("tool_choice".to_owned(), choice);
    }

    to_bytes(&Value::Object(out))
}

/// `value` 不满足 `ok` 就 400。`null` 恒等于「没设」，永远放行。
fn reject_unless(
    value: &Value,
    ok: impl FnOnce(&Value) -> bool,
    why: impl FnOnce() -> String,
) -> Result<(), TranslateError> {
    if value.is_null() || ok(value) {
        Ok(())
    } else {
        Err(TranslateError::Unsupported(why()))
    }
}

/// OpenAI 的 `stop` 是「字符串或字符串数组」，Anthropic 的 `stop_sequences` 只收数组。
fn stop_sequences(value: &Value) -> Result<Option<Value>, TranslateError> {
    match value {
        Value::Null => Ok(None),
        Value::String(_) => Ok(Some(Value::Array(vec![value.clone()]))),
        Value::Array(items) if items.iter().all(Value::is_string) => Ok(Some(value.clone())),
        _ => Err(TranslateError::Malformed(
            "`stop` 必须是字符串或字符串数组".to_owned(),
        )),
    }
}

/// OpenAI function 形状 → Anthropic `input_schema` 形状。
fn tools(value: &Value) -> Result<Value, TranslateError> {
    let items = value
        .as_array()
        .ok_or_else(|| TranslateError::Malformed("`tools` 必须是数组".to_owned()))?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let tool = item
            .as_object()
            .ok_or_else(|| TranslateError::Malformed("`tools[]` 必须是对象".to_owned()))?;
        match tool.get("type").and_then(Value::as_str) {
            Some("function") | None => {}
            Some(other) => {
                return Err(TranslateError::Unsupported(format!(
                    "OpenAI 工具类型 `{other}` 在 Anthropic Messages 里没有对应表达"
                )));
            }
        }
        let func = tool
            .get("function")
            .and_then(Value::as_object)
            .ok_or_else(|| TranslateError::Malformed("`tools[].function` 缺失".to_owned()))?;
        let name = func
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| TranslateError::Malformed("`tools[].function.name` 缺失".to_owned()))?;
        if present(func, "strict").and_then(Value::as_bool) == Some(true) {
            return Err(TranslateError::Unsupported(
                "`tools[].function.strict` 是一个 schema 遵从性保证，Anthropic 无对应物。\
                 去掉它后重试 —— 网关不会替你把保证降级成尽力而为"
                    .to_owned(),
            ));
        }
        let mut anth = Map::new();
        anth.insert("name".to_owned(), json!(name));
        if let Some(desc) = present(func, "description") {
            anth.insert("description".to_owned(), desc.clone());
        }
        // OpenAI 的 `parameters` 与 Anthropic 的 `input_schema` 都是 JSON Schema，
        // 逐字节等价，只是键名不同。
        anth.insert(
            "input_schema".to_owned(),
            present(func, "parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
        );
        out.push(Value::Object(anth));
    }
    Ok(Value::Array(out))
}

/// `tool_choice` + `parallel_tool_calls` 合成 Anthropic 的一个 `tool_choice` 对象。
fn tool_choice(src: &Map<String, Value>) -> Result<Option<Value>, TranslateError> {
    let serial = present(src, "parallel_tool_calls").and_then(Value::as_bool) == Some(false);
    let Some(choice) = present(src, "tool_choice") else {
        // 只写了 `parallel_tool_calls: false` 也要落地 —— 它是一条真实约束。
        return Ok(serial.then(|| json!({"type": "auto", "disable_parallel_tool_use": true})));
    };
    let mut mapped = match choice {
        Value::String(s) => match s.as_str() {
            "auto" => json!({"type": "auto"}),
            "none" => json!({"type": "none"}),
            "required" => json!({"type": "any"}),
            other => {
                return Err(TranslateError::Unsupported(format!(
                    "`tool_choice: {other}` 在 Anthropic Messages 里没有对应表达"
                )));
            }
        },
        Value::Object(o) if o.get("type").and_then(Value::as_str) == Some("function") => {
            let name = o
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    TranslateError::Malformed("`tool_choice.function.name` 缺失".to_owned())
                })?;
            json!({"type": "tool", "name": name})
        }
        _ => {
            return Err(TranslateError::Unsupported(
                "`tool_choice` 的这个形状在 Anthropic Messages 里没有对应表达".to_owned(),
            ));
        }
    };
    // Anthropic 只在 auto / any / tool 上接受这个开关，none 上带着会被 400。
    let serialisable = serial && mapped.get("type").and_then(Value::as_str) != Some("none");
    if let Some(o) = mapped.as_object_mut().filter(|_| serialisable) {
        o.insert("disable_parallel_tool_use".to_owned(), json!(true));
    }
    Ok(Some(mapped))
}

// ------------------------------------------------------------------ messages

/// 把 `messages[]` 拆成「顶层 `system` 块」+「Anthropic `messages[]`」。
///
/// Anthropic **没有 system role** —— 这是两个协议之间最硬的一处结构差异。
fn split_system(src: &Map<String, Value>) -> Result<(Vec<Value>, Vec<Value>), TranslateError> {
    let items = src
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| TranslateError::Malformed("`messages` 缺失或不是数组".to_owned()))?;

    let mut system = Vec::new();
    let mut msgs: Vec<Value> = Vec::new();
    let mut seen_dialogue = false;

    for (i, item) in items.iter().enumerate() {
        let msg = item
            .as_object()
            .ok_or_else(|| TranslateError::Malformed(format!("`messages[{i}]` 不是对象")))?;
        let role = msg
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(|| TranslateError::Malformed(format!("`messages[{i}].role` 缺失")))?;

        let (out_role, blocks) = match role {
            "system" | "developer" => {
                if seen_dialogue {
                    return Err(TranslateError::Unsupported(format!(
                        "`messages[{i}]` 的 {role} 消息出现在对话开始之后；                         Anthropic 只能把 system 放在顶层，搬过去会静默改变指令顺序"
                    )));
                }
                system.extend(text_blocks(msg.get("content"), i)?);
                continue;
            }
            "user" => {
                seen_dialogue = true;
                ("user", user_blocks(msg, i)?)
            }
            "assistant" => {
                seen_dialogue = true;
                ("assistant", assistant_blocks(msg, i)?)
            }
            "tool" => {
                seen_dialogue = true;
                ("user", vec![tool_result_block(msg, i)?])
            }
            "function" => {
                return Err(TranslateError::Unsupported(
                    "已废弃的 `role: \"function\"`，请改用 `role: \"tool\"` + `tool_call_id`"
                        .to_owned(),
                ));
            }
            other => {
                return Err(TranslateError::Unsupported(format!(
                    "`messages[{i}].role = {other}` 在 Anthropic Messages 里没有对应表达"
                )));
            }
        };

        if blocks.is_empty() {
            return Err(TranslateError::Malformed(format!(
                "`messages[{i}]` 翻译后没有任何内容块，Anthropic 会拒收空 content"
            )));
        }

        // 合并相邻同 role 的消息。一个 assistant 回合里的 N 个 tool_call 在 OpenAI
        // 侧是 N 条 `role: "tool"` 消息，在 Anthropic 侧必须挤进**一条** user 消息 ——
        // 不合并的话上游直接 400 "roles must alternate"。
        match msgs.last_mut() {
            Some(Value::Object(prev))
                if prev.get("role").and_then(Value::as_str) == Some(out_role) =>
            {
                if let Some(Value::Array(content)) = prev.get_mut("content") {
                    content.extend(blocks);
                }
            }
            _ => msgs.push(json!({"role": out_role, "content": blocks})),
        }
    }

    Ok((system, msgs))
}

/// 只收文本的 content（system 消息用）。
fn text_blocks(content: Option<&Value>, i: usize) -> Result<Vec<Value>, TranslateError> {
    match content {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(s)) if s.is_empty() => Ok(Vec::new()),
        Some(Value::String(s)) => Ok(vec![json!({"type": "text", "text": s})]),
        Some(Value::Array(parts)) => {
            let mut out = Vec::with_capacity(parts.len());
            for part in parts {
                let text = part
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|_| part.get("type").and_then(Value::as_str) == Some("text"))
                    .ok_or_else(|| {
                        TranslateError::Unsupported(format!(
                            "`messages[{i}].content[]` 只有 text part 能翻译成 Anthropic 的 \
                             system / tool_result 文本"
                        ))
                    })?;
                if !text.is_empty() {
                    out.push(json!({"type": "text", "text": text}));
                }
            }
            Ok(out)
        }
        Some(_) => Err(TranslateError::Malformed(format!(
            "`messages[{i}].content` 必须是字符串或数组"
        ))),
    }
}

fn user_blocks(msg: &Map<String, Value>, i: usize) -> Result<Vec<Value>, TranslateError> {
    match msg.get("content") {
        Some(Value::Array(parts)) => {
            let mut out = Vec::with_capacity(parts.len());
            for part in parts {
                match part.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        let text = part.get("text").and_then(Value::as_str).unwrap_or_default();
                        if !text.is_empty() {
                            out.push(json!({"type": "text", "text": text}));
                        }
                    }
                    Some("image_url") => {
                        let url = part
                            .get("image_url")
                            .and_then(|u| u.get("url"))
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                TranslateError::Malformed(format!(
                                    "`messages[{i}].content[].image_url.url` 缺失"
                                ))
                            })?;
                        out.push(image_block(url)?);
                    }
                    Some(other) => {
                        return Err(TranslateError::Unsupported(format!(
                            "OpenAI content part `{other}` 在 Anthropic Messages 里没有对应表达"
                        )));
                    }
                    None => {
                        return Err(TranslateError::Malformed(format!(
                            "`messages[{i}].content[].type` 缺失"
                        )));
                    }
                }
            }
            Ok(out)
        }
        other => text_blocks(other, i),
    }
}

/// `data:image/png;base64,XXX` → Anthropic 的 base64 source；`https://…` → url source。
///
/// 丢掉的只有 `image_url.detail`（Anthropic 不吃分辨率提示，图片照样送到）。
fn image_block(url: &str) -> Result<Value, TranslateError> {
    let Some(rest) = url.strip_prefix("data:") else {
        return Ok(json!({"type": "image", "source": {"type": "url", "url": url}}));
    };
    let (meta, data) = rest
        .split_once(',')
        .ok_or_else(|| TranslateError::Malformed("data URI 缺少分隔用的 `,`".to_owned()))?;
    let media_type = meta.strip_suffix(";base64").ok_or_else(|| {
        TranslateError::Unsupported(
            "Anthropic 的内联图片只接受 base64 data URI（`data:<media-type>;base64,…`）".to_owned(),
        )
    })?;
    Ok(json!({
        "type": "image",
        "source": {"type": "base64", "media_type": media_type, "data": data},
    }))
}

fn assistant_blocks(msg: &Map<String, Value>, i: usize) -> Result<Vec<Value>, TranslateError> {
    let mut out = text_blocks(msg.get("content"), i)?;
    // `refusal` 是模型确实产出的文本，只是 OpenAI 把它放在另一个字段里。
    if let Some(refusal) = msg.get("refusal").and_then(Value::as_str)
        && !refusal.is_empty()
    {
        out.push(json!({"type": "text", "text": refusal}));
    }
    for call in msg
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        match call.get("type").and_then(Value::as_str) {
            Some("function") | None => {}
            Some(other) => {
                return Err(TranslateError::Unsupported(format!(
                    "`tool_calls[].type = {other}` 在 Anthropic Messages 里没有对应表达"
                )));
            }
        }
        let id = call.get("id").and_then(Value::as_str).ok_or_else(|| {
            TranslateError::Malformed(format!("`messages[{i}].tool_calls[].id` 缺失"))
        })?;
        let func = call.get("function").ok_or_else(|| {
            TranslateError::Malformed(format!("`messages[{i}].tool_calls[].function` 缺失"))
        })?;
        let name = func.get("name").and_then(Value::as_str).ok_or_else(|| {
            TranslateError::Malformed(format!("`messages[{i}].tool_calls[].function.name` 缺失"))
        })?;
        let raw = func
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or_default();
        // OpenAI 把工具参数编码成**字符串里的 JSON**，Anthropic 直接就是对象。
        let input: Value = if raw.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(raw).map_err(|e| {
                TranslateError::Malformed(format!(
                    "`messages[{i}].tool_calls[].function.arguments` 不是合法 JSON：{e}"
                ))
            })?
        };
        if !input.is_object() {
            return Err(TranslateError::Malformed(format!(
                "`messages[{i}].tool_calls[].function.arguments` 必须编码 JSON object"
            )));
        }
        out.push(json!({"type": "tool_use", "id": id, "name": name, "input": input}));
    }
    Ok(out)
}

fn tool_result_block(msg: &Map<String, Value>, i: usize) -> Result<Value, TranslateError> {
    let id = msg
        .get("tool_call_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            TranslateError::Malformed(format!("`messages[{i}]` 是 role=tool 却没有 tool_call_id"))
        })?;
    let content = match msg.get("content") {
        Some(Value::String(s)) => Value::String(s.clone()),
        other => Value::Array(text_blocks(other, i)?),
    };
    Ok(json!({"type": "tool_result", "tool_use_id": id, "content": content}))
}

// ====================================================== 响应：Anthropic → OpenAI

fn response(body: &[u8]) -> Result<Bytes, TranslateError> {
    let src = parse_upstream_object(body)?;

    // 上游的错误体也要换方言 —— 客户端 SDK 只会解析它自己那套结构，
    // 回一个陌生结构会被渲染成一个无字的红叉。
    if let Some(err) = src.get("error").and_then(Value::as_object) {
        return to_bytes(&json!({
            "error": {
                "message": err.get("message").cloned().unwrap_or(Value::Null),
                "type": err.get("type").cloned().unwrap_or(Value::Null),
                "code": Value::Null,
                "param": Value::Null,
            }
        }));
    }

    let content = src
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| TranslateError::UpstreamShape("Anthropic 响应缺少 content[]".to_owned()))?;

    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => text.push_str(block.get("text").and_then(Value::as_str).unwrap_or("")),
            Some("tool_use") => tool_calls.push(json!({
                "id": block.get("id").cloned().unwrap_or(Value::Null),
                "type": "function",
                "function": {
                    "name": block.get("name").cloned().unwrap_or(Value::Null),
                    "arguments": serde_json::to_string(
                        block.get("input").unwrap_or(&json!({}))
                    ).unwrap_or_else(|_| "{}".to_owned()),
                },
            })),
            // thinking / redacted_thinking：请求方向已把 reasoning_effort 判成
            // Unsupported，上游不会开 thinking。万一开了也没有 OpenAI 侧落点。
            _ => {}
        }
    }

    let mut message = Map::new();
    message.insert("role".to_owned(), json!("assistant"));
    message.insert(
        "content".to_owned(),
        if text.is_empty() && !tool_calls.is_empty() {
            Value::Null
        } else {
            Value::String(text)
        },
    );
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_owned(), Value::Array(tool_calls));
    }

    let finish = src
        .get("stop_reason")
        .and_then(Value::as_str)
        .map_or(Value::Null, |r| json!(finish_reason(r)));

    let mut out = Map::new();
    out.insert(
        "id".to_owned(),
        src.get("id").cloned().unwrap_or(Value::Null),
    );
    out.insert("object".to_owned(), json!("chat.completion"));
    out.insert("created".to_owned(), json!(unix_now()));
    out.insert(
        "model".to_owned(),
        src.get("model").cloned().unwrap_or(Value::Null),
    );
    out.insert(
        "choices".to_owned(),
        json!([{"index": 0, "message": message, "finish_reason": finish, "logprobs": null}]),
    );
    if let Some(usage) = src.get("usage") {
        out.insert("usage".to_owned(), openai_usage(usage));
    }
    to_bytes(&Value::Object(out))
}

/// Anthropic `stop_reason` → OpenAI `finish_reason`。
fn finish_reason(stop: &str) -> &'static str {
    match stop {
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        "refusal" => "content_filter",
        // end_turn / stop_sequence / pause_turn 与未知取值都落 stop：
        // OpenAI 侧没有更贴切的桶，而编一个新值会让 SDK 的枚举解析炸掉。
        _ => "stop",
    }
}

fn openai_usage(usage: &Value) -> Value {
    let input = usage.get("input_tokens").and_then(Value::as_i64);
    let output = usage.get("output_tokens").and_then(Value::as_i64);
    let mut out = Map::new();
    if let Some(v) = input {
        out.insert("prompt_tokens".to_owned(), json!(v));
    }
    if let Some(v) = output {
        out.insert("completion_tokens".to_owned(), json!(v));
    }
    if let (Some(i), Some(o)) = (input, output) {
        out.insert("total_tokens".to_owned(), json!(i + o));
    }
    if let Some(v) = usage.get("cache_read_input_tokens").and_then(Value::as_i64) {
        out.insert(
            "prompt_tokens_details".to_owned(),
            json!({"cached_tokens": v}),
        );
    }
    Value::Object(out)
}

// ================================================= 流式：Anthropic SSE → OpenAI SSE

/// Anthropic 的六种事件塌缩成 OpenAI 的 `choices[].delta` 序列，收尾补 `data: [DONE]`。
///
/// 有状态的原因：Anthropic 的工具参数是**分片**到达的（`input_json_delta`），
/// 而 OpenAI 的 `tool_calls[].index` 是**工具调用序号**，不是 content block 序号 ——
/// 要记住当前打开的 block 对应第几个工具调用。
#[derive(Debug, Default)]
struct AnthropicSseToOpenAi {
    split: SseSplit,
    id: String,
    model: String,
    created: i64,
    /// `message_start` 已经发过（→ `delta.role` 只在首帧出现）。
    started: bool,
    /// 当前打开的 content block 对应的 OpenAI 工具调用序号；文本块是 `None`。
    tool_slot: Option<i64>,
    /// 已经见过几个 tool_use block。
    tool_seen: i64,
    usage: RelayUsage,
    /// `data: [DONE]` 已经发过（**必须最后且只有一次**）。
    done: bool,
}

impl StreamTranslator for AnthropicSseToOpenAi {
    fn push(&mut self, upstream_frame: &[u8]) -> Result<Vec<Bytes>, TranslateError> {
        let mut out = Vec::new();
        for event in self.split.push(upstream_frame)? {
            self.handle(&event, &mut out)?;
        }
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<Bytes>, TranslateError> {
        let mut out = Vec::new();
        if let Some(rest) = self.split.flush() {
            self.handle(&rest, &mut out)?;
        }
        // 一个字都没收到就别凭空造一个空流：让上层看见「上游什么都没给」。
        if self.started && !self.done {
            out.push(Bytes::from_static(OPENAI_DONE));
            self.done = true;
        }
        Ok(out)
    }

    fn usage(&self) -> Option<RelayUsage> {
        (!self.usage.is_empty()).then(|| self.usage.clone())
    }
}

impl AnthropicSseToOpenAi {
    fn handle(&mut self, event: &[u8], out: &mut Vec<Bytes>) -> Result<(), TranslateError> {
        let Some(data) = sse_data(event) else {
            return Ok(()); // 注释帧 / 只有 event: 的帧
        };
        let Ok(Value::Object(ev)) = serde_json::from_slice::<Value>(&data) else {
            return Ok(()); // 心跳与任何非 JSON 载荷：跳过，不打断在途的流
        };

        match ev.get("type").and_then(Value::as_str).unwrap_or_default() {
            "message_start" => {
                let msg = ev.get("message");
                self.id = str_at(msg, "id").to_owned();
                self.model = str_at(msg, "model").to_owned();
                self.created = unix_now();
                if let Some(u) = msg.and_then(|m| m.get("usage")) {
                    self.absorb_usage(u);
                }
                self.started = true;
                out.push(openai_frame(&self.chunk(
                    json!({"role": "assistant", "content": ""}),
                    Value::Null,
                    None,
                )));
            }
            "content_block_start" => {
                let block = ev.get("content_block");
                match block.and_then(|b| b.get("type")).and_then(Value::as_str) {
                    Some("tool_use") => {
                        let slot = self.tool_seen;
                        self.tool_seen += 1;
                        self.tool_slot = Some(slot);
                        out.push(openai_frame(&self.chunk(
                            json!({"tool_calls": [{
                                "index": slot,
                                "id": str_at(block, "id"),
                                "type": "function",
                                "function": {"name": str_at(block, "name"), "arguments": ""},
                            }]}),
                            Value::Null,
                            None,
                        )));
                    }
                    Some("text") => {
                        self.tool_slot = None;
                        let text = str_at(block, "text");
                        if !text.is_empty() {
                            let delta = json!({ "content": text });
                            out.push(openai_frame(&self.chunk(delta, Value::Null, None)));
                        }
                    }
                    // thinking / redacted_thinking：没有 OpenAI 侧落点
                    _ => self.tool_slot = None,
                }
            }
            "content_block_delta" => {
                let delta = ev.get("delta");
                match delta.and_then(|d| d.get("type")).and_then(Value::as_str) {
                    Some("text_delta") => {
                        let d = json!({ "content": str_at(delta, "text") });
                        out.push(openai_frame(&self.chunk(d, Value::Null, None)));
                    }
                    Some("input_json_delta") => {
                        // 分片的工具参数：OpenAI 侧原样做成 arguments 增量，
                        // 不做任何拼接或校验 —— 拼接是客户端的事，网关插手只会丢字节。
                        let slot = self.tool_slot.ok_or_else(|| {
                            TranslateError::UpstreamShape(
                                "input_json_delta 出现在一个非 tool_use 的 content block 上"
                                    .to_owned(),
                            )
                        })?;
                        let d = json!({"tool_calls": [{
                            "index": slot,
                            "function": {"arguments": str_at(delta, "partial_json")},
                        }]});
                        out.push(openai_frame(&self.chunk(d, Value::Null, None)));
                    }
                    // thinking_delta / signature_delta：请求方向已拒 reasoning_effort
                    _ => {}
                }
            }
            "content_block_stop" => self.tool_slot = None,
            "message_delta" => {
                if let Some(u) = ev.get("usage") {
                    self.absorb_usage(u);
                }
                let finish = ev
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                    .map_or(Value::Null, |r| json!(finish_reason(r)));
                // usage 挂在**这一帧**上，而不是另起一帧 `choices: []`：
                // 后者正是缺陷 #4 记在案的客户端崩法（手写 chunk["choices"][0] 抛 IndexError）。
                let usage = self.usage_value();
                out.push(openai_frame(&self.chunk(json!({}), finish, usage)));
            }
            "message_stop" => {
                if !self.done {
                    out.push(Bytes::from_static(OPENAI_DONE));
                    self.done = true;
                }
            }
            "error" => {
                // SSE 里的中途失败必须让客户端**察觉**。上抛让引擎把它变成
                // RelayError::Translate → RST_STREAM / 掐连接（缺陷 #6）。
                // 补一帧 data: [DONE] 才是最坏的做法：那是一次干净的 EOF。
                return Err(TranslateError::UpstreamShape(format!(
                    "上游流中途报错：{}",
                    ev.get("error").unwrap_or(&Value::Null)
                )));
            }
            _ => {}
        }
        Ok(())
    }

    fn chunk(&self, delta: Value, finish: Value, usage: Option<Value>) -> Value {
        let mut out = Map::new();
        out.insert("id".to_owned(), json!(self.id));
        out.insert("object".to_owned(), json!("chat.completion.chunk"));
        out.insert("created".to_owned(), json!(self.created));
        out.insert("model".to_owned(), json!(self.model));
        out.insert(
            "choices".to_owned(),
            json!([{"index": 0, "delta": delta, "finish_reason": finish}]),
        );
        if let Some(u) = usage {
            out.insert("usage".to_owned(), u);
        }
        Value::Object(out)
    }

    fn usage_value(&self) -> Option<Value> {
        if self.usage.is_empty() {
            return None;
        }
        let mut out = Map::new();
        if let Some(v) = self.usage.input_tokens {
            out.insert("prompt_tokens".to_owned(), json!(v));
        }
        if let Some(v) = self.usage.output_tokens {
            out.insert("completion_tokens".to_owned(), json!(v));
        }
        if let (Some(i), Some(o)) = (self.usage.input_tokens, self.usage.output_tokens) {
            out.insert("total_tokens".to_owned(), json!(i + o));
        }
        if let Some(v) = self.usage.cached_tokens {
            out.insert(
                "prompt_tokens_details".to_owned(),
                json!({"cached_tokens": v}),
            );
        }
        Some(Value::Object(out))
    }

    /// **绝不能丢 usage**：Anthropic 把总账拆在两帧里 —— `input_tokens` 在首帧
    /// `message_start`，`output_tokens` 在末帧 `message_delta`。只补不清，
    /// 后一帧没带的字段保留前一帧的值。
    fn absorb_usage(&mut self, usage: &Value) {
        if let Some(v) = usage.get("input_tokens").and_then(Value::as_i64) {
            self.usage.input_tokens = Some(v);
        }
        if let Some(v) = usage.get("output_tokens").and_then(Value::as_i64) {
            self.usage.output_tokens = Some(v);
        }
        if let Some(v) = usage.get("cache_read_input_tokens").and_then(Value::as_i64) {
            self.usage.cached_tokens = Some(v);
        }
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or_default()
}
