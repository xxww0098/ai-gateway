# gw-relay 透传保真度审计（只读）

审计对象：`crates/gw-proxy` + `crates/gw-provider`，提交基线 `1acdc49`。
审计范围：入站请求字节 → 上游请求字节 → 上游响应字节 → 回给客户端的字节，这条链路上的每一次
拷贝、重编码、丢弃、改写。

本文不改任何代码。所有结论都带 `file:line` 证据。

> **行号基线**：本文所有行号对应**提交 `1acdc49`**，不是工作区。
> 审计期间有并行 worker 正在同一个 worktree 上剥离旧品牌引用，工作区行号一直在漂，
> 所以证据一律以不可变的提交为准 —— 核对时用 `git show 1acdc49:<path>`。
> 部分条目额外给了符号名（函数 / 常量 / 字段），符号名在重构后仍然可定位。

---

## 0. 一句话结论

**流式响应体的字节保真度是好的**（没有任何 SSE 解析器介入，`event:` / `id:` / `retry:` /
注释行 / 多行 `data` 全部原样过）。**其余四条边全是漏的**：

- 请求体有 **1 MiB 硬上限**，超了直接 413；
- 请求体在 OpenAI/Codex 流式路径上被 **整体 JSON round-trip 重写**（键序、数字格式、转义全变）；
- `/v1/responses` 被 **发到了 `/v1/chat/completions`**，协议根本对不上；
- 上游 **4xx/5xx 的响应头全部丢失**（`retry-after`、`x-ratelimit-*`、`request-id` 全没了），
  错误体被转成 `String` 再重新贴上 `content-type: application/json`；
- 成功响应的 **多值 header 被折叠成最后一个**；
- 流式中途失败对客户端表现为 **一次干净的 EOF**，客户端无法区分「答完了」和「被截断了」。

---

## 1. 透传保真度缺陷表

按严重度排序。「客户端可观测后果」一律写到具体客户端的具体行为。

### S1 —— 破坏功能，必须在 gw-relay 里根除

| # | 缺陷 | 证据 file:line | 客户端可观测后果 | 严重度 | gw-relay 里应如何避免 |
|---|---|---|---|---|---|
| 1 | `/v1/responses` 的请求体被原样 POST 到上游的 `/v1/chat/completions`。路由把它归进 `ApiFamily::OpenAi`，dispatch 选中 `OpenAiCompatibleProvider`，而该 provider 只会构造 chat/completions 端点 | 路由 `crates/gw-proxy/src/routes.rs:684-688`；provider 选择 `routes.rs:81-87`；端点构造 `crates/gw-provider/src/openai.rs:103` → `crates/gw-provider/src/common.rs:280-313`（`chat_completions_endpoint`） | OpenAI Python SDK 调 `client.responses.create(model="gpt-5", input=[...])`，网关把 Responses 形状的 body 发到 chat/completions，OpenAI 返回 `400 {"error":{"message":"Unrecognized request argument supplied: input","type":"invalid_request_error"}}`。三个保留入口之一 **100% 不可用**。再叠加缺陷 #4：流式时还会被塞进 `stream_options`，Responses API 同样拒收 | **S1** | 端点不能由 provider 猜。`RelayRequest` 携带**入站 path**，上游 URL = `upstream_origin + inbound_path`（Anthropic/OpenAI 的路径本来就同名）。协议翻译只在真需要转义的 provider（gemini/vertex）上发生，且是显式的一次 `Translator`，不是 provider 内部的隐式改写 |
| 2 | 请求体 1 MiB 硬上限。hold 中间件把 body 读进内存做计费 peek，超限直接 413；handler 的 fallback 路径用同一个上限 | 常量 `crates/gw-proxy/src/hold.rs:45`；413 分支 `hold.rs:851-864`（`to_bytes(body, HOLD_REQUEST_BODY_LIMIT)` 超限返回 `Err`）；handler 侧同上限 `crates/gw-proxy/src/routes.rs:624-629` | Claude Code 带着完整会话历史 + 一次 `Read` 出来的大文件 + 一张截图发 `/v1/messages`，body 轻松过 1 MiB。客户端拿到 `413 {"error":"Payload Too Large","message":"request body exceeds the billing pre-flight limit"}`，Claude Code 直接报 `API Error: 413` 并中断本轮，用户唯一的出路是 `/compact`。这条不是"边缘 case"，是长会话的必然终点 | **S1** | 计费 peek 不该决定转发能力。`RelayBody` 分两态：`Buffered(Bytes)`（体积 ≤ peek 阈值，peek 与转发共用同一块内存）与 `Streaming(BoxBody)`（超阈值，**边收边转**，不缓冲、无上限）。超阈值时 model/stream 从 header 或 URL 兜底，拿不到就用保守估算 hold —— **计费降级，转发不降级** |
| 3 | 上游 4xx/5xx 的响应头**全部丢弃**。`ProviderError::Upstream` 只带 `status` 和 `body: String`，header 在 provider 里被丢在地上；`DispatchError::into_response` 只写 status + body + 一个自造的 `content-type` | 类型定义 `crates/gw-provider/src/types.rs:143-152`；丢弃点 `crates/gw-provider/src/openai.rs:183-188`、`crates/gw-provider/src/claude.rs:370-372` → `claude.rs:638-643`、`gemini.rs:263-265`、`codex.rs:372-377`、`vertex.rs:815-817`；回写点 `crates/gw-proxy/src/error.rs:193-216`；映射点 `routes.rs:308-316` | Anthropic 529 `overloaded_error` 带 `retry-after: 12`，网关吃掉。Anthropic Python SDK 的 `_calculate_retry_timeout()` 优先读 `retry-after` / `retry-after-ms`，读不到就退回 `0.5 * 2^n` 抖动退避 —— Claude Code 于是在上游明确要求等 12 秒时 0.5 秒就重试，把限流放大成雪崩。同理 OpenAI SDK 丢掉 `x-ratelimit-reset-requests`。`request-id` / `x-request-id` 一并消失，用户拿不到任何可以向 Anthropic 报障的 ID | **S1** | **删掉 `ProviderError::Upstream` 这个概念**。上游的 4xx/5xx 不是 gw-relay 的错误，它就是一个 `RelayResponse`，和 200 走完全相同的回写路径。`Result::Err` 只留给"连不上上游"（DNS / TCP / TLS / 超时）。failover 的判定读 `RelayResponse::status`，不需要把响应转成错误类型 |

### S2 —— 客户端行为已经不对，但不是立刻可见

