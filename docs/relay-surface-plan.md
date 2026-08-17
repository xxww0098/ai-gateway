# 三面入口收敛方案（audit-surface / 只读设计）

> 产出人：worker `audit-surface`。本轮**纯设计，未修改任何 `.rs`**。
> 第一性原理：**稳定的 request 转发 —— 透传优先，转义其次**。
> 硬约束不变：前端零改动、数据库兼容、计费语义不变。

---

## 0. 结论速览

| 项 | 结论 |
| --- | --- |
| 现存 `/v1` + `/v1beta` 路由（路径 × 方法） | **12 条**（9 个路径模式） |
| 可以删 | **6 条** |
| 必须保留 | **6 条**（3 个推理入口 + `GET /v1/models` + `GET /v1/models/{model}` + `POST /v1/messages/count_tokens`） |
| 「看起来该删但不能删」 | **1 组**：`GET /v1/models` / `GET /v1/models/{model}` —— 前端**一次都没 fetch 过**，但面板把 `${origin}/v1` 当 Base URL 印给用户，所有 OpenAI 兼容客户端拿到 base 后第一件事就是 `GET {base}/models` |
| 「看起来能删但会打脸」 | **1 组**：`/v1beta/**` —— 前端不调用，但 `QuickIntegrationPanel.tsx:80` 把 `${origin}/v1beta` 印在页面上。这是**「前端零改动」与「只保留三入口」的正面冲突**，见 §5.3，建议 410 过渡而非裸删 |
| provider 选择规则推荐 | **四级链：显式前缀 → `model_catalog_entries` 查表 → 配置化 `channel_key→provider` 映射 → 现有前缀猜测（降级为兜底并打点告警）**。数据源是面板已经在维护的表，前端零改动 |
| 一个必须先解决的外部阻塞 | `crates/gw-model/src/lib.rs` 已被误替换损坏（`o`/`m`/`g` → 字面量 ``[`compat`]``），**全工作区 `cargo check` 红**。已 escalate；`git checkout -- crates/gw-model/src/lib.rs` 即可还原 |

---

## 1. 证据基线（本方案的所有判定都建立在这四条上）

### 证据 A —— 前端对网关 `/v1`、`/v1beta` 的 HTTP 调用数：**0**

```
$ grep -rnE "(get|post|put|delete|fetch)\s*[<(].{0,40}['\`\"]/v1" frontend/src/
（无输出）
```

前端与后端之间**只有一条通路**：`frontend/src/shared/api/client.ts:56` 的
`createApiClient({ basePrefix: '/api/panel' })`，加上 `client.ts:101` 的
`sdkClient = createApiClient({ basePrefix: '/api/panel/admin/sdk-management' })`。

`frontend/src/` 里出现的 `/v1/models`、`/v1beta/models` 字面量全部是
**给上游 provider 拼探测 URL 用的**（`features/pricing/model_prices.ts:78-123`），
且这些 URL 不由浏览器直接发出 —— `model_prices.ts` 走
`apiCallRequest()`（`features/admin-proxy/api.ts:231`），它 `POST /api/panel/admin/sdk-management/api-call`，
由**后端**代为出网。所以它们连"前端发起的跨域请求"都不是。

> **结论**：删除任何一条 `/v1` / `/v1beta` 路由，**都不会让前端产生一个 404 的 fetch**。
> 「前端是否在调」这个判据，对全部 12 条路由的答案都是「否」。判定必须换一个更强的判据（见证据 B）。

### 证据 B —— 前端**印**给用户的 Base URL（这才是真正的约束）

`frontend/src/features/user-dashboard/components/QuickIntegrationPanel.tsx:16,78-80`：

```tsx
const origin = window.location.origin
...
{integrationTab === 'openai'
  ? `${origin}/v1`        // ← OpenAI 兼容 tab
  : `${origin}/v1beta`}   // ← Anthropic 原生 tab（!!）
```

三个 tab 是 `openai | anthropic | amp`（`QuickIntegrationPanel.tsx:5-9`），`amp` 走上面的分支，
所以这个三目表达式的 `else` 分支归 **anthropic**。也就是说：

- **OpenAI tab 印的是 `${origin}/v1`** —— 正确。任何 OpenAI 兼容客户端（Cursor / Cline / aider /
  OpenWebUI / LobeChat）拿到 base URL 后会自动 `GET {base}/models` 拉模型下拉框。
- **Anthropic tab 印的是 `${origin}/v1beta`** —— **这本身就是错的**。Anthropic 客户端
  （Claude Code、`@anthropic-ai/sdk`）需要的是 `${origin}`（SDK 自己拼 `/v1/messages`），
  `/v1beta` 是 Google 的版本段，把它给 Anthropic 客户端只会 404。
  紧接着的两个字段（`x-api-key`、`anthropic-version: 2023-06-01`，`QuickIntegrationPanel.tsx:89,97`）
  是纯正的 Anthropic 头，可见这行 `/v1beta` 是**前端既有缺陷**，不是有意设计。

`crates/gw-proxy/src/access.rs:47-51` 的 doc comment 明确把「面板已冻结、必须让它的说明成真」
写成了 `/v1beta` 存在的理由：

```
/// The previous SDK served this surface for free and it vanished with the SDK, but the
/// dashboard still hands it to tenants as their integration endpoint
/// (`QuickIntegrationPanel.tsx`, which is frozen), so a client that follows the
/// panel's instructions must land on a real route rather than a 404.
```

> **结论**：`/v1beta/**` 的唯一存在理由是「让一行已经写错了的前端文案不至于 404」。
> 这是 §5.3 要处理的冲突。

另有一条弱证据：`frontend/src/features/admin-proxy/components/OpenAiEditDialogBody.tsx:259`
的说明文字 —— 「（还须出现在网关 **GET /v1/models** 且未被计费拉黑）」。这是产品对
`GET /v1/models` 语义的书面承诺，虽然不是调用。

### 证据 C —— Go 参考实现里，这 12 条路由**一条都不存在**

`docs/go-routes.txt`（Go 侧完整路由 dump，149 行，工作区里被删了，用 `git show HEAD:docs/go-routes.txt` 取回）
里 **`/v1` 与 `/v1beta` 的匹配数为 0**。Go 注册的全部路由只有四类前缀：

```
/api/health, /api/health/ready
/api/panel/**            （145 条）
/api/payment/stripe/webhook
/metrics
```

这与 `crates/gw-proxy/src/lib.rs:34` 的自述完全吻合：

```
//! | [`routes`] | the SDK's own `/v1` handlers, which had no prior source in this repo |
```

以及 `crates/gw-proxy/src/routes.rs:4-6`：

```
//! `cmd/gateway/main.go` never wrote these routes; it inherited them from the
//! Builder, so the reference for *behaviour* is the billing pipeline they sat
//! inside, not a Go file.
```

> **结论**：整个 `/v1` 面来自先前 SDK 的 Builder，**不在本仓库的 Go 权威参照内**。
> 因此「与 Go 对齐」这条硬约束**只约束计费管线**（access → hold → dispatch → settle），
> **不约束路由表本身**。这 12 条路由的去留是一个纯粹的产品/工程决定，没有任何 parity 包袱。
>
> 补充：AGENTS.md §「查阅 Go 参考实现」教的
> `git log --oneline --all -- internal/` 在本仓库**返回空** —— `git log --all` 只有两个 commit
> （`712bbfb`、`1acdc49`），从未有过 `.go` 文件。Go 树不在这个 repo 的历史里，
> `docs/go-routes.txt` 是仅存的路由级 Go 证据。这一点值得回写进 AGENTS.md。

### 证据 D —— 当前**没有任何协议转义**，五个 executor 全是逐字节透传

| provider | 出网端点 | 对 body 的处理 |
| --- | --- | --- |
| `openai` | `chat_completions_endpoint()` → `{base}/v1/chat/completions`（`common.rs:280-302`） | 非流式**原样**；流式经 `ensure_include_usage`（`common.rs:248-268`） |
| `codex` | 同上（`codex.rs:190`） | 同上（`codex.rs:192-196`） |
| `claude` | `{base}/v1/messages`（`claude.rs:33,210-226`） | **原样** |
| `gemini` | `{base}/v1beta/models/{model}:{generateContent\|streamGenerateContent}`（`gemini.rs:105-133`） | **原样**；流式强制 `?alt=sse`（`gemini.rs:128-130`） |
| `vertex` | `{base}/v1/projects/{p}/locations/{l}/publishers/google/models/{model}:{action}`（`vertex.rs:489-509`） | **原样** |

全仓库唯一一处 body 改写是 `ensure_include_usage`（给流式请求补 `stream_options.include_usage=true`），
它**是计费的必要条件**：没有它，OpenAI 流式响应不带终局 `usage` 事件，
结算会落到 fallback（`AGENTS.md` §计费流程）。

> **结论**：转义层是**从零新建**，不是改造。`gw-relay` 的合同因此可以定得很干净：
> **默认零改写；`ensure_include_usage` 是唯一被授权的 body 变异，且必须显式声明。**

