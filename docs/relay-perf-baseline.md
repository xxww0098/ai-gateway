# gw-relay 性能基线：当前转发路径的真实开销

> 目的：把当前 `gw-proxy` 转发路径的开销量成数字，给正在并行开发的
> `crates/gw-relay`（wave 2）定出**可验收的目标**。**没有数字的优化建议本文一条都不写。**
>
> `gw-relay` 的第一性原理是「**中继层不认识 JSON**，只认识 `Bytes` / `HeaderMap` /
> `StatusCode` 和帧序列」（见 `crates/gw-relay/src/lib.rs`）。本文 §3 的热路径清单，
> 逐条都是那句话被违反时付出的代价：拷贝链、`serde_json::Value` 往返、
> 每 chunk 复制进 tail —— 三者全部来自「中继层认识 JSON」。
>
> 压测装置：[`scripts/perf/`](../scripts/perf/README.md)（一条命令复现）。
> 原始数据：`scripts/perf/results/`。

---

## 0. 一页结论

**对照组下界**是一个约 60 行的 axum 纯 `Bytes` 反代（请求/响应双向流式、零解析、零计费），
跑在同一套 tokio + hyper + reqwest 栈上、打同一个 mock 上游。**网关自身开销 = 实测 − 下界。**

| 口径 | 下界 | 当前网关 | 网关自身开销 |
| --- | ---: | ---: | ---: |
| 非流式 1 KiB→2 KiB，p50 | 53.9 µs | 65.2 µs | **+11.3 µs** |
| 非流式 256 KiB→1 MiB，p50 | 273.8 µs | 453.0 µs | **+179.2 µs** |
| SSE 建流固定成本，p50 | 56.8 µs | 68.4 µs | **+11.7 µs** |
| SSE 满速 500×1 KiB 整流，p50 | 1 125.7 µs | 1 420.1 µs | **+294.4 µs（0.585 µs/chunk）** |
| 吞吐（concurrency 16） | 68 836 rps | 50 962 rps | **降到 74.0%** |
| 每请求堆分配（1 KiB→2 KiB） | 84 次 / 29.8 KB | 233 次 / 56.1 KB | **+149 次 / +26.3 KB** |
| 每请求堆分配（256 KiB→1 MiB） | 104 次 / 1.83 MB | 262 次 / 5.50 MB | **+158 次 / +3.67 MB** |
| 分配字节 ÷ 载荷字节 | 1.40× | **4.20×** | +2.79 字节/载荷字节 |
| SSE 分配字节 ÷ 流量字节 | 0.107× | **0.937×** | 每个流字节被完整复制了一遍 |

五个最值得记住的数：

1. **小 body 的开销是分配次数堆出来的，不是拷贝堆出来的。** 1 KiB 请求只多花 11.3 µs，
   但多做了 **149 次堆分配**（合 **76 ns/次**）；CPU profile 里分配器占有效 CPU
   **25.3%**（下界 16.3%）。1 KiB + 2 KiB 一共才 3 KB，按拷贝斜率只值 0.4 µs ——
   剩下的全是分配。
2. **大 body 的开销是拷贝堆出来的，斜率 0.132 µs/KiB。** 每多 1 字节载荷，网关比下界
   多分配 **2.79 字节** —— 请求体和响应体各被完整搬运了 2~3 遍。
3. **流式中继不是零拷贝。** SSE 场景网关分配了 **0.937× 流量**的字节，下界只有 0.107×。
   元凶是 `StreamUsageBuffer` 把**每一个 chunk 完整复制进 tail 窗口**。
4. **`ensure_include_usage` 对 256 KiB 的流式请求要花 105 µs**（0.409 µs/KiB），
   因为它把整个 body 反序列化成 `serde_json::Value` 再重新序列化。1 KiB 时看不见，
   完全随 body 线性增长。
5. **带 `Idempotency-Key` 的 1 MiB 响应会多花 947 µs、多分配 4.90 MB**（≈ 4.7× 响应体）。
   这是 `hold::capture_body` 为幂等重放做的全量缓冲，只在带 key 时发生。

顺带**证伪**了两条常见猜测（详见 §3.1「查过、不是问题」）：
reqwest 客户端与连接池**确实复用**（`OnceLock`，五个 executor 共享）；
正常请求路径上**没有** `tokio::spawn` / `TaskTracker` 开销（只有客户端中途断流时才 spawn）。

gw-relay 的验收数字在 §5。

---

## 1. 怎么测的

### 1.1 四个被测端，同一份负载

| 被测端 | 是什么 | 用来回答 |
| --- | --- | --- |
| **floor**（理论下界） | ~60 行 axum 纯 `Bytes` 反代：请求体 `wrap_stream` 直接交给 reqwest，响应体 `bytes_stream()` 直接回客户端。零解析、零计费、零 `Vec<u8>` 落地 | 同一套运行时能做到的最好水平是多少 |
| **nomw** | 真 `gw_proxy::routes::chat_completions`，**不挂** access / hold 中间件 | 转发/中继本身有多贵 |
| **full** | 真 `gw_proxy::router()`，生产拓扑 | 生产上一共有多贵 |
| **idem** | 同 full，额外挂幂等管理器 | `capture_body` 的全量响应缓冲有多贵 |

