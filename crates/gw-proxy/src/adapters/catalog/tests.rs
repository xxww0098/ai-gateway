//! Policy and catalogue projection.

use super::*;
use crate::testsupport::fresh_db;

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

    let models = SqlModelCatalog::new(pool)
        .list_models()
        .await
        .expect("listing models");

    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        ["gpt-4o"],
        "a model served by two channels is one model to the client, and an \
         invisible one is not a model at all",
    );
    assert_eq!(models[0].owned_by, "azure", "ordered by channel_key");
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
            .resolve_channels(MODELS_URL_SENTINEL)
            .await
            .expect("resolving")
            .is_empty(),
        "路由查询也必须排除它，否则它会被当成一个可路由的模型",
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn routing_sees_models_the_catalogue_listing_hides() {
    // `visible` 是「对租户展示」开关，不是「允许调用」开关 —— 一个
    // visible = false 的模型今天照样能被调用。路由查询若继承了
    // `WHERE visible = TRUE`，会静默地把所有隐藏模型变成不可调用，
    // 表现为「某些模型突然 503」，极难归因。
    let pool = fresh_db("catalog_routing_visibility").await;
    sqlx::query(
        "INSERT INTO model_catalog_entries (channel_key, model_id, visible, models_url, created_at, updated_at) \
         VALUES ('house-a', 'hidden-model', FALSE, '', NOW(), NOW()), \
                ('house-b', 'hidden-model', FALSE, '', NOW(), NOW())",
    )
    .execute(&pool)
    .await
    .expect("seeding catalogue entries");

    let catalog = SqlModelCatalog::new(pool);
    assert!(
        catalog.list_models().await.expect("listing").is_empty(),
        "隐藏的模型不该出现在目录里",
    );
    assert_eq!(
        catalog
            .resolve_channels("hidden-model")
            .await
            .expect("resolving"),
        ["house-a", "house-b"],
        "但它必须仍然可路由，而且多渠道要按顺序全部返回",
    );
}

// ---------------------------------------------------------------- 快照解析器

/// 一个不碰数据库的目录，用来测快照与映射逻辑本身。
struct StubCatalog(Vec<(String, Vec<String>)>);

#[async_trait]
impl ModelCatalog for StubCatalog {
    async fn list_models(&self) -> anyhow::Result<Vec<ModelEntry>> {
        Ok(Vec::new())
    }

    async fn model_routes(&self) -> anyhow::Result<Vec<(String, Vec<String>)>> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn an_unrefreshed_resolver_says_nothing_so_the_chain_falls_back() {
    // 从未刷新过的快照是空的 —— 这不是退化，是安全灰度的默认态：
    // 四级链的 L2 全部落空、直接落 L4，行为与收敛前逐字节相同。
    let resolver = CatalogChannelResolver::new(Arc::new(StubCatalog(vec![(
        "some-model".to_owned(),
        vec!["some-channel".to_owned()],
    )])));

    assert!(resolver.snapshot_age().is_none());
    assert!(resolver.channels_for_model("some-model").is_empty());

    resolver.refresh().await.expect("refresh");
    assert!(resolver.snapshot_age().is_some());
    assert_eq!(resolver.channels_for_model("some-model"), ["some-channel"]);
}

#[tokio::test]
async fn l1_works_before_any_refresh_because_the_channel_map_is_static() {
    // 显式渠道前缀只查 `provider_for_channel`，那张表不依赖快照，
    // 所以 `<channel>/<model>` 这类写法从装上 resolver 的第一秒就生效。
    let resolver = CatalogChannelResolver::new(Arc::new(StubCatalog(Vec::new())))
        .with_channel("house", Provider::Vertex);

    assert_eq!(
        resolver.provider_for_channel("house"),
        Some(Provider::Vertex)
    );
    assert_eq!(
        resolver.provider_for_channel("a-channel-nobody-configured"),
        None,
        "没有显式映射时必须说不知道，由调用方落通配 executor，而不是在这里猜",
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
    let reasoning = thinking.reasoning.expect("thinking model must expose efforts");
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
