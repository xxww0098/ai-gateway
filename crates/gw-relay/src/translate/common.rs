//! 两个转义器家族共用的 SSE/JSON wire 原语。
//!
//! **COORDINATOR-OWNED**（`relay-google` 与 `relay-anthropic` 都要调用这里，
//! 归任一方都会造成跨属主编辑）。只放**与两侧方言无关**的原语：顶层对象解析、
//! SSE `data:` 载荷提取、两种下游方言的帧构造、unix 秒。
//! 「Google 长什么样」「OpenAI/Anthropic 长什么样」的知识都不在这个文件里。

use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use serde_json::{Map, Value};

use crate::contract::TranslateError;

// ============================================================ 顶层 JSON 入口

/// 把请求体解析成顶层 JSON 对象。
///
/// 非对象一律 [`TranslateError::Malformed`] —— 各入口的 body 在协议上都必须是
/// 一个 JSON object，数组或裸标量不是「我们不支持」，是客户端发错了。
pub(super) fn parse_object(body: &[u8]) -> Result<Map<String, Value>, TranslateError> {
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
pub(super) fn parse_upstream_object(body: &[u8]) -> Result<Map<String, Value>, TranslateError> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(other) => Err(TranslateError::UpstreamShape(format!(
            "顶层必须是 JSON object，收到 {}",
            kind_of(&other)
        ))),
        Err(e) => Err(TranslateError::UpstreamShape(e.to_string())),
    }
}

pub(super) fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ============================================================ SSE 载荷提取

/// 取出一个 SSE 事件里的 `data:` 载荷（多行 `data:` 按 SSE 规范用 `\n` 拼接）。
///
/// 没有 `data:` 字段返回 `None` —— 注释帧（`: keep-alive`）与只有 `event:` 的帧
/// 都走这条路，调用方据此跳过。
pub(super) fn sse_data(event: &[u8]) -> Option<Vec<u8>> {
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

// ============================================================ SSE 帧构造

/// Anthropic 的 SSE 帧：`event:` 与 `data:` **都要**。
///
/// Anthropic 官方 SDK 按 `event:` 行分派事件类型，只发 `data:` 会让它把每一帧
/// 都当成未知事件丢掉 —— 表现为「流跑完了但一个字都没显示」。
pub(super) fn anthropic_frame(event: &str, data: &Value) -> Bytes {
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
pub(super) fn openai_frame(data: &Value) -> Bytes {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(b"data: ");
    serde_json::to_writer(&mut out, data).unwrap_or_default();
    out.extend_from_slice(b"\n\n");
    Bytes::from(out)
}

// ============================================================ 杂项

/// unix 秒。OpenAI 的 `created` 字段要它。
pub(super) fn unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