**网关自身开销 = full − floor；access+hold 净成本 = full − nomw。**

四个被测端**同时在跑**（空闲不耗 CPU），同一轮里 A/B/A/B 交错打，跨轮取中位数。
本机后台负载不轻（load average 7~11，12 核），这个安排让后台漂移对四者同分布 ——
**绝对值偏悲观，差值可信**。每张表都给出 p50 的跨轮 min–max，离散度自己判断。

### 1.2 负载形态

| 代号 | 形态 | 请求数/轮 | 轮数 |
| --- | --- | ---: | ---: |
| a) `small` | 1 KiB 请求 / 2 KiB 响应，非流式 | 10 000 | 7 |
| b) `large` | 256 KiB 请求 / 1 MiB 响应，非流式 | 1 500 | 7 |
| c) `sse` | 500 chunk × 1 KiB × 间隔 1 ms | 40 | 3 |
| c-0) `ssettfb` | 1 chunk 无间隔（只测建流固定成本，样本量大） | 4 000 | 7 |
| c-1) `sseburst` | 500 chunk × 1 KiB **无间隔**（分辨每 chunk 成本） | 300 | 5 |
| 1c) `json*` | 同一 body 只切 `stream` 真假，响应压到最小 | 2 000~6 000 | 5 |

延迟档一律 **concurrency = 1**：要量的是每请求固定开销，并发只会把排队时间混进来。
吞吐单独一档量（§2.8）。连接全程 keep-alive 复用，不含 TCP 握手。

本文数字来自 **161 次压测运行、5 546 470 个请求**。

### 1.3 mock 上游与压测客户端为什么是自己写的

`crates/gw-proxy/src/testsupport/upstream.rs` 里的 `FakeProvider` 是 **`Provider` trait 的进程内
替身**：直接返回 `ProviderResponse`，整条路径上没有 reqwest、没有 socket、没有 HTTP 编解码、
没有 SSE 时间轴。用它量「转发开销」会漏掉本任务最关心的三样 —— 真实 reqwest 客户端与连接池、
body 在 `Bytes`/`Vec<u8>` 之间的真实搬运、流式中继的首字节延迟。而且它是
`#[cfg(test)] pub(crate)`，crate 外引用不到。

所以 `scripts/perf/perfkit/src/bin/mock-upstream.rs` 起了一个**真 HTTP 上游**。
**`testsupport` 一行未改，只作为只读参考。**

负载生成器同理自写（裸 TCP + 手写 HTTP/1.1）：本机没装 oha / wrk / bombardier / hey / k6
（已逐个确认），而且它们都不给 **SSE 每 chunk 的到达时刻**。

分配计数用自包的 `#[global_allocator]`（`counting_alloc.rs`），不是 dhat：要的是**每请求**
增量 `(计数差)/(请求数)`，dhat 给的是整进程堆快照；而且加 dhat 要动 workspace 的
`Cargo.toml`，那是协调者独占文件。计数默认关闭，由 `PERF_COUNT_ALLOC=1` 打开 ——
延迟档和分配档分开跑，免得两条 relaxed 原子加污染 p99。

### 1.4 这份基线**不包含**什么（读数字之前必须知道）

* **不含 Postgres / Redis。** `gw_proxy::ports::*` 全部由内存常量桩实现
  （`perfkit/src/stubs.rs`）。理由：要量的是转发路径本身，也就是 gw-relay 将要优化的部分；
  真 DB/Redis 的 RTT 是另一条曲线，它不随 body 大小变化、也不会被零拷贝改掉，混进来只会
  把 µs 级的网关开销淹没在 ms 级 IO 里。**代价：本文量到的 hold/settle 开销是下界**，
  生产上 hold 还要多一次 Redis Lua、settle 还要一次 PG 事务。
* **不含 TLS，因此不含 HTTP/2。** mock 上游是明文 http，reqwest 只在 TLS ALPN 协商时才启用
  h2，所以全程 HTTP/1.1。生产上游是 https，会走 h2 —— **h2 路径没有覆盖**。
* **不含限流 / 幂等 / 熔断的真实后端。** 限流器和熔断器挂的是「永远放行」的桩，
  幂等挂的是进程内 HashMap。它们的**调用点**开销在基线里，**后端 RTT** 不在。
* **不含跨账号 failover 的重试路径**（凭证池只放了 1 条，且从不失败）。
  注意 `Dispatcher::auths_for` 每请求克隆整份凭证表 —— 只有 1 条时它便宜，
  生产上几十上百条时会线性放大（§3 第 5 条）。
* **不是干净的压测机。** 测量期间本机同时在跑编译、浏览器和其它 agent。
  绝对值（尤其 rps）不要当成产能上限。

### 1.5 环境

```
host:    Darwin 25.6.0 arm64 / Apple M4 Pro / 12 核
rustc:   1.97.1 (8bab26f4f 2026-07-14)
profile: release —— opt-level=3, lto="thin", codegen-units=1
         （codegen 部分与根 workspace 逐字一致）
         唯一差异：保留符号（strip="none", debug=1）。这两项不改变生成的机器码，
         但没有它们 `sample` 的输出全是地址，CPU 热点档等于没测。
loadavg: 测量前 10.83 / 测量后 9.57
tokio:   每个被测进程 3 个 worker 线程（钉死以便复现）
采集:    2026-08-15T11:20Z —— 延迟 / 流式 / JSON / 吞吐 / 幂等 五档
         分配档与 profile 档来自同日稍早一轮：这两档对后台负载不敏感，
         分配计数是确定性的，两轮的每一个数字完全一致。
```

