#!/usr/bin/env bash
# gw-relay 性能基线 —— 一条命令跑完全部测量。
#
#   ./scripts/perf/run-baseline.sh            # 全量（约 8~12 分钟）
#   ROUNDS=1 ./scripts/perf/run-baseline.sh   # 快跑一遍确认装置能动
#   PHASES=latency ./scripts/perf/run-baseline.sh
#
# 产出：scripts/perf/results/*.json，再用 scripts/perf/summarize.py 汇总成表。
#
# 设计要点（改之前先读）：
#   * 三个被测端同时在跑（floor / full / nomw / idem），空闲时不耗 CPU。
#     这样同一轮里可以 A/B/A/B 交错，把机器本身的负载漂移摊到每一个被测端上 ——
#     绝对值会被后台负载污染，**差值不会**。
#   * 延迟档一律 concurrency=1。要量的是「每请求固定开销」，并发只会把排队
#     时间混进来。吞吐单独一档量。
#   * 分配计数档单独重启进程（PERF_COUNT_ALLOC=1），不与延迟档混跑。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PERFKIT="$ROOT/scripts/perf/perfkit"
RESULTS="${RESULTS:-$ROOT/scripts/perf/results}"
TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/cargo-audit-perf}"
BIN="$TARGET_DIR/release"

ROUNDS="${ROUNDS:-5}"
SSE_ROUNDS="${SSE_ROUNDS:-3}"
WORKERS="${WORKERS:-3}"
PHASES="${PHASES:-latency sseburst jsonrewrite alloc throughput idempotency profile}"

# 每档请求数（concurrency=1）
N_SMALL="${N_SMALL:-10000}"
N_LARGE="${N_LARGE:-1500}"
N_SSE="${N_SSE:-40}"
N_SSE_TTFB="${N_SSE_TTFB:-4000}"

# 端口
P_MOCK=18081
P_FLOOR=18082;  A_FLOOR=18092
P_FULL=18080;   A_FULL=18090
P_NOMW=18084;   A_NOMW=18094
P_IDEM=18086;   A_IDEM=18096

# 场景 path（query 会被网关原样透传给 mock，用来选形态）
Q_SMALL='/v1/chat/completions?resp_bytes=2048'
Q_LARGE='/v1/chat/completions?resp_bytes=1048576'
Q_SSE='/v1/chat/completions?stream=1&chunks=500&chunk_bytes=1024&interval_us=1000'
Q_SSE_TTFB='/v1/chat/completions?stream=1&chunks=1&chunk_bytes=1024&interval_us=0'

B_SMALL=1024
B_LARGE=262144

mkdir -p "$RESULTS"

log() { printf '\033[36m[perf]\033[0m %s\n' "$*" >&2; }

# ---------------------------------------------------------------- 构建

build() {
  log "构建 release（CARGO_TARGET_DIR=${TARGET_DIR}）"
  (cd "$PERFKIT" && CARGO_TARGET_DIR="$TARGET_DIR" cargo build --release >&2)
}

# ---------------------------------------------------------------- 进程

