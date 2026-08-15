# gw-relay 验收：T1–T13 实测（wave 3）

> **本文的一半数字不能用于验收判定，请先读 §1。**
>
> 采集时本机 load average 在 **13 → 245** 之间（12 核），同时有 5 个 `rustc` 与
> 12 个 `cargo` 在跑（另外两个 worker 正在改 `crates/**`，协调者也在跑全量
> `cargo check` / `cargo test --workspace`）。**对照组 `floor` 自己**在其中三个
> 档相对 wave 1 基线膨胀了 4 倍到 11 倍 —— 那三个档的数字发出去是有害的。
>
> 另外三个档 `floor` 复现到 wave 1 的 **2% 以内**，那部分数字是实的。哪些能用、
> 哪些不能用，逐档判定在 §2，判据是**对照组自己的复现度**，不是我的感觉。
>
> 装置：[`scripts/perf/`](../scripts/perf/README.md)（一条命令复现）。
> 原始数据：`scripts/perf/results-wave3/`。基线与 T1–T13 的定义：
> [`docs/relay-perf-baseline.md`](relay-perf-baseline.md) §5。

---

## 0. 一页结论

| | 结论 |
| --- | --- |
| **装置** | `relay` 被测端接线完成，与 `floor` / `full` / `nomw` 同轮交错跑；另补了基线 §5.2 点名的两个未覆盖档（failover 重放、TLS+h2），全部跑通 |
| **T1–T13** | 8 条数据可信（**5 条达标、3 条未达标**），5 条被负载污染或未覆盖，见 §3 |
| **最大的一条未达标** | **T6：256 KiB→1 MiB 非流式，relay 比 floor 慢 558.8 µs**（目标 ≤ 50 µs）。这一条数据可信、跨 5 轮稳定、且被三次独立测量复现 |
| **根因** | `RelayEngine::relay` 对**非流式响应整体缓冲**（`engine.rs:246` → `collect_frames`）。已定位到具体开销点，见 §4.1 |
| **最漂亮的一条** | **T9：SSE 分配字节 ÷ 流量字节 = 0.12×**（目标 ≤ 0.15，当前网关 0.937×）。**流式路径确实做到了零拷贝** |
| **权威验收** | **未完成。** 需要一个安静窗口重跑，条件见 §7 |

小 body 那一档 `gw-relay` 已经打赢了它要打的靶子：
**+3.2 µs / 105 次分配 / 35.3 KB**（当前网关 +11.3 µs / 233 次 / 56.1 KB，下界 84 次 / 29.8 KB）。
大 body 那一档没有 —— 而且方向是反的：**比今天的网关还慢 3 倍**。

---

## 1. 测量环境被污染了，以及污染有多大

### 1.1 环境

```
host:    Darwin 25.6.0 arm64 / Apple M4 Pro / 12 核
rustc:   1.97.1 (8bab26f4f 2026-07-14)
profile: release —— opt-level=3, lto="thin", codegen-units=1（与根 workspace 一致）
loadavg: 开跑前 13.01；跑到一半 76 → 102 → 122 → 150；协调者报告峰值 245
并发干扰: 5 个 rustc + 12 个 cargo（wave 3 的另外两个 worker + 协调者的全量校验）
采集:    2026-08-15T13:59Z 起，7 档
被测端:  floor + relay 两个（采集时 `gw-proxy` 编不过，`full`/`nomw`/`idem` 没起）
总量:    492 776 个请求，**stalls = 0**
```

### 1.2 污染有多大：拿**对照组自己**跟 wave 1 比

`floor` 是同一份二进制、同一套负载、同一台机器。它自己相对 wave 1 涨了多少，
就是这一档被负载污染了多少。**这是唯一不依赖判断的判据。**

