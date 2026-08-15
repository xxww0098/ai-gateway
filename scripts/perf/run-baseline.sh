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
# failover 档默认**不在**里面：它要单独重启 relay 进程（换 handler + 换 body 模式），
# 与 T1–T13 那一轮混跑会打断"同轮交错"。用 `PHASES=failover` 单独跑。
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
P_RELAY=18088;  A_RELAY=18098
# TLS + h2 档单独一套端口：它要重启整个栈（mock 换 https、两个被测端换 client），
# 与明文档同时在跑会互相抢连接池。
P_MOCK_TLS=18071
P_FLOOR_TLS=18072; A_FLOOR_TLS=18073
P_RELAY_TLS=18074; A_RELAY_TLS=18075

# 场景 path（query 会被网关原样透传给 mock，用来选形态）
Q_SMALL='/v1/chat/completions?resp_bytes=2048'
Q_LARGE='/v1/chat/completions?resp_bytes=1048576'
Q_SSE='/v1/chat/completions?stream=1&chunks=500&chunk_bytes=1024&interval_us=1000'
Q_SSE_TTFB='/v1/chat/completions?stream=1&chunks=1&chunk_bytes=1024&interval_us=0'

B_SMALL=1024
B_LARGE=262144

# 交错跑的被测端列表。加/减被测端只改这一行 —— 下面每个档都从它展开，
# 免得"某个档忘了加 relay"这种只在汇总表里显示为空行的错误。
# `PERF_TARGETS="floor relay"` 可以在 gw-proxy 编不过时只跑这两个。
TARGETS="${PERF_TARGETS:-floor full nomw relay}"
# jsonrewrite 档只需要"被测端 vs 下界"，跑全部被测端是浪费（它是 4 组 × 每组 2000~6000 发）。
JSON_TARGETS="${PERF_JSON_TARGETS:-floor full relay}"

# 被测端 → 端口 / admin 端口。bash 3.2 没有关联数组。
port_of() {
  case "$1" in
    floor) echo $P_FLOOR ;; full) echo $P_FULL ;; nomw) echo $P_NOMW ;;
    idem)  echo $P_IDEM  ;; relay) echo $P_RELAY ;;
    *) echo "未知被测端: $1" >&2; return 1 ;;
  esac
}
admin_of() {
  case "$1" in
    floor) echo $A_FLOOR ;; full) echo $A_FULL ;; nomw) echo $A_NOMW ;;
    idem)  echo $A_IDEM  ;; relay) echo $A_RELAY ;;
    *) echo "未知被测端: $1" >&2; return 1 ;;
  esac
}

mkdir -p "$RESULTS"

log() { printf '\033[36m[perf]\033[0m %s\n' "$*" >&2; }

# ---------------------------------------------------------------- 构建

