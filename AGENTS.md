# 使用中文和我沟通

# ai-gateway 项目规范

@docs/rust-engineering.md 是总工程规范。
后端是 Rust + axum，不依赖任何上游网关 SDK。
## 项目结构

```
ai-gateway/
├── Cargo.toml              # 虚拟根清单：resolver 3、edition 2024、MSRV 1.97
├── CONTRACT.md             # 工程契约：所有权、硬约束、数据库既成事实
├── rust-toolchain.toml     # 钉死 1.97.1 + clippy/rustfmt
├── migrations/             # sqlx SQL 迁移（对既有 schema 幂等）
├── crates/                 # 平铺，目录名 = crate 名（规则 1.3）
│   ├── gw-config/          #   YAML + 环境变量配置
│   ├── gw-model/           #   实体、迁移、种子、列解码适配器（compat）
│   ├── gw-infra/           #   PG 池、Redis、缓存、限流、熔断
│   ├── gw-authcore/        #   JWT、API Key、AES-GCM 凭证加密、AuthStore
│   ├── gw-pricing/         #   ModelPriceCache + 四列单价 Calculator
│   ├── gw-ledger/          #   Hold/Settle/Release 账本（Redis Lua + PG）
│   ├── gw-provider/        #   5 个上游 executor + 协议翻译 + usage 解析
│   ├── gw-proxy/           #   /v1/* 与 /v1beta/* 代理内核
│   ├── gw-panel/           #   /api/panel/** 运营面板，按业务域切分
│   └── gw-server/          #   组合根：装配、迁移、种子、优雅停机
├── tools/xtask/            # 架构门禁（cargo xtask ci）
├── docs/                   # 工程规范与调研文档
├── frontend/               # React 前端（独立构建）
├── deploy/                 # Dockerfile + compose
├── config.yaml             # 运行时配置（不入库）
└── config.example.yaml     # 配置模板
```

## 编码规范

**工程规范以 `/Volumes/Acasis/Code/REPO/ozon/ozon/ozon-pod/docs/rust-engineering.md` 为准**，
本文件不复述它（规则 5.6：索引文档只链接、不复述）。`CONTRACT.md` 记录本仓库已做出的决定。
最容易被违反、且已做成门禁的几条：

- **模块可达性**：每个 `.rs` 都必须能从 `mod` 声明到达。不可达文件不报错、不警告、review 也看不出来，
  但它一行都不会被编译。这是唯一一类代码审查绝对看不出来的缺陷 —— 靠 `cargo xtask ci` 拦
- **crate 依赖单向**：域之间不许反向依赖。共享词汇上收到 crate 根（`gw_panel::paging` 就是这么来的）
- **一个概念只声明一处**：写下第二个 `impl From<AFoo> for BFoo` 时先问为什么有两个 Foo
- **按业务域切模块，不按技术层**：删掉一个功能应该等于删掉一个文件夹。
  「退款」的用户路由与管理员路由住在同一个 `commerce/` 里
- **依赖 feature 收敛**：`[workspace.dependencies]` 只钉版本 + `default-features = false`，
  业务 feature 写在成员里
- **绝不写 `#![deny(warnings)]`**；CI 用 `RUSTFLAGS="-D warnings"`

## 计费流程

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

**停机时必须排空在途结算**：`StreamSettler::drop` 走 `TaskTracker`（流式中途断开，以及一元
把账本写入从请求路径卸下来的那条），`gw-server` 在 graceful shutdown
返回之后才 `close()` + `wait()`。顺序反了会让在途 Settle 随 runtime 一起死，hold 只能等 TTL —— 用户白嫖。

## 测试

```bash
make test          # cargo test --workspace（无需外部服务）
make test-ignored  # 需要真 Postgres/Redis 的那一档
make gates         # cargo xtask ci —— 9 条架构门禁
make lint          # clippy --all-targets -- -D warnings
```

需要外部服务的测试**要么 fail-loud 并告诉人怎么修，要么 `#[ignore]`，绝不允许读不到环境变量就 return** ——
那会让覆盖率变成假的。

**测试不许复述源码里的字面量**：把实现抄进断言的测试，通过是构造出来的。测不写死在源码里的性质。
写完一条守护性测试后，**先把它要防的 bug 塞回去、确认测试真的会失败**，再还原。

## 构建 & 运行

```bash
make build         # cargo build --release → ./ai-gateway
make run           # 构建并以 config.yaml 启动
./ai-gateway --config config.yaml
./ai-gateway --version
./ai-gateway --health-check   # 探针：ready → 0，否则 1
```

## 关键约束

- **前端零改动**：`frontend/` 是契约的另一半。JSON 字段名、大小写、嵌套、分页信封、HTTP 状态码、
  业务错误码（与 HTTP 码不是一回事）都必须与前端期望一致
- **数据库兼容**：表名/列名与既有 schema 一致，现有 Postgres 数据可直接读写。
  已实测 `pg_dump --schema-only` diff 为空。列的既成事实见 `CONTRACT.md` §3.5
  （钱是 `numeric`、整数列全是 `bigint` 必须用 `i64`、`o_auth_sessions` 这类反直觉列名）
- **计费语义不变**：Hold/Settle/Release 的签名与语义不可修改
- 每次提交必须 `cargo check --workspace`、`cargo xtask ci` 通过