| 档（**对照组 `floor` 自己**） | wave 1（loadavg ~10） | wave 3（loadavg 76–245） | 膨胀 |
| --- | ---: | ---: | ---: |
| 1) a) 1 KiB→2 KiB p50 | 53.9 µs | 53.8 µs | **1.00×** |
| 1) b) 256 KiB→1 MiB p50 | 273.8 µs | 267.7 µs | **0.98×** |
| 1) c-0) SSE 建流 p50 | 56.8 µs | 56.0 µs | **0.99×** |
| 1b) c-1) SSE 满速整流 p50 | 1 125.7 µs | 4 363.1 µs | **3.88×** |
| 1c) 1 KiB 非流式 p50 | 51.5 µs | 570.3 µs | **11.06×** |
| 1c) 256 KiB 非流式 p50 | 103.5 µs | 823.6 µs | **7.96×** |
| 3) 吞吐 rps（concurrency 16） | 68 836 | 3 897 | **0.06×（慢 17.7 倍）** |
| 2) 分配：a) 次数 / 字节 | 84.0 / 29 771 | 84.0 / 29 771 | **1.00×（逐位相同）** |
| 2) 分配：c) SSE 字节 | 55 090 | 55 080 | **1.0002×** |
| 2) 分配：b) 大 body 字节 | 1 829 601 | 1 092 326 | **0.60×** |

读法：**档 1 与档 2 的 a)/c) 是干净的，档 1b / 1c / 3 与档 2 的 b) 不是。**
档 1 跑在开头（loadavg 13），跑完之后负载才起来；档 1b / 1c / 3 正撞在峰值上。

> 分配次数为什么大多不受负载影响：它是确定性计数器，同一条代码路径跑同样的
> 请求就分配同样多次。**唯一的例外是 b) 大 body** —— 1 MiB 响应被 reqwest
> 切成几个 chunk 是由 socket 行为决定的，负载一变 chunk 数就变，于是分配次数跟着变
> （wave 1 的 104.3 次 → wave 3 的 93.3 次）。这一条 wave 1 写的
> 「两轮独立重跑完全复现」在重负载下**不成立**，本文把它订正过来。

### 1.3 干净的那部分为什么可信

档 1 五轮逐轮 p50（µs），floor / relay 交错跑：

| 场景 | r1 | r2 | r3 | r4 | r5 | 逐轮 Δ |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| a) floor | 53.8 | 55.2 | 53.8 | 52.5 | 52.7 | |
| a) relay | 57.7 | 58.1 | 56.6 | 57.0 | 44.4 | +3.9 / +2.9 / +2.8 / +4.5 / **−8.3** |
| b) floor | 280.1 | 267.7 | 267.4 | 263.9 | 269.2 | |
| b) relay | 820.0 | 826.5 | 823.5 | 829.6 | 846.2 | +539.9 / +558.8 / +556.1 / +565.7 / +577.0 |
| c-0) floor | 58.0 | 56.8 | 55.1 | 56.0 | 45.3 | |
| c-0) relay | 63.3 | 61.4 | 61.2 | 61.0 | 51.5 | +5.3 / +4.6 / +6.1 / +5.0 / +6.2 |

* b) 的逐轮差值离散度 ±3%（539.9–577.0，中位 558.8），c-0) 是 ±15%
  （4.6–6.2，中位 5.2）—— **这两条是实的**。
* a) 的 r5 是个异常轮（relay 44.4 µs，比 floor 还快 8.3 µs）—— 同一轮里 c-0) 的
  floor 也从 56 掉到 45.3，说明是**机器整体变快了一瞬**而不是 relay 变快了。
  跨轮取中位数正是为这种情况准备的：中位数落在 r4 的 57.0 上，未被 r5 拖偏。
  但它提醒一件事：**a) 档只有 +3.2 µs 的余量，异常轮再多两个就能翻盘**，
  重测时这一条要盯住跨轮 min–max。

---

## 2. 逐档可信度判定

| 档 | 对应 T | 判定 | 依据 |
| --- | --- | --- | --- |
| 1 latency a) | T1 | **可信（余量薄）** | floor 复现 1.00×；逐轮 Δ 稳定，但有 1 个异常轮 |
| 1 latency b) | T4 T6 | **可信** | floor 复现 0.98×；逐轮 Δ 离散 ±3% |
| 1 latency c-0) | T7 | **可信** | floor 复现 0.99×；逐轮 Δ 离散 ±15% |
| 1b sseburst | T8 | **污染** | floor 自己涨 3.88× |
| 1c jsonrewrite | T10 | **污染** | floor 自己涨 8–11×；算出来的重写代价是负数 |
| 2 alloc a) | T2 T3 | **可信** | floor 与 wave 1 **逐位相同** |
| 2 alloc c) SSE | T9 | **可信** | floor 与 wave 1 差 0.02% |
| 2 alloc b) 大 body | T5 | **污染** | floor 自己变了 40%（chunk 数随负载漂） |
| 3 throughput | T13 | **污染，且偏乐观** | floor 自己慢 17.7×。机器饱和时瓶颈是调度器不是网关，比值会**假性收敛到 100%** |
| 5 profile | T12 | **可信** | 判的是"有没有这个调用"，不是耗时 |
| 4 idempotency | T11 | **未覆盖** | `gw-relay` 里没有幂等，`hold::capture_body` 是 `gw-proxy` 的东西 |

