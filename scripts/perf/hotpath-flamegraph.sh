#!/usr/bin/env bash
# 热路径火焰图：真 gw_proxy::router() + 本地 mock 上游，不打真实供应商。
#
#   ./scripts/perf/hotpath-flamegraph.sh
#
# 产出：
#   docs/hotpath-flamegraph.svg
#   scripts/perf/results/hotpath.folded
#   scripts/perf/results/hotpath.perf.data
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
OUT_SVG="${OUT_SVG:-$ROOT/docs/hotpath-flamegraph.svg}"
DURATION="${DURATION:-20}"
WORKERS="${WORKERS:-3}"
P_MOCK="${P_MOCK:-18081}"
P_FULL="${P_FULL:-18080}"
A_FULL="${A_FULL:-18090}"
Q_SMALL='/v1/chat/completions?resp_bytes=2048'
B_SMALL=1024

export PATH="${HOME}/.cargo/bin:/home/box/.cargo/bin:${PATH}"
export CARGO_HOME="${CARGO_HOME:-/home/box/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-/home/box/.rustup}"

mkdir -p "$RESULTS" "$(dirname "$OUT_SVG")"

need() { command -v "$1" >/dev/null || { echo "missing $1" >&2; exit 1; }; }
need perf
need inferno-collapse-perf
need inferno-flamegraph
need rustfilt

if [[ ! -x "$BIN/gateway" || ! -x "$BIN/mock-upstream" || ! -x "$BIN/loadgen" ]]; then
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

echo "warmup"
"$BIN/loadgen" --host 127.0.0.1 --port "$P_FULL" --path "$Q_SMALL" \
  --body-bytes "$B_SMALL" --concurrency 4 --requests 400 --mode unary \
  --label warm --warmup-ms 0 --timeout-ms 5000 >/tmp/hotpath-warm.log 2>&1 || true

DATA="$RESULTS/hotpath.perf.data"
FOLDED="$RESULTS/hotpath.folded"
echo "perf record ${DURATION}s on gateway pid=$GW_PID (cycles:u, dwarf)"
# 用户态即可：perf_event_paranoid=2 不需要 sudo。dwarf 比 fp 更能拆开内联。
perf record -o "$DATA" -F 99 -g --call-graph dwarf,16384 -e cycles:u -p "$GW_PID" -- \
  "$BIN/loadgen" --host 127.0.0.1 --port "$P_FULL" --path "$Q_SMALL" \
    --body-bytes "$B_SMALL" --concurrency 8 --duration "$DURATION" --mode unary \
    --label hotpath --warmup-ms 0 --timeout-ms 5000 \
    --out "$RESULTS/hotpath.load.json"

echo "collapsing stacks"
# rustfilt 把 v0 符号解成 gw_proxy::kernel::layer 这种可读名。
perf script -i "$DATA" 2>/dev/null | inferno-collapse-perf | rustfilt > "$FOLDED"

inferno-flamegraph --title "ai-gateway hot path (full kernel + mock upstream, unary 1KiB/2KiB)" \
  --subtitle "perf record -F 99 -e cycles:u --call-graph dwarf; rustfilt; see docs/hotpath-flamegraph.md" \
  --width 1600 --colors rust --deterministic --minwidth 0.05 \
  --notes "Repro: ./scripts/perf/hotpath-flamegraph.sh" \
  < "$FOLDED" > "$OUT_SVG"

echo "wrote $OUT_SVG ($(wc -c < "$OUT_SVG") bytes)"
echo "folded stacks: $FOLDED"
python3 - "$FOLDED" <<'PY'
from collections import Counter
from pathlib import Path
import sys
c = Counter()
total = 0
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
print(f"samples: {total}")
print("top leaves:")
for name, n in c.most_common(15):
    pct = (100.0 * n / total) if total else 0
    print(f"  {pct:5.1f}%  {name}")
PY
