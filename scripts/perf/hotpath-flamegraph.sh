#!/usr/bin/env bash
# 热路径火焰图：真 gw_proxy::router() + 本地 mock 上游，不打真实供应商。
#
#   ./scripts/perf/hotpath-flamegraph.sh              # MODE=both（默认）
#   MODE=unary  ./scripts/perf/hotpath-flamegraph.sh
#   MODE=stream ./scripts/perf/hotpath-flamegraph.sh
#
# 产出：
#   docs/hotpath-flamegraph.svg            # unary
#   docs/hotpath-flamegraph-stream.svg     # stream
#   scripts/perf/results/hotpath.folded / hotpath-stream.folded
#   scripts/perf/results/hotpath.perf.data / hotpath-stream.perf.data
#
# 依赖：linux-perf、inferno、rustfilt、rustc 1.97.1（见 rust-toolchain.toml）。
# 本机没有真实上游时，
# 压的是 perfkit 的 mock-upstream（真 HTTP，不是进程内 FakeProvider）。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PERFKIT="$ROOT/scripts/perf/perfkit"
RESULTS="${RESULTS:-$ROOT/scripts/perf/results}"
TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/cargo-hotpath-perfkit}"
BIN="$TARGET_DIR/release"
DURATION="${DURATION:-20}"
WORKERS="${WORKERS:-3}"
P_MOCK="${P_MOCK:-18081}"
P_FULL="${P_FULL:-18080}"
A_FULL="${A_FULL:-18090}"
MODE="${MODE:-both}"
FORCE_REBUILD="${FORCE_REBUILD:-0}"

Q_UNARY='/v1/chat/completions?resp_bytes=2048'
# 满速 SSE：32 个 256 B 的 data 帧 + 终局 usage，interval=0。
# 要量的是每 chunk 中继，不是 mock 定时器；chunk 够多才能在火焰图上压过 hold。
Q_STREAM='/v1/chat/completions?stream=1&chunks=32&chunk_bytes=256&interval_us=0'
B_SMALL=1024

export PATH="${HOME}/.cargo/bin:/home/box/.cargo/bin:${PATH}"
export CARGO_HOME="${CARGO_HOME:-/home/box/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-/home/box/.rustup}"

mkdir -p "$RESULTS" "$ROOT/docs"

need() { command -v "$1" >/dev/null || { echo "missing $1" >&2; exit 1; }; }
need perf
need inferno-collapse-perf
need inferno-flamegraph
need rustfilt

if [[ "$FORCE_REBUILD" == "1" || ! -x "$BIN/gateway" || ! -x "$BIN/mock-upstream" || ! -x "$BIN/loadgen" ]]; then
  echo "building perfkit release (symbols kept, codegen matches workspace)…"
  (cd "$PERFKIT" && CARGO_TARGET_DIR="$TARGET_DIR" cargo build --release --bins)
fi

cleanup() {
  local pid
  for pid in "${PIDS[@]:-}"; do
    kill "$pid" 2>/dev/null || true
  done
  wait 2>/dev/null || true
}
PIDS=()
trap cleanup EXIT

echo "starting mock-upstream on :$P_MOCK"
"$BIN/mock-upstream" "$P_MOCK" >/tmp/hotpath-mock.log 2>&1 &
PIDS+=($!)

echo "starting gateway (PERF_MODE=full) on :$P_FULL"
PERF_PORT="$P_FULL" PERF_ADMIN_PORT="$A_FULL" PERF_MODE=full \
  PERF_UPSTREAM="http://127.0.0.1:$P_MOCK" PERF_WORKERS="$WORKERS" \
  "$BIN/gateway" >/tmp/hotpath-gateway.log 2>&1 &
PIDS+=($!)
GW_PID=${PIDS[-1]}

for i in $(seq 1 50); do
  if curl -sf "http://127.0.0.1:$A_FULL/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.2
done