---

## 3. T1–T13 逐条

命令：`RESULTS=scripts/perf/results-wave3 python3 scripts/perf/summarize.py --acceptance relay`

| # | 指标 | 目标 | 实测 | 达标? | 差多少 | 可信? |
| ---: | --- | ---: | ---: | :---: | ---: | :---: |
| T1 | 非流式 1 KiB→2 KiB，自身开销 p50 | ≤ 4.0 µs | **3.2 µs** | **达标** | 余 0.8 µs | 可信（余量薄） |
| T2 | 同上，每请求堆分配次数 | ≤ 110 次 | **105.0 次** | **达标** | 余 5.0 次 | 可信 |
| T3 | 同上，每请求堆分配字节 | ≤ 36 000 B | **35 325 B** | **达标** | 余 675 B | 可信 |
| T4 | 大 body 斜率（开销 ÷ 载荷） | ≤ 0.030 µs/KiB | **0.435 µs/KiB** | **未达标** | 超 14.5× | 可信 |
| T5 | 分配字节 ÷ 载荷字节（256 KiB→1 MiB） | ≤ 1.6× | 2.85× | — | — | **污染** |
| T6 | 256 KiB→1 MiB，自身开销 p50 | ≤ 50 µs | **558.8 µs** | **未达标** | 超 11.2× | 可信 |
| T7 | SSE 建流固定成本（c-0） | ≤ 4.0 µs | **5.2 µs** | **未达标** | 超 1.2 µs | 可信 |
| T8 | SSE 每 chunk 额外开销（c-1 满速） | ≤ 0.15 µs | 3.00 µs | — | — | **污染** |
| T9 | SSE 分配字节 ÷ 流量字节 | ≤ 0.15× | **0.122×** | **达标** | 余 0.028× | 可信 |
| T10 | 256 KiB 流式请求的 JSON 重写代价 | ≤ 10 µs | −142.7 µs | — | — | **污染**（负数即证据） |
| T11 | 1 MiB 响应 + `Idempotency-Key` 额外开销 | ≤ 100 µs | — | — | — | **未覆盖** |
| T12 | 每请求 `getentropy` 系统调用 | 0 | **0.00%** | **达标** | — | 可信 |
| T13 | 吞吐（concurrency 16）相对下界 | ≥ 90% | 94.0% | — | — | **污染，偏乐观** |

**可信的 8 条里：5 条达标（T1 T2 T3 T9 T12），3 条未达标（T4 T6 T7）。**

参照系（wave 1 实测的当前网关 `full`，同样以 floor 为零点）：

| # | 当前网关 `full` | 目标 | **`relay` 本轮** |
| ---: | ---: | ---: | ---: |
| T1 | +11.3 µs | ≤ 4.0 | **+3.2 µs** ✅ |
| T2 | 233 次 | ≤ 110 | **105 次** ✅ |
| T3 | 56.1 KB | ≤ 36 KB | **35.3 KB** ✅ |
| T6 | +179.2 µs | ≤ 50 | **+558.8 µs** ❌ **比现状还差 3.1 倍** |
| T7 | +11.7 µs | ≤ 4.0 | **+5.2 µs**（比现状好 2.2×，但仍未达标） |
| T9 | 0.937× | ≤ 0.15 | **0.122×** ✅ **好 7.7 倍** |
| T12 | 1.93% | 0 | **0%** ✅ |

---

## 4. 三条未达标的逐条分析

### 4.1 T4 / T6：大 body 慢 558.8 µs —— 非流式响应被整体缓冲

**这一条是本轮最重要的发现，而且它比现状更差，不是"还差一点"。**

先把开销落在哪一侧拆开。同一套装置，只换请求/响应大小（`json-*` 那一档的隔离手法，
但这一格是我为这个问题新加的，见 `run-baseline.sh` 的 `Q_RESP1M`）：

