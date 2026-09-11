# 01 — 请求成功结算幂等

## 完成状态（工作树已验证，未部署）

- 迁移是 `0011_settlement_intents.sql`，不是方案草稿里的 0009（0010 已被支付切片占用）。
- `Ledger::settle_tx` 在扣款前 CAS 占用 `request_id`；重放返回 `AlreadySettled`，调用方事务回滚。
- 已删除 `skip_if_already_logged`。失败 `usage_logs` 行不再挡住随后的成功结算。
- A0a/A0b/A0c 在隔离 PostgreSQL 上先红后绿；既有 usage 12 项、支付 12 项、账本 25 项保持绿色。
- 未给全部 `usage_logs.request_id` 或 settle 流水加唯一约束。

长期合同如下。

## 合同

同一个网关 `request_id`：任意两个连接（热路径、补账、重放）最多把 `users.balance` 减少一次，最多插入一行 `failed=false` 的 `usage_logs`。`failed=true` 的审计行不是成功标记，不能挡住随后的成功结算。

00 的 A0a/A0b/A0c 必须翻绿。B 的 Redis TTL 问题**本切片不关**。

## 接缝

主人：`Ledger::settle_tx`。`SqlUsageStore` 只编排，不再做 SELECT 查重。

1. `migrations/0009_settlement_intents.sql`（`IF NOT EXISTS`，禁止 DROP）：

```sql
CREATE TABLE IF NOT EXISTS settlement_intents (
  request_id text PRIMARY KEY,
  user_id bigint NOT NULL,
  hold_amount numeric NOT NULL DEFAULT 0,
  subscription_id bigint,
  status text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  settled_at timestamptz
);
-- status: pending | settled | released（01 只写 settled/released）
```

2. 在**同一事务**里、`lock_balance` 之后（或作为 debit 的前置 CAS）：

```text
INSERT INTO settlement_intents (request_id, user_id, hold_amount, status, settled_at)
VALUES ($id, $user, $cost, 'settled', now())
ON CONFLICT (request_id) DO UPDATE
  SET status = 'settled', settled_at = now()
  WHERE settlement_intents.status = 'pending'
RETURNING request_id;
```

RETURNING 为空 → `LedgerError::AlreadySettled`，事务回滚，钱不动。零费用成功也要占坑，避免后来的正费用重放再扣。

3. 删除 `SettlementCommit.skip_if_already_logged` 及所有赋值。热路径与补账走同一条 `commit_settlement`。FakeUsageStore 改为按 `request_id` 去重。

4. **不要**给全部 `usage_logs.request_id` 加唯一。失败行与成功行共用该列。可选 belt（仅当生产扫描无重复成功行）：

```sql
CREATE UNIQUE INDEX IF NOT EXISTS idx_usage_logs_request_id_success
  ON usage_logs (request_id)
  WHERE failed = false AND request_id IS NOT NULL AND request_id <> '';
```

23505 映射为 `AlreadySettled`，不是 500（500 会把 hold 留下直到 TTL）。

5. 禁止给 `balance_logs.reference` 在 `type='settle'` 上加唯一：shortfall 会写两行同 reference 的 settle（见 0008 注释）。

## 人能跑什么

00 的 A0a/A0b 变为：余额只减一笔，一行成功 usage，败者 `AlreadySettled`。A0c 变为：failed 行之后成功结算**会**扣一次。

```bash
cargo test -p gw-ledger --test postgres_ledger -- --ignored
cargo test -p gw-proxy --lib adapters::usage -- --ignored
```

## 验证

- 两连接并发同 `request_id`：一次 `Committed`，一次 `AlreadySettled`。
- 独立 `Ledger::settle`（若测试仍走它）同样幂等。
- 事务回滚后 intent 不得停在 `settled`。
- `claim_finalize` 行为不变（进程内）。
- HTTP 200 仍可早于 SQL。

## 实现者可自行决定

intent 行的辅助列（`hold_amount` 在 01 可用实际费用占位，03 再在 hold 时写入预留额）。`AlreadySettled` 放在 `LedgerError` 还是现有 `SettleReceipt`。

## 必须保持绿色

partial-debit / shortfall 测试；`a_client_chosen_trace_id_cannot_poison_the_settle_path`；迁移幂等。

## 会改变本切片的反馈

生产已有重复成功 `request_id`：先做人工去重（保留较早成功行），再考虑 optional unique。不要在脏数据上 `CREATE UNIQUE INDEX`。
