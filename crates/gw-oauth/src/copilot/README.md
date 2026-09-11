# GitHub Copilot

Device-code (no PKCE), VS Code GitHub App `Iv1.b507a08c87ecfe98`. Exchange `ghu_` → `tid=` before hopping to `api.githubcopilot.com`. Do not use OpenCode `Ov23li8` (issues `gho_`, exchange 404).

## Cache

Prefix hash + `X-Interaction-Id` (fallback `dsh-copilot` **is** written). Extra system at messages suffix. Claude on Copilot is still Completions.

## 稳定性

Never send `ghu_` as Bearer to `api.githubcopilot.com` — exchange `GET /copilot_internal/v2/token` first. Quota is `GET api.github.com/copilot_internal/user` with `Authorization: token <ghu_>`, not `tid=`. OpenCode `Ov23li8` mints `gho_` that 404 on exchange (preview `model_not_supported` / Business 403); this hop stays on the VS Code GitHub App (`Iv1.`, `ghu_`). Identity headers `GitHubCopilotChat/0.35.0` + `Editor-Version vscode/1.107.0` + `Copilot-Integration-Id vscode-chat` are required or Business/preview 403s. No PKCE, no GHES. Do not invent `X-Interaction-Type: agent-session-name-generation`. Copilot `pro` is Pro, not Codex Pro 20x.