| 隔离实验 | floor p50 | relay p50 | Δ | 折算 |
| --- | ---: | ---: | ---: | --- |
| 1 KiB 请求 → 256 B 响应 | 53.1 | 56.9 | **+3.8** | 固定成本，与 T1 一致 |
| **256 KiB 请求** → 256 B 响应 | 103.5 | 132.3 | **+28.7** | 减去固定成本 = **+24.9 µs / 256 KiB = 0.097 µs/KiB** |
| 1 KiB 请求 → **1 MiB 响应** | 236.8 | 734.3 | **+497.5** | 减去固定成本 = **+493.7 µs / 1 MiB = 0.482 µs/KiB** |

> 这三格是在一个较安静的窗口（loadavg ~18）用手工脚本单跑的（2 轮 × 600 发，
> 原始数据 `scripts/perf/results-wave3/isolation/`），floor 的绝对值
> （53.1 / 103.5 / 236.8）与 wave 1 基线（53.9 / 103.5 / —）对得上，
> 所以这三格本身没被污染。
> 第三格现在已经固化进 `phase_jsonrewrite`（`json-resp1m-*`），下一轮自动出，
> 不必再手工跑。

**请求侧 0.097 µs/KiB ≈ 一次全量拷贝**（wave 1 实测单次拷贝 0.118 µs/KiB），
而今天的网关请求侧是 0.236 µs/KiB（两次）—— **请求侧 `Bytes` 化生效了，省了一半。**

**响应侧 0.482 µs/KiB ≈ 四次全量拷贝的量级，而代码里只有一次拷贝。**
差额来自别的东西。分配计数印证只有一次：

| 场景 | floor 分配字节/请求 | relay | Δ | 载荷 |
| --- | ---: | ---: | ---: | ---: |
| b) 256 KiB→1 MiB | 1 092 326 | 3 732 762 | **+2 640 436** | 1 310 720 |

Δ ≈ 2.0× 载荷，即请求体与响应体各被额外整搬了一遍 —— 与"一次 `join()`"吻合。
按 0.118 µs/KiB 折算，**拷贝只值 ≈ 152 µs**。剩下的 **≈ 400 µs 不是拷贝**。

**根因**（`crates/gw-relay/src/engine.rs:243-248`）：

```rust
let body = if is_event_stream(&head.headers) {
    RelayResponseBody::Stream(watch_frames(...))       // 流式：逐帧转发
} else {
    collect_frames(head.body, probe, budget).await     // 非流式：整体收完再返回
};
```

非流式响应**收完最后一个字节才返回响应头**。于是：

1. **上游读与客户端写不再重叠。** `floor` 是 `bytes_stream()` 直接接到出站 body 上，
   1 MiB 边收边发；`relay` 先收满 1 MiB，再发 1 MiB。同一份 IO 从并行变串行 ——
   这是那 400 µs 的主要来源，而且它**不随拷贝优化而消失**。
2. `collect_frames` 每一帧一次 `timeout_at`（`engine.rs:368`），1 MiB 被 reqwest 切成
   几十帧就是几十次 tokio 定时器注册/注销。
3. `join()` 的一次 1 MiB 拷贝（`body.rs:198`）。

**这个设计不是疏忽，是有意的**：非流式的 usage 在完整 JSON 的末尾，
`UsageProbe` 必须拿到整份 body 才解析得出来（`engine.rs:387` 的 `guard.observe(&whole)`）。
所以「非流式响应不缓冲」与「非流式也要精确计费」目前是互斥的。

**我没有改它** —— `crates/gw-relay/**` 不是我的文件，而且这是一个契约级取舍
（缓冲换计费精度），该由协调者决定。可能的方向，按代价排序，供决策：

| 方案 | 代价 | 能拿回多少 |
| --- | --- | --- |
| 帧边转发边喂 probe，probe 自己攒完整 body（只在非流式路径） | probe 多一份 body 的常驻，但 IO 重新并行 | 那 ≈ 400 µs |
| 只在 `content-length` 小于阈值时缓冲，超阈值转流式 + usage 落 fallback | 大响应计费降级（与缺陷 #2 的取舍同构） | 大 body 全部 |
| 保持缓冲，但把 `timeout_at` 挪到循环外（一次 deadline 而不是每帧一次） | 无 | 几十 µs，治标 |

**在此之前，b) 档（大 body 非流式）上线会比今天慢。**

### 4.2 T7：SSE 建流固定成本 +5.2 µs（目标 ≤ 4.0）

差 1.2 µs，跨 5 轮稳定（+5.3 / +4.6 / +6.1 / +5.0 / +6.2）。比当前网关的 +11.7 µs
好 2.2 倍，但没到线。

