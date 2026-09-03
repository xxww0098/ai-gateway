# 本机 CLI OAuth 导入

网关和 `plugins/agw-oauth` **优先读本机已有凭据**，不把浏览器登录当主路径。

## 会读哪些文件

| 路径 | Provider | 来源 |
| --- | --- | --- |
| `~/.codex/auth.json` | `codex` | Codex CLI |
| `~/.claude/.credentials.json` | `claude` | Claude Code（Linux / 非 Keychain） |
| `$CLAUDE_CONFIG_DIR/.credentials.json` | `claude` | Claude Code 覆盖目录 |
| `~/.grok/auth.json`、`$GROK_HOME/auth.json` | `xai` | Grok CLI |
| `~/.hermes/auth.json` | `codex` / `xai` | Hermes 多账号店 |
| `~/.kiro/credentials.json` | `kiro` | Kiro IDE |
| `~/.aws/sso/cache/kiro-auth-token.json` | `kiro` | Kiro SSO cache |

扫描根目录：`AGW_LOCAL_OAUTH_HOME` → `HOME` → `USERPROFILE`。**不接受客户端传入的任意路径。**

面板上传同一形状的 JSON 也能识别（不必先改 `provider` 字段）。

## 怎么导入

1. **网关进程所在机器已有 CLI 登录**（管理员 JWT 或管理员 `agw-` key）：

   ```bash
   curl -X POST -H "Authorization: Bearer <admin>" \
     http://127.0.0.1:8888/api/panel/admin/sdk-management/auth-files/import-local
   ```

   已存过的 access/refresh token 会跳过，不会复制一行。

2. **把 CLI 文件拖进面板**「凭证 / auth-files」上传。Codex / Claude Code 原文件现在能直接识别。

3. **Harness 插件**（本机有 CLI 文件）：`/agw import`，或设置页 **导入本机 CLI 凭据**。已登录且 key 是管理员时，会 POST 到上面的 `/auth-files`。

## 对比 [dsh-plugin-oauth-subs](https://github.com/xxww0098/dsh-plugin-oauth-subs)

那是 Harness 里直连各订阅上游的 loopback 代理。本仓库是 LLM 中转网关：凭证进 `auth_records`，推理只从 `gw-relay` 出网。

| 参考插件能力 | 本仓库 | 说明 |
| --- | --- | --- |
| Codex 读 `~/.codex/auth.json` | 已做 | 导入 + 既有 PKCE 刷新 |
| Claude Code `~/.claude/.credentials.json` | 已做 | 参考插件没有这一家；OAuth token 走 Bearer + `oauth` beta |
| Grok / Hermes 本地文件 | 已做 | 既有 device-code 仍在 |
| Kiro 本地 / SSO cache | 已做 | 既有 import / device / auth-code |
| GLM / Z.ai / BigModel | **不做** | 没有 `gw-provider` 执行器，前端契约冻结 |
| Antigravity | **不做** | 面板已明确 404，无后端 |
| Cursor 导入 / Keychain | **不做** | 无 Cursor 上游；不读 Keychain |
| Ollama Cloud / Kimi / OpenCode | **不做** | 无对应 planner；Ollama Cloud 是 API key，不是 OAuth 文件 |
| DSH 设置页 / 配额条 / loopback `:8318` | **不做** | 与网关架构无关的 UI |

## 缓存命中（顺带）

参考插件把加权缓存命中从 ~27% 拉到 ~95%，靠的是**账号亲和 + Anthropic prompt-cache 头**。本仓库对应改动：

- Claude：缺省补 `anthropic-beta`（OAuth 带 `oauth-…` + `prompt-caching-…`；API key 只带 prompt-caching）。客户端已发该头则不覆盖。
- Claude：请求体没有 `cache_control` 时，给最后一个 system 块和最后一个 tool 打 `ephemeral` 断点。已有断点则整份 body 不改写。
- Claude OAuth 凭证改为 `Authorization: Bearer`（不再误放 `x-api-key`）。
- Claude OAuth 的 Messages / count_tokens **必须**走 `claude::fingerprint`（头 + billing system[0]）。细则与 TLS 不做清单：`docs/claude-fingerprint.md`。
- 渠道亲和：`(user, model)` 的 model 与价目表同一套 `normalize_model_key`，避免 `GPT-4o` / `gpt-4o` 拆成两个粘性槽、换账号打爆前缀缓存。

预期：同一租户、同一模型、稳定 system/tools 前缀的多轮对话，Anthropic 侧应能看到 `cache_read_input_tokens`；大小写不同的模型名不再换号。没有真实流量基线，不报百分比。