# bash 3.2（macOS 自带）在 `set -u` 下展开空数组会报 unbound，所以用
# 空格分隔的字符串而不是数组。
PIDS=""
cleanup() {
  local pid
  for pid in $PIDS; do
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT INT TERM

wait_ready() { # host_port
  local url="$1" i=0
  while [ $i -lt 100 ]; do
    if curl -sf "$url" >/dev/null 2>&1; then return 0; fi
    sleep 0.1; i=$((i+1))
  done
  echo "服务未就绪: $url" >&2; return 1
}

start_stack() { # count_alloc(0|1)
  local count="$1"
  log "启动 mock + floor + gateway(full/nomw/idem)，PERF_COUNT_ALLOC=$count"
  PERF_COUNT_ALLOC="$count" "$BIN/mock-upstream" "$P_MOCK" >"/tmp/perf-mock.log" 2>&1 & PIDS="$PIDS $!"

  PERF_COUNT_ALLOC="$count" PERF_PORT=$P_FLOOR PERF_ADMIN_PORT=$A_FLOOR \
    PERF_UPSTREAM="http://127.0.0.1:$P_MOCK" PERF_WORKERS=$WORKERS \
    "$BIN/floor" >"/tmp/perf-floor.log" 2>&1 & PIDS="$PIDS $!"

  PERF_COUNT_ALLOC="$count" PERF_PORT=$P_FULL PERF_ADMIN_PORT=$A_FULL PERF_MODE=full \
    PERF_UPSTREAM="http://127.0.0.1:$P_MOCK" PERF_WORKERS=$WORKERS \
    "$BIN/gateway" >"/tmp/perf-gw-full.log" 2>&1 & PIDS="$PIDS $!"

  PERF_COUNT_ALLOC="$count" PERF_PORT=$P_NOMW PERF_ADMIN_PORT=$A_NOMW PERF_MODE=nomw \
    PERF_UPSTREAM="http://127.0.0.1:$P_MOCK" PERF_WORKERS=$WORKERS \
    "$BIN/gateway" >"/tmp/perf-gw-nomw.log" 2>&1 & PIDS="$PIDS $!"

  PERF_COUNT_ALLOC="$count" PERF_PORT=$P_IDEM PERF_ADMIN_PORT=$A_IDEM PERF_MODE=full \
    PERF_IDEMPOTENCY=1 PERF_UPSTREAM="http://127.0.0.1:$P_MOCK" PERF_WORKERS=$WORKERS \
    "$BIN/gateway" >"/tmp/perf-gw-idem.log" 2>&1 & PIDS="$PIDS $!"

  wait_ready "http://127.0.0.1:$P_MOCK/health"
  for a in $A_FLOOR $A_FULL $A_NOMW $A_IDEM; do wait_ready "http://127.0.0.1:$a/health"; done
}

stop_stack() { cleanup; PIDS=""; sleep 0.5; }

# ---------------------------------------------------------------- 跑一次

# run <label> <port> <path> <body_bytes> <requests> <mode> <concurrency> <extra...>
run() {
  local label="$1" port="$2" path="$3" body="$4" reqs="$5" mode="$6" conc="$7"; shift 7
  "$BIN/loadgen" --port "$port" --path "$path" --body-bytes "$body" \
    --concurrency "$conc" --requests "$reqs" --mode "$mode" --label "$label" \
    --warmup-ms 1200 --timeout-ms 5000 --out "$RESULTS/$label.json" "$@" >/dev/null 2>>"$RESULTS/loadgen.err"
}

# ---------------------------------------------------------------- 档 1：延迟

phase_latency() {
  log "档 1/5 延迟（concurrency=1，$ROUNDS 轮交错）"
  local r t
  for r in $(seq 1 "$ROUNDS"); do
    for t in floor full nomw; do
      case $t in
        floor) port=$P_FLOOR ;; full) port=$P_FULL ;; nomw) port=$P_NOMW ;;
      esac
      run "lat-small-$t-r$r"  "$port" "$Q_SMALL"    "$B_SMALL" "$N_SMALL"    unary 1
      run "lat-large-$t-r$r"  "$port" "$Q_LARGE"    "$B_LARGE" "$N_LARGE"    unary 1
      run "lat-ssettfb-$t-r$r" "$port" "$Q_SSE_TTFB" "$B_SMALL" "$N_SSE_TTFB" sse   1
    done
    log "  轮 $r/$ROUNDS 完成"
  done
  for r in $(seq 1 "$SSE_ROUNDS"); do
    for t in floor full nomw; do
      case $t in
        floor) port=$P_FLOOR ;; full) port=$P_FULL ;; nomw) port=$P_NOMW ;;
      esac
      run "lat-sse-$t-r$r" "$port" "$Q_SSE" "$B_SMALL" "$N_SSE" sse 1
    done
    log "  SSE 长流轮 $r/$SSE_ROUNDS 完成"
  done
}

