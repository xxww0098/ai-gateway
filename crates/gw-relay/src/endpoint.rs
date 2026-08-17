//! 三面入口的规范定义、上游选择、以及 15 格矩阵的派发。
//!
//! OWNER: worker `relay-endpoints`（本文件 + `endpoint/**` 全归它，
//! 可以自由增删子模块，只要都能从这里的 `mod` 声明到达 —— 规范 2.6）。
//!
//! 三件事：
//! 1. **入口规范**：路径、方法、content-type、流式判定（三入口统一规则）。见 [`spec`]。
//! 2. **上游选择**：四级链取代今天的字符串前缀猜测
//!    （`routes.rs:73-97` 的 `contains("codex")` / `starts_with("claude-")`，
//!    新模型名一出现就静默走错上游）。见 [`upstream`]。
//! 3. **矩阵派发**：直通 / 转义 / 400 三选一，400 必须用**入口方言**的错误信封。见 [`matrix`]。
//!
//! 附带：[`include_usage`] 根除缺陷 #4（`ensure_include_usage()` 的整体 JSON round-trip）。
//!
//! # 这四个模块之间的调用顺序
//!
//! ```text
//! spec::validate(method, path, headers)        -> Surface        （不匹配 → 404/405/400）
//! spec::RequestSpec::parse(surface, body_peek) -> RequestSpec     （**全链路唯一一次** JSON 解析，缺陷 #15）
//! upstream::select(surface, model, resolver)   -> Selection       （四级链，L4 命中打点告警）
//! matrix::route(surface, provider, model)      -> Route           （15 格显式表）
//!   ├─ Passthrough → crate::engine 恒等转发
//!   ├─ Translate   → crate::translate::{google, anthropic}
//!   └─ Reject      → HTTP 400 + 入口方言错误信封（body 已在 Route 里）
//! include_usage::splice(...)                   -> Option<[Bytes;2]>（仅 OpenAI Chat 上游 + 流式）
//! ```
//!
//! # 已知缺口（已接受，不要试图去修）
//!
//! 面板 `frontend/src/features/user-dashboard/components/QuickIntegrationPanel.tsx:80`
//! 的 Anthropic tab 仍然把 `${origin}/v1beta` 印给用户。`/v1beta` 已被硬删，
//! 本 crate 里不存在这个前缀（[`spec::validate`] 对它返回
//! [`spec::SurfaceError::UnknownPath`]），所以照着面板配的用户会拿到 404。
//!
//! 这行前端文案**今天就是错的** —— `/v1beta` 是 Google 的版本段，给 Anthropic
//! 客户端本来就 404（`@anthropic-ai/sdk` 自己会拼 `/v1/messages`，它需要的 base
//! 是裸 `${origin}`）。收敛只是把「错但碰巧有个路由在」变成「错且路由也没了」。
//! 前端冻结，改不了。这是已知且已接受的代价。

pub mod include_usage;
pub mod json_peek;
pub mod matrix;
pub mod spec;
pub mod upstream;

pub use include_usage::{IncludeUsagePolicy, splice_include_usage};
pub use matrix::{
    Cell, Provider, RejectReason, Route, cell, reject_body, route, translator_for, upstream_dialect,
};
pub use json_peek::top_level_field;
pub use spec::{RequestSpec, SurfaceError, accept_conflicts_with_body, validate};
pub use upstream::{ChannelResolver, InMemoryChannelResolver, Selection, SelectionLevel, select};

#[cfg(test)]
mod tests;
