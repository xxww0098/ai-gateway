-- 一张支付订单最多只能入账一次（billing-hardening slice 02）。
-- 支付结算把 `credit_tx(..., "payment_order:{id}")` 与订单状态翻 `paid` 放进同一
-- 事务，靠这里的分区唯一索引把幂等钉在库里，不靠应用层"先查再写"。对已 paid 但
-- 缺流水的历史行，重试经同一索引补入恰好一次。
--
-- 分区条件必须同时限定 `type = 'credit'`：其它业务的同名 reference 不是支付
-- 入账身份，而且一次 shortfall 结算会写多条同 reference 的 settle 流水。
-- 不能让支付的唯一性约束干扰这些合法审计记录。
--
-- LIKE 模式里的下划线是单字符通配符，所以 `payment_order:%` 也会匹配
-- `paymentXorder:%`。这里用 `ESCAPE '!'` 把它钉成字面下划线；调用方
-- （gw-ledger 的 credit_tx 预查）带着同一个字面谓词做参数化查询，
-- 规划器才能证明谓词蕴含索引条件、真正用上这个索引。
--
-- 不要把唯一性放宽到全部 credit：`initial_register_credit` 是跨用户的同一字面量
-- （注册奖励的身份包含 user_id）。兑换、订阅等业务的事务与去重另行治理，
-- 不属于本索引的作用域。
--
-- 脏数据语义：如果历史上真的发生过双重入账（同 reference 的多行 credit），本索引
-- 的创建会**失败并阻断启动迁移** —— 这是有意的。绝不自动删除钱的数据；运维必须
-- 先用 slice 02 的预检 SQL 核对每一条重复行、人工裁决后再放行。
CREATE UNIQUE INDEX IF NOT EXISTS idx_balance_logs_payment_order_credit
    ON balance_logs (reference)
    WHERE type = 'credit' AND reference LIKE 'payment!_order:%' ESCAPE '!';
