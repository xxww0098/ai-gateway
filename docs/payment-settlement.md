# 支付确认：原子入账与重试边界

## 一个提交点，两个业务事实

支付确认必须把“订单已支付”和“余额已入账”放进同一个 PostgreSQL 事务。应用不应先提交 paid，再用另一笔事务充值并在失败时把状态改回 pending：进程退出不会执行这种补偿，旧补偿本身也会让重试再次入账。

[支付编排](../crates/gw-panel/src/commerce/payment.rs) 拥有订单状态；[账本余额操作](../crates/gw-ledger/src/ledger/balance.rs) 拥有用户余额及其流水。前者按 **订单行 → 用户行 → 流水** 的顺序加锁/写入，不把订单 SQL 下沉到账本，也不改变 Hold / Settle / Release。

新确认事务不会提交 paid 而缺少对应 credit 的状态。已有 paid 但缺流水的旧行，仍需经过重试才能修复；本改动不是全库自动回填任务。

## 只为支付定义去重身份

支付信用流水的身份是 `type = credit` 加字面前缀 `payment_order:` 的完整 reference。订单主键已经是全局身份，不再额外建一套幂等键。

- 同一支付 reference、同一用户、同一落盘金额：返回 `AlreadyApplied`，不再改余额。金额在 SQL 中按既有 `float8 → numeric` 入账转换比较，不用任意 epsilon，也不假设往返浮点表示无损。例如 `0.1 + 0.2` 落库为 `0.3`，同一入参重试必须被正确识别。
- reference 已存在但用户或金额不一致：报错并回滚，不把污染数据当作成功重试。
- 普通 credit / debit 的 reference 仍是审计描述，不自动变成幂等键。注册奖励、兑换和订阅购买各有自己的业务身份，后续应在各自事务中治理。
- 数据库的部分唯一索引同时限定类型和**字面**前缀。SQL `LIKE` 中 `_` 是通配符，因此使用 `LIKE 'payment!_order:%' ESCAPE '!'`；类似 `paymentXorder:` 的普通 reference 不应被误伤。

用户行锁使同一用户的账本写入串行，唯一索引为直接 SQL 或跨用户冲突兜底。不能捕获任意唯一冲突后返回成功。支付去重查询也带同一字面索引谓词，让参数化 reference 查询能够使用该部分索引。

## 事务接口不替调用者提交

`Ledger::credit_tx` / `debit_tx` 使用调用方的事务连接，只操作 PostgreSQL。成功结果为 `BalanceChange`；只有支付 credit 可能返回 `AlreadyApplied`，普通 debit 仍返回 Applied 或 `LedgerError::InsufficientBalance`。所有错误必须由业务调用方传播并回滚整笔业务事务，不能忽略错误后继续提交其它业务写入。

独立的 `credit` / `debit` 仍保留原有 `Result<()>` 接口，自己开始事务、调用对应 `_tx`、提交。支付编排则在同一事务内调用 `_tx` 后更新订单。提交后调用既有的 best-effort 余额缓存失效；事务回滚前不动 Redis，也不增加一条无版本的余额发布路径。

这里保证的是 PostgreSQL 原子性，不是 Redis 与 PostgreSQL 的分布式提交。缓存失效失败、提交后进程退出、余额版本乱序仍属于 [计费加固的后续工作](../specs/billing-hardening/README.md)。

## 历史修复与 HTTP 合同

- pending + 无 credit：写一次 credit，标记 paid。
- pending + 已有匹配 credit：恢复为 paid，不再充值。
- paid + 无 credit：补入一次，保留既有 paid 时间；后续重试不再入账。
- paid + 已有匹配 credit：保持不变。
- 不存在、取消或其它不可结算状态：不入账。

`settle_payment_order` 的 true 表示本次实际入账，false 表示没有新增入账。管理确认保留“成功 200、重复 409”的接口，Stripe 对有效重复通知仍返回 200；真实结算错误返回 500，让平台重投。HTTP 200 不单独证明钱已到账，验收必须读取余额和指定 reference 的流水。

历史修复依赖流水没有被人工删除/改写的前提；发现人工调账、错用户/金额或无法解释的旧记录时，先对账，不能盲目批量调用修复。数据补齐后是否移除 paid 修复分支，应按 [05 的核对清单](../specs/billing-hardening/slices/05-control-plane-siblings.md) 单独裁决。

## 部署前只读检查

以下 SQL 供部署负责人在备份、停写与对账流程中使用；本次开发只在新建的隔离测试集群执行测试，没有连接真实配置中的数据库。

### 重复支付 credit：必须先人工对账

```sql
SELECT reference, COUNT(*) AS credits,
       array_agg(id ORDER BY id) AS log_ids,
       array_agg(user_id ORDER BY id) AS users,
       SUM(amount) AS credited_amount
FROM balance_logs
WHERE type = 'credit'
  AND reference LIKE 'payment!_order:%' ESCAPE '!'
GROUP BY reference
HAVING COUNT(*) > 1;
```

结果非空时，新唯一索引会拒绝创建，这是防止带病启用去重的失败保护。**不要删除流水、改金额或丢弃索引来让迁移通过。** 重复 credit 可能代表真实重复余额，必须先决定财务处置。

### 已有流水与订单不一致

```sql
SELECT o.id AS order_id, o.user_id AS order_user, o.amount_usd,
       b.id AS log_id, b.user_id AS credit_user, b.amount
FROM payment_orders o
JOIN balance_logs b
  ON b.type = 'credit'
 AND b.reference = 'payment_order:' || o.id::text
WHERE b.user_id IS DISTINCT FROM o.user_id
   OR b.amount IS DISTINCT FROM (o.amount_usd::float8)::numeric;
```

这些记录应失败关闭，而不是自动认作重复入账。金额按网关既有 float8 → numeric 入账表示精确比较；这不是 Decimal 迁移，也不引入容差。另需人工确认旧订单金额为有限正数；不得为了消除告警盲目改写旧账。

### 已 paid 但没有对应 credit

```sql
SELECT o.id, o.user_id, o.amount_usd, o.paid_at
FROM payment_orders o
WHERE o.status = 'paid'
  AND NOT EXISTS (
    SELECT 1 FROM balance_logs b
    WHERE b.type = 'credit'
      AND b.reference = 'payment_order:' || o.id::text
  );
```

确认属于旧崩溃窗口后，经正常支付确认路径重试；观察余额与流水，不另写一条直接 UPDATE balance 的回填脚本。

## 切换与回退约束

1. 备份并完成上述只读检查，安排维护窗口，停止并排空旧版本的支付写入者（包括多副本、回调和后台任务）。
2. 使用包含 `0010_control_plane_credit_unique.sql` 的新二进制走既有迁移入口。普通 CREATE INDEX 会阻塞 balance_logs 写入，不把它描述为在线无锁迁移。
3. 只有迁移成功后才开放新支付确认路径。不要让旧的 pending→paid→credit 补偿实现与新修复实现同时写同一个库。
4. 回退同样需要停写并保持索引；不能直接恢复旧版支付补偿路径，不能通过 DROP INDEX 绕过兼容问题。

本次先处理 02，是因为 01 与 02 的约束只是同一 ledger 文件的写入所有权冲突，并无功能依赖。后续新增迁移必须选大于已部署最大版本的新编号，不补插一个较小的“预留编号”；方案中的文件编号不是已发布的迁移历史。