| # | 缺陷 | 证据 file:line | 客户端可观测后果 | 严重度 | gw-relay 里应如何避免 |
|---|---|---|---|---|---|
| 4 | `ensure_include_usage()` 对 OpenAI/Codex 的**每一个流式请求**做整体 JSON round-trip：`from_slice` → `Value` → `to_vec`。`serde_json` 未开 `preserve_order`，`Map` 就是 `BTreeMap` | 实现 `crates/gw-provider/src/common.rs:248-268`（`ensure_include_usage`）；调用点 `crates/gw-provider/src/openai.rs:109-113`、`crates/gw-provider/src/codex.rs:193-197` | 见下方 §2 的实测字节 diff。最具体的一条：客户端显式写了 `stream_options:{"include_usage":false}`，被静默翻成 `true`（`common.rs:262` 无条件 `insert`），于是 SSE 末尾多出一帧 `data: {"choices":[],"usage":{...}}`。任何手写 `chunk["choices"][0]` 的客户端在这一帧上抛 `IndexError` / `undefined is not an object`。其次：大整数 `"seed": 12345678901234567890` 变成 `1.2345678901234568e+22`，上游按浮点收 seed，可复现性没了 | **S2** | 计费不许改请求体的**结构**。两条路，都比现在好：(a) **定点字节插入** —— 顶层扫一遍有没有 `"stream_options"`，没有就在最外层 `{` 之后插入 `"stream_options":{"include_usage":true},`，其余字节 100% 原样，用 `Frame` 序列 `[prefix, original_slice]` 拼，一次拷贝都不用；(b) **尊重客户端**，不塞，落 fallback 计费，用 `billing.force_include_usage` 开关兜底。默认走 (a)，配置可切 (b) |
| 5 | 客户端的 `accept-encoding` 被原样转发给上游，而 reqwest **没有开 gzip/brotli/deflate/zstd feature** —— 网关既不解压也不认识压缩体 | 黑名单里没有 `accept-encoding`：`crates/gw-provider/src/types.rs:35-50`；转发点 `types.rs:59-68`；reqwest feature 集 `crates/gw-provider/Cargo.toml:20`（`json, stream, rustls-tls, multipart, charset, http2` —— 无任何压缩 feature）；usage 解析点 `openai.rs:189`、`claude.rs:376`、`gemini.rs:269` | **字节保真度反而是对的**（压缩体原样回、`content-encoding` 原样回、客户端能解），坏的是计费：OpenAI Python SDK 底下的 httpx 默认发 `accept-encoding: gzip, deflate`，Anthropic SDK 同理。上游（两家都在 Cloudflare 后面）按需 gzip，`parse_openai_usage(&body)` 拿到的是 gzip 魔数 `1f 8b`，必然 `None` → `usage_present=false` → 非严格模式落 `SettlementPlan::Settle{fallback: missing_upstream_usage}`（`crates/gw-proxy/src/usage.rs:121-145`），**每一次非流式请求都按估算而不是真实 token 计费**；严格模式下更糟：`StrictSkip`，请求成功但 `usage_logs.failed = true` 且完全不结算 | **S2** | 旁路统计与压缩是互斥的，必须显式选一边。推荐：**请求方向把 `accept-encoding` 收敛成 `identity`**（写进 §3 的"必须改"清单），上游就不会压，probe 拿到明文。想保留客户端侧压缩的部署，用配置打开"probe 挂解压 tee"这条更贵的路径，并接受它的 CPU 成本。**不要**留成现在这种"看起来能跑、计费静默失真"的状态 |
| 6 | 流式中途失败（idle timeout 或 transport error）对客户端表现为**一次干净的 EOF**。`Body::from_stream` 的错误类型是 `Infallible`，`StreamChunk::Error` 只写了一行 `tracing::warn!` 就被吞掉 | 流体构造 `crates/gw-proxy/src/routes.rs:405-437`（`Ok::<Bytes, std::convert::Infallible>`，`routes.rs:412-415`）；吞错点 `routes.rs:422-427`；错误来源 `crates/gw-provider/src/common.rs:470-483`；idle 看门狗 `common.rs:337-361`（`with_stream_idle_timeout`），默认 300s `common.rs:41`（`DEFAULT_STREAM_IDLE_TIMEOUT`） | Claude Code 迭代 `/v1/messages` 的 SSE，上游卡死 5 分钟后网关静默收尾：HTTP 层是**正常结束**（h1 发终止 chunk / h2 发 END_STREAM），SSE 里没有 `message_stop`。Claude Code 拿到一个截断的助手消息，**不报错、不重试**，用户看到答案说到一半就停了，以为模型就是这么回的。OpenAI SDK 同理（没有 `[DONE]` 也不抛）。顺带一个计费漏洞：`s.failed = true` 让 `plan_settlement` 走 `Release`（`crates/gw-proxy/src/usage.rs:122-126`），**已经产出的 token 全额退款** | **S2** | 中继必须能表达"我失败了"。`RelayResponseBody` 的 stream item 类型是 `Result<Frame<Bytes>, RelayError>`，**不是** `Infallible`。中途失败就把 `Err` 交给 hyper：h2 发 `RST_STREAM`，h1 直接掐连接（不发终止 chunk）—— 这是 SSE 唯一能让客户端察觉截断的手段。同时在 SSE 语义层补发一帧 `event: error`（Anthropic 的流式错误就长这样），两条都做 |
| 7 | 响应的**多值 header 被折叠成最后一个**。`HeaderMap::iter()` 对同名多值会 yield 多次，循环体用的是 `insert`（覆盖全部旧值）而不是 `append` | 流式 `crates/gw-proxy/src/routes.rs:444-449`；非流式 `routes.rs:557-562`。对照组：出站方向是对的，用 `keys()` + `get_all()` + `append`（`crates/gw-provider/src/types.rs:59-68`） | OpenAI 经 Cloudflare 返回两条 `set-cookie`（`__cf_bm=...` 与 `_cfuvid=...`），客户端只收到最后一条。Node 的 `openai` SDK 用的 undici 会把 cookie 存进连接上下文，缺了 `__cf_bm` 之后每个请求都被 Cloudflare 当新会话，bot 分数掉下去就开始吃 403 挑战页 —— 表现为**间歇性 403 而不是稳定失败**，最难排查的那种 | **S2** | 回写用 `for name in src.keys() { for v in src.get_all(name) { dst.append(name.clone(), v.clone()) } }`，和出站方向共用同一个函数。gw-relay 里 header 复制只能有**一处**实现 |
| 8 | 幂等回放**只保留 `Content-Type`**，其余 header 全丢；流式响应永远记成 `truncated` | 只抓 content-type：`crates/gw-proxy/src/hold.rs:446-453`；只写这一个：`hold.rs:499-512`；回放补默认：`crates/gw-proxy/src/idempotency.rs:106-127`；流式判定 `hold.rs:877-884` | 带 `Idempotency-Key` 重放一次成功的非流式请求，回放响应里 `x-request-id`、`openai-processing-ms`、`anthropic-ratelimit-*` 全部消失 —— 与首次响应**不一致**，任何做 request-id 对账的客户端（以及网关自己的排障）在重放上断链。更硬的一条：**流式响应永远进不了缓存**（`hold.rs:881-884` 直接返回 `None`），于是 `truncated = true`（`hold.rs:509-511`），带同一个 `Idempotency-Key` 的重试直接吃 `409 idempotency_replay_unavailable`（`crates/gw-proxy/src/error.rs:106-107`）。OpenAI SDK 把 409 当不可重试错误直接抛给用户 | **S2** | `CachedResponse` 存**完整的 `HeaderMap`**（除 hop-by-hop），不是一个 `Content-Type`。序列化用 `Vec<(String, String)>` 保多值。流式要么不给幂等语义（首次就明确拒 `Idempotency-Key` + SSE 的组合），要么在中继层做 tee 落盘 —— 但**别**留成"缓存了一个不能用的条目，然后拿 409 打客户端的脸" |

### S3 —— 字节确实被改了，后果可控但不该有

