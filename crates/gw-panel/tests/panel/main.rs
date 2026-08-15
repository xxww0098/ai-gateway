//! `gw-panel` 的连库集成测试 —— **一个二进制**。
//!
//! 规范 2.8：每个 `tests/*.rs` 都是一个独立 crate，各自链接一遍整个被测库。
//! 把这些用例拆成八个顶层文件会新增七次重量级链接，而拆成八个 `mod` 是零链接
//! 成本，且同样满足"按功能命名"（规范 2.4）。
//!
//! 每个文件对应一条业务不变量，命名按**测的是什么**，不按工单号：
//!
//! | 模块 | 不变量 | 原实现对应 |
//! | --- | --- | --- |
//! | [`bootstrap_admin`] | 有管理员之后引导路径永久失效 | `TestEnsureBootstrapAdmin*` / `TestRegisterBootstrap*` |
//! | [`subscription_purchase`] | 扣款与订阅同生共死；失败必补偿 | `TestPurchaseConservation` / `TestCompensationLogEmitted` / `TestPurchaseInsufficientBalanceNoWrites` |
//! | [`payment_settlement`] | 重复确认只入账一次 | `TestPaymentOrderSettlementCreditsOnceIdempotent` |
//! | [`redeem_code`] | 一张码只能被兑一次 | `TestRedeemCodePersistedAndSingleUse` |
//! | [`refund_disposition`] | 一份申请只有一个终态 | `TestRefundPersistedAndSingleDisposition` |
//! | [`balance_history`] | 翻页后累计余额仍然正确 | `TestBalanceHistoryRunningBalancePaginated` |
//! | [`group_entitlement`] | 非基线分组要有未过期订阅 | `TestAvailableGroupsFiltersByEntitlement` / `TestRebindRejectsUnentitled` |
//! | [`api_key_lifecycle`] | 明文只出现一次、撤销是软删 | —— |
//!
//! 全部标了 `#[ignore]`，见 [`common`] 的模块文档里的跑法。

// COORDINATOR-OWNED. Both panel workers add test modules here, so this entry
// point cannot belong to either of them — a cross-owner edit would be needed
// every time one of them adds a file. The modules themselves stay owned by
// whoever wrote them.
mod common;

mod api_key_lifecycle;
mod audit_chain;
mod balance_history;
mod bootstrap_admin;
mod group_entitlement;
mod payment_settlement;
mod pricing_cache;
mod redeem_code;
mod refund_disposition;
mod subscription_purchase;
