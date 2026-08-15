# Rust 大型项目开发宝典

> 面向 10 万行以上、多 crate、多人协作的 Rust 工程。
>
> 每条规则都由两类依据支撑：**（一）** 业界共识（微软 Pragmatic Rust Guidelines、matklad 的 rust-analyzer 三篇、Roman Kashitsyn 的 IC 经验、corrode / Rust Project Primer，以及 Cargo Book 与 Rust Book）；
> **（二）** 在一个 233,238 行真实 Rust 仓库上的**实测**——包括**三条推翻了流行建议的测量结果**。
>
> 标注约定：**［实测］** = 本文作者在真实仓库上跑过并计时；**［共识］** = 来自权威规范或多篇文章的一致结论；**［推翻］** = 实测结果与流行建议相反。完整出处见附录 C。

---

## 序 · 怎么用这本宝典

### 三条总纲

1. **结构**：单 crate 内按**业务域**组织模块；多 crate 必进 workspace，平铺在 `crates/` 下；**逻辑在 lib、入口在 main**；crate DAG **要宽不要深**。
2. **测试**：单元测试跟着代码走（大模块放到独立 `tests.rs`），集成测试合并成少数二进制且只测公共 API，内部 crate 关掉 doctest。
3. **构建**：**先量化（`cargo build --timings`）再优化**；冷构建调 profile、热增量拆 crate、CI 另开一份（关 incremental、关 debuginfo）。

### 一条元规则（最重要）

> **不能被机器检查的规则，会腐烂。**

写在文档里的约定，半年后一定与代码不一致——不是因为团队不自律，而是因为没有任何东西会在它被违反的那一刻叫停。

本宝典每一部的末尾都给出**该部规则如何变成 `cargo xtask` 门禁**。**规则与门禁是一对，只写规则等于没写。**

一个真实的教训［实测］：某仓库的 ADR 明文规定「发布前必须运行 `local-release-gates.sh` 验证架构、测试、备份恢复」。该脚本在一次**以重构 wire 层为目的**的 30 文件删除提交中被顺带删掉，同批消失的还有 5 个部署测试套件。**两份引用它的 ADR 都没有修订，此后数月无人察觉。**

---

# 第一部 · 项目结构

## 1.1 两个及以上 crate 必须统一进 workspace ［共识：M-CARGO-WORKSPACE］

**为什么**：workspace 的核心价值**不是**「组织 crate」，而是**共享 `Cargo.lock` + 共享 `target/` 编译缓存**。两个独立 package 各编一份依赖，是纯粹的浪费。

**怎么做**：

```toml
[workspace]
members = ["crates/*", "apps/desktop/src-tauri", "services/cloud"]
resolver = "3"          # edition 2024 + 声明了 rust-version 时，resolver 3 是 MSRV 感知的，严格更优
```

> 注意：**虚拟清单不会从 edition 推断 resolver**，必须显式写。微软 M-LATEST-EDITION 说「一般不必写 resolver」——那条只对带 `[package]` 的根清单成立。写 `"2"` 是能用但过时的选择。

## 1.2 根清单用虚拟 manifest ［共识：matklad］

根 `Cargo.toml` **只有 `[workspace]` 段，没有 `[package]`**。不要把主 crate 放在根上。

**为什么**：主 crate 一旦占据根位置，它就成了「默认的那个」，新代码会不断被塞进去，因为放别处需要额外决策。仓库最大的那个 crate 通常就是这么长出来的。

## 1.3 crate 平铺，目录名 = crate 名 ［共识：M-CRATES-FLAT-FOLDER + matklad］

**规则**：所有 crate 平铺在 `crates/` 下**一级目录**；超过 1–2 打 crate 才考虑分层。**禁止把 crate 塞进另一个 crate 的 `src/` 里。**

**为什么不嵌套**：没有完美的层级；加新 crate 时要纠结放哪；树会随项目腐烂。rust-analyzer 的 32 个 crate 全部平铺。

**为什么目录名必须等于 crate 名**：`cargo tree`、编译错误、`--timings` 报告里出现的都是 **crate 名**。当目录叫 `psd-x` 而 crate 叫 `psdx` 时，你无法把报告里的一行映射回磁盘上的一个目录。

**多语言单仓库的例外**：`apps/ services/ tools/` 这类顶层划分**不是** Rust 拓扑决策——当 `tools/` 下同时有 Rust crate 和 pnpm 包时，按部署角色划分是合理的。M-CRATES-FLAT-FOLDER 管的是 **crate 的布局**，不是多语言仓库的布局。**把这个例外写进文档，而不是留给后人重新发现。**

**反例**［实测］：某仓库的 `services/cloud/migration` 是一个独立 workspace 成员，却位于 `services/cloud` 这个 package 的目录内部。更糟的是主 crate 用 15 处 `#[path = "../migration/src/*.rs"]` 把它的源码**再编译一遍**——5,049 行里有 3,967 行被编译两次，一次进 `libcloud_migration.rlib`，一次进 `libozon_cloud.rlib`。

## 1.4 内部 crate 固定 `version = "0.0.0"` ［共识：matklad］

不发布的 crate 不需要维护 semver。用 `[workspace.package]` 继承：

```toml
[workspace.package]
edition = "2024"
rust-version = "1.97"    # 库 crate 应声明 MSRV
version = "0.0.0"
```

成员侧：

```toml
[package]
name = "my-crate"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
```

**反模式**：成员里硬编码 `version = "0.1.0"` / `edition = "2024"`。它们不会报错，只会在你需要统一升级时逐个绊你一下。

## 1.5 薄 main、厚 lib ［共识：proj-lib-main-split］

**规则**：业务逻辑放 `lib.rs`（及其模块树），`main.rs` 只做参数解析与调用。

**为什么**：这不是审美问题——**集成测试（`tests/`）只能访问库的公共 API，无法 import `main.rs` 里的任何类型**。逻辑写在 `main.rs` 里 = 这部分逻辑永远无法被集成测试覆盖。

**标准形态**：

```rust
// src/main.rs — 16 行是健康的
fn main() {
    my_app_lib::run();
}

// src/lib.rs — 只有 mod 声明和一个入口
#![forbid(unsafe_code)]
pub mod domain;
pub mod infra;
pub fn run() { app::run(); }
```

多个二进制放 `src/bin/`，每个文件自动成为独立目标。

## 1.6 按业务域组织模块，不按技术层 ［共识：M-BALANCED-MODULES + rustprojectprimer］

**操作性判据（proj-mod-by-feature）**：

> **删掉一个功能，应该等于删掉一个文件夹。**

如果删除「优惠券」功能需要动 `api/`、`service/`、`repository/`、`dto/` 四个目录，说明你是按技术层切的。

**分层架构（API / service / repository）的失效方式**［实测］：某 80,476 行的 crate 按技术层漂移后，测得——

- 持久化层（`store`）反向 import 了 4 个业务域，**29 处引用全部来自同一个文件的头部**；
- API 层（`commands`）里藏着产品规则：一个「适配器」函数硬编码了「每次投递 1–15 张图」的业务上限；
- 结果是 **13 对互相依赖的模块、58 个 3–4 节点的环**。

用上面的判据实测该仓库的 5 个功能，**每个功能平均散落在 6 个顶层模块**，无一收敛在单个文件夹内。

**正确形态**：

```
src/
├── domain/          # 业务域：无 SQL、无 HTTP、无 UI 框架类型
│   ├── listing/
│   ├── billing/
│   └── account/
├── infra/           # 可替换的适配器：可依赖 domain，反之不可
│   ├── store/       # 持久化
│   ├── http/        # 外部 API 客户端
│   └── secret/
└── ipc/             # 对外适配层：唯一允许出现框架 DTO 的地方
```

