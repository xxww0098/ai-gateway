# wave 3 / proxy-integration —— 交付报告

> 产出人：worker `proxy-integration`。独占 `crates/gw-proxy/**`，未碰任何其他 crate 的源码。
> 验收：`cargo check --workspace --all-targets` 零错误；`cargo clippy -p gw-proxy --all-targets -- -D warnings`
> 我的文件零警告；`cargo test -p gw-proxy` **218 passed / 0 failed / 19 ignored**；
> `cargo xtask ci` **9 条门禁全绿**。

---

## 0. 速览

| 项 | 结果 |
| --- | --- |
| 路由收敛 | 删 6 留 6，**硬删**，无 410 过渡 |
| 计费范围修复 | `GET /v1/models`、`GET /v1/models/{model}`、`POST /v1/messages/count_tokens` 移出计费范围 |
| Hold/Settle/Release | **签名与语义一字未动** |
| gw-relay 接线 | 优先级 1、2、3 全部完成；优先级 4（`RelayEngine`）按指示留给 wave 4 |
| 三条必带发现 | 全部落地（其中 L1 张力在 gw-proxy 侧解决） |
| 前端 | **一行未改** |

---

## 1. 路由收敛（`lib.rs`）

**删除 6 条**（`crates/gw-proxy/src/lib.rs`）：

| 路由 | 处理 |
| --- | --- |
| `POST /v1/completions` | 删路由 + 删 handler |
| `POST /v1/embeddings` | 删路由 + 删 handler |
| `POST /v1/models/{model}`（Gemini GA 别名） | 去掉 `.post(...)`，只留 `.get(model_detail)` |
| `GET /v1beta/models` | 删路由 + 删 handler |
| `GET /v1beta/models/{model}` | 删路由 + 删 handler |
| `POST /v1beta/models/{model}` | 删路由 + 删 handler |

连带删除：`ApiFamily` 枚举、`default_provider()`、`provider_candidates()`、`gemini_generate`、
`split_model_action`、`gemini_models`、`gemini_model_detail`、`gemini_model_json`、
`GEMINI_GENERATION_METHODS`、`access::V1BETA_PATH_PREFIX`、三个凭证载体常量、
`key_query_param`、`strip_consumed_credentials`、`redact_query`。

`access::is_proxy_path` 收敛为 `path.starts_with("/v1/")`。
**连带收益**：与 `gw-server/src/metrics.rs` 的 `path.starts_with("/v1/")` 口径自动一致
（历史上 `/v1beta` 流量被鉴权、被计费，却不进 `cpa_v1_requests_total`）。

### 关于 `redact_query` —— 我删了，与方案 §4.2 的「保留」建议不同

方案建议保留它做纵深防御。我删了：它的**全部存在理由**（`?key=` 是 Gemini 面的凭证载体）
和 `/v1beta` 在同一个变更里消失，零调用方，且 `gw-server` 没有装 `TraceLayer`
（已 grep 确认，query string 不会被自动记进日志）。留一个理由已经消失的安全函数，
下一个人只会以为「查询串仍然是凭证材料」。**如果协调者认为该留，一句话就能加回来。**

### 已知缺口（已在代码注释里写明，`lib.rs` crate doc + `endpoint.rs`）

面板 `QuickIntegrationPanel.tsx:80` 仍把 `${origin}/v1beta` 印给用户，照着配的用户会 404。
补充事实（也写进注释了）：**那行文案今天本来就是错的** —— `/v1beta` 是 Google 的版本段，
给 Anthropic 客户端本来就 404，`@anthropic-ai/sdk` 需要的 base 是裸 `${origin}`。
收敛只是把「错但碰巧有个路由在」变成「错且路由也没了」。前端冻结，已知且已接受。

`GET /v1/models` 保留的理由写在 `routes::models` 的 doc 上，用的是**面板印的 Base URL**
这条论据（不是「前端在调」—— 前端对 `/v1` 的 HTTP 调用数是 0）。

---

## 2. 计费范围修复（`hold.rs`）

采纳 `hold.rs:592` 早就写好的修复分支：

```rust
pub fn is_billable(method: &Method, path: &str) -> bool {
    is_proxy_path(path)
        && !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
        && !path.ends_with("/count_tokens")
}
```

