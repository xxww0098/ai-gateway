//! P1 四格：`{openai-completions, anthropic-messages} × {gemini, vertex}`。
//!
//! OWNER: worker `relay-google`（本文件 + `google/**`）。
//!
//! gemini 与 vertex 的 wire 协议是**同一个** GenerateContent（只是 endpoint
//! 前缀与鉴权不同），所以这里是 **2 个转义器覆盖 4 格**，不是 4 个。

#[cfg(test)]
mod tests;