# --------------------------------------------------- 档 1b：SSE 满速（每 chunk 成本）
#
# 为什么需要这一档：c) 规定的 1 ms 间隔，在本机实测被放大成 ~2.35 ms 的 mock
# 定时器抖动，三个被测端的 chunk 间隔中位数因此全部落在 2.35 ms ± 10 µs ——
# 网关每 chunk 的真实成本（个位数 µs）被埋在定时器噪声底下，量不出来。
# 把间隔设成 0，chunk 间隔就变成"中继一个 chunk 要多久"本身，差值才有分辨率。

Q_SSE_BURST='/v1/chat/completions?stream=1&chunks=500&chunk_bytes=1024&interval_us=0'

phase_sseburst() {
  log "档 1b SSE 满速（500×1 KiB，无间隔）—— 分辨每 chunk 中继成本"
  local r t port
  for r in $(seq 1 "${ROUNDS}"); do
    for t in floor full nomw; do
      case $t in
        floor) port=$P_FLOOR ;; full) port=$P_FULL ;; nomw) port=$P_NOMW ;;
      esac
      run "lat-sseburst-$t-r$r" "$port" "$Q_SSE_BURST" "$B_SMALL" 300 sse 1
    done
  done
}

# ------------------------------------------ 档 1c：ensure_include_usage 的 JSON 往返
#
# 隔离思路：**同一个 256 KiB 请求体**，响应都压到最小（不让响应干扰），
# 只切换 `stream` 真假。
#   stream=false → payload 走 `to_vec()` + `into_owned()` 两次拷贝
#   stream=true  → 多一次 `ensure_include_usage`：整个 body 反序列化成
#                  `serde_json::Value` 再重新序列化（common.rs:226/238）
# 两者之差，减掉 1 KiB 处同样两档的差（那是"流式路径本身"的固定成本），
# 剩下的就是 JSON 往返随 body 增长的部分。

Q_TINY_UNARY='/v1/chat/completions?resp_bytes=256'
Q_TINY_SSE='/v1/chat/completions?stream=1&chunks=1&chunk_bytes=64&interval_us=0'

phase_jsonrewrite() {
  log "档 1c ensure_include_usage 的 serde_json 往返（同一 body，只切 stream）"
  local r t port
  for r in $(seq 1 "${ROUNDS}"); do
    for t in floor full; do
      case $t in
        floor) port=$P_FLOOR ;; full) port=$P_FULL ;;
      esac
      run "json-u1k-$t-r$r"   "$port" "$Q_TINY_UNARY" "$B_SMALL" 6000 unary 1
      run "json-s1k-$t-r$r"   "$port" "$Q_TINY_SSE"   "$B_SMALL" 6000 sse   1
      run "json-u256k-$t-r$r" "$port" "$Q_TINY_UNARY" "$B_LARGE" 2000 unary 1
      run "json-s256k-$t-r$r" "$port" "$Q_TINY_SSE"   "$B_LARGE" 2000 sse   1
    done
  done
}

# ---------------------------------------------------------------- 档 2：分配

# alloc_run <label> <port> <admin> <path> <body> <reqs> <mode> [extra...]
alloc_run() {
  local label="$1" port="$2" admin="$3" path="$4" body="$5" reqs="$6" mode="$7"; shift 7
  # 预热在 reset 之前，避免把首次分支/连接建立的分配算进来
  "$BIN/loadgen" --port "$port" --path "$path" --body-bytes "$body" --concurrency 1 \
    --requests 200 --mode "$mode" --label warm --warmup-ms 0 --timeout-ms 5000 "$@" >/dev/null 2>&1
  curl -sf -XPOST "http://127.0.0.1:$admin/reset" >/dev/null
  local t0 t1
  t0=$(python3 -c 'import time;print(time.time())')
  "$BIN/loadgen" --port "$port" --path "$path" --body-bytes "$body" --concurrency 1 \
    --requests "$reqs" --mode "$mode" --label "$label" --warmup-ms 0 --timeout-ms 5000 \
    --out "$RESULTS/$label.load.json" "$@" >/dev/null 2>&1
  t1=$(python3 -c 'import time;print(time.time())')
  curl -sf "http://127.0.0.1:$admin/stats" \
    | python3 -c "
import json,sys
s=json.load(sys.stdin)
n=$reqs
print(json.dumps({
  'label': '$label', 'requests': n, 'wall_seconds': $t1-$t0,
  'alloc_count': s['alloc_count'], 'alloc_bytes': s['alloc_bytes'],
  'dealloc_count': s['dealloc_count'], 'realloc_count': s['realloc_count'],
  'alloc_per_req': s['alloc_count']/n, 'bytes_per_req': s['alloc_bytes']/n,
}, indent=2))" > "$RESULTS/$label.alloc.json"
}