**Hold / Settle / Release 的签名与语义一行未动。** 改的是「哪些路径进入计费管线」，
不是「计费怎么算」。doc 里把旧的「Go parity」理由替换成证据 C 的证伪。

### 一个我在验证时发现的事实，必须说清楚

修完之后**有两道独立的门**挡住这三条路径的计费：

1. `is_billable` 排除了 GET/HEAD/OPTIONS 与 `/count_tokens`；
2. `hold::handle` 新增的 `gw_relay::endpoint::validate` 对非三入口路径直接放行给 axum。

我做规范 2.11 验证时（把 `is_billable` 改回纯前缀）发现：
**两条端到端用例 `listing_models_costs_the_tenant_nothing` /
`counting_tokens_costs_the_tenant_nothing` 仍然通过**，因为第二道门接住了。
真正钉住 `is_billable` 这条谓词的是三条单元用例
（`the_zero_cost_endpoints_are_out_of_billing_scope`、
`a_safe_method_is_never_billable_whatever_the_path`、
`the_zero_cost_endpoints_are_authenticated_but_not_billed`），它们**确认会红**。
这是纵深防御，不是缺陷 —— 但端到端那两条的保护力比名字看起来弱，如实记下。

---

## 3. gw-relay 接线

### 3.1 ✅ `upstream::select` 四级链取代前缀猜测

`routes.rs:73-97` 的 `provider_candidates()` 整个删除，换成
`routes/routing.rs::select_upstreams` → `gw_relay::endpoint::upstream::select`。

**`resolver = None` 时与今天逐字节等价**，由用例
`without_a_resolver_the_chain_is_byte_for_byte_the_old_prefix_guess` 对拍钉住
（参照是 gw-relay 自己保留的 `prefix_guess`，不是抄进断言的字面量）。

**`ChannelResolver` 的真实实现已提供**：`adapters::CatalogChannelResolver`。

- **L2 数据源**：`ModelCatalog` 新增 `resolve_channels(model_id)` 与 `model_routes()`
  两个方法（带默认空实现，所以任何现有实现不改也能编译）。
  `SqlModelCatalog` 的实现是**独立的一条 SQL**，**没有** `WHERE visible = TRUE`
  —— `visible` 是展示开关不是调用开关，继承它会静默地把隐藏模型变成不可调用。
  `#[ignore]` 集成用例 `routing_sees_models_the_catalogue_listing_hides` 钉住这条。
- **L3 映射表**：默认词表取自前端已经写死的渠道下拉框
  （`model_prices.ts` 的 `providerToChannelMap`），所以默认配置天生与面板对齐，
  前端零改动。可用 `with_channel()` 补充。
  **没有新增 `config.yaml` 的 `routing:` 段** —— `gw-config` 属 `platform` worker，
  我没碰。词表在代码里是有意的：它只是一份默认值，不是配置面。
- **缓存**：`ArcSwap` 快照 + `refresh()` + `spawn_refresh(interval)`，与
  `ChannelPolicyCache` 同一种生命周期。**从未刷新过的快照是空的** ——
  此时 L2 全落空、直接落 L4，与今天逐字节相同。这是安全灰度的默认态。

**⚠️ 它没有被装上。** `Dispatcher::with_channel_resolver()` 是显式 opt-in，
而装配点在 `gw-server/src/wiring.rs`（`platform` 的文件，我不能改）。
所以**当前运行时行为 = 纯 L4 兜底 = 与收敛前完全相同**。要打开需要 `platform` 加两行：

```rust
let routes = Arc::new(gw_proxy::adapters::CatalogChannelResolver::new(catalog.clone()));
let _ = routes.refresh().await;                       // 或挂到既有的刷新 ticker 上
Arc::clone(&routes).spawn_refresh(Duration::from_secs(60));
// Dispatcher::…​.with_channel_resolver(routes)
```

同理，`Dispatcher::refresh_auths()` 也是 `pub`，可以挂到同一个 ticker 上。

### 3.2 ✅ `matrix::route` 取代隐式派发

`routes/routing.rs::partition_routable` 按 15 格表把候选切成「能转发的」与「必须 400 的」。
`Route::Reject`（P3 三格）与 `Route::Translate`（转义器尚未接线的 7 格）
都回 **HTTP 400 + 入口自身方言的错误信封**，并且**不计费**（走释放路径）。

