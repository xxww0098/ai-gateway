# 使用中文沟通

AI-GateWay：Rust + axum LLM 中转，密钥只认 `agw-`。本文件只写实践与禁令，细节只引用。

- 产品、目录、运行、测试：[README.md](README.md)
- Rust 工程规范：[docs/rust-engineering.md](docs/rust-engineering.md)
- 已定契约（schema、crate 边界）：[CONTRACT.md](CONTRACT.md)
- 用户接入：面板 `/docs`；Harness：[plugins/agw-oauth](plugins/agw-oauth)

## 一、应遵循的实践

选择满足当前需求的最简实现，避免不必要的抽象、配置和间接层。

分层构建系统，从能端到端运行的最小版本起步，逐步叠加新功能，始终保证已有产品可用。

保持组件模块化和关注点分离；优先使用成熟维护的库，并优先利用项目已有依赖，不随意新增包。

做长期架构决策，不接受权宜之计。

本仓库已经定下、写代码时必须带着走的事实：

- 品牌是 AI-GateWay，新密钥只发 `agw-`。
- 公开 `/v1` JSON、Hold/Settle/Release 语义、既有表列名都不改。钱是 `numeric`/`f64`，整数是 `bigint`/`i64`，OAuth 表是 `o_auth_sessions`。
- **钱的键是服务端生成的 `BillingOperationId`**，不是客户端可控的 `X-Trace-ID`。见下文「计费身份」。
- **推理 HTTP 只从 `gw-relay` 出网**，`gw-provider` 只规划路由不发包。见下文「谁发包」。
- 面板 JSON 与业务错误码是契约。接入说明只写 `gw-proxy` 里真实存在的路径（Claude 用站点 origin + `/v1/messages`，不要发明 `/v1beta`）。
- 按业务域切模块；删一个功能等于删一个文件夹。规范细节走 `docs/rust-engineering.md`，门禁走 `cargo xtask ci`。

## 二、明确禁止的行为

不要保留向后兼容性，直接移除废弃路径，不添加兼容层、回退或迁移脚本。

不要无理由重写通用功能，也不要未经文档和类型检查就假设库缺乏某项能力。

不要用未完成的复杂性换取当前可用的产品，也不接受计划以后替换的临时方案。

因此也不要：

- 接受或兼容 `cpa-`、`CPA Gateway`、旧 localStorage 键。
- 改 Hold/Settle/Release 的签名与语义，或让停机跳过在途结算排空（见下文「计费与停机」）。
- 为「以后可能用到」加抽象、新依赖、或第二套同名类型。

## 计费身份

**一次计费操作的身份由服务端生成，客户端碰不到。**

| 类型（`gw-ledger`） | 谁生成 | 键住什么 |
| --- | --- | --- |
| `BillingOperationId` | **服务端**，在 hold 准入处 mint | hold / settle / release / 对账 / `usage_logs.event_key` |
| `ClientTraceId` | 客户端 `X-Trace-ID` 或进程内生成 | 日志、响应头、`usage_logs.request_id` |
| `IdempotencyScope` | 由客户端 `Idempotency-Key` 派生 | **客户端重试**去重，不是钱的键 |
| `UpstreamAttemptId` | dispatcher，每次 HTTP 尝试一个 | 一个 RoutePlan / failover 下的单次尝试 |

四个是**四个类型**，不是四个 `String`：`ClientTraceId` 传不进要 `BillingOperationId` 的参数位。
从前它们是同一个 `X-Trace-ID`，于是一个**客户端能自己指定的头**同时是账本的键 ——
重放或撞车就落到同一行。

由此定下的不变量：

- `X-Trace-ID` **只是观测**。它不许出现在 hold / settle / release / 对账 / `event_key` 的参数里。
- `usage_logs.event_key` 是操作 id 的文本，**已结算的行上永不为空串**；`request_id` 留给客户端链路 id。
- 预付模式下 `reserved_amount` == `admitted_liability`：**准入时拿去和余额比的那个上限就是预留住的数**，
  不是一个更小的下限 —— 预留得比认可的责任少，正是大请求结算成欠款的来路。