**选型标准**（常见五种模式）：

| 模式 | 适用规模 | 判据 |
| --- | --- | --- |
| 扁平 | < 1,000 行 | 单文件模块够用 |
| 分层（API/service/repo） | 中等，且**领域简单** | 竖切，容易退化成上面的反例 |
| **DDD 按业务域** | **大型系统首选** | 有明确的领域词汇表 |
| 插件（trait + registry） | 需要开放扩展点 | **没有真实扩展需求时不要用**——投机抽象 |

模块命名也按业务域，不按技术角色。`traits/`、`errors/`、`types/` 这种目录**不帮任何人找到东西**——它们是分层架构在模块名上的残留［共识：M-BALANCED-MODULES］。用户必须找到才能用的类型（`Client`、`Draft`）放 crate 根；其余按用例分组（`account`、`network`）。

## 1.7 可见性阶梯：默认私有，逐级放开 ［共识：M-BALANCED-MODULES］

```
(私有) → pub(super) → pub(crate) → pub(in path) → pub
```

**每一个 `pub` 都是一个承诺。** 公共 API 越小越好。

**自查指标**［实测］：健康的大型 crate，`pub` 与 `pub(crate)` 的比例应接近 1:1 或更保守。某仓库实测 **1,579 个 `pub` 对 190 个 `pub(crate)`，比例 8.3:1**，其中两个模块（合计 10,763 行）**一次都没用过 `pub(crate)`**。

进一步测量：468 个可从 crate 外触达的 `pub` 条目中，**169 个（36%）在自己模块之外从未被引用**——纯粹的过度暴露。

**最省力的修法**：不要逐个条目改，**先改 `mod` 声明**。把 `pub mod internal_thing;` 降为 `pub(crate) mod internal_thing;`，其内部所有 `pub` 条目一次性被封住。

## 1.8 依赖必须单向；环是拆分的死敌 ［共识 + 实测］

**Rust 允许 crate 内部模块成环，禁止 crate 之间成环。**

这个不对称是本条规则存在的全部理由：**模块环不会报错，所以它会一直长，直到你想拆 crate 的那天才发现拆不动。**

**实测**［实测］：某 80,476 行 crate 的 15 个顶层模块中，**10 个处于同一个强连通分量，覆盖 65,761 行（占该 crate 81.7%）**。这个仓库从未成功拆出过任何 crate——不是没人想，是没人能。

**诊断方法**（可直接抄）：

```bash
# 提取模块间 crate::X 引用，构建邻接表，跑 Tarjan SCC
grep -rn "crate::[a-z_]*" src/ --include='*.rs' | ...
```

**破环方法**［共识：Kashitsyn］：**把共享类型抽到第三个模块/crate**。

环几乎总是由「A 需要 B 的类型、B 也需要 A 的类型」造成。抽出 `records` / `types` / `model` 之类的叶子 crate，双方都依赖它，环立刻消失。

**更便宜的破环**：如果 A 只依赖 B 的**类型名**、不依赖 B 的字段定义，把那个类型改成关联类型，A 就不必再依赖 B——改 B 的字段不会再重编 A 及其下游［共识：Kashitsyn］。

**判断哪些模块「今天就能拆」**：算**出度**。出度为 0 的模块（只被别人依赖、不依赖别人）可以立即抽成 crate，不必等破环。

抽出之后立刻检查：新 crate 会不会变成**反向依赖枢纽**（见 1.11）。一个什么都依赖的 `types` crate，只是把环换成了全仓库重编。

## 1.9 一个概念只声明一处

**反例**［实测］：某仓库的「类目 Schema」模型曾被**平行声明了三套**（分属两个模块），代价是 **208 行纯 `From<>` 转换代码**，以及一个 2,418 行的「legacy」模型——它**仅有的两个消费者，都在明文规定「不拥有」它的那个模块里**。Desktop 现已收成 listing 拥有的一份 `CategorySchemaSnapshot`（`listing/records.rs` 类型体 + `listing/schema.rs` 行为）。

**识别信号**：当你写下第二个 `impl From<AFoo> for BFoo` 时，停下来问：为什么有两个 Foo？

## 1.10 文件长度是复杂度的另一种形态

深路径 `use crate::a::b::c::d::Thing` 常被当成结构过复杂的信号。这是对的，但**不完整**。

**实测发现**［实测］：某仓库 768 条 crate 内 `use` 的深度分布是 `0:139 / 1:417 / 2:211 / 3:1 / 4+:0`——**路径深度问题几乎不存在**。但同一个仓库里：

| 文件 | 行数 |
| --- | --- |
| `store/mod.rs` | **8,768**（其中单个 `impl` 块 3,493 行 / 79 个方法） |
| `psd.rs` | **7,428**（251 个类型声明） |
| `tests/postgres_http.rs` | **7,069**（23 个测试） |

**结论：树很浅不代表结构健康。** 当没有目录层级可用时，复杂度不会消失，它会变成**文件长度**。

**经验阈值**：单文件超过 **1,000 行**、单 crate 超过 **5,000 行** 就该审视一次。这不是硬性上限，而是「该停下来看一眼」的信号。

**好消息**：浅树意味着**有拆分余量**。加一层 `store/repo/*` 只会把最大深度从 2 推到 3，仍远低于警戒线。

## 1.11 crate DAG 要宽不要深 ［共识：matklad Fast Builds + Kashitsyn］

**一个 crate 最重要的属性，是它（传递地）不依赖哪些 crate。**

编译是按 crate DAG 并行的。下面两条链的总代码量可以一样，墙钟时间差一个数量级：

```
# 慢：必须串行
A → B → C → D → E

# 快：公共词汇 + 互不依赖的功能 + 叶子拼装
          ┌→ feature_a ┐
vocab ────┼→ feature_b ├─→ app
          └→ feature_c ┘
```

**两种必须拆开的依赖枢纽**：

| 枢纽 | 症状 | 修法 |
| --- | --- | --- |
| 出度大（依赖很多） | 改它自己很慢；它挡住整张图的并行 | 按被测对象拆 `test-utils`，按功能拆 `replica` 式拼装 crate |
| 入度大（被很多依赖） | **改一行，半个仓库重编** | 让它小、让它稳；能用关联类型消掉的依赖不要变成真实 `use` |

**组件接线优先运行时多态**［共识：Kashitsyn］。`Arc<dyn Interface>` 比重泛型参数 `Foo<A, B, C>` 少一份单态化税，也少一份「改一个 impl 就重编所有接线方」的增量。编译期多态留给热路径上真正需要零成本的地方。

这是 3.9「crate 是重编译边界」在结构上的对称命题：拆 crate 只在 DAG 变宽时才赚到并行；拆成一条更深的链，冷构建只会更慢。

## 1.12 工作区内依赖走 `[workspace.dependencies]`，禁止手写 `path` ［共识：M-CRATES-IN-WORKSPACE］

```toml
# ❌ 成员互相用相对路径
[dependencies]
sibling = { path = "../sibling" }

# ✅ 一律经 workspace 表
# 根：
[workspace.dependencies]
sibling = { path = "crates/sibling" }
# 成员：
[dependencies]
sibling.workspace = true
```

手写 `path` 不会报错，但会让「到底用的是哪一份」无法从根清单一眼看清，也让 `cargo tree` / 架构门禁必须同时解析两种写法。

## 第一部的门禁

