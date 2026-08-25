//! 四条**非推理**路由：token 计数与三条目录 / 用量读。
//!
//! 它们与三个推理入口住在一起本来只是历史顺序 —— 它们**全部不计费**
//! （`hold::is_billable` 排除全部 GET 与 `/count_tokens`），
//! 不进重试链，也不产 usage。切出来之后，「删掉目录这个功能」
//! 等于删掉这一个文件（规则 1.6）。
//!
//! 唯一与推理共用的东西是那条出网路径：`count_tokens` 仍然经
//! [`super::Dispatcher::send`] 走 `gw-relay`，因为**工作区只有一个推理 HTTP 出口**。

use std::time::Instant;

use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use gw_provider::types::ProviderRequest;
use gw_relay::Surface;

use super::stream::{Relayed, relay_response, usage_probe};
use super::{Inbound, inbound, map_error, request_metadata, select_upstreams, transport_error};
use crate::ProxyState;
use crate::body::Outbound;
use crate::error::{DispatchError, HoldRejection};
use crate::kernel::RelayCtx;

/// `POST /v1/messages/count_tokens` —— Anthropic 的 token 计数，入口 C 的附属端点。
///
/// **不计费**（`hold::is_billable` 排除 `/count_tokens`）：Anthropic 自己对它
/// 收 0，而收敛前网关按那个模型的 LLM 费率收钱。见 [`crate::hold::is_billable`]。
///
/// 它也**不进 dispatch 的重试链**：只挑一个账号，不 failover。
///
/// # 上游怎么说，客户端就看到什么
///
/// 这是一条**恒等转发**：请求体原样送上去，响应原样送回来。网关不再解析
/// `{"input_tokens": N}` 再重新拼一个 —— 解析一次就多一个会漏、会改口径的地方，
/// 而这个信封本来就是 Anthropic 方言入口自己的形状。上游给不出数，客户端就看到
/// 上游给的那个错误，而不是网关编的一个数字。
pub async fn count_tokens(State(state): State<ProxyState>, req: Request) -> Response {
    let Inbound {
        model,
        body,
        headers,
        query,
        ..
    } = match inbound(req, Surface::AnthropicMessages).await {
        Ok(i) => i,
        Err(response) => return response,
    };
    let mut outbound = Outbound::new(body);
    let selection = select_upstreams(
        Surface::AnthropicMessages,
        &model,
        state.dispatch.resolver.as_deref(),
    );
    for candidate in &selection.candidates {
        let name = candidate.as_str();
        let Some(planner) = state.dispatch.planner(name) else {
            continue;
        };
        let auths = state.dispatch.auths_for(name).await;
        let Some(auth) = state.dispatch.channels.pick(&auths) else {
            continue;
        };
        let request = ProviderRequest {
            model: model.clone(),
            payload: outbound.payload(),
            stream: false,
            metadata: request_metadata(Surface::AnthropicMessages, None, None),
            headers: headers.clone(),
            query: query.clone(),
        };
        let plan = match planner.plan_count_tokens(auth, &request).await {
            Ok(plan) => plan,
            Err(err) => return map_error(err).into_response(),
        };
        // 这条路径本来就不 failover，所以「只有一次」正好够用；
        // 拿不到 body 只可能是上一轮已经发过了，那时循环早就 return 了。
        let Some(outgoing) = outbound.next(plan.body.clone()) else {
            break;
        };
        let (probe, handle) = usage_probe(plan.dialect);
        return match state.dispatch.send(&plan, &request, outgoing, probe).await {
            Ok(response) => relay_response(
                &state,
                Relayed {
                    response,
                    handle,
                    billing: None,
                    auth_id: auth.id.clone(),
                    provider: name,
                    model: model.clone(),
                    started: Instant::now(),
                },
            ),
            Err(err) => transport_error(err).into_response(),
        };
    }
    DispatchError::NoUpstream(model).into_response()
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

/// `GET /v1/usage` —— 用现有 Bearer 查钱包余额和当地今日按模型的 token 消耗。
///
/// **不计费**：`is_billable` 排除全部 GET，不会 Hold/Settle。
/// 响应是裸 JSON（与 `GET /v1/models` 一样），不是面板 `{code,data}` 信封。
/// 账本 / 用量读失败走内部错误，不 402 —— 这条路径不进预扣，402 会误导客户端以为被拒付。
pub async fn usage(State(state): State<ProxyState>, req: Request) -> Response {
    let Some(meta) = req
        .extensions()
        .get::<RelayCtx>()
        .map(|ctx| ctx.access.clone())
    else {
        return HoldRejection::MissingAccessContext.into_response();
    };

    let (balance, models) = tokio::join!(
        state.hold.ledger().available_balance(meta.user_id),
        state
            .hold
            .usage_store()
            .model_usage_since(meta.user_id, local_today_start()),
    );
    let (balance_usd, models) = match (balance, models) {
        (Ok(balance), Ok(models)) => (balance, models),
        (Err(err), _) => {
            tracing::warn!(%err, user_id = meta.user_id, "usage ledger lookup failed");
            return DispatchError::Internal(err.into()).into_response();
        }
        (_, Err(err)) => {
            tracing::warn!(%err, user_id = meta.user_id, "usage model lookup failed");
            return DispatchError::Internal(err).into_response();
        }
    };

    axum::Json(serde_json::json!({
        "object": "usage",
        "balance_usd": balance_usd,
        "models": models.iter().map(|row| serde_json::json!({
            "model": row.model,
            "requests": row.requests,
            "tokens_in": row.tokens_in,
            "tokens_out": row.tokens_out,
            "tokens": row.tokens(),
        })).collect::<Vec<_>>(),
    }))
    .into_response()
}

/// 当地今日零点，与面板 `/user/usage/stats` 的 `today` 窗口同一口径。
fn local_today_start() -> chrono::DateTime<chrono::Utc> {
    use chrono::{Local, TimeZone as _};
    let day = Local::now().date_naive();
    match Local.from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap_or_default()) {
        chrono::LocalResult::Single(at) => at.to_utc(),
        chrono::LocalResult::Ambiguous(earliest, _) => earliest.to_utc(),
        chrono::LocalResult::None => chrono::Utc::now(),
    }
}

/// `GET /v1/models/{model}` — one catalogue entry.
pub async fn model_detail(State(state): State<ProxyState>, Path(model): Path<String>) -> Response {
    let Some(catalog) = &state.dispatch.catalog else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match catalog.get_model(&model).await {
        Ok(Some(entry)) => axum::Json(model_json(&entry)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
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
    if let Some(reasoning) = &model.reasoning
        && !reasoning.efforts.is_empty()
    {
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
    value
}
