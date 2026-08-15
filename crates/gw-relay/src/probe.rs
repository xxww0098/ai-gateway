//! [`crate::UsageProbe`] 的实现 —— 旁路 usage 取数。
//!
//! OWNER: worker `relay-core`。
//!
//! 缺陷 #16 的落点：现在 `StreamUsageBuffer` 对每个 chunk 全量 memcpy
//! （tail 每字节必拷 + head 前 32 KiB 再拷一份 + 每 64 KiB 一次 memmove，
//! 约 2× 全流量的额外内存带宽）。这里改成只持 [`bytes::Bytes`] 句柄的环，
//! 或者更省的增量行解析（SSE 是行式的，跨帧只需缓一个半行）。
//!
//! head 窗口只有 Anthropic 需要（`input_tokens` 在首帧 `message_start` 里），
//! 所以它由 [`crate::UsageProbe::needs_head`] 按实现开关，不是所有人都付这份钱。

#[cfg(test)]
mod tests;
