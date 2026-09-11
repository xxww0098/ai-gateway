//! Policy and catalogue projection.

use super::*;
use crate::testsupport::{FakeCatalog, fresh_db};

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn policies_are_returned_verbatim_and_absent_accounts_are_simply_absent() {
    // Accounts without a row are the common case; defaulting them is the
    // cache's job, so this must not invent rows for them.
    let pool = fresh_db("catalog_policies").await;
    sqlx::query(
        "INSERT INTO channel_policies (auth_id, weight, priority, enabled, created_at, updated_at) \
         VALUES ('acct-1', 5, 2, TRUE, NOW(), NOW()), \
                ('acct-2', 1, 0, FALSE, NOW(), NOW())",
    )
    .execute(&pool)
    .await
    .expect("seeding policies");

    let mut policies = SqlChannelPolicyStore::new(pool.clone())
        .list_channel_policies()
        .await
        .expect("listing policies");
    policies.sort_by(|a, b| a.auth_id.cmp(&b.auth_id));

    assert_eq!(policies.len(), 2);
    assert_eq!(policies[0].weight, 5);
    assert_eq!(policies[0].priority, 2);
    assert!(policies[0].enabled);
    assert!(!policies[1].enabled);
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn an_empty_policy_table_is_not_an_error() {
    let pool = fresh_db("catalog_policies_empty").await;
    assert!(
        SqlChannelPolicyStore::new(pool)
            .list_channel_policies()
            .await
            .expect("listing")
            .is_empty(),
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn the_catalogue_hides_invisible_models_and_deduplicates_by_id() {
    let pool = fresh_db("catalog_models").await;
    sqlx::query(
        "INSERT INTO model_catalog_entries (channel_key, model_id, visible, models_url, created_at, updated_at) \
         VALUES ('openai', 'gpt-4o', TRUE, '', NOW(), NOW()), \
                ('azure',  'gpt-4o', TRUE, '', NOW(), NOW()), \
                ('openai', 'secret-preview', FALSE, '', NOW(), NOW())",
    )
    .execute(&pool)
    .await
    .expect("seeding catalogue entries");

    let catalog = SqlModelCatalog::new(pool);
    let models = catalog.list_models().await.expect("listing models");

    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        ["gpt-4o"],
        "a model served by two channels is one model to the client, and an \
         invisible one is not a model at all",
    );
    assert_eq!(models[0].owned_by, "azure", "ordered by channel_key");

    let found = catalog.get_model("gpt-4o").await.expect("lookup");
    assert_eq!(
        found.as_ref().map(|m| m.owned_by.as_str()),
        Some("azure"),
        "detail must pick the same channel as the listing",
    );
    assert!(
        catalog
            .get_model("secret-preview")
            .await
            .expect("hidden lookup")
            .is_none(),
        "an invisible model is not a catalogue entry",
    );
    assert!(
        catalog
            .get_model("does-not-exist")
            .await
            .expect("missing lookup")
            .is_none(),
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn the_sentinel_row_is_never_a_model() {
    // 面板把「上游模型列表 URL」也存进这张表，用一个保留的 model_id 占位，
    // 而它自己的查询显式排除了那一行。收敛前 `list_models` **没有**排除，
    // 只靠「哨兵行是以 visible = false 写入的」挡住 —— 只要有人通过面板
    // 把那行翻成 visible，它就会作为一个模型出现在 `GET /v1/models` 里。
    let pool = fresh_db("catalog_sentinel").await;
    sqlx::query(
        "INSERT INTO model_catalog_entries (channel_key, model_id, visible, models_url, created_at, updated_at) \
         VALUES ('openai', $1, TRUE, 'https://example.invalid/models', NOW(), NOW()), \
                ('openai', 'gpt-4o', TRUE, '', NOW(), NOW())",
    )
    .bind(MODELS_URL_SENTINEL)
    .execute(&pool)
    .await
    .expect("seeding catalogue entries");

    let catalog = SqlModelCatalog::new(pool);
    let ids: Vec<String> = catalog
        .list_models()
        .await
        .expect("listing models")
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids, ["gpt-4o"]);
    assert!(
        catalog
            .get_model(MODELS_URL_SENTINEL)
            .await
            .expect("sentinel lookup")
            .is_none(),
        "detail must not surface the reserved row either",
    );
}

// ---------------------------------------------------------------- 快照缓存

#[tokio::test]
async fn a_warm_snapshot_answers_list_and_detail_without_rereading() {
    let inner = FakeCatalog::with(vec![ModelEntry {
        id: "gpt-4o".into(),
        owned_by: "openai".into(),
        ..ModelEntry::default()
    }]);
    let cache = CachedModelCatalog::new(inner.clone());
    cache.refresh().await.expect("refresh");
    let listed_after_refresh = inner.list_calls();

    let listed = cache.list_models().await.expect("list");
    let found = cache.get_model("gpt-4o").await.expect("get");
    assert_eq!(listed.len(), 1);
    assert_eq!(found.as_ref().map(|m| m.id.as_str()), Some("gpt-4o"));
    assert_eq!(
        inner.list_calls(),
        listed_after_refresh,
        "a ready snapshot must not walk the inner listing again",
    );
    assert!(
        cache.get_model("missing").await.expect("miss").is_none(),
        "an id absent from the snapshot is not a catalogue entry",
    );
}

#[tokio::test]
async fn the_first_read_loads_the_snapshot_once() {
    let inner = FakeCatalog::with(vec![ModelEntry {
        id: "only".into(),
        ..ModelEntry::default()
    }]);
    let cache = CachedModelCatalog::new(inner.clone());
    assert_eq!(inner.list_calls(), 0);

    let first = cache.get_model("only").await.expect("first get");
    let second = cache.list_models().await.expect("list");
    assert_eq!(first.as_ref().map(|m| m.id.as_str()), Some("only"));
    assert_eq!(second.len(), 1);
    assert_eq!(
        inner.list_calls(),
        1,
        "list and detail share the load that warms the snapshot",
    );
}

#[tokio::test]
async fn a_failed_refresh_keeps_the_previous_snapshot() {
    let inner = FakeCatalog::with(vec![ModelEntry {
        id: "kept".into(),
        ..ModelEntry::default()
    }]);
    let cache = CachedModelCatalog::new(inner.clone());
    cache.refresh().await.expect("warm");
    inner
        .fail_list
        .store(true, std::sync::atomic::Ordering::SeqCst);

    assert!(cache.refresh().await.is_err());
    assert_eq!(
        cache
            .get_model("kept")
            .await
            .expect("stale get")
            .map(|m| m.id)
            .as_deref(),
        Some("kept"),
        "a failed refresh must not wipe a good snapshot",
    );
}

#[test]
fn vision_capabilities_keep_image_and_text_and_drop_unknown_modalities() {
    let mut entry = ModelEntry::default();
    apply_capabilities(
        &mut entry,
        Some(&serde_json::json!({
            "context_length": 128000,
            "max_output_tokens": 16384,
            "input_modalities": ["text", "image", "audio"],
            "reasoning": { "efforts": [] }
        })),
    );
    assert_eq!(entry.input_modalities, ["text", "image"]);
    assert!(
        entry.reasoning.is_none(),
        "an empty effort list means the model has no thinking"
    );
    assert_eq!(entry.context_length, Some(128000));
    assert_eq!(entry.max_output_tokens, Some(16384));
}

#[test]
fn text_only_capabilities_do_not_advertise_image() {
    let mut entry = ModelEntry::default();
    apply_capabilities(
        &mut entry,
        Some(&serde_json::json!({
            "context_length": 8192,
            "max_output_tokens": 2048,
            "input_modalities": ["text"]
        })),
    );
    assert_eq!(entry.input_modalities, ["text"]);
    assert!(!entry.input_modalities.iter().any(|m| m == "image"));
    assert!(entry.reasoning.is_none());
}

#[test]
fn reasoning_is_copied_from_the_catalog_and_not_invented() {
    let mut thinking = ModelEntry::default();
    apply_capabilities(
        &mut thinking,
        Some(&serde_json::json!({
            "reasoning": {
                "efforts": [
                    {"id": "low", "name": "Low"},
                    {"id": "high", "name": "High"}
                ],
                "default_effort": "high"
            }
        })),
    );
    let reasoning = thinking
        .reasoning
        .expect("thinking model must expose efforts");
    assert_eq!(
        reasoning
            .efforts
            .iter()
            .map(|e| e.id.as_str())
            .collect::<Vec<_>>(),
        ["low", "high"]
    );
    assert_eq!(reasoning.default_effort.as_deref(), Some("high"));

    let mut plain = ModelEntry::default();
    apply_capabilities(&mut plain, Some(&serde_json::json!({})));
    assert!(plain.reasoning.is_none());
}

#[test]
fn missing_capabilities_leave_limits_unset_instead_of_guessing() {
    let mut entry = ModelEntry {
        id: "plain".into(),
        ..ModelEntry::default()
    };
    apply_capabilities(&mut entry, None);
    assert_eq!(entry.context_length, None);
    assert_eq!(entry.max_output_tokens, None);
    assert!(entry.input_modalities.is_empty());
    assert!(entry.reasoning.is_none());
}
