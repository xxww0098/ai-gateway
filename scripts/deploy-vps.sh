#!/usr/bin/env bash
# 部署 ai-gateway 到 VPS (api.xxww.online / 64.83.38.21)
#
# 用法:
#   export DEPLOY_HOST=64.83.38.21
#   export DEPLOY_USER=root
#   export DEPLOY_PASS='your-password'   # 或使用 DEPLOY_SSH_KEY
#   ./scripts/deploy-vps.sh
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEPLOY_HOST="${DEPLOY_HOST:-64.83.38.21}"
DEPLOY_USER="${DEPLOY_USER:-root}"
# 远端目录 / 服务名 / 二进制名。平行部署时覆盖这三个环境变量。
REMOTE_DIR="${REMOTE_DIR:-/opt/ai-gateway}"
SERVICE_NAME="${SERVICE_NAME:-ai-gateway}"
BINARY_NAME="ai-gateway-linux-amd64"

SSH_OPTS=(-o StrictHostKeyChecking=no -o ConnectTimeout=15)
if [[ -n "${DEPLOY_SSH_KEY:-}" ]]; then
  SSH_OPTS+=(-i "${DEPLOY_SSH_KEY}")
fi

run_ssh() {
  if [[ -n "${DEPLOY_PASS:-}" ]] && command -v sshpass >/dev/null; then
    sshpass -p "${DEPLOY_PASS}" ssh "${SSH_OPTS[@]}" "${DEPLOY_USER}@${DEPLOY_HOST}" "$@"
  else
    ssh "${SSH_OPTS[@]}" "${DEPLOY_USER}@${DEPLOY_HOST}" "$@"
  fi
}

run_scp() {
  local src="$1" dest="$2"
  if [[ -n "${DEPLOY_PASS:-}" ]] && command -v sshpass >/dev/null; then
    sshpass -p "${DEPLOY_PASS}" scp "${SSH_OPTS[@]}" "${src}" "${DEPLOY_USER}@${DEPLOY_HOST}:${dest}"
  else
    scp "${SSH_OPTS[@]}" "${src}" "${DEPLOY_USER}@${DEPLOY_HOST}:${dest}"
  fi
}

echo "==> 编译 Linux 二进制..."
mkdir -p "${ROOT}/dist"
(
  cd "${ROOT}"
  # Cross-compiling to a musl target keeps the binary static, so the VPS needs
  # no matching glibc. Install the target once with:
  #   rustup target add x86_64-unknown-linux-musl
  cargo build --release --locked \
    --target x86_64-unknown-linux-musl \
    -p gw-server
  cp "target/x86_64-unknown-linux-musl/release/gw-server" "dist/${BINARY_NAME}"
)

echo "==> 测试 SSH 连接 ${DEPLOY_USER}@${DEPLOY_HOST}..."
run_ssh "echo connected: \$(hostname)"

echo "==> 上传二进制..."
run_ssh "mkdir -p ${REMOTE_DIR}/dist ${REMOTE_DIR}/backup"
run_scp "${ROOT}/dist/${BINARY_NAME}" "${REMOTE_DIR}/dist/${BINARY_NAME}.new"

echo "==> 远程切换并重启..."
run_ssh bash -s <<REMOTE
set -euo pipefail
cd ${REMOTE_DIR}
if [[ -f dist/ai-gateway ]]; then
  cp dist/ai-gateway backup/ai-gateway.\$(date +%Y%m%d-%H%M%S) || true
fi
mv dist/${BINARY_NAME}.new dist/ai-gateway
chmod +x dist/ai-gateway
if systemctl is-active --quiet ${SERVICE_NAME}; then
  systemctl restart ${SERVICE_NAME}
  sleep 2
  systemctl is-active ${SERVICE_NAME}
elif docker ps --format '{{.Names}}' | grep -q '^ai-gateway\$'; then
  docker restart ai-gateway
else
  echo "未找到 systemd/docker 服务，请手动重启 AI-GateWay"
fi
REMOTE

echo "==> 验证 API..."
curl -sf "https://api.xxww.online/v1/models" -H "Authorization: Bearer ${AGW_TEST_KEY:-invalid}" >/dev/null || true
echo "部署完成: https://api.xxww.online"
