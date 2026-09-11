# Kiro

AWS Kiro. Login: Social PKCE (`app.kiro.dev`), Builder ID / IdC device, Entra, or `ksk_` key. Chat hop is CodeWhisperer `GenerateAssistantResponse` eventstream, not OpenAI.

## Login

- `device` / `idc`: AWS SSO OIDC `client/register` then `device_authorization` (JSON camelCase). Builder ID uses `https://view.awsapps.com/start`.
- `authcode` / `social`: portal PKCE at `app.kiro.dev/signin`. Authorize `redirect_uri` is origin-only (`http://localhost:<port>`). Token exchange needs the landed path plus `login_option`.
- `import`: 卡密 / JSON / CSV / `ksk_`. JSON objects must carry `access_token` (or `ksk_`).

Refresh **must** rewrite `expiresAt` from the refresh JSON (`expiresIn` / `expiresAt`). Reusing the stored millisecond stamp makes TokenManager refresh every turn → `/refreshToken` 429.

## Cache

`conversationState.conversationId` = DSH pin **plus model**. Fallback `dsh-kiro:<model>`. Never `Date.now()`. System is a history user+ack pair (`I will follow these instructions.`); extras at history suffix, never between toolUses and toolResults. Tools stay on current `userInputMessageContext`. No Codex `session-id` / `prompt_cache_key` headers.

## 稳定性

`MONTHLY_REQUEST_COUNT` is a client error (400 `kiro_quota`), not a hammerable 429 — map it in the hop later. `INSUFFICIENT_MODEL_CAPACITY` → 503; `USER_REQUEST_RATE_EXCEEDED` → 429. 401/403 become 400 so the subscription key is not treated as AUTH.