| # | 缺陷 | 证据 file:line | 客户端可观测后果 | 严重度 | gw-relay 里应如何避免 |
|---|---|---|---|---|---|
| 9 | query 参数**双重百分号编码**。入站不解码，出站用 `append_pair` 重新编码 | 不解码 `crates/gw-proxy/src/routes.rs:648-657`（注释自称"providers re-encode"，那正是 bug）；重编码 `crates/gw-provider/src/common.rs:305-311`、`crates/gw-provider/src/claude.rs:589-597` | 客户端发 `?tag=a%20b`，上游收到 `?tag=a%2520b`（`%` 被再编码成 `%25`）。`+` 变 `%2B`。任何在 query 上带编码值的集成静默拿到错的值 | **S3** | 原始 query string 是**字节**，直接拼到上游 URL 后面，一次都不要解析。provider 自己要 set 的参数（`alt=sse`）用字节级 append/replace |
| 10 | `expect` 不在出站黑名单里 | `crates/gw-provider/src/types.rs:35-50` | 客户端发 `Expect: 100-continue`，网关把它转给上游。hyper 的 client 不实现 100-continue 协商，只会把 header 发出去并立刻发 body；上游回的 `100 Continue` 被当作信息响应吞掉。多一次 RTT，某些严格的中间设备会直接 417 | **S3** | `expect` 是逐跳的连接控制头，代理必须自己消费。加进黑名单 |
| 11 | `truncate_failure_body()` 是**死代码** —— 全仓无生产调用点，错误体实际**完全不截断** | 定义 `crates/gw-provider/src/common.rs:146-152`（`truncate_failure_body`）；调用点只有测试 `common_tests.rs:153,157,158`；真实路径显式不截断并写了注释 `crates/gw-provider/src/claude.rs:633-643` | 上游返回一个 500 KB 的 Cloudflare HTML 错误页，网关整页 `from_utf8_lossy` 成 `String`，进 `DispatchError::Upstream`，再整页回给客户端并贴上 `content-type: application/json`。Anthropic Python SDK 在 `APIStatusError` 构造里 `response.json()` 失败，异常信息变成一大段 HTML | **S3** | 中继不看错误体，也不持有它 —— 它就是一段 `Bytes`，原样回。要做日志采样就在**旁路** clip，clip 的结果只进日志，不进回写路径。同时把 §3 里"顺手改了"的 `content-type: application/json` 一起撤掉 |
| 12 | 非 UTF-8 / 二进制错误体被 `from_utf8_lossy` 有损替换成 U+FFFD | `crates/gw-provider/src/openai.rs:186`、`claude.rs:641`、`gemini.rs:264`、`codex.rs:375`、`vertex.rs:816`；`routes.rs:260` 还有一处（可重试状态的分支） | 上游返回 protobuf / gzip / 截断的 UTF-8 错误体，客户端收到的是被 `�` 污染过的字节，长度也变了。虽然罕见，但这是**字节契约上的原则性破口**：只要类型是 `String`，就不存在保真度可言 | **S3** | 错误体永远是 `Bytes`，不经过 `String`。这条随缺陷 #3 一起解决 |
| 13 | 请求体的多余全量拷贝，且**每次 failover 重复付一遍** | `routes.rs:217` `inbound.body.to_vec()`（在 `while tried.len() < MAX_UPSTREAM_ATTEMPTS` 的循环体内，`routes.rs:208`）；`routes.rs:220` `headers.clone()` 同理；provider 侧再拷一次：`claude.rs:281`、`gemini.rs:179`、`vertex.rs:556` 的 `req.payload.clone()`，`openai.rs:131`、`codex.rs:215` 的 `payload.into_owned()` | 无直接客户端可见后果，但一个 900 KB 的请求在 3 次 failover 下要 memcpy 约 5.4 MB。加上 §4 的响应侧拷贝，这是纯浪费 | **S3** | 见 §2 的类型契约：`Bytes` 全程 refcount，`clone()` 是 O(1) |
| 14 | 响应体全量拷贝：`bytes().await?.to_vec()` 把 reqwest 已经拿到的 `Bytes` 复制进 `Vec<u8>` | `openai.rs:182`、`claude.rs:369`、`gemini.rs:262`、`codex.rs:371`、`vertex.rs:814`；类型根因 `crates/gw-provider/src/types.rs:74` `body: Vec<u8>` | 同上。注意末端 `Body::from(Vec<u8>)` 反而是零拷贝（`Bytes::from(Vec)` 接管 buffer）—— 唯一那次拷贝就是这个 `to_vec()`，纯属被 `Vec<u8>` 这个字段类型逼出来的 | **S3** | `ProviderResponse::body: Bytes` |
| 15 | 请求体的 JSON 被**解析两遍**（hold 一遍、handler 再一遍），流式还有第三遍（`ensure_include_usage`） | `crates/gw-proxy/src/hold.rs:866` 与 `crates/gw-proxy/src/routes.rs:632` 都调 `parse_body_peek`；第三遍 `common.rs:252` | 同上，纯 CPU 浪费。900 KB 的 body 解析三遍 | **S3** | peek 结果进 `RelayContext`，只解析一次 |
| 16 | `StreamUsageBuffer` 对**每一个 chunk 做全量 memcpy**，外加周期性 memmove | `crates/gw-provider/src/streambuf.rs:83` `self.tail.extend_from_slice(p)`（每字节必拷）；`streambuf.rs:84-87` 超过 `2 × tail_limit` 时 `drain(..drop)` 是一次最多 64 KiB 的 memmove；head 窗口另有一次部分拷贝 `streambuf.rs:77-80`；调用点 `common.rs:464`、`crates/gw-provider/src/vertex.rs:758` | 无直接客户端后果。量化：**每转发 1 字节额外复制约 1 字节**（tail），前 32 KiB 再多复制一份（head），另外每流过 64 KiB 触发一次 ≤64 KiB 的 memmove —— 整体约 2× 全流量的额外内存带宽。峰值常驻 96 KiB × 并发流数 | **S3** | 见 §2 的 `UsageProbe`：tail 用 `VecDeque<Bytes>` 环（只持句柄，`Bytes::clone` 是 refcount，**零字节拷贝**），或者更省的**增量行解析**（SSE 是行式的，跨 chunk 只需缓一个半行）。head 只有 Claude 需要（`message_start` 带 `input_tokens`，`claude.rs:325-342`），做成 `UsageProbe::needs_head()` |

### S4 —— 记录在案，暂不构成缺陷

| # | 观察 | 证据 file:line | 说明 |
|---|---|---|---|
| 17 | `/v1/*` 上客户端发来的 `x-api-key` 会被转发给上游 | 黑名单无此项 `types.rs:35-50`；剥离只发生在 `/v1beta`：`crates/gw-proxy/src/access.rs:153-164`（`strip_consumed_credentials`） | 对 claude/gemini/vertex 无害（provider 自己 `insert` 覆盖：`claude.rs:247`、`gemini.rs:150`）。对 **openai/codex** 是真的会把租户 header 原样送到 OpenAI —— 不是凭证泄漏（`/v1/*` 的 access 层只认 `Authorization`，`x-api-key` 到不了认证），但是无意义的信息外泄。gw-relay 里应统一：**凡是本层读过的凭证载体，本层负责剥掉** |
| 18 | 流式时 `Accept` 被强制覆盖成 `text/event-stream` | `crates/gw-provider/src/openai.rs:120-122`、`crates/gw-provider/src/claude.rs:620-624` | 上游是按 body 里的 `stream` 字段决定框架的，不看 `Accept`，所以这次覆盖没有产生正确性收益。归入 §3 的"顺手改了" |
| 19 | Gemini 的 model/stream 只能从 URL 拿，hold 阶段按空 model 估算 | `crates/gw-proxy/src/routes.rs:700-730`（代码已自陈） | 已知且已记录，Go 侧同构。只影响预扣额度，不影响结算 |
| 20 | `alt=sse` 被强制 set，客户端不能覆盖 | `crates/gw-provider/src/gemini.rs:129-131` | 当前是为了让 usage 解析器只处理一种框架。若 `UsageProbe` 支持 JSON-array 框架，这条改写就可以撤掉 |

---

## 2. 逐条回答任务清单

### 2.1 请求方向

**入站 Bytes → 上游 body 之间发生了几次拷贝／重编码？**

四段，其中两次是全量堆拷贝、一次可选重编码：

| 段 | 位置 | 代价 |
|---|---|---|
| socket → `Bytes` | `hold.rs:852` `axum::body::to_bytes` | 一次必要的聚合（多 frame 合并成连续内存） |
| `Bytes` → 还回 `req.body` | `hold.rs:867` `Body::from(bytes.clone())` | **零拷贝**（`Bytes::clone` 是 refcount）—— 这一步写对了 |
| `PeekedBody(Bytes)` → `ProviderRequest.payload: Vec<u8>` | `routes.rs:217` `inbound.body.to_vec()` | **全量堆拷贝 #1**，而且它在 failover 循环体内（`routes.rs:208`），**每次尝试拷一遍** |
| `payload` → reqwest body | claude `claude.rs:281` / gemini `gemini.rs:179` / vertex `vertex.rs:556` 的 `req.payload.clone()`；openai `openai.rs:131` / codex `codex.rs:215` 的 `payload.into_owned()` | **全量堆拷贝 #2** |
| reqwest `.body(Vec<u8>)` → 网络 | reqwest 内部 | 零拷贝（`Bytes::from(Vec)` 接管 buffer） |
| （仅 openai/codex 流式）JSON round-trip | `common.rs:252` + `common.rs:264` | **一次全量反序列化 + 一次全量重序列化**，见下 |

即：**`PeekedBody(Bytes)` → `ProviderRequest.payload: Vec<u8>` 这一跳付了两次全量拷贝**（`to_vec` 一次，provider 的 `clone`/`into_owned` 一次），根因是 `types.rs:12-28` 把字段声成了 `Vec<u8>`。

另外，请求 JSON 被**解析 2~3 遍**：`hold.rs:866`（计费 peek）、`routes.rs:632`（handler 再 peek 一遍）、流式再加 `common.rs:252`。

**`ensure_include_usage()` 的触发条件与字节差异**

触发条件（`common.rs:248-268`（`ensure_include_usage`）），三个与门：
1. 调用方传了 `stream = true` —— 只有 `openai.rs:109-113` 和 `codex.rs:193-197` 会传；
2. payload 非空且能反序列化成 **JSON object**（数组/标量原样返回）；
3. body 里 `stream` 的值是**字面量 `true`**（`"true"` 字符串不算）。

