//! Unit tests for the model-catalog request shapes.
//!
//! The four handlers are SQL-shaped and covered by the integration suite; what
//! is pinned here is the wire contract the frontend depends on
//! (`features/admin-proxy/components/OpenAiEditDialogBody.tsx` and
//! `pages/admin/proxy/AdminProxyProvidersPage.tsx`) and the sentinel that keeps
//! a channel's URL row out of the model list.

use super::*;

#[test]
fn the_models_url_sentinel_cannot_collide_with_a_real_model() {
    // It shares a unique index with real model ids, so a provider that ever
    // shipped a model by this name would overwrite a channel's URL row. The
    // double-underscore convention is what keeps that from happening.
    assert!(MODELS_URL_MODEL_ID.starts_with("__"));
    assert!(MODELS_URL_MODEL_ID.ends_with("__"));
}

#[test]
fn the_models_url_request_matches_the_frontend_payload() {
    // `AdminProxyProvidersPage.tsx` PUTs exactly this.
    let req: ModelsUrlRequest =
        serde_json::from_str(r#"{"channel_key":"openai","models_url":"https://x/v1/models"}"#)
            .expect("frontend payload must deserialize");
    assert_eq!(req.channel_key, "openai");
    assert_eq!(req.models_url, "https://x/v1/models");
}

#[test]
fn an_omitted_models_url_deserializes_rather_than_failing() {
    // 旧实现把请求绑定到字符串结构体，缺失键即为 ""，请求依然
    // is still valid — it just becomes the no-op update the handler documents.
    let req: ModelsUrlRequest =
        serde_json::from_str(r#"{"channel_key":"openai"}"#).expect("partial payload");
    assert!(req.models_url.is_empty());
}

#[test]
fn ensure_channel_accepts_the_frontend_payload() {
    // `OpenAiEditDialogBody.tsx` POSTs `{channel_key, model_ids}`.
    let req: EnsureChannelRequest =
        serde_json::from_str(r#"{"channel_key":"openai","model_ids":["gpt-4o","o3-mini"]}"#)
            .expect("frontend payload must deserialize");
    assert_eq!(req.model_ids.len(), 2);
}

#[test]
fn ensure_channel_tolerates_a_missing_model_list() {
    let req: EnsureChannelRequest =
        serde_json::from_str(r#"{"channel_key":"openai"}"#).expect("partial payload");
    assert!(req.model_ids.is_empty());
}

#[test]
fn visibility_defaults_to_hidden_when_omitted() {
    // `bool` 在旧实现中的零值同样是 false，所以省略 `visible` 会隐藏
    // model rather than showing it. Pinned because the safer default is also
    // the surprising one.
    let req: VisibilityRequest =
        serde_json::from_str(r#"{"channel_key":"openai","model_id":"gpt-4o"}"#)
            .expect("partial payload");
    assert!(!req.visible);
}

#[test]
fn visibility_round_trips_both_states() {
    for (raw, want) in [("true", true), ("false", false)] {
        let req: VisibilityRequest = serde_json::from_str(&format!(
            r#"{{"channel_key":"openai","model_id":"m","visible":{raw}}}"#
        ))
        .expect("payload");
        assert_eq!(req.visible, want);
    }
}
