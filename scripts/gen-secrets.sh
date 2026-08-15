#!/usr/bin/env bash
# 生成 ai-gateway 生产环境所需的强密钥。
#
# 用法:
#   ./scripts/gen-secrets.sh            # 打印可直接粘贴的 env 片段
#   ./scripts/gen-secrets.sh >> .env    # 追加到 env 文件
#
# 生成两把 32 字节（256 位）密钥：
#   - JWT_SECRET                 面板 JWT 的 HS256 签名密钥（启动时要求 >= 32 字节）
#   - CREDENTIAL_ENCRYPTION_KEY  上游凭证落库 AES-256-GCM 加密密钥（auth_records.metadata）
#
# 二者均从 CSPRNG 取随机字节后以 hex 编码（64 个十六进制字符）。请妥善保管：
#   - JWT_SECRET 轮换会使所有现有面板会话失效（用户需重新登录）。
#   - CREDENTIAL_ENCRYPTION_KEY 一旦丢失/更换，已加密的上游凭证将无法解密
#     （需要重新录入凭证），切勿在已有加密数据后随意更换。
set -euo pipefail

gen_key() {
  # 优先 openssl；回退到 /dev/urandom + xxd/od。
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 32
  elif command -v xxd >/dev/null 2>&1; then
    head -c 32 /dev/urandom | xxd -p -c 64
  else
    od -An -tx1 -N32 /dev/urandom | tr -d ' \n'
    echo
  fi
}

JWT_SECRET="$(gen_key)"
CREDENTIAL_ENCRYPTION_KEY="$(gen_key)"

cat <<EOF
# --- ai-gateway 生产密钥（由 scripts/gen-secrets.sh 生成，请妥善保管，勿入库）---
export JWT_SECRET=${JWT_SECRET}
export CREDENTIAL_ENCRYPTION_KEY=${CREDENTIAL_ENCRYPTION_KEY}
EOF