---

## 2. 路由去留判定表

`crates/gw-proxy/src/lib.rs:179-209` 注册的全部路由（axum 0.8 语法，`{model}` 是路径参数）：

| # | 方法 + 路径 | 注册处 | handler | 判定 | 依据 |
| --- | --- | --- | --- | --- | --- |
| 1 | `POST /v1/chat/completions` | `lib.rs:180` | `routes::chat_completions`（`routes.rs:674-678`） | **保留（入口 A）** | 三入口之一。测试覆盖最厚：`routes/tests/dispatch.rs` 里 20+ 个用例、`hold/tests.rs` 里 15+ 个用例都以它为载体 |
| 2 | `POST /v1/completions` | `lib.rs:181` | `routes::completions`（`routes.rs:679-683`） | **删除** | Legacy Completions（2023 年即 deprecated）。**零测试覆盖**、**零前端引用**、**零面板引用**；Go 侧不存在（证据 C）。全仓库只有两处 doc comment 提到它（`routes.rs:46,680`）。删了不会有任何测试变红 |
| 3 | `POST /v1/responses` | `lib.rs:182` | `routes::responses`（`routes.rs:684-688`） | **保留（入口 B）** | 三入口之一。⚠️ **今天是坏的**：它被 `ApiFamily::OpenAi` 派到 `openai`/`codex` executor，而两者都只会打 `{base}/v1/chat/completions`（`common.rs:296-302`、`codex.rs:190`）。Responses 形状的 body 送进 Chat Completions 端点，上游必 400。**零测试覆盖**，所以这个洞今天没人发现。见 §3.6 与 §4.6 |
| 4 | `POST /v1/embeddings` | `lib.rs:183` | `routes::embeddings`（`routes.rs:689-693`） | **删除** | 不是 LLM 推理入口。**零测试覆盖**（`idempotency/tests.rs:37` 只把 `"/v1/embeddings"` 当作**一个用于验证 scoped_key 不碰撞的任意字符串**，不是路由测试，改成任何别的字符串都一样过）、零前端、零面板。而且 `ApiFamily::Embeddings` 的 `default_provider()` 是 `openai`，但 openai executor 打的是 `/chat/completions` —— 与 #3 同一个洞，embeddings body 进 chat 端点，必 400。**今天就没在工作** |
| 5 | `POST /v1/messages` | `lib.rs:184` | `routes::messages`（`routes.rs:694-698`） | **保留（入口 C）** | 三入口之一。`routes/tests/gemini.rs:386` 用它验证「`x-api-key` 在 `/v1` 上不被剥离」（Anthropic 自己的头）。今天是**唯一一条真正端到端直通的路径**（`/v1/messages` → `claude` → `{base}/v1/messages`，逐字节） |
| 6 | `POST /v1/messages/count_tokens` | `lib.rs:185` | `routes::count_tokens`（`routes.rs:746-773`） | **必须保留，但必须降级修复** | 见 §2.1 |
| 7 | `GET /v1/models` | `lib.rs:186` | `routes::models`（`routes.rs:781-801`） | **必须保留** | 见 §2.2 |
| 8 | `GET /v1/models/{model}` | `lib.rs:187-190` | `routes::model_detail`（`routes.rs:804-824`） | **保留** | 见 §2.2 |
| 9 | `POST /v1/models/{model}` | `lib.rs:187-190`（同一条 `.route()` 的 `.post()`） | `routes::gemini_generate`（`routes.rs:715-730`） | **删除** | 这是 Google Generative Language API 的 **GA 版**原生入口（`/v1/models/{m}:generateContent`），即任务里说的「`/v1beta` 的 `/v1` 别名」。属于**被收敛掉的 Gemini 客户端入口**。零前端、零面板。测试引用仅 `hold/tests.rs:25` 一行 `is_billable(&POST, "/v1/models/gemini-2.5-pro:generateContent")` —— 那是断言 `is_billable` 的前缀语义，不是断言这条路由存在；删路由后该断言仍然成立（`is_proxy_path` 只看前缀），**不会变红**。⚠️ 注意它与 #8 共用路径模式，只靠方法区分：`POST /v1/models/gpt-4o` 今天会走 `ApiFamily::Gemini` → `provider_candidates("gpt-4o", Gemini)` = `["openai","gemini"]` → 把 Gemini 形状的 body 打进 OpenAI 的 `/chat/completions`。又一个死洞 |
| 10 | `GET /v1beta/models` | `lib.rs:195` | `routes::gemini_models`（`routes.rs:856-870`） | **删除（建议 410 过渡）** | Gemini 客户端入口，在收敛范围内。前端零调用（证据 A），但面板印了 `${origin}/v1beta`（证据 B）。测试会红：见 §5.1 |
| 11 | `GET /v1beta/models/{model}` | `lib.rs:196-199` | `routes::gemini_model_detail`（`routes.rs:877-891`） | **删除（建议 410 过渡）** | 同上 |
| 12 | `POST /v1beta/models/{model}` | `lib.rs:196-199`（同一条 `.route()` 的 `.post()`） | `routes::gemini_generate`（`routes.rs:715-730`） | **删除（建议 410 过渡）** | Gemini 原生推理入口（`{model}` 实际吃下 `gemini-2.5-pro:streamGenerateContent` 这种带冒号的整段，由 `split_model_action()`（`routes.rs:733-738`）拆开）。这是被收敛的第四个入口，其能力由「三入口 + 转义」承接（§3.6） |

**合计：删除 6 条（#2 #4 #9 #10 #11 #12），保留 6 条（#1 #3 #5 #6 #7 #8）。**

### 2.1 `POST /v1/messages/count_tokens` —— 为什么必须保留，以及必须同时修什么

**保留的理由**（判定为「保留」而非「删除」）：

1. 它**不是 LLM 推理入口**，而是 anthropic-messages 入口的附属端点。任务原文：
   「收敛的是 LLM 推理入口，不是把面板打死」—— 同理，也不是把入口的附属端点打死。
2. Claude Code 与 `@anthropic-ai/sdk` 会主动调它做上下文预算。面板的 Anthropic tab
   把 Anthropic 原生客户端列为一等公民（`QuickIntegrationPanel.tsx:7,20`），
   删掉 count_tokens 等于让这条产品承诺打折。
3. 前端零调用、面板零调用，所以**保留它不需要付任何前端代价**；删掉它却要付客户端代价。
   不对称，保留占优。

**但它今天有两个必须一并修掉的缺陷**（两个都不是 Go parity 包袱 —— 证据 C 已证明 Go 侧没有这条路由）：

| 缺陷 | 位置 | 事实 |
| --- | --- | --- |
| ① 返回的是**伪造**的 token 数 | `routes.rs:767` 调 `provider.count_tokens(...)`，而**五个 provider 的实现全是同一句** `Ok(approximate_tokens_from_bytes(req.payload.len()))`（`claude.rs:462`、`codex.rs:501`、`gemini.rs:317`、`openai.rs:243`、`vertex.rs:862`），`common.rs:133-138` 是 `size.div_ceil(4)` | 没有任何一个 provider 真的去调上游的 count_tokens 端点。客户端拿到的是「字节数 ÷ 4」，一个和真实 tokenizer 无关的数 |
| ② 伪造完还要收费 | `hold.rs:599-601` `is_billable == is_proxy_path`，`hold.rs:566-590` 的 doc 自己写明了：Anthropic 对 count_tokens 收费为 0，网关却按 `max(ActiveHoldAmount, Estimate(model, stream=true, rate_mult))` 收 —— 「carries a real model name, so it is priced at that model's rate and **can cost considerably more**」 | 用户为一个免费且返回假数据的调用付费 |

`hold.rs:592-599` 已经把修复缝留好了：

```rust
is_proxy_path(path)
    && !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
    && !path.ends_with("/count_tokens")
```

原注释说「Changing it is a product decision, not a porting one」。证据 C 已经把
「porting」这个理由消掉了 —— Go 里根本没有这条路由，不存在 A/B 对账时会被误判为移植 bug 的风险。
**建议本轮就采纳这个修复**，并把 `ClaudeProvider::count_tokens` 改为真实转发到
`{base}/v1/messages/count_tokens`；非 claude 的 provider 返回 `400`（见 §3.6 的处理原则）。

### 2.2 `GET /v1/models` 与 `GET /v1/models/{model}` —— 「看起来该删但不能删」的那一组

按证据 A 的字面标准（前端是否 fetch），它们是**零引用**，看起来最该删。**但必须保留**：

1. **它是 OpenAI 兼容协议的强制组成部分。** 面板把 `${origin}/v1` 作为 Base URL 交给用户
   （`QuickIntegrationPanel.tsx:79`），而所有 OpenAI 兼容客户端拿到 base 后的**第一个请求**
   就是 `GET {base}/models`：Cursor、Cline、aider、OpenWebUI、LobeChat 用它渲染模型下拉框，
   没有它整个客户端配置流程走不通。删掉 `/v1/models` = 面板印的那行 Base URL 作废 =
   违反「前端零改动」的**实质**（不是字面）。