它比 T1 的 +3.2 µs 多出来的 2.0 µs，是流式路径独有的那几步：`is_event_stream`
的 content-type 判定、`watch_frames` 的 `stream::unfold` + `ProbeGuard`（一个
`Mutex<Option<Box<dyn UsageProbe>>>` 的堆分配）、以及每帧一次 `timeout` 定时器。
**没有单独隔离测量过这三者各占多少** —— 这一条只有结构推断，不要当实测。

### 4.3 T1 的余量只有 0.8 µs

达标了，但 4.0 的目标里只剩 20% 余量，而 §1.3 那个异常轮说明这一档的跨轮抖动
能到 8 µs 量级。**重测时如果 min–max 跨度超过 ±3 µs，这一条要按"未判定"处理，
而不是按中位数报达标。**

---

## 5. 补的两个档（基线 §5.2 点名的未覆盖项）

### 5.1 跨账号 failover 的重放代价 —— `Bytes` 化到底省了多少

基线 §1.4：「不含跨账号 failover 的重试路径（凭证池只放了 1 条，且从不失败）」。

装置（`PHASES=failover`）：上游按 `?fail_first=N` 对前 N 次尝试回 429（带
`retry-after: 12` 与 `x-ratelimit-*`），被测端带 `x-perf-attempt` 头重试到成功。
**两个 body 模式跑同一个二进制、同一条代码路径，只差一行**：

* `bytes` —— `payload.clone()`，`Bytes` 的 refcount 加一，零字节拷贝；
* `vec` —— `Bytes::copy_from_slice(&payload)`，复刻今天 `routes.rs:217` 的
  `inbound.body.to_vec()` 落在 failover 循环体内（审计缺陷 #13）。

256 KiB 请求体，每档 300 发（分配档）/ 400 发（延迟档）：

| 尝试次数 | `bytes` 分配字节/请求 | `vec` 分配字节/请求 | **Δ** | Δ ÷ 256 KiB |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 796 617 | 1 077 452 | **280 835** | **1.07×** |
| 2 | 845 586 | 1 379 967 | **534 380** | **2.04×** |
| 4 | 915 647 | 1 966 410 | **1 050 763** | **4.01×** |

**结论：`Bytes` 化省下的正好是「请求体大小 × 尝试次数」的 memcpy，一次不多一次不少。**
审计报告 #13 估的「900 KB 请求在 3 次 failover 下要 memcpy 约 5.4 MB」，
按这个斜率折算是 900 KB × 3 = 2.7 MB 的**请求侧**部分 —— 数量级对得上。

顺带一个本来没打算量到的数：`bytes` 模式自己也随尝试次数涨
（796 KB → 845 KB → 916 KB，每多一次尝试 +40~70 KB）。那是**每次尝试重建
`HeaderMap` + reqwest 建请求**的成本，与 body 无关，`Bytes` 化管不着它。

> **延迟列不可用。** 实测 Δp50 是 +10.6 / −9.0 / −1.4 µs，符号都不稳定 ——
> 一次 256 KiB memcpy 只值约 30 µs，在 loadavg 150 的噪声底下量不出来。
> 分配计数是确定性的，所以上表用分配列下结论，**没有用延迟列**。

### 5.2 TLS + HTTP/2

基线 §5.2：「上线前**必须**补一档 TLS+h2 的对照，否则 T4/T5 在 h2 的分帧开销下会失真」。

装置（`PHASES=tls`）：mock 上游用自签证书跑 https，ALPN 给 `["h2","http/1.1"]`；
`floor` 与 `relay` 都换成放行自签证书的 client。**客户端一侧（loadgen → 被测端）
仍然是明文 HTTP/1.1** —— 要量的是上游那一跳。

`relay` 侧换的是 `Transport`（`RelayEngine::with_transport`），池配置逐字照抄
`gw-relay` 的 `shared_client()`（connect 超时、每 host 100 条空闲、90 s 回收、
TCP keepalive 60 s、h2 心跳 30/20 s + while_idle），**只多一条
`danger_accept_invalid_certs(true)`**。理由：`ReqwestTransport` 的 client 由
`shared_client()` 建，没有放行自签证书的口子，而 mock 只能用自签证书。

**h2 真的协商上了**，不是假设 —— mock 收到第一发请求时打一行：