**调用它的 provider：只有 `openai` 和 `codex`。** `claude` / `gemini` / `vertex` 都不调（各自的
`build_request` 直接 `.body(req.payload.clone())`）。

字节差异 —— 以下是**实测输出**（用仓库锁定的 `serde_json 1.0.151`、`default-features = false, features = ["std"]`，即 `Map` = `BTreeMap`，无 `preserve_order`）：

```text
IN : { "model": "gpt-4o", "stream": true, "messages": [{"role":"user","content":"hi"}], "temperature": 0.7 }
OUT: {"messages":[{"content":"hi","role":"user"}],"model":"gpt-4o","stream":true,"stream_options":{"include_usage":true},"temperature":0.7}
```

逐项：

| 差异 | 实测 |
|---|---|
| **键序** | 全对象**递归**按字典序重排。注意 `messages[0]` 内部的 `{"role","content"}` 也被排成 `{"content","role"}` —— 改写深入到了消息数组内部 |
| **空白** | 全部剥除（`to_vec` 是 compact 形式） |
| **数值格式** | `1e3` → `1000.0`；`-0` → `-0.0`；`1.0` → `1.0`（保留）；`0.1` → `0.1`（ryu 往返正确）；**`12345678901234567890123` → `1.2345678901234568e+22`（精度丢失，整数变浮点）** |
| **unicode 转义** | `\/` → `/`；`你好` 会被还原成原始 UTF-8 字节 `你好` |
| **重复键** | `{"model":"a","model":"b"}` → `{"model":"b"}`（后者胜，前者消失） |
| **`stream_options` 非对象** | `"stream_options":null` → `{"include_usage":true}`（原值被吃掉） |
| **幂等性** | 第二遍是字节级 no-op ✔（这条注释是准的，`common.rs:245-246`） |

**客户端自带 `stream_options` 会被覆盖吗？** —— **`include_usage` 会被无条件覆盖**
（`common.rs:262` 是无条件 `insert`），同级别的其他键会保留（`common.rs:258-261`）。实测：

```text
IN : {"model":"m","stream":true,"stream_options":{"include_usage":false,"include_obfuscation":true}}
OUT: {"model":"m","stream":true,"stream_options":{"include_obfuscation":true,"include_usage":true}}
```

客户端明确写的 `false` 被翻成 `true`。

**`HOLD_REQUEST_BODY_LIMIT = 1 MiB` 被打满时会发生什么？**

**拒绝，413，请求根本到不了上游。** 代码路径：

1. `hold.rs:166` `is_billable()` → `access.rs:61-63` 的 `is_proxy_path()`，`/v1/` 与 `/v1beta/` 全部命中；
2. `hold.rs:188` 调 `peek_request_body()`；
3. `hold.rs:852` `axum::body::to_bytes(body, HOLD_REQUEST_BODY_LIMIT)` —— axum 的语义是**超限返回 `Err`**，不截断；
4. `hold.rs:854-864` 直接 `return Err(413 + {"error":"Payload Too Large","message":"request body exceeds the billing pre-flight limit"})`；
5. handler 侧的 fallback（无 `PeekedBody` 时）用同一个上限，行为一致：`routes.rs:624-629` 返回裸 `413`。

所以：**不截断、不静默放行，超过 1 MiB 的请求现在完全不能转发。** 注意 `hold.rs:840-843` 的注释
说"Go 会原地截断，那会静默损坏转发的 payload，所以我们改成拒绝" —— 方向是对的（截断比拒绝更坏），
但两个选项都错，正确答案是**不缓冲**（见缺陷 #2 的修法）。

### 2.2 Header 方向

**出站黑名单 `is_skipped_proxy_header` 是否恰当？**（`types.rs:35-50`）

现名单：`authorization`、`connection`、`content-length`、`host`、`keep-alive`、
`proxy-authenticate`、`proxy-authorization`、`te`、`trailer`、`transfer-encoding`、`upgrade`。

| header | 现状 | 判定 |
|---|---|---|
| `content-encoding` | **转发** | ✅ **正确**。网关不解压请求体，所以必须把编码声明一起转过去。（副作用：body 是 gzip 时计费 peek 解析失败 → `model` 为空，落 family 默认 provider + 默认价格。低频，记录即可） |
| `accept-encoding` | **转发** | ⚠️ **字节保真正确、计费错**。见缺陷 #5。gw-relay 里应改成显式 `identity` |
| `expect` | **转发** | ❌ **该丢没丢**。逐跳的连接控制头，代理必须自己消费。见缺陷 #10 |
| `content-type` | **转发** | ✅ 正确。但缺失时 provider 会替客户端编一个 `application/json`（`openai.rs:117-119`、`claude.rs:613-618`）—— 属"顺手改了" |
| `content-length` | 丢弃 | ✅ **必须丢**。body 长度可能变（`ensure_include_usage`），且 reqwest 会按实际 body 重算 |
| `host` | 丢弃 | ✅ **必须丢**。必须跟着上游 URL 重建 |
| `authorization` | 丢弃 | ✅ **必须丢**。上游凭证来自账号池 |
| `x-api-key` | **转发** | ⚠️ claude/gemini/vertex 会覆盖（无害），openai/codex 会原样送出。见观察 #17 |
| `cookie` | **转发** | ⚠️ 客户端 cookie 送到 OpenAI/Anthropic，无意义。低危 |
| `te` / `trailer` / `transfer-encoding` / `upgrade` / `connection` / `keep-alive` / `proxy-*` | 丢弃 | ✅ 正确，RFC 7230 §6.1 |

复制方式本身是**对的**：`types.rs:59-68` 用 `keys()` + `get_all()` + `append`，多值 header 完整保留。

**响应 header 的处理**

两个回写点：流式 `routes.rs:444-462`，非流式 `routes.rs:554-570`。规则相同：遍历上游 header，
跳过 `is_hop_by_hop`（`routes.rs:573-586`），其余 `insert` 进响应。

逐个检查任务点名的头（**仅限 2xx 路径**；4xx/5xx 见下）：

| header | 成功响应 | 幂等回放 | 错误响应(4xx/5xx) |
|---|---|---|---|
| `content-type` | ✅ 原样；缺失时补 `application/json`（`routes.rs:563-568`）/ 流式补 `text/event-stream`（`routes.rs:451-456`） | ✅ 唯一被保留的头（`hold.rs:446-451`） | ❌ **被改写**成 `application/json`（`error.rs:203`），哪怕上游发的是 `text/html` |
| `content-encoding` | ✅ 原样（不在 hop-by-hop 名单） | ❌ 丢失 | ❌ 丢失 |
| `content-length` | ❌ **丢弃**（`routes.rs:584`），由 hyper 按实际 body 长度重算 —— 行为正确（虽然它并不是 RFC 意义上的 hop-by-hop） | ❌ 丢失，重算 | ❌ 丢失，重算 |
| `transfer-encoding` | ❌ 丢弃 —— ✅ 正确，逐跳 | — | — |
| `retry-after` | ✅ 原样 | ❌ 丢失 | ❌ **丢失** ← 缺陷 #3 的核心 |
| `x-request-id` | ✅ 原样 | ❌ 丢失 | ❌ **丢失** |
| `request-id` | ✅ 原样 | ❌ 丢失 | ❌ **丢失** |
| `openai-*`（`openai-organization` / `openai-processing-ms` / `openai-version`） | ✅ 原样 | ❌ 丢失 | ❌ **丢失** |
| `anthropic-*`（`anthropic-ratelimit-*` / `anthropic-organization-id`） | ✅ 原样 | ❌ 丢失 | ❌ **丢失** |
| `x-ratelimit-*` | ✅ 原样 | ❌ 丢失 | ❌ **丢失** |
| `cf-ray` | ✅ 原样 | ❌ 丢失 | ❌ **丢失** |
| `set-cookie`（多值） | ⚠️ **只剩最后一条** ← 缺陷 #7 | ❌ 丢失 | ❌ 丢失 |
| `cache-control` | ✅ 原样；流式缺失时网关自行补 `no-cache`（`routes.rs:457-462`） | ❌ 丢失 | ❌ 丢失 |

一句话：**成功路径上响应头基本是原样过的（除多值折叠），失败路径上一个都不剩。**

**幂等回放的 header 与首次响应是否一致？**