2. **产品文案已经书面承诺。** `OpenAiEditDialogBody.tsx:259`：
   「（还须出现在网关 **GET /v1/models** 且未被计费拉黑）」。这行字在管理员配置上游模型的对话框里，
   直接把 `/v1/models` 写成了「模型对用户可见」的判定条件之一。
3. **成本为零。** 数据源 `SqlModelCatalog`（`adapters/catalog.rs:63-91`）读的是
   `model_catalog_entries`，与推理路径完全解耦；handler 只有 20 行。
   `GET /v1/models/{model}` 与它同源（`routes.rs:804-824`），对应 OpenAI SDK 的
   `client.models.retrieve()`，删它同样零收益。

**但同样要顺手修计费**：`hold.rs:566-585` 自陈一次目录读取按 fallback estimate 收费约
`$0.004`（`default_price_per_1k_tokens: 0.001` × `estimatedTokens = 1000`，见 `config.yaml`）。
一个纯 DB 读取按 LLM 价格收钱。用 §2.1 的同一个 `is_billable` 修复一并解决
（`GET` 直接排除）。

**注意与 `gw-panel` 的分工不要混淆**：用户「模型」页读的是
`GET /api/panel/user/models`（`gw-panel/src/billing/mod.rs:68` → `billing/prices.rs:492`），
它自己直接查 `model_catalog_entries`（`prices.rs:427`），**不经过 `/v1/models`**。
两条路都读同一张表，但是两个独立实现。删 `/v1/models` 不会影响面板页面 —— 这正是它
「看起来该删」的原因，也正是必须靠证据 B 而不是证据 A 来判定的原因。

一处低危不对称，顺手记下：面板的查询显式排除了哨兵行
（`prices.rs:427` `AND model_id <> '__models_url__'`，注释见 `prices.rs:429-431`），
而 `SqlModelCatalog::list_models`（`adapters/catalog.rs:73-80`）**没有**排除，
只靠哨兵行是以 `visible = false` 写入的（`gw-panel/src/ops/catalog.rs:116`）挡住。
只要有人通过 `POST /admin/model-catalog/openai-visibility`（`ops/catalog.rs:236`）
把该行翻成 visible，`__models_url__` 就会作为一个模型出现在 `GET /v1/models` 里。
建议在 `adapters/catalog.rs` 的 SQL 上补一句 `AND model_id <> $1` 对齐面板。

---

## 3. 三个保留入口的规范定义

### 3.0 公共部分（三入口一致）

| 项 | 规范 |
| --- | --- |
| 鉴权 | `Authorization: Bearer <token>`，**且仅此一种**。`token` 为 `agw-<hex>`（API Key）或 HS256 JWT（`access.rs:39,100-111,229-242`）。收敛后 `credential_from` 退化为纯 `bearer_token()`，`x-goog-api-key` / `x-api-key` / `?key=` 三种 carrier 随 `/v1beta` 一起下线（`access.rs:112-126`） |
| 请求 Content-Type | `application/json`（含 `charset` 后缀）。⚠️ 现状陷阱：`parse_body_peek`（`hold.rs:757-762`）在 Content-Type **不含 `json` 子串**时**直接放弃解析并返回全零 peek**，于是 `model=""`、`stream=false`、`max_tokens=0` —— 请求不会被拒，而是带着空模型名走完整个计费与派发链。`gw-relay` 应当把「非 JSON Content-Type」升级为 `400`，而不是静默降级 |
| 请求体大小上限 | `HOLD_REQUEST_BODY_LIMIT = 1 MiB`（`hold.rs:45`），超限 `413`（`hold.rs:844-863`）。body 被 peek 后原样放回并挂在 `PeekedBody` 扩展上（`hold.rs:82,192`），handler 复用（`routes.rs:622-630`），**全程只读一次** |
| 响应 Content-Type | **由上游决定，网关不改写**。非流式缺省补 `application/json`（`routes.rs:563-568`）；流式缺省补 `text/event-stream` + `Cache-Control: no-cache`（`routes.rs:451-462`） |
| 逐跳头 | 请求方向由 `is_skipped_proxy_header`（`gw-provider/src/types.rs:35-50`）过滤，响应方向由 `is_hop_by_hop`（`routes.rs:573-586`）过滤。两张表**不一致**（响应侧多 `content-length`，请求侧多 `host`），这是对的，但 `gw-relay` 应当把两张表放在同一个模块里相邻声明，避免各自漂移 |
| 计费 | 三入口全部计费。`access → hold → dispatch → settle` 顺序不可改（`lib.rs:161-177`），结算**恰好一次**（`routes.rs:357-368` 非流式 / `routes.rs:391-403` + `StreamSettler::finish`/`drop` 流式） |
| 失败语义 | 5xx 与 429 换账号重试，最多 `MAX_UPSTREAM_ATTEMPTS = 3` 个账号（`routes.rs:40,208`）；其余 4xx **立即上抛不烧号池**（`routes.rs:291-306`） |

### 3.1 入口 A —— `openai-completions`

| 项 | 值 |
| --- | --- |
| 路径 / 方法 | `POST /v1/chat/completions` |
| 请求 CT | `application/json` |
| 响应 CT | 非流式 `application/json`；流式 `text/event-stream`（SSE，`data: {...}\n\n`，终局 `data: [DONE]`） |
| 流式判定 | **请求体顶层 `stream: true`**，别无其他（详见 §3.4） |
| 网关必须解析 | `model`、`stream`、`max_tokens` / `max_completion_tokens`（`hold.rs:765-788`） |
| 网关必须改写 | 流式时 `stream_options.include_usage = true`（`common.rs:248-268`）——**唯一被授权的 body 变异**，缺了它就没有终局 usage，结算必落 fallback |
| 网关不该碰 | `messages`、`tools`、`tool_choice`、`response_format`、`temperature`、`top_p`、`seed`、`n`、`logprobs`、`user`、`metadata`、`reasoning_effort`，以及任何未来新增字段。**透传的定义就是：未列入「必须解析」清单的字段，网关既不读也不写，更不做 schema 校验** |

### 3.2 入口 B —— `openai-responses`

| 项 | 值 |
| --- | --- |
| 路径 / 方法 | `POST /v1/responses` |
| 请求 CT | `application/json` |
| 响应 CT | 非流式 `application/json`；流式 `text/event-stream`（**带 `event:` 行的具名事件流**，与 Chat Completions 的匿名 `data:` 流不同：`response.created` / `response.output_text.delta` / `response.completed` …） |
| 流式判定 | **请求体顶层 `stream: true`**，与入口 A **完全相同**（详见 §3.4） |
| 网关必须解析 | `model`、`stream`、`max_output_tokens` ⚠️ |
| 网关不该碰 | `input`、`instructions`、`tools`、`tool_choice`、`text`、`reasoning`、`previous_response_id`、`store`、`include` |

⚠️ **两个今天就存在的缺陷，本轮必须一并修**：

1. **`max_output_tokens` 没有被解析。** `parse_body_peek`（`hold.rs:765-775`）只认
   `max_tokens` 与 `max_completion_tokens`。Responses API 的输出上限字段叫
   `max_output_tokens`，于是每一个 `/v1/responses` 请求的 `max_tokens` 都是 0，
   `preflight_upper_bound`（`hold.rs:609-620`）退化为
   `max(hold_amount, Estimate(stream=true))` —— 预扣偏保守，不会漏计费，但会**过度冻结余额**。
   修法：在 `parse_body_peek` 的回落链里加一级 `max_tokens → max_completion_tokens → max_output_tokens → 0`。
2. **出网端点是错的。** `openai` 与 `codex` executor 都只会构造
   `{base}/v1/chat/completions`（`common.rs:296-302`、`codex.rs:190`），
   Responses 形状的 body 打进 Chat Completions 端点必 400。
   修法：`gw-provider` 需要一个 `responses_endpoint()`，按同样的
   「base 可能已含全路径 / 已含 `/v1` / 是裸 origin」三分支收敛到 `{base}/v1/responses`。
   这是**换端点，不是转义** —— 见 §3.6 的 B×openai / B×codex 两格。

### 3.3 入口 C —— `anthropic-messages`

