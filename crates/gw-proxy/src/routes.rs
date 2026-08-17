//! 三个 `/v1` 入口与上游派发。
//!
//! 行为的参照是这些路由所处的计费管线：preflight hold → dispatch → settle。
//!
//! # 本轮接入 `gw-relay` 的三件事
//!
//! | 取代了什么 | 换成了什么 | 根除的缺陷 |
//! | --- | --- | --- |
//! | `provider_candidates()` 的字符串前缀猜测 | [`gw_relay::endpoint::upstream::select`] 四级链 | `docs/relay-surface-plan.md` §3.5.1 的四个静默失效场景 |
//! | 「所有入口都直通、错了让上游回 400」的隐式派发 | [`gw_relay::endpoint::matrix::route`] 的 15 格显式表 | 审计缺陷 #1（S1）的一半 —— 见下面的**已知缺口** |
//! | `hold.rs` 与 `routes.rs` 各解析一次 body | 唯一一次解析的 [`RequestSpec`] 经请求扩展下发 | 审计缺陷 #15（S3） |
//!
//! # 已知缺口（本轮**未**根除，需要 `gw-provider` 配合）
//!
//! 缺陷 #1 的另一半：`POST /v1/responses` 打到 openai / codex 时，
//! [`gw_relay::endpoint::matrix::upstream_dialect`] 已经正确判定为
//! [`gw_relay::UpstreamDialect::OpenAiResponses`]，**但 `gw-provider` 的
//! executor 仍然只会构造 `{base}/v1/chat/completions`**
//! （`gw-provider/src/openai.rs` 的 `chat_completions_endpoint`），
//! 而 `ProviderRequest` 上没有承载入口方言的字段（`gw-provider/src/types.rs`
//! 是协调者独占，本轮不动）。所以入口 B 打到 OpenAI 系上游时**仍然是坏的**。
//!
//! 需要的补丁在别人的文件里：`gw-provider` 新增 `responses_endpoint()`，
//! 并让 `ProviderRequest` 带上方言（`docs/relay-surface-plan.md` §4.6）。
//! 或者由 wave 4 用 [`gw_relay::engine::RelayEngine`] 整体取代本文件的转发段
//! —— 那时端点由入口决定，provider 根本不参与拼 URL。
//!
//! Dispatch 选出上游候选，通过 [`crate::channel`] 挑账号，失败时换**另一个**账号
//! 重试。结算**恰好一次**，在最后一次尝试之后 —— 跨账号重试只结算一次，
//! 所以 failover 不会重复计费。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use axum::body::Bytes;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use gw_authcore::{AuthRecord, AuthStore};
use gw_provider::types::{Provider, ProviderError, ProviderRequest};
use gw_relay::Surface;
use gw_relay::endpoint::spec::RequestSpec;
use gw_relay::endpoint::upstream::ChannelResolver;

use crate::ProxyState;
use crate::channel::ChannelPool;
use crate::error::DispatchError;
use crate::hold::PeekedBody;
use crate::kernel::{Phase, RelayCtx};
use crate::ports::{CircuitBreaker, ModelCatalog};
use crate::settlectx::BillingHandle;
use crate::usage::Settlement;

mod routing;
mod stream;

pub(crate) use routing::{dialect_error, partition_routable, rewrite_model, select_upstreams};
#[cfg(test)]
pub(crate) use stream::is_hop_by_hop;
use stream::{schedule_release, stream_response, unary_response};

/// How many upstream accounts one client request may burn through.
/// Bounded so a fully-broken pool fails fast instead of walking every account.
pub const MAX_UPSTREAM_ATTEMPTS: usize = 3;

/// 凭证快照的最大陈旧窗口。
///
/// 写入 `auth_records` 的只有面板（新增/编辑/删除凭证、OAuth 回调），
/// **推理热路径上没有任何写入方**，所以一个有界陈旧的快照是安全的：
/// 管理员改完凭证，最迟这么久后生效。与 [`crate::channel::ChannelPolicyCache`]
/// 是同一种姿态。
pub const AUTH_SNAPSHOT_TTL: Duration = Duration::from_secs(5);

