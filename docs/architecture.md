# 架构与项目树

## 目录表达所有权，而不是排版

保留根 [Cargo workspace](../Cargo.toml) 的平铺 crate 结构。crate 是编译与依赖边界，模块才是单个协议、状态或算法的归属；只有出现独立复用、发布或依赖隔离需求时才新建 crate。不要为了让目录树对称，把业务拆成横贯全仓的 `services/`、`utils/` 或另一个通用框架。

下面是职责导航，不是完整文件清单；真实成员与依赖以 Cargo 清单为准。

```text
ai-gateway/
├── crates/              # 平铺 Rust 所有者；crate 内再按业务或协议职责拆分
├── frontend/            # 面板客户端，独立构建
├── plugins/             # 外部客户端集成，不进入代理热路径
├── migrations/          # 既有数据库的演进，不随目录整理改 schema
├── tools/xtask/         # 可执行的仓库约束
├── scripts/             # 开发、发布操作脚本
├── deploy/              # 部署拓扑与镜像配置
├── docs/                # 架构原理、运维参考、标明范围的审查证据
└── specs/               # 尚待执行的方案；不等于已经实现的保证
```

## 后端：组合根、请求编排和能力各有主人

- **组合根**：[gw-server](../crates/gw-server/src/wiring.rs) 负责装配进程级资源与组件。[停机流程](../crates/gw-server/src/drain.rs) 负责等待请求与结算任务；不要把装配配置散入 executor。
- **数据面**：[gw-proxy](../crates/gw-proxy/src/lib.rs) 编排鉴权、预扣、路由、结算与恢复。[ports](../crates/gw-proxy/src/ports.rs) 是消费者要求的能力，[adapters](../crates/gw-proxy/src/adapters) 是这些能力的真实存储/服务实现。两者目前在同一 crate，属于可测试接缝，**不是**编译期存储隔离；不要为了消灭 adapter 而把业务 SQL 塞进 gw-infra。
- **控制面**：[gw-panel](../crates/gw-panel/src) 按身份、商务、上游、计费展示等业务域组织。同一业务的用户与管理员入口共享业务归属，不为角色另造一套后端能力。
- **上游边界**：[gw-provider](../crates/gw-provider/src) 管 executor；[gw-oauth](../crates/gw-oauth/src) 管上游家族的凭证流程及相关协议适配。[gw-relay](../crates/gw-relay/src) 的转发 engine 与内容感知的 endpoint/translate 分工明确：**engine 的字节通路不解释业务，不代表整个 crate 不解析 JSON**。
- **能力与底座**：账本归 gw-ledger、价格计算归 gw-pricing、实体及兼容解码归 gw-model、角色定义归 gw-role、池/缓存/限流原语归 gw-infra、鉴权与凭证密码学归 gw-authcore、配置归 gw-config。不能因为同一请求会调用它们，就把它们合并成一个大 crate。

`gw-relay`、`gw-oauth`、`gw-role`、`gw-config` 保持不依赖工作区其它业务 crate；面板与代理互不依赖，由 server 组合。Cargo 能拒绝环，但不会替代对新增依赖方向的架构评审。

## 在模块内部拆职责

请求构造、响应转换、状态缓存、传输帧编解码有不同的变化原因，应各有一个实现位置：

- Antigravity 的请求入口保留协议 API，内部把响应/流转换与会话状态分开。pin 和 thought-signature 的静态存储只能有一份；跨请求的测试必须共享同一把隔离锁。
- Kiro 的 eventstream 编解码归请求模块的 codec 子模块；请求构造不能同时拥有 CRC/header 帧细节。原有消费者仍经请求 API 调用，避免目录调整引起调用面迁移。
- 订阅的 UTC 日/周/月重置日历由 [gw-model 的 subscription](../crates/gw-model/src/subscription.rs) 定义。创建订阅与 [代理轮转](../crates/gw-proxy/src/hold.rs) 使用同一实现；额度判断、行锁和账本事务不是日历的职责。

拆分时保留少数明确的公开入口，子模块默认私有。公开入口的显式 re-export 可以维持领域边界；不要额外创建仅为转发而存在的文件或函数。测试优先通过原有请求/响应入口，而不是新私有 helper。

## 前端：目标分层与下一步落点

目标方向是 `app/pages → features → shared`。这是治理方向，**当前树尚未完全满足**；不要把它当成现有 ESLint 已保证的事实。

```text
frontend/src/
├── app/                        # 建议落点：路由、带业务的应用布局与装配
├── pages/                      # 路由页面，组合业务能力
├── features/                   # 业务模块，不反向导入 pages
└── shared/
    ├── api/                    # 传输、错误、通用客户端；不再复制请求实现
    ├── gateway/                # 建议落点：客户端端点与协议约定
    └── components/ui/          # 无业务依赖的基础 UI
```

这里的 `app/` 与 `shared/gateway/` 是**待实施建议**，不是已经搬迁的目录。实施顺序：

1. 将文档页中的 Base URL 计算移到共同的协议所有者，让 API Key、快速接入和文档页一起消费它。
2. 先区分应用布局和基础 UI，再处理 shared 对 auth 的依赖。凭证读取和 401 处理必须保留同一生命周期，不为打破路径反向引用引入全局可变注册器。
3. 管理 API 优先复用现有 `sdkClient`；辨清 admin-proxy 的业务动作与通用 HTTP 请求，不再新造第三套客户端。
4. 清理失效的规则作用域，再在 CI 中真实执行层级规则。类型导入和运行时环分别分析；仅凭跨目录 import 不能断言包体变大或发生运行时循环。

本轮不修改 frontend，不更换 UI，不移动页面路由。

## 验收与风险分开

可执行规则的唯一目录是 [xtask registry](../tools/xtask/src/gates.rs)。它检查可达性、所有权及清单/文件等约束；绿灯不是“资金安全已证明”，也不等于前端依赖检查通过。调整树必须同时保持原有测试被编译和执行，不能通过增加 allowlist、删测试或扩大 ignore 绕过问题。

资金一致性工作沿用 [计费加固方案](../specs/billing-hardening/README.md) 单独推进。目录整理不修改支付入账、结算幂等、恢复存储、额度软硬上限或数据库迁移。需要真实 PostgreSQL/Redis 的验收不能用纯单测或空账本替代。