| 项 | 值 |
| --- | --- |
| 路径 / 方法 | `POST /v1/messages` |
| 请求 CT | `application/json` |
| 请求必需头 | `anthropic-version: 2023-06-01`（客户端发，网关**原样透传**，不校验不补默认值 —— 校验是上游的职责，补默认值会掩盖客户端配置错误）。`claude.rs:34` 已有该常量，用于网关自己发起的 OAuth 刷新，不要与转发路径混用 |
| 响应 CT | 非流式 `application/json`；流式 `text/event-stream`（**带 `event:` 行**：`message_start` / `content_block_delta` / `message_delta` / `message_stop`） |
| 流式判定 | **请求体顶层 `stream: true`**，与入口 A、B **完全相同**（详见 §3.4） |
| 网关必须解析 | `model`、`stream`、`max_tokens`（Anthropic 侧 `max_tokens` 是**必填**字段，所以这一入口的预扣估算是三者里最准的） |
| 网关不该碰 | `messages`、`system`、`tools`、`tool_choice`、`stop_sequences`、`temperature`、`top_k`、`top_p`、`thinking`、`metadata` |
| 附属端点 | `POST /v1/messages/count_tokens`（见 §2.1）。它共享同一套鉴权，但**不计费**（修复后），也**不进 dispatch 重试链** —— `routes.rs:751-772` 是一条独立的、只挑一个账号、不 failover 的短路径 |

⚠️ 注意 `x-api-key`：在 `/v1beta` 上它是**租户凭据**、必须剥离（`access.rs:153-164`）；
在 `/v1` 上它是 **Anthropic 自己的上游头**、必须透传。`access.rs:158-160` 用
`if !path.starts_with(V1BETA_PATH_PREFIX) { return; }` 守住了这个区别，
`access/tests.rs:446` 与 `routes/tests/gemini.rs:386` 各钉了一半。
**收敛掉 `/v1beta` 后 `strip_consumed_credentials` 整个函数变成死代码**，
但这条「`/v1` 上的 `x-api-key` 属于上游」的语义必须保留在注释里，否则下一个人会重新加回剥离逻辑。

### 3.4 流式判定规则（三入口统一）

**现状（事实）**：

- 三个入口的判据**完全相同**，且**只有一个来源**：请求 body 顶层的布尔字段 `stream`。
  链路：`hold::parse_body_peek`（`hold.rs:770,787`）解析出 `BodyPeek.stream`
  → `routes::inbound`（`routes.rs:632-643`）读进 `Inbound.stream`
  → `endpoint!` 宏（`routes.rs:659-672`）传给 `dispatch()`
  → 决定调 `provider.execute_stream()` 还是 `provider.execute()`（`routes.rs:225,249`）。
- **`Accept` 头从头到尾没有参与判定。** 全仓库找不到一处读 `Accept` 做流式决策的代码。
  `Accept` 只在**出网方向**被 provider 设置（`codex.rs:203`、`gemini.rs`
  的 `default_content_negotiation`），从不在入网方向被读取。
- 唯一的例外是被删掉的 Gemini 入口：`routes.rs:722` `let stream = action.starts_with("stream")`
  —— 由 **URL 的 action 后缀**决定，body 完全不参与（因为 Gemini 的 body 里既没有
  `model` 也没有 `stream`，`routes.rs:703-709` 的注释详述了这个洞及其对预扣的影响）。
  这个特例随 `/v1beta` 一起消失，收敛后**判定规则全网只剩一条**。

**规范（`gw-relay` 应当固化的规则）**：

> **规则 S1：`stream` 只由请求体顶层的 `stream` 字段决定。字段缺失、非布尔、或 body 无法解析为 JSON 对象，一律视为 `false`。**
>
> **规则 S2：`Accept` 头永不参与流式判定。**
>
> **规则 S3：两者冲突时以 body 为准，且必须打一条 `warn!` 结构化日志**
> （字段：`path`、`body_stream`、`accept`、`request_id`），供运维定位客户端配置错误。
> 冲突指：`Accept: text/event-stream` 而 `stream != true`，或 `stream == true` 而
> `Accept: application/json`。

**为什么是 body 赢，而不是 Accept 赢** —— 这不是口味问题，是「透传优先」的直接推论：

body 是**要原样送到上游**的那个东西。上游（OpenAI / Anthropic）自己就是**只看 body 的 `stream`**
来决定响应形状的。如果网关按 `Accept` 判定为流式、却把 `stream: false` 的 body 原样送上去，
上游会返回一个**一次性 JSON**，而网关已经进了 `execute_stream()` 分支、
已经给客户端回了 `text/event-stream` 头 —— 网关将被迫**自己把 JSON 切成 SSE 帧**。
那就是凭空发明了一次转义，且是最坏的一种：它掩盖了客户端与上游之间的真实分歧。
反过来，若按 body 判定，网关的判断与上游的判断**永远一致**，转发始终是恒等变换。

同理，也**不要**在冲突时改写 body 去迎合 `Accept` —— 那是第二种 body 变异，
会让「唯一被授权的 body 变异是 `ensure_include_usage`」这条合同破防。

**这条规则对计费的连带影响**（必须在同一处写清楚，否则会被当成小事改掉）：
`stream` 同时决定预扣的上界 —— `preflight_upper_bound`（`hold.rs:609-620`）
里的 `calc.estimate(model, true, rate_mult)` 恒按流式估，
而 `estimate_with_max_tokens(model, max_tokens, stream, ...)` 吃 `stream` 参数。
所以流式判定错 = 预扣金额错 = 402 误判。

### 3.5 上游选择规则

#### 3.5.1 现状与它为什么会静默走错

`routes::provider_candidates`（`routes.rs:73-97`）：

```rust
let mut candidates: Vec<&'static str> = if lower.contains("codex") {
    vec!["codex", "openai"]
} else if lower.starts_with("claude-") {
    vec!["claude"]
} else if lower.starts_with("gemini-") {
    vec!["gemini", "vertex"]
} else if lower.starts_with("gpt-") || lower.starts_with("o1")
    || lower.starts_with("o3") || lower.starts_with("o4")
    || lower.starts_with("text-embedding") {
    vec!["openai"]
} else {
    Vec::new()
};
if let Some(default) = family.default_provider() && !candidates.contains(&default) {
    candidates.push(default);
}
```

配合 `ApiFamily::default_provider()`（`routes.rs:56-65`）：`OpenAi|Embeddings → "openai"`、
`Claude → "claude"`、`Gemini → "gemini"`。

**四个具体的失效场景**（都是静默的 —— 不报错，只是打到错误的上游然后拿一个上游 4xx）：

| 场景 | 今天的行为 | 后果 |
| --- | --- | --- |
| 新模型名不匹配任何前缀，例如 `claude-opus-5` 之外的新命名、`gpt-5.5`、`o5-mini` | `starts_with` 全部落空 → `candidates = []` → 只剩 `family.default_provider()` | 端点族默认碰巧对了就对，碰巧错了就错。`/v1/chat/completions` 上的任何新 Anthropic 模型名会被打到 `openai` |
| 名字里恰好含 `codex` 的非 Codex 模型（如某个 OpenAI 兼容上游的 `qwen-codex-7b`） | `lower.contains("codex")` → `["codex","openai"]` | 优先打 Codex OAuth 账号池，全部失败后才回落 openai。烧号池 + 拉长首字延迟 |
| 同名模型跨上游（`gemini-2.5-pro` 同时可从 gemini 与 vertex 出） | 硬编码 `["gemini","vertex"]`，顺序写死 | 管理员无法通过面板调整优先级；`channel_policies` 表里的 `priority`（`adapters/catalog.rs:31-37`）只在**同一个 provider 的账号之间**排序，管不到 provider 之间 |
| OpenAI 兼容第三方上游（DeepSeek / Qwen / Kimi / 自建 vLLM），模型名如 `deepseek-chat` | 全部落空 → `default_provider()` = `openai` | **碰巧是对的**，但纯属运气：它对是因为 `openai` executor 就是那个通配的 OpenAI 兼容 executor（`OpenAiCompatibleProvider`，`wiring.rs:436-446`）。任何一次前缀表的改动都可能把这个运气破坏掉 |

根因一句话：**模型名是一个由上游厂商随时变更的字符串，而路由决策却把它当成了一个稳定的枚举。**

#### 3.5.2 推荐方案：四级解析链，第一个命中即止

关键洞察：**「模型 → 渠道」的权威映射已经存在于数据库里，而且面板已经在维护它。**

`model_catalog_entries(channel_key, model_id, visible, models_url, ...)`
（`migrations/0001_init.sql:199-208`，`(channel_key, model_id)` 唯一索引见
`migrations/0003_indexes.sql:50`）。管理员在
「上游模型与用户模型页 → 从上游获取模型列表」（`OpenAiEditDialogBody.tsx:253-263`）
点一下，前端就调 `POST /api/panel/admin/model-catalog/ensure-openai-channel`
（`gw-panel/src/ops/catalog.rs:152-201`），把 `(channel_key, model_id)` 成批写进这张表。
`SqlModelCatalog::list_models`（`adapters/catalog.rs:65-90`）今天已经在读它，
只是把 `channel_key` 当成 OpenAI 的 `owned_by` 字段直接吐出去了 —— **一个现成的路由表被当成展示字段用了**。

**建议的解析链**：