/// Upstream execution: account selection, retry, and the single settlement.
///
/// Collectively implements the auth manager + the conductor's retry
/// loop.
pub struct Dispatcher {
    providers: Vec<Arc<dyn Provider>>,
    auth_store: Arc<dyn AuthStore>,
    /// 按 provider 预分组的可用凭证快照。见 [`AuthSnapshot`]。
    auths: ArcSwap<AuthSnapshot>,
    /// 单飞闸门：快照过期时只允许一个请求去重载，其余继续用旧快照。
    reloading: tokio::sync::Mutex<()>,
    channels: Arc<ChannelPool>,
    settlement: Arc<Settlement>,
    circuit_breaker: Option<Arc<dyn CircuitBreaker>>,
    catalog: Option<Arc<dyn ModelCatalog>>,
    /// 四级链的 L1/L2/L3 数据源。`None` = 一键回滚到纯前缀猜测（L4）。
    resolver: Option<Arc<dyn ChannelResolver>>,
}

/// 按 provider 预分组、已过滤 `is_usable()` 的凭证快照。
///
/// # 为什么存在（性能基线热点 #5）
///
/// 收敛前每个请求、每个候选 provider 都调一次 `auth_store.list()`：对
/// `PostgresAuthStore` 那是**一次全表 SELECT、对每一行做一次 AES-GCM 解密、
/// 再克隆整份凭证表**，然后当场丢掉除一个 provider 之外的全部结果。
/// 基线里凭证池只有 1 条所以量不出来（`docs/relay-perf-baseline.md` 明确
/// 标注这条是**结构推断不是实测**），但代价随凭证数线性增长。
///
/// 快照把这件事从「每请求 O(凭证数)」变成「每 [`AUTH_SNAPSHOT_TTL`] 一次」，
/// 命中路径只有一次 `ArcSwap::load` + 一次 `HashMap` 查表 + 一次
/// `Arc<[AuthRecord]>` 的 refcount 增量 —— **零 `AuthRecord` 克隆**。
struct AuthSnapshot {
    by_provider: HashMap<String, Arc<[AuthRecord]>>,
    /// 快照生成时刻。`None` = 从未成功加载过，任何读取都视为过期。
    loaded_at: Option<Instant>,
}

impl AuthSnapshot {
    fn empty() -> Self {
        Self {
            by_provider: HashMap::new(),
            loaded_at: None,
        }
    }

    fn is_fresh(&self, ttl: Duration) -> bool {
        self.loaded_at.is_some_and(|at| at.elapsed() < ttl)
    }

    fn get(&self, provider: &str) -> Arc<[AuthRecord]> {
        self.by_provider
            .get(provider)
            .cloned()
            .unwrap_or_else(|| Arc::from(Vec::new()))
    }
}

impl Dispatcher {
    /// Wires the upstream side of the pipeline.
    ///
    /// The manager plus the executor registration loop.
    pub fn new(
        providers: Vec<Arc<dyn Provider>>,
        auth_store: Arc<dyn AuthStore>,
        channels: Arc<ChannelPool>,
        settlement: Arc<Settlement>,
    ) -> Self {
        Self {
            providers,
            auth_store,
            auths: ArcSwap::from_pointee(AuthSnapshot::empty()),
            reloading: tokio::sync::Mutex::new(()),
            channels,
            settlement,
            circuit_breaker: None,
            catalog: None,
            resolver: None,
        }
    }

    /// Hands the same breaker to hold and dispatch.
    #[must_use]
    pub fn with_circuit_breaker(mut self, cb: Arc<dyn CircuitBreaker>) -> Self {
        self.circuit_breaker = Some(cb);
        self
    }

