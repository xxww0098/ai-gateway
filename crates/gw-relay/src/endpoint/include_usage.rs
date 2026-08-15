//! `stream_options` 的**定点字节插入** —— 根除缺陷 #4。
//!
//! OWNER: worker `relay-endpoints`。
//!
//! # 今天错在哪
//!
//! `ensure_include_usage()`（`common.rs:248-268`）对 OpenAI/Codex 的**每一个流式请求**
//! 做整体 JSON round-trip：`from_slice` → `Value` → `to_vec`。实测三个后果：
//!
//! | 后果 | 成因 |
//! | --- | --- |
//! | 递归键序被重排 | `serde_json` 未开 `preserve_order`，`Map` 就是 `BTreeMap` |
//! | 客户端显式写的 `include_usage: false` 被无条件翻成 `true` | `common.rs:262` 是无条件 `insert` |
//! | `"seed": 12345678901234567890` 变成 `1.2345678901234568e+22` | 大整数落进 `f64` |
//!
//! 第二条最具体：翻成 `true` 之后 SSE 末尾多出一帧 `data: {"choices":[],"usage":{…}}`，
//! 任何手写 `chunk["choices"][0]` 的客户端在这一帧上抛 `IndexError` /
//! `undefined is not an object`。
//!
//! # 这里怎么做
//!
//! 顶层看一眼有没有 `"stream_options"`（这一眼搭在
//! [`super::spec::RequestSpec::parse`] 那**唯一一次**解析上，不再单独解一遍），
//! 没有就在最外层 `{` 之后插入那一小段字节，其余字节 **100% 原样**：
//! 返回的是 `[prefix, original.slice(open+1..)]` 两段，第二段是
//! [`Bytes`] 的零拷贝切片，与原 body 共享同一块 allocation。
//! **不反序列化、不重序列化、不重排键、不动数字格式。**

use bytes::{BufMut as _, Bytes, BytesMut};

use crate::contract::UpstreamDialect;
use crate::endpoint::spec::RequestSpec;

/// 插入的那一小段。放在最外层 `{` 之后，所以自带尾随逗号。
const INSERT: &[u8] = br#""stream_options":{"include_usage":true},"#;

/// 空对象场景下的插入段：`{}` 里没有后续键，尾随逗号会让 JSON 非法。
const INSERT_INTO_EMPTY: &[u8] = br#""stream_options":{"include_usage":true}"#;

/// 要不要为了计费去碰客户端的请求体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IncludeUsagePolicy {
    /// 缺省：缺 `stream_options` 就定点插入。没有它，OpenAI 流式响应不带终局
    /// `usage` 事件，结算必落 fallback（`AGENTS.md` §计费流程）。
    #[default]
    Force,
    /// **尊重客户端**：一个字节都不碰，接受 usage 缺失并落 fallback 计费。
    ///
    /// 这是「透传优先于计费」的字面执行。部署方如果宁可少收准一点也不愿意让
    /// 网关碰请求体，就选它 —— 代价是明确的、可预期的，而不是静默失真。
    RespectClient,
}

/// 定点插入的两段字节：`[prefix, rest]`。拼成帧序列发出去，**一次全量拷贝都没有**。
///
/// - `prefix`：原 body 从头到最外层 `{`（含）的那一段，加上插入的键值对。
///   这是唯一一次分配，长度是「前导空白 + 40 字节左右」。
/// - `rest`：原 [`Bytes`] 从 `{` 之后到结尾的**零拷贝切片**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spliced {
    pub prefix: Bytes,
    pub rest: Bytes,
}

impl Spliced {
    /// 两段的总长度。给 `content-length` 用（body 长度变了，必须重算 ——
    /// 这正是 `content-length` 在请求方向被丢弃重算的原因，
    /// 见 `docs/relay-passthrough-audit.md` §4.1 条 6）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.prefix.len() + self.rest.len()
    }

    /// 永远为假（`prefix` 至少含一个 `{`）。为满足 clippy 的 `len_without_is_empty` 而存在。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 该不该插、以及插成什么样。返回 `None` = **一个字节都不动**。
///
/// 四个条件全部满足才插：
///
/// 1. 策略是 [`IncludeUsagePolicy::Force`]；
/// 2. 这是一个流式请求（非流式响应本来就带 `usage`）；
/// 3. 上游是 **OpenAI Chat Completions**。
///    **不包括 [`UpstreamDialect::OpenAiResponses`]** —— Responses API 不认识
///    `stream_options`，塞进去上游直接 400。今天正是这么坏的：缺陷 #1（打错端点）
///    叠加缺陷 #4（还塞 `stream_options`），入口 B 双重不可用；
/// 4. 客户端顶层**没有**写 `stream_options`。写了就一个字节都不动 ——
///    缺陷 #4 里「`include_usage: false` 被翻成 `true`」的问题在这里自然消失，
///    因为「客户端写了」这件事本身就是终止条件，不必去看它写的是 `true` 还是 `false`。
///
/// 另外 body 必须看得见（[`RequestSpec::body_visible`]）且确实是个 JSON 对象。
/// 看不见就不插 —— 计费降级，转发不降级。
#[must_use]
pub fn splice_include_usage(
    body: &Bytes,
    spec: &RequestSpec,
    upstream: UpstreamDialect,
    policy: IncludeUsagePolicy,
) -> Option<Spliced> {
    if policy != IncludeUsagePolicy::Force
        || !spec.stream
        || upstream != UpstreamDialect::OpenAiChat
        || spec.stream_options_present
        || !spec.body_visible
    {
        return None;
    }

    let open = body.iter().position(|b| !b.is_ascii_whitespace())?;
    if body[open] != b'{' {
        return None; // 顶层不是对象，插不进去，也不该猜
    }
    let empty_object = body[open + 1..]
        .iter()
        .find(|b| !b.is_ascii_whitespace())
        .is_none_or(|b| *b == b'}');

    let insert = if empty_object {
        INSERT_INTO_EMPTY
    } else {
        INSERT
    };
    let mut prefix = BytesMut::with_capacity(open + 1 + insert.len());
    prefix.put_slice(&body[..=open]);
    prefix.put_slice(insert);

    Some(Spliced {
        prefix: prefix.freeze(),
        rest: body.slice(open + 1..),
    })
}

#[cfg(test)]
mod tests;