| 级 | 规则 | 数据源 | 说明 |
| --- | --- | --- | --- |
| L1 | **显式渠道前缀**：模型名形如 `<channel_key>/<model_id>` 或匹配管理员配置的「模型前缀」 | 请求体的 `model` 字段本身 | 面板已有「模型前缀」输入框（`OpenAiEditDialogBody.tsx:244`）与 `GET/PUT /api/panel/admin/sdk-management/force-model-prefix`（`docs/go-routes.txt`）。产品里**已经存在**「用前缀限定渠道」的概念，直接接上，前端零改动 |
| L2 | **模型目录查表**：`SELECT channel_key FROM model_catalog_entries WHERE model_id = $1 AND model_id <> '__models_url__' ORDER BY channel_key` | Postgres，经缓存 | 权威表。返回多行时得到一个**有序候选列表**，天然支持「同一模型多渠道」，替代今天硬编码的 `["gemini","vertex"]` |
| L3 | **`channel_key → provider` 映射** | `gw-config` 新增可选段 | L2 得到的是 `channel_key`（管理员自填的自由文本），必须映射到 5 个 executor 名之一。见下 |
| L4 | **兜底 = 今天的 `provider_candidates()` 原样保留**，但每次命中都 `warn!` + 计数 | 硬编码前缀表 | **这是整个方案能安全上线的关键**：目录为空（全新部署、或管理员没点过「获取模型列表」）时，行为与今天**逐字节相同**。同时把「静默走错」变成一个可观测事件 |

**L3 的映射表**（写在 `config.yaml`，不是 DB —— 不需要迁移，不碰「数据库兼容」这条硬约束）：

```yaml
routing:
  # channel_key（面板里管理员自填）→ 上游 executor 名（gw-provider 的五选一）
  # 未列出的 channel_key 一律落 openai —— OpenAiCompatibleProvider 是那个通配的。
  channel_providers:
    openai:            openai
    openai_compatible: openai
    claude:            claude
    gemini:            gemini
    gemini-cli:        gemini
    aistudio:          gemini
    vertex:            vertex
    codex:             codex
  # 关掉 L2/L3 回到纯前缀猜测，用于出事时一键回滚
  catalog_routing_enabled: true
```

默认值来自前端已经写死的渠道词表 `providerToChannelMap`
（`frontend/src/features/pricing/model_prices.ts:153-162`：
`claude / gemini / vertex / codex / gemini-cli / aistudio / kimi / antigravity`），
所以默认配置**天生和面板的下拉框对齐**。

**四个必须写进实现的落地细节**（每一条都是踩过一次就回不来的坑）：

1. **L2 的查询绝对不能复用 `list_models()`。** `adapters/catalog.rs:76` 带
   `WHERE visible = TRUE`。`visible` 是「对租户展示」开关，不是「允许调用」开关 ——
   今天一个 `visible=false` 的模型照样能被调用（前缀猜测不看这张表）。
   若路由查询继承了 `visible = TRUE`，会**静默地把所有隐藏模型变成不可调用**，
   这是一个没人要求过的行为变更，而且表现为「某些模型突然 503」，极难归因。
   → 路由查询是**独立的一条 SQL**，`ModelCatalog` trait 需要新增
   `resolve_channels(model_id) -> Vec<String>`，不要在 `list_models` 上加参数。
2. **必须加缓存。** `list_models()` 今天是全表扫，而路由查询在**推理热路径**上。
   照抄仓库里现成的范式：`ChannelPolicyCache` + `channel_policy_refresh` 定时刷新 guard
   （`wiring.rs:292` 附近、`Guards` 结构体）。新增一个 `ModelRouteCache` 走同一套生命周期。
3. **候选列表要与 `MAX_UPSTREAM_ATTEMPTS = 3`（`routes.rs:40`）协同。** 今天候选最多 2 个 provider；
   L2 可能吐出更多。`dispatch()` 的 `while tried.len() < MAX_UPSTREAM_ATTEMPTS`（`routes.rs:208`）
   是**跨 provider 共享**的账号预算，不是 per-provider。候选变长不会放大重试次数，
   但会让「第 3 个 provider 永远轮不到」。需要显式决定：是给 provider 数也设上限，
   还是把预算改成 per-provider。**建议维持现状（共享预算）** —— 它保证了最坏情况延迟有界，
   这是比「一定试遍所有上游」更重要的性质。
4. **`ApiFamily` 的角色要降级。** 收敛后 `ApiFamily` 只剩 `OpenAi`（入口 A、B 共用）
   与 `Claude`（入口 C），`Embeddings` 与 `Gemini` 两个变体随路由一起删。
   `default_provider()` 从「第二级判据」降为「L4 兜底内部的最后一环」，
   语义从「这个端点默认走谁」变成「前缀表和目录都没话说时，按客户端说的方言猜一个」。

**为什么这套不破坏三条硬约束**：

- **前端零改动**：数据源是面板已有的表，写入路径是面板已有的按钮，映射表在 `config.yaml`。
  `frontend/` 一行不动。
- **数据库兼容**：只读 `model_catalog_entries`，不加列、不加表、不改索引。
- **计费语义不变**：`provider_candidates()` 的**返回类型和调用点都不变**
  （`routes.rs:186` 与 `routes.rs:751`），改的只是它内部怎么算出这个列表。
  hold / settle / release 的签名、时序、金额计算全部不受影响。

### 3.6 转义矩阵：3 个入口 × 5 个上游

图例：**直通** = 零 body 改写（`ensure_include_usage` 除外）；**转义** = 必须做协议翻译；
**400** = 直接拒绝，不勉强翻译。「今天」一列是**现状事实**，不是目标。

| | **openai**<br>`{base}/v1/chat/completions` | **codex**<br>`{base}/v1/chat/completions`（OAuth） | **claude**<br>`{base}/v1/messages` | **gemini**<br>`{base}/v1beta/models/{m}:generateContent` | **vertex**<br>`{base}/v1/projects/…/models/{m}:{action}` |
| --- | --- | --- | --- | --- | --- |
| **A. openai-completions**<br>`POST /v1/chat/completions` | ✅ **直通**<br><sub>今天：可用</sub> | ✅ **直通**<br><sub>今天：可用</sub> | 🔁 **转义（P2）**<br><sub>今天：直通 → 上游 400</sub> | 🔁 **转义（P1 必答）**<br><sub>今天：直通 → 上游 400</sub> | 🔁 **转义（P1 必答，与左格同一个转义器）**<br><sub>今天：直通 → 上游 400</sub> |
| **B. openai-responses**<br>`POST /v1/responses` | ✅ **直通**（需新增 `responses_endpoint()`）<br><sub>今天：**打错端点** → 400</sub> | ✅ **直通**（同左；Responses 本就是 Codex 的原生协议）<br><sub>今天：**打错端点** → 400</sub> | ⛔ **400**<br><sub>今天：直通 → 400</sub> | ⛔ **400**<br><sub>今天：直通 → 400</sub> | ⛔ **400**<br><sub>今天：直通 → 400</sub> |
| **C. anthropic-messages**<br>`POST /v1/messages` | 🔁 **转义（P2）**<br><sub>今天：直通 → 400</sub> | 🔁 **转义（P2）**<br><sub>今天：直通 → 400</sub> | ✅ **直通**<br><sub>今天：可用</sub> | 🔁 **转义（P1 必答）**<br><sub>今天：直通 → 400</sub> | 🔁 **转义（P1 必答，与左格同一个转义器）**<br><sub>今天：直通 → 400</sub> |

**统计：直通 5 格 / 转义 7 格（P1 四格 + P2 三格）/ 直接 400 三格 = 15 格。**

**分层与优先级**：

- **P0 · 直通 5 格**（A×openai、A×codex、B×openai、B×codex、C×claude）
  唯一的工作量是给 B 补一个 `responses_endpoint()`。`gw-relay` 的核心必须把这 5 格做成
  **恒等转发**：body 原样、query 原样、非逐跳头原样、响应原样。
  这 5 格的正确性是本项目的第一性原理，其余 10 格都可以后置。

- **P1 · 必答转义 4 格**（A/C × gemini/vertex）
  这是「gemini/vertex 全部保留，靠转义承接」这条已定决策的**全部内容**。
  gemini 与 vertex 的 wire 协议是**同一个** GenerateContent（只是 endpoint 前缀与鉴权不同，
  对比 `gemini.rs:105-133` 与 `vertex.rs:489-509`），所以是
  **2 个转义器（OpenAI↔Google、Anthropic↔Google）覆盖 4 格**，不是 4 个。
  每个转义器要做四件事，缺一不可：
  1. 请求：`messages[]` → `contents[]`（`role: assistant` → `model`），
     `system`/首条 system message → `systemInstruction`，
     `temperature`/`top_p`/`max_tokens` → `generationConfig`，
     `tools` → `functionDeclarations`。
  2. 响应（非流式）：`candidates[].content.parts[]` → OpenAI `choices[].message` /
     Anthropic `content[]`。
  3. 响应（流式）：Google 的 `?alt=sse`（`gemini.rs:128-130` 已强制）分帧 → 目标方言的 SSE。
     Anthropic 方向要额外合成 `message_start` / `content_block_start` / `message_stop`
     这些 Google 侧没有对应物的框架事件。
  4. **usage 必须原样落到 `UsageRecord`**：`usageMetadata.promptTokenCount` /
     `candidatesTokenCount` / `cachedContentTokenCount`。
     **转义可以丢字段，绝不能丢 usage** —— 丢了就落 fallback 结算，直接违反计费语义不变。

