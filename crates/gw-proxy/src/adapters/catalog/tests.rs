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
