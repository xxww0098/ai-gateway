# GLM

Zhipu GLM Coding Plan. Login is ZCode CLI poll (`provider: zai | bigmodel`), not PKCE. Default chat hop is Anthropic (`/api/anthropic`). Completions `paas/v4` is leftover.

Z.ai (global) and BigModel (China) are the two ZCode welcome-screen buttons. CLI init provider ids are `zai` and `bigmodel` (`zcode` 500s).

```text
POST zcode.z.ai/api/v1/oauth/cli/init   { provider: "zai" | "bigmodel" }
open authorize_url, poll /oauth/cli/poll/{flow_id}

Z.ai:      poll JWT → POST api.z.ai/api/auth/z/login → mint id.secret
BigModel:  poll JWT is the Coding Plan bearer (no biz mint)
```

Refresh is a no-op (`NEVER_EXPIRES_MS`). Start Plan (`zcode-plan`) is not supported: Desktop injects an Aliyun captcha header the gateway will not forge (upstream `400 code 3007`).

## Cache

Implicit prefix hash. Sticky: Anthropic `metadata.user_id` + `x-session-id` + first-block `cache_control: ephemeral`. Never send `prompt_cache_key` / Codex / Grok headers. Do not import another family's cache.

## Request

Anthropic default: missing `max_tokens` → 128000; GLM-5.3 / Flash force `thinking.type=enabled` + `clear_thinking: false` (Turbo is hybrid). Completions leftover: unknown roles (`developer`) → `system`; copy assistant `reasoning` onto `reasoning_content`.

## 稳定性

- A 576-token remnant after a leading-system splice is a prefix break, not a Grok affinity miss.
- UA must stay ZCode Desktop 3.10.1 (`ZCode/3.10.1 ai-sdk/anthropic/3.0.81`). 150% is **identity**, not protocol — switching to Anthropic is not what bills at 67%.
- CLI init/poll UA is `ZCode/3.10.1` only. Do not leak a plugin/gateway name.
- Never post CLI provider `zcode` (BigModel init 500). Region aliases `cn`/`zcode`/`china` still send `bigmodel`.
- Completions 400 `1214`: DSH `developer` is not a Zhipu role.
- Thinking prefix miss: 5.3 / Flash cannot `disabled`; keep `clear_thinking: false` and prior `reasoning_content`.
- Card title is never `zcode` / poll `user.id` / JWT `sub` / numeric uid. Empty userinfo → omit the heading.
- Do not hop `…/zcode-plan/anthropic` (Start Plan captcha 3007). Trial chat stays in ZCode.app.
