# 热路径火焰图

`docs/hotpath-flamegraph.svg` 是 **真 `gw_proxy::router()`**（`PERF_MODE=full`，
一层 `kernel::layer`）在本地 mock 上游上的 CPU 采样，不是生产流量，也不是
进程内 `FakeProvider`。

采样时间：2026-08-17 13:24 CST（UTC+8）。上一轮（少拷 body / peek）是 12:31 CST。

## 怎么出的

环境：Linux 6.12、`perf_event_paranoid=2`、用户态 `cycles:u`，不需要 sudo。
符号来自 perfkit 的 release 配置（`opt-level=3` / `lto=thin` / `codegen-units=1`，
`strip=none` + `debug=1`，机器码与根 workspace 一致，只多一张符号表）。

```bash
# 依赖：linux-perf、cargo install inferno rustfilt、rustc 1.97.1
./scripts/perf/hotpath-flamegraph.sh
```

脚本做的事：

1. `cargo build --release --bins`（`scripts/perf/perfkit`，独立 workspace）
2. 起 `mock-upstream :18081` + `gateway PERF_MODE=full :18080`
3. loadgen 预热 400 发
4. `perf record -F 99 -g --call-graph dwarf,16384 -e cycles:u -p $GW_PID`，
   同时 loadgen 以 concurrency=8 打 20s 一元请求（1 KiB 请求 / 2 KiB 响应）
5. `perf script | inferno-collapse-perf | rustfilt | inferno-flamegraph` → SVG

没有真实供应商时，这就是「本地 harness + mock 上游」路径。
不要拿绝对值当产能上限：这台机器同时在跑别的编译。

perfkit 给 gateway 套了 `counting_alloc` 全局分配器（为分配计数档服务）。
图上的 `perfkit::counting_alloc` / 偏高的 `malloc`/`cfree` **有一部分是装置税**，
生产二进制没有这层包装。

本轮 harness 仍然是 in-memory `NullLedger`，**没有 Redis RTT**。生产上
`hold_gated` 把 GET-balance + HOLD + EXPIRE 收成一条 Lua，那一截省下的
往返不会出现在这张图上。图上能看到的是：常见路径不再 `tokio::join!`
两趟账本 peek，`available_balance` 从栈上消失，改成一次 `hold_gated`。

## 这一轮读数

loadgen 20s / concurrency=8 / 一元 1 KiB→2 KiB。括号里是 12:31 那一轮
（少拷 body / peek 不再整棵 serde）的对照。

| 项 | 值 | 上一轮 |
| --- | --- | --- |
| 请求数 | 792 701 | 766 080 |
| rps | 39 587 | 38 282 |
| 延迟 p50 / p99 | 200 µs / 309 µs | 207 µs / 323 µs |
| errors / non_200 / stalls | 0 / 0 / 0 | 0 / 0 / 0 |
| perf 样本 | 4 263（`cycles:u`，dwarf） | 4 205 |

栈上出现的内核帧（任意位置，按采样权重）：

| 占比 | 上一轮 | 帧 |
| --- | --- | --- |
| 65.5% | 65.5% | `gw_proxy::kernel::layer` |
| 60.7% | 60.7% | `HoldMiddleware::handle` |
| 55.3% | 55.5% | `HoldMiddleware::handle_reserved` |
| 54.3% | 55.1% | `routes::dispatch` |
| 41.8% | 41.8% | `OpenAiCompatibleProvider::execute`（打 mock 上游） |
| 4.1% | 3.9% | `routes::unary_response` |
| 2.9% | 2.9% | `hold::peek_request_body` |
| 2.9% | 2.7% | `Settlement::settle` |
| 1.5% | 1.7% | `json_peek::parse_top_fields` |
| 1.4% | 1.3% | `routes::inbound` |
| 0.4% | — | `hold_gated`（预扣门 + 预扣合成一次调用） |
| 0.0% | — | `available_balance`（常见路径不再单独 peek） |

`HoldMiddleware` 仍然包着 dispatch → execute，所以占比几乎不动是预期：
省掉的是账本 peek 的 future / vtable，不是上游 HTTP。叶子热点仍是
`malloc` / `cfree` / `Bytes` 引用计数（含 harness `counting_alloc` 税）。

## 复现

```bash
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=/tmp/cargo-hotpath-perfkit
DURATION=20 ./scripts/perf/hotpath-flamegraph.sh
```

中间产物（**不进 git**，体积大）：`scripts/perf/results/hotpath.perf.data`、
`scripts/perf/results/hotpath.folded`。摘要在
`scripts/perf/results/hotpath.load.json`。