# `gateway` 被测端要拖进 gw-proxy / gw-provider 半个工作区。wave 3 里那两个 crate
# 可能正被别的 worker 改着、编不过 —— 此时 `PERF_NO_GATEWAY=1` 只构建
# floor / relay / mock / loadgen，照样能跑 relay 对 floor 的那几档。
# 少了哪些被测端会明明白白写进 env.txt，不会变成一张看起来完整的空表。
build() {
  local feat=""
  if [ "${PERF_NO_GATEWAY:-0}" = "1" ]; then
    feat="--no-default-features"
    log "PERF_NO_GATEWAY=1 —— 跳过 gateway 被测端（full / nomw / idem 不会启动）"
    TARGETS="$(echo "$TARGETS" | tr ' ' '\n' | grep -Ev '^(full|nomw|idem)$' | tr '\n' ' ')"
    JSON_TARGETS="$(echo "$JSON_TARGETS" | tr ' ' '\n' | grep -Ev '^(full|nomw|idem)$' | tr '\n' ' ')"
  fi
  log "构建 release（CARGO_TARGET_DIR=${TARGET_DIR}${feat:+ $feat}）"
  (cd "$PERFKIT" && CARGO_TARGET_DIR="$TARGET_DIR" cargo build --release $feat >&2)
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
  log "启动 mock + floor + relay + gateway(full/nomw/idem)，PERF_COUNT_ALLOC=$count"
  PERF_COUNT_ALLOC="$count" "$BIN/mock-upstream" "$P_MOCK" >"/tmp/perf-mock.log" 2>&1 & PIDS="$PIDS $!"

  PERF_COUNT_ALLOC="$count" PERF_PORT=$P_FLOOR PERF_ADMIN_PORT=$A_FLOOR \
    PERF_UPSTREAM="http://127.0.0.1:$P_MOCK" PERF_WORKERS=$WORKERS \
    "$BIN/floor" >"/tmp/perf-floor.log" 2>&1 & PIDS="$PIDS $!"

  PERF_COUNT_ALLOC="$count" PERF_PORT=$P_RELAY PERF_ADMIN_PORT=$A_RELAY \
    PERF_UPSTREAM="http://127.0.0.1:$P_MOCK" PERF_WORKERS=$WORKERS \
    "$BIN/relay" >"/tmp/perf-relay.log" 2>&1 & PIDS="$PIDS $!"

  local admins="$A_FLOOR $A_RELAY"
  if [ -x "$BIN/gateway" ] && [ "${PERF_NO_GATEWAY:-0}" != "1" ]; then
    PERF_COUNT_ALLOC="$count" PERF_PORT=$P_FULL PERF_ADMIN_PORT=$A_FULL PERF_MODE=full \
      PERF_UPSTREAM="http://127.0.0.1:$P_MOCK" PERF_WORKERS=$WORKERS \
      "$BIN/gateway" >"/tmp/perf-gw-full.log" 2>&1 & PIDS="$PIDS $!"

    PERF_COUNT_ALLOC="$count" PERF_PORT=$P_NOMW PERF_ADMIN_PORT=$A_NOMW PERF_MODE=nomw \
      PERF_UPSTREAM="http://127.0.0.1:$P_MOCK" PERF_WORKERS=$WORKERS \
      "$BIN/gateway" >"/tmp/perf-gw-nomw.log" 2>&1 & PIDS="$PIDS $!"

    PERF_COUNT_ALLOC="$count" PERF_PORT=$P_IDEM PERF_ADMIN_PORT=$A_IDEM PERF_MODE=full \
      PERF_IDEMPOTENCY=1 PERF_UPSTREAM="http://127.0.0.1:$P_MOCK" PERF_WORKERS=$WORKERS \
      "$BIN/gateway" >"/tmp/perf-gw-idem.log" 2>&1 & PIDS="$PIDS $!"
    admins="$admins $A_FULL $A_NOMW $A_IDEM"
  fi

  wait_ready "http://127.0.0.1:$P_MOCK/health"
  for a in $admins; do wait_ready "http://127.0.0.1:$a/health"; done
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
  local r t port
  for r in $(seq 1 "$ROUNDS"); do
    for t in $TARGETS; do
      port=$(port_of "$t")
      run "lat-small-$t-r$r"  "$port" "$Q_SMALL"    "$B_SMALL" "$N_SMALL"    unary 1
      run "lat-large-$t-r$r"  "$port" "$Q_LARGE"    "$B_LARGE" "$N_LARGE"    unary 1
      run "lat-ssettfb-$t-r$r" "$port" "$Q_SSE_TTFB" "$B_SMALL" "$N_SSE_TTFB" sse   1
    done
    log "  轮 $r/$ROUNDS 完成"
  done
  for r in $(seq 1 "$SSE_ROUNDS"); do
    for t in $TARGETS; do
      port=$(port_of "$t")
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
    for t in $TARGETS; do
      port=$(port_of "$t")
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
# 第五格：**小请求 + 1 MiB 响应**。上面四格全是"响应压到最小"，量的都是请求侧；
# 大 body 的开销到底落在请求侧还是响应侧，只有加上这一格才拆得开。
# wave 3 加的：relay 在 b) 档比 floor 慢 559 µs，而 256 KiB 请求侧只解释了 29 µs。
Q_RESP1M='/v1/chat/completions?resp_bytes=1048576'

phase_jsonrewrite() {
  log "档 1c ensure_include_usage 的 serde_json 往返（同一 body，只切 stream）"
  local r t port
  for r in $(seq 1 "${ROUNDS}"); do
    for t in $JSON_TARGETS; do
      port=$(port_of "$t")
      run "json-u1k-$t-r$r"   "$port" "$Q_TINY_UNARY" "$B_SMALL" 6000 unary 1
      run "json-s1k-$t-r$r"   "$port" "$Q_TINY_SSE"   "$B_SMALL" 6000 sse   1
      run "json-u256k-$t-r$r" "$port" "$Q_TINY_UNARY" "$B_LARGE" 2000 unary 1
      run "json-s256k-$t-r$r" "$port" "$Q_TINY_SSE"   "$B_LARGE" 2000 sse   1
      run "json-resp1m-$t-r$r" "$port" "$Q_RESP1M"    "$B_SMALL" 1500 unary 1
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
  local t port admin
  for t in $TARGETS; do
    port=$(port_of "$t"); admin=$(admin_of "$t")
    idle_noise "noise-$t" "$admin" 5
    alloc_run "alloc-small-$t" "$port" "$admin" "$Q_SMALL" "$B_SMALL" 5000 unary
    alloc_run "alloc-large-$t" "$port" "$admin" "$Q_LARGE" "$B_LARGE" 500  unary
    alloc_run "alloc-sse-$t"   "$port" "$admin" "$Q_SSE"   "$B_SMALL" 20   sse
  done
  if [ -x "$BIN/gateway" ] && [ "${PERF_NO_GATEWAY:-0}" != "1" ]; then
    alloc_run alloc-small-idem "$P_IDEM" "$A_IDEM" "$Q_SMALL" "$B_SMALL" 5000 unary --idempotency 1
    alloc_run alloc-large-idem "$P_IDEM" "$A_IDEM" "$Q_LARGE" "$B_LARGE" 500  unary --idempotency 1
  fi
  stop_stack
  start_stack 0
}

# ---------------------------------------------------------------- 档 3：吞吐

phase_throughput() {
  log "档 3/5 吞吐（concurrency=16，本机后台负载会污染绝对值）"
  local r t port
  for r in $(seq 1 3); do
    for t in $TARGETS; do
      port=$(port_of "$t")
      "$BIN/loadgen" --port "$port" --path "$Q_SMALL" --body-bytes "$B_SMALL" \
        --concurrency 16 --duration 5 --mode unary --label "tput-small-$t-r$r" \
        --warmup-ms 1500 --timeout-ms 5000 --out "$RESULTS/tput-small-$t-r$r.json" >/dev/null 2>&1
    done
  done
}

# ---------------------------------------------------------------- 档 4：幂等

phase_idempotency() {
  # 幂等只有 gateway 被测端有（hold 层的 capture_body）。gw-relay 里没有幂等，
  # T11 因此是"未覆盖"而不是"未达标" —— 见 docs/relay-perf-acceptance.md。
  if [ ! -x "$BIN/gateway" ] || [ "${PERF_NO_GATEWAY:-0}" = "1" ]; then
    log "档 4/5 幂等 —— gateway 被测端没起，跳过"
    return 0
  fi
  log "档 4/5 幂等开销（hold 层 capture_body 全量缓冲响应体）"
  local r
  for r in $(seq 1 3); do
    run "idem-small-on-r$r"  "$P_IDEM" "$Q_SMALL" "$B_SMALL" "$N_SMALL" unary 1 --idempotency 1
    run "idem-small-off-r$r" "$P_FULL" "$Q_SMALL" "$B_SMALL" "$N_SMALL" unary 1
    run "idem-large-on-r$r"  "$P_IDEM" "$Q_LARGE" "$B_LARGE" "$N_LARGE" unary 1 --idempotency 1
    run "idem-large-off-r$r" "$P_FULL" "$Q_LARGE" "$B_LARGE" "$N_LARGE" unary 1
  done
}

# ------------------------------------------- 档 6：跨账号 failover 的重放代价
#
# `docs/relay-perf-baseline.md` §5.2 的第二个未覆盖项。基线里凭证池只有 1 条且
# 从不失败，所以"`Bytes` 化到底省了多少"一直只有结构推断。
#
# 装置：上游按 `?fail_first=N` 回 N 次 429，relay 带 `x-perf-attempt` 重试到成功。
# 两个 body 模式跑**同一条路径**，只差一行：
#   bytes → `Bytes::clone`（refcount 加一）
#   vec   → `Bytes::copy_from_slice`（全量拷贝，复刻 routes.rs:217 的 to_vec()）
# 两者之差 = `Bytes` 化省下来的东西。attempts 取 1/2/4 是为了看它随重试次数线性。
#
# 这一档**自己起 relay 进程**（handler 与 body 模式是启动期决定的），
# 所以它先 stop_stack，跑完再把常规栈拉回来。

phase_failover() {
  log "档 6 跨账号 failover 的重放代价（bytes vs vec，1/2/4 次尝试）"
  stop_stack
  PERF_COUNT_ALLOC=1 "$BIN/mock-upstream" "$P_MOCK" >"/tmp/perf-mock.log" 2>&1 & PIDS="$PIDS $!"
  wait_ready "http://127.0.0.1:$P_MOCK/health"

  local mode n fails label
  for mode in bytes vec; do
    PERF_COUNT_ALLOC=1 PERF_PORT=$P_RELAY PERF_ADMIN_PORT=$A_RELAY \
      PERF_UPSTREAM="http://127.0.0.1:$P_MOCK" PERF_WORKERS=$WORKERS \
      PERF_RELAY_FAILOVER=8 PERF_RELAY_BODY_MODE="$mode" \
      "$BIN/relay" >"/tmp/perf-relay-fo-$mode.log" 2>&1 & PIDS="$PIDS $!"
    wait_ready "http://127.0.0.1:$A_RELAY/health"

    for n in 1 2 4; do
      fails=$((n - 1))
      label="fo-$mode-a$n"
      # 延迟：跨轮交错，与其它档同一个纪律。
      local r
      for r in $(seq 1 "$ROUNDS"); do
        run "$label-r$r" "$P_RELAY" \
          "/v1/chat/completions?resp_bytes=2048&fail_first=$fails" "$B_LARGE" 400 unary 1
      done
      # 分配：同一个进程，reset 之后重打一遍。
      alloc_run "alloc-$label" "$P_RELAY" "$A_RELAY" \
        "/v1/chat/completions?resp_bytes=2048&fail_first=$fails" "$B_LARGE" 300 unary
    done

    # 换 body 模式要换进程（handler 与模式都是启动期决定的）。
    stop_stack
    PERF_COUNT_ALLOC=1 "$BIN/mock-upstream" "$P_MOCK" >"/tmp/perf-mock.log" 2>&1 & PIDS="$PIDS $!"
    wait_ready "http://127.0.0.1:$P_MOCK/health"
  done
  stop_stack
  start_stack 0
}

# ------------------------------------------------- 档 7：TLS + HTTP/2 的对照
#
# `docs/relay-perf-baseline.md` §1.4 / §5.2 的第一个未覆盖项，原文：
#   「生产上游是 https，会走 h2 —— h2 路径没有覆盖」
#   「上线前必须补一档 TLS+h2 的对照，否则 T4/T5 在 h2 的分帧开销下会失真」
#
# 装置：mock 上游用自签证书跑 https（ALPN 给 h2），floor 与 relay 都换成放行
# 自签证书的 client。**客户端一侧（loadgen → 被测端）仍然是明文 HTTP/1.1** ——
# 要量的是上游那一跳的分帧开销，不是客户端那一跳。
#
# relay 侧换的是 `Transport`（`RelayEngine::with_transport`），池配置逐字照抄
# `gw-relay` 的 `shared_client()`，只多一条 `danger_accept_invalid_certs`。
# 见 `perfkit/src/bin/relay.rs` 的 `TlsTransport`。
#
# 已知不可比之处：证书校验被关掉了（自签），所以这一档**不含**证书链验证的成本。
# 它每连接一次、且 keep-alive 下摊到近零，但记在这里免得数字被当成含它。

CERT="${PERF_TLS_CERT:-/tmp/perf-tls-cert.pem}"
KEY="${PERF_TLS_KEY:-/tmp/perf-tls-key.pem}"

phase_tls() {
  log "档 7 TLS + HTTP/2 对照（上游那一跳走 https/h2）"
  if [ ! -f "$CERT" ] || [ ! -f "$KEY" ]; then
    log "  生成自签证书 $CERT"
    # `-addext` 不是可选项：不带扩展时 openssl 生成的是 X.509 **v1** 证书，
    # rustls 直接拒收（`UnsupportedCertVersion`），而且 client 那边关掉了证书校验
    # 也救不了 —— 版本检查在校验之前。第一次跑这一档正是这么废掉的。
    openssl req -x509 -newkey rsa:2048 -keyout "$KEY" -out "$CERT" \
      -days 2 -nodes -subj "/CN=localhost" \
      -addext "subjectAltName=IP:127.0.0.1,DNS:localhost" >/dev/null 2>&1 || {
        echo "openssl 生成证书失败，跳过 TLS 档" >&2; return 0; }
  fi
  stop_stack

  PERF_TLS_CERT="$CERT" PERF_TLS_KEY="$KEY" \
    "$BIN/mock-upstream" "$P_MOCK_TLS" >"/tmp/perf-mock-tls.log" 2>&1 & PIDS="$PIDS $!"
  PERF_TLS=1 PERF_PORT=$P_FLOOR_TLS PERF_ADMIN_PORT=$A_FLOOR_TLS PERF_WORKERS=$WORKERS \
    PERF_UPSTREAM="https://127.0.0.1:$P_MOCK_TLS" \
    "$BIN/floor" >"/tmp/perf-floor-tls.log" 2>&1 & PIDS="$PIDS $!"
  PERF_TLS=1 PERF_PORT=$P_RELAY_TLS PERF_ADMIN_PORT=$A_RELAY_TLS PERF_WORKERS=$WORKERS \
    PERF_UPSTREAM="https://127.0.0.1:$P_MOCK_TLS" \
    "$BIN/relay" >"/tmp/perf-relay-tls.log" 2>&1 & PIDS="$PIDS $!"
  for a in $A_FLOOR_TLS $A_RELAY_TLS; do wait_ready "http://127.0.0.1:$a/health"; done
  # mock 是 https，`wait_ready` 的 curl 得放行自签证书。
  local i=0
  while [ $i -lt 100 ]; do
    if curl -skf "https://127.0.0.1:$P_MOCK_TLS/health" >/dev/null 2>&1; then break; fi
    sleep 0.1; i=$((i+1))
  done
  if [ $i -ge 100 ]; then
    echo "TLS mock 未就绪，跳过 TLS 档（看 /tmp/perf-mock-tls.log）" >&2
    stop_stack; start_stack 0; return 0
  fi

  local r t port
  for r in $(seq 1 "$ROUNDS"); do
    for t in floor relay; do
      case $t in floor) port=$P_FLOOR_TLS ;; relay) port=$P_RELAY_TLS ;; esac
      run "tls-small-$t-r$r" "$port" "$Q_SMALL" "$B_SMALL" "$N_SMALL" unary 1
      run "tls-large-$t-r$r" "$port" "$Q_LARGE" "$B_LARGE" "$N_LARGE" unary 1
      run "tls-ssettfb-$t-r$r" "$port" "$Q_SSE_TTFB" "$B_SMALL" "$N_SSE_TTFB" sse 1
    done
    log "  TLS 轮 $r/$ROUNDS 完成"
  done
  stop_stack
  start_stack 0
}

