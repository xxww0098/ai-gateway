# 真实上游冒烟测试

默认 CI（`make test` / `cargo xtask ci`）**不跑**这些测试。缺凭据时不要用「读不到环境变量就 return」——那会让覆盖率变假。本档用 `#[ignore]` 把门，显式跑时缺凭据会 fail-loud。

## 怎么跑

```bash
# 1. 本机已有 CLI 登录，或先 export 令牌（不要写进仓库）
#    ~/.codex/auth.json
#    ~/.claude/.credentials.json
#    扫描根目录可用 AGW_LOCAL_OAUTH_HOME 指到一个假 $HOME

REAL_API=1 cargo test -p gw-provider --lib -- --ignored --nocapture real_
# 或：make test-real-api
```

`make test-ignored`（Postgres/Redis 那一档）会 `--skip real_`，避免工作区 `--ignored` 扫到后因没 `REAL_API=1` 红灯。

## 测什么

| 测试 | 请求 | 凭据 |
| --- | --- | --- |
| `real_codex_models_lists_when_local_oauth_exists` | `GET https://api.openai.com/v1/models` | Codex CLI `~/.codex/auth.json` |
| `real_claude_models_lists_when_local_oauth_exists` | `GET https://api.anthropic.com/v1/models`（带 fingerprint 头） | Claude Code `~/.claude/.credentials.json` |

Claude 这条在 `get_text` **之前**会跑 `assert_oauth_http_fingerprint`（UA / `x-app` / Stainless / oauth beta）。缺 cloak 头直接 panic，不会出网。

**没有** `POST /v1/messages` 真实测试：`gw-relay` 仍是 rustls，和 Claude Code 的 Chrome/Node ClientHello 对不齐，半套伪装会烧订阅号。见 `docs/claude-fingerprint.md`。

可选：`export AGW_CLAUDE_CODE_VERSION=2.1.233`（或本机 `claude --version`）后再跑，User-Agent 才和安装钉在一起。

401 = CLI 会话过期，重新 `codex login` / `claude` 后再跑。解析器单测在 `gw-provider` / `gw-panel` / `plugins/agw-oauth` 里，不碰网络。
