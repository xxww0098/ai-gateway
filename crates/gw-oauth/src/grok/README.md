# Grok

xAI Grok subscription. Default login is RFC 8628 device-code against `auth.x.ai` (OIDC discovery host must be `x.ai` / `*.x.ai`). PKCE is the loopback fallback; the panel supplies its own `redirect_uri`. Chat hop is grok-build Responses (`api.x.ai/v1/responses`).

## Login

| Item | Value |
|---|---|
| `client_id` | Grok CLI public client (see `CLIENT_ID`) |
| discovery | `https://auth.x.ai/.well-known/openid-configuration` |
| UA | `grok-cli/0.2.93` |
| scope | `openid profile email offline_access grok-cli:access api:access` |

`start` → device (default) or `method=pkce`. Exchange is form `authorization_code`. Refresh is form `refresh_token`; `invalid_grant` is permanent. Device poll is one RFC 8628 step (`authorization_pending` / `slow_down` / tokens).

## Cache

Sticky: `x-grok-conv-id` = `x-grok-session-id` = body `prompt_cache_key`. Per-request `x-grok-req-id` + `x-grok-model-override`. Never copy Codex `session-id` / `x-client-request-id`. Do not lift extra system into top-level `instructions`; park extras at the **input suffix**. Never write `service_tier` (Grok Fast is a no-op; leftover `-fast` is peeled locally).

## Quota

Two sources, both required:

1. CLI `GET /v1/billing?format=credits` + `/v1/user?include=subscription`
2. grok.com `GetGrokCreditsConfig` gRPC-web (weekly pool when JSON omits `creditUsagePercent`)

Prepaid `0` and product shells with no numbers are not rows. No Codex-style reset-credit redeem.

## 稳定性

- **Affinity miss, not prefix break** (2026-09-04): a later **512-token** cache block with reuse **< 10%** is the wrong xAI shard. Sticky set is grok-build whole (`x-grok-conv-id` / `x-grok-session-id` / `x-grok-req-id` / `x-grok-model-override`). Healthy long session: weighted hit ≥ 80%, **zero** affinity misses. Compression / plan rebuild with 0 hits is not a shard miss.
- **Do not mix Codex headers** (2026-08-31): never emit `session-id` / `x-client-request-id` on this hop. xAI ignores them and cache lands on the wrong shard.
- **No Grok Fast** (2026-08-30): xAI accepts `priority` but throughput does not change. Never send `service_tier`. Peel a stale `grok-*-fast` id so it cannot 400 as a fake model.
- **Weekly pool is the gRPC frame** (2026-08-30): unified-billing SuperGrok / X Premium+ often omit `creditUsagePercent` and show prepaid 0 + empty "Grok Code". JSON percent wins when present; otherwise merge `GetGrokCreditsConfig`. Hide prepaid 0 and numberless product rows.
