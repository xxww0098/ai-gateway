# OAuth 参考仓库

跨家族 hop 的参考仓索引。改某家登录 / 对话 / 缓存先对照这里钉住的官方 CLI 或社区逆向，再改 `crates/gw-oauth/src/<id>/`。

| 文件 | 职责 |
|---|---|
| 本文件 | 官方 / 社区仓库、钉住的版本、抄什么、不发明什么 |
| [`crates/gw-oauth/src/<id>/README.md`](../crates/gw-oauth/src/codex/README.md) | 那一家的设计源（端点、函数、wire 字段）与稳定性 |
| [`docs/oauth-error.md`](oauth-error.md) | 故障与验收 |

缓存按家族隔离。对照仓 A 的头不能写到家族 B。

## 总表

| 家族 | 一线对照 | 本 hop 钉住 | 设计源 |
|---|---|---|---|
| Codex | [openai/codex](https://github.com/openai/codex) `rust-v0.153.4` | UA `codex_cli_rs/0.153.4` | [`codex/README.md`](../crates/gw-oauth/src/codex/README.md) |
| Grok | [xai-org/grok-build](https://github.com/xai-org/grok-build) | UA `grok-cli/0.2.93` | [`grok/README.md`](../crates/gw-oauth/src/grok/README.md) |
| GLM | ZCode Desktop 3.10.1 + [docs.z.ai](https://docs.z.ai/devpack/quick-start) | UA `ZCode/3.10.1 ai-sdk/anthropic/3.0.81` | [`glm/README.md`](../crates/gw-oauth/src/glm/README.md) |
| Kiro | [kiro.dev/docs/models](https://kiro.dev/docs/models) | eventstream `GenerateAssistantResponse` | [`kiro/README.md`](../crates/gw-oauth/src/kiro/README.md) |
| Antigravity | Antigravity.app hub 2.11.0 | UA `antigravity/hub/2.11.0`；daily-cloudcode-pa | [`antigravity/README.md`](../crates/gw-oauth/src/antigravity/README.md) |
| Cursor | Cursor CLI `loginDeepControl` | 指纹 `cli-2026.07.23-e383d2b` | [`cursor/README.md`](../crates/gw-oauth/src/cursor/README.md) |
| Ollama Cloud | [docs.ollama.com/cloud](https://docs.ollama.com/cloud) | Bearer → `ollama.com/v1` | [`ollama/README.md`](../crates/gw-oauth/src/ollama/README.md) |
| Kimi | 官方 Kimi Code CLI | 设备码、无 PKCE | [`kimi/README.md`](../crates/gw-oauth/src/kimi/README.md) |
| Copilot | VS Code GitHub Copilot App `Iv1.b507a08c87ecfe98` | UA `GitHubCopilotChat/0.35.0`；`ghu_`→`tid=` | [`copilot/README.md`](../crates/gw-oauth/src/copilot/README.md) |
| Claude | 既有网关 Claude Code OAuth | `client_id=9d1c250a-…`；token 走 `x-api-key` | [`claude/README.md`](../crates/gw-oauth/src/claude/README.md) |

Gemini CLI OAuth（`gemini-cli-auth-url`）已删除。Gemini 只走 API key；Google 订阅编码助手走 Antigravity。xAI 存储 id 是 `grok`，不再写 `xai`。
