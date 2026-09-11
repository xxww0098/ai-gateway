# ai-gateway

商业化 LLM API 中转网关，Rust + axum 实现，**不依赖任何上游网关 SDK**。
对外提供 OpenAI 兼容的 `/v1/*` 代理接口，内置多租户鉴权、按 token 精算计费、配额与订阅、
以及完整的运营管理面板（`/api/panel/**` + React 前端）。

## 能力概览

- **OpenAI 兼容代理** — 转发到 OpenAI / Claude / Gemini / Vertex / Codex 等上游，统一计费。
- **精确计费** — 预扣（Hold）→ 精算（Settle）→ 退款（Release）的账本流程，按四列单价（输入/输出/缓存/推理）逐 token 计费；结算与用量日志单事务原子提交。
- **安全** — JWT + API Key 双鉴权、登录限流、上游凭证 AES-GCM 落库加密、JWT 全端登出撤销。
- **运营面板** — 用户/分组/订阅/兑换码/退款/公告/定价/审计，支付充值（人工确认即可收款）。

工程约束见 [`CONTRACT.md`](CONTRACT.md)，架构与目录落点见 [`docs/architecture.md`](docs/architecture.md)；调研、运维与审查记录从[文档导航](docs/README.md)进入。

DeepSeek Harness 用户可用 [`plugins/agw-oauth`](plugins/agw-oauth)（AGW-Oauth）OAuth 登录 AI-GateWay，无需手写模型配置。

## 仓库结构

```
ai-gateway/
├── Cargo.toml              # 虚拟根清单：resolver 3、edition 2024、MSRV 1.97
├── CONTRACT.md             # 工程契约：所有权、硬约束、数据库既成事实
├── rust-toolchain.toml     # 钉死 1.97.1 + clippy/rustfmt
├── migrations/             # sqlx SQL 迁移（对既有 schema 幂等）
├── crates/                 # 平铺，目录名 = crate 名（规则 1.3）
│   ├── gw-config/          #   YAML + 环境变量配置
│   ├── gw-model/           #   实体、迁移、种子、列解码与订阅日历
│   ├── gw-role/            #   角色语义的叶子 crate
│   ├── gw-infra/           #   PG 池、Redis、缓存、限流、熔断
│   ├── gw-authcore/        #   JWT、API Key、AES-GCM 凭证加密、AuthStore
│   ├── gw-pricing/         #   ModelPriceCache + 四列单价 Calculator
│   ├── gw-ledger/          #   Hold/Settle/Release 账本（Redis Lua + PG）
│   ├── gw-oauth/           #   上游 OAuth 家族（一家一目录，缓存隔离）
│   ├── gw-provider/        #   上游 executor、协议适配装配与 usage 解析
│   ├── gw-proxy/           #   /v1/* 代理内核（无 /v1beta）
│   ├── gw-panel/           #   /api/panel/** 运营面板，按业务域切分
│   ├── gw-relay/           #   字节转发 engine + 端点规范与协议转换
│   └── gw-server/          #   组合根：装配、迁移、种子、优雅停机
├── tools/xtask/            # 架构门禁（cargo xtask ci）
├── plugins/agw-oauth/      # DeepSeek Harness：设备码登录 AI-GateWay
├── docs/                   # 架构、审查、运维与性能证据（README 导航）
├── specs/                  # 待实施方案，与已落地保证区分
├── scripts/                # 开发与发布操作脚本
├── frontend/               # React 前端（独立构建）
├── deploy/                 # Dockerfile + compose
├── config.yaml             # 运行时配置（不入库）
└── config.example.yaml     # 配置模板
```

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
cd frontend && npm ci --legacy-peer-deps && npm run dev
```

启动后注意日志：若出现 JWT secret 为空、`CREDENTIAL_ENCRYPTION_KEY` 未设置、或
`sslmode=disabled` 的告警，说明对应生产项未配置（见下文上线清单）。

## 调用中转（用户视角）

面板接入指南：打开 `/docs`。

1. 在面板注册并创建 API Key（`agw-` 前缀；本版本起硬切换，旧前缀不再接受）。
2. 像调用 OpenAI 一样请求 `/v1/*`，用该 Key 作 Bearer：

```bash
curl https://<your-host>/v1/chat/completions \
  -H "Authorization: Bearer agw-xxxxxxxx..." \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}'
```

请求会先按预估额度预扣，响应结束后按上游返回的真实 token 用量精算扣费；余额不足返回 `402`。

## 测试

```bash
make test          # cargo test --workspace（无需外部服务）
make test-ignored  # 需要真 Postgres/Redis 的那一档
make gates         # cargo xtask ci —— 9 条架构门禁
make lint          # clippy --all-targets -- -D warnings
cd frontend && npm test       # 前端 vitest
```

```bash
make build         # cargo build --release → ./ai-gateway
make run           # 构建并以 config.yaml 启动
./ai-gateway --config config.yaml
./ai-gateway --version
./ai-gateway --health-check   # 探针：ready → 0，否则 1
```

测试分层与约束见 [`CONTRACT.md`](CONTRACT.md)；资金一致性的已知风险与待实施工作见[计费加固方案](specs/billing-hardening/README.md)。架构门禁通过不代表计费竞态已经解决。

## 上线

生产部署前请确认密钥注入、`sslmode=require`、反向代理 / TLS、密钥不入库自检等配置项。

Docker（vps-api，不替换现网 CPA / APS）：

```bash
cp deploy/env.vps.example /opt/ai-gateway/.env   # 填 JWT_SECRET、CREDENTIAL_ENCRYPTION_KEY、DB_PASSWORD
docker compose --env-file /opt/ai-gateway/.env -f deploy/docker-compose.vps.yml up -d --build
# 把 deploy/Caddyfile.vps 追加到宿主机 Caddy：agw.xxww.online
```

`scripts/deploy-vps.sh` 是 musl 二进制部署示例，和上面的 Docker 栈二选一。

**支付**：当前支持订单持久化与管理员人工确认（`PUT /api/panel/admin/orders/:id/confirm`）。
支付确认现将订单状态、余额与信用流水纳入同一 PostgreSQL 事务，支持重复通知及已确认的历史缺账重试。上线前必须完成[支付迁移与对账检查](docs/payment-settlement.md)，不能新旧支付写入者混跑；其余资金一致性工作见[计费加固方案](specs/billing-hardening/README.md)。接入 Stripe/支付宝/微信**实时结算**仍需各渠道商户账号与密钥。