    /// Source for `GET /v1/models`.
    #[must_use]
    pub fn with_catalog(mut self, catalog: Arc<dyn ModelCatalog>) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// 装上四级链的 L1/L2/L3 数据源。
    ///
    /// **不装 = 一键回滚**：`resolver` 为 `None` 时 [`select_upstreams`] 直接落 L4，
    /// 结果与收敛前的 `provider_candidates()` 逐字节相同。这是安全灰度的前提。
    ///
    /// # 为什么这是显式 opt-in，而不是跟着 [`Self::with_catalog`] 自动装上
    ///
    /// 装上 resolver 会打开 **L1 显式渠道前缀**（`<channel_key>/<model_id>`），
    /// 而这一级会剥掉前缀并**改写请求体的 `model`**（见 [`rewrite_model`]）。
    /// 对 `codex/gpt-5` 这类「前缀是给网关看的」写法，这正是想要的。
    ///
    /// 但 OpenRouter 风格的模型名**长得一模一样**：`openai/gpt-4o`、
    /// `anthropic/claude-3.5-sonnet` 里的斜杠是**模型名的一部分**。
    /// 一个把 `openai` executor 指向 OpenRouter 的部署，今天 `openai/gpt-4o`
    /// 是能工作的；自动打开 L1 会把它改写成 `gpt-4o`，**当场变成上游 404**。
    ///
    /// `gw-relay` 已经把伤害面收窄到「前缀必须是一个已知 `channel_key`」，
    /// 但这两类名字在**已知渠道名**上仍然会碰撞（`openai` 就是其一）。
    /// 所以打开与否必须由部署方判断，不能由装配顺序替它决定。
    #[must_use]
    pub fn with_channel_resolver(mut self, resolver: Arc<dyn ChannelResolver>) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// The account pool, for the metrics exporter.
    pub fn channels(&self) -> &Arc<ChannelPool> {
        &self.channels
    }

    fn provider(&self, name: &str) -> Option<&Arc<dyn Provider>> {
        self.providers.iter().find(|p| p.name() == name)
    }

    /// Live credentials for one provider, newest state first.
    ///
    /// 命中快照时**零 I/O、零 `AuthRecord` 克隆**。快照过期时单飞重载：
    /// 只有一个任务真的去查库，其余任务继续用旧快照往前走
    /// —— DB 抖动不该变成推理延迟。
    async fn auths_for(&self, provider: &str) -> Arc<[AuthRecord]> {
        let snapshot = self.auths.load();
        if snapshot.is_fresh(AUTH_SNAPSHOT_TTL) {
            return snapshot.get(provider);
        }
        drop(snapshot);

        let Ok(_guard) = self.reloading.try_lock() else {
            // 别人正在重载。用当前快照往前走，而不是排队等 DB。
            return self.auths.load().get(provider);
        };
        // 拿到闸门之后再看一眼：等锁期间可能已经有人刷好了。
        let snapshot = self.auths.load();
        if snapshot.is_fresh(AUTH_SNAPSHOT_TTL) {
            return snapshot.get(provider);
        }
        drop(snapshot);

        match self.reload_auths().await {
            Ok(snapshot) => snapshot.get(provider),
            Err(err) => {
                tracing::warn!(%err, provider, "listing upstream credentials failed");
                // 保留旧快照：一次查库失败不该让整池凭证凭空消失。
                self.auths.load().get(provider)
            }
        }
    }

    /// 重载并发布快照。
    ///
    /// `pub` 是为了让组合根能把它挂到既有的刷新 ticker 上
    /// （照抄 `ChannelPolicyCache::spawn_refresh` 的生命周期）。不挂也能工作
    /// —— [`Self::auths_for`] 会在快照过期时自己重载。
    pub async fn refresh_auths(&self) -> anyhow::Result<()> {
        let _guard = self.reloading.lock().await;
        self.reload_auths().await.map(drop)
    }

