//! HoldLease 续租：把预留的到期时刻往后推，**金额一个字节都不动**。
//!
//! # 为什么需要它
//!
//! Redis 里的预留只有一个创建时间戳（`holds:ts` 哈希），两个 Lua 脚本都按
//! `now - hold_ttl` 当截止线清理它。`hold_ttl` 是 300 秒。于是一条健康的、
//! 跑了六分钟的流式回复会在**请求还活着的时候**被自己的过期清理掉 ——
//! 那一刻起余额闸门看不见这笔在途负债，后续请求可以把同一份钱再花一遍，
//! 而这条流的输出到结算时可能已经没有预留可扣（欠款，或干脆白嫖）。
//!
//! # 为什么不是「把 TTL 调大」
//!
//! 因为 TTL 同时是**崩溃后多久能把钱放出来**。调到一小时，一次进程崩溃就会
//! 冻住租户一小时的余额。300 秒是**租约的一片**，不是流的最长时长：
//! 流在活着的时候每 [`crate::DEFAULT_HOLD_TTL`] 的一半续一次，
//! 崩了就最多冻 300 秒。这两个需求本来就该由两个旋钮承担。
//!
//! # 为什么续租还有上限
//!
//! 否则「一条永不结束的流」= 永久冻结。[`DEFAULT_MAX_HOLD_DURATION`] 是
//! 那个硬顶：从**首次预扣**算起超过它就拒绝续租，预留随剩余 TTL 自然消亡，
//! 那一行留给对账。首次预扣的时刻取自 `billing_operations.created_at` ——
//! 持久的那一行，不是「最后一次续租 + TTL」这种自我延长的量。

use std::time::Duration;

use crate::LedgerError;
use crate::ids::BillingOperationId;
use crate::keys::{holds_key, holds_ts_key};
use crate::ledger::{Ledger, hold_keys_ttl};
use crate::operation::OperationState;
use crate::scripts::{HOLD_NOT_FOUND, RENEW_SCRIPT};

/// 一笔预留从首次预扣算起最长能活多久。
///
/// 与 [`crate::DEFAULT_HOLD_TTL`] 是**两件事**：那个是一片租约的长度
/// （也就是崩溃后余额被冻多久），这个是租约能续多少片。
pub const DEFAULT_MAX_HOLD_DURATION: Duration = Duration::from_secs(30 * 60);

/// 一次成功续租留下的事实。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeaseRenewal {
    /// 预留金额。**续租不改它** —— 这里没有「换个金额续租」这种操作。
    pub reserved_amount: f64,
    /// 从首次预扣到现在。
    pub held_for: Duration,
}

/// 租约是否已经用尽：从首次预扣算起到了硬顶。
///
/// `>=` 而不是 `>`：到点就是到点，一个恰好等于上限的租约不该再续一片。
#[must_use]
pub fn lease_exhausted(held_for: Duration, max_total: Duration) -> bool {
    held_for >= max_total
}

impl Ledger {
    /// 这个账本允许一笔预留活多久。
    #[must_use]
    pub fn max_hold_duration(&self) -> Duration {
        self.max_hold_duration
    }

    /// 刷新预留的租约到期时刻。
    ///
    /// 只动 `holds:ts` 里的那一个时间戳，**不碰 zset 的分数**，也不重新做
    /// 余额检查 —— 续租不是第二次准入，它不许改变谁欠谁多少钱。
    ///
    /// 刻意不写审计行：续租不动钱，而一条跑一小时的流会因此每分钟往
    /// `balance_logs` 塞一行噪音。「这笔预留活了多久」由持久那一行的
    /// `created_at` 回答。
    ///
    /// 调用方是 `gw-proxy` 的流式回写包装：客户端的 body 还在动，就周期性地
    /// 续一次。中继引擎不调它（`gw-relay` 是计费盲的）。
    ///
    /// # Errors
    /// [`LedgerError::HoldNotFound`] 当这个操作没有活着的预留（持久那一行不在、
    /// 已终态、或 Redis 里的成员已经过期）—— 续租**绝不凭空造出一笔预留**；
    /// [`LedgerError::LeaseExpired`] 当从首次预扣算起已经到达
    /// [`max_hold_duration`](Self::max_hold_duration)；
    /// [`LedgerError::RedisNotConfigured`] 当账本没有 Redis；
    /// 其余是底层查询 / Redis 错误。
    pub async fn renew_lease(
        &self,
        user_id: i64,
        operation: &BillingOperationId,
    ) -> Result<LeaseRenewal, LedgerError> {
        let Some(mut conn) = self.redis_conn() else {
            return Err(LedgerError::RedisNotConfigured);
        };

        // 首次预扣的时刻来自持久那一行 —— 它跨进程、跨 Redis 逐出依然成立。
        // 顺带把「这个操作还是不是 held、是不是这个租户的」一起查了：终态操作
        // 的预留没有任何续租的理由。
        let held_for = self.held_for(user_id, operation).await?;
        if lease_exhausted(held_for, self.max_hold_duration) {
            return Err(LedgerError::LeaseExpired);
        }

        let now = Ledger::now_unix();
        let key_ttl_secs = hold_keys_ttl(self.hold_ttl()).as_secs() as i64;
        let reply: String = {
            let mut inv = RENEW_SCRIPT.prepare_invoke();
            inv.key(holds_key(user_id));
            inv.key(holds_ts_key(user_id));
            inv.arg(operation.as_str());
            inv.arg(now);
            inv.arg(key_ttl_secs);
            match inv.invoke_async(&mut conn).await {
                Ok(reply) => reply,
                Err(err) if crate::ledger::marks(&err, HOLD_NOT_FOUND) => {
                    return Err(LedgerError::HoldNotFound);
                }
                Err(err) => return Err(err.into()),
            }
        };
        let reserved_amount = reply.parse::<f64>().map_err(|err| {
            LedgerError::Other(anyhow::anyhow!("parsing renewed hold amount: {err}"))
        })?;

        Ok(LeaseRenewal {
            reserved_amount,
            held_for,
        })
    }

    /// 这个操作从首次预扣到现在过了多久。
    ///
    /// 读的是 `billing_operations`：非终态操作的唯一真相。行不在、不是这个
    /// 租户的、或已经终态，都是 [`LedgerError::HoldNotFound`]。
    async fn held_for(
        &self,
        user_id: i64,
        operation: &BillingOperationId,
    ) -> Result<Duration, LedgerError> {
        let created_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
            "SELECT created_at FROM billing_operations \
             WHERE billing_operation_id = $1 AND user_id = $2 AND state = $3",
        )
        .bind(operation.as_str())
        .bind(user_id)
        .bind(OperationState::Held.as_str())
        .fetch_optional(self.db())
        .await?
        .ok_or(LedgerError::HoldNotFound)?;

        // NULL 的 `created_at`（历史行允许为空）读作「刚刚」：把一个未知年龄
        // 当成已经到顶会立刻杀掉一条健康的流，当成刚开始最多多续一片。
        Ok(created_at
            .map(|at| (chrono::Utc::now() - at).to_std().unwrap_or(Duration::ZERO))
            .unwrap_or(Duration::ZERO))
    }
}

#[cfg(test)]
mod tests;
