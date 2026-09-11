-- 服务面的一笔入账最多只能落账一次（ozon-pod 服务面契约 §4）。
-- 服务面把 `credit_tx(..., "service_credit:{idempotency_key}")` 放进一个事务，
-- 靠这里的分区唯一索引把幂等钉在库里，不靠应用层"先查再写"：超时重试不会重复
-- 入账，同一 reference 被换成另一个 user 或另一个金额会撞索引并被拒绝（409），
-- 绝不对账到另一笔钱。
--
-- 分区条件必须同时限定 `type = 'credit'`：同一个 reference 在 settle / debit
-- 流水里不是入账身份（一次超预留结算会写两条同 reference 的 settle 行），
-- 放宽会把这些合法审计记录一起锁死。
--
-- LIKE 模式里的下划线是单字符通配符，所以 `service_credit:%` 也会匹配
-- `serviceXcredit:%`。这里用 `ESCAPE '!'` 把它钉成字面下划线；调用方
-- （gw-ledger 的 credit_tx 预查）带着同一个字面谓词做参数化查询，
-- 规划器才能证明谓词蕴含索引条件、真正用上这个索引。
--
-- 脏数据语义：历史上若真的存在同 reference 的多行 credit，本索引的创建会**失败
-- 并阻断启动迁移** —— 这是有意的。绝不自动删除钱的数据；运维必须先人工核对每一条
-- 重复行、裁决后再放行。
CREATE UNIQUE INDEX IF NOT EXISTS idx_balance_logs_service_credit
    ON balance_logs (reference)
    WHERE type = 'credit' AND reference LIKE 'service!_credit:%' ESCAPE '!';