    async fn reload_auths(&self) -> anyhow::Result<Arc<AuthSnapshot>> {
        let records = self.auth_store.list().await?;
        let mut grouped: HashMap<String, Vec<AuthRecord>> = HashMap::new();
        for record in records.into_iter().filter(AuthRecord::is_usable) {
            grouped
                .entry(record.provider.clone())
                .or_default()
                .push(record);
        }
        // xAI Grok OAuth credentials are stored as provider `xai` but speak
        // the OpenAI-compatible wire. Attach them to the openai bucket so the
        // existing OpenAI executor can use them (with the record's base_url).
        // Do not merge `kiro` — that API is not OpenAI-compatible.
        if let Some(xai) = grouped.get("xai").cloned() {
            grouped.entry("openai".to_owned()).or_default().extend(xai);
        }
        let snapshot = Arc::new(AuthSnapshot {
            by_provider: grouped
                .into_iter()
                .map(|(provider, records)| (provider, Arc::from(records)))
                .collect(),
            loaded_at: Some(Instant::now()),
        });
        self.auths.store(Arc::clone(&snapshot));
        Ok(snapshot)
    }
}

/// Runs one client request against the upstream pool and settles it once.
///
/// `billing` is `None` for the endpoints [`crate::hold::is_billable`] excludes
/// (token counting, catalogue reads); those never reserve, never settle.
async fn dispatch(state: &ProxyState, surface: Surface, mut inbound: Inbound) -> Response {
    let stream = inbound.stream;
    let billing = inbound.billing.clone();
    let model = std::mem::take(&mut inbound.model);
    let dispatcher = &state.dispatch;
    let selection = select_upstreams(surface, &model, dispatcher.resolver.as_deref());
    if selection.candidates.is_empty() {
        return finish_error(
            state,
            billing.as_ref(),
            DispatchError::UnknownModel(model.clone()),
        );
    }

    // L1 剥掉了渠道前缀时，**上游收到的模型名也必须跟着变**。
    // 这条张力写在 `gw_relay::endpoint::upstream::Selection::upstream_model` 的
    // doc 里但没有在那一层解决：中继层的合同是「唯一被授权的 body 变异是
    // `stream_options` 定点注入」，所以剥前缀只算出名字、不改 body。
    // 在这里解：`gw-proxy` 不受那条合同约束，而客户端写 `codex/gpt-5` 时
    // `codex/` 明确是给网关看的 —— 把它原样转给上游只会拿一个「模型不存在」。
    let (model, body) = match &selection.upstream_model {
        Some(stripped) => (stripped.clone(), rewrite_model(&inbound.body, stripped)),
        None => (model, std::mem::take(&mut inbound.body)),
    };

    // 15 格显式表：直通 / 转义 / 400。挑第一个不是 `Reject` 的候选；
    // 全部 `Reject` 时把第一个的错误信封原样回给客户端。
    let (candidates, reject) = partition_routable(surface, &selection, &model);
    if candidates.is_empty()
        && let Some((status, body)) = reject
    {
        // 400 **不计费** —— 走释放路径而不是结算。
        schedule_release(state, billing.as_ref());
        return dialect_error(status, body);
    }

    if let Some(ctx) = inbound.relay.as_mut() {
        ctx.advance(Phase::Routed);
    }

    let user_id = billing.as_ref().map(|b| b.ctx.user_id).unwrap_or(0);
    let preferred = dispatcher.channels.preferred(user_id, &model);
    let mut tried: Vec<String> = Vec::new();
    let mut last_error: Option<DispatchError> = None;

    for provider_name in candidates {
        let Some(provider) = dispatcher.provider(provider_name) else {
            continue;
        };
        let auths = dispatcher.auths_for(provider_name).await;
        if auths.is_empty() {
            last_error.get_or_insert_with(|| DispatchError::NoUpstream(provider_name.to_owned()));
            continue;
        }

        while tried.len() < MAX_UPSTREAM_ATTEMPTS {
            if let Some(ctx) = inbound.relay.as_mut() {
                ctx.advance(Phase::Attempting);
            }
            let Some(auth) = dispatcher
                .channels
                .pick_sticky(&auths, preferred.as_deref(), &tried)
            else {
                break; // this provider's accounts are exhausted for this request
            };
            let auth = auth.clone();
            tried.push(auth.id.clone());

            let request = ProviderRequest {
                model: model.clone(),
                // `Bytes::clone` 是 refcount：三次 failover 重试零拷贝
                // （审计缺陷 #13/#14 —— 一个 900 KB 的请求此前要 memcpy 约 5.4 MB）。
                payload: body.clone(),
                stream,
                metadata: request_metadata(surface, billing.as_ref()),
                headers: inbound.headers.clone(),
                query: inbound.query.clone(),
            };
            let started = Instant::now();

            if stream {
                match provider.execute_stream(&auth, request).await {
                    Ok(upstream) => {
                        // The connection stood up, so the account is working;
                        // a mid-stream failure is reported through the usage
                        // outcome instead.
                        record_success(state, provider_name, &auth.id, user_id, &model).await;
                        if let Some(ctx) = inbound.relay.as_mut() {
                            ctx.advance(Phase::Relaying);
                        }
                        return stream_response(
                            state,
                            upstream,
                            billing,
                            auth.id,
                            provider_name,
                            started,
                        );
                    }
                    Err(err) => {
                        record_failure(state, provider_name, &auth.id).await;
                        if !is_retryable(&err) {
                            return finish_error(state, billing.as_ref(), map_error(err));
                        }
                        last_error = Some(map_error(err));
                    }
                }
            } else {
                match provider.execute(&auth, request).await {
                    // An error status the provider relayed rather than raised
                    // is still that account failing: fail over to the next one
                    // instead of handing the client a 503 another credential
                    // would have served.
                    Ok(response) if is_retryable_status(response.status) => {
                        record_failure(state, provider_name, &auth.id).await;
                        last_error = Some(DispatchError::Upstream {
                            status: StatusCode::from_u16(response.status)
                                .unwrap_or(StatusCode::BAD_GATEWAY),
                            body: String::from_utf8_lossy(&response.body).into_owned(),
                        });
                    }
                    Ok(response) => {
                        record_success(state, provider_name, &auth.id, user_id, &model).await;
                        if let Some(ctx) = inbound.relay.as_mut() {
                            ctx.advance(Phase::Relaying);
                        }
                        return unary_response(
                            state,
                            response,
                            billing,
                            auth.id,
                            provider_name,
                            started,
                        );
                    }
                    Err(err) => {
                        record_failure(state, provider_name, &auth.id).await;
                        if !is_retryable(&err) {
                            return finish_error(state, billing.as_ref(), map_error(err));
                        }
                        last_error = Some(map_error(err));
                    }
                }
            }
        }
    }

    let err = last_error.unwrap_or_else(|| DispatchError::NoUpstream(model));
    finish_error(state, billing.as_ref(), err)
}

