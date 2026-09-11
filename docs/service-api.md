# 服务面 API（/api/service）— ozon-pod 集成契约

- 状态：v1 冻结（供 ozon-pod-web 实现与两侧测试共同引用）
- 机器可读夹具：[`crates/gw-panel/tests/fixtures/service-api.json`](../crates/gw-panel/tests/fixtures/service-api.json)（ozon-pod 侧镜像：`fixtures/agw/service-api.json`）
- 不在本契约内：`/v1/*` 代理与结算时序、面板 API、ozon-pod 侧存储

## 1. 这份契约解决什么

ai-gateway（AGW）是 ozon-pod-web 的 AI 上游与唯一计费方。ozon-pod 需要：为每个 Workspace 建一个 AGW 账号并拿到 API Key；用该 Key 走 `/v1/*` 转发；用同一账号读取余额与消费明细；把用户在 ozon-pod 支付/兑换得到的钱幂等入账。本契约只定义账号、密钥、余额、用量与入账这五件事的服务面。

## 2. 通用规则

| 项 | 规则 |
| --- | --- |
| 挂载 | 仅当配置 `service.token` 非空时挂载 `/api/service/**`；未配置时整组路由不存在（404） |
| 鉴权 | `Authorization: Bearer <service.token>`；常量时间比较；缺失或不匹配 → HTTP 401 + `code 1001` |
| 信封 | 与面板一致：成功 `{"code":0,"message":"ok","data":{…}}`，失败 `{"code":<业务码>,"message":"…"}` |
| 业务码 | `1001` 未授权 · `4000` 请求非法 · `4004` 对象不存在 · `4009` 冲突 · `5000` 内部错误 |
| 金额 | decimal **字符串**（8 位小数），如 `"10.00000000"`；token 数与计数为整数 |
| 时间 | RFC3339 UTC，如 `2026-09-11T10:00:00Z` |
| 幂等 | 每个写操作都必须能在超时重试下安全重放，见 §4 |
| 供给账号 | 由服务面创建，**不赠注册额度**（`initial_register_credit` 不适用），且**没有可用口令**：入库的是随机口令的 bcrypt 哈希，明文不返回、不落库 |

## 3. 端点

### 3.1 `POST /api/service/users` — 建号（按 email 幂等）

请求：`{"email":"owner+ws1@example.com","display_name":"Workspace 1"}`（`display_name` 可省，写入 `username`）

响应 `data`：`{"user_id":42,"email":"owner+ws1@example.com","created":true,"status":"active"}`

- email 先 trim + 小写；不含 `@` → 4000。
- 已存在同 email：返回既有 `user_id` 与 `created:false`，**不改余额、不改状态**。
- 已存在但 `status != 'active'` → HTTP 409 + 4009。
- 新行：`role='user'`、`balance=0`、`status='active'`、`concurrency=1`。

### 3.2 `POST /api/service/users/{user_id}/keys` — 发/轮换 Key

请求：`{"name":"ozon-pod:workspace:1"}`（1–64 字符，非空）

响应 `data`：`{"key_id":9,"key":"agw-<64hex>","key_prefix":"agw-<12hex>","name":"ozon-pod:workspace:1","rotated":true}`

- 单事务：先把该 user 下同名且 `status='active'` 的行置 `revoked`，再插入新行（`status='active'`）。
- `rotated` 表示本次是否撤销了旧 key（同名重发 = 轮换，天然可重放：超时重试不会留下两个 active key）。
- 明文只在本次响应出现一次；库里只存 `key_hash` 与 `key_prefix`（复用 `gw-authcore` 的 `new_api_key`/`api_key_prefix`/`hash_api_key`）。
- 未知 `user_id` → 4004。

### 3.3 `GET /api/service/users/{user_id}/balance`

响应 `data`：`{"balance":"12.34000000","currency":"USD"}`（`currency` 取自 `service.currency`，默认 `"USD"`）

### 3.4 `GET /api/service/users/{user_id}/usage`

查询参数：`start`、`end`（RFC3339，可选；默认最近 7 天；窗口上限 90 天，超限 → 4000）、`page`（默认 1）、`page_size`（默认 50，上限 200）

- 窗口是**闭区间** `[start, end]`；缺省 `end` = 服务器当前时间，缺省 `start` = `end - 7 天`。
- `page_size` 超上限**夹紧到 200**（不是报错）；`page` 从 1 开始。

响应 `data`：`{"page":1,"page_size":50,"total":1,"rows":[…]}`，每行：

```json
{
  "id": 100, "request_id": "…", "idempotency_key": "ozon:req-1",
  "model": "gpt-4o", "provider": "openai",
  "tokens_in": 120, "tokens_out": 30, "tokens": 150,
  "cost": "0.00015000", "actual_cost": "0.00015000",
  "failed": false, "duration_ms": 812, "created_at": "2026-09-11T10:00:00Z"
}
```

**扣款口径**：`actual_cost` 是实际扣款（失败请求为 `"0.00000000"`），`cost` 是价表算出额；ozon-pod 以 `actual_cost` 记账，两者都原样透传，不做运算。

实现口径（与面板用量明细同一套四列回退规则，别名只是历史列名）：

| 输出 | 取值 |
| --- | --- |
| `tokens_in` / `tokens_out` | `input_tokens > 0 ? input_tokens : tokens_in`（`output_tokens` 同理） |
| `tokens` | `tokens_in + tokens_out` |
| `cost` | `total_cost > 0 ? total_cost : cost` |
| `actual_cost` | `failed = true` → `"0.00000000"`；否则 `actual_cost > 0 ? actual_cost : cost` |
| `created_at` | 秒精度 RFC3339 UTC（与 §2 的示例同形，无小数秒） |

