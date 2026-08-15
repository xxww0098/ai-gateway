//! `RelayBody` / `RelayResponseBody` 的构造与消费。
//!
//! OWNER: worker `relay-core`。
//!
//! 这里是缺陷 #2（1 MiB 硬上限）的落点：入站 body 在 peek 阈值内走
//! [`crate::RelayBody::Buffered`]（peek 与转发共用同一块内存），超阈值走
//! `Streaming`（边收边转、无上限、计费降级到保守估算）。
//! **计费降级，转发不降级。**

#[cfg(test)]
mod tests;
