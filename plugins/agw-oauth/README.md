# AGW-Oauth

DeepSeek Harness plugin: OAuth into **AI-GateWay** and use its `/v1` models without writing a model config file.

## Install

From this repository checkout:

```bash
dsh plugin --profile web add ./plugins/agw-oauth
```

From GitHub (allow the package `prepare` script so `src/` builds to `lib/`):

```bash
dsh plugin --profile web add github:xxww0098/ai-gateway
```

The plugin path inside the repo is `plugins/agw-oauth` (`dsh-agw-oauth`).

## Environment

| Variable | Meaning |
| --- | --- |
| `AGW_ORIGIN` | AI-GateWay origin, e.g. `https://gw.example.com`. Required before the first login. |

After OAuth the origin and API key are stored under `$DSH_HOME/agw-oauth.json` (default `~/.dsh/agw-oauth.json`).

## Login

1. Set `AGW_ORIGIN` and start Harness (`dsh web` or the CLI).
2. Run `/agw login`. The command returns as soon as a verification URL and user code exist.
3. Open the URL, sign into the existing AI-GateWay panel, and approve.
4. The plugin polls, receives an `agw-` API key plus the gateway origin, then `GET {origin}/v1/models`.
5. Models appear on the `ai-gateway` provider route. `resolveModel()` exposes catalog thinking efforts, `text`/`image` modalities, and input/output token limits — only what the catalog listed.

`/agw status` and `/agw logout` are also available.

## Gateway routes

The plugin talks to:

- `POST /api/panel/oauth/dsh/device/code`
- `POST /api/panel/oauth/dsh/device/token`
- `GET /oauth/dsh` (browser consent; existing panel login)
- `POST /api/panel/oauth/dsh/device/approve` (authenticated)
- `GET {origin}/v1/models` and `POST {origin}/v1/chat/completions`

## 中文

见 [README.zh.md](./README.zh.md)。

## Optional HTTP API

When the web profile is running, the plugin also exposes:

- `POST /agw-oauth/login/start` — same as `/agw login`; returns URL and user code immediately
- `GET /agw-oauth/status` — `{ loggedIn, origin, watch }`
