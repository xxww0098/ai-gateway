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

## 计费与停机

```
Request → access 中间件（API Key/JWT → 租户）
  → hold 中间件
      preflight：有未清偿欠款 → 402 outstanding_debt
                 max(holdAmount, EstimateWithMaxTokens, Estimate) 与余额比对
                 不足 → 402 insufficient_balance，且不创建 Redis hold
      → ledger.hold（预扣）
  → gw-provider 执行（含跨账号 failover）
  → 旁路解析 usage
  → 结算：pricing.compute 精算 → ledger.settle 或 release → 写 UsageLog → 累加订阅配额
```

三种结算模式必须都在：

- **正常**：按上游真实 usage 精算
- **fallback**：上游缺 usage 且非 strict 时，用 `max(ActiveHoldAmount, Estimate(stream=true))` 兜底，
  并在 `UsageLog.RawMetadata.billing_fallback.reason=missing_upstream_usage` 标注
- **strict**：`billing.strict_usage_metadata_mode=true` 时不结算不释放，
  写 `UsageLog{failed=true, reason=missing_upstream_usage_strict}`，hold 随 TTL 过期

其余不变量：`settle` 按 `min(balance, actual)` 做 partial-debit，欠款写 `BalanceLog.Metadata.shortfall_usd`，
通过 `shortfall_resolve:<requestID>:<debitLogID>` 的 Credit 配对解除；订阅购买 `Debit` 成功但建订阅失败时，
立刻以 `subscription_purchase:<pkgID>:compensate:<debitRef>` 回滚。

停机时必须排空在途结算：`StreamSettler::drop` 走 `TaskTracker`（流式中途断开，以及一元
把账本写入从请求路径卸下来的那条），`gw-server` 在 graceful shutdown
返回之后才 `close()` + `wait()`。顺序反了会让在途 Settle 随 runtime 一起死，hold 只能等 TTL —— 用户白嫖。

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

**当前状态**：`make lint`、`cargo test --workspace`（含 `--ignored` 全档）、`cargo xtask ci`、前端
`npm run typecheck` / `npm test` / `npm run build` 全绿。

**仍待处理的既存 bug（与环境无关，未在本次修复范围内）**：`GET /api/panel/admin/audit-logs`
（`gw-panel/src/ops/audit_log.rs` 的 `fetch_panel`/`fetch_sdk`/`fetch_balance`）把日期过滤参数按 `text` 绑定，
无过滤时也会让 Postgres 报 `operator does not exist: timestamp with time zone >= text`，导致列表接口 500。
测试档未覆盖该路径。修法：把查询里第二处 `created_at >= $3` / `<= $4` 也显式写成 `$3::timestamptz` / `$4::timestamptz`。