- 同一个 `BillingOperationId` 换了金额或换了请求指纹再来预扣 = `HoldError::OperationConflict`，
  **不是 OK，也不是覆盖**。终态操作同理不许重开。

## 计费与停机

```
Request → access 中间件（API Key/JWT → 租户）
  → hold 中间件
      preflight：有未清偿欠款 → 402 outstanding_debt
                 max(holdAmount, EstimateWithMaxTokens, Estimate) 与余额比对
                 不足 → 402 insufficient_balance，且不创建预留
      → mint BillingOperationId（服务端）
      → ledger.admit_operation：先写 billing_operations 那一行，再取 Redis 预留
  → gw-provider 规划路由（RoutePlan，含跨账号 failover）
  → gw-relay 逐字节中继（旁路 UsageProbe 读 usage，不进回写路径）
  → 结算：pricing.compute 精算 → ledger.settle_once 或 release_once
          → 写 UsageLog（event_key = 操作 id）→ 累加订阅配额
```

**`settle_once` / `release_once` 归 `gw-ledger`**，语义是「首个终态调用赢」：
条件更新 `UPDATE billing_operations SET state=... WHERE state='held'` 拿到行锁，
并发的第二次看到 0 行，返回 already-terminal，**不再扣第二次钱**。
这条保证写在事务里，**没有调用方开关** —— 一个需要记得打开的幂等保护就是一个会被忘记的保护。

`billing_operations`（Postgres）是**非终态操作的唯一真相**。Redis 里的预留是缓存：
它会过期、会被逐出、会随机器一起没，这些都不能说明钱有没有入账。所以对账扫的是
`terminal_at IS NULL` 的行，**不是 Redis 的 TTL**。「预留最后清」依旧：PG 提交成功而
Redis 清理失败时，那一行仍然可对账。

三种结算模式必须都在：

- **正常**：按上游真实 usage 精算
- **fallback**：上游缺 usage 且非 strict 时，用 `max(ActiveHoldAmount, Estimate(stream=true))` 兜底，
  并在 `UsageLog.RawMetadata.billing_fallback.reason=missing_upstream_usage` 标注
- **strict**：`billing.strict_usage_metadata_mode=true` 时不结算不释放，
  写 `UsageLog{failed=true, reason=missing_upstream_usage_strict}`；操作停在 `held`，
  预留随 TTL 过期，那一行留给对账

其余不变量：`settle` 按 `min(balance, actual)` 做 partial-debit，欠款写 `BalanceLog.Metadata.shortfall_usd`，
通过 `shortfall_resolve:<billingOperationID>:<debitLogID>` 的 Credit 配对解除；订阅购买 `Debit` 成功但建订阅失败时，
立刻以 `subscription_purchase:<pkgID>:compensate:<debitRef>` 回滚。

停机时必须排空在途结算：`StreamSettler::drop` 走 `TaskTracker`（流式中途断开，以及一元
把账本写入从请求路径卸下来的那条），`gw-server` 在 graceful shutdown
返回之后才 `close()` + `wait()`。顺序反了会让在途 Settle 随 runtime 一起死，hold 只能等 TTL —— 用户白嫖。

## 谁发包

**`gw-relay` 是工作区里唯一的推理 HTTP 出口。**

- `gw-provider` 只产出 `RoutePlan`（端点、凭证、provider 自己的头、可能改写过的 body）——
  一个纯值，不含 socket。`Provider::execute` / `execute_stream` 已**删除**，不是弃用。
- 唯一留在 `gw-provider` 的 HTTP 是**凭证刷新**，集中在 `oauth.rs`：它打的是身份提供方的
  token 端点，不带租户载荷，响应不回给客户端，也不在请求路径上。
- 这条规矩由 `gw-provider/src/route/tests.rs` 的源码扫描守着：除 `oauth.rs` 外任何文件出现
  `.send()`、或任何文件出现 `execute` / `execute_stream`，测试就红。加新上游时请照 `RoutePlan` 写。

