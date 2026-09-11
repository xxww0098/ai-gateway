-- 计费热路径上的两个部分索引。
--
-- 两处都是「表在无界增长，而查询只关心其中极小一部分」的形状：
-- 全表扫描随表增长线性变慢，而真正命中的行几乎恒定为常数个。
-- 用部分索引把索引体积钉在「命中行数」而不是「表行数」上。
--
-- ⚠️ 运维注意：下面两条是**非 CONCURRENTLY** 的建索引语句，会取
-- ACCESS EXCLUSIVE 锁。sqlx 的迁移在一个事务里执行，写不了 CONCURRENTLY。
-- balance_logs 很大时请在维护窗口内升级，或先手工
-- `CREATE INDEX CONCURRENTLY` 建好同名索引再跑迁移（IF NOT EXISTS 会跳过）。
-- 与 0008 / 0010 / 0013 在 balance_logs 上建索引的做法一致。

-- 1) 「这个用户名下还有没有未清偿欠款」——gw-proxy 的 hold 前闸门
--    （Ledger::has_unresolved_shortfall）与面板的订阅购买前闸门，每次
--    查询都跑一遍 UNRESOLVED_DEBT_PREDICATE。
--
--    谓词里的 `COALESCE((metadata->>'shortfall_usd')::float8, 0) > 0` 不可
--    索引，所以此前是「按 user_id 取全部流水，再逐行解析 jsonb」：一个
--    50 万行的租户实测 29ms，且随流水线性增长。阴性结论只缓存 10s，
--    于是每个活跃租户每 10s 就要付一次这个代价。
--
--    这里只索引**带 shortfall_usd 键**的行 —— 只有真欠款行会带这个键，
--    普通 settle / credit 行不进索引，写放大接近于零。查询侧在
--    UNRESOLVED_DEBT_PREDICATE 里补一句 `d.metadata ? 'shortfall_usd'`
--    （逻辑上是 `COALESCE(...) > 0` 的蕴含式，不改变结果集），
--    planner 才能把两者对上。同一份数据实测 29ms -> 0.049ms。
CREATE INDEX IF NOT EXISTS idx_balance_logs_shortfall_open
    ON balance_logs (user_id)
    WHERE type = 'settle' AND metadata ? 'shortfall_usd';

-- 2) 崩溃恢复扫描——reconcile 每 5 分钟跑一次
--    `WHERE status = 'pending' AND created_at < now() - grace`
--    （Ledger::list_pending_intents）。
--
--    settlement_intents 每个可计费请求写一行、**从不删除**，终态行
--    （settled/released）占绝对多数。没有索引时这是一次全表并行顺序扫描：
--    150 万行实测 58ms，并且随部署时长线性增长。索引只收 pending 行，
--    扫描代价变成 O(未结算请求数)。同一份数据实测 58ms -> 0.025ms。
CREATE INDEX IF NOT EXISTS idx_settlement_intents_pending_created
    ON settlement_intents (created_at)
    WHERE status = 'pending';
