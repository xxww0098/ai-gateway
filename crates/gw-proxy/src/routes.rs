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
//! # 已根除的协议接缝
//!
//! `/v1/responses` 的端点由入口元数据决定；7 个 Translate 格现在在 proxy
//! 中显式调用对应 [`gw_relay::Translator`]。请求、普通响应和 SSE 都只翻译一次，
//! translated stream 的 usage 直接取自同一个状态机，不再额外挂 probe 重复解析。
//! 无法等价表达的 Responses→非 OpenAI 三格仍按矩阵明确返回 400。
//!
//! //! Dispatch 选出上游候选，通过 [`crate::channel`] 挑账号，失败时换**另一个**账号
//! 重试。结算**恰好一次**，在最后一次尝试之后 —— 跨账号重试只结算一次，
//! 所以 failover 不会重复计费。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use gw_authcore::{AuthRecord, AuthStore};
use gw_ledger::UpstreamAttemptId;
use gw_provider::route::{RoutePlan, RoutePlanner};
use gw_provider::types::{ProviderError, ProviderRequest};
use gw_relay::endpoint::spec::RequestSpec;
use gw_relay::endpoint::upstream::ChannelResolver;
use gw_relay::engine::{RelayEngine, RelayOptions};
use gw_relay::{
    RelayBody, RelayRequest, RelayTransportError, Surface, TranslateError, UpstreamTarget,
};

use crate::ProxyState;
use crate::body::{InboundBody, Outbound, read_inbound, rewritable};
use crate::channel::ChannelPool;
use crate::error::DispatchError;
use crate::kernel::{Phase, RelayCtx};
use crate::ports::{CircuitBreaker, ModelCatalog};
use crate::settlectx::BillingHandle;
use crate::usage::Settlement;

mod catalogue;
mod routing;
mod stream;
mod translation;

