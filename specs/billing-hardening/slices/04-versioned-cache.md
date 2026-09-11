# 04 — Redis 余额条件发布

## 完成状态（工作树已验证，未部署）

- 迁移是 `0012_users_balance_version.sql`（0011 已被结算意图占用）。
- `set_balance` 递增 version；`SettleOutcome` / `BalanceChange::Applied` 带出。
- 发布 Lua 比较伴生键 `ai-gateway:billing:balance:ver:{userId}`；未改 HOLD/GET-BALANCE 准入脚本。
- credit/debit/支付提交后条件发布，不再 DEL 余额键。release / 零额 settle 只删 hold 成员。
- C0 与 stale-fill 测试在隔离 Redis+Postgres 上绿色。滚动窗口内旧二进制仍可能裸 SETEX，需短窗口弹齐。

长期合同如下。

建议在 01 之后做，以便 `SettleOutcome` 一次带上 version。不依赖 03。

## 合同

SQL 先提交 remaining=90 的 A，后提交 remaining=70 的 B；即使 A 最后才 `SETEX`，Redis 也必须是 70（或空），不能是 90。冷填充不得用更旧的 SELECT 覆盖更新的发布。偏高幅度按后续已提交扣款**之和**验收，不按“最多一笔”。

## 接缝

主人：`gw-ledger` 的 `set_balance` / `clear_reservation` / `refresh_balance_cache`。

1. `migrations/0011_users_balance_version.sql`：`ALTER TABLE users ADD COLUMN IF NOT EXISTS balance_version bigint NOT NULL DEFAULT 0;`
2. `set_balance`：`balance_version = balance_version + 1` 并 `RETURNING`。`settle_tx` / `credit_tx` / `debit` 带出 version。
3. 伴生键 `ai-gateway:billing:balance:ver:{userId}`（`keys.rs`）。**不要**把 version 编码进余额 string——`HOLD_SCRIPT` 的 `tonumber(GET)` 是切过合同。
4. 新发布 Lua（**不是**改 hold 汇总脚本）：

```lua
-- KEYS[1]=balance KEYS[2]=ver
-- ARGV[1]=balance ARGV[2]=version ARGV[3]=ttl
local cur = tonumber(redis.call('GET', KEYS[2]) or '0')
if tonumber(ARGV[2]) < cur then return 'STALE' end
redis.call('SETEX', KEYS[1], ARGV[3], ARGV[1])
redis.call('SETEX', KEYS[2], ARGV[3], ARGV[2])
return 'OK'
```

同 version 的填充允许（`>=` 或先比后写）；更新的 commit 必有更大 version。

5. `clear_reservation` 用该 Lua，禁止无条件 `SETEX`。
6. `refresh_balance_cache` 用 SELECT 当时的 `(balance, version)` 走同一 Lua，禁止 GET-then-SET。
7. `credit`/`debit` 改为发布 `(balance, version)`，不要 `DEL`：空键会让迟到的旧 SETEX 看起来像填充。
8. `release` / 零额 settle：只删 hold 成员，不要 `DEL` 余额键。

滚动发布：旧二进制仍可能裸 `SETEX`，窗口约 `balance_ttl`（默认 30s）。**不加双格式兼容层**；要求短窗口内弹齐实例。

## 人能跑什么

```bash
cargo test -p gw-ledger --test redis_ledger -- --ignored
```

C0 翻绿。另加：A 推迟、B 与 C 都发布后 A 再发布，缓存等于最终 SQL 而不是 90；填充 SELECT=100 与较新 settle=70 竞态，缓存停留 70。

## 验证

- Hold / get-balance Lua **字节级**不改准入公式。
- `moving_money_drops_the_cached_balance` 若断言 `DEL`，改为断言版本化发布后的值等于 SQL。
- 注释“最多偏一笔”删除或改成与测试一致。

## 实现者可自行决定

`clear_hold` 由 ledger 内部查 version，还是经 `SettleOutcome` 传出版本。优先后者，避免适配器把 version 丢在地上。

## 必须保持绿色

两路同时 hold 的 Lua 准入测试；commit 之后才清 Redis 的测试。
