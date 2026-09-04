# Claude Code fingerprint（OAuth / 订阅）

Claude Code 的 access token **只授权给 Claude Code 客户端**。半套伪装（头像 Node、TLS 仍是 rustls）是封号来源。

**本仓库现在 fail-closed：** OAuth 的 Messages / count_tokens **不会**交给 `gw-relay`。`claude::fingerprint` 仍会算出头和 billing 块，供对照和以后开闸；`refuse_unverified_send` 在 Chrome uTLS 落地前是唯一合法出口。

Console API key **不走**这套门。Token refresh（`/v1/oauth/token`）是身份流量，不是推理。本机导入 `~/.claude/.credentials.json` 仍可用。

参考插件 [dsh-plugin-oauth-subs](https://github.com/xxww0098/dsh-plugin-oauth-subs) **没有** Claude agent。它跑在 Node 里，TLS 就是 Node/OpenSSL。

## 版本钉扎

1. `AGW_CLAUDE_CODE_VERSION`（例如 `2.1.233`）
2. 本机 `claude --version` 解析出的最高 `x.y.z`
3. 文档钉 `2.1.233`（只在发版时改；进程内不升降）

同一进程 User-Agent 不抖动。换版本：改环境变量或升级 CLI 后**重启**网关。

## Cloak 会算什么（现在不算发出去）

| 层 | 内容 | 状态 |
| --- | --- | --- |
| HTTP | `User-Agent: claude-cli/<ver> (external, cli)`、`x-app: cli`、Stainless `js`/`node`/本机 OS+arch、稳定 session、每请求 `x-client-request-id`。不发 `X-Stainless-Helper-Method`。 | 已实现，只用于对照 |
| Body | `system[0]` = billing（无 `cache_control`）；缺 identifier 时插入 Claude Code 身份句；稳定 `metadata.user_id`。 | 已实现，只用于对照 |
| `cch` | 公开 JS 侧 SHA-256（[NTT123 gist](https://gist.github.com/NTT123/579183bdd7e028880d06c8befae73b99)：`"hey"` + `2.1.37` → `0d9` / `fa690`）。Bun xxHash64 无本机抓包前不实现。 | 对照用 |
| TLS | `gw-relay` = reqwest **rustls**。需要 Chrome ClientHello（CLIProxyAPI `HelloChrome_Auto`）。 | **未实现 → 拒绝发送** |

macOS Keychain 读凭据：本环境是 Linux，不做。

## 本机抓包对照

这台 runner **没有**安装 `claude`，因此**没有**对真 Claude Code 出站请求的抓包。开闸前必须在装了 Claude Code 的机器上做：

```bash
# 1. 钉版本（与本机 CLI 一致，只升不降）
export AGW_CLAUDE_CODE_VERSION=$(claude --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)

# 2. 从一次真 claude /v1/messages 抓包，写成 JSON（不要提交 token）
#    { "user_agent": "claude-cli/… (external, cli)",
#      "system0": "x-anthropic-billing-header: …",
#      "headers": { "x-app": "cli", … } }
#    放到 ~/.claude/agw-capture.json 或 AGW_CLAUDE_CAPTURE

# 3. 对照 cloak 不变量（不打 Anthropic）
REAL_API=1 cargo test -p gw-provider --lib -- --ignored --nocapture real_claude

# 4. 单测（公开 billing 向量 + fail-closed）
cargo test -p gw-provider --lib claude::fingerprint
```

对照项：UA 是否 `claude-cli/<同一主版本>`、`system[0]` 是否 billing、没有 `X-Stainless-Helper-Method`。**不要**在 rustls 上对照 JA3 然后宣称一致。

抓包与 cloak 一致、并且 `gw-relay` 有 Chrome ClientHello 之后，才能把 `chrome_tls_ready` 打开。在那之前把闸打开 = 半套伪装。
