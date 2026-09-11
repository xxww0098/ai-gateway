# Kimi

Kimi Code Plan. Device-code (no PKCE), `client_id=17e5f671-d194-4dfb-9706-5516cb48c098`. Import `~/.kimi-code/credentials/kimi-code.json`. Chat: `api.kimi.com/coding/v1/chat/completions`.

## Cache

Prefix hash. Strip Codex/Grok fields. Extra system at messages suffix. `dsh-kimi` is analyzer-only.

## 稳定性

Do not invent a fourth DSH `api` value (`openai-completions` only — `kimi-openai-completions` drops the whole settings blob). Device `expired_token` restarts `device_authorization`, it is not a refresh. Device body is `client_id` only (no `scope`, no PKCE). Effort belongs in `thinking.effort`, not `reasoning_effort` on the wire. Do not impersonate Pi on `User-Agent` / `X-Msh-Platform`. No Codex `session-id` / Grok `x-grok-conv-id`.
