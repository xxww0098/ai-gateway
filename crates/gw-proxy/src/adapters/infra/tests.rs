//! Fail-open behaviour, and the Redis round trip behind `#[ignore]`.

use super::*;

/// A limiter with no Redis (the "nil client" configuration).
fn limiter_without_redis() -> SharedRateLimiter {
    SharedRateLimiter::new(Arc::new(InfraLimiter::new(
        None,
        gw_infra::RateLimitSettings::default(),
    )))
}

fn breaker_without_redis() -> SharedCircuitBreaker {
    SharedCircuitBreaker::new(Arc::new(InfraBreaker::new(
        None,
        gw_infra::CircuitBreakerSettings::default(),
    )))
}

#[tokio::test]
async fn a_limiter_without_redis_admits_traffic_instead_of_stopping_it() {
    // Fail-open is the deliberate posture: a rate limiter outage must not take
    // the proxy down with it.
    let (allowed, release_id) = limiter_without_redis()
        .allow("7", 1, "gpt-4o", None)
        .await
        .expect("failing open is not an error");
    assert!(allowed);
    assert!(
        release_id.is_none(),
        "nothing was reserved, so there is nothing to release",
    );
}

#[tokio::test]
async fn releasing_a_slot_that_was_never_taken_is_harmless() {
    limiter_without_redis()
        .release_concurrency("7", "")
        .await
        .expect("an empty release id is the fail-open case, not an error");
}

#[tokio::test]
async fn a_breaker_without_redis_admits_traffic() {
    assert!(
        breaker_without_redis()
            .allow("openai")
            .await
            .expect("failing open is not an error"),
    );
}

#[tokio::test]
async fn recording_into_a_breaker_without_redis_is_swallowed() {
    // A breaker that cannot record is a degraded signal, never a reason to fail
    // a request that already succeeded — `record` has no error channel at all.
    let breaker = breaker_without_redis();
    breaker.record("openai", true).await;
    breaker.record("openai", false).await;
}

/// Connects to the Redis the workspace's other integration tests use.
async fn test_redis() -> Redis {
    let addr = std::env::var("REDIS_TEST_ADDR").unwrap_or_else(|_| "127.0.0.1:6379".to_owned());
    let client = redis::Client::open(format!("redis://{addr}/15"))
        .expect("REDIS_TEST_ADDR is not a usable Redis address");
    redis::aio::ConnectionManager::new(client)
        .await
        .expect("cannot reach Redis; start one or unset the test")
}

#[tokio::test]
#[ignore = "需要本地 Redis（REDIS_TEST_ADDR，默认 127.0.0.1:6379 的 db 15）"]
async fn an_entry_round_trips_and_expires_on_its_own() {
    let store = RedisIdempotencyStore::new(test_redis().await);
    let key = "gw-proxy-test:roundtrip";
    store.delete(key).await.expect("clean slate");

    assert_eq!(store.get(key).await.expect("get"), None);
    store
        .set(key, b"payload".to_vec(), Duration::from_secs(30))
        .await
        .expect("set");
    assert_eq!(
        store.get(key).await.expect("get"),
        Some(b"payload".to_vec())
    );

    store.delete(key).await.expect("delete");
    assert_eq!(store.get(key).await.expect("get"), None);
}

#[tokio::test]
#[ignore = "需要本地 Redis（REDIS_TEST_ADDR，默认 127.0.0.1:6379 的 db 15）"]
async fn only_the_first_claimant_wins_the_key() {
    // This is the whole point of the claim: a concurrent duplicate must lose,
    // or both requests bill.
    let store = RedisIdempotencyStore::new(test_redis().await);
    let key = "gw-proxy-test:claim";
    store.delete(key).await.expect("clean slate");

    assert!(
        store
            .set_nx(key, b"first".to_vec(), Duration::from_secs(30))
            .await
            .expect("set_nx"),
    );
    assert!(
        !store
            .set_nx(key, b"second".to_vec(), Duration::from_secs(30))
            .await
            .expect("set_nx"),
    );
    assert_eq!(
        store.get(key).await.expect("get"),
        Some(b"first".to_vec()),
        "the loser must not overwrite the winner's entry",
    );

    store.delete(key).await.expect("cleanup");
}
