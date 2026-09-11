# OAuth 故障

每条 ≤12 行：`## YYYY-MM-DD：标题`（最新在上），只要 **现象 / 根因 / 修复**。

## 2026-09-07：上游 OAuth 换成 oauth-subs 家族树

**现象**：面板只有 Gemini CLI / Claude / Codex / xAI / Kiro；Antigravity 与 Kimi 的 auth-url 是故意 404。Kiro 不进 `/v1` 矩阵，xAI 并进 openai 桶走 Completions，Codex hop 不是 chatgpt.com Responses。

**根因**：网关 OAuth agent 是旧的五家合写在 `gw-panel/upstream/oauth`，没有按家族隔离缓存，也没有 oauth-subs 已验证的 hop。

**修复**：新增 `gw-oauth`，一家一个目录；删 Gemini CLI OAuth；xAI 改存 `grok`；Antigravity / GLM / Cursor / Ollama / Kimi / Copilot 进家族树。Claude 订阅 OAuth 保留（oauth-subs 无 Claude 家族）。