/// 终止一个还没到上游就被网关拒掉的请求的计费：**释放，不结算**。
///
/// 账本写入走 [`schedule_release`]（`StreamSettler` → `ProxyState::drain`），
/// 不挡错误响应回给客户端。
fn finish_billing_failed(state: &ProxyState, billing: Option<&BillingHandle>) {
    schedule_release(state, billing);
}

/// Whether a failure is worth trying on a different account.
///
/// A 4xx other than 429 is the caller's fault and will fail identically
/// everywhere, so it is surfaced immediately instead of burning the pool.
fn is_retryable(err: &ProviderError) -> bool {
    match err {
        ProviderError::Upstream { status, .. } => is_retryable_status(*status),
        ProviderError::Credential(_) | ProviderError::Transport(_) => true,
        ProviderError::Other(_) => false,
    }
}

/// Statuses that say "this account, right now" rather than "this request".
fn is_retryable_status(status: u16) -> bool {
    status >= 500 || status == 429
}

fn map_error(err: ProviderError) -> DispatchError {
    match err {
        ProviderError::Upstream { status, body } => DispatchError::Upstream {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            body,
        },
        other => DispatchError::Internal(anyhow::Error::new(other)),
    }
}

/// Metadata handed to the provider, mirroring what the SDK put on its request
/// options so an executor can log or shard by tenant.
/// 交给 executor 的每请求元数据。
///
/// # `surface_path` —— 缺陷 #1（S1）的最后一公里
///
/// executor 靠这个键决定打哪个上游端点（`gw_provider::common::request_surface`）。
/// **不写它，`POST /v1/responses` 就会静默回落到 chat/completions** ——
/// executor 侧的 `responses_endpoint` 已经就绪，但键缺失时的兼容回落正好等于
/// 缺陷 #1 的原状：Responses 形状的 body 被发到 Chat Completions 端点，上游必 400，
/// 三个保留入口之一 100% 不可用。
///
/// 存路径本身而不是新造一个枚举字符串：路径 ↔ 入口的映射
/// [`Surface::path`] / [`Surface::from_path`] 已经在 `gw-relay` 里声明过一次了。
fn request_metadata(
    surface: Surface,
    billing: Option<&BillingHandle>,
) -> std::collections::HashMap<String, String> {
    let mut meta = std::collections::HashMap::with_capacity(3);
    meta.insert(
        gw_provider::common::SURFACE_PATH_METADATA_KEY.to_owned(),
        surface.path().to_owned(),
    );
    if let Some(b) = billing {
        meta.insert("request_id".to_owned(), b.ctx.request_id.clone());
        meta.insert("user_id".to_owned(), b.ctx.user_id.to_string());
    }
    meta
}