**为什么把 `Translate` 也归到 400**：这 7 格今天的行为是「原样转发 → 上游必 400」
（方案 §3.6「今天」一列，7 个转义格全是这样），所以换成网关自己的 400
**不破坏任何今天能工作的流量**，而客户端能从错误里读到「该改用哪个入口」。

**一条我在写测试时才发现、值得记下的性质**：
**不装 resolver 时矩阵的 400 分支根本到不了。** L4 兜底总会把入口的默认 provider
追加到候选末尾，而入口默认永远落在直通格上，于是候选里必有一个能转发的。
所以矩阵的 400 只会在**目录或显式前缀明确说了这个模型属于哪个渠道**时触发。
这是灰度保证的一部分，由用例 `the_prefix_only_chain_always_keeps_a_passthrough_escape_hatch` 钉住。

**⚠️ 缺陷 #1（S1）只根除了一半。** `POST /v1/responses` 打到 openai / codex 时，
`matrix::upstream_dialect` 已经正确判定为 `UpstreamDialect::OpenAiResponses`，
**但 `gw-provider` 的 executor 仍然只会构造 `{base}/v1/chat/completions`**，
而 `ProviderRequest` 上没有承载入口方言的字段。所以**入口 B 打到 OpenAI 系上游时仍然是坏的**。
需要的补丁在别人的文件里（`gw-provider` 的 `responses_endpoint()` + `types.rs` 加方言字段，
方案 §4.6），或者由 wave 4 的 `RelayEngine` 整体取代转发段。已写进 `routes.rs` 的模块 doc。

### 3.3 ✅ `spec` 统一流式判定 + 消掉 body 三次解析（缺陷 #15）

- `hold::handle` 用 `gw_relay::endpoint::validate` 做路径/方法/content-type 三件套校验；
- `RequestSpec::parse` 是**全链路唯一一次** JSON 解析，结果作为请求扩展下发；
- `routes::inbound` **直接复用**这个扩展（扩展缺席只有一种成因：这条路径不计费、
  hold 层整个被跳过，此时在 `inbound` 解析一次，仍然是每请求恰好一次）；
- `parse_body_peek` / `BodyPeek` 删除，换成 `BillingPeek::from_spec`。

**顺带修好的两件事**：

1. **`max_output_tokens` 现在被解析了**（入口 B 的输出上限字段）。收敛前
   `parse_body_peek` 只认 `max_tokens` / `max_completion_tokens`，于是**每一个**
   `/v1/responses` 请求的 `max_tokens` 都是 0，预扣退化成保守估算、过度冻结余额。
2. **非 JSON content-type 从静默降级升级为 400**。收敛前它返回全零 peek，
   请求带着 `model=""` 走完整个计费与派发链。
   **这是一个用户可见的行为变更**：`curl -d`（默认 `x-www-form-urlencoded`）
   现在拿 400 而不是被放行。方案 §3.0 明确要求这样，且这类请求此前也只会拿一个上游 400。

**行为变更（正向）**：`/v1/` 下的**未注册路径**不再先创建一个 Redis hold 再被 404 掉
—— `validate` 返回 `UnknownPath` 时直接放行给 axum。

### 3.4 ⏸ `RelayEngine` 取代 `Dispatcher` 的转发段

**按指示留给 wave 4。** 我做完 1–3 就停了，没有为了做完而囫囵吞下 4。

---

## 4. 三条必须带进来的发现

### 4.1 ✅ Google 的计费会少收钱 —— 已落地并加了守护测试

`usage::billable_tokens(provider, tokens)`：**只对 `gemini` / `vertex`**
把 `reasoning` 折进 `output`，并把 `reasoning` 清零。

事实链（写在函数 doc 里）：

| 上游 | 「输出」字段 | 含不含思考 token |
| --- | --- | --- |
| OpenAI / Codex | `usage.completion_tokens` | **含**（`reasoning_tokens` 是它的明细） |
| Anthropic | `usage.output_tokens` | **含** |
| Google | `usageMetadata.candidatesTokenCount` | **不含**（思考在 `thoughtsTokenCount`，是并列项） |

叠加 `model_prices.reasoning_price_per1_m` 的**建表默认值是 0**
（`migrations/0001_init.sql:191`）—— 结论是确定的：
**Gemini 思考型模型的每一个思考 token 今天都是免费的**，而思考 token 在推理模型上
经常是输出的数倍。折进 `output` 按输出费率计价，正是 Google 自己的计费口径。