summarize() {
  python3 - "$1" <<'PY'
from collections import Counter
from pathlib import Path
import sys
c = Counter()
frames = Counter()
total = 0
needles = (
    "usage_stream",
    "UsageRelay",
    "IdleTimeout",
    "RelayBody",
    "stream_response",
    "StreamSettler",
    "with_stream_idle_timeout",
    "StreamUsageProbe",
    "observe",
    "unary_response",
    "gw_proxy::kernel::layer",
    "HoldMiddleware",
    "OpenAiCompatibleProvider",
    "tokio::time",
)
for line in Path(sys.argv[1]).read_text(errors="replace").splitlines():
    if not line.strip():
        continue
    stack, _, n = line.rpartition(" ")
    try:
        samples = int(n)
    except ValueError:
        continue
    total += samples
    leaf = stack.rsplit(";", 1)[-1]
    c[leaf] += samples
    for needle in needles:
        if needle in stack:
            frames[needle] += samples
print(f"samples: {total}")
print("top leaves:")
for name, n in c.most_common(15):
    pct = (100.0 * n / total) if total else 0
    print(f"  {pct:5.1f}%  {name}")
print("named frames (any stack position):")
for name, n in frames.most_common():
    pct = (100.0 * n / total) if total else 0
    print(f"  {pct:5.1f}%  {name}")
PY
}

run_one() {
  local kind="$1" path="$2" lmode="$3" svg="$4" stem="$5" title="$6" warm_n="$7" timeout_ms="$8"
  local conc="${9:-8}"
  local freq="${10:-99}"

  echo "warmup ($kind)"
  "$BIN/loadgen" --host 127.0.0.1 --port "$P_FULL" --path "$path" \
    --body-bytes "$B_SMALL" --concurrency 4 --requests "$warm_n" --mode "$lmode" \
    --label "warm-$kind" --warmup-ms 0 --timeout-ms "$timeout_ms" \
    >/tmp/hotpath-warm-$kind.log 2>&1 || true

  local data="$RESULTS/${stem}.perf.data"
  local folded="$RESULTS/${stem}.folded"
  echo "perf record ${DURATION}s on gateway pid=$GW_PID ($kind, cycles:u, dwarf, -F $freq, conc=$conc)"
  # 用户态即可：perf_event_paranoid=2 不需要 sudo。dwarf 比 fp 更能拆开内联。
  # 流式路径 CPU 更稀：提高采样频率、拉高并发，否则 20s 只有几十个样本。
  perf record -o "$data" -F "$freq" -g --call-graph dwarf,16384 -e cycles:u -p "$GW_PID" -- \
    "$BIN/loadgen" --host 127.0.0.1 --port "$P_FULL" --path "$path" \
      --body-bytes "$B_SMALL" --concurrency "$conc" --duration "$DURATION" --mode "$lmode" \
      --label "hotpath-$kind" --warmup-ms 0 --timeout-ms "$timeout_ms" \
      --out "$RESULTS/${stem}.load.json"

  echo "collapsing stacks ($kind)"
  # rustfilt 把 v0 符号解成 gw_proxy::kernel::layer 这种可读名。
  perf script -i "$data" 2>/dev/null | inferno-collapse-perf | rustfilt > "$folded"

  inferno-flamegraph --title "$title" \
    --subtitle "perf record -F $freq -e cycles:u --call-graph dwarf; rustfilt; see docs/hotpath-flamegraph.md" \
    --width 1600 --colors rust --deterministic --minwidth 0.05 \
    --notes "Repro: MODE=$kind ./scripts/perf/hotpath-flamegraph.sh" \
    < "$folded" > "$svg"

  echo "wrote $svg ($(wc -c < "$svg") bytes)"
  echo "folded stacks: $folded"
  summarize "$folded"
}

case "$MODE" in
  unary)
    run_one unary "$Q_UNARY" unary \
      "${OUT_SVG:-$ROOT/docs/hotpath-flamegraph.svg}" \
      hotpath \
      "ai-gateway hot path (full kernel + mock upstream, unary 1KiB/2KiB)" \
      400 5000
    ;;
  stream)
    run_one stream "$Q_STREAM" sse \
      "${OUT_SVG_STREAM:-$ROOT/docs/hotpath-flamegraph-stream.svg}" \
      hotpath-stream \
      "ai-gateway hot path (full kernel + mock upstream, SSE 32x256B burst)" \
      40 10000 16 999
    ;;
  both)
    run_one unary "$Q_UNARY" unary \
      "${OUT_SVG:-$ROOT/docs/hotpath-flamegraph.svg}" \
      hotpath \
      "ai-gateway hot path (full kernel + mock upstream, unary 1KiB/2KiB)" \
      400 5000
    run_one stream "$Q_STREAM" sse \
      "${OUT_SVG_STREAM:-$ROOT/docs/hotpath-flamegraph-stream.svg}" \
      hotpath-stream \
      "ai-gateway hot path (full kernel + mock upstream, SSE 32x256B burst)" \
      40 10000 16 999
    ;;
  *)
    echo "MODE must be unary, stream, or both (got $MODE)" >&2
    exit 1
    ;;
esac