# ---------------------------------------------------------------- 档 5：profile

phase_profile() {
  log "档 5/5 CPU 采样（/usr/bin/sample，macOS 自带）"
  if [ ! -x /usr/bin/sample ]; then
    echo "/usr/bin/sample 不可用，跳过 profile 档" >&2
    return 0
  fi
  # 每个被测端一份：同样 15 s、同样 concurrency=8 的负载，逐个采。
  # 文件名保持 §2.7 已有的两个（profile-gateway-full / profile-floor），
  # 新被测端一律 profile-<t>.txt —— profile-summary.py 按这个约定找。
  local t port pid load_pid out
  for t in $TARGETS; do
    port=$(port_of "$t")
    pid=$(lsof -nP -iTCP:"$port" -sTCP:LISTEN -t 2>/dev/null | head -1)
    if [ -z "$pid" ]; then
      echo "找不到 $t 进程（:$port），跳过它的 profile" >&2
      continue
    fi
    case "$t" in
      full) out="$RESULTS/profile-gateway-full.txt" ;;
      *)    out="$RESULTS/profile-$t.txt" ;;
    esac
    "$BIN/loadgen" --port "$port" --path "$Q_SMALL" --body-bytes "$B_SMALL" \
      --concurrency 8 --duration 22 --mode unary --label "profile-load-$t" --warmup-ms 500 --timeout-ms 5000 \
      --out "$RESULTS/profile-load-$t.json" >/dev/null 2>&1 &
    load_pid=$!
    sleep 3
    /usr/bin/sample "$pid" 15 1 -f "$out" >/dev/null 2>&1 || \
      echo "sample $t 失败（可能是 SIP / 权限），这一份无数据" >&2
    wait $load_pid || true
  done
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
  echo "targets: $TARGETS"
  echo "json_targets: $JSON_TARGETS"
  echo "phases: $PHASES"
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
    failover)     phase_failover ;;
    tls)          phase_tls ;;
    profile)      phase_profile ;;
    *) echo "未知档: $p" >&2 ;;
  esac
done
echo "loadavg_after: $(uptime | sed 's/.*load averages*: //')" >> "$RESULTS/env.txt"
stop_stack

log "全部完成。汇总： python3 $ROOT/scripts/perf/summarize.py"
