# 07 — 真实账本 profile

依赖：01–04。在双扣 / 陈旧缓存 / 丢恢复源的系统上做性能结论没有意义。

## 合同

一份可复现的计费热路径测量：真实 `Ledger` + `SqlUsageStore` + `ModelPriceCache` + Redis + 隔离 Postgres。上游继续 mock。压测结束必须 drain，再核对钱，不能只看 HTTP 200 和 RPS。

现有 `scripts/perf/perfkit` 的 NullLedger / FlatCalculator / NullUsageStore **不能**当计费证据。历史火焰图（2026-08-15，模拟账本）只说明网关非账本 CPU，不排名 SQL 锁或 Redis 脚本。

## 接缝

新测量模式，不要替换噪声地板基线：

维度：单用户 vs 多用户；冷/热缓存；带/不带订阅；非流式 / SSE / 取消 / 缺 usage。

并行记录：HTTP 延迟、结算滞后（pending 年龄）、Lua 脚本时间、`users` 行锁等待。

结束后断言：成功 settled intent 数、`balance_logs` 资金类行、残留 pending、残留 Redis hold、shortfall 数、成功 usage 行。

## 人能跑什么

在 perfkit 或 `scripts/perf` 增加 real-ledger 模式的命令与 README 用法。输出目录写清“这不是 NullLedger”。

## 验证

只有这份 profile 之后，才允许讨论：Lua HGETALL 随 hold 数线性、结算 SQL 往返合并、异步结算背压。那些是新切片，不写进 01–06。

## 实现者可自行决定

是否复用 `PHASES=` 还是新脚本。不要为了测量改准入公式。

## 必须保持绿色

现有 NullLedger 基线仍可跑，并标明它测的是非账本路径。