async fn record_success(
    state: &ProxyState,
    provider: &str,
    auth_id: &str,
    user_id: i64,
    model: &str,
) {
    state
        .dispatch
        .channels
        .health()
        .record_result(auth_id, true, None);
    state.dispatch.channels.remember(user_id, model, auth_id);
    if let Some(cb) = &state.dispatch.circuit_breaker {
        cb.record(provider, true).await;
    }
}

async fn record_failure(state: &ProxyState, provider: &str, auth_id: &str) {
    state
        .dispatch
        .channels
        .health()
        .record_result(auth_id, false, None);
    if let Some(cb) = &state.dispatch.circuit_breaker {
        cb.record(provider, false).await;
    }
}

/// Terminates billing for a request that never reached an upstream.
fn finish_error(
    state: &ProxyState,
    billing: Option<&BillingHandle>,
    err: DispatchError,
) -> Response {
    finish_billing_failed(state, billing);
    err.into_response()
}

// ---------------------------------------------------------------- handlers

/// Body + model resolved from an inbound request.
struct Inbound {
    model: String,
    stream: bool,
    body: Bytes,
    /// Inbound headers, forwarded upstream through
    /// `gw_provider::types::copy_outbound_headers` by each provider.
    headers: HeaderMap,
    /// Inbound query pairs, appended to the provider endpoint. Order and
    /// duplicates are both significant, hence a `Vec`.
    query: Vec<(String, String)>,
    billing: Option<BillingHandle>,
    relay: Option<RelayCtx>,
}

