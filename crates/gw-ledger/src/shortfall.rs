//! 欠款（partial-debit 留下的 `shortfall_usd` 标记行）的查询与注销。
//!
//! `settle` 按 `min(balance, actual)` 扣款，扣不动的部分写成一条
//! `type='settle'`、`amount=0`、`metadata.shortfall_usd>0` 的标记行。只要用户名下
//! 还有一条这样的行没被配对的 credit 注销，[`Ledger::has_unresolved_shortfall`]
//! 就是 true，两处生产 preflight（`gw-proxy` 的 hold 中间件、面板的订阅购买）
//! 会把请求挡在 402 `outstanding_debt`。
//!
//! # 注销 = 零额注销，不是补钱
//!
//! [`Ledger::write_off_shortfall`] 写的配对 credit 的 `amount` 是 **0**：
//!
//! * 门解开（谓词只看 reference 配对，不看金额）；
//! * **用户余额一分不变** —— 公司自己吃掉这笔没收回来的服务成本，不额外送等额余额；
//! * `verify_balance_integrity` 按 `amount` 求和，加一行 0 不扰动它。
//!
//! 所以这条路径**不能**走 `Ledger::credit`：那个方法硬性要求 `amount > 0` 并且真的
//! 加余额，等于替用户把欠的钱补上再免掉，是两笔账。

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::{Ledger, LedgerError, log_type};

/// How long a "no unresolved shortfall" verdict is served from memory before
/// the `balance_logs` predicate runs again.
///
/// Only the **negative** is cached: the only writer that can create a debt is
/// [`Ledger::settle_tx`] in this same process, and it drops the user's entry
/// the moment it records a shortfall — so a locked gate is observed
/// immediately here. A `true` verdict is never cached, so a write-off opens
/// the gate on the very next request. Across processes the window is this
/// TTL, the same one-TTL contract the Redis balance cache already accepts.
pub(crate) const SHORTFALL_CLEAR_TTL: Duration = Duration::from_secs(10);

/// 「未清偿欠款」的债务侧谓词，`d` 是 `balance_logs` 的别名。
///
/// 占位符：`$1` = settle 类型，`$2` = credit 类型；调用方自己的参数从 `$3` 起排。
///
/// 判定与列表**必须共用这一份**：两处一旦漂移，就会出现「列表里看不到、门却锁着」
/// 这种运维无从下手的状态。
///
/// `COALESCE` 挡的是 shortfall 特性之前写下的老行 —— 它们的 `->>` 是 NULL，
/// 不 COALESCE 会让比较短路成 NULL 而不是 false。
///
/// `reference` 列可空，同样要 COALESCE：拼接里只要有一个 NULL，整串就是 NULL，
/// `c.reference = NULL` 恒为 NULL、`NOT EXISTS` 恒真，那条欠款就会永远锁着门。
/// 注销语句必须用**同一个** COALESCE 拼法，否则这类行看得见、锁着门、却注销不掉。
/// 生产写入路径产生不了空 reference（settle 的 request id 非空），历史/手工数据会。
///
/// 配对 credit 的 reference 形如 `shortfall_resolve:<债务行 reference>:<债务行 id>`
/// （见 [`crate::shortfall_resolve_reference`]）；把 credit 钉死在 `(reference, id)`
/// 这一对上，孤儿 credit（指向不存在的行）就无法冒名解掉一笔真欠款。
/// `d.metadata ? 'shortfall_usd'` 是下面那条 `COALESCE(...) > 0` 的**蕴含式**：
/// 键不在时 `->>` 是 NULL，COALESCE 兜成 0，比较为假。所以补上它不改变结果集。
///
/// 它存在的唯一理由是让 planner 认得出 `0014` 的
/// `idx_balance_logs_shortfall_open` 这个部分索引 —— 部分索引只有在查询谓词能
/// **语法上蕴含**索引谓词时才会被使用，而 `COALESCE(cast(...))` 蕴含不了一个
/// `?`。少了这一句，索引建了也不会被选中，热路径仍是按 user_id 的全量扫描。
pub(crate) const UNRESOLVED_DEBT_PREDICATE: &str = "\
    d.type = $1
    AND d.metadata ? 'shortfall_usd'
    AND COALESCE((d.metadata::jsonb ->> 'shortfall_usd')::float8, 0) > 0
    AND NOT EXISTS (
      SELECT 1 FROM balance_logs c
      WHERE c.user_id = d.user_id
        AND c.type = $2
        AND c.reference = 'shortfall_resolve:' || COALESCE(d.reference, '') || ':' || d.id::text
    )";