OpenAI / Anthropic **不折**（折了就是重复计费）。归一**只作用于计价**，
写进 `usage_logs` 的仍然是上游原话，否则审计对不上上游账单。

守护测试：`google_thinking_tokens_are_not_free`（端到端，用一个把 reasoning 列
定价为 0 的计价器 —— 复刻建表默认值）与 `only_googles_output_field_needs_the_fold`。
**规范 2.11 验证：把 `billable_tokens` 改回恒等，两条用例都确认变红。**

### 4.2 ✅ L1 显式渠道前缀的未解张力 —— 我在 gw-proxy 侧解了

`gw-relay` 的 `upstream::select` 剥前缀选路由但**不改 body**（它的合同是
「唯一被授权的 body 变异是 `stream_options` 定点注入」），所以上游仍会收到带前缀的模型名。

**在 gw-proxy 侧解**：`Selection::upstream_model` 是 `Some(...)` 时，
`routes/routing.rs::rewrite_model` 改写请求体顶层的 `model` 值。
走整体 JSON round-trip —— 这条路径只在客户端**显式**写了 `<channel>/<model>` 时命中，
是 opt-in 的少数派用法，代价有界（不像 `stream_options` 那样每个流式请求都走一遍）。
解析失败（body 不是 JSON 对象）时**原样返回**。

**但我把打开 L1 做成了显式 opt-in，理由写在 `with_channel_resolver` 的 doc 里**：
OpenRouter 风格的模型名长得一模一样 —— `openai/gpt-4o`、`anthropic/claude-3.5-sonnet`
里的斜杠是**模型名的一部分**。gw-relay 已经把伤害面收窄到「前缀必须是已知 `channel_key`」，
但 `openai` 恰好既是渠道名又是 OpenRouter 的前缀，仍然会碰撞。
一个把 `openai` executor 指向 OpenRouter 的部署，今天 `openai/gpt-4o` 是能工作的，
自动打开 L1 会把它改写成 `gpt-4o` 当场变成上游 404。
**所以打开与否必须由部署方判断，不能由装配顺序替它决定。**

守护测试：`stripping_the_channel_prefix_rewrites_the_body_the_upstream_receives`、
`a_body_that_is_not_a_json_object_is_left_exactly_as_it_arrived`。
**规范 2.11 验证：把 `rewrite_model` 改成恒等，确认变红。**

### 4.3 ✅ 热点 #5 `Dispatcher::auths_for` —— 已改，并加了量化测试

`ArcSwap<AuthSnapshot>` 快照 + **按 provider 预分组** + 单飞重载（`try_lock`，
拿不到锁就用旧快照往前走，不排队等 DB）。TTL 5 秒，理由：写 `auth_records` 的
**只有面板**（新增/编辑/删除凭证、OAuth 回调），推理热路径上没有任何写入方
（已 grep 全仓库确认）。命中路径：一次 `ArcSwap::load` + 一次 `HashMap` 查表 +
一次 `Arc<[AuthRecord]>` refcount 增量，**零 `AuthRecord` 克隆**。

**我在改的过程中发现这条比基线记的还严重**：`auth_store` 在生产里是
`PostgresAuthStore`，`list()` 是**一次全表 SELECT + 对每一行做一次 AES-GCM 解密**
（`gw-authcore/src/store.rs:91-100` 的 `record_from_row`），不只是 `Vec` 克隆。
所以旧代码是**每请求、每候选 provider 一次 DB 往返 + N 次 AES-GCM 解密**。

**量化测试**：`the_credential_table_is_not_reloaded_once_per_request`。
`FakeAuthStore` 新增 `list_calls()` 计数（真实后端上「每请求几次 `list()`」
就是这条热点的可量化代理指标），跑 12 个请求断言加载次数 **< 请求数**。
测的是性质（不随请求数线性增长），不是某个具体 TTL 值。
**规范 2.11 验证：让 `auths_for` 绕过快照，确认变红。**

**我没测到的**：真实 Postgres 下的绝对收益。这是单元级的调用次数量化，
不是 `perf` 级的实测 —— 基线里凭证池只有 1 条，本来就量不出来。

---

## 5. 顺带项

