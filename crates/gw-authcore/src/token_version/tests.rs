use super::{TokenVersionStore, token_revoked};
use crate::jwt::{generate_jwt_with_version, validate_jwt};

/// The environment variable an operator sets to run the ignored Postgres tests.
const DB_URL_ENV: &str = "GW_TEST_DATABASE_URL";

#[test]
fn a_bump_invalidates_exactly_the_tokens_minted_before_it() {
    // The property, not the comparison: for any epoch, tokens minted at or
    // after it survive and every older one dies.
    for current in [0_i64, 1, 2, 7, 1_000] {
        for minted in 0..=current {
            assert_eq!(
                token_revoked(minted, current),
                minted < current,
                "token minted at {minted} against current epoch {current}"
            );
        }
        assert!(
            !token_revoked(current + 1, current),
            "a token minted after the bump must survive"
        );
    }
}

#[test]
fn a_user_who_never_revoked_keeps_every_token() {
    assert!(!token_revoked(0, 0), "absence of a row means epoch 0");
}

#[test]
fn revocation_is_carried_end_to_end_by_the_token_itself() {
    let token = generate_jwt_with_version(42, "a@b.c", "s3cret", 1, 4).expect("issuing succeeds");
    let claims = validate_jwt(&token, "s3cret").expect("the token is well-formed");

    // Signature stays valid across a bump — only the epoch comparison rejects it.
    assert!(!token_revoked(claims.token_version, 4));
    assert!(token_revoked(claims.token_version, 5));
    assert!(claims.is_revoked(5));
}

// ---------------------------------------------------------------------------
// Postgres-backed tests; see the note in `store::tests`.
//   GW_TEST_DATABASE_URL=postgres://… cargo test -p gw-authcore -- --ignored
// ---------------------------------------------------------------------------

async fn test_pool() -> sqlx::PgPool {
    let url = std::env::var(DB_URL_ENV).unwrap_or_else(|_| {
        panic!(
            "{DB_URL_ENV} is not set. These tests are --ignored by default; to run them, point \
             {DB_URL_ENV} at a Postgres database with the gateway migrations applied."
        )
    });
    sqlx::PgPool::connect(&url)
        .await
        .unwrap_or_else(|err| panic!("connecting to {DB_URL_ENV}: {err}"))
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn the_epoch_starts_absent_climbs_monotonically_and_reads_back() {
    let pool = test_pool().await;
    let store = TokenVersionStore::new(pool.clone());
    // A user id far outside the sequence, so this test owns its row.
    let user_id = 2_000_000_000 + i64::from(std::process::id() % 1_000_000);

    sqlx::query("DELETE FROM user_token_versions WHERE user_id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("clearing any leftover row");

    assert_eq!(
        store.current(user_id).await.expect("reading succeeds"),
        0,
        "a user with no row has never revoked"
    );

    let first = store.bump(user_id).await.expect("bumping succeeds");
    assert_eq!(first, 1, "the first logout-everywhere lands at 1");
    assert_eq!(
        store.current(user_id).await.expect("reading succeeds"),
        first
    );

    let second = store.bump(user_id).await.expect("bumping succeeds");
    assert!(second > first, "every bump must strictly advance the epoch");
    assert_eq!(
        store.current(user_id).await.expect("reading succeeds"),
        second
    );

    sqlx::query("DELETE FROM user_token_versions WHERE user_id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("cleaning up");
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn an_anonymous_request_never_touches_the_table() {
    let store = TokenVersionStore::new(test_pool().await);

    assert_eq!(store.current(0).await.expect("reading succeeds"), 0);
    assert_eq!(store.bump(0).await.expect("bumping succeeds"), 0);
}
