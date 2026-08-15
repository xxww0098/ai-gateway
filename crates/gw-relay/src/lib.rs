//! `gw-relay` —— 三面入口的**纯字节中继内核**。
//!
//! 本 crate 的第一性原理写在 [`contract`] 的开头，一句话版本：
//!
//! > **中继层不认识 JSON。** 它只认识 [`bytes::Bytes`]、[`http::HeaderMap`]、
//! > [`http::StatusCode`] 和帧序列。任何需要看懂内容的东西（计费、协议转义）
//! > 都挂在**旁路**，从中继拿只读视图，永远不往回写路径塞东西。
//!
//! 它**不依赖本工作区的任何其他 crate**。这是有意的：内核只做转发，
//! 鉴权、计费、账本、面板全部由 `gw-proxy` 在外面挂上去。一旦这里出现
//! `gw-ledger` 或 `gw-model`，"内核"这个词就不成立了。
//!
//! # 三个入口（收敛结论，见 `docs/relay-surface-plan.md`）
//!
//! | 入口 | 路径 |
//! | --- | --- |
//! | [`contract::Surface::OpenAiCompletions`] | `POST /v1/chat/completions` |
//! | [`contract::Surface::OpenAiResponses`]   | `POST /v1/responses` |
//! | [`contract::Surface::AnthropicMessages`] | `POST /v1/messages` |
//!
//! # 文件所有权（wave 2，不可越界）
//!
//! | 属主 | 独占文件 |
//! | --- | --- |
//! | **协调者** | `Cargo.toml`、`lib.rs`、`contract.rs`、`translate.rs` |
//! | `relay-core` | `headers.rs`、`body.rs`、`engine.rs`、`probe.rs` 及其同名目录 |
//! | `relay-endpoints` | `endpoint.rs` 及 `endpoint/**` |
//! | `relay-google` | `translate/google.rs` 及 `translate/google/**` |
//! | `relay-anthropic` | `translate/anthropic.rs` 及 `translate/anthropic/**` |
//!
//! 要改协调者独占的文件 → `orca orchestration ask`，别自己动手。
//!
//! # 棘轮（规范 5.3）
//!
//! 骨架期本 crate 允许 `todo!()`。**谁清空了本 crate 的最后一个 `todo!()`，
//! 就在同一个 commit 里往这里加 `#![deny(clippy::todo, clippy::unimplemented)]`**。
//! 加之前自查：`grep -rn 'todo!\|unimplemented!' src/ | wc -l` 为 0，
//! 且 `cargo clippy -p gw-relay --all-targets -- -D warnings` 通过。

pub mod contract;

mod body;
mod headers;

pub mod endpoint;
pub mod engine;
pub mod probe;
pub mod translate;

pub use contract::{
    Credential, Relay, RelayBody, RelayError, RelayRequest, RelayResponse, RelayResponseBody,
    RelayTimeouts, RelayTransportError, RelayUsage, StreamTranslator, Surface, TranslateError,
    Translator, UpstreamDialect, UpstreamTarget, UsageProbe,
};
pub use headers::copy_preserving_multivalue;
