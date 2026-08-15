#!/usr/bin/env bash
# 一键启动本地开发前后端（双热更新），Ctrl+C 同时退出。
#
# 用法:
#   ./dev.sh                 # 前端默认 http://127.0.0.1:3000
#   FRONTEND_PORT=5173 ./dev.sh
#
# - 后端 http://127.0.0.1:8888：cargo watch 监听 crates/、migrations/、
#   Cargo.toml、Cargo.lock、config.yaml，改动后自动重编译重启。
#   端口固定 8888 —— frontend/vite.config.ts 把 /api、/v1、/v1beta、/healthz
#   等硬编码代理到 127.0.0.1:8888，改了代理会断。
# - 前端 vite dev server：源码 HMR 热更新；FRONTEND_PORT 可覆盖（默认 3000，
#   与 config.example.yaml 的 frontend.port 一致），被占用时 strictPort 直接报错。
#
# 前置依赖:
#   - Postgres/Redis 按 config.yaml 可达，否则后端启动失败并退出（脚本随之整体退出）
#   - config.yaml 必须存在（cp config.example.yaml config.yaml）
#   - cargo-watch 缺失时报错并给出安装命令；frontend/node_modules 缺失时自动 npm ci
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKEND_PORT=8888
FRONTEND_PORT="${FRONTEND_PORT:-3000}"
CONFIG="$ROOT/config.yaml"

# ── 前置检查 ──
if ! command -v cargo-watch >/dev/null 2>&1; then
  echo "缺少 cargo-watch（后端热重载依赖）：cargo install cargo-watch" >&2
  exit 1
fi
if ! command -v npm >/dev/null 2>&1; then
  echo "缺少 npm：请先安装 Node.js" >&2
  exit 1
fi
if [[ ! -f "$CONFIG" ]]; then
  echo "缺少 $CONFIG：cp config.example.yaml config.yaml 后按需修改数据库/Redis 连接" >&2
  exit 1
fi
if [[ ! -x "$ROOT/frontend/node_modules/.bin/vite" ]]; then
  echo "frontend 依赖未安装，先执行 npm ci ..."
  # typescript@7 与 typescript-eslint@8 的 peer 范围冲突是既定的：
  # eslint 走 @typescript/typescript6 fork（见 scripts/ts6-for-eslint.cjs），
  # 裸 npm ci 会 ERESOLVE 失败，必须带 --legacy-peer-deps。
  (cd "$ROOT/frontend" && npm ci --legacy-peer-deps)
fi

# ── 进程清理 ──
BACKEND_PID=""
FRONTEND_PID=""

# 先递归杀子进程再杀自身：cargo-watch → cargo → gw-server 是一条链，
# 只 kill 顶层会把 gw-server 孤儿留给 runtime。
kill_tree() {
  local pid="$1" child children
  children="$(pgrep -P "$pid" 2>/dev/null || true)"
  for child in $children; do
    kill_tree "$child"
  done
  kill "$pid" 2>/dev/null || true
}

cleanup() {
  local status=$?
  trap - INT TERM EXIT
  [[ -n "$BACKEND_PID" ]] && kill_tree "$BACKEND_PID"
  [[ -n "$FRONTEND_PID" ]] && kill_tree "$FRONTEND_PID"
  wait "$BACKEND_PID" "$FRONTEND_PID" 2>/dev/null || true
  exit "$status"
}
trap cleanup INT TERM EXIT

# ── 启动 ──
echo "后端  http://127.0.0.1:$BACKEND_PORT  （cargo watch：crates/migrations/config.yaml 改动即重编译重启）"
echo "前端  http://127.0.0.1:$FRONTEND_PORT  （vite HMR）"
echo "后端首次编译需要几分钟；编译完成后 API 经 vite 代理到后端，访问前端地址即可。"
echo

(
  cd "$ROOT"
  cargo watch \
    -w crates \
    -w migrations \
    -w Cargo.toml \
    -w Cargo.lock \
    -w config.yaml \
    -x "run -- --config $CONFIG"
) &
BACKEND_PID=$!

(
  cd "$ROOT/frontend"
  # vite 8 的默认 host localhost 在这台机器上只绑 [::1]，与打印的 URL 不一致；
  # 显式绑 127.0.0.1，保证 http://127.0.0.1:$FRONTEND_PORT 真实可达。
  ./node_modules/.bin/vite --host 127.0.0.1 --port "$FRONTEND_PORT" --strictPort
) &
FRONTEND_PID=$!

# ── 任一进程退出 → 整体退出 ──
while kill -0 "$BACKEND_PID" 2>/dev/null && kill -0 "$FRONTEND_PID" 2>/dev/null; do
  sleep 1
done

if ! kill -0 "$BACKEND_PID" 2>/dev/null; then
  echo "后端已退出（编译错误或启动失败，见上方 cargo 输出），已停掉前端。" >&2
else
  echo "前端已退出，已停掉后端。" >&2
fi
exit 1
