# 热路径火焰图

`docs/hotpath-flamegraph.svg` 是 **真 `gw_proxy::router()`**（`PERF_MODE=full`，
一层 `kernel::layer`）在本地 mock 上游上的 **一元** CPU 采样。
`docs/hotpath-flamegraph-stream.svg` 是同一套装置上的 **SSE 满速** 采样。
都不是生产流量，也不是进程内 `FakeProvider`。

采样时间：2026-08-17 14:14 CST（UTC+8）。上一轮（hold 收成一条 Lua）是 13:24 CST。
AI-GateWay 对 NewAPI / CLIProxyAPI 的对照（架构 + 本仓库数字，没有它们的 rps）见
[`perf-vs-newapi-cliproxy.md`](perf-vs-newapi-cliproxy.md)。

## 怎么出的

环境：Linux 6.12、`perf_event_paranoid=2`、用户态 `cycles:u`，不需要 sudo。
符号来自 perfkit 的 release 配置（`opt-level=3` / `lto=thin` / `codegen-units=1`，
`strip=none` + `debug=1`，机器码与根 workspace 一致，只多一张符号表）。

```bash
# 依赖：linux-perf、cargo install inferno rustfilt、rustc 1.97.1
./scripts/perf/hotpath-flamegraph.sh              # MODE=both：一元 + 流式
MODE=unary  ./scripts/perf/hotpath-flamegraph.sh
MODE=stream ./scripts/perf/hotpath-flamegraph.sh
```

脚本做的事：

1. `cargo build --release --bins`（`scripts/perf/perfkit`，独立 workspace）
2. 起 `mock-upstream :18081` + `gateway PERF_MODE=full :18080`
3. loadgen 预热（一元 400 发 / 流式 40 发）
4. `perf record -g --call-graph dwarf,16384 -e cycles:u -p $GW_PID`：
   - 一元：`-F 99`，concurrency=8，20s，1 KiB 请求 / 2 KiB 响应
   - 流式：`-F 999`，concurrency=16，20s，SSE 32×256 B、`interval_us=0`
5. `perf script | inferno-collapse-perf | rustfilt | inferno-flamegraph` → SVG

流式档把采样频率和并发拉高，是因为每条请求要中继 33 个 chunk，
`-F 99` / conc=8 时 20s 只有几十个样本，拆不开 `RelayBody` / `UsageRelay`。

没有真实供应商时，这就是「本地 harness + mock 上游」路径。
不要拿绝对值当产能上限：这台机器同时在跑别的编译。

perfkit 给 gateway 套了 `counting_alloc` 全局分配器（为分配计数档服务）。
图上的 `perfkit::counting_alloc` / 偏高的 `malloc`/`cfree` **有一部分是装置税**，
生产二进制没有这层包装。

本轮 harness 仍然是 in-memory `NullLedger`，**没有 Redis RTT**。

## 一元读数

loadgen 20s / concurrency=8 / 一元 1 KiB→2 KiB。括号里是 13:24 那一轮
（hold 收成一条 Lua）的对照。

| 项 | 值 | 上一轮 |
| --- | --- | --- |
| 请求数 | 775 039 | 792 701 |
| rps | 38 727 | 39 587 |
| 延迟 p50 / p99 | 205 µs / 313 µs | 200 µs / 309 µs |
| errors / non_200 / stalls | 0 / 0 / 0 | 0 / 0 / 0 |
| perf 样本 | 4 322（`cycles:u`，dwarf） | 4 263 |

绝对值略低于上一轮，是同机编译噪声，不是一元路径回退：本轮没动 hold /
peek / `hold_gated`。栈上内核帧：

| 占比 | 上一轮 | 帧 |
| --- | --- | --- |
| 65.8% | 65.5% | `gw_proxy::kernel::layer` |
| 61.0% | 60.7% | `HoldMiddleware` |
| 40.1% | 41.8% | `OpenAiCompatibleProvider`（打 mock 上游） |
| 3.7% | 4.1% | `routes::unary_response` |

叶子仍是 `malloc` / `cfree` / `Bytes` 引用计数（含 harness 税）。
一元 `unary_response` 不再 `clone` usage，只是把 `UsageRecord` move 进
`UsageOutcome`，在这张图上几乎看不见。

## 流式读数

loadgen 20s / concurrency=16 / SSE 32×256 B burst（`interval_us=0`）+ 终局 usage。
每条响应 33–34 个 chunk（32 个 data 帧 + usage/`[DONE]`）。

| 项 | 值 |
| --- | --- |
| 请求数 | 5 912 |
| rps | 294 |
| 约合 chunk/s | ~195 000（33 chunk × 294 rps） |
| 整请求 p50 / p99 | 44.5 ms / 95.6 ms |
| TTFB p50 / p99 | 389 µs / 1.13 ms |
| chunk 间隔 p50 / p99 | 0.10 µs / 44.1 ms |
| errors / non_200 / stalls | 0 / 0 / 0 |
| perf 样本 | 1 503（`cycles:u`，dwarf，`-F 999`） |

整请求延迟被「32 个 chunk 的 mock unfold + 网关 poll」拉长，**不要拿它跟一元
200 µs 比产能**。TTFB（389 µs）才是建流成本；chunk 间隔 p50 = 0.10 µs 说明
满速路径上就绪的帧不再被 300 s idle `Sleep` 拖住。

栈上出现的流式帧（任意位置，按采样权重）：

| 占比 | 帧 | 说明 |
| --- | --- | --- |
| 31.3% | `gw_proxy::kernel::layer` | 每条流仍走鉴权 + 预扣 |
| 28.7% | `HoldMiddleware` | 同上 |
| 19.0% | `OpenAiCompatibleProvider` | 打 mock 上游 |
| 18.6% | `RelayBody` | 代理侧 `Stream`：转发 payload、latch usage |
| 13.7% | `UsageRelay` | provider 侧 `Stream`：probe + 映射 `StreamChunk` |
| 8.4% | `IdleTimeout` | 只在 inner `Pending` 时才武装可复用 `Sleep` |
| 5.1% | `StreamUsageProbe` / `observe` | 逐行 usage，驻留 O(单行) |
| 2.9% | `tokio::time` | 不再是每 chunk 一个 `timeout()` |
| 0.7% | `StreamSettler` | 结束时 inline settle；断线走 `TaskTracker` |

对比改之前的结构（没有单独的流式火焰图，只能看代码）：每 chunk 要过
**三层 `unfold`**（idle `timeout` + usage 累加 + 代理过滤），并且
`tokio::time::timeout(300s, next)` 在帧已就绪时也建一个 `Sleep`。
现在 idle 只在 `Pending` 时武装一次、跨 gap 复用；usage 与代理过滤是
`poll_next`，不再每 chunk 搬 `StreamUsageProbe` 的 `pending`。

叶子上原来的 `windows().any()`（`carries_usage`）从第一轮流式图的 2.7%
掉下去了：先找 `u` 再比对 `"usage"` 四个字节，语义不变。

## 复现

```bash
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=/tmp/cargo-hotpath-perfkit
DURATION=20 ./scripts/perf/hotpath-flamegraph.sh
```

中间产物（**不进 git**，体积大）：`scripts/perf/results/hotpath.perf.data`、
`hotpath-stream.perf.data`、对应的 `.folded`。摘要在
`scripts/perf/results/hotpath.load.json` 与 `hotpath-stream.load.json`。