---

## 2. 实测数据

### 2.1 a) 非流式小 body：1 KiB 请求 / 2 KiB 响应

| 被测端 | 轮数 | p50 µs | p95 µs | p99 µs | rps(串行) | p50 跨轮 min–max |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| 对照组下界 (floor) | 7 | 53.9 | 86.2 | 119.3 | 17 006 | 42.4–55.8 |
| 网关 无中间件 (nomw) | 7 | 60.7 | 95.8 | 121.3 | 15 042 | 60.1–63.7 |
| 网关 全链路 (full) | 7 | 65.2 | 104.4 | 155.4 | 14 005 | 63.2–68.3 |
| **nomw − 下界** | | **+6.8** | +9.6 | +1.9 | | |
| **full − 下界** | | **+11.3** | +18.2 | +36.0 | | |
| **access+hold 净成本 (full − nomw)** | | **+4.5** | +8.6 | +34.1 | | |

跨轮区间不重叠（floor 42.4–55.8 vs full 63.2–68.3），差值是实的。
p99 的差（+36 µs）是 p50 的三倍，说明**网关的尾部比下界更长** —— 与 §2.7 的
分配器占比一致（分配器争用在尾部放大）。

### 2.2 b) 非流式大 body：256 KiB 请求 / 1 MiB 响应

| 被测端 | 轮数 | p50 µs | p95 µs | p99 µs | rps(串行) | p50 跨轮 min–max |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| 对照组下界 (floor) | 7 | 273.8 | 335.6 | 366.7 | 3 538 | 267.9–287.6 |
| 网关 无中间件 (nomw) | 7 | 434.8 | 527.8 | 581.1 | 2 265 | 410.1–447.5 |
| 网关 全链路 (full) | 7 | 453.0 | 546.8 | 595.0 | 2 173 | 436.9–469.9 |
| **nomw − 下界** | | **+161.0** | +192.2 | +214.5 | | |
| **full − 下界** | | **+179.2** | +211.2 | +228.3 | | |
| **access+hold 净成本 (full − nomw)** | | **+18.3** | +19.0 | +13.8 | | |

**斜率**：`(179.2 − 11.3) µs / (1 310 720 − 3 072) 字节` = **0.132 µs/KiB**
（约 7.8 GB/s，与 2~3 遍 `memcpy` 在 M4 Pro 上的水平吻合）。

拆开看（§2.5 的隔离实验）：256 KiB **请求体**单独贡献 **+60.5 µs**
（0.236 µs/KiB ≈ 2 次全量拷贝，即 0.118 µs/KiB/次），
1 MiB **响应体**贡献剩下的 ≈ **119 µs**（≈ 1 次全量拷贝 + 一次全量 JSON 扫描）。

### 2.3 c) SSE

**c-0) 建流固定成本**（1 chunk 无间隔，7 轮 × 4 000 = 28 000 样本，可信）：

| 被测端 | 轮数 | p50 µs | p95 µs | p99 µs | p50 跨轮 min–max |
| --- | ---: | ---: | ---: | ---: | --- |
| 对照组下界 (floor) | 7 | 56.8 | 95.2 | 119.6 | 54.2–59.9 |
| 网关 无中间件 (nomw) | 7 | 64.8 | 95.0 | 120.9 | 61.1–69.3 |
| 网关 全链路 (full) | 7 | 68.4 | 108.5 | 165.7 | 65.9–76.2 |
| **full − 下界** | | **+11.7** | +13.3 | +46.1 | |

**c) 任务规定的长流**（500 chunk × 1 KiB × 1 ms）：

| 被测端 | 轮数 | TTFB p50 µs | TTFB p99 µs | chunk 间隔 p50 µs | chunk 间隔 p99 µs | 间隔 stddev µs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 对照组下界 (floor) | 3 | 518.9 | 2 066.6 | 2 357.3 | 4 220.8 | 565.9 |
| 网关 无中间件 (nomw) | 3 | 540.6 | 2 307.7 | 2 353.7 | 4 623.3 | 662.3 |
| 网关 全链路 (full) | 3 | 711.5 | 1 899.9 | 2 354.2 | 4 399.3 | 617.8 |
| **full − 下界** | | +192.5 | −166.7 | **−3.1** | +178.5 | +51.9 |

> ⚠️ **这一档的 chunk 间隔量不出网关开销 —— 这是装置的分辨率限制，不是「网关没有开销」。**
> 规定的 1 ms 间隔在本机被 mock 的定时器放大成 ~2.35 ms，三个被测端的间隔中位数
> 全部落在 2 355 ± 2 µs；网关每 chunk 的真实成本（实测 0.585 µs，见 c-1）
> 比这个定时器的抖动小三个数量级。间隔 stddev 的差（+51.9 µs）与 p99 的差（+178 µs）
> 方向一致（尾部确实更抖），但拆不出成因。
> TTFB 这一档只有 3 轮 × 40 = 120 个样本（p99 的差甚至是负的），与 c-0 的
> 28 000 样本冲突；**以 c-0 的 +11.7 µs 为准**。