/// `EXISTS` 判定（门禁）用的整句。`$3` = user_id。
fn exists_sql() -> String {
    format!(
        "SELECT EXISTS (
  SELECT 1 FROM balance_logs d
  WHERE d.user_id = $3 AND {UNRESOLVED_DEBT_PREDICATE}
)"
    )
}

/// 门禁那句 SQL 的**生产原文**，额外暴露给集成测试。
///
/// 0014 的 idx_balance_logs_shortfall_open 只有在查询谓词能蕴含它的部分索引
/// 谓词时才会被 planner 选中；而「谓词被改回不可索引的形状」这个回归，任何
/// **行为**测试都发现不了 —— 结果集完全一样，只是从 0.05ms 退化成 29ms。
/// 所以 tests/postgres_ledger/plan.rs 直接对这份原文做 EXPLAIN。让测试自己抄
/// 一份 SQL 就失去意义了：抄件和原件可以各自漂移。
///
/// 占位符顺序与 [`Ledger::has_unresolved_shortfall`] 一致：
/// $1 = settle 类型，$2 = credit 类型，$3 = user_id。
#[must_use]
pub fn shortfall_gate_sql() -> String {
    exists_sql()
}

/// 列表的总数。
fn count_sql() -> String {
    format!("SELECT COUNT(*)::bigint FROM balance_logs d WHERE {UNRESOLVED_DEBT_PREDICATE}")
}

/// 列表的一页。`$3` = limit，`$4` = offset。
fn page_sql() -> String {
    format!(
        "SELECT d.id AS log_id, d.user_id, COALESCE(d.reference, '') AS reference, \
         COALESCE((d.metadata::jsonb ->> 'shortfall_usd')::float8, 0) AS shortfall_usd, \
         d.created_at \
         FROM balance_logs d WHERE {UNRESOLVED_DEBT_PREDICATE} \
         ORDER BY d.id DESC LIMIT $3 OFFSET $4"
    )
}

/// 零额注销的那一句。`$1` = 欠款行 id，`$2` = credit 类型，`$3` = settle 类型，
/// `$4` = 操作者 user id。
const WRITE_OFF_SQL: &str = "
INSERT INTO balance_logs (user_id, amount, type, reference, metadata, created_at)
SELECT d.user_id, 0, $2,
       'shortfall_resolve:' || COALESCE(d.reference, '') || ':' || d.id::text,
       jsonb_build_object(
         'user_id', d.user_id,
         'timestamp', to_char(NOW() AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'),
         'shortfall_usd', COALESCE((d.metadata::jsonb ->> 'shortfall_usd')::float8, 0),
         'written_off_by', $4::bigint
       ),
       NOW()
FROM balance_logs d
WHERE d.id = $1
  AND d.type = $3
  AND COALESCE((d.metadata::jsonb ->> 'shortfall_usd')::float8, 0) > 0";

/// 一条未清偿的欠款，管理员视图的一行。
///
/// 直接按 [`page_sql`] 的列名解码：`reference` 在 SQL 里已经兜成空串，
/// `created_at` 在既有 schema 里可空，所以留成 `Option`。
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct OutstandingShortfall {
    /// 欠款标记行的 `balance_logs.id` —— 注销时要传的就是它。
    pub log_id: i64,
    pub user_id: i64,
    /// 结算行的 reference，即产生欠款的那个 request id。
    pub reference: String,
    pub shortfall_usd: f64,
    pub created_at: Option<DateTime<Utc>>,
}

/// 一次零额注销的处置结果。形状对齐 `gw-panel` 的 `refund::Disposition`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOff {
    /// 配对 credit 已写入，门就此解开。
    Written,
    /// 这条欠款早就被注销过了（重复提交 / 并发的败者）。
    AlreadyResolved,
    /// 给定 id 不是一条真欠款行：不存在、类型不对、或没有正的 `shortfall_usd`。
    Missing,
}

impl Ledger {
    /// Whether `user_id` has a fresh "no unresolved shortfall" verdict.
    ///
    /// A poisoned lock reads as a miss: the caller then re-runs the real
    /// query, which is the safe direction for a billing gate.
    pub(crate) fn shortfall_clear_fresh(&self, user_id: i64) -> bool {
        self.shortfall_clear
            .lock()
            .map(|map| {
                map.get(&user_id)
                    .is_some_and(|at| at.elapsed() < SHORTFALL_CLEAR_TTL)
            })
            .unwrap_or(false)
    }

