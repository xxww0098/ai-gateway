# 计费正确性加固

分析日期：2026-09-07。基准：当前工作区（含未提交改动），不是某个已发布版本。

本计划针对 [计费深度报告](/Users/xxww/.codex/visualizations/2026/09/07/01a07c05-efe5-7a71-a813-0bc337b2e65b/billing/analysis.md) 的五类静态窗口。四路只读探索核对了当前源码；三份独立草稿（最少切片 / 风险优先 / 接缝质量）由根代理合成。**不改计费语义、不改前端、不把性能当第一刀。**

## Next Agent Prompt

当前状态：**01–05 已在工作树实现并验证，未部署。** 兑换、订阅购买、注册入账与资金同事务。下一步是 [06-token-and-quota-contracts](slices/06-token-and-quota-contracts.md)（产品合同；硬额度仅在确认后做）。不要把本轮完成解释为全计费加固完成。

全局 TODO：

- [x] 00 钉死 A/B/C/D 表征（A0 先红后绿；B0/C0/D0 为现窗表征）
- [x] 01 请求成功结算幂等（关 A + failed 行不再挡成功结算）
- [x] 02 支付订单与 credit 同事务（工作树已验证；上线需停写与迁移预检查）
- [x] 03 持久 pending 作为恢复源（关 B；扫描 SQL pending，grace=hold_ttl）
- [x] 04 Redis 余额条件发布（关 C；`balance_version` + 条件 Lua，C0 已翻绿）
- [x] 05 兑换 / 订阅购买 / 注册入账（同事务；0013 分区唯一；支付补入分支仍保留）
- [x] 06a token 互斥 + 子费率留空回落（一手证据见 [slices/06](slices/06-token-and-quota-contracts.md) 的「落地记录」）
- [ ] 06b 价格快照（仍是结算现价）、06c Anthropic 缓存写独立价目列、06d 硬额度（仅产品确认后）
- [ ] 07 真实账本 profile（01–04 之后；禁止用 NullLedger 火焰图做计费性能结论）

结束本轮前更新本节的当前状态、下一拾取点和勾选。

不要：只改扫描间隔或拉长 Redis TTL 当恢复存储；给全部 `usage_logs.request_id` 或全部 `balance_logs.reference` 加唯一约束；改 `HOLD_SCRIPT` 的 float 编码；把订阅额度算进余额 Lua。

## 目标

一次成功请求只扣一次款；一次支付订单只入一次账；崩溃后仍能识别待结算且重放不重复收费；Redis 余额不得回升到比 Postgres 更旧的快照。

## 非目标

- 更快的 Lua / 更少 SQL 往返（放到 07 的测量之后）
- `f64` → Decimal 热路径迁移
- 真实 Stripe/支付宝/微信接通
- 退款 approve 自动退余额（现状是线下人工，已有测试钉死）
- 前端改动
- 让 HTTP 200 等待 SQL 提交
- 新建 `gw-billing` crate

## 切片图

```text
00 钉死竞态（必须在现码上失败）
 │
 ▼
01 成功结算幂等 ── settlement_intents 在 settle 时 CAS
 │
 ├──────────────► 02 支付 credit_tx 同事务
 │                    │
 │                    └─► 05 兑换 / 订阅购买 / 注册
 ▼
03 持久 pending + SQL 扫描（依赖 01，否则重放双扣）
 │
 ▼
04 余额版本条件发布
 │
 ▼
06 额度 / token / 价格合同（硬额度仅当产品要求）
 │
 ▼
07 真实 Postgres+Redis 计费 profile
```

上图是原规划顺序，不是 02 对 01 的功能依赖。本轮先完成支付 P1：02 已落地，余额操作归 `ledger/balance.rs`。后续 01 仍不得与其它 worker 同时编辑 ledger 共享文件；新增迁移必须使用大于已部署最大版本的新编号，不补插较小的预留版本。

## 已核对的事实（工作区，2026-09-07）

| 窗口 | 现状 | 谁亏钱 |
|---|---|---|
| A 并发双结算 | `skip_if_already_logged` 在 `users FOR UPDATE` 之前普通 SELECT；`request_id` 非唯一；热路径 `skip=false` | 用户被多扣 |
| B 恢复源先过期 | 默认成员 TTL 300s、键 360s；扫描只要 >30min 的 Redis 成员。`config.yaml` 把 TTL 配成 **3600s**，扫描年龄仍硬编码 30min：长请求可被扫描再被热路径再扣 | 默认 TTL：运营吃上游账单；3600s+自动补账：用户可能双扣 |
| C 缓存乱序发布 | 无版本 `SETEX`；注释“最多偏一笔”为假，偏高幅度是后续已提交扣款之和 | 短暂多放行，随后 shortfall 或运营垫上游 |
| D 订阅软额度 | `lock_and_rotate` 先 commit 再比较；无在途预留 | 仅当产品要硬帽时才是缺陷 |
| E paid 无入账 | `pending→paid` 与 `Ledger::credit` 两次提交；已 paid 直接返回；credit 无唯一 reference | 用户已付，余额不到 |

