# 项目树对抗审查

## 范围与结论

基准是本次会话开始时的当前工作树，不是 Git HEAD 或某个发布版本。根代理完成架构裁决，两个独立只读子代理检查后端和前端，随后按独占文件范围实施内部重构。既有未提交改动、锁文件、真实配置及数据库均不回滚、不迁移。

主目标：模块所有权、项目树、文档入口与架构门禁。邻接证据：前端 import、CI、支付/结算事务。排除：全量安全审计、线上流量、数据库故障注入、UI 改版和部署。

**结论：保留平铺 workspace；修正模块内部职责与文档导航，比整体搬迁更有价值。目录验收与资金正确性是不同结论，后者尚不能判定通过。**

## 按风险排序

### P1：支付状态与入账存在崩溃窗口（原审查发现，后续 02 已修复）

后续工作已实现并通过真实 PostgreSQL 验证，见 [支付确认说明](../payment-settlement.md) 与 [02 验证记录](../../specs/billing-hardening/slices/02-payment-credit-tx.md)。以下保留原审查时的证据，不代表当前实现仍是两次提交。

[settle_payment_order](../../crates/gw-panel/src/commerce/payment.rs) 先通过 pool 执行 `pending → paid`（163–174 行），再单独读取订单和调用 `Ledger::credit`（180–193 行）。进程若在两次持久写入之间退出，订单可留下 paid 而余额未到账；再次调用因认领行数为零直接返回（176–177 行）。错误补偿（195–205 行）不能处理进程已退出的情况。

反证边界：正常重复确认有条件 UPDATE 防护；这里不声称已经观察到生产丢账，也没有执行真实数据库故障注入。

归属：支付业务事务与 ledger 事务接缝。沿用 [计费加固方案](../../specs/billing-hardening/README.md)，先以真实 PG 钉住失败窗口，再让 paid 和 credit 进入同一事务。目录重构不应顺手改变资金流。README 已撤下无条件“入账幂等”的承诺。

### P2：架构门禁基线失败，且职责确实混杂（本轮处理）

基线 `cargo xtask ci` 的 file_length 失败项：

| 文件 | 基线行数 | 最终行数 | 裁决 |
|---|---:|---:|---|
| gw-oauth/antigravity/request.rs | 1671 | 741 | 分离请求构造、响应转换、会话状态 |
| gw-oauth/antigravity/request/tests.rs | 1252 | 116 | 按行为分组，保留同一测试隔离锁 |
| gw-oauth/kiro/request.rs | 1133 | 922 | 提取独立 eventstream 帧 codec |
| gw-proxy/hold.rs | 1005 | 977 | 消除重复日历，直接复用 gw-model |

行数只是信号；本次拆分依据不同变化原因，而不是把代码平均分块。没有放宽 allowlist、扩大 ignore、新增 crate 或改清单。

### P2：前端的协议知识放在页面层（未修，静态核实）

[user-api-keys/endpoints.ts](../../frontend/src/features/user-api-keys/endpoints.ts) 第 1 行及 QuickIntegrationPanel 引入 [pages/docs/guide.ts](../../frontend/src/pages/docs/guide.ts) 的 Base URL 计算。API Key 和快速接入依赖文档页面目录，会让文档视图整理牵动业务消费者。

归属：共同的网关协议模块，而非文档页。先移 Base URL 等纯协议知识，再统一导入并加 features 不依赖 pages 的检查。反证：guide 本身是轻量函数/数据模块，未测 bundle；**不能据此声称已加载整张文档 UI 或破坏 tree-shaking**。

### P2：shared 同时充当基础层与应用层（未修，静态核实）

[shared/api/client.ts](../../frontend/src/shared/api/client.ts) 第 1 行依赖 features/auth 的 store，60、83 行读取 token/执行 logout；shared/layout 也组合 auth 与通知业务。pricing、usage 又消费 admin-proxy 的请求 API，反方向还存在类型/展示依赖。

后果是所有权不明确、改动影响面难估计；建议先把应用布局和纯基础 UI 分开，再复用现有 sdkClient 收口传输。反证：auth_store 只依赖 Zustand；部分跨 feature 引用是 `import type`。**这不是已证明的运行时依赖环，更不是一个已证实的安全漏洞。** 不采用增加全局 token getter 注册器或第三套 HTTP client 的捷径。

### P2：前端唯一局部边界规则已脱离真实目录（未修，静态核实）

[eslint.config.js](../../frontend/eslint.config.js) 81–105 行保护 admin-users 路径，而这两处目录已不存在；[CI frontend job](../../.github/workflows/ci.yml) 只有 typecheck/test/build，没有执行 lint。即使更正路径，不接入 CI 也不构成合并门禁。

归属：前端架构约束与 CI。按真实模块设计规则，先清存量，再由 CI 执行；不能只加一个从不运行的规则。

### P2：验证暴露两个测试端缺陷（本轮追加修复）

全量测试先在 [gw-server wiring 测试](../../crates/gw-server/src/wiring/tests.rs) 失败：测试期望 `xai`，实际包含 `grok`。[provider 定义](../../crates/gw-provider/src/common.rs) 第 33–36 行已明确声明 `grok` 为存储/注册名。仅将测试期望对齐该既有约定，没有修改生产名称或把断言改成从实现读取答案。