    /// Records the gate's latest reading for `user_id`: a clean verdict is
    /// cached, an unresolved debt drops any stale clean entry.
    pub(crate) fn note_shortfall_verdict(&self, user_id: i64, unresolved: bool) {
        if let Ok(mut map) = self.shortfall_clear.lock() {
            if unresolved {
                map.remove(&user_id);
            } else {
                map.insert(user_id, Instant::now());
            }
        }
    }

    /// 用户名下是否还有至少一条未清偿欠款。
    ///
    /// 一条就足够让调用方（hold 中间件的 preflight、订阅购买的 preflight）用
    /// HTTP 402 `outstanding_debt` 挡下计费动作。
    ///
    /// 只读，与 hold / settle / release 无关。出错时调用方按自己的策略决定失败方向；
    /// 今天两处都失败关闭（默认拒绝）。
    ///
    /// # Errors
    /// 底层查询错误。
    pub async fn has_unresolved_shortfall(&self, user_id: i64) -> Result<bool, LedgerError> {
        // 阴性短缓存：谓词本身已经是 O(欠款行) 的索引扫描（0014 的
        // idx_balance_logs_shortfall_open），但每个请求仍然要为此付一次到 Postgres
        // 的往返。只缓存「干净」—— 欠款的唯一生产者 settle_tx 在写下 shortfall
        // 行的同时会掐掉这条缓存，锁门立即可见；开门（注销）从不缓存 true，
        // 下一次查询就放行。见 SHORTFALL_CLEAR_TTL 的文档。
        if self.shortfall_clear_fresh(user_id) {
            return Ok(false);
        }
        let unresolved: bool = sqlx::query_scalar(&exists_sql())
            .bind(log_type::SETTLE)
            .bind(log_type::CREDIT)
            .bind(user_id)
            .fetch_one(self.db())
            .await?;
        self.note_shortfall_verdict(user_id, unresolved);
        Ok(unresolved)
    }

    /// 跨用户分页列出未清偿欠款，外加过滤后的总条数。
    ///
    /// 管理员看得见才谈得上注销 —— 面板此前没有任何入口能看到这些行。
    ///
    /// # Errors
    /// 底层查询错误。
    pub async fn list_unresolved_shortfalls(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<OutstandingShortfall>, i64), LedgerError> {
        let total: i64 = sqlx::query_scalar(&count_sql())
            .bind(log_type::SETTLE)
            .bind(log_type::CREDIT)
            .fetch_one(self.db())
            .await?;

        let rows: Vec<OutstandingShortfall> = sqlx::query_as(&page_sql())
            .bind(log_type::SETTLE)
            .bind(log_type::CREDIT)
            .bind(limit)
            .bind(offset)
            .fetch_all(self.db())
            .await?;

        Ok((rows, total))
    }

    /// 零额注销一条欠款：写一条 `amount = 0` 的配对 credit。
    ///
    /// `shortfall_log_id` 是欠款标记行的 `balance_logs.id`，`operator_user_id`
    /// 是执行注销的管理员，落进 metadata 备查。
    ///
    /// 一条 `INSERT … SELECT` 就是全部，天生无竞态：user_id 与 reference 都从欠款行
    /// **派生**，调用方只能给一个 id，伪造不了用户，也伪造不了金额；重复注销由
    /// `0008` 的分区唯一索引挡在库里，撞上就是 [`WriteOff::AlreadyResolved`]。
    ///
    /// 这里**不需要** `lock_balance`，也**不需要** `invalidate_balance_cache`：
    /// 余额一个字都没动，既没有读-改-写要串行化，Redis 里缓存的余额也仍然是对的。
    ///
    /// # Errors
    /// 除唯一冲突（映射成 `AlreadyResolved`）之外的底层查询错误。
    pub async fn write_off_shortfall(
        &self,
        shortfall_log_id: i64,
        operator_user_id: i64,
    ) -> Result<WriteOff, LedgerError> {
        let done = sqlx::query(WRITE_OFF_SQL)
            .bind(shortfall_log_id)
            .bind(log_type::CREDIT)
            .bind(log_type::SETTLE)
            .bind(operator_user_id)
            .execute(self.db())
            .await;

        match done {
            Ok(result) if result.rows_affected() == 0 => Ok(WriteOff::Missing),
            Ok(_) => Ok(WriteOff::Written),
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                Ok(WriteOff::AlreadyResolved)
            }
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests;