退款 approve 不动余额，是产品设计，不是漏实现。

## 目标架构（一个不变量一个主人）

| 不变量 | 主人 | 不是谁的职责 |
|---|---|---|
| 请求成功只扣一次 | `Ledger::settle_tx` + `settlement_intents` | `usage_logs` 任意行、`claim_finalize`、HTTP 幂等键 |
| 崩溃后仍能找到待结算 | 同一张 `settlement_intents`（pending） | Redis hold TTL |
| 缓存不回写旧余额 | `users.balance_version` + 发布 Lua | Hold 准入脚本 |
| 订单 paid 必有 credit | `Ledger::credit_tx` 加入面板事务 | 面板自己读流水自行判断 |
| 订阅额度 | 现为软帽；硬帽才在订阅事务里预留 | Redis 用户余额 hold |

Redis hold 继续只做**准入锁**。Postgres intent 才是**恢复源**。`SqlUsageStore` 继续是 debit+usage+订阅累计的唯一 SQL 编排者。

## 神圣契约

- Hold / Settle / Release；Redis 预留只在 SQL commit **之后**清。
- Partial debit：余额不为负；shortfall 记在 `balance_logs`；Settle 不返回 `InsufficientBalance`。
- 网关自生成 `request_id`，不用客户端 `X-Trace-Id` 当结算键。
- `HOLD_SCRIPT` / `GET_BALANCE_SCRIPT` 保持 float `GET` 编码（多二进制切过合同）。
- 迁移：`IF NOT EXISTS`，禁止 `DROP`/`TRUNCATE`。
- 唯一索引学 `0008`：必须 `type` + 前缀分区。禁止给 settle 流水的 `reference` 全局唯一（一笔 shortfall 会写两行同 reference 的 settle）。
- 前端零改动。

## 草稿分歧与根代理裁决

| 分歧 | 最少切片 | 风险优先 | 接缝质量 | 裁决 |
|---|---|---|---|---|
| A 与 B 是否同一刀 | 合并，一张 intent 表 | 先 unique usage 再 intent | `settlement_claims` + 另表 pending | **一张 `settlement_intents`，两刀**：01 在 settle 时 CAS（关 A），03 在 hold 时写 pending 并改扫描（关 B） |
| 是否先写表征测试 | 否 | 是，红测才能动结构 | 否 | **要**（00）。A0b 若在 READ COMMITTED 下复现不了，停止加唯一“以防万一” |
| `skip_if_already_logged` | 可留作日志 | 热路径改 true + 排除 failed | **删掉** | **01 删除该标志**。第二个检查 = 第二个主人 |
| 控制面兑换/订阅 | 并进支付那一刀 | 支付先，兑换后 | 支付先，API 复用 | **02 只做支付 + `credit_tx`；05 复用** |
| 硬额度 | 不做 | 产品决定 | 产品决定 | **默认保持软额度**；06 先写合同 |

## 已知未知

- 生产是否已有重复的成功 `usage_logs.request_id`（01 加可选 partial unique 前必须扫描）。
- 是否已有 `paid` 但无 `payment_order:{id}` 的订单（02 的修复分支要先 `COUNT`）。
- 产品要不要打开自动补账：默认关着时 B 是运营吞账单；打开且 TTL=3600s 时 B 会变成 A。
- ~~OpenAI/Claude 的 cached/reasoning 价是替代价还是附加价~~ —— **已由厂商一手文档判定为替代价**，见 slices/06 的落地记录。
- Anthropic 的 `cache_creation`（1.25×–2× input）与 `cache_read`（0.1× input）需要一个独立价目列，
  否则缓存写永远按缓存读价少收。
- `gw-provider::usage::parse_openai_usage` 不认 Responses API 的 `input_tokens`/`output_tokens`，
  `/v1/responses` 因此落到 fallback 计费（四列数字被掩盖，不是缓存那件事）。

## 防火墙

- 不要新建计费 crate。
- 面板不得在已提交的状态翻转后再调独立 `Ledger::credit`。
- 面板不得读写 Redis hold/balance，不得插入 intent。
- 不要把额度公式写进余额 Lua。
- 不要用 perfkit 的 NullLedger 火焰图验收这组修复。
