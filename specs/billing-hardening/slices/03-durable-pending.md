# 03 — 持久 pending 作为恢复源

## 完成状态（工作树已验证，未部署）

- Hold 成功后写入 `settlement_intents` pending；插入失败则释放 Redis hold 并 503 关闭。
- 上游失败走 `Ledger::release`：先 pending→released，再清 Redis。
- 扫描 `created_at < now() - hold_ttl` 的 pending，经已幂等的 `commit_settlement` 补账。不再用 Redis 成员扣款。
- 年轻于 hold TTL 的 pending 不收；过期 pending 补一次后再扫 no-op。
- `BILLING_AUTO_RECONCILE_HOLDS` 仍控制是否扣款；仪表来自 SQL pending 扫描结果。

长期合同如下。

依赖：01。没有成功 CAS 就打开扫描，重放会双扣。

## 合同

进程在 HTTP 200 之后、SQL 结算之前崩溃：超过 Redis hold TTL 之后，系统仍能识别该 `request_id` 为待结算，补账一次，再扫是 no-op。`failed=true` usage 不得阻止这次补账。

Redis hold 仍是短准入锁（默认 300s 可以保持）。扫描**不再**用 Redis 成员当扣款来源。

## 接缝

同一张 `settlement_intents`（01 已建）：

1. Lua hold 成功、dispatch 之前：`INSERT … status='pending' ON CONFLICT DO NOTHING`。插入失败 → **释放 Redis hold 并 fail closed**（与幂等存储失败同一姿势）。不要向上游发请求。
2. 上游失败 / Release：同一 SQL 事务（若有）或独立事务把 pending → `released`，再 Redis release。
3. 成功结算：01 的 CAS `pending→settled`（若 01 时还不存在 pending 行，INSERT settled 仍成立；03 之后正常路径是 UPDATE pending）。
4. `reconcile.rs`：`SELECT * FROM settlement_intents WHERE status='pending' AND created_at < now() - grace`。`grace = max(configured hold_ttl, 扫描宽限)`，**禁止**再硬编码与 TTL 无关的 30 分钟。补账走 `commit_settlement`（已幂等）。
5. Redis `scan_stale_holds` 最多留作 `agw_orphaned_holds` 仪表；**不得**再对扫描结果扣款。
6. `BILLING_AUTO_RECONCILE_HOLDS` 含义变为“重放过期 pending”，不是“SCAN Redis”。

补账费用策略（产品已有行为，本切片保持）：孤儿若从未写下真实 usage，按 **hold 金额**收。热路径若先用真实 usage 占到 settled，扫描必为 AlreadySettled。不要让估算覆盖真实 usage。

## 人能跑什么

```bash
cargo test -p gw-proxy --lib reconcile -- --ignored
cargo test -p gw-ledger --test postgres_ledger -- --ignored
```

新用例：写 pending、删 Redis 键 / 等到超过 hold TTL、跑扫描 → 扣一次；再扫 → 不扣。年轻于 `hold_ttl` 的 pending 不收。

## 验证

- B0 表征不再作为“功能正确”：默认 300s TTL 下扫描仍能靠 SQL 找到 pending。
- 配置 `hold_ttl_seconds: 3600` 时，未超过 TTL 的活请求不会被扫描扣款。
- 01 的并发测试仍绿。
- 不把 pending 存进 Redis list/ZSET。

## 实现者可自行决定

`enqueue_pending` 放在 `BillingLedger` 还是 `Ledger` 公开方法；是否把 `plan_settlement` 的实际费用回写到 intent（让扫描收真实费用而不是 hold）。若做回写：必须在 SQL 提交前，且不能把已 settled 改回 pending。

## 必须保持绿色

Hold Lua 准入公式；drain 仍等待进程内结算任务。03 覆盖的是 drain 之后的 SIGKILL，不是替代 drain。

## 中止条件

产品明确“崩溃丢账由运营承担、标志保持关闭”：不要做扫描扣款，只修仪表与文档，避免 3600s TTL + 30min 扫描误伤活请求。把该决定写回 README。
