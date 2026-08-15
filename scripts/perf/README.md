# scripts/perf —— gw-relay 性能基线装置

一条命令跑完全部测量：

```bash
./scripts/perf/run-baseline.sh          # 全量，约 9~12 分钟
python3 scripts/perf/summarize.py       # 把 results/*.json 汇总成 markdown 表
```

结论与解读在 [`docs/relay-perf-baseline.md`](../../docs/relay-perf-baseline.md)。
本 README 只讲**怎么跑**和**装置是怎么搭的**。

---

## 1. 它测什么

三个被测端同时在跑，同一份 mock 上游、同一套负载：

| 被测端 | 端口 | 是什么 |
| --- | --- | --- |
| `floor` | 18082 | **理论下界**：约 60 行的 axum 纯 `Bytes` 反代，请求/响应双向流式，零解析零计费 |
| `nomw`  | 18084 | 真 `gw_proxy::routes::chat_completions`，**不挂** access / hold 两层中间件 |
| `full`  | 18080 | 真 `gw_proxy::router()`，生产拓扑（access → hold → dispatch） |
| `idem`  | 18086 | 同 `full`，额外挂上幂等管理器（量 `hold::capture_body` 的全量响应缓冲） |
| mock 上游 | 18081 | 见下 |

**网关自身开销 = `full` 实测 − `floor` 实测。**
**access+hold 净成本 = `full` − `nomw`。**

三个负载形态：

| 代号 | 形态 | 目的 |
| --- | --- | --- |
| a) `small` | 1 KiB 请求 / 2 KiB 响应，非流式 | 网关固定开销 |
| b) `large` | 256 KiB 请求 / 1 MiB 响应，非流式 | 拷贝开销随 body 增长的斜率 |
| c) `sse` | 500 chunk × 1 KiB × 1 ms | 流式中继开销、TTFB、chunk 间抖动 |
| c-0) `ssettfb` | 1 chunk，无间隔 | 只测"建流"这一步的固定成本，样本量大，p99 才可信 |

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
`crates/gw-authcore`。

* 根 `Cargo.toml` 一个字没动 —— `members = ["crates/*", "tools/xtask"]` 匹配不到
  `scripts/`，`cargo build --workspace` 也不会编到它；
* `crates/**` 下一个 `.rs` 没动；
* 它的 `[profile.release]` 与根 workspace **逐字一致**（`opt-level=3`、
  `lto="thin"`、`codegen-units=1`），否则"网关 vs 下界"的差值里会混进编译参数差异。

被测的 `gw-proxy` 需要的 `ports::*` 端口由 `perfkit/src/stubs.rs` 提供
**内存常量实现**，不碰 Postgres / Redis。这是刻意的，代价见
`docs/relay-perf-baseline.md` §"这份基线不包含什么"。

## 4. 目录

```
scripts/perf/
├── README.md              # 本文件
├── run-baseline.sh        # 一条命令跑完五档
├── summarize.py           # results/*.json → markdown 表
├── results/               # 产出（每次跑会覆盖）
└── perfkit/               # 独立 cargo 包（自己的 workspace）
    ├── Cargo.toml
    └── src/
        ├── lib.rs
        ├── counting_alloc.rs   # 计数 #[global_allocator]
        ├── admin.rs            # 旁路 admin 端口：/stats /reset /health
        ├── stubs.rs            # gw_proxy::ports 的内存实现
        └── bin/
            ├── mock-upstream.rs  # 真 HTTP 上游，三种负载形态
            ├── gateway.rs        # 真 gw-proxy（full / nomw / idem）
            ├── floor.rs          # 纯 Bytes 反代 = 理论下界
            └── loadgen.rs        # 裸 TCP HTTP/1.1 压测客户端
```

## 5. 单独跑某一档

```bash
PHASES=latency      ./scripts/perf/run-baseline.sh
PHASES="alloc"      ./scripts/perf/run-baseline.sh
PHASES="latency alloc throughput idempotency profile" ./scripts/perf/run-baseline.sh   # 默认
```

可调环境变量（都有默认值）：

| 变量 | 默认 | 含义 |
| --- | --- | --- |
| `ROUNDS` | 7 | 延迟档交错轮数，跨轮取中位数 |
| `SSE_ROUNDS` | 3 | SSE 长流轮数（每轮 ≈ 20 s/被测端） |
| `WORKERS` | 3 | 每个被测进程的 tokio worker 线程数（钉死以便复现） |
| `N_SMALL` / `N_LARGE` / `N_SSE` / `N_SSE_TTFB` | 10000 / 1500 / 40 / 4000 | 各场景请求数 |
| `RESULTS` | `scripts/perf/results` | 产出目录 |
| `CARGO_TARGET_DIR` | `/tmp/cargo-audit-perf` | 构建目录 |

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
  说明**，不要当作没发生。