### 3.5 `GET /api/service/users/{user_id}/usage/by-idempotency-key/{key}` — 单笔对账

响应 `data`：`{"found":true,"row":{…同 3.4 的行…}}`

- 命中 → `found:true`；未命中 → **HTTP 200 + `found:false,"row":null`**（不是 404：对 ozon-pod 表示"尚未结算，稍后再问"，而未知 `user_id` 才是 404/4004）。
- `key` 可能含 `:` 等字符，调用方负责百分号编码；服务端按解码后的原值匹配 `usage_logs.idempotency_key` + `user_id`。

### 3.6 `POST /api/service/users/{user_id}/credits` — 幂等入账

请求：`{"amount":"10.00000000","idempotency_key":"ozon-order:PO-1","note":"充值"}`（`note` 可省）

- `amount` 以 **decimal 字符串**为准（如上例）；实现同时接受 JSON 数字（同一个 decimal 值的另一种写法），但字符串是双方的规范写法。
- `note` 目前只被接受、**不落库**：账本的入账流水没有备注列，服务端不会为一个展示字段绕开账本自己写日志。

响应 `data`：`{"applied":true,"duplicate":false,"balance":"22.34000000"}`

- `amount` > 0 且 ≤ `service.max_credit`（默认 100000），否则 4000；`idempotency_key` 非空且 ≤128 字符。
- 账本 reference = `service_credit:<idempotency_key>`（见 §4）；重复 → `applied:false, duplicate:true` + 当前余额。
- 同一 reference 被用于不同 user 或不同 amount → HTTP 409 + 4009（绝不对账到另一笔钱）。
- 未清偿欠款（shortfall）语义沿用 `gw-ledger` 既有规则，不在本端点里特判。

### 3.7 `GET /api/service/models` — 目录只读镜像

响应 `data`：

```json
{"models":[{"id":"gpt-4o","owned_by":"openai",
  "input_price_per1_m":"0.00015000","output_price_per1_m":"0.00060000",
  "cached_input_price_per1_m":"0.00007500","reasoning_price_per1_m":"0.00060000"}]}
```

- `id` = `model_catalog_entries.model_id`；`owned_by` = 提供该模型的渠道 `channel_key`（与 `GET /v1/models` 的映射一致）。只列 `visible = true` 的行，同一模型只出现一次（渠道字典序最小的那个）。
- 四个价格字段名**逐字取 `gw-pricing` 的列名**（`input_price_per1_m`，`1` 与 `M` 之间断开，CONTRACT §3.5 的既有事实；**不是** `input_price_per_1m`），金额是 8 位小数的 decimal 字符串，单位是每 1M token；目录里有、价表里还没有的模型四个字段都是 `"0.00000000"`。
- **价格字段是可选的**（本条第一句就是契约允许的形态）：夹具 `endpoints.models.response.models[0]` 只给必填的 `id` / `owned_by`，形状断言必须按"必填字段都在且类型正确"来写，不能对模型条目做键集合完全相等。ozon-pod 只用它校验"默认中转模型存在"与做只读展示。

## 4. 入账幂等（本契约的硬约束）

- `balance_logs.reference` 已有按前缀分区的唯一索引（`payment_order:%`、`redeem:%`、`initial_register_credit`）。服务面新增第三个前缀：
  ```sql
  CREATE UNIQUE INDEX IF NOT EXISTS idx_balance_logs_service_credit
      ON balance_logs (reference)
      WHERE type = 'credit' AND reference LIKE 'service!_credit:%' ESCAPE '!';
  ```
- `gw-ledger` 的 `credit_tx` 预查目前只对 `payment_order:` 前缀去重；实现须把 `service_credit:` 纳入同一去重分支（建议收成一个前缀集合常量，避免两处写死），预查谓词必须与索引谓词逐字对应（含 `ESCAPE '!'`），否则规划器用不上该索引。
- 冲突语义：同 reference 不同 user 或不同金额 → 拒绝（409），**不能**静默改账。

## 5. 配置

| 配置键 | env | 默认 | 说明 |
| --- | --- | --- | --- |
| `service.token` | `SERVICE_TOKEN` | 空 | 空 = 服务面不挂载 |
| `service.initial_credit` | `SERVICE_INITIAL_CREDIT` | `0` | 供给账号的初始额度；契约要求 0，非 0 只作实验 |
| `service.currency` | `SERVICE_CURRENCY` | `"USD"` | 余额/金额的币种标签，只作展示与透传 |
| `service.max_credit` | `SERVICE_MAX_CREDIT` | `100000` | 单次入账上限 |

`initial_credit` 非 0 时，`POST /users` 在**建号那条事务里**给新账号入账一次，reference 为 `service_credit:initial:<user_id>`（仍属 §4 的命名空间，所以「一次建号只赠一次」由同一个部分唯一索引兜底）；默认 0 = 不赠，此时 `balance_logs` 里没有这个账号的任何流水。

## 6. 夹具与测试用法

两侧各存一份同内容 JSON。测试只断言**字段名集合与类型**（以及路径/方法/错误码），不断言夹具里的字面值——夹具是形状的单一事实源，不是样例数据。任何形状变更必须同一变更里更新两份夹具与本文。

## 7. 已知边界

- 服务面是**内网接口**：生产只在 compose 网络内可达；`SERVICE_TOKEN` 泄露等于可以任意建号与入账，按部署密钥对待（不进日志、不进镜像）。
- 供给账号没有可用口令，也没有密码登录路径；将来若要允许用户登录 AGW 面板，另立"设置口令"流程，不在本契约内扩。
- 用量行的出现即代表结算已结束（`usage_logs` 在结算时写入）；不存在"写入中"的中间行，因此 3.5 的 `found:false` 语义就是"还没结算完"。