**c-1) 满速长流**（同样 500 × 1 KiB，间隔设 0）—— 这一档才分辨得出每 chunk 成本：

| 被测端 | 轮数 | 整流 p50 µs | 跨轮 min–max | 每 chunk µs |
| --- | ---: | ---: | --- | ---: |
| 对照组下界 (floor) | 5 | 1 125.7 | 1 049.4–1 165.7 | 2.238 |
| 网关 无中间件 (nomw) | 5 | 1 459.0 | 1 307.4–1 466.3 | 2.901 |
| 网关 全链路 (full) | 5 | 1 420.1 | 1 285.4–1 464.8 | 2.823 |
| **full − 下界** | | **+294.4** | | **+0.585** |

> nomw 比 full 还慢 39 µs —— 两者跨轮区间几乎完全重叠，**在本装置的分辨率下不可分辨**。
> 这个结果合理：流式路径上 access+hold 只在建流时跑一次，与 chunk 数无关。

### 2.4 每请求堆分配（最能说明零拷贝做没做到的指标）

| 场景 | 被测端 | 请求数 | 分配次数/请求 | 分配字节/请求 | 相对下界(次) | 相对下界(字节) |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| a) 1 KiB→2 KiB | 对照组下界 (floor) | 5 000 | 84.0 | 29 771 | — | — |
| a) | 网关 无中间件 (nomw) | 5 000 | 167.0 | 46 812 | +83.0 | +17 041 |
| a) | 网关 全链路 (full) | 5 000 | 233.0 | 56 117 | **+149.0** | **+26 346** |
| a) | 网关 + 幂等 (idem) | 5 000 | 266.0 | 68 688 | +182.0 | +38 917 |
| b) 256 KiB→1 MiB | 对照组下界 (floor) | 500 | 104.3 | 1 829 601 | — | — |
| b) | 网关 无中间件 (nomw) | 500 | 195.7 | 5 455 642 | +91.4 | +3 626 041 |
| b) | 网关 全链路 (full) | 500 | 262.5 | 5 504 909 | **+158.2** | **+3 675 308** |
| b) | 网关 + 幂等 (idem) | 500 | 306.2 | 10 407 960 | +201.9 | **+8 578 359** |
| c) SSE 500×1 KiB | 对照组下界 (floor) | 20 | 1 088.8 | 55 090 | — | — |
| c) | 网关 无中间件 (nomw) | 20 | 1 310.3 | 470 791 | +221.5 | +415 700 |
| c) | 网关 全链路 (full) | 20 | 1 383.5 | 480 679 | **+294.7** | **+425 589** |

空载噪声：两个被测进程都是 **8 次分配/秒**（5 s 内 40 次）。折算到上表的实测速率，
噪声占比 < 0.1%，未从表中扣除。这一档跨两轮独立重跑**完全复现**（同一数字到个位）。

**放大倍数**（分配字节 ÷ 载荷字节）：

| | 载荷 | floor | full | idem |
| --- | ---: | ---: | ---: | ---: |
| b) 256 KiB + 1 MiB | 1 310 720 B | **1.40×** | **4.20×** | **7.94×** |
| c) SSE 501 × 1 KiB | 513 024 B | **0.107×** | **0.937×** | — |

SSE 那一行是本文最干净的一条证据：**下界只分配了流量的十分之一（真流式转发），
网关分配了流量的 0.94 倍 —— 每一个流字节都被完整复制了一遍。**

**字节斜率** = `(3 675 308 − 26 346) / (1 310 720 − 3 072)` = **2.79 额外分配字节/载荷字节**。

### 2.5 隔离实验：`ensure_include_usage` 的 serde_json 往返

同一个请求体、响应都压到最小（不让响应干扰），只切 `stream` 真假。

| 场景 | floor p50 | full p50 | 网关开销 |
| --- | ---: | ---: | ---: |
| 1 KiB 请求，非流式 | 51.5 | 63.8 | +12.2 |
| 1 KiB 请求，流式 | 71.5 | 80.0 | +8.6 |
| 256 KiB 请求，非流式 | 103.5 | 164.0 | **+60.5** |
| 256 KiB 请求，流式 | 124.0 | 285.6 | **+161.6** |

* 「流式 − 非流式」的额外网关开销：@1 KiB = **−3.7 µs**（噪声内）；@256 KiB = **+101.1 µs**。
* 两者之差 **104.8 µs / 256 KiB = 0.409 µs/KiB**，就是 `ensure_include_usage` 把整个 body
  反序列化成 `serde_json::Value` 再重新序列化的代价。
  **它只对流式请求发生，且完全随 body 线性增长。**
* 顺带得到：256 KiB **请求体**在非流式路径上的拷贝代价 = **60.5 µs = 0.236 µs/KiB**，
  对应 `to_vec()` + `into_owned()` 两次全量拷贝（≈ **0.118 µs/KiB/次**），
  与 §2.2 的整体斜率 0.132 µs/KiB 互相印证。

