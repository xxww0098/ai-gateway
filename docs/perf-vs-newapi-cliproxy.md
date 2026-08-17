# 热路径对照：AI-GateWay vs NewAPI vs CLIProxyAPI

对照的是**内核怎么走请求**，不是产品宣传。
AI-GateWay 量过自己的数字；**没有**在同一台机器上跑通 NewAPI / CLIProxyAPI 的二进制，
因此文中**没有**它们的 rps / p50。缺的数字就写「没量」，不编。

对照源码（只读 `gh api` / raw，未整仓 clone）：

| 项目 | 语言 | 读到的 `main` | 提交时间（CST） | 星标（读档日） |
| --- | --- | --- | --- | ---: |
| AI-GateWay（仓库 `xxww0098/ai-gateway`） | Rust / axum | `814af96`（#1–#4 已合） | 2026-08-17 14:36 | 0 |
| [QuantumNous/new-api](https://github.com/QuantumNous/new-api) | Go / Gin | `e2c7aa7b102c` | 2026-08-15 14:18 | 45 320 |
| [router-for-me/CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) | Go / Gin | `856ddd8df746` | 2026-08-17 12:51 | 47 507 |

产品名 **AI-GateWay**（仓库仍是 `xxww0098/ai-gateway`）。API Key 前缀是 `agw-`。

读档日：2026-08-17。#5（一元 settle 离开请求路径）当时仍 **open、未合**，
所以下面凡写「main 上的一元」都按 **settle 仍在请求路径上** 说。

相关图与复现：[`hotpath-flamegraph.md`](hotpath-flamegraph.md)、
[`scripts/perf/hotpath-flamegraph.sh`](../scripts/perf/hotpath-flamegraph.sh)。
最新 load JSON 用 [`scripts/perf/compare-notes.sh`](../scripts/perf/compare-notes.sh) 重打。

---

## 0. 一页结论

**我们量到的（本机 mock 上游 + in-memory `NullLedger`，没有 Redis RTT）：**

- 一元 1 KiB→2 KiB：38 727 rps，p50 205 µs / p99 313 µs（2026-08-17 14:14 CST）
- SSE 满速 32×256 B：294 rps，约 195 000 chunk/s；TTFB p50 389 µs；chunk 间隔 p50 0.10 µs
- 火焰图上内核仍是一层 `kernel::layer`；hold 占一元样本 61%；流式 idle 只在 `Pending` 时武装

**相对它们，架构上我们该更快的地方（未用它们的二进制验证）：**

1. 热路径层数：我们一层状态机；NewAPI 是 CORS → 解压 → body 缓存 → stats → TokenAuth → 限流 → Distribute → `Relay()` 编排。
2. 预扣往返：我们 hold + EXPIRE 收进 **一条 Redis Lua**（#3）；NewAPI 预扣是「令牌额度写 + 资金来源写」，信任额度够高时才整段跳过。
3. 流式：我们两层 `Stream` + 可复用 `Sleep`；NewAPI 是 scanner goroutine + `dataChan` + 写锁 + 空闲 `ticker.Reset` + 可选 ping goroutine；CLIProxyAPI 是 `ExecuteStream` 的 channel + 每 chunk `fmt.Fprintf`。
4. 流式结算：我们 `StreamSettler` 已离开请求路径；NewAPI 的 `PostTextConsumeQuota` / `SettleBilling` 在 `DoResponse` **返回之后、handler 返回之前**同步跑。

**它们现在明显更好的地方（不是性能）：**

- 供应商覆盖、协议互转、OAuth / CLI 登录面、运营面板、多租户额度表达式、任务类上游（MJ / Suno / 视频）。
- 成熟度：两家都是 4 万+ star、线上跑了很久；我们还在收热路径。
- CLIProxyAPI **热路径上没有 Hold/Settle**。比的是「转发 + 翻译」，不是「带账本的网关」。把它的延迟跟我们的 205 µs 比，是在比两种产品。

**下一刀要能在对照里看见，而不是只在注释里好看：** 合 #5（一元 settle 离路径）；
给 harness 接真 Redis，把「1 次 hold RTT」从架构主张变成数字；
别把 NewAPI 那种每行 `ticker.Reset` / 每 chunk channel 搬回来。

---

## 1. 口径：什么是苹果，什么不是

### 1.1 我们量了什么

装置是 [`scripts/perf/hotpath-flamegraph.sh`](../scripts/perf/hotpath-flamegraph.sh)：
真 `gw_proxy::router()`（`PERF_MODE=full`，一层 `kernel::layer`）打本地
`mock-upstream`。不是生产流量，不是进程内 `FakeProvider`。

| | 一元 | 流式 |
| --- | --- | --- |
| 时间 | 2026-08-17 14:14 CST | 同轮 |
| 机器 | Linux 6.12，`perf_event_paranoid=2`，用户态 `cycles:u` | 同左 |
| 账本 | in-memory `NullLedger`，**0 Redis RTT** | 同左 |
| 负载 | conc=8，20 s，1 KiB 请求 / 2 KiB 响应 | conc=16，20 s，SSE 32×256 B、`interval_us=0` |
| 采样 | `perf record -F 99 --call-graph dwarf` | `-F 999`（否则 20 s 只有几十个样本） |
| 原始 JSON | `scripts/perf/results/hotpath.load.json` | `hotpath-stream.load.json` |

perfkit 套了 `counting_alloc`。图上的 `malloc` / `cfree` **有一部分是装置税**，
生产二进制没有这层包装。本机当时还在跑别的编译，**绝对值偏悲观，不是产能上限**。

上一轮（hold 收成一条 Lua，13:24 CST）一元是 39 587 rps / p50 200 µs。
本轮 38 727 / 205 µs 是同机噪声，不是一元回退——#4 没动 hold / peek。

更早的 wave 1 基线（[`relay-perf-baseline.md`](relay-perf-baseline.md)，
2026-08-15，Darwin M4 Pro）是另一台机器、另一套交错对照（floor / nomw / full）。
**不要把 65 µs 的串行 p50 和 205 µs 的 conc=8 p50 横着比。**

### 1.2 我们没量什么

- **没有**起 NewAPI。它要 Gin + GORM + SQLite/MySQL/PG + Redis + 前端 + 渠道种子 + 令牌。
  本机没有它的数据库，也没有公平的「只留热路径」启动方式。
- **没有**起 CLIProxyAPI。它要 OAuth 材料（`auths/`）、管理密钥、供应商登录。
  没有登录就打不到它的 executor。
- **没有**写一个「假装是它们热路径」的 Go 玩具然后报 rps。
  那种数字只能证明「Go HTTP 也能很快」，证明不了 NewAPI 的 `Relay()` 或
  CLIProxyAPI 的 translator。

所以：我们的数字只描述**我们自己**在 mock 上游上的形状。
跟它们比的是**层数、分配、往返、流式调度**，不是「我们 3.8 万 rps、他们多少」。

### 1.3 产品本来就不是同一个

| | AI-GateWay | NewAPI | CLIProxyAPI |
| --- | --- | --- | --- |
| 定位 | 带 Hold/Settle 的租户网关 | 聚合分发 + 计费 + 管理后台 | CLI / OAuth 凭证代理 |
| 热路径账本 | Redis Lua hold + PG settle | 令牌额度 + 钱包/订阅预扣，差额结算 | **核心不预扣**；用量在 companion |
| 协议 | 上游 executor + 旁路 usage | 40+ adaptor，请求常 DeepCopy + 再 marshal | translator 注册表，gjson/sjson 改字节 |
| 流式结算 | `StreamSettler` 离路径 | `PostTextConsumeQuota` 在 handler 内同步 | 无等价账本 |

CLIProxyAPI 的 README 写得很清楚：核心不再带用量统计，管理面是另一套。
拿「没有预扣的代理」跟「每请求预扣」比延迟，CLIProxyAPI 少付的是**产品税**，
不是实现技巧。

---

## 2. 我们量到的数字与火焰图帧

### 2.1 一元（2026-08-17 14:14 CST）

`hotpath.load.json`：775 039 请求 / 20.01 s / errors=0 / stalls=0。

| 项 | 值 | 上一轮 13:24 CST |
| --- | ---: | ---: |
| rps | 38 727 | 39 587 |
| p50 / p99 | 205 µs / 313 µs | 200 µs / 309 µs |
| TTFB p50 | 205 µs | （同量级） |
| perf 样本 | 4 322 | 4 263 |

栈上内核帧（任意位置，按采样权重）：

| 占比 | 上一轮 | 帧 |
| ---: | ---: | --- |
| 65.8% | 65.5% | `gw_proxy::kernel::layer` |
| 61.0% | 60.7% | `HoldMiddleware` |
| 40.1% | 41.8% | `OpenAiCompatibleProvider`（打 mock 上游） |
| 3.7% | 4.1% | `routes::unary_response` |

叶子仍是分配器 / `Bytes` 引用计数（含 harness 税）。
`unary_response` 不再 clone usage，只把 `UsageRecord` move 进 `UsageOutcome`，
这张图上几乎看不见。**#5 未合，一元 settle 的账本写入仍在请求路径上**——
只是本轮 harness 的 `NullLedger` 让它便宜到看不出来。

### 2.2 流式（同轮）

`hotpath-stream.load.json`：5 912 请求 / 20.08 s / 每条 33–34 chunk。

| 项 | 值 |
| --- | ---: |
| rps | 294 |
| 约合 chunk/s | ~195 000（33 × 294） |
| 整请求 p50 / p99 | 44.5 ms / 95.6 ms |
| TTFB p50 / p99 | 389 µs / 1.13 ms |
| chunk 间隔 p50 / p99 | 0.10 µs / 44.1 ms |
| errors / stalls | 0 / 0 |
| perf 样本 | 1 503（`-F 999`） |

整请求延迟被「32 个 chunk 的 mock unfold + 网关 poll」拉长，
**不要拿 44.5 ms 跟一元 205 µs 比产能**。TTFB 才是建流成本；
chunk 间隔 p50 = 0.10 µs 说明满速路径上就绪的帧不再被 300 s idle `Sleep` 拖住。

| 占比 | 帧 | 说明 |
| ---: | --- | --- |
| 31.3% | `gw_proxy::kernel::layer` | 每条流仍走鉴权 + 预扣 |
| 28.7% | `HoldMiddleware` | 同上 |
| 19.0% | `OpenAiCompatibleProvider` | 打 mock 上游 |
| 18.6% | `RelayBody` | 代理侧 `Stream`：转发 payload、latch usage |
| 13.7% | `UsageRelay` | provider 侧 `Stream`：probe + 映射 `StreamChunk` |
| 8.4% | `IdleTimeout` | 只在 inner `Pending` 时武装可复用 `Sleep` |
| 5.1% | `StreamUsageProbe` / `observe` | 逐行 usage，驻留 O(单行) |
| 2.9% | `tokio::time` | 不再是每 chunk 一个 `timeout()` |
| 0.7% | `StreamSettler` | 结束时 inline settle；断线走 `TaskTracker` |

#4 之前每 chunk 过三层 `unfold`，并且 `timeout(300s, next)` 在帧已就绪时也建 `Sleep`。
现在 idle 只在 `Pending` 时武装一次、跨 gap 复用。

---

## 3. 内核怎么走：三条路径并排

### 3.1 AI-GateWay（`814af96`，#1–#4）

`kernel.rs` 把 NewAPI 的两件事借过来，产品没搬：一份 `RelayCtx`（对应
`RelayInfo`），加上具名 `Phase`。axum 热路径只挂**一层** `kernel::layer`：
先鉴权、再 `hold.handle`。B1（先鉴权再预扣）是状态机边，不是 `.layer()` 挂载顺序。

```
Received → Authenticated → Inspected → Gated → Reserved
        → Routed → Attempting → Relaying → Settled | Released | StrictHeld
```

预扣（#3）：`HOLD_SCRIPT` 在同一条 Lua 里读余额、清过期 hold、ZADD、HSET、
对 hold key `EXPIRE`。生产上这是 **1 次 Redis RTT**。本轮火焰图用的是
`NullLedger`，这条 RTT **没出现在 205 µs 里**。

Peek（#2）：计费只从唯一一次 `RequestSpec` 派生 `model` / `stream` / `max_tokens`，
不再整棵 `serde_json::Value`。

流式（#4）：`UsageRelay` + `RelayBody` 两层 `poll_next`；`IdleTimeout` 可复用。
`StreamSettler` 在流结束时 inline，客户端断线走 `ProxyState::drain` 的 `TaskTracker`。
**一元**在 main 上仍是 handler `.await settle`（#5 未合）。

### 3.2 NewAPI（`controller/relay.go` + `router/relay-router.go`）

`/v1/chat/completions` 进门之前已经过：

`CORS` → `DecompressRequestMiddleware` → `BodyStorageCleanup` → `StatsMiddleware`
→ `RouteTag` → `SystemPerformanceCheck` → `TokenAuth` → `ModelRequestRateLimit`
→ `Distribute`

`TokenAuth`（`middleware/auth.go`）：`ValidateUserToken` + `GetUserCache` +
IP 限制 + 分组校验，结果写进 Gin context。`Distribute` 为了拿 `model` 会
`GetBodyStorage` + `gjson`（JSON）或整段 `UnmarshalBodyReusable`，再
`CacheGetRandomSatisfiedChannel`，把渠道 key / base URL / 映射塞进 context。

然后 `controller.Relay`：

1. `GetAndValidateRequest` —— 按 `RelayFormat` 反序列化成 DTO
2. `GenRelayInfo` —— 组装 `RelayInfo`（渠道、格式、WS、计费槽）
3. 可选敏感词 + `EstimateRequestToken`（tokenizer；关掉计数时走
   `fastTokenCountMetaForPricing`，避免 `strings.Join` 大块文本）
4. `ModelPriceHelper` → `PreConsumeBilling` → `NewBillingSession`
5. 重试环：选渠道 → `GetBodyStorage` 再绕一圈 body → `TextHelper` / Claude / Gemini / …
6. 失败：`Billing.Refund`（`gopool` 异步）+ 可选违规费

`TextHelper`（`relay/compatible_handler.go`）在非透传时：

`DeepCopy` 请求 → adaptor `ConvertOpenAIRequest` → `common.Marshal` →
`RemoveDisabledFields` → `ApplyParamOverride` → `DoRequest` → `DoResponse` →
**同步** `PostTextConsumeQuota`（算价、`SettleBilling`、`RecordConsumeLog`、
更新用户/渠道已用额度）。

流式（`relay/helper/stream_scanner.go`）：

- `bufio.Scanner` 在独立 goroutine 里按行扫，`ticker.Reset(streamingTimeout)` **每行一次**
- 有效 `data:` 行丢进 `dataChan`（缓冲 10）
- 另一个 goroutine 持 `writeMutex` 调 `dataHandler`，每次写前 `SetWriteDeadline(30s)`
- 可选 ping goroutine（默认 10 s）
- 主循环 `select` 等 ticker / stop / 客户端断开
- handler 返回后才走到上面的 `PostTextConsumeQuota`

预扣本身（`service/billing_session.go`）：信任额度够高且非强制预扣时
`effectiveQuota = 0`，**整段预扣跳过**。否则先 `PreConsumeTokenQuota`（令牌行），
再 `funding.PreConsume`（钱包 `DecreaseUserQuota` 或订阅 delta）。
这是 **两次独立写**，不是一条 Lua。结算时资金来源与令牌额度再分两步提交。
`Refund` 已离路径（`gopool`）；**成功路径的 Settle 没有。**

### 3.3 CLIProxyAPI（`internal/api/server.go` + `openai_handlers.go`）

进门：`GinLogrusLogger` → Recovery → `CPATraceID` → 可选请求日志 → CORS →
Home 心跳门（未订阅则 503）→ 示例 key 安全模式 → 路由上的 `AuthMiddleware`。

`/v1/chat/completions`：

1. `ReadRequestBody` —— 整段 body 进 `[]byte`
2. `gjson` 看 `stream`；必要时 Responses 形态转 Chat Completions（`sjson`）
3. 非流式：`ExecuteWithAuthManager`（选凭证 + executor + translator）→
   `c.Writer.Write(resp)`；期间可挂 non-stream keep-alive
4. 流式：`ExecuteStreamWithAuthManager` 得到 `dataChan` / `errChan`，
   先 peek 第一块再提交 SSE 头，之后每块 `fmt.Fprintf("data: %s\n\n")` + `Flush`

没有 Hold，没有 Settle，没有 Redis 预扣。热路径上的重活是
**凭证选择 + 协议翻译 + channel 转发**。OAuth（Codex / Claude / Grok / Gemini /
Antigravity）和 Home 调度在旁边，不进账本。

---

## 4. 我们该更快的地方（架构，不是测出来的倍数）

这些是「同一类工作，我们少做了几步」。没有它们的 rps，不能写成「快 N%」。

### 4.1 热路径层数

NewAPI 在 `Relay()` 之前已经过 8 段 Gin 中间件，且 `TokenAuth` / `Distribute`
各自会碰缓存或 body。我们把 access → hold 收进一个函数体，handler 只看见
`RelayCtx`。少几次 context map 写入、少一次「中间件返回再进入下一层」的约定。

CLIProxyAPI 的中间件比 NewAPI 薄，但每条请求仍 `ReadRequestBody` 成拥有的
`[]byte`。我们 peek 之后 body 是 `Bytes` 引用计数（#2），dispatcher 不再读第二遍。

### 4.2 分配与拷贝

NewAPI 非透传路径：`DeepCopy` DTO → 转换 → `Marshal` → 再改字段。
大 body 时这是整棵树的分配。我们 #2 已经把 peek 从整棵 serde 拿掉；
OpenAI 兼容上游走 `Bytes`，不在热路径上再造一份 JSON 树。

CLIProxyAPI 用 gjson/sjson 改字节，比 `encoding/json` 往返便宜，
但仍是「先读成 `[]byte`，再按字段 set」。流式每块 `fmt.Fprintf` 会再格式化一次。

我们自己的旧账还在：wave 1 基线里 full 比 floor 多 **149 次 / +26.3 KB**
（1 KiB→2 KiB）。那是 2026-08-15 另一台机器的数，只能说明「分配仍是墙」，
不能拿来跟两家 Go 项目比绝对值。

### 4.3 Hold RTT

生产上我们预扣是 **1 次 Redis EVAL**（余额 + 入队 + EXPIRE）。
NewAPI 在未走信任旁路时是令牌写 + 资金写，常见还要先 `GetUserQuota` /
`HasActiveUserSubscription`。信任旁路（`GetTrustQuota()` 且余额够）会
**零预扣**——高余额用户那条路上，NewAPI 反而比我们少一次 Redis。
这是产品策略，不是我们能用火焰图赢的点；对照时要分开说。

CLIProxyAPI 没有这趟。比 hold RTT 它直接弃权。

本轮数字里 **没有 Redis**。主张「1 RTT」目前只是读 `scripts.rs` 的结论，
还不是 harness 差。

### 4.4 流式 idle 与每 chunk 成本

NewAPI：`ticker.Reset` 每行一次；就绪的 SSE 行也要过 channel + mutex。
空闲超时是「从最后一行重新计时」，语义对，但热路径上多一次 timer 武装。
另有 ping goroutine 和 30 s write deadline——这些是正确性，不是免费的。

CLIProxyAPI：chunk 走 Go channel，handler 侧 `Fprintf` + `Flush`。
没有 300 s idle `Sleep` 包在 `next` 外面，但也没有我们这种「Pending 才武装」的
`Stream` 状态机。

我们 #4 之后：`IdleTimeout` 占流式样本 8.4%，`tokio::time` 2.9%，
chunk 间隔 p50 = 0.10 µs。这是**对自己改之前**的胜利。
对它们，只能说：同样做「满速转发 + 旁路 usage」，我们没有每 chunk 建 `Sleep`，
也没有每行 `Reset` ticker。

### 4.5 流式结算离路径

NewAPI 的成功路径：流结束后 `PostTextConsumeQuota` 算价（`shopspring/decimal`、
可选 `billingexpr`）→ `SettleBilling` → `RecordConsumeLog` → 更新已用额度，
**挡住 handler 返回**。失败退款才进 `gopool`。

我们流式已经把写入交给 `StreamSettler`（图上 0.7%）。客户端先拿到 `[DONE]`。
一元还没做到这一步（#5）。

---

## 5. 它们更好的地方（承认，不找补）

### 5.1 供应商与协议面

NewAPI 的 `GetAdaptor` 覆盖 OpenAI / Claude / Gemini / Azure / Bedrock / 阿里 /
以及图像、音频、embeddings、rerank、Responses、Realtime WS、Midjourney、Suno、
视频任务。`TextHelper` 里那套 DeepCopy + Convert 换来的是「一个入口吃 40 家」。

CLIProxyAPI 的卖点是 **CLI OAuth**：Codex、Claude Code、Grok Build、Gemini、
Antigravity，再加 OpenAI/Claude/Gemini/Codex 互转和 embeddable SDK。
我们现在的 executor 面比这两家都窄；OAuth 面另一条分支在做，不在本 PR。

### 5.2 计费产品，不是计费热路径

NewAPI 有信任额度旁路、订阅/钱包回退、阶梯表达式、tool surcharge、
cache / audio / image 分项、额度饱和审计。我们有 Hold/Settle/Release、
欠款门、strict usage、partial debit——语义不同，功能密度他们高。

CLIProxyAPI 选择**不做**这套，把用量交给 Management Center。对「只要转发」的用户，
这是更轻的产品，不是更差的实现。

### 5.3 运营与成熟度

NewAPI：SQLite / MySQL / PG 三套、React 管理台、i18n、渠道亲和、自动封禁、
playground、WebAuthn。CLIProxyAPI：热重载、TUI、管理 API、插件、Home 心跳、
多账号 round-robin。两家都是 2023–2025 起盘、4 万+ star。
我们还在把热路径收成能讲清楚的状态机。线上事故、渠道奇葩、tokenizer 边界，
他们付过的学费我们还没付。

### 5.4 我们故意没做、他们已经做了的正确性成本

NewAPI 流式的 ping、write deadline、scanner 缓冲上限、客户端断开立刻关上游
`Body`——这些会在火焰图上留痕，但能少烧上游 token。
CLIProxyAPI 的 keep-alive、stream 先 peek 再提交 SSE 头（避免失败时已经
`text/event-stream`），也是正确性。
我们若为了「赢对照」把这些删掉，数字会好看，产品会变脆。下一刀不该走这条。

---

## 6. 下一刀：怎样才算「对照里看得见」

只列合进去之后**能在数字或层数上被看见**的。不列「再写一篇文档」。

| 优先级 | 刀 | 为什么对对照有用 | 现在的状态 |
| --- | --- | --- | --- |
| 1 | 合 #5：一元 settle 离路径 | 真 Redis/PG 时，一元 p50 会少掉账本写入；现在 `NullLedger` 上看不见 | PR open，main 未合 |
| 2 | harness 接真 Redis（一条 Lua） | 把「1 hold RTT」从读脚本变成差。没有它，对 NewAPI 的预扣主张是纸面的 | 本轮仍是 `NullLedger` |
| 3 | 不要把 NewAPI 的 stream 形状搬回来 | 每行 `ticker.Reset`、每 chunk channel、写锁，会直接出现在流式火焰图上 | #4 已避开 |
| 4 | 继续削 hold 里的分配，而不是再拆中间件 | 一元图上 `HoldMiddleware` 仍 61%；再挂一层 Gin 式中间件是在学他们的税 | #1 已收成一层 |
| 5 | 透传 / OpenAI 兼容路径保持「不 DeepCopy、不整树 marshal」 | 这是 NewAPI `TextHelper` 非透传路径最贵的固定税；我们一旦为了「多一家供应商」走回那条，对照就没了 | #2 已钉 peek |
| 6 | 同一 mock 上游、同一 loadgen，真的拉起两家 Go 二进制 | 唯一能写 rps 对照的办法。需要：NewAPI 的最小令牌+渠道种子、CLIProxyAPI 的无 OAuth 直连上游模式。本 PR 没做 | 未做，缺数字 |

#5 合进去之前，对外只能说：「流式 settle 已离路径；一元还没有。」
不要把 PR 标题写成已经发生的事。

---

## 7. 复现与重打数字

```bash
# 我们自己的火焰图（会覆盖 docs/*.svg 与 results/*.load.json）
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=/tmp/cargo-hotpath-perfkit
DURATION=20 ./scripts/perf/hotpath-flamegraph.sh

# 只重打最新 load JSON，不重新采样
./scripts/perf/compare-notes.sh
```

`hotpath-flamegraph.sh` 的行为本 PR **未改**。
`compare-notes.sh` 只读 `scripts/perf/results/hotpath.load.json` 与
`hotpath-stream.load.json`，没有就说没有。
