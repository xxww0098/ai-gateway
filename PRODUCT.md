# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

主用户是付费租户：开发者或 Agent 用户。他们在自建部署的多租户中转上注册、拿 `agw-` 密钥、把客户端接到 OpenAI 兼容的 `/v1`，然后盯用量、余额和订阅读下去。

运营方（`admin` / `super_admin`）是次要但完整的第二受众：管用户、渠道、定价、订单与工单。运营面板不是主设计对象，但不能丢掉。

## Product Purpose

AI-GateWay 是商业化 LLM API 中转。租户用一张余额、一套密钥调用多家上游；网关按真实 token 精算扣费。成功是：租户能稳定打通模型并看清自己花了什么；运营能把账跑平，停机也不丢在途结算。

## Positioning

不依赖任何上游网关 SDK。对外只认 `agw-` 密钥和站点 origin 上真实存在的 `/v1` 路径。钱走 Hold → Settle → Release，按输入 / 输出 / 缓存 / 推理四列单价精算，而不是估个整数抹平。

## Operating Context

- 自建部署，面向运营方自己的付费租户（多租户中转生意），不是对外公有 SaaS，也不是纯内部工具。
- 租户日常：注册 / 登录 → 控制台 → 建密钥 → `/docs` 按客户端接入 → `/v1` 调用 → 用量 / 财务 / 订阅。
- 运营日常：渠道与上游凭证、定价、人工确认订单、退款、工单、审计。
- 接入说明只写 `gw-proxy` 里真实存在的路径（Claude 用站点 origin + `/v1/messages`）。
- DeepSeek Harness 可用 `plugins/agw-oauth` 设备码登录，不必手写模型配置。

## Capabilities and Constraints

已确认：

- 面板（React）+ `/api/panel/**`；代理内核在 `gw-proxy` 的 `/v1/*`（无 `/v1beta`）。
- 角色：`user` | `admin` | `super_admin`。
- 租户面：总览、密钥、模型、订阅、订单、用量、财务、工单。
- 计费不变量：Hold / Settle / Release 语义与签名不改；停机必须排空在途结算。
- 列名与公开 JSON 契约见 `CONTRACT.md`：钱是 `numeric`/`f64`，整数是 `bigint`/`i64`，OAuth 表是 `o_auth_sessions`。
- 支付：订单已持久化、入账幂等；当前可靠收款路径是管理员人工确认。
- 前端已有栈：Vite + React + Tailwind + 现有 shadcn 组件；不要另起一套运行时。

未决（不要当成已承诺）：

- 用量页要不要做成 CAP Tracker 同级的分析产品（趋势下钻、完整模式、价格簿、PNG 导出），还是只借用它的版式来呈现本仓库已有的用量 / 费用数据。
- 是否对租户暴露 USD/CNY 切换、缓存命中率、按小时聚合等 CAP 专有指标——以本仓库真实字段为准，缺字段就不编。

## Brand Commitments

- 品牌名是 **AI-GateWay**。新密钥只发 `agw-`。不接受、不兼容 `cpa-`、`CPA Gateway`、旧 localStorage 键。
- 面板文案以中文为主；对外产品名保持 AI-GateWay。
- 用户指定用量分析面的视觉参考，且只约束字体、布局、分块，不在此文件展开视觉世界：
  - 参考图：会话中的「Token 用量统计」截图
  - 参考实现：[AITNR/cap-token-usage-tracker](https://github.com/AITNR/cap-token-usage-tracker)（主体在 `dashboard.go` 内嵌的普通模式仪表盘）
- 参考实现是 CLIProxyAPI 插件，不是本产品。禁止把 CAP 的完整模式、管理密钥、价格同步、备份恢复做成 AI-GateWay 能力，除非日后单独确认。

## Evidence on Hand

- 落地页与接入文案：`frontend/src/pages/public/HomePage.tsx`、`frontend/src/pages/docs/`
- 标识：`frontend/public/icon.svg`、`frontend/src/shared/components/ui/Logo.tsx`
- 契约与禁令：`CONTRACT.md`、`AGENTS.md`、`README.md`
- 用量现状：`frontend/src/pages/user/usage/UsagePage.tsx`（明细表 + 四张 KPI，无趋势图）
- 外部参考源码：https://github.com/AITNR/cap-token-usage-tracker
- 没有客户证言、案例或第三方评测。后续工作不得编造。

## Product Principles

1. 租户先能打通、再能看账：密钥、接入、余额、用量是一条链，任何一环含糊都会让人不敢继续花。
2. 账必须对得上真实 token，不允许用界面叙事盖过 Hold / Settle / Release。
3. 面板只展示本仓库算得出的数；缺 usage、缺单价就标明缺口，不把空值画成零成本。
4. 删功能等于删文件夹。不为「以后可能用到」加第二套同名概念。
5. 视觉可以学参考仪表盘的分块，产品边界仍是 AI-GateWay，不是 CAP Tracker。

## Accessibility & Inclusion

面板以中文为主。没有额外的无障碍标准要求；控件仍需可键盘操作、有可见焦点，但不把 WCAG 等级写成门禁。
