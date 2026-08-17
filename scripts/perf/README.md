# scripts/perf —— gw-relay 性能基线与验收装置

一条命令跑完全部测量：

```bash
./scripts/perf/run-baseline.sh                  # 全量（七档），约 20~40 分钟
python3 scripts/perf/summarize.py               # results/*.json → markdown 表
python3 scripts/perf/summarize.py --acceptance relay   # T1–T13 逐条 达标/未达标
python3 scripts/perf/profile-summary.py         # CPU 热点按类别归因
./scripts/perf/hotpath-flamegraph.sh           # 真内核 + mock 上游 → docs/hotpath-flamegraph.svg
```

wave 1 的基线结论在 [`docs/relay-perf-baseline.md`](../../docs/relay-perf-baseline.md)，
wave 3 的 T1–T13 验收结果在 [`docs/relay-perf-acceptance.md`](../../docs/relay-perf-acceptance.md)。
本 README 只讲**怎么跑**和**装置是怎么搭的**。

---

## 1. 它测什么

五个被测端同时在跑（空闲不耗 CPU），同一份 mock 上游、同一套负载：

| 被测端 | 端口 | 是什么 |
| --- | --- | --- |
| `floor` | 18082 | **理论下界**：约 60 行的 axum 纯 `Bytes` 反代，请求/响应双向流式，零解析零计费 |
| `nomw`  | 18084 | 真 `gw_proxy::routes::chat_completions`，**不挂** access / hold 两层中间件 |
| `full`  | 18080 | 真 `gw_proxy::router()`，生产拓扑（access → hold → dispatch） |
| `idem`  | 18086 | 同 `full`，额外挂上幂等管理器（量 `hold::capture_body` 的全量响应缓冲） |
| `relay` | 18088 | **wave 3 新增**：`gw_relay::RelayEngine` 包成 axum handler（`perfkit/src/bin/relay.rs`） |
| mock 上游 | 18081 | 见下 |

**网关自身开销 = `full` 实测 − `floor` 实测。**
**access+hold 净成本 = `full` − `nomw`。**
**gw-relay 内核开销 = `relay` − `floor`** —— T1–T13 全部按这个差值验收。

> `relay` 里**没有** access / hold / 幂等 / 限流 / 熔断 / 凭证池，所以结构上它可比的是
> `nomw` 而不是 `full`。T1–T13 是照着 floor 定的，验收也照着 floor 报，
> 但读数时要记得 `full − nomw` 那一段中间件成本它一分钱都没付。

被测端列表由脚本顶部的 `TARGETS` 一处定义、每个档从它展开。加一个被测端
只改那一行 + `port_of` / `admin_of` 两个 `case`，不会出现"某个档忘了加"这种
只在汇总表里显示为空行的错误。

负载形态：

| 代号 | 形态 | 目的 |
| --- | --- | --- |
| a) `small` | 1 KiB 请求 / 2 KiB 响应，非流式 | 网关固定开销 |
| b) `large` | 256 KiB 请求 / 1 MiB 响应，非流式 | 拷贝开销随 body 增长的斜率 |
| c) `sse` | 500 chunk × 1 KiB × 1 ms | 流式中继开销、TTFB、chunk 间抖动 |
| c-0) `ssettfb` | 1 chunk，无间隔 | 只测"建流"这一步的固定成本，样本量大，p99 才可信 |
| c-1) `sseburst` | 500 chunk × 1 KiB，**间隔 0** | 1 ms 那一档量不出每 chunk 成本（mock 定时器在本机被放大到 ~2.35 ms，把个位数 µs 埋掉了），这一档把定时器拿掉 |
| 1c) `json*` | 同一 body 只切 `stream` 真假，响应压最小 | 隔离 `ensure_include_usage` / `splice_include_usage` 的请求体重写代价 |
| 6) `fo-*` | 256 KiB 请求，上游前 n−1 次回 429 | 跨账号 failover 的重放代价：`Bytes::clone` vs 全量拷贝 |
| 7) `tls-*` | 同 a/b/c-0，但**上游那一跳走 https + h2** | §5.2 的第一个未覆盖项：h2 分帧下差值会不会失真 |

## 2. 为什么不用现成的东西

**mock 上游没用 `crates/gw-proxy/src/testsupport/upstream.rs`。**
那里的 `FakeProvider` 是 `Provider` trait 的**进程内替身**，直接返回
`ProviderResponse` —— 整条路径上没有 reqwest、没有 socket、没有 HTTP 编解码、
没有 SSE 时间轴。用它量出来的"转发开销"会漏掉本任务最关心的三样：真实的
reqwest 客户端与连接池、body 在 `Bytes`/`Vec<u8>` 之间的真实搬运、以及流式
中继的首字节延迟。而且它是 `#[cfg(test)] pub(crate)`，crate 外引用不到。
所以 `perfkit/src/bin/mock-upstream.rs` 起了一个真 HTTP 上游。
**`testsupport` 一行未改，只作为只读参考。**