- **P2 · 可做但后置 3 格**（A×claude、C×openai、C×codex）
  OpenAI ↔ Anthropic 双向。真实需求存在（Claude Code 指向 OpenAI 上游、
  Cursor 指向 Claude 上游），但优先级低于 P1，因为这两家各自的原生入口都已经直通了。
  未做之前**返回 400 而不是转发**，`error.message` 明确写出
  「模型 X 属于渠道 Y，请改用 `POST /v1/messages`」这类可执行指引。

- **P3 · 直接 400 三格**（B × claude/gemini/vertex）
  拒绝而不是勉强翻译，理由是**语义会真丢**，不是嫌麻烦：
  Responses API 的核心是**有状态的 item 模型**（`previous_response_id` 服务端会话续接、
  `store: true` 服务端留存、`include` 按需回填、`reasoning` item 的加密留存）。
  Anthropic 与 Google 都没有对应概念。把它翻过去，只能翻掉 `input`/`output` 的文本部分，
  而客户端会以为 `previous_response_id` 生效了 —— **一个静默的、跨轮次的正确性错误，
  比一个 400 坏得多**。加上 Responses 客户端指向非 OpenAI 模型的实际需求接近于零。

**400 的统一形状**（三格 P3 + 未实现期的三格 P2 共用）：
HTTP `400`，body 用**入口自身方言**的错误信封（OpenAI 入口回 `{"error":{"message":…,"type":"invalid_request_error","code":"unsupported_upstream"}}`，
Anthropic 入口回 `{"type":"error","error":{"type":"invalid_request_error","message":…}}`），
并且**不计费**（走 `finish_error` 的 `UsageOutcome::failed()` 路径，`routes.rs:536-551`，
释放而非结算）。用入口方言而不是网关自有格式，是因为客户端 SDK 只会解析它自己那套错误结构；
回一个陌生结构，客户端会把它渲染成一个无字的红叉。

---

## 4. 连锁改动点清单（file:line）

### 4.1 路由注册

| 位置 | 现状 | 动作 |
| --- | --- | --- |
| `crates/gw-proxy/src/lib.rs:180-199` | 12 条路由 | 删 6 条，剩 `/v1/chat/completions`、`/v1/responses`、`/v1/messages`、`/v1/messages/count_tokens`、`GET /v1/models`、`GET /v1/models/{model}` |
| `crates/gw-proxy/src/lib.rs:187-190` | `/v1/models/{model}` 同时挂 `.get(model_detail).post(gemini_generate)` | 去掉 `.post(...)`，只留 `.get(model_detail)` |
| `crates/gw-proxy/src/lib.rs:195-199` | `/v1beta/models`、`/v1beta/models/{model}` | 整块删除；过渡期改挂 410 handler（§5.3） |
| `crates/gw-proxy/src/lib.rs:161-177` | `router()` 的 doc 详述 `/v1beta` 为何是兄弟前缀 | 重写。这段注释是本仓库对「两个前缀」的唯一书面解释，删路由不删注释会留下一份说谎的文档 |
| `crates/gw-proxy/src/lib.rs:4-15` | crate 级 doc：「Two client-facing prefixes」+ `:countTokens`/`:embedContent` 的缺口说明 | 重写为「三个客户端入口」 |

### 4.2 鉴权面（`crate::access`）

| 位置 | 现状 | 动作 |
| --- | --- | --- |
| `access.rs:51` `V1BETA_PATH_PREFIX` | `"/v1beta/"` | 删（过渡期保留给 410 handler 用） |
| `access.rs:61-63` `is_proxy_path` | `starts_with("/v1/") \|\| starts_with("/v1beta/")` | 收敛为 `path.starts_with(V1_PATH_PREFIX)`。**连带收益**：自动消除与 `gw-server/src/metrics.rs:321`（`let v1 = path.starts_with("/v1/")`）的口径分歧 —— 今天 `/v1beta` 流量被鉴权、被计费，却**不进 `agw_v1_requests_total`** |
| `access.rs:72,77,82` 三个 carrier 常量 | `x-goog-api-key` / `x-api-key` / `?key=` | 删。`API_KEY_HEADER`（`x-api-key`）删除时格外小心：`/v1` 上它是 **Anthropic 上游头**，只是碰巧同名 |
| `access.rs:100-126` `credential_from` | 四种 carrier + 固定优先级 | 退化为 `headers.get(AUTHORIZATION).and_then(bearer_token)` |
| `access.rs:133-138` `key_query_param` | — | 删 |
| `access.rs:153-164` `strip_consumed_credentials` | `/v1beta` 专用 | 删；**注释里「`/v1` 上的 `x-api-key` 属于 Anthropic」这句话必须搬到 §4.5 的 `inbound()` 附近保留** |
| `access.rs:177-188` `redact_query` | 掩码 `?key=` | 保留（纵深防御，代价为零），但注释要改成「历史上 `/v1beta` 用 `?key=` 传凭据；现已下线，此函数作为兜底保留」 |
| `access.rs:14-18, 44-51, 87-98` doc | 大段解释两个方言面 | 重写 |

### 4.3 计费面（`crate::hold`）

| 位置 | 现状 | 动作 |
| --- | --- | --- |
| `hold.rs:599-601` `is_billable` | `is_proxy_path(path)` | 采纳 `hold.rs:592-598` 已写好的修复：排除 `GET/HEAD/OPTIONS` 与 `/count_tokens`。**这是本轮唯一一处触碰计费的改动，必须单独一个 commit、单独说明** |
| `hold.rs:560-598` doc | 详述「两个零成本端点为何仍计费」并归因 Go parity | 重写：证据 C 已证明 Go 侧无此路由，parity 理由不成立 |
| `hold.rs:765-788` `parse_body_peek` 的 `Payload` | 只认 `max_tokens` / `max_completion_tokens` | 加 `max_output_tokens`（入口 B 的字段，§3.2） |
| `hold.rs:757-762` 非 JSON Content-Type 静默放弃 | 返回全零 peek，请求继续 | 改为 `400`（§3.0） |
| `hold.rs:166` `if !is_billable(...)` | 唯一调用点 | 无需改，但 `is_billable` 语义变化后要复核 `hold.rs:170-178` 的 fail-closed 分支：`GET /v1/models` 将**跳过 hold 层**，因此它的 `AccessMetadata` 不再被 hold 消费 —— 确认 `routes::models` 不依赖任何 hold 侧扩展（现状：不依赖，`routes.rs:781` 只吃 `State`） |

### 4.4 catalog / ModelCatalog

| 位置 | 动作 |
| --- | --- |
| `crates/gw-proxy/src/ports.rs:440-452` `ModelEntry` / `trait ModelCatalog` | 新增 `async fn resolve_channels(&self, model_id: &str) -> anyhow::Result<Vec<String>>`（§3.5.2 细则 1：**不要**复用 `list_models`） |
| `crates/gw-proxy/src/adapters/catalog.rs:63-91` `SqlModelCatalog` | 实现新方法；同时给 `list_models` 的 SQL 补 `AND model_id <> '__models_url__'` 对齐面板（§2.2 末） |
| `crates/gw-proxy/src/routes.rs:141-146` `with_catalog` | 保留。catalog 从「只喂 `/v1/models`」升级为「同时喂路由」，doc 要改 |
| `crates/gw-proxy/src/routes.rs:830-848` `GEMINI_GENERATION_METHODS` + `gemini_model_json` | **删**（只服务 `/v1beta` 目录） |
| `crates/gw-proxy/src/routes.rs:856-891` `gemini_models` / `gemini_model_detail` | **删** |
| `crates/gw-server/src/wiring.rs:292` `.with_catalog(Arc::new(SqlModelCatalog::new(pg.clone())))` | 保留；新增 `ModelRouteCache` 的构造与刷新 guard（照抄 `channel_policy_refresh` 的生命周期） |

### 4.5 dispatch / ApiFamily / provider 选择