```rust
// xtask: 结构类门禁
check_no_orphan_modules()      // 见 2.6，最重要的一条
check_module_direction()        // 构建 crate::X 邻接矩阵，断言允许表
check_crate_direction()         // 解析各成员 Cargo.toml，断言依赖方向
check_no_raw_path_deps()        // 1.12 —— 成员间禁止 path = ".."，必须 workspace = true
check_file_length()             // 超过阈值的文件进白名单，只许减不许增
```

---

# 第二部 · 测试

## 2.1 七层测试与健康配比 ［共识：Rust Book + rustprojectprimer］

| 层 | 位置 | 作用 |
| --- | --- | --- |
| 单元 | `#[cfg(test)] mod tests;`（独立文件） | 覆盖内部逻辑，可访问私有项 |
| 集成 | `tests/` | **只测公共 API**，是契约的守卫 |
| 文档 | `///` 里的代码块 | 保证示例不腐烂 |
| 属性 | `proptest` / `quickcheck` | 生成式覆盖输入空间 |
| 异步 | `#[tokio::test]` | — |
| E2E | 独立目标 | 系统级 |
| 基准 | `benches/` | **性能是需求时必备** |

**健康配比**：大量快速单元测试 + 适量保护公共契约的集成测试 + 少数系统测试。

**最常被跳过的两层，和跳过的代价**：

- **基准测试**［实测］：某仓库在根 `Cargo.toml` 用注释明确写着「PSD 光栅化含计算密集循环，故设 `opt-level = 3`」——但仓库里 **`benches/` 目录数量为 0**。**为一个从不度量的指标，支付一份永久的构建成本。** 没有基准，你既不知道优化是否有效，也不知道哪天它被改回去了。
- **文档测试**：公共 API 的可执行文档。某仓库 1,115 个单元测试对应 **2 个可执行 doc test**。对已发布的库这是缺口；对内部 crate 这是正确的税（见 2.10）。

## 2.2 单元内嵌、集成走 `tests/` ［共识：Rust Book ch11 + M-INTEGRATION-TESTS］

```rust
// src/parser.rs —— 单元测试就放在被测代码下面（测试即文档）
pub fn parse(s: &str) -> Result<Ast> { ... }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn parses_empty_input() { ... }
}
```

**大模块应优先用文件变体**［共识：matklad］：

```rust
#[cfg(test)]
mod tests;   // 指向同目录 tests.rs / parser_tests.rs
```

只改测试时，Cargo 知道这份文件只在 `--test` 下参与编译，**不会重编库本身**。内嵌 `mod tests { ... }` 做不到这一点。**`#[cfg(test)]` 门控绝不能忘。**

**绝不要**：为了让测试能访问，把私有函数改成 `pub(crate)`。测试应当适应设计，而不是设计迁就测试。

## 2.3 共享辅助必须放 `tests/common/mod.rs`，**不是** `tests/common.rs` ［共识：Rust Book ch11］

这是 Rust 测试组织里**最经典的陷阱**：

- `tests/common.rs` → 被当作**一个独立的集成测试二进制**编译，你会看到一个跑了 0 个测试的空测试目标；
- `tests/common/mod.rs` → 正确，作为模块被 `mod common;` 引入。

**跳过它的代价**［实测］：某仓库**全仓库不存在任何 `tests/common/`**——既没有正确形式，也没有错误形式，是从未尝试共享。结果：

| 辅助函数 | 逐字节重复的份数 |
| --- | --- |
| `fixture_psd` | **14** |
| `solid` | 6 |
| `json_eq` / `golden_json` / `clean_for_json` | 各 5 |

## 2.4 集成测试按**功能**命名，不按冲刺号或工单号

`user_auth.rs` ✅ · `auth.rs` ⚠️（太泛） · `text_edit_m14.rs` ❌ · `ticket46_tests.rs` ❌

**为什么**：`_m14` 编码的是「什么时候写的」，`ticket46` 编码的是「哪个工单要求的」——两者都不是「测了什么」。半年后想知道「文本编辑功能覆盖够不够」，你必须逐个打开文件。**这直接摧毁了「测试即文档」的价值。**

**实测**：某仓库有 19 个文件按冲刺/工单命名，其中 5 个 `composite_*_m15` 文件按**冲刺**而非**行为**切分，边界对读者完全不可预测。

## 2.5 测试专用依赖走 `[dev-dependencies]`

`tempfile`、`assert_cmd`、`mockall`、`serial_test`、`proptest`、`criterion` 一律放 `[dev-dependencies]`。它们不会进入下游的依赖图。

**注意**：`[dependencies]` 里的东西**对测试自动可见**，所以在两处都写是冗余的，且会在下次升级时静默不同步。

**并行测试共享状态**：用 `serial_test` 而不是 `--test-threads=1`——后者会拖慢整个套件来迁就少数几个测试。

## 2.6 每个 `.rs` 都必须能被 `mod` 图到达 ★ 本宝典的原创规则

Rust Book 警告说「`src/tests/` 目录会被静默忽略」。**这只是更普遍规律的一个特例**：

> **rustc 只编译从 crate 根经 `mod` 声明可达的文件。任何其他 `.rs` 文件，编译器根本不会打开。**

不报错、不警告、不提示。它在 `git` 里、在你的编辑器里、在 code review 里看起来完全正常——**但它不存在**。

**实测后果**［实测］：某仓库有 **24 个文件、7,801 行 Rust** 处于这种状态，其中包含 **85 个测试函数**。它们出现在 `grep` 统计的「2,505 个测试」里，实际执行数为 **0**。

最危险的两个：

```
api_key_client_never_sends_a_bearer_token
oauth_client_never_sends_seller_api_key_headers
```

**有人以为这两条凭据隔离保证被测试守着。它们一次都没跑过。**

**精确对账方法**（任何仓库都能跑）：

```bash
grep -rn --include='*.rs' -E '#\[(tokio::)?test\]' src | wc -l   # 源码里写了多少
cargo test --lib -- --list | grep -c ': test$'                    # 实际存在多少
# 两者之差 = 从未编译的测试数
```

**这条规则必须做成门禁**，因为它是**唯一一类「代码审查绝对看不出来」的缺陷**：

```rust
/// 每个成员 src/ 下的 .rs 都必须能从 mod 声明、#[path] 或 cargo target 到达。
/// 不可达文件不会被编译、不会被 lint、不会被测试——它们读起来像已交付的代码，但不是。
fn check_no_orphan_modules(root: &Path, violations: &mut Vec<String>) { ... }
```

## 2.7 测试辅助代码不得进入生产库的公共 API

**问题的根源是真实的**：`tests/` 把库当外部依赖链接，所以 `#[cfg(test)]` 模块**对集成测试不可见**。于是人们把测试辅助改成 `pub mod test_support;`。

**代价**［实测］：某仓库两个这样的模块（合计 481 行）被编译进生产库——其中一个所在的 crate 是桌面应用的**普通依赖**，另一个所在的 crate **连 `[features]` 段都没有**，无从关闭。

**正确解法，按优先级**：

1. **`tests/common/mod.rs`**——辅助代码留在测试树里，对库的公共 API 完全隐形。**这正是 2.3 那条规则存在的理由。**
2. 确实需要库内部能力时，用 **单一 `test-util` feature 门控**［共识：M-TEST-UTIL］，且不给任何消费者启用：
   ```toml
   [features]
   test-util = []
   ```
   ```rust
   #[cfg(feature = "test-util")]
   pub fn bypass_certificate_checks() { ... }
   ```

## 2.8 集成测试二进制的数量本身是成本

