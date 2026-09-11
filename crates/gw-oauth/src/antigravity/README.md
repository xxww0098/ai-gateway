# Antigravity

Google Antigravity (daily-cloudcode-pa). Login is Google installed-app OAuth (no PKCE; `access_type=offline` `prompt=consent`), client from CLIProxyAPI. Chat hop is `generateContent` / `streamGenerateContent`. Missing `cloudaicompanionProject` cannot chat.

## Cache

`request.sessionId` = DSH pin as-is; fallback `dsh-antigravity:<model>`. Extra snapshots become a trailing **user** turn. Never `Date.now()`, never `implicitCacheConfig`.

## 稳定性

- Chat / loadCodeAssist User-Agent is only `antigravity/hub/<ver> <os>/<arch>`. Never send `Client-Metadata`, `x-goog-api-client`, or `anthropic-beta` on chat. onboardUser is the exception (longer UA + `x-goog-api-client`).
- Daily hub first. 5xx / transport may fall back to prod `cloudcode-pa`; 4xx (including 429) must not. onboardUser stays daily-only.
- `invalid_grant` / `invalid_client` / `unauthorized_client` drop the session. `VALIDATION_REQUIRED` does not — the operator verifies the Google account; do not treat it as a dead API key.
- `request.sessionId` is the DSH pin (fallback `dsh-antigravity:<model>`). Never `Date.now()`. Extra system snapshots trail as a **user** turn, never a GLM-style trailing system, and never between a `functionCall` group and its `functionResponse`.
- Hits: `cachedContentTokenCount` (and CLI aliases) → OpenAI `prompt_tokens_details.cached_tokens`. Do not emit `implicitCacheConfig`.
- Gemini 3 unsigned `functionCall` groups become a user observation; do not invent `thoughtSignature` / empty string / `skip_thought_signature_validator`. Claude / GPT-OSS still send unsigned calls, with allowlisted protobuf `parameters` (not OpenAI JSON Schema keywords).