### 2.6 幂等（`Idempotency-Key` 触发 `hold::capture_body`）

| 场景 | 幂等 | p50 µs | p99 µs | Δp50 | Δ分配字节/请求 |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 KiB→2 KiB | off | 56.9 | 116.8 | | |
| 1 KiB→2 KiB | on | 64.9 | 116.8 | **+8.0** | +12 571 |
| 256 KiB→1 MiB | off | 485.7 | 687.4 | | |
| 256 KiB→1 MiB | on | 1 433.0 | 1 933.2 | **+947.3** | **+4 903 051** |

1 MiB 响应带一个幂等 key，延迟涨到 **2.95 倍**，多分配 **4.90 MB ≈ 4.7 倍响应体**。
这个差值跨三轮独立重跑稳定复现（+948 / +964 / +947 µs）。

> 这个数第一次跑时是**负的**（−520 µs，「开幂等更快」）。原因是预热段和正式测量段各自从 0
> 开始生成 key，正式段每一发都命中预热留下的缓存条目、直接重放，根本没打上游。
> 修法见 `loadgen.rs` 里 `KEY_SEQ` 的注释。**记在这里是因为它是本装置最容易复发的假数据。**

### 2.7 CPU 热点 profile

工具：`/usr/bin/sample`（macOS 自带，无需 root）。cargo-flamegraph / samply 本机都没装，
装它们要走网络，不属于「一条命令能复现」的范围。
归因方式：只统计**叶子帧**（下一行缩进不比自己深）—— 那才是自身 CPU 时间。
`__psynch_cvwait` / park 是线程空闲等活，单列并从分母里扣掉。
两个 profile 在同样 15 s、同样 concurrency=8 的负载下采集。

| 类别（占**扣除 park 后的有效 CPU**） | 网关 full | 对照组 floor |
| --- | ---: | ---: |
| **分配器 malloc/free/realloc** | **25.30%** | 16.28% |
| **memmove / memset / memcpy** | **5.67%** | 4.07% |
| **`getentropy`（uuid v4 取熵）** | **1.93%** | **0%** |
| **serde_json** | **2.35%** | **0%** |
| 网络系统调用 recv/writev/kevent | 15.52% | 31.35% |

读法：网关把本该花在收发包上的 CPU（31% → 16%）挪去了分配器（16% → 25%）、
memcpy（4.1% → 5.7%），以及两样对照组根本不存在的东西 —— **每请求一次
`getentropy` 系统调用**和 **serde_json**。

单帧 top（网关，占全部样本）：`_xzm_free` 3.64%、`_xzm_xzone_malloc_tiny` 2.46%、
`_platform_memmove` 1.80%、`getentropy` 1.28%、
`serde_json::read::SliceRead::skip_to_escape` 0.39%。
`getentropy` 的调用方在调用图里直接可见：**`gw_proxy::hold::trace_id_from`**
（`uuid::Uuid::new_v4()` → `uuid::fmt::format_simple`）。

完整输出：`scripts/perf/results/profile-summary.txt`，原始调用图
`profile-gateway-full.txt` / `profile-floor.txt`。

### 2.8 吞吐（concurrency = 16，5 s × 3 轮）

| 被测端 | rps 中位数 | rps min–max | p50 µs | p99 µs |
| --- | ---: | --- | ---: | ---: |
| 对照组下界 (floor) | 68 836 | 68 190–68 850 | 229.5 | 369.9 |
| 网关 无中间件 (nomw) | 58 597 | 55 317–59 390 | 270.2 | 437.0 |
| 网关 全链路 (full) | 50 962 | 48 992–52 109 | 308.5 | 514.0 |

**网关吞吐 = 下界的 74.0%**，两层中间件占其中约 11 个百分点（nomw 是 85.1%）。
这一轮的跨轮离散度很小（floor 68 190–68 850，不到 1%），数值可用。

> 早先在机器更忙时跑过一轮同样配置，floor 的 rps 在 9 663–28 561 之间跳。
> **同一台机器上，这张表的绝对值可以差 3 倍。** 上面这一轮是较安静的窗口，
> 但仍不是干净压测机 —— 相对关系可信，绝对值仅供参考。

### 2.9 装置自身的缺陷（如实记录）

161 次运行 / 5 546 470 个请求中，出现 **2 次单请求超时**（`stalls`）：
一次在 `floor`、一次在 `nomw`，都在 SSE-TTFB 场景，发生率约 **1/2 800 000**。
表现为客户端写完请求后永远等不到响应，三个线程全部 park。
`loadgen` 的超时 + 重连机制吸收了它，样本数不受影响。

对照组和网关两侧都出现过，**所以最可能的位置是三者共用的那一段**：我手写的裸 TCP
HTTP/1.1 客户端（chunked 解析之后的连接复用状态），而不是 `gw-proxy`。
**没有定位到根因** —— 不要把它当成网关的问题，也不要当成它不存在，复现时它还会出现。

---

## 3. 热路径开销清单（按收益排序）

排序依据：a) 小 body 的 11.3 µs / 149 次分配，b) 大 body 的 0.132 µs/KiB 斜率，
c) SSE 的 0.937× 分配放大，三个口径的贡献。行号以当前工作区为准。