**每个 `tests/*.rs` 都是一个独立的 crate**，各自编译并链接整个被测库。

**实测**［实测］：某仓库 `cargo test --no-run --workspace` 需要 **441 秒**，产出 **69 个测试二进制**；而其中 1,334 个单元测试的**实际运行时间是 0.11 秒**。

> **编译耗时是运行耗时的约 4,000 倍。**
>
> 这意味着：**任何「让测试跑得更快」的努力都方向错了。** 杠杆在构建图和二进制数量上。

**所以拆分巨型测试文件时，要拆成模块而不是二进制**：

```
tests/
└── cloud/
    ├── main.rs            # 唯一的集成测试二进制
    ├── common/mod.rs      # 共享辅助
    ├── auth_sessions.rs   # mod，不是独立 crate
    └── entitlements.rs
```

把一个 7,000 行文件拆成 24 个顶层 `tests/*.rs`，会新增 ~21 次针对最重依赖集的链接。拆成 24 个 `mod` 则**零链接成本**，且同样满足「按功能命名」。

Cargo 自身做过同样的合并：测试套件编译时间降 **3×**，磁盘产物降 **5×**［共识：matklad］。

**内部 crate 更进一步**：能用 `#[cfg(test)]` 单元测试讲清楚的，就不要再开集成测试二进制。集成测试的价值是「逼你走公共 API」——内部 crate 没有外部消费者，这份税交得不值。

**已发布的库 crate 反过来**：优先一个集成测试 crate（常叫 `it`），少写单元测试，让测试本身成为公共 API 的设计反馈。

## 2.9 依赖外部服务的测试：要么 fail-loud，要么 `#[ignore]`，**绝不静默跳过**

**反模式**：

```rust
#[test]
fn my_db_test() {
    let Ok(url) = std::env::var("DATABASE_URL") else { return };  // ❌ 静默通过
    ...
}
```

CI 上环境变量没配 → 测试「通过」→ 没人发现覆盖率是假的。

**两种可接受形态**：

```rust
// (a) fail-loud：缺依赖就报错，并告诉人怎么修
let db = EphemeralDatabase::acquire()
    .expect("PostgreSQL 未就绪，请运行 `cargo xtask db up`");

// (b) 显式忽略：cargo test -- --ignored 才跑
#[test]
#[ignore = "需要本地 PostgreSQL"]
fn my_db_test() { ... }
```

**（a）更好**，因为它让「我以为跑了」变成「我知道没跑」。

## 2.10 内部 crate 关闭 doctest ［共识：matklad］

**每个文档测试都是一个独立二进制**，要单独链接一次。内部 crate 没有「示例即文档」的外部读者，却按 crate 数缴纳这份税。

```toml
# 不发布的 crate
[lib]
doctest = false
```

已发布、公共 API 稳定的 crate 保留 doctest——那是契约的一部分。内部 crate 的「2 个可执行 doc test 对 1,115 个单元测试」（见 2.1）不是覆盖不足，是**不该付的编译税**。

## 2.11 测试不得复述源码里的字面量 ［共识：M-TAUTOLOGICAL-TESTS］

```rust
const CHECKPOINTS: [u32; 4] = [0, 90, 180, 270];

#[test]
fn checkpoints_are_correct() {
    assert_eq!(CHECKPOINTS, [0, 90, 180, 270]);  // ❌ 通过是因为抄了同一处
}
```

这种测试通过是构造出来的。它增加噪声、在后续改动里制造假安全感，对行为零断言。

有意义的测法是测**不在源码里写死的性质**：间距相等、单调递增、越界被拒、与另一份权威数据一致。

AI 助手特别容易写出这种测试——它会把实现抄进断言。审查时看到「期望值与被测常量逐字相同」，直接删。

## 第二部的门禁

```rust
check_no_orphan_modules()       // 2.6 —— 最高优先级
check_no_test_code_in_lib()     // 2.7 —— 禁止非 feature 门控的 pub fn/mod test*
check_tests_common_shape()      // 2.3 —— 禁止 tests/common.rs，禁止 src/tests/
check_no_silent_test_skip()     // 2.9 —— 禁止测试体内因缺环境变量而 return
check_internal_doctest_off()    // 2.10 —— 未发布 crate 必须 doctest = false
```

---

# 第三部 · 构建速度

## 3.1 先量化，再优化 ［共识：matklad Fast Builds + corrode］

```bash
cargo build --workspace --timings     # 产出 HTML 报告：每个单元的耗时与并行度
cargo check --workspace --timings     # check 与 build 的瓶颈往往不同
cargo tree --duplicates               # 被编译多次的 crate
cargo tree -i <crate>@<ver>           # 反向追溯：谁把这个版本拖进来的
cargo tree -e features -p <crate>     # feature 实际展开成了什么
cargo llvm-lines | head               # 泛型单态化：哪份函数被实例化了多少次
RUSTFLAGS="-Zmacro-stats" cargo +nightly build   # 哪个 proc-macro 生成了多少代码
```

`--timings` 图里看三件事：绿条（正在编）够不够宽、红条（等 CPU）是不是堆在同一个 crate 后面、蓝条（等依赖）是不是一条长链。红/蓝都指向 1.11 的 DAG 形状，不是 profile。

**没有 `--timings` 就动 profile，等于闭着眼睛调参。**

## 3.2 `opt-level` 必须精确到包，**永远不要用 `"*"`** ★ 本宝典最重要的构建规则

**诱人的反模式**：

```toml
[profile.dev.package."*"]
opt-level = 3          # 「让依赖都跑快点」
```

**`"*"` 匹配每一个非 workspace 包**，在 Cargo 里这包括你根本没想到的东西：

- **所有 proc-macro crate**——它们是编译期运行的宿主程序，**不产出任何交付代码**；
- **所有构建脚本及其依赖**——`build.rs` 里的 `cc` 会继承 `OPT_LEVEL`，把捆绑的 C 源码按 `-O3` 编译；
- 整个服务端 / TLS / ORM / GUI 栈——与你想加速的计算路径毫无关系。

**实测归因**［实测］，某仓库依赖构建 CPU 的分布：

| 桶 | 占比 |
| --- | --- |
| proc-macro + 构建依赖宿主工具 | **38.3%** |
| 重复版本 crate | 10.6% |
| **真正需要 `opt-level = 3` 的计算集** | **5.0%** |

**为了让 5% 变快，给 38% 的代码生成工具链交了税。**

**正确做法**：枚举真正在热路径上的包。

```toml
[profile.dev]
opt-level = 0

# 只给计算热路径开优化——从 `cargo tree -p <你的计算 crate>` 推导出这个清单
[profile.dev.package.image]      { opt-level = 3 }
[profile.dev.package.flate2]     { opt-level = 3 }
[profile.dev.package.rayon]      { opt-level = 3 }
# ... 通常 20–30 个包，而不是 600 个
```

## 3.3 `build-override` 与 `package` 覆盖是**一对**，必须同时设 ★★ ［推翻·实测］

**这是本宝典最反直觉的一条，也是最容易踩的坑。**

按 3.2 删掉 `"*"` 之后，你会**顺带把所有 proc-macro 也降到 `opt-level = 0`**——因为它们本来就是靠 `"*"` 才被优化的。

**实测结果**［实测］：

| 配置 | 冷 `cargo check --workspace` |
| --- | --- |
| 现状（`"*" = 3`） | 254.9 s |
| **只删通配符** | **269.2 s ← 更慢了** |
| 删通配符 **+ `build-override`** | **75.8 s** |

**只做一半的「优化」是净倒退。**