**不一致，而且差得很远。** 只有 `Content-Type` 被存下来（`hold.rs:446-451` 单独取出，
`hold.rs:504-506` 单独写入），回放时若连它也没有就补 `application/json`
（`idempotency.rs:120-126`）。上表第三列全是 ❌。

补充两条：
- 流式响应**永远不会被缓存**（`hold.rs:877-884` 检测到 `event-stream` 直接返回 `None`），于是条目落
  `truncated = true`（`hold.rs:509-511`），重放直接吃 `409 idempotency_replay_unavailable`；
- 非流式响应只有在 `size_hint().upper()` 已知且 ≤ 10 MiB 时才缓存（`hold.rs:889-898`），
  否则同样 `truncated`。

### 2.3 Status 与错误体

**上游 4xx/5xx 的 body 与 header 是否原样回给客户端？**

- **header：完全丢失。** `ProviderError::Upstream` 只有 `{status: u16, body: String}`
  （`types.rs:143-146`），provider 在构造它的那一刻就把 `response.headers()` 扔了
  （`openai.rs:183-188`、`claude.rs:370-372`、`gemini.rs:263-265`、`codex.rs:372-377`、
  `vertex.rs:815-817`）。
- **status：保留**（`error.rs:186-188` 直接用上游 status；`routes.rs:311` 做 `from_u16`，
  不认识的值降级成 `502`）。
- **body：内容保留但被包了一层 content-type。** `error.rs:198-206` 明确不再套网关自己的错误信封
  （注释说"an upstream body is already in the caller's dialect"，这个决定是对的），但强行贴上
  `content-type: application/json`（`error.rs:203`）。上游发 HTML 错误页时这就是撒谎。
- body 为空时才落回网关信封 `{"error":{"message":..,"type":"upstream_error"}}`（`error.rs:208-215`）。

**对客户端 SDK 的直接后果**：能识别出"这是个 429"（status 在），但**识别不出该等多久**
（`retry-after` 没了），也**识别不出配额什么时候恢复**（`x-ratelimit-reset-*` 没了）。
Anthropic 的 529 `overloaded_error` 在 Claude Code 里就变成了盲目指数退避。

**`ProviderError::Upstream { body: String }` 遇到非 UTF-8 / 二进制错误体？**

全部走 `String::from_utf8_lossy(...).into_owned()`（`openai.rs:186`、`claude.rs:641`、
`gemini.rs:264`、`codex.rs:375`、`vertex.rs:816`，以及 `routes.rs:260`）。非法字节被替换成
U+FFFD（EF BF BD），**长度和内容都变了**。这是类型选择直接导致的信息破坏 —— 只要错误体的类型是
`String`，保真度就无从谈起。

**`truncate_failure_body` 截断了多少、截断后还回不回给客户端？**

**它一行都没跑。** 定义在 `common.rs:146-152` 的 `truncate_failure_body`（上限 4 KiB，`from_utf8_lossy` 防切码点），
但全仓 grep 只有三个调用点，全在 `common_tests.rs:153/157/158`。**生产路径零调用**。
`claude.rs:633-637` 的注释还明确写着"所以这里不截断"。

结论：**错误体既不截断也不裁剪，整体回给客户端**（缺陷 #11）。这个函数是死代码，
gw-relay 里不要继承它 —— 要做日志采样就放旁路，别放回写路径。

**跨账号 failover 时最终回给客户端的是哪一次尝试的错误？**

**最后一次尝试的错误。** 逻辑在 `routes.rs:198-289`：

- `last_error` 在每次可重试失败时被**无条件覆盖**：流式 `routes.rs:246`，非流式 `routes.rs:257-261`
  和 `routes.rs:280`；
- `NoUpstream`（该 provider 一个可用账号都没有）用的是 `get_or_insert_with`（`routes.rs:204`），
  **不会**覆盖已有的真实上游错误 —— 这个细节是对的；
- **不可重试的错误（4xx 且非 429，判定见 `routes.rs:295-306`）立即返回，不 failover**
  （`routes.rs:243-246`、`routes.rs:276-279`）；
- `tried` 是**跨 provider 的全局计数**（`routes.rs:195`、`routes.rs:208`），所以
  `MAX_UPSTREAM_ATTEMPTS = 3` 是"整个请求最多打 3 个上游账号"，不是每 provider 3 次;
- 循环耗尽后 `routes.rs:287-288` 用 `last_error`，全空才降级成 `NoUpstream(model)`。

于是：**3 个账号连着 429，客户端拿到第 3 个 429 的 body，且没有任何一次的 `retry-after`。**

### 2.4 流式（保真度重灾区）

**SSE 是否被重新分帧？**

**没有被解析，也没有被重新分帧。** 证据链：

1. `common.rs:462-467`（`usage_stream` 的 Streaming 分支）：`response.bytes_stream()` 的每一项原样包成 `StreamChunk::Payload(chunk)`
   转发，旁路 `buf.write(&chunk)` 只是复制去解析 usage，不影响转发的那份；
2. `routes.rs:411-416`：`StreamChunk::Payload(bytes)` 原样交给 `Body::from_stream`；
3. `routes.rs:405`：axum 的 `Body::from_stream` 把每一项写成一个 data frame。

**边界语义要说准**：保留的是 **HTTP 帧边界**（HTTP/1.1 的 chunk / HTTP/2 的 DATA frame），
不是"上游一次 TCP read"的边界 —— 后者在 hyper 之下就已经不可见了。网关**既不合并也不拆分**
上游给的帧，这是中继能做到的最好情况。

顺带确认：`eventsource-stream` 在 `crates/gw-provider/Cargo.toml:27` 声明了，但**全仓无任何调用**
（未使用依赖）。正因为没人用它，SSE 才没被解析。

**`event:` / `id:` / `retry:` / 注释行 / 多行 `data` 是否原样保留？**

**全部原样保留**，因为压根没有 SSE 解析器介入回写路径。唯一读这些字节的是旁路的 usage 解析器
（`claude.rs:331-341` 的 `sse_data_payload`、`crates/gw-provider/src/usage.rs:362-373` 的
`parse_sse_usage`、`gemini.rs:200-217`），它们只**读**，从不改，也不回写。

这是当前实现**最好的一条**，gw-relay 必须原样继承。

**`StreamUsageBuffer`（head 32 KiB + tail 64 KiB）是拷贝还是引用？每 chunk 额外多少字节？**

**是拷贝，不是引用。**

- tail：`streambuf.rs:83` `self.tail.extend_from_slice(p)` —— **每个 chunk 的每一个字节都被复制一遍**；
- head：`streambuf.rs:77-80`，前 32 KiB 再多复制一份（填满后冻结）；
- 压缩：`streambuf.rs:84-87`，`tail` 长到 `2 × 64 KiB` 时 `drain(..drop)`，一次 ≤64 KiB 的 memmove；
  摊销下来**每流过 64 KiB 触发一次**；
- `bytes()`（`streambuf.rs:99-129`）在流末尾再做一次完整重组拷贝（≤96 KiB + 1）。

量化：**每转发 1 字节 ≈ 额外复制 1 字节**（tail），前 32 KiB 再加 1 份（head），加上摊销的 memmove，
总体约 **2× 全流量的额外内存带宽**。常驻内存 ≈ 96 KiB × 并发流数（Vertex 另有一路独立实现：
`vertex.rs:716-791`，同样的 `StreamUsageBuffer` + 一个额外的 per-chunk `tally.observe(&chunk)`，
`vertex.rs:758-759`）。

**`with_stream_idle_timeout` 的语义：超时后对客户端表现为什么？**

`common.rs:337-361`（`with_stream_idle_timeout`）：超时 yield 一次 `Err(StreamIdleElapsed)` 然后终止内层流。
`common.rs:477-482` 把它翻成 `StreamChunk::Error`。`routes.rs:422-427` 收到后 —— **只打一行
`tracing::warn!`，设 `failed = true`，继续循环**。下一轮进 `UsagePhase::Trailing`，可能再吐一个
`StreamChunk::Usage`（不写字节），然后 `None` → `finish()` → 流正常结束。

对客户端就是：**一次干净的 EOF**。HTTP 层完全正常（h1 发终止 chunk / h2 发 END_STREAM），
SSE 里没有 `message_stop` / `[DONE]`，也没有任何 `event: error`。客户端看到的是一个**被截断但语法
合法**的响应。默认窗口 300 秒（`common.rs:41`（`DEFAULT_STREAM_IDLE_TIMEOUT`））。

