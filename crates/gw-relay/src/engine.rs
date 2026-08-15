//! [`crate::Relay`] 的实现 —— 纯字节中继引擎。
//!
//! OWNER: worker `relay-core`。
//!
//! 它不认识 JSON、不认识方言、不认识计费。它只做四件事：
//! 拼上游 URL（origin + 入站 path + 原始 query 字节）、换凭证、
//! 把 body 交出去、把响应原样接回来。
//!
//! **上游返回的任何 status 都是 `Ok`。**`Err` 只留给"没能拿到上游响应"。

#[cfg(test)]
mod tests;