为什么不是两套：provider 那套自己实现的回写路径会在非 2xx 上丢掉整份 header
（429 的 `retry-after` 收不到，SDK 就按自己猜的退避打回去）、把多值 `set-cookie` 折成一条、
把流式中途失败表现成一次干净的 EOF。中继层已经把这三件事做对了，第二套只会把它们做错。

## 列名既成事实

见 [CONTRACT.md](CONTRACT.md) §3.5。钱是 `numeric`/`f64`，整数列是 `bigint`/`i64`，OAuth 表是 `o_auth_sessions`。

## 测试

需要外部服务的测试**要么 fail-loud 并告诉人怎么修，要么 `#[ignore]`，绝不允许读不到环境变量就 return** ——
那会让覆盖率变成假的。

**测试不许复述源码里的字面量**：把实现抄进断言的测试，通过是构造出来的。测不写死在源码里的性质。
写完一条守护性测试后，**先把它要防的 bug 塞回去、确认测试真的会失败**，再还原。

## Cursor Cloud specific instructions

依赖（Rust 1.97.1 工具链、cargo-watch、`frontend/node_modules`、PostgreSQL 16、Redis 7）已随快照装好；
update script 只做增量刷新（`cargo fetch` + `frontend` 的 `npm ci`）。下面是**每次会话需要自己做**的、不显然的启动约定。

**必须手动起服务（本 VM 没有 systemd）**：

```bash
sudo pg_ctlcluster 16 main start                     # PostgreSQL 16
sudo redis-server /etc/redis/redis.conf --daemonize yes   # Redis
```

- `pg_hba.conf` 里 `127.0.0.1/32` 与 `::1/128` 已改成 `trust`，所以 `config.example.yaml` 的空密码能连；
  角色 `ai_gateway` 已建、已授 `CREATEDB`，库 `ai_gateway`（运行用）与 `ai_gateway_test`（跑 `--ignored` 用，已灌好迁移）都在。
- 仓库根的 `config.yaml` 已生成（`.gitignore` 忽略、随快照保留），含开发用 `JWT_SECRET`/`CREDENTIAL_ENCRYPTION_KEY`
  与 `bootstrap_admin_email: admin@example.com`（首个注册该邮箱的用户自动升管理员）。丢了就 `cp config.example.yaml config.yaml` 再 `make gen-secrets` 填密钥。

**跑起来**（命令本身见 [README.md](README.md) / [dev.sh](dev.sh) / [Makefile](Makefile)，不复述）：先起 PG/Redis，再 `./dev.sh`
（后端 `cargo watch` 在 `:8888`、前端 vite 在 `:3000`，vite 把 `/api`、`/v1`、`/healthz` 代理到 `:8888`）。

**`--ignored` 集成档的连接串**（README 只给了范例，这里给本机可直接用的一组）：

```bash
DATABASE_URL=postgres://ai_gateway@127.0.0.1:5432/ai_gateway_test \
GW_TEST_DATABASE_URL=postgres://ai_gateway@127.0.0.1:5432/ai_gateway_test \
GW_TEST_REDIS_URL=redis://127.0.0.1:6379 \
  cargo test --workspace -- --ignored
```

`gw-model` 读 `DATABASE_URL`、其余读 `GW_TEST_*`；`gw-model`/`gw-panel` 会 `CREATE/DROP DATABASE gw_*_test_*`（靠 `ai_gateway` 的 CREATEDB），
`gw-authcore`/`gw-infra`/`gw-ledger` 直接用连接串指向的库（`ai_gateway_test` 已有迁移）。若换用 `postgres` 超级用户跑过，
留下的 `gw_*_test_*` 库属主是 `postgres`，`ai_gateway` 删不掉 —— 用 `postgres` 身份 `dropdb` 清掉即可。

**当前状态**：`make lint`、`cargo test --workspace`、`cargo xtask ci` 全绿。
`--ignored` 全档需要本机 PG/Redis（见上），换机器后请先起服务再跑一遍。
