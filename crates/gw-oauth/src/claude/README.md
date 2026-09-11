# Claude

Existing AI-GateWay Claude Code OAuth (`client_id=9d1c250a-…`). Not ported from dsh-plugin-oauth-subs. PKCE S256, callback may paste `code#state`. Token is sent upstream as `x-api-key`, not Bearer.

## Cache

None in this crate. Claude executor in `gw-provider` owns the Messages hop.

## 稳定性

Do not drop this family when replacing Gemini CLI / xAI / Kiro agents — there is no oauth-subs replacement.
