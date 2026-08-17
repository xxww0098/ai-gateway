#!/usr/bin/env bash
# 重打 AI-GateWay 最近一次热路径 load JSON。不采样、不起进程、不编造对照数字。
#
#   ./scripts/perf/compare-notes.sh
#   RESULTS=/path/to/results ./scripts/perf/compare-notes.sh
#
# 读：
#   $RESULTS/hotpath.load.json
#   $RESULTS/hotpath-stream.load.json
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RESULTS="${RESULTS:-$ROOT/scripts/perf/results}"

python3 - "$RESULTS" <<'PY'
from pathlib import Path
import json
import sys

results = Path(sys.argv[1])
files = [
    ("unary", results / "hotpath.load.json"),
    ("stream", results / "hotpath-stream.load.json"),
]

def us(v):
    if v is None:
        return "—"
    if v >= 1000:
        return f"{v/1000:.2f} ms"
    return f"{v:.2f} µs"

missing = []
print(f"results dir: {results}")
print()
for kind, path in files:
    if not path.is_file():
        missing.append(str(path))
        print(f"## {kind}")
        print(f"missing: {path}")
        print()
        continue
    data = json.loads(path.read_text())
    lat = data.get("latency") or {}
    ttfb = data.get("ttfb") or {}
    gap = data.get("chunk_gap") or {}
    chunks = data.get("chunks_per_response") or {}
    print(f"## {kind}  ({path.name})")
    print(f"  label:        {data.get('label', '—')}")
    print(f"  target:       {data.get('target', '—')}")
    print(f"  concurrency:  {data.get('concurrency', '—')}")
    print(f"  requests:     {data.get('requests', '—')}")
    print(f"  rps:          {data.get('rps', 0):.2f}")
    print(f"  wall:         {data.get('wall_seconds', 0):.3f} s")
    print(f"  errors/non_200/stalls: {data.get('errors', 0)} / {data.get('non_200', 0)} / {data.get('stalls', 0)}")
    print(f"  latency p50/p99: {us(lat.get('p50_us'))} / {us(lat.get('p99_us'))}")
    print(f"  ttfb    p50/p99: {us(ttfb.get('p50_us'))} / {us(ttfb.get('p99_us'))}")
    if kind == "stream":
        print(f"  chunks/response: {chunks.get('min', '—')}–{chunks.get('max', '—')}")
        print(f"  chunk gap p50/p99: {us(gap.get('p50_us'))} / {us(gap.get('p99_us'))}")
    print()

if missing:
    print("no invented numbers: files above are absent.", file=sys.stderr)
    sys.exit(1)
PY