| 位置 | 动作 |
| --- | --- |
| `routes.rs:44-54` `enum ApiFamily` | 删 `Embeddings`、`Gemini` 两个变体，只留 `OpenAi`、`Claude` |
| `routes.rs:56-65` `default_provider` | 保留但降级为 L4 兜底内部的最后一环（§3.5.2 细则 4） |
| `routes.rs:73-97` `provider_candidates` | **核心改动**：改为四级链。签名保持 `(model, family) -> Vec<&'static str>` 不变会拿不到 `&ProxyState`（要查缓存），所以签名需变为 `async fn(&Dispatcher, &str, ApiFamily) -> Vec<String>` 或加一个 `&dyn ModelRouter` 参数。**两个调用点**：`routes.rs:186`（`dispatch`）与 `routes.rs:751`（`count_tokens`），两处都要跟着改 |
| `routes.rs:659-672` `endpoint!` 宏 | 保留；实例从 5 个减到 3 个 |
| `routes.rs:679-683` `completions` / `689-693` `embeddings` | **删 handler** |
| `routes.rs:700-730` `gemini_generate` | **删**（含 `routes.rs:703-714` 那段说明 Gemini 预扣估算洞的长注释 —— 洞随路由一起消失） |
| `routes.rs:733-738` `split_model_action` | **删**（`pub`，需确认无外部使用者：全仓库仅 `routes.rs:720` 与 `routes/tests/dispatch.rs:34` 引用） |
| `routes.rs:606-645` `inbound()` | 保留；`access::strip_consumed_credentials` 的调用（`routes.rs:620`）随函数一起删 |
| `routes.rs:40` `MAX_UPSTREAM_ATTEMPTS` | 不改（§3.5.2 细则 3） |

### 4.6 gw-provider（转义层的落点）

| 位置 | 动作 |
| --- | --- |
| `gw-provider/src/common.rs:280-302` `chat_completions_endpoint` | 保留；**新增** `responses_endpoint()`，三分支收敛逻辑照抄 |
| `gw-provider/src/codex.rs:190` | 按入口方言在 `chat_completions_endpoint` / `responses_endpoint` 之间选择 |
| `gw-provider/src/openai.rs:103` | 同上 |
| `gw-provider/src/types.rs:156-186` `trait Provider` | 转义层需要知道**入口方言**才能决定翻不翻。`ProviderRequest`（`types.rs`）需要携带一个 `ApiFamily`/`dialect` 字段，或由 `gw-relay` 在装配时选择转义 wrapper。⚠️ `types.rs` 与 `lib.rs` 是**协调者独占**（CONTRACT §3），本轮不动，需 `ask` |
| `gw-provider/src/types.rs:179-185` `count_tokens` 默认实现 | 默认 `Ok(0)`；`claude.rs:462` 应改为真实转发 `{base}/v1/messages/count_tokens`，其余四家返回 `ProviderError`（§2.1） |

### 4.7 gw-server / gw-config

| 位置 | 动作 |
| --- | --- |
| `gw-server/src/wiring.rs:337` `gw_proxy::router(proxy_state).merge(gw_panel::router(panel_state))` | 装配点不变 |
| `gw-server/src/wiring.rs:414-461` `build_providers` | **不改** —— 五个上游全部保留是已定决策 |
| `gw-server/src/wiring.rs:389-411` `sdk_seed_config` | 不改（六个 provider 的 seed 文档与客户端入口无关） |
| `gw-server/src/metrics.rs:318-325` `track` | `path.starts_with("/v1/")` 与新的 `is_proxy_path` 自动一致，无需改。但 410 过渡期的 `/v1beta` 请求会**不被计数**，这是对的（它不是代理流量） |
| `gw-server/src/lib.rs:17,100-118` doc | 「`/v1/*` 代理面」的措辞要跟着改 |
| `gw-config/src/lib.rs:163` `Config` / `258` `SdkConfig` / `312` `SdkProviderConfig` | 新增 `routing` 段（§3.5.2）。⚠️ `gw-config` 属 worker `platform`（CONTRACT §3），需协调 |
| `config.yaml` / `config.example.yaml` | 新增 `routing:` 段。两份要同步改，否则 `gw-config` 的测试会发现漂移 |

### 4.8 受影响的测试文件清单

| 文件 | 影响 |
| --- | --- |
| `crates/gw-proxy/src/routes/tests/gemini.rs`（17 个用例） | 15 个删除，**2 个（`:359` `:379`）必须迁到 `dispatch.rs` 保命**。见 §5.1 |
| `crates/gw-proxy/src/routes/tests.rs:18` | `mod gemini;` 需删；`use` 列表里 `signed_get` 可能变成未使用 |
| `crates/gw-proxy/src/routes/tests/dispatch.rs:10-44` | 三个 `provider_candidates` 单测 + 一个 `split_model_action` 单测需重写。见 §5.1 |
| `crates/gw-proxy/src/routes/tests/dispatch.rs:306-346` | `listing_models_is_billed_the_fallback_estimate_exactly_as_go_bills_it` —— **断言反转**（改计费后不再计费） |
| `crates/gw-proxy/src/routes/tests/dispatch.rs:348-368` | `counting_tokens_is_billed_exactly_as_go_bills_it` —— **断言反转** |
| `crates/gw-proxy/src/access/tests.rs:71-113` | `both_dialect_prefixes_are_on_the_metered_surface`、`the_gate_matches_what_the_billing_layer_reserves_for`、`the_panel_and_health_surfaces_keep_their_own_auth` —— 含 `/v1beta` 断言，需重写 |
| `crates/gw-proxy/src/access/tests.rs:375-451` | `the_gemini_surface_reads_the_carriers_google_sdks_actually_use`、`the_carrier_priority_is_fixed_...`、`a_blank_carrier_is_no_credential_at_all`、`a_consumed_credential_is_removed_from_what_gets_relayed` —— **全部删除**（被测函数消失）。⚠️ `access/tests.rs:446` 那半个 case（`/v1` 上 `x-api-key` **不**被剥离）测的是**必须保留的语义**，要换个载体重写 |
| `crates/gw-proxy/src/hold/tests.rs:22-44` | `is_billable` 的两组断言（`hold/tests.rs:25` 含 `/v1/models/gemini-2.5-pro:generateContent`；`:33-41` `the_two_zero_cost_endpoints_are_billed_because_go_bills_them`）—— 后者**整个反转** |
| `crates/gw-proxy/src/idempotency/tests.rs:37` | `"/v1/embeddings"` 只是一个用于验证 scoped_key 不碰撞的任意字符串，**不会红**；但为免误导，建议换成一个仍然存在的路径 |
| `crates/gw-proxy/src/testsupport/harness.rs:84` / `upstream.rs:197-205` | `FakeCatalog` 需要跟着实现新的 `resolve_channels` |
| `crates/gw-proxy/src/adapters/catalog/tests.rs:49` | `the_catalogue_hides_invisible_models_and_deduplicates_by_id` —— 加哨兵行排除后需补一条断言；新增 `resolve_channels` 需要自己的 `#[ignore]` 集成测试（需真 PG） |
| `crates/gw-config/src/tests.rs` | 新增 `routing` 段的解析与默认值测试 |
| `crates/gw-server/src/tests.rs:44` | 注释提到 `/v1/*`，措辞跟改 |

---

## 5. 迁移风险

### 5.1 删除后会变红的测试（按删除项归因）

**A. 删 `/v1beta/**`（#10 #11 #12）—— 代价最大的一项**

`crates/gw-proxy/src/routes/tests/gemini.rs` 共 **17 个用例，其中 15 个**直接死掉：

| 用例 | 行 |
| --- | --- |
| `the_gemini_dialect_answers_on_the_prefix_the_dashboard_advertises` | `:26` |
| `a_gemini_call_reserves_and_settles_on_the_same_pipeline_as_v1` | `:46` |
| `the_stream_action_streams_although_the_body_never_asked_for_it` | `:66` |
| `a_gemini_stream_settles_once_from_the_usage_it_carried` | `:91` |
| `a_gemini_stream_without_usage_falls_back_rather_than_billing_zero` | `:105` |
| `an_anonymous_gemini_call_is_turned_away_before_it_can_reserve` | `:117` |
| `the_gemini_catalogue_needs_credentials_too` | `:133` |
| `the_gemini_catalogue_answers_in_googles_envelope_not_openais` | `:147` |
| `one_gemini_catalogue_entry_is_addressable_by_its_bare_id` | `:183` |
| `the_gemini_catalogue_is_billed_exactly_as_the_openai_one_is` | `:201` |
| `a_gemini_client_authenticates_the_way_google_sdks_actually_do` | `:230` |
| `the_key_query_parameter_authenticates_and_is_billed_like_any_other` | `:252` |
| `a_bad_credential_is_rejected_whichever_carrier_it_arrives_on` | `:269` |
| `the_authorization_header_outranks_the_carriers_google_uses` | `:303` |
| `a_consumed_credential_is_never_relayed_to_google` | `:322` |

**另外 2 个必须保留并搬家**（它们测的是 `/v1` 面，只是住在 `gemini.rs` 里）：
`the_v1_surface_keeps_reading_authorization_and_nothing_else`（`:359`）与
`an_anthropic_key_header_still_reaches_the_claude_upstream_on_v1`（`:379`）。
后者钉的正是 §3.3 那条「`/v1` 上的 `x-api-key` 是 Anthropic 上游头、必须透传」的语义 ——
**这是删 `/v1beta` 时最容易被顺手带走的一条不变量**，删文件前先把它迁到 `dispatch.rs`。

加 `access/tests.rs` 的 4 个整体删除（`:375` `:395` `:416` `:424`）与 2 个含 `/v1beta`
断言需改写的（`:71` `:85`）。**合计 21 个用例受影响：15 删 + 4 删 + 2 改写。**