/// Reuses the body the hold layer already buffered, or reads it directly when
/// no pre-flight ran.
///
/// # 全链路唯一一次 body 解析（根除缺陷 #15）
///
/// 计费层已经解析过一次并把 [`RequestSpec`] 挂在请求扩展上，这里**直接复用**。
/// 收敛前这里会调第二次 `parse_body_peek`，一个 900 KB 的 body 被解析两遍
/// （流式还有第三遍，在 `ensure_include_usage` 里）。
///
/// 扩展缺席只有一种成因：这条路径**不计费**（`hold::is_billable` 排除的
/// `count_tokens` 与两条 catalogue 读），hold 层整个被跳过。此时在这里解析一次
/// —— 仍然是每请求恰好一次。
///
/// # ⚠️ `/v1` 上的 `x-api-key` 属于上游，不许剥
///
/// 收敛前这里调 `access::strip_consumed_credentials` 剥掉 Gemini 面的租户凭据
/// 载体。`/v1beta` 硬删之后那个函数整个消失了，但**它守住的语义必须留下**：
/// `x-api-key` 在 `/v1` 上是 **Anthropic 自己的上游头**，claude executor 需要它，
/// 网关必须原样透传。它与 Gemini 面上那个同名的租户凭据载体只是碰巧同名。
/// 下一个人如果「顺手」在这里加一句 `headers.remove("x-api-key")`，
/// Anthropic 直连会立刻全线 401。
async fn inbound(req: Request, surface: Surface) -> Result<Inbound, Response> {
    let (mut parts, body) = req.into_parts();
    let billing = parts.extensions.remove::<BillingHandle>();
    let relay = parts.extensions.remove::<RelayCtx>();
    let peeked = parts.extensions.remove::<PeekedBody>();
    let spec = parts.extensions.remove::<RequestSpec>();
    let headers = parts.headers;
    let query = parse_query(parts.uri.query().unwrap_or_default());

    let body = match peeked {
        Some(PeekedBody(bytes)) => bytes,
        None => match axum::body::to_bytes(body, crate::hold::HOLD_REQUEST_BODY_LIMIT).await {
            Ok(bytes) => bytes,
            Err(_) => return Err(StatusCode::PAYLOAD_TOO_LARGE.into_response()),
        },
    };

    let spec = spec.unwrap_or_else(|| RequestSpec::parse(surface, Some(&body)));
    Ok(Inbound {
        model: billing
            .as_ref()
            .map(|b| b.ctx.model.clone())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| spec.model().unwrap_or_default().trim().to_owned()),
        stream: spec.stream,
        body,
        headers,
        query,
        billing,
        relay,
    })
}

/// Splits a raw query string into ordered `(key, value)` pairs, percent-decoding
/// nothing: the providers re-encode when they build the outbound URL.
fn parse_query(raw: &str) -> Vec<(String, String)> {
    raw.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (k.to_owned(), v.to_owned()),
            None => (pair.to_owned(), String::new()),
        })
        .collect()
}

