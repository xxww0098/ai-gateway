# 热路径火焰图

`docs/hotpath-flamegraph.svg` 是 **真 `gw_proxy::router()`**（`PERF_MODE=full`，
一层 `kernel::layer`）在本地 mock 上游上的 CPU 采样，不是生产流量，也不是
进程内 `FakeProvider`。

采样时间：2026-08-17 11:46 CST（UTC+8）。

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

## 这一轮读数

loadgen 20s / concurrency=8 / 一元 1 KiB→2 KiB：

| 项 | 值 |
| --- | --- |
| 请求数 | 752 034 |
| rps | 37 575 |
| 延迟 p50 / p99 | 211 µs / 329 µs |
| errors / non_200 / stalls | 0 / 0 / 0 |
| perf 样本 | 4 297（`cycles:u`，dwarf） |

栈上出现的内核帧（任意位置，按采样权重）：

| 占比 | 帧 |
| --- | --- |
| 64.5% | `gw_proxy::kernel::layer` |
| 61.6% | `HoldMiddleware::handle` |
| 56.1% | `HoldMiddleware::handle_reserved` |
| 53.8% | `routes::dispatch` |
| 40.5% | `OpenAiCompatibleProvider::execute`（打 mock 上游） |
| 3.4% | `routes::unary_response` |
| 2.5% | `Settlement::settle` |
| 2.3% | `hold::peek_request_body` |

叶子热点仍是分配与 `Bytes` 引用计数（`malloc`/`cfree`/`bytes_*_shared`），
以及 serde_json 扫字符串。这和「热路径已经是一层状态机 + 零拷贝 body」
对得上：剩下的 CPU 主要在 HTTP 编解码和堆，不在中间件叠罗汉。

## 复现

```bash
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=/tmp/cargo-hotpath-perfkit
DURATION=20 ./scripts/perf/hotpath-flamegraph.sh
```

中间产物（**不进 git**，体积大）：`scripts/perf/results/hotpath.perf.data`、
`scripts/perf/results/hotpath.folded`。摘要在
`scripts/perf/results/hotpath.load.json`。