| # | 开销点 | 证据 | 预估收益 | 难度 |
| ---: | --- | --- | --- | --- |
| 1 | **body 全量拷贝链**：`PeekedBody(Bytes)` → `payload: Vec<u8>` → reqwest body；回程 `Bytes` → `body: Vec<u8>` | `routes.rs:212` `inbound.body.to_vec()`；`openai.rs:126` `.body(payload.into_owned())`；`openai.rs:171` `response.bytes().await?.to_vec()`；类型定义 `types.rs:14` `ProviderRequest.payload: Vec<u8>` / `types.rs:71` `ProviderResponse.body: Vec<u8>`。**实测**：分配字节 4.20× vs 下界 1.40×；斜率 0.132 µs/KiB；256 KiB 请求体单独 +60.5 µs（§2.5）；单次全量拷贝 0.118 µs/KiB | 256 KiB→1 MiB 场景省 **~120 µs**（去掉请求侧 2 次 + 响应侧 1 次全量拷贝）；分配字节 4.20× → ≈1.6×；小 body 场景省 ~2 µs | **中**。`ProviderRequest.payload` / `ProviderResponse.body` 换成 `Bytes` 要动协调者独占的 `types.rs`，五个 executor 全要跟 |
| 2 | **`StreamUsageBuffer` 每 chunk 全量复制进 tail** | `streambuf.rs:76` `self.tail.extend_from_slice(p)`；`:79` `self.tail.drain(..drop)`（tail 超 128 KiB 时 memmove 64 KiB）；`common.rs:423` `buf.write(&chunk)`；`common.rs:416` `(meta.parse)(&buf.bytes())` 收尾再拼一次。**实测**：SSE 分配 0.937× 流量 vs 下界 0.107×（+425 KB/请求）；每 chunk +0.585 µs | 改成保留**最后 N 个 `Bytes` 句柄**（`VecDeque<Bytes>` + 累计字节数）而不是复制字节：SSE 额外分配 425 KB/请求 → 接近 0；每 chunk 省 **~0.4 µs**，500 chunk 的流省 **~200 µs** | **中**。`bytes()` 的语义（拼出可解析的 head + `\n` + tail）必须保住；`streambuf.rs` 归 provider-openai |
| 3 | **幂等的全量响应缓冲** | `hold.rs:871` `capture_body`；`:894` `to_bytes(body, IDEMPOTENCY_BODY_CAPTURE_LIMIT)`（10 MiB 上限）；`:516` `cached.body = body.to_vec()`；`idempotency.rs` 再 base64 编码进 Redis 值。**实测**：1 MiB 响应 **+947 µs**、**+4.90 MB**（≈4.7× 响应体，§2.6） | 大响应场景省 **~900 µs**。可选做法：只缓冲 ≤ N KB 的响应（超了标 `truncated`，代码已有这条路径），或缓冲时复用同一份 `Bytes` 不再 `to_vec` + base64 | **中–高**。改的是幂等重放语义，要和计费不变量一起想 |
| 4 | **`ensure_include_usage` 的 serde_json 往返**（仅流式请求） | `common.rs:226` `from_slice::<Value>(payload)`（整树构建：每个 key/value 一次 String 分配）；`:238` `to_vec(&Value::Object(body))`。**实测**：256 KiB 流式请求 **+104.8 µs = 0.409 µs/KiB**；1 KiB 时在噪声内（§2.5）。profile 里 serde_json 占有效 CPU 2.35%（对照 0%） | 改成字节级探测 + 定点插入（确认 `"stream":true` 后在对象末尾插 `"stream_options":{"include_usage":true}`），省 **≈ 全部 105 µs**（256 KiB）；小 body 省 ~1–2 µs | **低–中**。要保住幂等性与「body 里没写 `stream:true` 就不动」的语义；`common.rs` 归 provider-openai |
| 5 | **`Dispatcher::auths_for` 每请求克隆整份凭证表** | `routes.rs:153–164`：`auth_store.list().await` 返回 `Vec<AuthRecord>` 再 `filter().collect()`。每条 `AuthRecord` 含 6 个 `String` + 3 个 `serde_json::Value`。本基线**只放了 1 条**凭证，所以它在 a) 的 149 次分配里只占 ~10 次 | **未单独实测**（凭证数 = 1 时看不出来）。按结构推断：生产上 N 条凭证 → 每请求 ~10×N 次分配 + N 次 JSON 树深拷贝；50 条凭证 ≈ 500 次分配/请求，会**超过当前全部开销**。改成 `ArcSwap<Arc<[AuthRecord]>>` 快照 + 按 provider 预分组 → 每请求 0 次分配 | **低**。改动全在 `Dispatcher` 内部 |
| 6 | **每请求一次 `getentropy` 系统调用**（`uuid::Uuid::new_v4()`） | `hold.rs:793`/`:800` `trace_id_from` → `Uuid::new_v4().to_string()`。**实测**：profile 里 `getentropy` 占有效 CPU **1.93%**，对照组 **0%**；调用方在调用图里直接可见 | 换成每线程 `ChaCha8Rng`（`rand` 已在依赖里）或「进程随机前缀 + 原子计数」：省掉一次 syscall + 一次 36 字节 String 格式化，小 body 场景约 **1–2 µs**。request_id 只是 trace id + hold key，不需要密码学随机性 | **低** |
| 7 | **HeaderMap 被克隆三遍** | `routes.rs:617` `req.headers().clone()`（每请求）；`routes.rs:215` `inbound.headers.clone()`（**每次上游尝试**一次）；`openai.rs:111` `copy_outbound_headers` 再逐条 `name.clone()` + `value.clone()`。profile 里 http/HeaderMap 相关帧 3.82% vs 对照 2.20% | 小 body 场景约 **10–20 次分配、1–3 µs** | **低–中**（同样触及 `types.rs`）。`ProviderRequest.headers` 改成 `Arc<HeaderMap>`，或让 executor 直接借用 |
| 8 | **每请求一堆小 String / HashMap** | `routes.rs:71` `model.to_ascii_lowercase()` + `Vec<&str>`；`hold.rs:719` `infer_provider` 又 lowercase 一次；`routes.rs:315` `request_metadata` 建 HashMap + 4 个 String（**每次上游尝试**）；`hold.rs:191` `meta.user_id.to_string()`；`hold.rs:181` 与 `routes.rs:616` 各 `path().to_owned()` 一次；`SettleCtx` 的 5 个 String 被 clone 进 `BillingHandle` 再 clone 进 `StreamSettler`。**实测**：access+hold 净成本 = +66 次分配 / +9.3 KB / +4.5 µs | 逐条消除（`Cow`/`&str`/`SmallVec`/`ArcStr`）约省 **40–60 次分配、3–5 µs** | **低**，但条数多、收益分散 |
| 9 | **hold 的请求体全量缓冲**（`peek_request_body`） | `hold.rs:847` `to_bytes(body, HOLD_REQUEST_BODY_LIMIT)`；`:862` `Body::from(bytes.clone())`（`Bytes` clone 只加引用计数，不拷贝）。注意 **nomw 也缓冲**（`routes.rs:626` 自己读 body），因为 `ProviderRequest.payload` 要完整 body | **不是可以删掉的开销**：只要计费要读 `model`/`stream`/`max_tokens`，请求体就必须落地一次。真正可删的是它之后的两次拷贝（第 1 条）。若 gw-relay 想做真流式请求体，就必须接受「只 peek 前 N KB 猜 model」这个折中 | **高**（改的是计费语义的输入前提） |

