# ai-gateway — 工程契约

ai-gateway：Rust + axum 实现的 LLM 中转网关，不依赖任何上游网关 SDK。

11 个 worker 分两波在**同一个 worktree**里工作。之所以能并行，全靠下面这份所有权表：
**只写属于你的文件，绝不碰别人的。**

> **工程规范以 [`docs/rust-engineering.md`](docs/rust-engineering.md) 为准（1108 行，必读）。**
> 本文件不复述它（规则 5.6：索引文档只链接、不复述，复述出来的那份就是半年后过期的那份）。
> 下面只列**本仓库已经做出的决定**，以及**该文档里最容易被违反的几条**。

---

## 1. 硬约束

| 约束 | 说明 |
| --- | --- |
| 前端零改动 | `frontend/` 一行都不能改。后端必须复刻前端契约的 JSON 字段名、大小写、HTTP 状态码、错误体 |
| 数据库兼容 | 表名/列名必须与既有 schema 一致，现有 Postgres 数据可直接被 Rust 读写 |
| 计费语义不变 | Hold / Settle / Release 三段式、partial-debit shortfall、strict-usage-metadata 模式，逐条对齐 `AGENTS.md` |
| 不改前端 | `frontend/` 只读。后端产出全部落在 crate 里 |

## 2. 已经定下的工程决定（别改，要改先 ask）

- 虚拟根清单，`resolver = "3"`，`edition = "2024"`，`rust-version = "1.97"`，内部 crate 一律 `version = "0.0.0"`（规则 1.2 / 1.4 / 4.4）
- crate 平铺在 `crates/`，**目录名 = crate 名**；`tools/xtask` 是唯一例外（规则 1.3 / 5.1）
- `[profile.dev.build-override] opt-level = 3` 与 per-package 覆盖是**一对**，谁都不许只删一半（规则 3.3，实测只删一半比不改还慢：269s vs 255s，配齐是 75.8s）
- **禁止** `[profile.*.package."*"]`（规则 3.2，反模式 #1）
- `[workspace.dependencies]` 只钉版本 + `default-features = false`；**业务 feature 写在成员里**（规则 4.1，反模式 #24）
- `[workspace.lints]` 已就位，每个成员都写了 `[lints] workspace = true`。当前 `clippy::todo` / `unimplemented` 是 `warn`（存量＝骨架里的 `todo!()`）；
  **谁清空了自己 crate 的最后一个 `todo!()`，就在同一个 commit 里把它翻成 `deny`**（规则 5.3 棘轮）
- 内部 crate 一律 `[lib] doctest = false`（规则 2.10）
- 薄 main、厚 lib：`gw-server` 的逻辑全在 `lib.rs`，`main.rs` 只有三行（规则 1.5 —— `tests/` 够不到 `main.rs`）
- **绝不写 `#![deny(warnings)]`**；CI 用 `RUSTFLAGS="-D warnings"`（规则 5.3）

## 3. 文件所有权（不可越界）

### 第一波（9 个并行）

| worker | 独占目录 / 文件 |
| --- | --- |
| `research` | `docs/**` |
| `platform` | `crates/gw-config/**`、`crates/gw-server/**`、`tools/xtask/**` |
| `model-migrations` | `crates/gw-model/**`、`migrations/**` |
| `infra` | `crates/gw-infra/**` |
| `authcore` | `crates/gw-authcore/**` |
| `ledger-pricing` | `crates/gw-ledger/**`、`crates/gw-pricing/**` |
| `provider-openai` | `crates/gw-provider/src/{common,openai,codex,usage,streambuf}.rs` |
| `provider-claude` | `crates/gw-provider/src/{claude,gemini,vertex}.rs` |
| `proxy-kernel` | `crates/gw-proxy/**` |
| `relay` | `crates/gw-relay/**` |

### 第二波（面板，依赖第一波）

`gw-panel` 按**业务域**切，不按 admin/user 角色切（规则 1.6：删掉一个功能应该等于删掉一个文件夹。
「退款」既有用户路由又有管理员路由 —— 它们必须住在同一个 `commerce/` 里，而不是散在 `user.rs` 和 `admin.rs`）。

| worker | 独占目录 |
| --- | --- |
| `panel-identity` | `crates/gw-panel/src/{identity,commerce,support}/**` |
| `panel-upstream` | `crates/gw-panel/src/{billing,upstream,ops}/**` |
| 两个面板 worker 共用 | `crates/gw-panel/tests/panel/*.rs`（各自写各自的用例文件，规则 2.8 的单一二进制布局） |

**协调者独占**（要改就 `ask`，别自己动手）：
`Cargo.toml`、`rust-toolchain.toml`、`.cargo/config.toml`、`CONTRACT.md`、
`crates/*/Cargo.toml`、`crates/gw-provider/src/{lib,types}.rs`、`crates/gw-panel/src/lib.rs`、
`crates/gw-panel/tests/panel/main.rs`（两个 worker 都要往里加 mod，归任一方都会造成跨属主编辑）。

## 3.5 数据库列的既成事实（由 model-migrations 实测确立，全员必读）

