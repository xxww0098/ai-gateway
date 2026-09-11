# 00 — 钉死现有竞态

## 当前进度

A0a/A0b/A0c 已在隔离 PostgreSQL 上先红后绿（01）。B0 不等式单测与 400s hold 对 30 分钟扫描不可见为现窗表征。C0 无版本发布可写回更旧余额、D0 软额度并发用到 11，为保持 `cargo test -- --ignored` 可跑而断言现窗；04/06 修复时再翻合同。E0 仍由 02 保持。

## 合同

在**不改生产逻辑**的前提下，用真实 Postgres / Redis 测试把报告中的五类窗口变成必须失败的表征。A0b（两连接并发 skip=true）如果意外通过，停止 01 的“以防万一”唯一约束，先把实际 SQL 顺序打出来。

## 接缝

新测试，不改 `commit_settlement` / `settle_payment_order` / `clear_reservation` 行为。可加**测试专用**延迟钩子（例如 `clear_hold` 发布前 sleep），钩子默认关闭。

| 编号 | 断言（现码必须红） | 位置 |
|---|---|---|
| A0a | `skip=false` 串行两次 `commit_settlement` 同 `request_id` → 余额减两倍 | `gw-proxy` `adapters/usage/tests.rs` |
| A0b | 两 `PgPool` 连接、N 路 `skip=true` 并发 → 成功 usage 行 > 1 或余额减多于一笔 | 同上；模式抄 `payment_settlement.rs` 八路 |
| A0c | 先插入 `failed=true` usage，再 `skip=true` → `AlreadySettled` 且余额不动 | 同上 |
| B0 | `hold_keys_ttl(DEFAULT_HOLD_TTL) < DEFAULT_STALE_AFTER`；种 age=400s 的 hold 后 `scan_stale_holds(30min)` 为空 | `gw-ledger/tests/redis_ledger.rs` + 常量单测 |
| C0 | A 先提交 remaining=90 但推迟发布；B 提交并发布 70；A 再发布 → Redis GET=90 而 SQL=70 | `gw-ledger/tests/redis_ledger.rs` |
| D0 | 额度 10、已用 9，两路 `lock_and_rotate` + `evaluate_quota(1)` 都通过；两路结算后 used=11 | `gw-proxy` quota/hold 测试 |
| E0a | 种子 `paid` 且无 `payment_order:{id}` 流水 → `settle_payment_order` 不入账 | `gw-panel/tests/panel/payment_settlement.rs` |
| E0b | 入账成功后把订单改回 pending 再 settle → 余额翻倍（证明 credit 无唯一键） | 同上 |

禁止用 `FakeUsageStore` 证明 A。那是进程内 mutex，证不了 READ COMMITTED。

## 人能跑什么

```bash
# 若 gw-proxy 因 gw-provider 既有编译错误连不上，先单独记为阻断，不要改计费逻辑去“绕过”
cargo test -p gw-ledger --test redis_ledger -- --ignored
cargo test -p gw-proxy --lib adapters::usage -- --ignored
cargo test -p gw-panel --test panel payment_settlement -- --ignored
```

需要 `GW_TEST_DATABASE_URL` / Redis。缺环境时 `#[ignore]` 必须把 howto 写进 ignore 字符串（仓库规则 2.9）。

## 验证

- 上表每条在当前代码失败（或 B0 的 TTL 不等式单测通过作为表征）。
- 不改 `migrations/`。
- 既有串行测试（`the_reconcile_guard_makes_a_second_run_a_no_op`、支付八路成功路径）仍绿。

## 留给实现者的自由

测试夹具如何注入发布延迟；是否抽公共 `fresh_db` 助手。

## 必须保持绿色

gw-ledger / gw-pricing 现有 lib 测试；迁移幂等测试。

## 中止条件

- A0b 在真实两连接下**通过**：不要进入 01 加索引。把 `commit_settlement` 的实际语句顺序贴回本切片。
- gw-provider 编译阻断导致 00 无法运行：先修编译（`codex.rs` HeaderMap），仍不算计费切片。
