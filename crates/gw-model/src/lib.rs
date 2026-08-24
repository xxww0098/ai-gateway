//! 持久化实体 + SQL 迁移 + 启动种子。
//!
//! 本 crate 提供持久化实体、SQL 迁移和启动种子。
//!
//! # 与既有 schema 保持兼容
//!
//! 这个 crate 的第一约束不是「好看」，是**能读写既有的那个库**：
//!
//! * 表名/列名一律是历史建库的默认命名策略的产物。有几个反直觉的必须记住：
//!   `OAuthSession` → `o_auth_sessions`、`InputPricePer1M` → `input_price_per1_m`、
//!   `ApiKeyID` → `api_key_id`、`IPAddress` → `ip_address`。
//! * 既有 schema 里 `float64` 对应的是 Postgres `numeric` 列。sqlx 不会把 `NUMERIC` 解进 `f64`，
//!   所以实体字段上带 `#[sqlx(try_from = "compat::Money")]` —— 字段本身仍是 `f64`，
//!   JSON 输出与历史序列化逐位一致。细节和「写入方向要注意什么」见 [`compat`] 模块文档。
//! * 历史建库只在 tag 写了 `not null` 的列上加 NOT NULL，其余可空；它读 NULL 时给
//!   结构体填零值。实体里凡是历史非指针字段都走 `compat::{Text,Int,Bool,Ts}`
//!   适配器复刻这个行为，这样老库里的 NULL 不会让 Rust 端整行解不出来。
//! * 历史上是指针的字段（`*uint` / `*string` / `*float64` / `*time.Time`）保持
//!   `Option<T>` —— 那里的 NULL 是有语义的（"不限额" ≠ "限额 0"）。
//!
//! # 谁负责写
//!
//! 本 crate 只定义**行的形状**和迁移/种子，不含查询。具体的 SQL 由用它的 crate
//! 自己写（`gw-ledger` 的账本、`gw-panel` 的面板查询…），因为查询的取舍属于那些
//! crate 的业务，塞进来只会让这里变成一个谁都要改的公共池。
//!
//! OWNER: worker `model-migrations`。

// 规范 5.3 的棘轮：本 crate 的公开路径已经没有 `todo!()` / `unimplemented!()`，
// 就地把它们钉成 deny。根清单里的 `clippy::todo = "warn"` 是给还有存量骨架的
// crate 留的门，等最后一个清完再由协调者翻成工作区级 deny。
#![deny(clippy::todo, clippy::unimplemented)]

pub mod compat;
pub mod seed;

mod billing;
mod catalog;
mod channel;
mod commerce;
mod migrate;
mod sdk;
mod subscription;
mod support;
mod user;

pub use billing::{BalanceLog, BillingOperation, OperationLog, UsageLog};
pub use catalog::{ModelCatalogEntry, ModelPrice};
pub use channel::ChannelPolicy;
pub use commerce::{PaymentOrder, RedeemCode, Refund};
pub use migrate::{MIGRATOR, run as run_migrations};
pub use sdk::{AmpcodeConfig, AuthRecord, OAuthSession, ProviderConfig};
pub use subscription::{
    Subscription, SubscriptionPackage, next_daily_reset_after, next_monthly_reset_after,
    next_weekly_reset_after,
};
pub use support::{Announcement, Ticket, TicketReply};
pub use user::{ApiKey, Group, User, UserTokenVersion};

/// 所有实体主键的类型。
///
/// 既有 schema 是 `bigserial` / `bigint`，所以 Rust 这边是 `i64`
/// ——**不是** `i32`：用 `i32` 解 `INT8` 列会在运行时报类型不匹配。
pub type Id = i64;

#[cfg(test)]
mod testsupport;

#[cfg(test)]
mod tests;