附带的计费后果：`failed = true` 让 `plan_settlement` 走 `SettlementPlan::Release`
（`crates/gw-proxy/src/usage.rs:122-126`），**即使 `usage` 已经解析出来了也会被丢弃**，
已经产出的 token 全额退还给租户。

**客户端断开时取消信号能否传播到上游？**

**能。** 链路：客户端断开 → hyper 丢弃 response body → `Body::from_stream` 的 unfold future 被 drop
→ unfold 的 state（`routes.rs:406` 的 `Some((chunks, settler))`）被 drop → `chunks` 即
`StreamResponse::chunks`（`types.rs:115`），它持有 reqwest 的 `bytes_stream()` → reqwest/hyper 的
body 被 drop → HTTP/2 发 `RST_STREAM`，HTTP/1.1 关闭连接（连接不回池）。

**上游不会继续跑完，token 不会白烧。**

同一次 drop 还会触发 `StreamSettler::drop`（`routes.rs:502-533`），把结算丢进
`ProxyState::drain` 这个 `TaskTracker`（`crates/gw-proxy/src/lib.rs:97-112`），所以 hold 也不会
等 TTL。这一段设计是对的，gw-relay 必须保留。

（一个次要观察：断开时 `usage` 通常还没解析出来，于是 outcome 是 `{usage: None, failed: false}`
→ 走 fallback 估算计费。语义上合理。）

### 2.5 压缩与 HTTP 版本

**reqwest client 是否复用？连接池参数？**

**是真全局单例。** `common.rs:93-96`（`shared_client`）：

```rust
pub fn shared_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| build_or_default(client_builder())).clone()
}
```

`reqwest::Client::clone` 是 `Arc` 克隆，所以五个 provider 共用**同一个连接池**
（`openai.rs:61`、`claude.rs:130`、`gemini.rs:62`、`codex.rs:99`、`vertex.rs` 同构）。
`streaming_client()`（`common.rs:101-103`）就是 `shared_client()` 的别名。
`new_http_client(timeout)`（`common.rs:112-114`（`new_http_client`））会新建 client（新池），但**全仓无调用点** ✔。

池参数 `common.rs:67-71`（`client_builder`）：

| 参数 | 值 |
|---|---|
| `pool_max_idle_per_host` | 100 |
| `pool_idle_timeout` | 90s |
| 整请求超时 | **不设**（流式必须如此）；非流式按请求挂 `.timeout(self.timeout)`，默认 60s（`DEFAULT_TIMEOUT`，`common.rs:35`；挂载点 `openai.rs:136-138`） |
| `connect_timeout` | **未设置** ← 缺口：连不上的上游会占满 60s 的整请求预算 |
| `tcp_keepalive` | **未设置** |
| `http2_keep_alive_*` | **未设置** ← h2 长连接上没有心跳，NAT/LB 静默断连要靠 idle 看门狗兜 |
| 代理 | 走 `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY` 环境变量（reqwest 默认） |

**是否启用了 HTTP/2？各上游分别是什么协议版本？**

- **上游方向**：`crates/gw-provider/Cargo.toml:20` 开了 `http2` + `rustls-tls`，rustls 的 ALPN 会同时
  提供 `h2` 与 `http/1.1`。**Anthropic (`api.anthropic.com`) 与 OpenAI (`api.openai.com`) 都支持
  h2 并会在 ALPN 里选中它 → 实际走 HTTP/2。** Google 的
  `generativelanguage.googleapis.com` / Vertex 同理。没有 `http2_prior_knowledge()`，
  所以纯 HTTP 的自建上游（`base_url` 指向 `http://…`）会退回 HTTP/1.1。
- **客户端方向**：`crates/gw-server/src/lib.rs:160` 用 `axum::serve(listener, app)`，底下是
  hyper-util 的 auto builder → 同时支持 HTTP/1.1 与 **h2c（prior-knowledge）**。
  网关自己**不做 TLS 终止**，所以要让浏览器/SDK 用上 h2 必须靠前置代理做 ALPN。
  `crates/gw-proxy/Cargo.toml:21` 和 `gw-server/Cargo.toml:29` 都开了 axum 的 `http2` feature。

**是否存在「上游 gzip → 网关解压 → 回给客户端时重新压缩或明文」的浪费？**

**不存在。** 网关在两个方向上都不碰压缩：

- reqwest 没开任何压缩 feature（`crates/gw-provider/Cargo.toml:20`）→ 不加 `accept-encoding`、
  不自动解压；
- `tower-http` 的 `compression-gzip` 在 `crates/gw-server/Cargo.toml:31` 声明了，但
  `gw-server/src/lib.rs:114-124` 的 `app_router` **从未挂载 `CompressionLayer`** → 声明了没用上。

所以没有"解压→再压缩"的浪费。**真正的问题是另一个方向**：压缩字节被原样透传（保真度是对的），
但旁路的 usage 解析器读的也是压缩字节，必然解析失败 → 计费静默降级（缺陷 #5）。

---

## 3. gw-relay 字节契约建议

### 3.1 核心原则

> **中继层不认识 JSON。** 它只认识 `Bytes`、`HeaderMap`、`StatusCode` 和帧序列。
> 任何需要看懂内容的东西（计费、协议转义）都挂在**旁路**，从中继拿只读视图，
> 永远不往回写路径塞东西。

推论三条：
1. **上游的 4xx/5xx 不是错误**，它是一个响应。`Result::Err` 只留给"连不上"。
2. **没有 `String`**。请求体、响应体、错误体全是 `Bytes`。
3. **没有 `Vec<u8>`**。见 §3.4 的论证。

### 3.2 建议的类型签名

```rust
// ---------------------------------------------------------------- 请求

/// 客户端请求的字节视图。gw-relay 只读，不解析。
pub struct RelayRequest {
    pub method:  http::Method,
    /// 入站的 path + query，**原始字节**。不解码、不重编码、不拆键值对。
    pub target:  http::uri::PathAndQuery,
    /// 入站 header，原样。凭证剥离与 hop-by-hop 过滤发生在 `into_upstream()`。
    pub headers: http::HeaderMap,
    pub body:    RelayBody,
}

/// 请求体的两态。缺陷 #2 的解药就是这个 enum。
pub enum RelayBody {
    /// 已缓冲：体积在 peek 阈值内，计费 peek 与转发**共用同一块内存**。
    /// `clone()` 是 refcount，failover 重试零拷贝。
    Buffered(bytes::Bytes),
    /// 未缓冲：超过阈值，边收边转，**无上限**。计费只能降级到保守估算。
    Streaming(http_body_util::combinators::BoxBody<bytes::Bytes, RelayError>),
}

impl RelayBody {
    /// 计费 peek 唯一被允许的入口：只读，只在 Buffered 上生效。
    /// Streaming 返回 None —— 调用方必须显式处理"我看不到 body"这件事。
    pub fn peek(&self) -> Option<&[u8]> { … }

    /// 转发用。Buffered 走 Bytes（零拷贝），Streaming 直接把 body 接过去。
    pub fn into_upstream(self) -> reqwest::Body { … }
}

// ---------------------------------------------------------------- 响应

/// 上游响应的字节视图。**4xx / 5xx 也是这个类型**，没有第二条回写路径。
pub struct RelayResponse {
    pub status:  http::StatusCode,
    /// 上游 header，原样。hop-by-hop 过滤在写出时做，且用 `append` 保多值。
    pub headers: http::HeaderMap,
    pub body:    RelayResponseBody,
}

pub enum RelayResponseBody {
    Buffered(bytes::Bytes),
    /// item 是 `Result` 而不是 `Infallible` —— 缺陷 #6 的解药。
    /// 中途失败交给 hyper：h2 发 RST_STREAM，h1 掐连接，客户端能察觉截断。
    Stream(http_body_util::combinators::BoxBody<bytes::Bytes, RelayError>),
}

// ---------------------------------------------------------------- 中继

pub struct UpstreamTarget {
    pub origin:     url::Url,         // scheme + host + 可选路径前缀
    pub credential: Credential,       // 换掉 Authorization / x-api-key / x-goog-api-key
    pub timeouts:   RelayTimeouts,    // connect / 非流式整请求 / 流式 idle
}

#[async_trait::async_trait]
pub trait Relay: Send + Sync {
    /// 唯一出口。`Err` 只表示"没能拿到上游响应"（DNS/TCP/TLS/connect 超时），
    /// **上游返回的任何 status 都是 Ok**。
    async fn relay(
        &self,
        req:   RelayRequest,
        to:    &UpstreamTarget,
        probe: Option<Box<dyn UsageProbe>>,   // 旁路，见 3.3
    ) -> Result<RelayResponse, RelayTransportError>;
}

/// 中继只认识"连不上"这一种失败。
#[derive(Debug, thiserror::Error)]
pub enum RelayTransportError {
    #[error("connect failed: {0}")] Connect(#[source] reqwest::Error),
    #[error("upstream idle for {0:?}")] Idle(std::time::Duration),
    #[error("credential is not a valid header value")] BadCredential,
}
```

