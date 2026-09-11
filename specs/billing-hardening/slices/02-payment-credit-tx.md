# 02 — 支付订单与入账同事务

## 完成状态（工作树已验证，未部署）

- E0a/E0b 在隔离 PostgreSQL 16 实测原实现失败（4 个原有用例通过、2 个新增失败），修复后通过。
- 独立复审发现金额浮点往返误拒重试；新增用例先红，改为 SQL 落盘金额比较后绿。第二次独立复审未发现剩余实质新增回归。
- 真实 PostgreSQL：账本 25 项、panel 全部 61 项（含支付 12 项）、model 8 项通过，共 94 项；未连接生产库。
- 工作区默认测试 1734 passed / 0 failed / 148 ignored；上述 94 项另行启用，不混算。Redis 档未执行。
- 全 workspace 严格 Clippy 与 9 条架构门禁通过。generic plan 的 EXPLAIN 确认支付查询可使用部分索引，不作为性能测量。
- 未部署、未自动回填旧账；请求结算幂等与 Redis 版本发布仍待后续切片。

长期理由与上线约束见 [支付确认说明](../../../docs/payment-settlement.md)。

## 合同

不存在持久状态 `paid ∧ ¬credit(payment_order:{id})`。管理确认、Stripe webhook、崩溃重试、并发确认：最多入账一次。已 paid 但缺流水的历史行，重试会补入一次。

00 的 E0a 翻成“补入恰好一次”；E0b 翻成“第二笔 credit 为 AlreadyApplied，余额不变”。

## 接缝

主人：`Ledger::credit_tx` 管钱；`settle_payment_order` 管订单状态，必须加入同一 `Transaction`。

```text
Ledger::credit_tx(&mut PgConnection, user_id, amount, reference)
  -> Applied { balance_after } | AlreadyApplied

Ledger::debit_tx(&mut PgConnection, user_id, amount, reference)
  -> Applied { balance_after } | LedgerError::InsufficientBalance

Ledger::credit(...) = begin + credit_tx + commit + 缓存失效/发布
Ledger::debit(...)  = begin + debit_tx  + commit + 缓存失效/发布
```

本切片支付路径只调用 `credit_tx`。`debit_tx` 一并提供，供 05 的订阅购买加入同一事务；不要让 05 再发明第二种 debit。

实现边界澄清：两者返回 `Result<BalanceChange, LedgerError>`，只有 `payment_order:` credit 支持 AlreadyApplied。普通 credit/debit 继续允许重复审计 reference；不能在本片悄悄把所有 reference 升格为幂等键。05 应在自己的业务状态/身份上实现重复请求处理。支付 reference 命中其它用户或不同落盘金额时必须报错，不是 AlreadyApplied。金额按入账相同的 PostgreSQL float8 → numeric 转换比较，不直接比较二进制浮点往返值，也不使用任意容差。

`credit_tx` / `debit_tx` 形状对齐 `settle_tx`：锁用户、改余额、插 `balance_logs`，**不碰 Redis**。Redis 仍在 commit 之后。InsufficientBalance 必须中止整段调用方事务。

支付（单事务）：

```text
BEGIN
  SELECT payment_orders WHERE id FOR UPDATE
  若 pending:
      credit_tx(..., "payment_order:{id}")
      UPDATE status='paid'
  若 paid:
      credit_tx(...)   -- 唯一约束下缺流水则补，已有则 AlreadyApplied
  COMMIT
  发布/失效缓存
```

删除 `paid→pending` 补偿。它制造“credit 已提交、状态被改回 pending、重试再 credit”的窗口。

迁移 `0010_control_plane_credit_unique.sql`（学 0008，分区唯一）：

```sql
CREATE UNIQUE INDEX IF NOT EXISTS idx_balance_logs_payment_order_credit
  ON balance_logs (reference)
  WHERE type = 'credit' AND reference LIKE 'payment!_order:%' ESCAPE '!';
```

**不要** unique 全部 credit：`initial_register_credit` 是跨用户字面量（05 用 `(user_id, reference)`）。

面板不得把 `payment_orders` SQL 塞进 `gw-ledger`。

本切片**只改支付**。兑换/订阅/注册见 05。本轮优先处理已确认的支付 P1，02 先于 01 实施；原顺序只用于避免同一 ledger 文件的并行写入，并非功能依赖。未来 01 使用大于已部署最大版本的迁移编号，不补插较小的预留版本。

## 人能跑什么

```bash
cargo test -p gw-panel --test panel payment_settlement -- --ignored
cargo test -p gw-ledger --test postgres_ledger -- --ignored
```

部署前必须停写、预检查重复 credit/用户金额冲突，并避免新旧支付实现混跑；完整 SQL 和回退约束见 [支付确认说明](../../../docs/payment-settlement.md)。`LIKE` 的下划线要转义，不能误约束 `paymentXorder:`。以下只列旧 paid 缺流水的检查：

```sql
SELECT id FROM payment_orders o
 WHERE status='paid'
   AND NOT EXISTS (
     SELECT 1 FROM balance_logs
      WHERE type='credit' AND reference = 'payment_order:' || o.id
   );
```

## 验证

| 场景 | 期望 |
|---|---|
| pending 结算一次 | paid + 一行 credit |
| 种子 paid 无流水 | 补入一次 |
| 上述再并发 8 路 | 仍一行 |
| 已 paid 且已有流水 | 余额不变；webhook 仍 200，管理确认可保持 409 |
| `credit_tx` 后 ROLLBACK | 仍 pending，重试入一次 |
| 真 ledger 错误 | 500，Stripe 可重试 |

不要用 webhook HTTP 200 当入账证明；断言 `users.balance` 与 `balance_logs.reference`。

## 实现者可自行决定

`AlreadyApplied` 的类型名；webhook 对“已补入”是否与“原本已付”共用 200。

## 必须保持绿色

现有八路并发成功测试；mock 创建支付接口；退款 approve 仍不动余额。

## 脚手架

历史 paid-无-credit 的 ensure 分支：数据补完且 E0a 不再出现后，在 05 或后续提交删掉“从 paid 补入”的特殊路径，只保留“pending 同事务入账 + 已付 AlreadyApplied”。删除条件写进 05 的核对清单。