macro_rules! endpoint {
    ($(#[$meta:meta])* $name:ident, $surface:expr) => {
        $(#[$meta])*
        pub async fn $name(State(state): State<ProxyState>, req: Request) -> Response {
            match inbound(req, $surface).await {
                Ok(i) => dispatch(&state, $surface, i).await,
                Err(response) => response,
            }
        }
    };
}

endpoint!(
    /// 入口 A · `POST /v1/chat/completions` —— OpenAI Chat Completions 方言。
    chat_completions,
    Surface::OpenAiCompletions
);
endpoint!(
    /// 入口 B · `POST /v1/responses` —— OpenAI Responses 方言。
    ///
    /// ⚠️ **打到 openai / codex 时今天仍然是坏的**：矩阵已经把它判成
    /// [`gw_relay::UpstreamDialect::OpenAiResponses`]，但 `gw-provider` 的
    /// executor 只会构造 `{base}/v1/chat/completions`。见模块 doc 的「已知缺口」。
    responses,
    Surface::OpenAiResponses
);
endpoint!(
    /// 入口 C · `POST /v1/messages` —— Anthropic Messages 方言。
    messages,
    Surface::AnthropicMessages
);

/// `POST /v1/messages/count_tokens` —— Anthropic 的 token 计数，入口 C 的附属端点。
///
/// **不计费**（`hold::is_billable` 排除 `/count_tokens`）：Anthropic 自己对它
/// 收 0，而收敛前网关按那个模型的 LLM 费率收钱。见 [`crate::hold::is_billable`]。
///
/// 它也**不进 dispatch 的重试链**：只挑一个账号，不 failover。
pub async fn count_tokens(State(state): State<ProxyState>, req: Request) -> Response {
    let inbound = match inbound(req, Surface::AnthropicMessages).await {
        Ok(i) => i,
        Err(response) => return response,
    };
    let selection = select_upstreams(
        Surface::AnthropicMessages,
        &inbound.model,
        state.dispatch.resolver.as_deref(),
    );
    for candidate in &selection.candidates {
        let name = candidate.as_str();
        let Some(provider) = state.dispatch.provider(name) else {
            continue;
        };
        let auths = state.dispatch.auths_for(name).await;
        let Some(auth) = state.dispatch.channels.pick(&auths) else {
            continue;
        };
        let request = ProviderRequest {
            model: inbound.model.clone(),
            payload: inbound.body.clone(),
            stream: false,
            metadata: std::collections::HashMap::new(),
            headers: inbound.headers.clone(),
            query: inbound.query.clone(),
        };
        return match provider.count_tokens(auth, request).await {
            Ok(count) => axum::Json(serde_json::json!({ "input_tokens": count })).into_response(),
            Err(err) => map_error(err).into_response(),
        };
    }
    DispatchError::NoUpstream(inbound.model).into_response()
}

/// `GET /v1/models` —— OpenAI 形状的模型目录。
///
/// **必须保留**，理由不是「前端在调」（前端对 `/v1` 的 HTTP 调用数是 0），
/// 而是面板 `QuickIntegrationPanel.tsx:79` 把 `${origin}/v1` 作为 Base URL
/// 印给用户 —— 所有 OpenAI 兼容客户端（Cursor / Cline / aider / OpenWebUI /
/// LobeChat）拿到 base 之后的第一个请求就是 `GET {base}/models`，
/// 用它渲染模型下拉框。删了它，照面板指引配置的客户端在连接测试阶段就失败。
///
/// **不计费**：纯 DB 读，不出网。收敛前它按 fallback estimate 收租户约 $0.004
/// —— 见 [`crate::hold::is_billable`]。
pub async fn models(State(state): State<ProxyState>) -> Response {
    let Some(catalog) = &state.dispatch.catalog else {
        return axum::Json(serde_json::json!({ "object": "list", "data": [] })).into_response();
    };
    match catalog.list_models().await {
        Ok(models) => axum::Json(serde_json::json!({
            "object": "list",
            "data": models.iter().map(model_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(err) => {
            tracing::warn!(%err, "model catalog lookup failed");
            DispatchError::Internal(err).into_response()
        }
    }
}

/// `GET /v1/models/{model}` — one catalogue entry.
pub async fn model_detail(State(state): State<ProxyState>, Path(model): Path<String>) -> Response {
    let Some(catalog) = &state.dispatch.catalog else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match catalog.list_models().await {
        Ok(models) => match models.iter().find(|m| m.id == model) {
            Some(m) => axum::Json(model_json(m)).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
        Err(err) => DispatchError::Internal(err).into_response(),
    }
}

/// OpenAI listing object plus the catalog fields Harness needs.
pub(crate) fn model_json(model: &crate::ports::ModelEntry) -> serde_json::Value {
    let mut value = serde_json::json!({
        "id": model.id,
        "object": "model",
        "created": model.created,
        "owned_by": model.owned_by,
    });
    if let Some(n) = model.context_length {
        value["context_length"] = serde_json::json!(n);
    }
    if let Some(n) = model.max_output_tokens {
        value["max_output_tokens"] = serde_json::json!(n);
    }
    if !model.input_modalities.is_empty() {
        value["input_modalities"] = serde_json::json!(model.input_modalities);
    }
    if let Some(reasoning) = &model.reasoning {
        if !reasoning.efforts.is_empty() {
            let mut body = serde_json::json!({
                "efforts": reasoning
                    .efforts
                    .iter()
                    .map(|e| serde_json::json!({ "id": e.id, "name": e.name }))
                    .collect::<Vec<_>>(),
            });
            if let Some(default_effort) = &reasoning.default_effort {
                body["default_effort"] = serde_json::json!(default_effort);
            }
            value["reasoning"] = body;
        }
    }
    value
}

#[cfg(test)]
mod tests;