### 3.3 旁路取数：计费如何在不改字节的前提下拿到 model / stream / usage

计费一共只需要三样东西（从 `crates/gw-proxy/src/usage.rs:213-245` 的 `Settlement::settle` 倒推）：
`SettleCtx.model`、`SettleCtx.stream`、以及 `UsageRecord` 的四个 token 列。

**model / stream —— 只读 peek，不重序列化**

现在的 `parse_body_peek`（`hold.rs:752-791`）用一个只含所需字段的 `#[derive(Deserialize)]` 结构 —— **这个做法本身是对的**，它不产生新字节。要改的只有两点：

1. **只解析一次**，结果进 `RelayContext`，不要在 handler 里再解一遍（现在 `hold.rs:866` 和 `routes.rs:632` 各解一遍）；
2. `RelayBody::Streaming` 时 `peek()` 返回 `None`，计费显式走"model 未知"分支（从 URL / header 兜底，
   兜不到就按最保守的估算 hold）—— **转发不因为计费看不见 body 而失败**。

**include_usage —— 定点字节插入，不做 `Value` round-trip**

```rust
/// 顶层扫一遍有没有 `"stream_options"`。
/// - 有 → 一个字节都不动（尊重客户端，缺陷 #4 的 include_usage:false 问题自然消失）
/// - 没有 → 返回一个两段的 body：[前缀片段, 原始 Bytes 的切片]
///
/// 不反序列化、不重序列化、不重排键、不动数字格式。
/// 用 `Frame` 序列拼，连一次全量拷贝都不需要。
pub fn splice_include_usage(body: &Bytes) -> RelayBody { … }
```

插入点就是最外层 `{` 之后：`{"stream_options":{"include_usage":true},` + 原始字节的 `[1..]` 切片。
唯一的字节差异是这 42 字节的前缀，客户端写的其余部分**逐字节不变**。

再加一个开关 `billing.force_include_usage`（默认 `true`）—— 关掉就完全不碰请求体，
接受 fallback 计费。这是"透传优先于计费"的字面执行。

**usage —— 旁路 probe，零字节拷贝**

```rust
/// 计费方实现它并交给中继。中继保证：
/// 1) 每个**转发出去的** chunk 都会被 observe 一次，顺序与转发顺序一致；
/// 2) observe 拿到的是 `&Bytes`（句柄），实现方要留就 `clone()`（refcount），
///    **中继绝不为 probe 复制字节**;
/// 3) 上游 EOF（或中途失败）之后 finish() 恰好被调一次。
pub trait UsageProbe: Send {
    /// Claude 需要 head（message_start 带 input_tokens），其余四家只要 tail。
    fn needs_head(&self) -> bool { false }
    fn observe(&mut self, chunk: &bytes::Bytes);
    fn finish(self: Box<Self>) -> Option<UsageTokens>;
}
```

实现建议，从省到更省：

- **tail 环**：`VecDeque<Bytes>` + 一个累计长度，超过窗口就 `pop_front`。只持句柄，
  **零字节拷贝**（对比现在 `streambuf.rs:83` 的每字节 memcpy）；
- **增量行解析**（更好）：SSE 是行式的，维护一个"未完成行"的小 buffer，每凑齐一行就喂给解析器，
  命中 `usage` 就更新 tally 并丢弃。内存 `O(单行)`，跨 chunk 只需拷半行。
  usage 解析器现在就是逐行的（`usage.rs:362-373`、`claude.rs:331-341`），改造成本很低。

**压缩这道坎必须显式处理**：probe 拿到的是**转发出去的**字节，上游若 gzip 了它就读不懂。
所以 gw-relay 在请求方向把 `accept-encoding` 收敛成 `identity`（写进 §4 的"必须改"）。
想保留客户端侧压缩的部署，用 `RelayOptions::preserve_client_encoding` 打开，
并明确接受"usage 落 fallback"这个后果 —— **要么正确，要么显式降级，不要静默失真**。

### 3.4 为什么是 `bytes::Bytes` / `http_body::Body` 而不是 `Vec<u8>`

**1. `Vec<u8>::clone` 是 memcpy，`Bytes::clone` 是原子加一。**
现在的 failover 循环（`routes.rs:208-284`）每次尝试都 `inbound.body.to_vec()`（`routes.rs:217`），
provider 再 `clone()` 一次。一个 900 KB 的请求在 3 次 failover 下要 memcpy 约 5.4 MB —— 而这些拷贝
的内容**完全一样**。`Bytes` 下这个数字是 0。

**2. `Bytes` 能零成本切片，`Vec<u8>` 不能。**
`splice_include_usage` 的两段 body（前缀 + `original.slice(1..)`）在 `Bytes` 下共享同一块 allocation；
`Vec<u8>` 下你只能重新分配再拷。同理，`UsageProbe` 的 tail 环持有的是切片句柄，不是副本。

**3. `Vec<u8>` 这个字段类型是当前每一次多余拷贝的直接原因。**
`ProviderRequest.payload: Vec<u8>`（`types.rs:14`）逼出了 `routes.rs:217` 的 `to_vec()`；
`ProviderResponse.body: Vec<u8>`（`types.rs:74`）逼出了五个 provider 的
`response.bytes().await?.to_vec()`（`openai.rs:182` 等）。而末端 `Body::from(Vec<u8>)` 反倒是零拷贝
（`Bytes::from(Vec)` 直接接管 buffer）—— **唯一那次拷贝纯粹是为了满足字段类型**。

**4. `Bytes` 是 axum 和 reqwest 之间的公共货币。**
`axum::body::to_bytes` 产出 `Bytes`，`reqwest::Body::from(Bytes)` 消费 `Bytes`，
`reqwest::Response::bytes_stream()` 产出 `Bytes`，`axum::body::Body::from_stream` 消费 `Bytes`。
选 `Vec<u8>` 等于在两端各自设一道关卡，然后自己交关税。

**5. `http_body::Body` 是"不缓冲"唯一的表达方式。**
`Vec<u8>` 和 `Bytes` 都必须先有完整内容才能存在，`Body` 是一个帧序列 —— 它才能表达
"边收边转、无上限"。**缺陷 #2（1 MiB 上限）只能靠 `Body` 解，`Bytes` 解不了。**

**6. `http_body::Body` 的 `Error` 类型是表达"我中途失败了"的唯一手段。**
`Bytes` 的流可以停，但停不出"这不是正常结束"。缺陷 #6 的根因就是现在把 error type 定成了
`Infallible`（`routes.rs:413`）。

**一句话**：`Bytes` 解决"同一段字节被复制 N 次"，`http_body::Body` 解决"字节必须先全部到齐"
和"结束与失败无法区分"。这三个问题正好覆盖了缺陷表里的 #2、#6、#13、#14、#16。

---

## 4. 不可避免的改写清单

区分「协议/安全要求必须改」与「顺手改了、gw-relay 里应当撤销」。

### 4.1 必须改（保留）