pub use catalogue::{count_tokens, model_detail, models, usage};
pub(crate) use routing::{dialect_error, partition_routable, rewrite_model, select_upstreams};
use stream::{Relayed, is_retryable_status, relay_response, schedule_release};
use translation::{UsageHandle, prepare_response, usage_probe};

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
    planners: Vec<Arc<dyn RoutePlanner>>,
    /// **The only inference HTTP exit in the workspace.**
    ///
    /// Behind the trait rather than the concrete engine so a test can inject
    /// one whose transport is scripted — the engine itself (probe guard, frame
    /// forwarding, idle watchdog) is then exercised for real.
    relay: Arc<dyn gw_relay::Relay>,
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
        planners: Vec<Arc<dyn RoutePlanner>>,
        auth_store: Arc<dyn AuthStore>,
        channels: Arc<ChannelPool>,
        settlement: Arc<Settlement>,
    ) -> Self {
        Self {
            planners,
            relay: Arc::new(RelayEngine::new(RelayOptions::default())),
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

    /// Swaps in a relay whose transport is scripted. Test-only wiring: the
    /// production path always takes the default [`RelayEngine`].
    #[must_use]
    pub fn with_relay(mut self, relay: Arc<dyn gw_relay::Relay>) -> Self {
        self.relay = relay;
        self
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

    fn planner(&self, name: &str) -> Option<&Arc<dyn RoutePlanner>> {
        self.planners.iter().find(|p| p.name() == name)
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
///
/// # One HTTP exit
///
/// The provider decides *where* (a [`RoutePlan`]) and `gw_relay::RelayEngine`
/// does the sending. That is why an upstream 429 keeps its `retry-after` here
/// and a mid-stream failure reaches the client as a reset rather than as a
/// clean EOF: there is no second copy of the response path to lose them in.
async fn dispatch(state: &ProxyState, surface: Surface, inbound: Inbound) -> Response {
    let Inbound {
        model,
        stream,
        body,
        headers,
        query,
        billing,
        mut relay,
    } = inbound;
    let dispatcher = &state.dispatch;
    let selection = select_upstreams(surface, &model, dispatcher.resolver.as_deref());
    if selection.candidates.is_empty() {
        return finish_error(state, billing.as_ref(), DispatchError::UnknownModel(model));
    }

    let (model, body) = match &selection.upstream_model {
        Some(stripped) => {
            let Some(bytes) = rewritable(&body) else {
                return finish_error(
                    state,
                    billing.as_ref(),
                    DispatchError::BodyNotRewritable(model),
                );
            };
            (
                stripped.clone(),
                RelayBody::Buffered(rewrite_model(bytes, stripped)),
            )
        }
        None => (model, body),
    };
    // Translation needs the whole source document. Keep one refcounted view
    // before `Outbound` takes ownership; direct oversized bodies remain streamed.
    let source_body = rewritable(&body).cloned();
    let mut outbound = Outbound::new(body);

    let (candidates, reject) = partition_routable(surface, &selection, &model);
    if candidates.is_empty()
        && let Some((status, body)) = reject
    {
        schedule_release(state, billing.as_ref());
        return dialect_error(status, body);
    }

    if let Some(ctx) = relay.as_mut() {
        ctx.advance(Phase::Routed);
    }

    let user_id = billing.as_ref().map(|b| b.ctx.user_id).unwrap_or(0);
    let preferred = dispatcher.channels.preferred(user_id, &model);
    let mut tried: Vec<String> = Vec::new();
    let mut last_error: Option<DispatchError> = None;
    let mut fallback: Option<Attempt> = None;

    'providers: for route in candidates {
        let provider_name = route.name;
        let Some(planner) = dispatcher.planner(provider_name) else {
            continue;
        };
        let auths = dispatcher.auths_for(provider_name).await;
        if auths.is_empty() {
            last_error.get_or_insert_with(|| DispatchError::NoUpstream(provider_name.to_owned()));
            continue;
        }

        let translated_body = match route.translator {
            Some(translator) => {
                let Some(body) = source_body.as_ref() else {
                    last_error = Some(DispatchError::BodyNotRewritable(model.clone()));
                    continue 'providers;
                };
                match translator.translate_request(&model, body) {
                    Ok(body) => Some(body),
                    Err(err) => {
                        return finish_translation_error(state, billing.as_ref(), surface, err);
                    }
                }
            }
            None => None,
        };

        while tried.len() < MAX_UPSTREAM_ATTEMPTS {
            if let Some(ctx) = relay.as_mut() {
                ctx.advance(Phase::Attempting);
            }
            let Some(auth) = dispatcher
                .channels
                .pick_sticky(&auths, preferred.as_deref(), &tried)
            else {
                break;
            };
            let auth = auth.clone();
            let attempt_id = billing
                .as_ref()
                .map(|b| UpstreamAttemptId::for_attempt(&b.ctx.operation, &auth.id, tried.len()));
            tried.push(auth.id.clone());

            let request_payload = translated_body
                .clone()
                .unwrap_or_else(|| outbound.payload());
            let request = ProviderRequest {
                model: model.clone(),
                payload: request_payload.clone(),
                stream,
                metadata: request_metadata(
                    route.planner_surface(surface),
                    billing.as_ref(),
                    attempt_id.as_ref(),
                ),
                headers: headers.clone(),
                // Production forwards the exact representation through
                // `raw_query`; pairs remain only for internal fixtures.
                query: Vec::new(),
                raw_query: Some(query.clone()),
            };
            let plan = match planner.plan(&auth, &request).await {
                Ok(plan) => plan,
                Err(err) => {
                    record_failure(state, provider_name, &auth.id, None).await;
                    last_error = Some(map_error(err));
                    continue;
                }
            };
            if plan.dialect != route.upstream {
                record_failure(state, provider_name, &auth.id, None).await;
                last_error = Some(DispatchError::Internal(anyhow::anyhow!(
                    "planner dialect {:?} disagrees with route dialect {:?}",
                    plan.dialect,
                    route.upstream,
                )));
                continue;
            }

            let outgoing = match route.translator {
                Some(_) => Some(RelayBody::Buffered(
                    plan.body.clone().unwrap_or(request_payload),
                )),
                None => outbound.next(plan.body.clone()),
            };
            let Some(outgoing) = outgoing else {
                break 'providers;
            };

            let started = Instant::now();
            let (probe, handle) = if route.translator.is_some() {
                (None, None)
            } else {
                let (probe, handle) = usage_probe(plan.dialect);
                (Some(probe), Some(handle))
            };
            match dispatcher.send(&plan, &request, outgoing, probe).await {
                Ok(response) => {
                    let retry_after = retry_after_hint(&response.headers);
                    let (response, handle) = match prepare_response(
                        response,
                        handle,
                        route.translator,
                        stream,
                        plan.dialect,
                    )
                    .await
                    {
                        Ok(prepared) => prepared,
                        Err(err) => {
                            record_failure(state, provider_name, &auth.id, None).await;
                            return finish_translation_error(state, billing.as_ref(), surface, err);
                        }
                    };

                    if is_retryable_status(response.status) {
                        record_failure(state, provider_name, &auth.id, retry_after).await;
                        fallback = Some(Attempt {
                            response,
                            handle,
                            auth_id: auth.id.clone(),
                            provider: provider_name,
                            started,
                        });
                        continue;
                    }

                    if response.status.is_success() {
                        record_success(state, provider_name, &auth.id, user_id, &model).await;
                    } else {
                        record_failure(state, provider_name, &auth.id, retry_after).await;
                    }
                    if let Some(ctx) = relay.as_mut() {
                        ctx.advance(Phase::Relaying);
                    }
                    return relay_response(
                        state,
                        Relayed {
                            response,
                            handle,
                            billing,
                            auth_id: auth.id,
                            provider: provider_name,
                            model,
                            started,
                        },
                    );
                }
                Err(err) => {
                    record_failure(state, provider_name, &auth.id, None).await;
                    last_error = Some(transport_error(err));
                }
            }
        }
    }

    if let Some(attempt) = fallback {
        if let Some(ctx) = relay.as_mut() {
            ctx.advance(Phase::Relaying);
        }
        return relay_response(
            state,
            Relayed {
                response: attempt.response,
                handle: attempt.handle,
                billing,
                auth_id: attempt.auth_id,
                provider: attempt.provider,
                model,
                started: attempt.started,
            },
        );
    }

    let err = last_error.unwrap_or_else(|| DispatchError::NoUpstream(model));
    finish_error(state, billing.as_ref(), err)
}

/// One upstream answer held aside while the dispatcher tries another account.
struct Attempt {
    response: gw_relay::RelayResponse,
    handle: UsageHandle,
    auth_id: String,
    provider: &'static str,
    started: Instant,
}

impl Dispatcher {
    /// Turns a [`RoutePlan`] into the one HTTP request the relay sends.
    ///
    /// The inbound headers ride along verbatim — `gw-relay` owns the
    /// hop-by-hop denylist and the credential swap, so filtering them a second
    /// time here would give the workspace two denylists to keep in step.
    /// The plan's own headers are layered on top of them.
    ///
    /// `body` 由调用方决定（见 [`Outbound`]），**不是**在这里无条件包一层
    /// `RelayBody::Buffered` —— 那会让一个本来边收边转的体在这里被重新缓冲。
    async fn send(
        &self,
        plan: &RoutePlan,
        req: &ProviderRequest,
        body: RelayBody,
        probe: Option<Box<dyn gw_relay::UsageProbe>>,
    ) -> Result<gw_relay::RelayResponse, RelayTransportError> {
        let (origin, target) = plan
            .split()
            .map_err(|err| RelayTransportError::BadTarget(err.to_string()))?;

        let mut headers = req.headers.clone();
        for name in plan.headers.keys() {
            headers.remove(name);
            for value in plan.headers.get_all(name) {
                headers.append(name.clone(), value.clone());
            }
        }

        self.relay
            .relay(
                RelayRequest {
                    method: axum::http::Method::POST,
                    target,
                    headers,
                    // 已缓冲就 refcount 一份，流式就把流整个交出去 —— **不再无条件缓冲**。
                    body,
                },
                &UpstreamTarget {
                    origin,
                    credential: plan.credential.clone(),
                    timeouts: plan.timeouts,
                    dialect: plan.dialect,
                },
                probe,
            )
            .await
    }
}

/// 终止一个还没到上游就被网关拒掉的请求的计费：**释放，不结算**。
///
/// 账本写入走 [`schedule_release`]（`StreamSettler` → `ProxyState::drain`），
/// 不挡错误响应回给客户端。
fn finish_billing_failed(state: &ProxyState, billing: Option<&BillingHandle>) {
    schedule_release(state, billing);
}

/// A planning failure: a broken credential, an unassemblable endpoint.
fn map_error(err: ProviderError) -> DispatchError {
    match err {
        ProviderError::Upstream { status, body } => DispatchError::Upstream {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            body,
        },
        other => DispatchError::Internal(anyhow::Error::new(other)),
    }
}

/// "Never got a response head" — DNS, TCP, TLS, connect timeout, a target that
/// will not parse. Always a 502: the request itself was fine.
fn transport_error(err: RelayTransportError) -> DispatchError {
    DispatchError::Upstream {
        status: StatusCode::BAD_GATEWAY,
        body: err.to_string(),
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
    attempt: Option<&UpstreamAttemptId>,
) -> std::collections::HashMap<String, String> {
    let mut meta = std::collections::HashMap::with_capacity(5);
    meta.insert(
        gw_provider::common::SURFACE_PATH_METADATA_KEY.to_owned(),
        surface.path().to_owned(),
    );
    if let Some(b) = billing {
        meta.insert("request_id".to_owned(), b.ctx.client_trace.to_string());
        // The money key, so an upstream log line joins back to the billing row.
        meta.insert(
            "billing_operation_id".to_owned(),
            b.ctx.operation.to_string(),
        );
        meta.insert("user_id".to_owned(), b.ctx.user_id.to_string());
    }
    if let Some(attempt) = attempt {
        // Failover produces several attempts per operation; billing settles
        // once, per operation, never per attempt.
        meta.insert("upstream_attempt_id".to_owned(), attempt.to_string());
    }
    meta
}

fn finish_translation_error(
    state: &ProxyState,
    billing: Option<&BillingHandle>,
    surface: Surface,
    err: TranslateError,
) -> Response {
    schedule_release(state, billing);
    let status = match &err {
        TranslateError::Unsupported(_) | TranslateError::Malformed(_) => StatusCode::BAD_REQUEST,
        TranslateError::UpstreamShape(_) => StatusCode::BAD_GATEWAY,
    };
    let message = err.to_string();
    let body = match surface {
        Surface::AnthropicMessages => serde_json::json!({
            "type": "error",
            "error": {
                "type": if status == StatusCode::BAD_REQUEST {
                    "invalid_request_error"
                } else {
                    "api_error"
                },
                "message": message,
            }
        }),
        Surface::OpenAiCompletions | Surface::OpenAiResponses => serde_json::json!({
            "error": {
                "message": message,
                "type": if status == StatusCode::BAD_REQUEST {
                    "invalid_request_error"
                } else {
                    "upstream_error"
                },
                "param": null,
                "code": null,
            }
        }),
    };
    (status, axum::Json(body)).into_response()
}

fn retry_after_hint(headers: &HeaderMap) -> Option<Duration> {
    const MAX_RETRY_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
    let raw = headers
        .get(axum::http::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim();
    let delay = if let Ok(seconds) = raw.parse::<u64>() {
        Duration::from_secs(seconds)
    } else {
        let at = chrono::DateTime::parse_from_rfc2822(raw)
            .ok()?
            .with_timezone(&chrono::Utc);
        (at - chrono::Utc::now()).to_std().ok()?
    };
    Some(delay.min(MAX_RETRY_AFTER))
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

async fn record_failure(
    state: &ProxyState,
    provider: &str,
    auth_id: &str,
    retry_after: Option<Duration>,
) {
    state
        .dispatch
        .channels
        .health()
        .record_result(auth_id, false, retry_after);
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
    /// 入站体的两态。**看不见不等于转不出去** —— 超
    /// [`crate::body::BILLING_PEEK_LIMIT`] 的体是一条流，照样发给上游。
    body: RelayBody,
    /// Inbound headers, forwarded upstream through
    /// `gw_provider::types::copy_outbound_headers` by each provider.
    headers: HeaderMap,
    /// Query exactly as it appeared after `?`.
    query: String,
    billing: Option<BillingHandle>,
    relay: Option<RelayCtx>,
}

/// Takes the body the hold layer already read, or reads it here when no
/// pre-flight ran. **两条路都只读前缀**，超阈值的部分边收边转。
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
    let peeked = parts.extensions.remove::<InboundBody>();
    let spec = parts.extensions.remove::<RequestSpec>();
    let headers = parts.headers;
    let query = parts.uri.query().unwrap_or_default().to_owned();

    let body = match peeked.and_then(InboundBody::take) {
        Some(body) => body,
        // 扩展缺席只有一种成因：这条路径**不计费**，hold 层整个被跳过。
        // 这里同样只读前缀 —— 超阈值不是 413，是「计费看不见」。
        None => read_inbound(body).await?,
    };

    let spec = spec.unwrap_or_else(|| RequestSpec::parse(surface, body.peek()));
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
    /// OpenAI / Codex 原生直通；Claude / Google 三格因有状态 item 语义无法
    /// 等价表达，按 15 格矩阵明确返回入口方言的 400。
    responses,
    Surface::OpenAiResponses
);
endpoint!(
    /// 入口 C · `POST /v1/messages` —— Anthropic Messages 方言。
    messages,
    Surface::AnthropicMessages
);

#[cfg(test)]
mod tests;
