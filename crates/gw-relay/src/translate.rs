//! 协议转义层 —— 15 格矩阵里需要翻译的 7 格。
//!
//! **COORDINATOR-OWNED**（两个 worker 都要往这里加 `mod`，归任一方都会造成
//! 跨属主编辑）。子模块的内容各归各的属主，见 [`crate`] 的所有权表。
//!
//! 分层依据是**上游 wire 协议**而不是账号 provider 名：`gemini` 与 `vertex`
//! 是两套鉴权与端点前缀，wire 协议却是同一个 GenerateContent，所以
//! [`google`] 一个模块覆盖 P1 四格。
//!
//! | 模块 | 覆盖格子 | 属主 |
//! | --- | --- | --- |
//! | [`google`] | P1 四格：`{openai-completions, anthropic-messages} × {gemini, vertex}` | `relay-google` |
//! | [`anthropic`] | P2 三格：`openai-completions × claude`、`anthropic-messages × {openai, codex}` | `relay-anthropic` |
//!
//! 矩阵里剩下的 8 格不归这里：直通 5 格由 [`crate::engine`] 恒等转发，
//! 直接 400 的 3 格（`openai-responses × {claude, gemini, vertex}`）由
//! [`crate::endpoint`] 在派发前拒掉 —— Responses API 的有状态 item 模型
//! （`previous_response_id` / `store` / 加密 reasoning item）在对端没有对应概念，
//! 翻过去只能翻文本部分，而客户端会以为续接生效了。**一个静默的跨轮次正确性
//! 错误，比一个 400 坏得多。**

pub mod anthropic;
pub mod google;