| # | 改写 | 现状 file:line | 为什么必须 |
|---|---|---|---|
| 1 | `Authorization` 丢弃并换成上游账号凭证 | 丢弃 `crates/gw-provider/src/types.rs:38`；注入 `openai.rs:125`、`codex.rs:209` | 租户凭证与上游凭证是两个信任域，不能串 |
| 2 | `x-api-key` 覆盖为 Anthropic 上游凭证 | `crates/gw-provider/src/claude.rs:247`（`insert`，不是 `if !contains_key`） | 同上。注释 `claude.rs:236-239` 的理由是对的 |
| 3 | `x-goog-api-key` 覆盖为 Google 上游凭证 | `crates/gw-provider/src/gemini.rs:150`；Vertex 用 `Authorization: Bearer` `vertex.rs:543-551` | 同上 |
| 4 | `/v1beta` 上剥离租户凭证载体（`x-goog-api-key` / `x-api-key` / `?key=`） | `crates/gw-proxy/src/access.rs:153-164`（`strip_consumed_credentials`） | **安全必需**。不剥就把租户 key 原样送给 Google。原则要推广：**凡是本层读过的凭证载体，本层负责剥掉**（观察 #17 就是这条没推广到 `/v1/*`） |
| 5 | `Host` 丢弃，按上游 URL 重建 | 丢弃 `types.rs:41`，reqwest 重建 | 请求换了 origin，`Host` 必须跟着换 |
| 6 | 请求 `Content-Length` 丢弃并重算 | 丢弃 `types.rs:40`，reqwest 按实际 body 重算 | body 长度可能变（`stream_options` 插入），且 `Streaming` 态下压根没有确定长度 |
| 7 | 逐跳 header 消费：`connection` / `keep-alive` / `te` / `trailer` / `transfer-encoding` / `upgrade` / `proxy-authenticate` / `proxy-authorization` | 请求方向 `types.rs:36-48`；响应方向 `routes.rs:573-586` | RFC 7230 §6.1。代理必须自己消费，不能转发 |
| 8 | 响应 `Content-Length` 丢弃并重算 | `routes.rs:584`（挂在 `is_hop_by_hop` 名下） | 流式没有确定长度；非流式的长度由本地 body 决定。**行为正确，但分类不严谨** —— gw-relay 里应把它从 hop-by-hop 名单里挪出来，单独作为"本地重算"处理 |
| 9 | 请求 URL 的 origin 换成上游端点 | `common.rs:280-313`（`chat_completions_endpoint`）、`claude.rs:214-232`、`gemini.rs:104-134` | 中继的定义就是这个 |
| 10 | **`expect: 100-continue` 应加入黑名单**（当前**缺失**） | `types.rs:35-50` 没有它 | 逐跳的连接控制头，代理必须自己消费（缺陷 #10） |
| 11 | **`accept-encoding` → `identity`**（当前未做，建议加入必须改） | 见缺陷 #5 | 旁路 usage 统计与端到端压缩互斥，必须显式选一边。默认选正确性 |
| 12 | 客户端凭证不进 tracing / 日志（`?key=` 脱敏） | `crates/gw-proxy/src/access.rs:176-188` 的 `redact_query` | 安全必需。gw-relay 里所有渲染 URI 的地方都必须走它 |

### 4.2 顺手改了（gw-relay 里应当撤销）

| # | 改写 | 现状 file:line | 为什么可以撤 |
|---|---|---|---|
| 1 | `ensure_include_usage` 的整体 JSON round-trip | `crates/gw-provider/src/common.rs:248-268`（`ensure_include_usage`） | 目的（拿到 usage）只需要 42 字节的定点插入。整体重序列化重排了递归键序、改了数字格式、丢了重复键、还把客户端的 `include_usage:false` 翻成 `true`。**用 §3.3 的 `splice_include_usage` 替换** |
| 2 | 流式时 `Accept` 被强制覆盖成 `text/event-stream` | `openai.rs:120-122`、`claude.rs:620-624`（`insert`） | 上游按 body 的 `stream` 字段决定框架，不看 `Accept`。这次覆盖没有正确性收益。真要保底就 `if !contains_key` |
| 3 | 请求 `Content-Type` 缺失时补 `application/json` | `openai.rs:117-119`、`codex.rs:201-203`、`claude.rs:613-618` | 客户端没给就不该替它编一个。让上游按 RFC 处理缺失 |
| 4 | 请求 `Accept` 缺失时补 `application/json` | `openai.rs:122-124`、`claude.rs:625-630` | 同上 |
| 5 | 响应 `Content-Type` 缺失时补 `application/json` | `routes.rs:563-568` | 上游没说的话，网关不该替它说 |
| 6 | 响应 `Cache-Control` 缺失时补 `no-cache`（仅流式） | `routes.rs:457-462` | 网关自己发明的语义，上游从没说过 |
| 7 | 流式响应 `Content-Type` 缺失时补 `text/event-stream` | `routes.rs:451-456` | 同上。上游若真没给，那是上游的问题 |
| 8 | 错误响应的 `content-type` 一律写成 `application/json` | `crates/gw-proxy/src/error.rs:200-206` | 上游发的可能是 `text/html`（Cloudflare 错误页）。随缺陷 #3 一起，改成**原样回上游的 content-type** |
| 9 | 错误体经过 `String` + `from_utf8_lossy` | `openai.rs:186`、`claude.rs:641`、`gemini.rs:264`、`codex.rs:375`、`vertex.rs:816`、`routes.rs:260` | 有损，且长度会变。错误体是 `Bytes`，原样回 |
| 10 | query 的"不解码 + 重编码" | `routes.rs:648-657` + `common.rs:305-311` + `claude.rs:589-597` | 净效果是双重百分号编码（缺陷 #9）。原始 query string 直接拼 |
| 11 | Gemini 强制 `alt=sse` | `gemini.rs:129-131` | 现在是为了让 usage 解析只面对一种框架。若 `UsageProbe` 支持 JSON-array 框架，这条就该撤 —— 客户端要 JSON-array 框架是它的自由 |
| 12 | 响应 header 多值折叠 | `routes.rs:444-449`、`routes.rs:557-562` | 纯 bug，不是任何人的设计意图。改 `insert` → `append`（缺陷 #7） |

### 4.3 边界情况（既非必须、也非顺手，是产品决策）

| # | 改写 | 现状 file:line | 说明 |
|---|---|---|---|
| 1 | 1 MiB 请求体上限 | `crates/gw-proxy/src/hold.rs:45` | 这是**计费实现细节泄漏成了转发能力限制**。gw-relay 里应该拆成两个数：`PEEK_BUFFER_LIMIT`（计费想看多少，超了就降级估算）和**没有**转发上限 |
| 2 | 10 MiB 幂等缓存上限 | `hold.rs:49` | 这个上限是合理的（缓存容量是真实约束）。但超限后的行为不合理：现在是"记 truncated 然后拿 409 打客户端"。应该改成"不记条目，让重试正常重跑" |
| 3 | `MAX_UPSTREAM_ATTEMPTS = 3` 是跨 provider 的全局计数 | `routes.rs:40`、`routes.rs:195`、`routes.rs:208` | 语义值得明确写进文档：不是"每 provider 3 次"，是"整个请求最多打 3 个上游账号" |

---

## 5. 附：本次审计用到的验证手段

- `ensure_include_usage` 的字节 diff 是**实测**的，不是推断：用仓库 `Cargo.lock` 锁定的
  `serde_json 1.0.151`（`default-features = false, features = ["std"]`，即 `Map = BTreeMap`）
  在 scratchpad 里跑了一个独立 9 用例的最小复现，输出见 §2.1 的表格。
- `truncate_failure_body` / `eventsource-stream` / `new_http_client` 的"零生产调用"结论来自全仓
  `grep`，不是印象。
- 压缩结论来自 `crates/gw-provider/Cargo.toml:20` 的 feature 集（无任何压缩 feature）与
  `crates/gw-server/src/lib.rs:114-124`（未挂载 `CompressionLayer`）两处交叉确认。
- **行号做过一次程序化核对**：把每一条 `file:line` 引用对着 `git show 1acdc49:<path>` 抽出的
  blob 逐条断言"这一行确实含有我说的那个构造"，共 70 条，全绿。
  之所以钉在提交而不是工作区：审计期间并行 worker 正在同一个 worktree 上剥离旧品牌引用，
  `types.rs` / `common.rs` / `openai.rs` / `codex.rs` / `streambuf.rs` / `claude.rs` / `gemini.rs` /
  `vertex.rs` / `routes.rs` / `hold.rs` / `error.rs` / `access.rs` / `gw-server/src/lib.rs`
  在两小时内漂了多次。提交是不可变的，工作区不是。
- 本次审计**未修改任何 `.rs` / `Cargo.toml` / config**，也未运行 `cargo clean`。
  唯一的 cargo 调用是 scratchpad 里的独立最小复现，`CARGO_TARGET_DIR=/tmp/cargo-audit-passthrough`。