# idle_noise <label> <admin> <seconds>
idle_noise() {
  local label="$1" admin="$2" secs="$3"
  curl -sf -XPOST "http://127.0.0.1:$admin/reset" >/dev/null
  sleep "$secs"
  curl -sf "http://127.0.0.1:$admin/stats" | python3 -c "
import json,sys
s=json.load(sys.stdin)
print(json.dumps({'label':'$label','idle_seconds':$secs,
  'alloc_count':s['alloc_count'],'alloc_per_sec':s['alloc_count']/$secs}, indent=2))" \
  > "$RESULTS/$label.idle.json"
}

phase_alloc() {
  log "档 2/5 每请求堆分配（PERF_COUNT_ALLOC=1，需重启进程）"
  stop_stack
  start_stack 1
  idle_noise noise-full "$A_FULL" 5
  idle_noise noise-floor "$A_FLOOR" 5
  alloc_run alloc-small-floor "$P_FLOOR" "$A_FLOOR" "$Q_SMALL" "$B_SMALL" 5000 unary
  alloc_run alloc-small-full  "$P_FULL"  "$A_FULL"  "$Q_SMALL" "$B_SMALL" 5000 unary
  alloc_run alloc-small-nomw  "$P_NOMW"  "$A_NOMW"  "$Q_SMALL" "$B_SMALL" 5000 unary
  alloc_run alloc-large-floor "$P_FLOOR" "$A_FLOOR" "$Q_LARGE" "$B_LARGE" 500  unary
  alloc_run alloc-large-full  "$P_FULL"  "$A_FULL"  "$Q_LARGE" "$B_LARGE" 500  unary
  alloc_run alloc-large-nomw  "$P_NOMW"  "$A_NOMW"  "$Q_LARGE" "$B_LARGE" 500  unary
  alloc_run alloc-sse-floor   "$P_FLOOR" "$A_FLOOR" "$Q_SSE"   "$B_SMALL" 20   sse
  alloc_run alloc-sse-full    "$P_FULL"  "$A_FULL"  "$Q_SSE"   "$B_SMALL" 20   sse
  alloc_run alloc-sse-nomw    "$P_NOMW"  "$A_NOMW"  "$Q_SSE"   "$B_SMALL" 20   sse
  alloc_run alloc-small-idem  "$P_IDEM"  "$A_IDEM"  "$Q_SMALL" "$B_SMALL" 5000 unary --idempotency 1
  alloc_run alloc-large-idem  "$P_IDEM"  "$A_IDEM"  "$Q_LARGE" "$B_LARGE" 500  unary --idempotency 1
  stop_stack
  start_stack 0
}

# ---------------------------------------------------------------- 档 3：吞吐

phase_throughput() {
  log "档 3/5 吞吐（concurrency=16，本机后台负载会污染绝对值）"
  local r t port
  for r in $(seq 1 3); do
    for t in floor full nomw; do
      case $t in
        floor) port=$P_FLOOR ;; full) port=$P_FULL ;; nomw) port=$P_NOMW ;;
      esac
      "$BIN/loadgen" --port "$port" --path "$Q_SMALL" --body-bytes "$B_SMALL" \
        --concurrency 16 --duration 5 --mode unary --label "tput-small-$t-r$r" \
        --warmup-ms 1500 --timeout-ms 5000 --out "$RESULTS/tput-small-$t-r$r.json" >/dev/null 2>&1
    done
  done
}

