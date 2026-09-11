# Codex

ChatGPT Codex subscription. Official hop: openai/codex `rust-v0.153.4`.

**Not** `api.openai.com` pay-as-you-go. Chat hop is `chatgpt.com/backend-api/codex`.

## Files

| File | Role |
|---|---|
| [`mod.rs`](mod.rs) | Client id, endpoints, PKCE `start` |
| [`login.rs`](login.rs) | Form exchange, JSON refresh, session claims, hop headers |
| [`request.rs`](request.rs) | Responses body: lift system/developer into `instructions`, suffix-park extras, strip gpt-5.6-rejected fields |
| [`cache.rs`](cache.rs) | `prompt_cache_key` + `session-id` / `thread-id` / `x-client-request-id`. Do not import from another family |
| [`quota.rs`](quota.rs) | `wham/usage` + reset-credits JSON parse |
| [`import.rs`](import.rs) | `~/.codex/auth.json` then Hermes `openai-codex` |

## Login

PKCE S256, `auth.openai.com`. Exchange is form-encoded; refresh is JSON `{ client_id, grant_type, refresh_token }`. Authorize flags: `prompt=login`, `id_token_add_organizations`, `codex_cli_simplified_flow`, `originator`. `chatgpt_account_id` must come from id_token `https://api.openai.com/auth`. Permanent refresh codes: `refresh_token_expired` / `reused` / `invalidated` / `invalid_grant`.

## Chat

`POST chatgpt.com/backend-api/codex/responses`. Sticky cache: `session-id` = `thread-id` = `x-client-request-id` = `prompt_cache_key`. Drop DSH `session_id`. Fast: body `service_tier` `fast` → `priority` **and** `x-codex-routing-hint`. `store` must be `false`. Default `include` is `reasoning.encrypted_content`.

## Do not

Invent `x-codex-installation-id` / `parent-thread-id` / `x-codex-turn-metadata`. Do not copy these headers onto Grok/GLM/Kiro/Antigravity/Cursor/Ollama/Kimi/Copilot. Do not send `prompt_cache_retention`. Do not use Completions / Anthropic as the Codex hop.

## 稳定性

- chatgpt.com 400 `Unsupported parameter: session_id`: copy the pin onto `prompt_cache_key` **then delete** `session_id`. Headers stay `session-id` = `thread-id` = `x-client-request-id`.
- Long-session cache ~27% / shard miss: those three headers were dropped, and extra leading developer/system sat at the front of `input` and busted the prefix. Park extras at the **input suffix**. Same-request retries replay `x-codex-turn-state`. Compress / plan rebuild 0-hits are not shard misses. Healthy: weighted hit ≥ 80%, **zero** affinity misses.
- gpt-5.6 400 on `prompt_cache_retention` / `prompt_cache_options` / `safety_identifier` / `max_output_tokens` (Codex #39397): strip in `normalize_request`.
- Fast echoing `default`/`auto`: body `service_tier` alone is not enough; send `x-codex-routing-hint` (`model=<id>;tier=priority`) and `store: false`. Unqualified `-fast` (mini) must not be forwarded.
- Do not reuse this cache helper for Grok / GLM / Kiro / Antigravity (2026-08-31 mix-up). Grok ignores Codex `session-id`.
- Pro badge: `pro` → **Pro 20x**, `prolite` → **Pro 5x** (`crate::plan`). GLM `pro` stays Pro.
- Catalog: no `minimal` / `ultra` (CLI multi-agent, 400) / `gpt-5.3-codex` (ChatGPT account 400 “not supported when using Codex with a ChatGPT account”). Models GET needs `client_version`.
- Mid-stream disconnect: do not finish as HTTP 200 + clean EOF; that became `stream ended before a terminal response event` and blind retries.

See `docs/oauth.md` and `docs/oauth-error.md`.