### ✅ 热点 #6 `Uuid::new_v4()` 每请求一次 `getentropy`

`hold::trace_id_from` 的兜底换成「**进程随机前缀 + 单调原子计数**」：
前缀用 `LazyLock` + `Uuid::new_v4()` 在进程启动时取**一次**熵，
计数器保证同进程内唯一。形状仍是定长十六进制文本，对既有的
`usage_logs.request_id` / Redis hold 键完全兼容（列是文本，没有 UUID 约束）。

**我没验证的**：`getentropy` 归 0 这条验收目标（T12）我**没有实测**。
它需要跑 `scripts/perf` 的 profile，而那是 `perf-accept` 的活。
从代码上讲每请求的调用点已经消失，但**这是推断，不是实测**。

### ⏸ 热点 #3 幂等的全量响应缓冲 —— 我没动

`hold::capture_body` 对 1 MiB 响应 +947 µs / +4.90 MB。它改的是**幂等重放语义**，
和计费不变量纠缠：`finalize_idempotency` 用 `truncated` 标记区分
「存了但不能重放」与「没存」，而「没存」意味着重试会**被重新计费**。
任何流式化改造都要同时想清楚这条。
本轮 `orca orchestration ask` **不可用**（Orca app 未运行，CLI 报
`Could not connect to the running Orca app`），所以我按指示「不确定就别硬改」，留着没动。

---

## 6. 我没做 / 没验证的（务必读这一节）

1. **`CatalogChannelResolver` 没有被装上。** 装配点在 `gw-server/src/wiring.rs`，
   不是我的文件。当前运行时 = 纯 L4 兜底 = 与收敛前完全相同。见 §3.1 的两行补丁。
2. **缺陷 #1 只根除一半。** `/v1/responses` → openai/codex 仍然打到
   `/v1/chat/completions`。需要 `gw-provider` 的 `responses_endpoint()`
   与 `types.rs` 的方言字段，都不是我的文件。见 §3.2。
3. **7 个转义格现在回 400，转义器没有接线。** `gw-relay` 里的转义器已经写好，
   但接线需要 `RelayEngine`（优先级 4，留给 wave 4）。
4. **`getentropy` 归 0（T12）没有实测**，只有代码级推断。见 §5。
5. **热点 #5 的绝对性能收益没有实测**，只有单元级的调用次数量化。见 §4.3。
6. **`config.yaml` 的 `routing:` 段没有新增**（`gw-config` 属 `platform`）。
   L3 映射表当前是代码里的默认词表 + `with_channel()` 补充。
7. **`redact_query` 被我删了**，与方案 §4.2 的「保留」建议不同，理由见 §1。
   要留一句话就能加回来。
8. **规范 2.11 的「塞回 bug 确认变红」我做了 7 组**（见各节），
   但 `max_output_tokens` 那条没做 —— 它的实现在 `gw-relay` 里，
   把 bug 塞回去要改别人的 crate。我只验证了**接线**（`BillingPeek` 确实读到了它）。
9. **`cargo clippy` 全绿是在 `-A clippy::double_must_use` 下取得的** ——
   那条告警来自 `crates/gw-provider/src/common.rs:260`（`provider-bytes` 的文件），
   不是我的。我的文件在无豁免时也零告警。

---

## 7. 测试账

| 文件 | 变化 |
| --- | --- |
| `routes/tests/gemini.rs` | **删除**（17 条：15 条测已硬删的 `/v1beta` 面） |
| `routes/tests/dispatch.rs` | 迁入那 2 条测 `/v1` 凭证语义的用例；新增四级链 / 矩阵 / 凭证快照 / 已删路由共 12 条；2 条计费断言反转 |
| `access/tests.rs` | 4 条载体用例删除并重写为 2 条；3 条门用例重写为 3 条（含「计费 ⊂ 鉴权」的单向蕴含） |
| `hold/tests.rs` | `is_billable` 3 条重写（含新增的「安全方法永不计费」）；peek 4 条改走 `BillingPeek` |
| `usage/tests.rs` | 新增 Google 思考 token 的 2 条守护用例 |
| `adapters/catalog/tests.rs` | 新增 4 条（2 条 `#[ignore]` 需真 PG，2 条纯内存） |

`cargo test -p gw-proxy`：**218 passed / 0 failed / 19 ignored**。