原因：derive 宏的**执行速度**在大型仓库里是一等成本。某仓库有 **1,771 个 `Serialize` + 1,916 个 `Deserialize` + 403 个 `TS`** derive 要展开——未优化的 `syn` 慢，慢在每一次编译。

**正确写法（两半必须在同一个 commit 里）**：

```toml
[profile.dev]
opt-level = 0

# ★ 编译期运行的东西保持优化：proc macro、build script 及其依赖
[profile.dev.build-override]
opt-level = 3
debug = false

# 运行期热路径按包点名
[profile.dev.package.image]
opt-level = 3
```

**记忆口诀**：`build-override` 管**编译时跑的代码**，`package.<name>` 管**运行时跑的代码**。两者是正交的，缺一不可。

## 3.4 冷构建与热增量是两个不同的问题，不要混为一谈 ★ ［实测］

**这是最容易误导人的地方。** 同一个 profile 改动：

| 场景 | 效果［实测］ |
| --- | --- |
| **冷** `cargo build --workspace` | 278.9 s → **159.0 s** |
| **冷** `cargo check --workspace` | 277.9 s → **75.8 s（3.7×）** |
| **热**增量（改一个文件重编） | 19–26 s → 19–26 s，**无差异** |

**所以先问：你的痛点到底是哪个？**

| 痛点描述 | 真正的杠杆 |
| --- | --- |
| 「新克隆 / 切分支 / CI / Docker 构建要好几分钟」 | **profile**（3.2 + 3.3），一次清单编辑，2–4 倍 |
| 「**保存一次要等 25 秒**」 | **拆 crate**（3.9）。profile 对此**完全无效** |

热增量的耗时由**你正在编辑的那个 crate 有多大**决定。一个 80,476 行的 crate，无论 `opt-level` 设什么，改一行都要重编它自己。

## 3.5 链接器建议必须按平台实测，不要照抄 ★ ［推翻·实测］

流行建议是「Linux 用 mold、macOS 用 lld」。**在 macOS 上，这条建议今天是错的。**

实测（M4 Pro / macOS 26 / Apple `ld` PROJECT ld-1267，同一条链接命令各跑 3 次）：

| 二进制 | Apple ld-prime（默认） | `ld_classic` | Rust 自带 `ld64.lld` |
| --- | ---: | ---: | ---: |
| 34 MB 桌面应用 | **0.298 s** | 0.523 s | 0.495 s |
| 36 MB 服务端 | **0.178 s** | 0.366 s | 0.310 s |

- **默认最快**；`ld64.lld` 慢 1.7 倍。
- **`mold` 根本不支持 Mach-O**（Mach-O 支持在已停止维护的 `sold` 里）。
- 链接占约 25 秒增量重建的 0.2–0.3 秒——**约 1%**，天花板本来就很低。

**Apple 在 Xcode 15 引入的新链接器（ld-prime）已经很快了**，早年「macOS 链接慢」的经验已经过期。

同理，2021 年「`split-debuginfo = "unpacked"` 让 macOS 增量快 70%」这条也过期了：对 `debug` 非零且走增量的 `dev` profile，**`unpacked` 早已是 macOS 默认值**。写进清单不会坏，但也不会再给你那 70%。

**规则**：链接器 / debuginfo 优化**在 Linux 上通常有效，在现代 macOS 上通常无效**。无论哪种情况，**测了再改**，并把测量结果写进 `.cargo/config.toml` 的注释里，避免后人反复争论。

## 3.6 feature 裁剪是最被低估的杠杆 ［共识：corrode + M-FEATURES-ADDITIVE］

```toml
# ❌ 把整个 tokio 拖进来
tokio = { version = "1", features = ["full"] }

# ✅ 只要用得到的
tokio = { version = "1", default-features = false, features = ["rt-multi-thread", "macros", "time"] }
```

关掉 `tokio` 的默认 features 实测可省约 23% 编译时间（corrode 引用的测量）。

**必查项**：

- `default-features = false` + 按需列举，对所有重型依赖（`tokio`、`reqwest`、`sea-orm`、`image`）。
- **库 crate 的默认 feature 会在 workspace 构建里生效**。若你的库 `default = ["native-tls"]` 而所有消费者都显式选 `rustls-tls`，那么 `cargo build --workspace` 会把**两套 TLS 栈都编译一遍**［实测］。
- 用 `cargo tree -e features` 验证展开结果，不要靠推测。
- `cargo machete` / `cargo udeps` 找完全没用到的依赖。
- **同一依赖在不同成员上启用不同 feature，会被编多次**。这是 `[workspace.dependencies]` 管不了的——它只统一版本，不统一 feature 展开。`cargo-hakari`（workspace-hack crate）专门收这个问题；**先用 `cargo tree -e features` 确认真有分裂，再引入**，不要预防性加一层。
- **共享类型 crate 不要无条件 `derive(Serialize, Deserialize)`**。serde 及其宏会成为入度枢纽上的编译瓶颈：每个只用到类型、不做序列化的下游都得等它。把 derive 收进可选 feature，只在真正编解码的叶子 crate 打开［共识：corrode］。

## 3.7 `optional` 依赖 ≠ `cfg` 门控 ★ 常见误解

```rust
#[cfg(debug_assertions)]
mod pg_backend;              // 源码被门控了
```

```toml
postgres = "0.19"            # ❌ 但依赖没有！release 构建照样编译整个 postgres
```

**`cfg` 属性拦不住 Cargo 编译一个已声明的依赖。** 上例中 release 二进制会链接一个永远到不了的 PostgreSQL 客户端［实测］。

**正确写法**：

```toml
[features]
dev-backend = ["dep:postgres"]

[dependencies]
postgres = { version = "0.19", optional = true }
```

```rust
#[cfg(feature = "dev-backend")]
mod pg_backend;
```

## 3.8 不要 `cargo clean` ［共识：matklad Fast Builds + corrode］

`cargo clean` 摧毁增量缓存，下一次构建从零开始。

**但 `target/` 确实会失控**［实测］：某仓库的 `target/` 达到 **239 GB / 608,180 个文件**，其中 `incremental/` 108 GB。这不只是磁盘问题——实测一次后台文件系统回收让一个 137 秒的构建膨胀到 280 秒。

**正确做法**：

```bash
cargo install cargo-sweep
cargo sweep --time 15        # 清理 15 天未使用的产物，保留活跃缓存
```

## 3.9 crate 是重编译的边界 ［共识：M-SMALLER-CRATES + matklad Fast Builds］

**改动一个 crate 里的任何一行，整个 crate 重编。** 这是 crate 拆分最实际的理由。微软的口令是「拿不准就拆」：一个子模块能独立使用，就应该是独立 crate。

**经验值**：单 crate 控制在 **5,000 行**以内（corrode / rustprojectprimer）。

**推论：常改的代码与稳定的代码要分开。** 把每天都动的 `config` 和几乎不动的 `core` 放同一个 crate，等于让 `core` 每天重编一次。

拆完立刻看 1.11：拆成宽 DAG 才赚并行；拆成更深的链，冷构建更慢。**这也是 3.4 里「热增量唯一的杠杆」**——但它是多天的结构工作，不是一次清单编辑。**排期时要诚实对待这个成本差异。**

## 3.10 `sccache` 有适用条件，不是无脑装 ［共识 + 实测判断］

`sccache` 与 `CARGO_INCREMENTAL=1` **不兼容**。在单台开发机上，它通常**打不过** cargo 自己的增量缓存。

**它真正有价值的场景**：CI（每次都是冷缓存）、或多台机器共享缓存。

