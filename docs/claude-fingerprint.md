# Claude Code fingerprint（OAuth / 订阅）

Claude Code 的 access token **只授权给 Claude Code 客户端**。网关用这类 token 打 Anthropic（`/v1/messages`、`/v1/models`、count_tokens）时，请求必须先走 `gw-provider` 的 `claude::fingerprint` cloak，不能只换 Bearer。

参考插件 [dsh-plugin-oauth-subs](https://github.com/xxww0098/dsh-plugin-oauth-subs) **没有** Claude / Anthropic agent（只有 Codex / Grok / GLM / Kiro / …）。它跑在 Node 里，TLS 就是 Node/OpenSSL，没有单独的 JA3 伪装层。本仓库的对应物是：OAuth 路径强制走 cloak；`gw-relay` 仍是 rustls。

Console API key **不走**这套伪装。

## 版本钉扎

优先级：

1. `AGW_CLAUDE_CODE_VERSION`（例如 `2.1.233`）
2. 本机 `claude --version` 解析出的最高 `x.y.z`
3. 文档钉 `2.1.233`（与本机常见安装对齐；**只在发版时改这一处**，进程内不升降）

同一进程里 User-Agent 不会上下抖动。换版本：改环境变量或升级 CLI 后重启网关。

## 会补的缺口

客户端没带才补，已带则原样转发（真 Claude Code 经网关时不改它的 `cch`）：

| 层 | 内容 |
| --- | --- |
| HTTP | `User-Agent: claude-cli/<ver> (external, cli)`、`x-app: cli`、`X-Stainless-{Lang,Runtime,OS,Arch}`、`X-Claude-Code-Session-Id`（进程内稳定）、`x-client-request-id`（每请求）。**不**发 `X-Stainless-Helper-Method`。 |
| Body | `system[0]` = `x-anthropic-billing-header: …`（无 `cache_control`）；缺 identifier 时插入 Claude Code 身份句；`metadata.user_id` 稳定。 |
| Prompt cache | 计费块之后的最后一个 system / 最后一个 tool 打 `ephemeral`（客户端已有 `cache_control` 则不再改）。 |

`cch` / `cc_version` 后缀用社区公开的 JS 侧 SHA-256 算法（[NTT123 gist](https://gist.github.com/NTT123/579183bdd7e028880d06c8befae73b99) 向量：`"hey"` + `2.1.37` → `0d9` / `fa690`）。较新的 Bun 原生路径用 xxHash64 签整份 body；**没有本机 `claude` 抓包之前不实现第二套**，避免半套算法。

## TLS（明确不做）

`gw-relay` 出网是 reqwest + **rustls**。Chrome/Node 的 ClientHello（uTLS / `HelloChrome_Auto`）要换 HTTP 栈，不是加几个头。本仓库不引入第二套推理客户端。

因此：

- **不**把 rustls JA3 假装成 Chrome。
- **不**加 `POST /v1/messages` 真实上游测试（那会在错误 TLS 画像上烧订阅号）。
- 真实冒烟只做 `GET /v1/models`，并且带上 cloak 头。见 `docs/real-api-tests.md`。

要对齐 JA3：在有本机 `claude` 抓包的前提下另开工作，给 Anthropic OAuth 单独做 Chrome uTLS 传输。未完成之前，订阅流量的 TLS 风险由运营方自己权衡。

## 本机抓包对照（有 Claude Code 时）

```bash
# 1. 钉版本
export AGW_CLAUDE_CODE_VERSION=$(claude --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)

# 2. 单测（含公开 billing 向量）
cargo test -p gw-provider --lib claude::fingerprint

# 3. 抓一份真 claude 的 /v1/messages，对照：
#    User-Agent 是否 claude-cli/<同一版本>
#    system[0] 是否 billing 且无 cache_control
#    不要对照 rustls ClientHello（已知不一致）
```
