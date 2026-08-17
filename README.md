# ai-gateway

商业化 LLM API 中转网关，Rust + axum 实现，**不依赖任何上游网关 SDK**。
对外提供 OpenAI 兼容的 `/v1/*` 代理接口，内置多租户鉴权、按 token 精算计费、配额与订阅、
以及完整的运营管理面板（`/api/panel/**` + React 前端）。

## 能力概览

- **OpenAI 兼容代理** — 转发到 OpenAI / Claude / Gemini / Vertex / Codex 等上游，统一计费。
- **精确计费** — 预扣（Hold）→ 精算（Settle）→ 退款（Release）的账本流程，按四列单价（输入/输出/缓存/推理）逐 token 计费；结算与用量日志单事务原子提交。
- **安全** — JWT + API Key 双鉴权、登录限流、上游凭证 AES-GCM 落库加密、JWT 全端登出撤销。
- **运营面板** — 用户/分组/订阅/兑换码/退款/公告/定价/审计，支付充值（人工确认即可收款）。

架构与编码规范见 [`AGENTS.md`](AGENTS.md)。

DeepSeek Harness 用户可用 [`plugins/agw-oauth`](plugins/agw-oauth)（AGW-Oauth）OAuth 登录 AI-GateWay，无需手写模型配置。

## 快速开始（本地）

前置：Rust 1.97+、PostgreSQL、Redis。

```bash
# 1. 准备配置
cp config.example.yaml config.yaml
#    填写 database / redis 连接；生成并填入密钥：
make gen-secrets          # 输出 JWT_SECRET 与 CREDENTIAL_ENCRYPTION_KEY
export JWT_SECRET=...      # 或写入 config.yaml 的 auth.jwt.secret
export CREDENTIAL_ENCRYPTION_KEY=...

# 2. 构建并运行（自动建表 / migrate）
make run                  # cargo build --release 后运行 ./ai-gateway
#    或：./ai-gateway --config config.yaml

# 3. 前端（独立构建）
cd frontend && npm ci && npm run dev
```

启动后注意日志：若出现 JWT secret 为空、`CREDENTIAL_ENCRYPTION_KEY` 未设置、或
`sslmode=disabled` 的告警，说明对应生产项未配置（见下文上线清单）。

## 调用中转（用户视角）

1. 在面板注册并创建 API Key（`cpa-` 前缀）。
2. 像调用 OpenAI 一样请求 `/v1/*`，用该 Key 作 Bearer：

```bash
curl https://<your-host>/v1/chat/completions \
  -H "Authorization: Bearer cpa-xxxxxxxx..." \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}'
```

请求会先按预估额度预扣，响应结束后按上游返回的真实 token 用量精算扣费；余额不足返回 `402`。

## 测试

```bash
make test          # cargo test --workspace
make gates         # cargo xtask ci —— 9 条架构门禁
make lint          # clippy -D warnings
cd frontend && npm test       # 前端 vitest
```

## 上线

生产部署前请确认密钥注入、`sslmode=require`、反向代理 / TLS、密钥不入库自检等配置项。
`scripts/deploy-vps.sh` 提供了一个 VPS 部署示例。

**支付**：订单已持久化、入账幂等，当前可通过管理员人工确认（`PUT /api/panel/admin/orders/:id/confirm`）
收款；接入 Stripe/支付宝/微信**实时结算**需各渠道商户账号与密钥。