**没有 CI 时，装它是负收益。** 先建 CI，再谈 sccache。

## 3.11 profile 模板（可直接抄）

```toml
[profile.dev]
opt-level = 0
debug = 1                # 行号表：够用于 backtrace，比完整 DWARF 生成/链接都快
incremental = true
codegen-units = 256      # 保持重编译的爆炸半径小

# ★ 编译期运行的代码保持优化（见 3.3，与下面的 package 覆盖是一对）
[profile.dev.build-override]
opt-level = 3
debug = false

# 运行期计算热路径（见 3.2，按 cargo tree 推导，通常 20–30 个）
[profile.dev.package.image]
opt-level = 3

# 迭代非计算代码时用：cargo build --profile fast
[profile.fast]
inherits = "dev"
debug = 0

[profile.release]
opt-level = 3
lto = "thin"             # release 用 thin LTO；dev 绝不开 LTO
codegen-units = 1
strip = "symbols"
incremental = false

# 打包冒烟用，不必等 1-CGU
[profile.release-fast]
inherits = "release"
lto = false
codegen-units = 16
```

> `panic = "abort"` 能进一步减小体积，但**会改变运行时语义**——任何依赖 `catch_unwind` 的代码都会失效，且测试需要 unwind。**默认不要加。**

## 3.12 CI 和本地是第三种不同的问题 ［共识：matklad Fast Builds］

3.4 把冷构建和热增量拆开了。CI 是第三种：每次都接近冷构建，但还要缓存、还要出可比较的数字。

**CI 上该关的东西，本地不该关**：

```toml
# 只给 CI 用的 profile，或在 CI 脚本里用环境变量覆盖
[profile.ci]
inherits = "dev"
incremental = false    # 冷构建上增量是纯开销，还把 target/ 撑大、把缓存打废
debug = 0              # debuginfo 是 target/ 膨胀的主因；要行号用 "line-tables-only"
```

```bash
# CI 脚本
export CARGO_INCREMENTAL=0
export RUSTFLAGS="-D warnings"   # 见 5.3：只在 CI 拒警告，绝不写 #![deny(warnings)]
cargo test --workspace --no-run  # 先看编译耗时
cargo test --workspace           # 再看运行耗时
```

**不要把整个 `target/` 丢进 CI 缓存。** 缓存该留的是很少变的依赖，不是每次 PR 都变的本仓库 crate。缓存整棵 `target/` 又大又脏，命中率差，还拖慢缓存读写。用 `Swatinem/rust-cache` 或自己按「依赖 / 本仓库」切开。

**CI 是标准化基准。** 本地增量随改动类型乱跳，不能用来判断「仓库是不是越来越慢」。每次 PR 的 CI 墙钟时间才是可比较的时间序列。

## 3.13 日常习惯

| 习惯 | 收益 |
| --- | --- |
| 用 `cargo check` 而非 `cargo build` 迭代 | 跳过 codegen，通常快数倍 |
| `cargo-nextest` 跑测试 | 进程级并行 + 每测试计时；也修掉 Cargo 串行跑多个测试二进制的问题（见 2.8） |
| `[workspace] default-members` 排除探针/示例 crate | 裸 `cargo build` 不再编译它们［实测：一个 50 行的探针 crate 可以拖进整整一套 HTTP 栈］ |
| Docker 构建用 `cargo-chef` 或 BuildKit cache mount | 否则每次镜像构建从零重编所有依赖 |
| `target/` 放内置 SSD | 外置卷实测写入带宽约为内置的一半 |
| macOS：把终端加入 Developer Tools | 免去 Gatekeeper 对每个新编出来的二进制做一次公证检查（corrode / Zed） |

## 第三部的门禁

```rust
check_no_wildcard_opt_level()   // 3.2 —— 禁止 [profile.*.package."*"]
check_build_override_paired()   // 3.3 —— 有 package 覆盖就必须有 build-override
check_no_default_features()     // 3.6 —— 重型依赖必须显式 default-features = false
```

---

# 第四部 · 依赖与版本治理

## 4.1 `[workspace.dependencies]` 统一版本 ［共识：M-CARGO-WORKSPACE］

```toml
# 根 Cargo.toml —— 只钉版本和 default-features，不在这里开业务 feature
# ［共识：M-CARGO-WORKSPACE］workspace 表开 feature 会让所有成员被动继承
[workspace.dependencies]
serde = { version = "1.0.229", default-features = false }
tokio = { version = "1.47.1", default-features = false }
```

```toml
# 成员 —— 谁用谁开
[dependencies]
serde = { workspace = true, features = ["derive"] }
tokio = { workspace = true, features = ["rt-multi-thread", "macros"] }
```

**收益的诚实说明** ★［实测］：

流行说法是「统一版本能大幅提速，因为每个重复版本都要多编一次」。**这个说法需要归因验证。**

某仓库实测：`Cargo.lock` 有 788 个包版本，**30 个 crate 被编译 2–4 次**。但逐个用 `cargo tree -i` 反查后发现，**绝大多数重复是上游传递依赖强制的**（例如 `syn 2` 和 `syn 3` 同时存在，因为不同的宏库各自依赖）——**你无法通过统一自己的清单来消除它们**。

真正由自己造成、可消除的重复只有少数几个。

**所以 `[workspace.dependencies]` 的首要价值是治理，不是速度**：

- 它把「23 个 crate 在 10 处各自声明、19 处写法不一致」变成一处；
- 它阻止**下一次** `cargo update` 引入新的分裂。

**照实说，不要夸大。** 夸大的收益承诺会在没兑现时损害整个方案的可信度。

## 4.2 `[workspace.lints]` 统一 lint ［共识：M-CARGO-WORKSPACE］

```toml
# 根
[workspace.lints.rust]
unsafe_code = "deny"
unused_must_use = "deny"
unreachable_pub = "warn"       # 对应 1.7 的「pub 收敛」

[workspace.lints.clippy]
todo = "deny"
unimplemented = "deny"
dbg_macro = "deny"
# 其余按 5.3 棘轮：今天违规数为 0 的才能 deny
```

```toml
# ★ 每个成员必须显式启用，否则整张表无效
[lints]
workspace = true
```

**采纳策略见 5.3——直接上会被回滚。**

## 4.3 `=` 精确版本 vs `^` 兼容版本

在有 `[workspace.dependencies]` 的前提下，**`=` 通常是多余的**：workspace 表已经给了单点控制，再叠一层 `=` 只会让「某成员用 `=1.2.3`、另一成员用 `^1.2.3`」这种分裂更隐蔽。

**真正需要 `=` 的场景**：该依赖的 patch 版本曾经引入过破坏（如 `tauri` 这类与构建产物强绑定的框架）。**这时要写注释说明原因**，否则后人不敢动也不知道为什么。

## 4.4 MSRV、edition、resolver