```
mock-upstream listening on https://127.0.0.1:18071 (ALPN h2/http1.1)
mock-upstream: first request arrived over HTTP/2.0
```

（明文档同一份二进制打的是 `HTTP/1.1`。这一行是这一档唯一的自检：证书装上了、
连接建起来了，都**不**保证 ALPN 没退化成 http/1.1。）

**这一档的数字本轮不可用**：它在 loadavg 150 的窗口跑，且只跑了 1 轮 ×
少量请求（目的是验证装置能跑通）。装置就位了，数字下一轮补。

它要回答的问题写在这里，免得下一轮忘了：**`Δ(TLS+h2) − Δ(明文 h1)` 是否接近 0。**
接近 0 → 明文档的 T4/T5 结论能外推到生产；不接近 → h2 的分帧把开销结构改了，
T4/T5 必须在 h2 下重定。

已知不可比之处：证书校验被关掉了（自签），所以这一档**不含证书链验证成本**。
它每连接一次、keep-alive 下摊到近零，但记在这里免得数字被当成含它。

---

## 6. 装置自身的缺陷（如实记录）

沿用 wave 1 §2.9 的标准：装置有什么毛病就写什么毛病。

1. **本轮 492 776 个请求，`stalls = 0`。** wave 1 那 2 次未定位的单请求超时
   （发生率 ~1/2 800 000）本轮没有复现 —— 样本量比 wave 1 小一个数量级，
   **不能当成它被修好了**。
2. **`splice_include_usage` 走的是 chunked，`Buffered` 走的是 `content-length`。**

   证据（把 relay 的上游指到一个 `nc` 监听口，dump 它实际发出去的字节）：

   ```text
   POST /v1/chat/completions HTTP/1.1
   accept-encoding: identity                      <- 客户端发的是 gzip，被收敛（缺陷 #5）
   content-type: application/json
   authorization: Bearer perf-upstream-key        <- 客户端的 Bearer 被换掉（审计 §4.1 #1）
   host: 127.0.0.1:18099
   transfer-encoding: chunked                     <- **没有 content-length**
                                                  <- 客户端的 x-api-key 被剥掉了（观察 #17）
   29
   {"stream_options":{"include_usage":true},      <- 新分配的 41 字节
   2E
   "model":"gpt-4o","stream":true,"max_tokens":8} <- 原 Bytes 的零拷贝切片，46 字节
   0
   ```

   两个 chunk 就是「零拷贝定点插入」的直接证据：插入段单独一帧，原 body
   一个字节都没搬。代价也在同一张图里 —— `transfer-encoding: chunked`。

   `relay` 在流式请求上要把 `Spliced` 的两段字节拼成帧流发出去（这是"零拷贝插入"
   的代价），而 `RelayBody::Streaming` 没有确定长度，于是上游收到的是 chunked；
   非流式那条路是 `Buffered(Bytes)`，reqwest 会带 `content-length`。
   `floor` 两条路都是 chunked（`wrap_stream`）。所以 1c 档里
   「流式 vs 非流式」的差里**混进了一点传输编码的差异**（单帧 chunk 头，约几十字节
   + 一次多余的 write）。量级远小于要测的 100 µs，但它在那里。
   根因是 `headers::is_locally_rebuilt` 把 `content-length` 无条件剥掉、
   由传输层重算，而 `Spliced::len()` 算好的长度**目前没有通路交给 `RelayEngine`**。
   这是 `gw-relay` 的一个真实缺口，不只是装置问题。
3. **`fail_first` 档的重试判据是被测端自己打的 `x-perf-attempt` 头**，
   不是上游的计数器。好处是并发与预热互不污染、重跑可复现；
   代价是它测不出"上游按真实速率限流"这种时序相关的场景。
4. **TLS 档关掉了证书校验**（见 §5.2）。
5. **`profile` 档的百分比在重负载下不可比**：relay 的 park 占到 53%，
   说明采样窗口里线程大量在等 CPU 而不是等活。**`getentropy = 0%`（T12）
   不受影响** —— 它判的是"这个调用在不在调用图里"。

---

## 7. 权威验收要什么条件

本轮**没有**完成权威验收。下一轮请按这个条件派：

