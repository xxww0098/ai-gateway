//! P2 三格：`openai-completions × claude`、`anthropic-messages × {openai, codex}`。
//!
//! OWNER: worker `relay-anthropic`（本文件 + `anthropic/**`）。
//!
//! 真实需求：Claude Code 指向 OpenAI 上游、Cursor 指向 Claude 上游。

#[cfg(test)]
mod tests;
