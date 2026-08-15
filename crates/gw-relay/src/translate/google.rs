//! P1 四格：`{openai-completions, anthropic-messages} × {gemini, vertex}`。
//!
//! OWNER: worker `relay-google`（本文件 + `google/**`）。
//!
//! gemini 与 vertex 的 wire 协议是**同一个** GenerateContent（只是 endpoint
//! 前缀与鉴权不同），所以这里是 **2 个转义器覆盖 4 格**，不是 4 个：
//! [`OpenAiToGoogle`] 与 [`AnthropicToGoogle`]。端点拼接与鉴权归
//! [`crate::engine`]，本模块的输入是字节、输出是字节，不碰 HTTP。
//!
//! # 根除的缺陷
//!
//! `docs/relay-passthrough-audit.md` 缺陷 **#1**（S1）在 gemini/vertex 上的
//! 四格：今天这四格是**直通**的（`docs/relay-surface-plan.md` §3.6 的
//! A×gemini / A×vertex / C×gemini / C×vertex，「今天」列全是「直通 → 上游 400」），
//! 客户端 100% 拿不到响应。协议翻译从此是一次**显式的**
//! [`crate::Translator`] 调用，不是藏在 provider 内部的隐式改写。
//!
//! 附带守住的三条：
//!
//! - 缺陷 **#4**：不做「整体 JSON round-trip 之后再塞一个字段」。这里是按字段
//!   重建，客户端没写的东西不会凭空出现；流式方向也**不会**合成一帧客户端
//!   没要过的 usage chunk。
//! - 缺陷 **#16**：走转义路径时 [`crate::StreamTranslator::usage`] **取代**
//!   [`crate::UsageProbe`]。上游帧已经在这里解析过一次，再挂一个探针把每个
//!   chunk 全量 memcpy 一遍是纯浪费。
//! - 缺陷 **#20**：Google 侧的 `alt=sse` 框架由本模块消费。转义路径上客户端
//!   看到的是目标方言的框架，Google 的框架不再泄漏出去。
//!
//! # usage 的口径（**读这一段，别猜**）
//!
//! [`crate::RelayUsage`] 拿到的是 **Google 的原始计数**，不做任何换算：
//!
//! | Google | `RelayUsage` |
//! | --- | --- |
//! | `usageMetadata.promptTokenCount` | `input_tokens` |
//! | `usageMetadata.candidatesTokenCount` | `output_tokens` |
//! | `usageMetadata.cachedContentTokenCount` | `cached_tokens` |
//! | `usageMetadata.thoughtsTokenCount` | `reasoning_tokens` |
//!
//! 两条**下游计费必须知道**的口径差异，都源于 Google 与 OpenAI 的约定不同：
//!
//! 1. Google 的 `candidatesTokenCount` **不包含** `thoughtsTokenCount`
//!    （OpenAI 的 `completion_tokens` 是**包含** `reasoning_tokens` 的）。
//!    所以对思考型模型，计费的输出量是 `output_tokens + reasoning_tokens`，
//!    只按 `output_tokens` 算会**少收**。
//! 2. Google 的 `promptTokenCount` **包含** `cachedContentTokenCount`
//!    （与 OpenAI 一致，与 Anthropic 相反）。
//!
//! 客户端看到的响应信封里做了这两个换算（各自方言的语义），
//! **但 [`crate::RelayUsage`] 里没有** —— 那里永远是上游原话。
//! 四个计数全是 [`Option`]：「上游没给」与「上游说 0」必须能分开，
//! 前者要落 fallback 结算，后者不能。
//!
//! # 允许静默丢弃的字段（**只有这张表上的**）
//!
//! 其余任何未知字段一律 [`crate::TranslateError::Unsupported`] → 400。
//! 静默丢一个有语义的字段，是审计报告里反复强调的那类「静默的正确性错误」，
//! 比一个 400 坏得多。
//!
//! | 字段 | 出现在 | 为什么丢了没有语义损失 |
//! | --- | --- | --- |
//! | `stream` | 两个入口的顶层 | Google 的流式由 endpoint 决定（`:streamGenerateContent`），body 里没有这个开关；原样带过去 Google 会以未知字段拒收。谁流式由 `engine` 判定 |
//! | `stream_options` | openai 顶层 | 只控制 OpenAI 要不要在末帧回 usage。Google 每次都回 `usageMetadata`，计费不依赖它 |
//! | `user` / `safety_identifier` / `prompt_cache_key` / `metadata` | 两个入口的顶层 | 纯遥测标签，上游用来做滥用归因；不参与生成，丢了输出一字不差 |
//! | `store` / `service_tier` | openai 顶层 | OpenAI 侧的留存与调度档位，Google 没有对应概念，也不改变本次输出 |
//! | `messages[].name` | openai 消息 | OpenAI 的发言人标签。Google 的 `contents[]` 根本没有这个位置 |
//! | `messages[].refusal` / `annotations` | openai 消息 | 上一轮**响应**的回显，模型不读它们 |
//! | `thinking` / `redacted_thinking` block | anthropic 消息 | 上一轮 assistant 思考的回放。Google 的 `thoughtSignature` 只在同一次 Google 会话内有效，跨上游转过去只会误导 |
//! | `cache_control` | anthropic 各处 | 提示缓存的**成本**优化提示，不改变模型看到的内容（Google 的缓存要显式建 `cachedContent` 资源，网关不能替客户端建） |
//! | `tools[].function.strict` | openai 工具 | OpenAI 侧的 schema 强制执行开关。Google 不做强制，但工具**契约本身**（name/description/parameters）一字不改地过去了 |
//!
//! 几个**明确拒绝**的、容易被误以为「顺手翻一下就行」的字段，理由一并记在这里：
//!
//! | 字段 | 为什么是 400 而不是翻译 |
//! | --- | --- |
//! | `n > 1` | Google 的 `candidateCount` 能给多候选，但流式方向要按 `candidates[].index` 分发进多个 `choices[]`。只在非流式支持、流式静默只回第一个，是两条路径行为不一致的静默错误 |
//! | `response_format: json_schema` | Google 的 `responseSchema` 只认 OpenAPI 3.0 子集，OpenAI 的 strict schema 必带 `additionalProperties: false`。摘掉 schema 只留 `application/json`，客户端会拿到一个不合它 schema 的 JSON 却按 schema 去解 |
//! | `parallel_tool_calls: false` / `disable_parallel_tool_use: true` | Google 没有这个旋钮。**默认值放行、非默认值拒绝** —— SDK 无脑带上的默认值不该打客户端一个 400，真要关掉的请求也不该被无视 |
//! | 远程图片 URL | Google 只收内联 base64。替客户端去下载再转码是网关不该有的副作用，失败起来还莫名其妙 |
//! | 认不出的 `tool_call_id` / `tool_use_id` | Google 按**名字**匹配 functionResponse，没有 id 这个概念。名字找不回来时随便编一个，模型会收到一个它没调用过的工具的结果 |
//! | Anthropic 的服务端工具（`web_search_*` 等） | 由 Anthropic 自己执行，Google 侧没有任何对应物 |
//!
//! 注意 `cache_control` 是靠 [`crate::TranslateError::Unsupported`] 的**反面**
//! 实现的：block 级的未知键不检查（Anthropic 到处都能挂 `cache_control`，
//! 逐处列白名单只会在下一次 API 演进时误伤），顶层与消息级则严格检查。
//!
//! # 流式的帧序列合法性
//!
//! 两个方向都保证产出的序列在**目标方言里合法**，这比逐字节比对一个抄来的
//! 期望字符串有意义得多（规范 2.11）：
//!
//! - OpenAI 方向：`data: [DONE]` 最后一帧且只有一次；`delta.role` 只在首帧。
//! - Anthropic 方向：`message_start` 先于任何 `content_block_delta` 且只有一次；
//!   `message_stop` 最后一帧且只有一次；每个 delta 落在一对 start/stop 之间；
//!   `index` 从 0 起严格递增。

mod anthropic;
mod openai;
mod wire;

pub use anthropic::AnthropicToGoogle;
pub use openai::OpenAiToGoogle;

#[cfg(test)]
mod tests;