### 3.1 查过、**不是**问题的（别再花时间）

* **reqwest client 复用 / 连接池** —— 没问题。`common.rs:86` `shared_client()` 是
  `OnceLock`，五个 executor（`openai` / `claude` / `codex` / `gemini` / `vertex`）全部用它；
  `new_http_client()`（`common.rs:104`）**零调用者**（grep 证实）。池配置
  `pool_max_idle_per_host(100)` + `pool_idle_timeout(90 s)`。非流式请求用
  `RequestBuilder::timeout` 逐请求限时，没有为了超时另建一个 client 而分裂连接池。
  **唯一缺口**：HTTP/2 只在 TLS ALPN 时启用，本基线是明文 http，**h2 路径未覆盖**。
* **`tokio::spawn` / `TaskTracker` 的每请求开销** —— 不存在。`routes.rs:531`
  的 `TaskTracker::spawn` 只在 `StreamSettler::drop` 且尚未 `finish` 时发生，也就是
  **客户端中途断流**才走。正常路径 0 次 spawn；`record_success` / `record_failure`
  都是直接 await。这条从清单里划掉。

---

## 4. 11.3 µs 与 179.2 µs 是怎么构成的（把口径对上）

小 body 场景网关比下界多 **149 次分配**、多 **11.3 µs**，折合 **76 ns/次分配**。
profile 里分配器占有效 CPU 25.3%（下界 16.3%）—— 两个口径互相印证：
**a) 场景的开销主体是分配次数，不是字节搬运**（1 KiB + 2 KiB 一共才 3 KB，
按 0.132 µs/KiB 只值 0.4 µs）。

反过来 b) 场景：+158 次分配（与 a) 的 +149 几乎相同，说明「次数」这部分是固定成本），
却多花 179.2 µs —— 差额 168 µs 全部来自 **+3.67 MB 的字节搬运**。

**结论：gw-relay 要同时打两个靶子 —— 小请求打分配次数，大请求打拷贝次数。
只做零拷贝不减分配，小请求一分钱好处都拿不到；只减分配不做零拷贝，大请求纹丝不动。**

---

## 5. gw-relay 的验收数字目标

全部在**本装置**上验收（`./scripts/perf/run-baseline.sh`，同一台机器、同一轮交错），
全部以 **p50 差值（gw-relay − floor）** 为准 —— 绝对值受后台负载影响，差值不受。