| 条件 | 值 | 为什么 |
| --- | --- | --- |
| **loadavg 上限** | 开跑前 **≤ 4**（12 核），全程 **≤ 12** | wave 1 在 loadavg ~10 下的 floor 是可复现的；本轮 76+ 时 floor 自己涨 11 倍。留一半余量 |
| **无并发编译** | 全程 0 个 `rustc` / `cargo` | 编译是本机最大的干扰源，且它抢的正是 tokio worker 要的核 |
| **被测端** | `floor` + `nomw` + `full` + `relay` 四个**同轮交错** | 只有四个同轮跑，`relay vs full` 才是同一把尺子量出来的。本轮只有两个（采集时 `gw-proxy` 编不过） |
| **重复次数** | `ROUNDS=5`、`SSE_ROUNDS=3`（默认值，**不要砍**） | 跨轮取中位数对偶发长尾免疫，砍到 1 轮就不免疫了 |
| **验收判据** | 每一档先看 **floor 自己**跨轮 min–max 的跨度 | 跨度 > ±5% 的档直接判"未判定"，不要报中位数 |
| **档** | `latency sseburst jsonrewrite alloc throughput idempotency profile`，再单跑 `failover` 与 `tls` | 后两个要重启栈，混进来会打断"同轮交错" |
| **预计耗时** | 四被测端全量约 **40–60 分钟**，加两个补档约 **75 分钟** | |

跑法：

```bash
./scripts/perf/run-baseline.sh                                   # 七档，四被测端
PHASES=failover ./scripts/perf/run-baseline.sh                   # 补档 1
PHASES=tls      ./scripts/perf/run-baseline.sh                   # 补档 2
python3 scripts/perf/summarize.py --acceptance relay             # T1–T13 表
python3 scripts/perf/summarize.py                                # 全部明细表
python3 scripts/perf/profile-summary.py                          # T12
```

`summarize.py --acceptance <被测端>` 会自己算出 13 行的
`[目标 | 实测 | 达标/未达标 | 差多少]`，缺数据的行打「未覆盖」而**不是**打 0。
自检：`--acceptance full` 跑 wave 1 的 `results/`，13 行应当逐个复现
`docs/relay-perf-baseline.md` §5 的「当前」列（已验证：11.3 / 233 / 56 117 /
0.131 / 4.20 / 179.2 / 11.7 / 0.59 / 0.94 / 104.8 / 947.3 / 1.93% / 74.0%）。

---

## 8. 装置改了什么（wave 3）

`scripts/perf/**` 之外**一个字没动**。

| 文件 | 改了什么 |
| --- | --- |
| `perfkit/src/bin/relay.rs` | **新增**。`RelayEngine` 包成 axum handler，挂三个入口；含 failover handler 与 TLS `Transport` |
| `perfkit/src/bin/mock-upstream.rs` | `fail_first` 参数（回 429，带 `retry-after`）；TLS + h2 监听；首发请求的 HTTP 版本自检 |
| `perfkit/src/bin/floor.rs` | `PERF_TLS=1` 放行自签证书 |
| `perfkit/src/lib.rs` / `stubs.rs` | `PERF_API_KEY` 上收到 crate 根（`stubs` 现在在 `gateway` feature 下） |
| `perfkit/Cargo.toml` | `gw-relay` 依赖；`gw-proxy`/`gw-provider`/`gw-authcore` 挂到 `gateway` feature（默认开）；`axum-server` + `rustls` 供 TLS 档 |
| `run-baseline.sh` | 被测端列表一处定义（`TARGETS`）+ `port_of`/`admin_of`；`relay` 端口 18088/18098；`PERF_NO_GATEWAY`；`phase_failover`；`phase_tls`；profile 档按被测端循环；`json-resp1m` 那一格 |
| `summarize.py` | `relay` 被测端；`--acceptance` 出 T1–T13 表；1c 档的汇总函数（原来是手算的）；failover 表；TLS 表 |
| `profile-summary.py` | 按被测端列出 profile（`profile-<t>.txt`），缺的写明"跳过"而不是当 0 |

`gateway` feature 那一条值得单说：`gw-proxy` / `gw-provider` / `gw-authcore`
三条依赖现在是可选的。多 worker 并行改 `crates/**` 时工作区中间态必然编不过，
`PERF_NO_GATEWAY=1` 就能只构建 `floor` / `relay` / `mock` / `loadgen`、
照样跑 relay 对 floor 的那几档 —— 本轮正是这么跑起来的。
**少了哪些被测端会写进 `env.txt` 的 `targets:` 行**，不会变成一张看起来完整的空表。