这些用例大多在测**通用性质**（流式结算恰好一次、匿名请求不计费、凭据不外泄），
只是碰巧以 Gemini 面为载体。**不要连同性质一起删** —— 逐条判断：
「这条测的是 Gemini 面，还是测的是一条恰好跑在 Gemini 面上的通用不变量？」
后者要迁移到入口 A 或 C 上重写。

**B. 删 `POST /v1/models/{model}`（#9）**

无用例变红。`hold/tests.rs:25` 的
`assert!(is_billable(&Method::POST, "/v1/models/gemini-2.5-pro:generateContent"))`
断言的是 `is_billable` 的前缀语义（路径以 `/v1/` 开头），与该路由是否注册无关，**仍然成立**。

**C. 删 `/v1/completions`（#2）与 `/v1/embeddings`（#4）**

**零用例变红。** 这两条今天完全没有路由级测试。`idempotency/tests.rs:37` 用
`"/v1/embeddings"` 只是要一个「与 `/v1/messages` 不同的路径字符串」，
换成 `"/v1/responses"` 效果完全一样。

**D. 改 `is_billable`（§2.1 / §2.2 的计费修复）**

三个用例**断言反转**（不是删，是改成相反的期望）：
`routes/tests/dispatch.rs:306`、`routes/tests/dispatch.rs:348`、`hold/tests.rs:33`。
另外 `routes/tests/gemini.rs:201` 也是同类，但它会先随 A 一起消失。

**E. `provider_candidates` 换实现**

`routes/tests/dispatch.rs:10` `a_recognisable_model_outranks_the_endpoint_it_arrived_on`、
`:20` `a_gemini_model_falls_back_to_vertex_because_both_serve_it`、
`:28` `an_unrecognised_model_still_reaches_the_endpoint_default`
—— 三条都在测前缀猜测的具体行为，需要重写为「四级链」的性质测试：
L2 命中时不看前缀、L2 落空时回落到与今天**逐字节相同**的结果、
L4 命中时打点计数增加。

按 `AGENTS.md`「测试不许复述源码里的字面量」：新测试**不要**把
`channel_providers` 的默认映射抄进断言，测的应该是
**「配置里写了什么，路由就走什么」**这条性质。

### 5.2 frontend 会 404 的调用

**没有。** 证据 A 已证明前端对 `/v1`、`/v1beta` 的 HTTP 调用数为 0。
删除任何一条路由都不会产生一个失败的前端 fetch。

**但有两处前端会变得「说谎」**：

| 位置 | 内容 | 删除后 |
| --- | --- | --- |
| `QuickIntegrationPanel.tsx:80` | Anthropic tab 印 `${origin}/v1beta` | 用户照此配置 → 404（**注意：这行今天就是错的**，`/v1beta` 对 Anthropic 客户端本来就不通，删除只是把「错但有个路由在」变成「错且路由也没了」） |
| `OpenAiEditDialogBody.tsx:259` | 「还须出现在网关 GET /v1/models」 | **不受影响** —— `GET /v1/models` 判定为保留 |

### 5.3 过渡期：删除 vs 410 Gone —— 建议

**这是本方案里唯一一处「已定决策」与「硬约束」正面冲突的地方，必须显式处理。**

- 已定决策：客户端入口只保留三个，Gemini 客户端入口在收敛范围内。
- 硬约束：前端零改动 —— 而前端印着 `${origin}/v1beta`。
- 两者不可同时完全满足。

**建议：分路由采取不同策略。**

| 路由 | 策略 | 理由 |
| --- | --- | --- |
| `POST /v1/completions`、`POST /v1/embeddings`、`POST /v1/models/{model}` | **硬删** | 零前端引用、零面板引用、零测试、Go 侧不存在，且**今天就是坏的**（§2 表 #3 #4 #9 —— 三者都会把 body 打到错误的上游端点）。删掉一个本来就返回上游 400 的路由，风险为负 |
| `GET /v1beta/models`、`GET /v1beta/models/{model}`、`POST /v1beta/models/{model}` | **410 Gone，保留两个发布周期，然后硬删** | 见下 |

**为什么 `/v1beta` 值得一个 410 而不是裸 404**：

1. 面板印着这个地址（证据 B），且**前端冻结，我们改不了那行字**。
   404 对用户是「网关坏了」；410 + 一条可读的 message 是「这个面下线了，请改用 X」——
   在我们无法修正前端文案的前提下，**410 的 body 是唯一能触达用户的说明渠道**。
2. 410 的语义精确：`Gone` 表示「资源曾经存在，已被有意移除，且不会回来」，
   正是这里的情况。404 会让客户端/中间件误以为是路径拼错或临时故障而重试。

**410 handler 的三个实现要点**（都不是可选的）：

1. **必须注册在中间件栈之外。** 告诉调用者「这个面没了」不需要凭据，
   更不该计费。做法：`is_proxy_path` 先收敛为只认 `/v1/`，
   然后在 `gw_proxy::router()` 的 `.layer(...)` **之后**（即外层）单独 `.route("/v1beta/{*rest}", any(gone))`。
   如果挂在 layer 内层，`access::layer` 会先给它一个 401，用户永远看不到那句说明。
2. **body 用 Google 的错误信封**，因为它面对的是 Gemini 客户端：
   ```json
   {"error":{"code":410,"status":"NOT_FOUND","message":
     "本网关的 /v1beta Gemini 原生入口已下线。Gemini 与 Vertex 模型仍可用，请改用 OpenAI 兼容入口 POST /v1/chat/completions 或 Anthropic 入口 POST /v1/messages，Base URL 使用 <origin>/v1。"}}
   ```
3. **打一个专门的 metric**（`agw_v1beta_gone_total`）。两个发布周期后看这个计数：
   若持续为 0，硬删；若不为 0，说明真有用户在用，此时再回来重新讨论，
   而不是靠猜。**这是把「要不要删」从一次赌博变成一次测量的唯一办法。**

**同时必须记账的一条前端 TODO**（前端解冻后的第一件事）：
`QuickIntegrationPanel.tsx:78-80` 的三目表达式应改为 Anthropic tab 印 `${origin}`
（而非 `${origin}/v1beta`）—— 这行今天就是错的，与本次收敛无关，但收敛让它从
「错但碰巧有路由」变成「错且暴露」。

### 5.4 其余风险

| 风险 | 说明 | 缓解 |
| --- | --- | --- |
| **`is_billable` 改动触碰计费红线** | `AGENTS.md` / `CONTRACT.md` 都把「计费语义不变」列为硬约束 | 严格说，改的是**计费范围**（哪些路径计费），不是**计费语义**（hold/settle/release 三段式、partial-debit、strict 模式）。三段式一行不动。但仍建议：单独 commit、单独说明、由协调者过一遍。**这是全方案唯一需要显式批准的改动** |
| **`provider_candidates` 引入 DB 查询到热路径** | 每个推理请求多一次查表 | 必须走缓存（§3.5.2 细则 2）。缓存未命中时**回落 L4 而不是等 DB**，保证 DB 抖动不会变成推理延迟 |
| **候选列表变长撞上 `MAX_UPSTREAM_ATTEMPTS`** | 见 §3.5.2 细则 3 | 维持共享预算，最坏延迟有界 |
| **转义层丢 usage** | 直接违反计费语义 | P1 转义器的验收标准里，`usageMetadata` → `UsageRecord` 的映射必须有独立测试，且必须覆盖流式（Google 的 usage 只在最后一帧） |
| **`gw-provider/src/types.rs` 是协调者独占** | 转义层需要在 `ProviderRequest` 上加入口方言字段 | 实现阶段先 `ask`，不要自行修改（CONTRACT §3） |
| **工作区当前编译不过** | `crates/gw-model/src/lib.rs` 被误替换损坏，全工作区 `cargo check` 红 | 已 escalate。`git checkout -- crates/gw-model/src/lib.rs` 还原。**本文档的「哪些测试会红」一节因此是静态读取测试源码枚举的，未能用 `cargo test --list` 实测对账** |

---

## 6. 需要拍板的三件事

1. **`is_billable` 的计费范围修复**（§2.1 / §2.2）—— 触碰硬约束边缘，建议协调者/用户显式批准。
   不批准的话，`GET /v1/models` 与 `count_tokens` 继续按 LLM 价格收费，方案其余部分不受影响。
2. **`/v1beta` 走 410 过渡还是直接硬删**（§5.3）—— 涉及「前端零改动」与「三入口收敛」的取舍。
   本方案推荐 410 + 两个发布周期 + metric 观测。
3. **P2 三格（OpenAI ↔ Anthropic 双向转义）做不做、什么时候做**（§3.6）——
   不做则这三格返回 400，不影响 P0/P1 的正确性。

---

*本文档由 worker `audit-surface` 产出，只写 `docs/relay-surface-plan.md`，未修改任何 `.rs`、`Cargo.toml` 或 `frontend/`。*