已用本机 Postgres 16 验证：既有库与
`migrations/0001-0003` 建出来的库，`pg_dump --schema-only` diff **完全为空**。
所以下面这些不是约定，是**已经固化的事实**，写查询时照做：

| 事实 | 后果 |
| --- | --- |
| 钱是 `numeric` 列 | 实体字段仍然是 `f64`，靠 `gw_model::compat::Money` 解码。**绑参直接传 `f64` 可用**；只有 `SUM()` 这类标量聚合要写 `::float8` |
| **所有整数列都是 `bigint`** | 一律用 `i64`。骨架里的 `concurrency: i32` 会在运行时报错，已改。别再写 `i32` |
| 反直觉列名 | `o_auth_sessions`（不是 `oauth_sessions`）、`input_price_per1_m`（不是 `input_price_per_1m`）。既有 schema 的既成事实，照抄别猜 |
| 绝大多数列可空 | 既有库只在部分列上加 NOT NULL；读 NULL 时 sqlx 报 `UnexpectedNull`。用 `gw_model::compat` 里的解码适配器，别直接解成 `String`/`i64` |

## 4. 测试要求（规则第二部）

- 单元测试用 `#[cfg(test)] mod tests;` **指向同目录的 `tests.rs` 独立文件**，不要内嵌 `mod tests { }`
  —— 内嵌的话改测试会重编库本身（规则 2.2）
- 集成测试**合并成少数二进制**，共享辅助放 `tests/common/mod.rs`（**不是** `tests/common.rs`，那会被当成一个跑 0 个测试的空二进制，规则 2.3 / 2.8）
- 集成测试按**功能**命名，不按工单号/冲刺号（规则 2.4）
- 需要 Postgres/Redis 的测试：**要么 fail-loud 并告诉人怎么修，要么 `#[ignore = "需要本地 X"]`。
  绝不允许「读不到环境变量就 return」** —— 那会让覆盖率变成假的（规则 2.9，反模式 #7）
- **测试不许复述源码里的字面量**（规则 2.11）。把实现抄进断言的测试，通过是构造出来的，直接删。
  测那些**不写死在源码里的性质**：单调性、边界被拒、跨进程一致性
- ★ **每个 `.rs` 都必须能从 `mod` 声明到达**（规则 2.6）。不可达文件不报错、不警告、review 也看不出来，
  但它一行都不会被编译。**自查方式已升级为门禁**：
  ```bash
  cargo xtask ci      # 9 条门禁，no_orphan_modules 是其中最重要的一条
  ```
  **不要再用 grep 数 `#[test]` 对账**。那个办法两个方向都会错：漏匹配带参数的
  `#[tokio::test(flavor = "multi_thread")]`（少报），也会数进测试夹具字符串里的 `#[test]`（多报，
  xtask 上实测虚报 85 vs 70）。门禁直接解析 `mod` 图与 `#[path]`，按 rustc 的真实规则判定可达性。

## 5. 你的完成标准

1. `cargo check -p <你的 crate>` 与 `cargo clippy -p <你的 crate>` 干净
2. `cargo test -p <你的 crate>` 通过，且上面第 6 节的自查两个数字对得上
3. 公开 API 有 doc comment，写明职责与关键不变量
4. 你负责的公开路径不留 `todo!()`；清空了就自己上棘轮（**不用 ask 我，下面这个写法已验证可用**）：

   在你 crate 的 `src/lib.rs` 顶部加一行
   ```rust
   #![deny(clippy::todo, clippy::unimplemented)]
   ```

   **不要**试图在成员 `Cargo.toml` 里写 `[lints.clippy] todo = "deny"` —— Cargo 会直接拒绝解析：
   `cannot override 'workspace.lints' in 'lints'`。`[lints] workspace = true` 与成员级 `[lints.*]`
   互斥，二选一。而放弃 `workspace = true` 去手抄整张表，等于把根清单复制一份到成员里，
   下次改根表就漂移了（规则 5.6）。

   注意这与规则 5.3 禁止的 `#![deny(warnings)]` 不是一回事：那条禁的是**笼统**的 deny
   （rustc 升级引入新 lint 就全仓库红灯）。点名两条具体 lint 没有这个问题。

   上棘轮前请自查三项，全绿再加：`grep -rn 'todo!\|unimplemented!' src/ | wc -l` 为 0；
   `#[test]` 数量与 `cargo test -p <crate> --lib -- --list | grep -c ': test$'` 相等（规则 2.6）；
   `cargo clippy -p <crate> --all-targets -- -D warnings` 通过。
5. 用 `CARGO_TARGET_DIR=/tmp/cargo-<你的名字>` 跑构建，避免多人抢同一个 target 锁
6. 用 `cargo check` 迭代，不要用 `cargo build`（规则 3.13）；**永远不要 `cargo clean`**（规则 3.8）

## 6. 卡住了怎么办

- 需要改协调者独占的文件、或需要加依赖 → `orca orchestration ask --question "..." --json`
- 发现别的 crate 的契约不够用 → `ask`，别自己去改别人的 crate
- 干完了 → 按 preamble 发一次 `worker_done`