# ---------------------------------------------------------------- 档 4：幂等

phase_idempotency() {
  log "档 4/5 幂等开销（hold 层 capture_body 全量缓冲响应体）"
  local r
  for r in $(seq 1 3); do
    run "idem-small-on-r$r"  "$P_IDEM" "$Q_SMALL" "$B_SMALL" "$N_SMALL" unary 1 --idempotency 1
    run "idem-small-off-r$r" "$P_FULL" "$Q_SMALL" "$B_SMALL" "$N_SMALL" unary 1
    run "idem-large-on-r$r"  "$P_IDEM" "$Q_LARGE" "$B_LARGE" "$N_LARGE" unary 1 --idempotency 1
    run "idem-large-off-r$r" "$P_FULL" "$Q_LARGE" "$B_LARGE" "$N_LARGE" unary 1
  done
}

# ---------------------------------------------------------------- 档 5：profile

phase_profile() {
  log "档 5/5 CPU 采样（/usr/bin/sample，macOS 自带）"
  if [ ! -x /usr/bin/sample ]; then
    echo "/usr/bin/sample 不可用，跳过 profile 档" >&2
    return 0
  fi
  local gw_pid
  gw_pid=$(lsof -nP -iTCP:$P_FULL -sTCP:LISTEN -t 2>/dev/null | head -1)
  if [ -z "$gw_pid" ]; then
    echo "找不到 gateway 进程，跳过 profile" >&2
    return 0
  fi
  "$BIN/loadgen" --port "$P_FULL" --path "$Q_SMALL" --body-bytes "$B_SMALL" \
    --concurrency 8 --duration 22 --mode unary --label profile-load --warmup-ms 500 --timeout-ms 5000 \
    --out "$RESULTS/profile-load.json" >/dev/null 2>&1 &
  local load_pid=$!
  sleep 3
  /usr/bin/sample "$gw_pid" 15 1 -f "$RESULTS/profile-gateway-full.txt" >/dev/null 2>&1 || \
    echo "sample 失败（可能是 SIP / 权限），profile 档无数据" >&2
  wait $load_pid || true

  local floor_pid
  floor_pid=$(lsof -nP -iTCP:$P_FLOOR -sTCP:LISTEN -t 2>/dev/null | head -1)
  if [ -n "$floor_pid" ]; then
    "$BIN/loadgen" --port "$P_FLOOR" --path "$Q_SMALL" --body-bytes "$B_SMALL" \
      --concurrency 8 --duration 22 --mode unary --label profile-load-floor --warmup-ms 500 --timeout-ms 5000 \
      --out "$RESULTS/profile-load-floor.json" >/dev/null 2>&1 &
    load_pid=$!
    sleep 3
    /usr/bin/sample "$floor_pid" 15 1 -f "$RESULTS/profile-floor.txt" >/dev/null 2>&1 || true
    wait $load_pid || true
  fi
}

# ---------------------------------------------------------------- 主流程

build
{
  echo "host: $(uname -srm)"
  echo "cpu: $(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo unknown)"
  echo "ncpu: $(sysctl -n hw.ncpu 2>/dev/null || nproc)"
  echo "rustc: $(rustc --version)"
  echo "started: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "loadavg_before: $(uptime | sed 's/.*load averages*: //')"
  echo "rounds: $ROUNDS  sse_rounds: $SSE_ROUNDS  workers: $WORKERS"
} > "$RESULTS/env.txt"

start_stack 0
for p in $PHASES; do
  case "$p" in
    latency)      phase_latency ;;
    sseburst)     phase_sseburst ;;
    jsonrewrite)  phase_jsonrewrite ;;
    alloc)        phase_alloc ;;
    throughput)   phase_throughput ;;
    idempotency)  phase_idempotency ;;
    profile)      phase_profile ;;
    *) echo "未知档: $p" >&2 ;;
  esac
done
echo "loadavg_after: $(uptime | sed 's/.*load averages*: //')" >> "$RESULTS/env.txt"
stop_stack

log "全部完成。汇总： python3 $ROOT/scripts/perf/summarize.py"