后续直接相关套件的 Grok 登录/配额测试出现 `IncompleteMessage`。[mock server](../../crates/gw-oauth/src/grok/test_support.rs) 使用非阻塞 listener，却把 accept 得到的 socket 交给阻塞式 reader。在本机最小实验中，未发任何请求字节时 read 立即返回 `WouldBlock`（而非等待配置的读超时）；改为阻塞后可收到完整数据。原因是 macOS 会继承该标记。fixture 已显式设置 accepted socket 为阻塞，保持原读超时；不加 sleep/retry、不忽略测试、不修改生产 HTTP 客户端。

### P3：文档入口与代码事实漂移（本轮处理入口）

README/CONTRACT 引向已删除的 rust-engineering.md；AGENTS.md 现只写代理协作，不再承载所宣称的计费规则；根目录树漏列 gw-role，并将整个 gw-relay 描述成不识内容的纯中继。入口改为 architecture + docs 导航，并修正 lint 状态及测试计数建议。

反证边界：没有重写全部历史报告与源码注释。旧报告中的规则编号、路径和测量环境仍应作为历史上下文阅读，不能自动升格为当前约束。

## 对独立审查意见的裁决

- **不接受“门禁红说明 hook 被绕过”**：当前有未提交工作，未检查 hook，无法推出绕过结论。
- **不接受“kiro/xai 无人拥有”**：所有权门禁明确从 coordinator 所有的 lib.rs 继承；这是当前分配，不是缺口。可以在下一次实际分工时细化，不虚构负责人。
- **不把类型导入当运行时环，不凭目录名宣称包体变大**：需要解析实际 import 图或构建数据。
- **不做全仓依赖清扫**：零文本引用只是候选证据；与本轮模块整理无关的 manifest/lock 变化留待单独验证。
- **接受有测试保护的内部拆分**：虽然比只改文档更大，但用户明确要求优化项目树，且现有门禁已经失败。保留公开 API、单一状态所有者及原测试，避免把机械挪动变成行为改造。

## 验证记录

- 基线：xtask 78 个测试通过；架构门禁 8/9 通过，file_length 报上述 4 项。
- 修改前一次 OAuth/proxy 联跑返回 exit 101；当次尾部摘要未保留。紧接着用简洁输出复跑：OAuth 277 通过；proxy 254 通过、19 ignored。初次失败原因未定位，不将一次绿灯解释为没有偶发失败。
- 最终架构门禁：`cargo xtask ci` 全部通过，4 项 file_length 失败归零。
- 全量/直接相关测试曾分别暴露上述 xai/grok 断言与 Grok mock socket 故障，均定位后按测试端的既有语义修复，最终结果以下方汇总为准。
- `cargo check -p gw-oauth --all-targets --locked` 与 `cargo clippy -p gw-oauth -p gw-proxy --all-targets --locked -- -D warnings` 通过。
- 集成编译曾发现 Antigravity 提取遗漏 4 个 helper；根代理从修改前证据恢复，并移除未使用导入。随后按原文完整函数清单比对：74 个生产函数及 36 个测试/辅助函数均一一保留，函数文本除可见性标记外无差异。大文件原文末尾 39 行来自实现者修改前读取的转录证据，不把截断快照冒充完整备份。
- 测试端修复后：`cargo test --workspace --locked -- --format terse` 通过，**1734 passed / 0 failed / 133 ignored**（不含前置的 Grok 46 项及 wiring 单项定向复测）。ignored 是现有外部服务档；没有启动真实 PostgreSQL/Redis，不声称这些用例通过。
- `cargo clippy --workspace --all-targets --locked -- -D warnings` 通过；修改文件 rustfmt 检查通过。
- Grok 的 46 项相关测试在 fixture 修复后额外连续跑 3 轮，全部通过，不作为对初次 exit 101 原因的追溯证明。
- 变更文档的 48 个本地链接均存在。
- 独立 `codex review` 对本轮限定差异的结论：未发现实质性新增回归。它对照会话前快照及其注明的原文尾部证据，静态检查函数体、公开入口、共享缓存、测试接线与文档；未代替根代理跑测试，也不为前述未修风险背书。
- 本机安装的 rust skill 缺少 verify_patch.py；不伪造该脚本的 proven 结果，实际验证以列明的 Cargo 命令和退出码为准。

## 补丁与决策

| 补丁 | 不变量与所有者 | 拒绝的捷径 |
|---|---|---|
| Antigravity 内部分解 | 请求 API 不变，状态只有一份，测试共用隔离锁 | 复制静态缓存、削弱断言、改变协议算法 |
| Kiro codec 提取 | 字节帧函数体保持，类型与公开入口不变 | 添加 wrapper、修改 CRC/错误策略 |
| 订阅日历归一 | 创建和准入共用 model 日历；不改额度比较和持有顺序 | 新建 billing crate、混入事务重构 |

日历复用保留可表示的下一 UTC 重置点；在 Chrono 日期上限无法表示下一周期时，沿用 model 的 checked/fallback 语义，不再保留代理原实现的溢出 panic。这是类型极值的行为差异，不是线上时钟范围的计费策略调整。

用户未指定的范围/架构选择及置信度见 [决策记录](choices.md)。
