# Cursor

Cursor subscription. Login: PKCE poll `cursor.com/loginDeepControl` (no loopback), or `method=import` token JSON. Chat hop is Connect/protobuf `AgentService/Run` over HTTP/2.

## Login

| Method | What the operator sees |
|---|---|
| PKCE poll | Open `https://cursor.com/loginDeepControl?challenge=&uuid=&mode=login&redirectTarget=cli`. The panel polls `GET https://api2.cursor.sh/auth/poll?uuid=&verifier=`. 404 = still waiting; backoff 1s→10s. |
| Import | Paste `{ "accessToken", "refreshToken" }` (camelCase or snake_case). Not a second OAuth. |

Refresh: `POST https://api2.cursor.sh/auth/exchange_user_api_key`, `Authorization: Bearer <refresh>`, body `{}`. A known-bad refresh is backed off in-process so a stale CLI token cannot stall every snapshot.

## Cache

`conversation_id` = DSH pin plus model (`-fast` peeled). Historical turn ids are content hashes (`stable_id`), never `randomUUID()`. Hits: `TurnEndedUpdate.cache_read_tokens`.

## Fingerprint

`x-cursor-client-type: cli`. This hop is loginDeepControl OAuth, not `@cursor/sdk` API-key `Agent.create`. Do not send `x-parent-request-id`. `x-request-id` = `x-original-request-id`.

## HTTP/2

Cursor RPCs are **HTTP/2 only** (`application/connect+proto` for Run, `application/proto` for unary GetUsableModels / AvailableModels). An ALPN-stripping TLS proxy fails with `h2 is not supported`. Stay on HTTP/2; do not fall back to HTTP/1.1. Unary calls are one-shot; Run is a bidirectional Connect stream that answers KV get/set from the local blob store.

Protobuf is a hand-rolled subset (Run / GetUsableModels / AvailableModels / KV). Do not vendor a protobuf crate or the generated `agent_pb`.

## Do not

- Switch `x-cursor-client-type` to `sdk`
- Stamp turn / message / request ids with `Date.now()` or a fresh UUID
- Copy Codex `session-id` / Grok `x-grok-conv-id` onto this hop
- Invent `/v1beta` or a loopback callback for loginDeepControl

## 稳定性

HTTP/2 ALPN proxies fail with `h2 is not supported`. Stay `cli` client-type — this hop is loginDeepControl OAuth, not `@cursor/sdk` API keys.