| # | 指标 | 当前 | **目标** | 依据 |
| ---: | --- | ---: | ---: | --- |
| T1 | 非流式 1 KiB→2 KiB，网关自身开销 p50 | 11.3 µs | **≤ 4.0 µs** | 开销 ≈ 76 ns × 分配次数（§4）。做到 T2 的 110 次即 ≈ (110−84)×76 ns ≈ 2.0 µs，留一倍余量 |
| T2 | 同上，每请求堆分配**次数** | 233 次 | **≤ 110 次** | 下界 84 次。一个要做路由 + 选凭证 + 转发 header 的中继，合理增量是 ~25 次，不是 149 次 |
| T3 | 同上，每请求堆分配**字节** | 56.1 KB | **≤ 36 KB** | 下界 29.8 KB + 3 KB 载荷 + 余量 |
| T4 | 大 body 斜率（网关开销 ÷ 载荷） | 0.132 µs/KiB | **≤ 0.030 µs/KiB** | 单次全量拷贝实测 0.118 µs/KiB（§2.5）。目标即「总共不到 1/4 次拷贝」；真零拷贝应为 0 |
| T5 | 分配字节 ÷ 载荷字节（256 KiB→1 MiB） | 4.20× | **≤ 1.6×** | 下界 1.40×，留 0.2× 余量 |
| T6 | 256 KiB→1 MiB，网关自身开销 p50 | 179.2 µs | **≤ 50 µs** | T1 的 4 µs + T4 的 0.030 × 1 280 KiB = 42 µs，取整留余量 |
| T7 | SSE 建流固定成本（c-0，28 000 样本） | +11.7 µs | **≤ 4.0 µs** | 与 T1 同源：建流路径的固定开销不该比非流式高 |
| T8 | SSE 每 chunk 额外开销（c-1 满速） | +0.585 µs | **≤ 0.15 µs** | 去掉 `StreamUsageBuffer` 的逐 chunk 复制后，只剩一次 `Bytes` 引用计数 + 一次 stream 状态机 |
| T9 | SSE 分配字节 ÷ 流量字节 | 0.937× | **≤ 0.15×** | 下界 0.107×。**这一条是「流式到底零拷贝没有」的判据** |
| T10 | 256 KiB 流式请求的 JSON 重写代价 | +104.8 µs | **≤ 10 µs** | 字节级插入不构建 `Value` 树，代价应与 body 大小基本无关 |
| T11 | 1 MiB 响应 + `Idempotency-Key` 的额外开销 | +947 µs | **≤ 100 µs** | 幂等缓冲复用同一份 `Bytes`（不 `to_vec`、不 base64）后只剩一次 Redis 写 |
| T12 | 每请求 `getentropy` 系统调用次数 | 1 次 | **0 次** | profile 里可直接验证：该帧应从 top-list 消失 |
| T13 | 吞吐（concurrency 16）相对下界 | 74.0% | **≥ 90%** | T1–T3 达标后固定开销降到 1/3，排队放大随之收敛 |

### 5.1 验收怎么跑

```bash
./scripts/perf/run-baseline.sh          # 全部七档
python3 scripts/perf/summarize.py       # T1–T11、T13 的表
python3 scripts/perf/profile-summary.py # T12（getentropy 行应为 0%）
```

接法：`gw-relay` 是库（`RelayEngine`）不是 router，所以在
`perfkit/src/bin/gateway.rs` 里加一个 `PERF_MODE=relay` 分支，把 `RelayEngine`
包成一个 axum handler 挂在 `/v1/chat/completions` 上；再在 `run-baseline.sh` 的
`start_stack` 里多起一个端口（建议 18088 / admin 18098），并在几处
`for t in floor full nomw` 里加上 `relay`。`summarize.py` 的 `TARGETS` 加一项即可出表。

**验收时机**：`gw-relay` 目前仍允许 `todo!()`（骨架期，见其 `lib.rs` 的棘轮说明），
本文**没有**测它 —— 半成品的数字会误导。等它清空 `todo!()` 上棘轮之后再按 T1–T13 验收。

### 5.2 这些目标**不**覆盖什么

* Postgres / Redis 的真实 RTT（§1.4）。gw-relay 若改动 hold/settle 的调用次数，
  必须另开一份带真 Redis 的对照。
* HTTP/2 与 TLS。生产上游走 h2，本基线全程 HTTP/1.1。**上线前必须补一档 TLS+h2 的
  对照**，否则 T4/T5 在 h2 的分帧开销下会失真。
* 跨账号 failover 的重试路径（凭证池只有 1 条且从不失败），以及 §3 第 5 条的
  `auths_for` 随凭证数线性放大 —— 那条**没有实测**，只有结构推断。
* 干净压测机上的绝对产能（§2.8 的说明）。

---

## 6. 复现

```bash
./scripts/perf/run-baseline.sh                          # 全量（七档），约 20 分钟
ROUNDS=1 SSE_ROUNDS=1 ./scripts/perf/run-baseline.sh    # 快跑一遍确认装置能动
PHASES=latency ./scripts/perf/run-baseline.sh           # 只跑某一档

python3 scripts/perf/summarize.py --raw          # 汇总表 + 逐轮原始值
python3 scripts/perf/profile-summary.py --save   # CPU 热点表
```

装置细节、七个档的含义、单独起进程调试的办法见
[`scripts/perf/README.md`](../scripts/perf/README.md)。

`scripts/perf/perfkit` 是一个**独立 workspace**，通过 `path` 依赖**只读引用**
`crates/gw-proxy` / `gw-provider` / `gw-authcore`。根 `Cargo.toml` 与 `crates/**`
一个字没动，`cargo build --workspace` 也编不到它。