**负载生成器是自写的。** 本机没有 oha / wrk / bombardier / hey / k6（已确认），
而且它们都不给 **SSE 每 chunk 的到达时刻** —— 那是 c) 场景唯一诚实的量法。
`perfkit/src/bin/loadgen.rs` 是裸 TCP + 手写 HTTP/1.1，在每个 chunk 边界打时间戳，
连接全程 keep-alive 复用（量的是稳态开销，不含 TCP 握手）。

**分配计数是自己包的 `#[global_allocator]`，不是 dhat。** 要的是**每请求**增量，
即 `(计数差) / (请求数)`，而 dhat 给的是整进程堆快照；而且加 dhat 要动
workspace 的 `Cargo.toml`，CONTRACT §3 划给协调者独占。

## 3. 装置的所有权边界

`perfkit` 是一个**独立 workspace**（它自己的 `Cargo.toml` 里有个空
`[workspace]` 表），通过 `path` 依赖引用 `crates/gw-proxy`、`crates/gw-provider`、
`crates/gw-authcore`、`crates/gw-relay`。

`gw-proxy` / `gw-provider` / `gw-authcore` 三条依赖挂在 **`gateway` feature** 下
（默认开）。理由：`gateway` 被测端要拖进半个工作区，而 `relay` / `floor` /
`mock-upstream` / `loadgen` 只需要 `gw-relay` 或什么都不需要。多 worker 并行改
`crates/**` 时工作区中间态必然编不过，`PERF_NO_GATEWAY=1` 就能只构建后四个、
照样跑 relay 对 floor 的那几档 —— 而不是干等。少了哪些被测端会写进 `env.txt`
的 `targets:` 行，不会变成一张看起来完整的空表。

* 根 `Cargo.toml` 一个字没动 —— `members = ["crates/*", "tools/xtask"]` 匹配不到
  `scripts/`，`cargo build --workspace` 也不会编到它；
* `crates/**` 下一个 `.rs` 没动；
* 它的 `[profile.release]` 的 **codegen 部分**与根 workspace 逐字一致（`opt-level=3`、
  `lto="thin"`、`codegen-units=1`），否则"网关 vs 下界"的差值里会混进编译参数差异。
  **唯一差异是保留符号**（`strip="none"`、`debug=1`）：这两项不改变生成的机器码，
  但没有它们 `/usr/bin/sample` 的输出全是 `??? load address + 0x…`，
  CPU 热点档等于没测（第一次跑基线正是这么废掉的）。

被测的 `gw-proxy` 需要的 `ports::*` 端口由 `perfkit/src/stubs.rs` 提供
**内存常量实现**，不碰 Postgres / Redis。这是刻意的，代价见
`docs/relay-perf-baseline.md` §"这份基线不包含什么"。

## 4. 目录

```
scripts/perf/
├── README.md              # 本文件
├── run-baseline.sh        # 一条命令跑完七档
├── summarize.py           # results/*.json → markdown 表
├── profile-summary.py     # sample 调用图 → 按类别的 CPU 占比
├── results/               # 产出（每次跑会覆盖；含两份 ~1.5 MB 的原始 profile 调用图）
└── perfkit/               # 独立 cargo 包（自己的 workspace）
    ├── Cargo.toml
    └── src/
        ├── lib.rs
        ├── counting_alloc.rs   # 计数 #[global_allocator]
        ├── admin.rs            # 旁路 admin 端口：/stats /reset /health
        ├── stubs.rs            # gw_proxy::ports 的内存实现
        └── bin/
            ├── mock-upstream.rs  # 真 HTTP 上游，三种负载形态
            ├── gateway.rs        # 真 gw-proxy（full / nomw / idem），feature = "gateway"
            ├── relay.rs          # 真 gw_relay::RelayEngine（relay），含 failover / TLS 两档
            ├── floor.rs          # 纯 Bytes 反代 = 理论下界
            └── loadgen.rs        # 裸 TCP HTTP/1.1 压测客户端
```

## 5. 七个档

| 档 | 名字 | 量什么 |
| --- | --- | --- |
| 1 | `latency` | a/b/c 三种形态的 p50/p95/p99（concurrency=1，交错多轮） |
| 1b | `sseburst` | SSE 间隔设 0，分辨每 chunk 中继成本（1 ms 那一档分辨不出来） |
| 1c | `jsonrewrite` | 同一 body 只切 `stream` 真假，隔离 `ensure_include_usage` 的 JSON 往返 |
| 2 | `alloc` | 每请求堆分配次数与字节（重启进程，`PERF_COUNT_ALLOC=1`） |
| 3 | `throughput` | concurrency=16 的 rps（**会被本机后台负载污染**） |
| 4 | `idempotency` | `Idempotency-Key` 触发的全量响应缓冲代价 |
| 5 | `profile` | `/usr/bin/sample` 采 15 s CPU 调用图（每个被测端一份） |
| 6 | `failover` | 跨账号重试的重放代价（**默认不跑**：它要重启 relay 进程换 handler） |
| 7 | `tls` | TLS + HTTP/2 对照（**默认不跑**：它要重启整个栈换 https 上游） |