```toml
[workspace.package]
edition = "2024"          # 新 crate 一律用最新 edition
rust-version = "1.97"     # 库 crate 必须声明

[workspace]
resolver = "3"            # MSRV 感知；虚拟清单不会自动推断，必须显式写
```

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.97.1"
profile = "minimal"
components = ["clippy", "rustfmt"]   # ★ minimal 不含它们！
```

★ **`profile = "minimal"` 不安装 clippy 和 rustfmt**［实测坑］。你本机有，只是因为 `stable` 工具链带了。**新克隆的贡献者今天加的 lint 门禁会静默不运行。** 加门禁时必须同一次改动补上 `components`。

## 4.5 feature 必须可叠加 ［共识：M-FEATURES-ADDITIVE］

库 crate 的任意 feature 组合都必须能编过。这不是风格，是 Cargo 的特征统一（feature unification）：workspace 里只要有一个成员开了 `foo`，其他成员也会看到 `foo`。

推论：

- 不要做 `no-std` feature，做 `std` feature。
- 打开一个 feature 不得删除或改语义已有的公共项。
- feature 不得要求调用方再手动打开另一个 feature。
- 互斥 feature（`native-tls` / `rustls-tls`）是已知的痛，必须在文档里写清楚，并用 `cargo hack --feature-powerset` 在每夜任务里验。

3.6 里「库的默认 feature 会在 workspace 构建里生效」就是这条规则的实测特例。

## 4.6 读 `Cargo.lock`，不要只读 `Cargo.toml` ［共识：matklad Fast Builds］

`Cargo.toml` 写的是你以为自己依赖了什么。`Cargo.lock` 写的是二进制里真正有什么。

定期打开 lockfile，对每一个**直接或传递**依赖问：它对坐在产品前面的人解决了什么问题？rust-analyzer 曾经因为日志库的过滤表达式拖进整份 `regex`——他们有精确的 Rust / Markdown 解析器，运行时根本不需要正则。

同一份 lockfile 也能抓住「隐式全局 + 重复版本」这种运行时幽灵：IC 的两个包分别链了 `prometheus 0.9` 和 `0.10`，二进制里出现两份全局 registry，一半指标永远看不见［共识：Kashitsyn］。显式传递 logger / metrics / runtime，这种 bug 会变成编译错误。

改 feature、给上游发 PR、或换一个更瘦的库，通常就够了。**先读 lockfile，再谈「我们依赖不多」。**

---

# 第五部 · 门禁与自动化

## 5.1 所有检查类自动化收进 `cargo xtask` ［共识：matklad］

```toml
# .cargo/config.toml
[alias]
xtask = "run -p xtask --"
```

**为什么不用 Makefile/脚本堆**：xtask 是 Rust 代码——跨平台、能被类型检查、能被测试、能复用 `toml`/`serde` 等库解析清单。

**但要分清边界**：**进程监管、数据库集群生命周期、SSH/Docker 传输、代码签名与打包，这些天然是 shell 形状的**，用 Rust 重写只会更糟。

**判据**：**「检查」进 xtask，「编排」留给 shell。** 而且 shell 应当**调用** `cargo xtask ci`，而不是自己重新实现检查。

## 5.2 门禁阶梯：按「每单位信心的成本」排序

| 层 | 时间预算 | 内容 |
| --- | --- | --- |
| **pre-commit** | ≤ 6 s | `fmt --check`、生成物 `--check`、版本一致性、架构门禁 |
| **pre-push** | ≤ 3 min | 上层全部 + `clippy`（只管 deny 集）+ `test` + 前端检查 |
| **发布门禁** | 不限 | 上层全部 + 端到端契约 + 备份恢复演练 |
| **每夜** | 非阻塞 | lint 存量趋势、`tree --duplicates`、`audit`、`--timings`、`cargo hack --feature-powerset` |

**关键洞察**［实测］：某仓库的 4 条既有检查**合计 5.2 秒、全部通过、且无人调用**——`cargo xtask architecture` 在全仓库的出现位置只有 `.md` 文档和它自己的 usage 字符串。

> **最高性价比的工程改进，往往不是写新检查，而是把已经写好、已经通过的检查接上变更路径。**

## 5.3 lint 采纳用棘轮，不用大爆炸 ★

**一张会让几百处报错的 lint 表，第一天就会被回滚。**

**正确顺序**：

1. **落表**：所有 `deny` 项选**今天违规数为 0** 的规则；真实存量一律先设 `warn` 并**在注释里记录基数**。
2. `cargo clippy --fix` 机械清除（通常能清掉 30–40%）。
3. 找**根因**：某仓库 499 处告警中，`result_large_err` 一条占 163 处——**这不是 163 个缺陷，而是一个设计事实**（某个被广泛返回的 `Result` 其 `Err` 变体过大）。在根上装箱一次，163 处一起消失。
4. **清一条，就在同一个 commit 里把它从 `warn` 翻成 `deny`。**

**永远不要在源码里写 `#![deny(warnings)]`。** 它会在 rustc 升级时把新 lint 变成全仓库红灯，也会让下游用户的编译无故失败［共识：matklad］。

CI 上用环境变量即可（见 3.12）：

```bash
RUSTFLAGS="-D warnings" cargo test --workspace
```

本地保持 warn，逐条棘轮翻成 `deny`。门禁才不会静默倒退。

## 5.4 没有 CI 时，git hooks 是唯一的自动化机制

有些团队有**正当理由**不用托管 CI（成本、合规、构建产物本来就在本地）。**这个决定可以尊重**，但必须回答：那么检查在哪里跑？

```bash
# .git/hooks/pre-commit —— 由 `cargo xtask hooks --install` 写入
#!/bin/sh
exec cargo xtask ci --tier 1
```

**没有 CI 又没有 hooks = 没有任何东西是自动的**，一切依赖人记得。而人一定会忘——序言里那个被顺带删掉的发布门禁就是这么消失的。

## 5.5 规则必须可机器检查，否则会腐烂 ★ 本部最重要的一条

**实测**［实测］：某仓库文档里有 30 条明文规则，逐条核对后——

| 状态 | 条数 |
| --- | --- |
| 完全有自动检查 | **8** |
| 部分检查 | 6 |
| **仅存在于散文里** | **16** |

**且 7 处文档自相矛盾**，包括：两份 ADR 引用一个已被删除的脚本；同一个 README 相隔 99 行自我否定；两份 ADR 记载了一个**不存在的子命令**。

**规律**：**有门禁的规则一直成立，没门禁的规则全部漂移。** 这不是团队素质问题，是机制问题。

**推论**：写规则时就要问「这条怎么检查」。**答不上来的规则，要么改写成可检查的形式，要么承认它只是建议。**

## 5.6 文档 SSOT：一个事实恰有一处

每条事实只有一个「文档之家」：

- 目录树 / 所有权 / 依赖方向 → 布局文档
- 长期架构选择 → ADR
- 命令 / 构建 / 部署 → README
- 大 bug 的根因 → 错误日志文档

**索引文档应当链接，而不是复述。** 复述出来的那份，就是半年后与代码不一致的那份。

**可门禁化的部分**：

```rust
check_layout_sync()   // 布局文档里列出的路径必须都存在；每个 workspace 成员必须被列出
check_docs_home()     // 按改动区域要求对应文档在同一提交范围内被更新
```

---

# 附录 A · 速查表

## 新项目开局清单

```
□ 虚拟根清单 + resolver = "3"
□ [workspace.package] edition / rust-version / version = "0.0.0"
□ [workspace.dependencies] —— 从第一个共享依赖开始就用
□ [workspace.lints] + 每个成员 [lints] workspace = true
□ rust-toolchain.toml 含 components = ["clippy", "rustfmt"]
□ crates/ 平铺，目录名 = crate 名
□ 薄 main + 厚 lib
□ tools/xtask crate + .cargo/config.toml 的 alias
□ [profile.dev] opt-level=0 + build-override opt-level=3
□ tests/common/mod.rs（第一个集成测试就建）
□ 内部 crate `[lib] doctest = false`
□ benches/（如果性能是需求）
□ git hooks 或 CI —— 第一天就接上；CI 关 incremental、关 debuginfo
```

## 定期体检命令

