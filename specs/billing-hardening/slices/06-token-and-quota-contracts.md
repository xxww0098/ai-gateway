# 06 — 额度、token、价格合同

本切片默认**不改钱的并发语义**，除非产品书面要求硬额度。00 的 D0 作为软额度规格保留：两路并发可以超过印刷帽。

## 合同

三件独立的事，禁止从函数名推断：

1. **额度：软（现状）vs 硬。** 软 = 周期用量是已结算花费，并发收尾可以超过印刷帽，下一笔 admit 在结算后拦住。硬 = admit 时 `settled + in_flight_H ≤ L`。
2. **价格：结算时现价（现状）vs admit 快照。** 长流期间改价可以让账单与 hold 不一致。
3. **token 列：四列相加（现状计算器）vs 按供应商互斥。** OpenAI 的 C⊂I、R⊂O 在替代价下会把缓存/推理卖两次。Google 只在计价前 `O:=O+R; R:=0`，日志仍保留上游字段。

建议的诚实默认（未接到相反产品指令前）：

- 额度保持软；在 `SubscriptionQuotaStore` 注释与 402 文案写明。
- 价格保持结算现价；hold 是估计。
- **token 互斥按供应商落到 `billable_tokens` 旁的显式转换**——这是静默多收费，不是“优化”。落地前用下面金样确认价目定义。

## 接缝（仅当做 token 互斥）

`gw-proxy::usage::billable_tokens` / 每供应商 `to_billable(raw) -> TokenUsage`。计算器继续四列相加，但输入已是可计费列。日志继续存**原始**列。

金样（数字可变，关系不能变）：

| 供应商 | 原始 | 计费（若 C/R 为替代价） |
|---|---|---|
| OpenAI | I=120 C=40 O=80 R=30 | I'=80 C=40 O'=50 R=30 |
| Gemini/Vertex | 已有 fold：O 含 thoughts，R=0 | 保持；另处理 C⊂I |
| Claude | cache write+read 已打进 C；I 与 cache 通常互斥 | 不要再减；若要 Anthropic 两个缓存价再拆列 |

目录展示四零 vs 结算 fallback：本切片**不改前端**。最多在 API 已有字段里区分“目录价”与“缺价 fallback”；没有字段就只在后端注释/文档写清，不要违反前端零改动。

## 硬额度（仅产品确认后）

不要把额度公式写进余额 Lua。在订阅行同一事务：

```text
BEGIN
  SELECT subscriptions FOR UPDATE
  rotate
  if used + reserved + H > L → ROLLBACK 402
  reserved += H
  INSERT quota_hold(request_id, H, expires_at)
COMMIT
然后现有 Redis hold_gated
```

结算/失败调差；过期用 SQL `quota_holds.expires_at` 扫，与 Redis TTL 对齐。`evaluate_quota` / `rotate_counters` 仍是唯一算术。

两连接：limit=10 used=9 H=1 → 一路 Reserved，一路 Exceeded；结算后 used=10 不是 11。

## 人能跑什么

软额度：D0 保持绿且注明“这是合同”。  
token：`usage/tests.rs` 加 OpenAI/Claude 金样，现有 Google fold 测试保持。  
硬额度：产品点头后再加并发 402。

## 验证

- 未确认硬帽则不得加 `quota_holds`。
- 未确认替代价则不得改 OpenAI 减法（改错会**少收**）。把“价目是替代还是附加”列为 README 已知未知，直到有人回答。
- `f64` vs SQL `numeric` **不是**本切片门闩。硬帽若做，比较放在 SQL numeric，H 在 admit 时四舍五入到 micro-USD。

## 实现者可自行决定

`to_billable` 放 proxy 还是 provider。不要在 provider 解析阶段 fold（除已有 Google 计价 fold），以免日志与上游对不上。

## 必须保持绿色

`evaluate_quota` 等于上限可通过；计算器四列乘法测试；Google fold 测试。
---

## 落地记录（2026-09-10）

### token 互斥：已实现

「替代价 vs 附加价」这个已知未知**已经有一手答案**：是**替代价**。

| 事实 | 一手来源 |
| --- | --- |
| OpenAI `cached_tokens` 是 `prompt_tokens` 的明细，官方指南直接给减法 `ordinaryInputTokens = inputTokens - cachedTokens - cacheWriteTokens` | https://developers.openai.com/api/docs/guides/prompt-caching |
| OpenAI reasoning「are billed as output tokens」，定价表只有 input / cached input / output 三档 | https://developers.openai.com/api/docs/guides/reasoning |
| Google `promptTokenCount` "includes the number of tokens in the cached content"；`totalTokenCount = prompt + thoughts + candidates` | https://ai.google.dev/api/generate-content#UsageMetadata |
| Anthropic "Total input tokens … is the summation of `input_tokens`, `cache_creation_input_tokens`, and `cache_read_input_tokens`" | https://platform.claude.com/docs/en/api/messages |

落地位置：`gw-proxy/src/usage.rs` 的 `nesting` / `billable_tokens`。四列互斥只作用于
**计价**；`usage_logs` 继续写上游原话。

金样（与上面表格的语义一致，数字来自 seed 价）：

| 供应商 | 原始 | 计价视图 |
| --- | --- | --- |
| OpenAI / Codex | I=120 C=40 O=80 R=30 | I=80 C=40 O=50 R=30 |
| Gemini / Vertex | I=1000 C=800 O=100 R=400 | I=200 C=800 O=500 R=0 |
| Claude | I=100 C=910 O=200 R=0 | 原样 |

不认识的上游一律原样计价：少减只是按原价收，多减是真金白银少收，方向不对称。

### 子费率留空 = 按基准费率（本轮新增，超出原切片）

`cached_input_price_per1_m` 与 `reasoning_price_per1_m` 的建表默认值都是 0。
互斥之后如果继续把 0 读成「免费」，缓存与思考 token 会直接白送；所以
`Calculator::compute` 现在把 0 读成「没配」，回落到 input / output 基准费率。

这让「留空」与「显式填成基准价」完全等价，也是唯一自洽的默认（见 `PRODUCT.md`
第三条产品原则）。**只影响子费率留空的价目行**，配了折扣价的行行为不变。

### 仍未做

- **价格快照**：仍是结算时现价。
- **Anthropic 两个缓存价**：`cache_creation`（1.25×/2× input）与 `cache_read`
  （0.1× input）被 `gw-provider` 并成一列 `cached`，只能按 `cached_input_price`
  计 —— 缓存写被系统性少收约 92%。修它需要第五列价目（`cache_write_price_per1_m`）
  或在解析层拆列，属产品或 schema 决策。
- **硬额度**：未做，默认仍是软额度。