## 5.1 单独跑某一档

```bash
PHASES=latency       ./scripts/perf/run-baseline.sh
PHASES=alloc         ./scripts/perf/run-baseline.sh
PHASES=sseburst      ./scripts/perf/run-baseline.sh   # SSE 满速：分辨每 chunk 成本
PHASES=jsonrewrite   ./scripts/perf/run-baseline.sh   # 隔离 ensure_include_usage
PHASES=failover      ./scripts/perf/run-baseline.sh   # 跨账号 failover 的重放代价
PHASES=tls           ./scripts/perf/run-baseline.sh   # TLS + h2 对照
# 默认全跑（不含 failover / tls，它们要重启栈，混进来会打断"同轮交错"）：
PHASES="latency sseburst jsonrewrite alloc throughput idempotency profile" ./scripts/perf/run-baseline.sh
```

可调环境变量（都有默认值）：

| 变量 | 默认 | 含义 |
| --- | --- | --- |
| `ROUNDS` | 5 | 延迟/满速流/JSON 档的交错轮数，跨轮取中位数 |
| `SSE_ROUNDS` | 3 | SSE 长流轮数（每轮 ≈ 20 s/被测端） |
| `WORKERS` | 3 | 每个被测进程的 tokio worker 线程数（钉死以便复现） |
| `N_SMALL` / `N_LARGE` / `N_SSE` / `N_SSE_TTFB` | 10000 / 1500 / 40 / 4000 | 各场景请求数 |
| `RESULTS` | `scripts/perf/results` | 产出目录 |
| `CARGO_TARGET_DIR` | `/tmp/cargo-audit-perf` | 构建目录 |
| `PERF_TARGETS` | `floor full nomw relay` | 交错跑哪几个被测端 |
| `PERF_JSON_TARGETS` | `floor full relay` | 1c 档跑哪几个（它是 4 组 × 数千发，跑全套是浪费） |
| `PERF_NO_GATEWAY` | 0 | 1 = 不构建也不启动 `gateway`（`gw-proxy` 编不过时照样能跑 relay 对 floor） |
| `PERF_TLS_CERT` / `PERF_TLS_KEY` | `/tmp/perf-tls-*.pem` | TLS 档的自签证书；不存在就现生成 |

## 6. 手动起单个进程（调试用）

```bash
B=/tmp/cargo-audit-perf/release
$B/mock-upstream 18081 &
PERF_PORT=18080 PERF_ADMIN_PORT=18090 PERF_MODE=full \
  PERF_UPSTREAM=http://127.0.0.1:18081 PERF_WORKERS=3 $B/gateway &

curl -s -XPOST 'http://127.0.0.1:18080/v1/chat/completions?resp_bytes=2048' \
  -H 'authorization: Bearer cpa-perfbaselinekey' -H 'content-type: application/json' \
  -d '{"model":"gpt-4o","stream":false,"max_tokens":512}' | head -c 200

# 分配计数（需要进程带 PERF_COUNT_ALLOC=1 起）
curl -sXPOST http://127.0.0.1:18090/reset && curl -s http://127.0.0.1:18090/stats
```

`loadgen` 参数：

```
--host --port --path --body-bytes --concurrency
--duration <秒> | --requests <总数>      # 二选一
--mode unary|sse --warmup-ms --timeout-ms
--idempotency 0|1                        # 每请求带唯一 Idempotency-Key
--label --out <json 路径>
```

## 7. 读数字之前必须知道的

* **这台机器不是干净的压测机**。测量期间本机 load average 在 10~12（12 核），
  同时跑着编译、浏览器和别的 agent。因此：
  * **绝对值（rps、p99）偏悲观，不要当成产能上限**；
  * **差值（网关 − 下界）是可信的**：run-baseline.sh 在同一轮里 A/B/A/B 交错跑
    三个被测端，后台负载的漂移对三者是同分布的；跨轮取中位数而不是平均数。
  * 每张表都同时给出 p50 的跨轮 min–max，离散度自己看。
* **延迟档一律 `concurrency=1`**。要量的是每请求固定开销，并发只会把排队时间
  混进来。吞吐单独一档量，并且明确标注它被后台负载污染。
* `loadgen` 的 `stalls` 字段：单请求超时后重连继续的次数。**非零就要在结论里
  说明**，不要当作没发生。基线那一轮：161 次运行 / 3 619 149 个请求里出现 **3 次**，
  全部落在 `floor`（对照组）的 SSE-TTFB 场景，网关侧 0 次 —— 那是我这 60 行对照组
  反代自身的缺陷，不是 `gw-proxy` 的。复现时它还会出现。
* **幂等档的 key 必须全进程唯一**。预热段和正式段共用一个 loadgen 进程，如果两段
  各自从 0 开始编号，正式段每一发都会命中预热留下的缓存条目、直接重放、根本不打
  上游 —— 量出来会是"开幂等比不开还快 138 µs"。`loadgen.rs` 的 `KEY_SEQ` 就是为此
  存在的，别把它改回 per-run 计数器。