```bash
cargo build --workspace --timings      # 构建瓶颈
cargo tree --duplicates                # 重复版本
cargo tree -e features -p <crate>      # feature 实际展开
cargo machete                          # 无用依赖
cargo clippy --workspace --all-targets # lint 存量（趋势只许降）
cargo test --workspace -- --list | wc -l   # 与 grep 计数对账（见 2.6）
cargo hack --feature-powerset -p <lib> # feature 组合是否都能编（见 4.5）
du -sh target/                         # 超过 ~50 GB 就 cargo sweep
```

# 附录 B · 反模式清单

| # | 反模式 | 后果 | 规则 |
| --- | --- | --- | --- |
| 1 | `[profile.dev.package."*"] opt-level = 3` | 为 5% 的收益给 38% 的工具链交税 | 3.2 |
| 2 | 删通配符但不加 `build-override` | **比不改还慢** | 3.3 |
| 3 | 照抄「macOS 换 lld」 | **性能倒退**；mold 根本不支持 Mach-O | 3.5 |
| 4 | 文件不在 `mod` 图里 | 代码与测试**静默失效**，review 看不出来 | 2.6 |
| 5 | `pub mod test_support;` | 测试代码进生产库依赖图 | 2.7 |
| 6 | `tests/common.rs` | 被当成空测试二进制 | 2.3 |
| 7 | 测试因缺环境变量 `return` | 覆盖率是假的 | 2.9 |
| 8 | 把 7,000 行测试拆成 24 个顶层 `tests/*.rs` | 新增 ~21 次重量级链接 | 2.8 |
| 9 | `cfg(debug_assertions)` 门控源码但依赖非 optional | release 仍编译并链接 | 3.7 |
| 10 | 按技术层切模块 | 删一个功能要动 6 个目录 | 1.6 |
| 11 | 持久化层 import 业务域 | crate 永远拆不出来 | 1.8 |
| 12 | 同一概念声明多套 + `From<>` 互转 | 转换代码成为永久税 | 1.9 |
| 13 | 测试文件按冲刺号/工单号命名 | 摧毁「测试即文档」 | 2.4 |
| 14 | `cargo clean` 清空间 | 摧毁增量缓存 | 3.8 |
| 15 | 没有 CI 的仓库装 sccache | 负收益 | 3.10 |
| 16 | 源码里 `#![deny(warnings)]` 或一次性上 `-D warnings` | rustc 升级即全红；或第一天被回滚 | 5.3 |
| 17 | 写了门禁但没有任何入口调用 | 等于没写 | 5.2 |
| 18 | `profile = "minimal"` 却依赖 clippy 门禁 | 新贡献者处静默不运行 | 4.4 |
| 19 | 成员间手写 `path = "../sibling"` | 依赖图有两套写法，门禁漏检 | 1.12 |
| 20 | 抽出一个什么都依赖的 `types` crate | 环没了，全仓库重编还在 | 1.11 |
| 21 | 内部 crate 保留 doctest | 每个示例单独链接一次，无人读 | 2.10 |
| 22 | 测试断言抄被测常量 | 通过是构造出来的 | 2.11 |
| 23 | CI 缓存整个 `target/`、开着 incremental | 缓存又脏又大，冷构建更慢 | 3.12 |
| 24 | workspace 表里给 serde 开 `derive` | 所有成员被迫编 serde 宏 | 4.1 |
| 25 | 照抄「macOS 设 split-debuginfo = unpacked」 | 早已是默认值，零收益 | 3.5 |

# 附录 C · 证据来源

**规范与长文**（按本文引用密度）：

| 来源 | 本文用到的部分 |
| --- | --- |
| 微软 [Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/)（2026.6） | M-CARGO-WORKSPACE、M-CRATES-FLAT-FOLDER、M-CRATES-IN-WORKSPACE、M-LATEST-EDITION、M-MSRV、M-SMALLER-CRATES、M-BALANCED-MODULES、M-INTEGRATION-TESTS、M-TEST-UTIL、M-FEATURES-ADDITIVE、M-TAUTOLOGICAL-TESTS、M-STATIC-VERIFICATION |
| matklad [Large Rust Workspaces](https://matklad.github.io/2021/08/22/large-rust-workspaces.html) | 虚拟根清单、平铺、`0.0.0`、xtask |
| matklad [Fast Rust Builds](https://matklad.github.io/2021/09/04/fast-rust-builds.html) | `--timings`、宽 DAG、读 lockfile、CI 关 incremental / debuginfo、不要缓存整棵 `target/`、不要 `#![deny(warnings)]` |
| matklad [Delete Cargo Integration Tests](https://matklad.github.io/2021/02/27/delete-cargo-integration-tests.html) | 合并集成测试二进制、内部 crate 关 doctest、测试放到独立文件以免重编库 |
| Roman Kashitsyn [Rust at scale](https://mmapped.blog/posts/03-rust-packages-crates-modules)（IC，~35 万行） | 依赖枢纽、关联类型破环、运行时多态接线、显式传递 logger/metrics |
| corrode [Tips For Faster Rust Compile Times](https://corrode.dev/blog/tips-for-faster-rust-compile-times/)（2026-03 更新） | feature 裁剪、`cargo llvm-lines`、`-Zmacro-stats`、serde 下沉到叶子、Gatekeeper、`cargo-hakari` |
| [Rust Project Primer](https://rustprojectprimer.com/organization/index.html) | 组织与耦合、指向以上长文的阅读路径 |
| Rust Book ch11 / Cargo Book | `tests/common/mod.rs`、集成测试即独立 crate、profile 语义 |
| Leapcell *Mastering Large Project Organization in Rust* | workspace 共享 lock / `target/`；resolver 仍写 `"2"`，已被本文 1.1 覆盖 |

**本文明确不收、或收了但降级为「先测」的流行建议**：

- 「macOS 换 lld / mold」——3.5 实测推翻；mold 不支持 Mach-O。
- 「只删 `opt-level = "*"`」——3.3 实测比不改更慢。
- 「`[workspace.dependencies]` 能大幅提速」——4.1 实测主要是治理，不是速度。
- 「`split-debuginfo = "unpacked"` 让 macOS 快 70%」——已是默认值。
- 微软 M-LATEST-EDITION「一般不必写 resolver」——对虚拟清单不成立。
- cranelift / watt / dylib 包装依赖——实验性，未在本仓库测量，不写进规则。

**实测来源**：一个 233,238 行 Rust + 53,100 行 TypeScript 的真实生产仓库（10 个 workspace 成员，edition 2024，rustc 1.97.1），在 Apple M4 Pro / 12 核 / 24 GB 上测量。测量方法与可靠性约束：

- 冷构建全部使用全新的 `CARGO_TARGET_DIR`，`/usr/bin/time -p` 计时；
- 会话内绝对耗时存在漂移（一次后台文件系统回收让同一构建从 137 s 膨胀到 280 s），**因此所有比值均取背靠背 A/B 配对，不跨时段比较绝对值**；
- 链接器对比通过捕获 rustc 真实链接命令后原地重跑，每种链接器 3 次；
- 孤儿模块结论用两种独立方法交叉验证（rustc dep-info 差分 vs `mod` 可达性扫描），并与仓库自有的一份人工审计逐字吻合。

**本宝典中标注［推翻］的三条**，是实测结果与流行建议相反的地方：换链接器（3.5）、只删 `opt-level` 通配符（3.3）、以及 `[workspace.dependencies]` 的提速幅度（4.1）。

> **收录它们，是因为一本只会复述网络建议的宝典没有价值。**
> 遇到与本文冲突的建议时，**在你自己的仓库上测一遍**——这正是规则 3.1 的意义。
