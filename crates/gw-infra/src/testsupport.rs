//! Helpers shared by the Redis-backed module tests.
//!
//! `#[cfg(test)]`-gated at the `mod` declaration (rule 2.7), so none of this
//! reaches the production crate or its dependents.

use crate::Redis;

/// Database the Redis-backed tests work in. Deliberately not 0: the suite
/// writes real keys, and nobody's development data should be next to them.
pub(crate) const TEST_REDIS_DB: i64 = 15;

/// Where the Redis-backed tests look for a server.
pub(crate) fn test_redis_addr() -> String {
    std::env::var("REDIS_TEST_ADDR").unwrap_or_else(|_| "127.0.0.1:6379".to_owned())
}

/// Connects, or fails loudly with the fix (rule 2.9).
///
/// Every caller is `#[ignore]`d, so getting here means somebody explicitly ran
/// `cargo test -p gw-infra -- --ignored`: a missing server is a failure to
/// report, never a test that quietly passes.
pub(crate) async fn test_redis() -> Redis {
    let addr = test_redis_addr();
    match crate::init_redis(&addr, "", TEST_REDIS_DB).await {
        Some(conn) => conn,
        None => panic!(
            "Redis 未就绪：请在 {addr} 启动 redis-server（或设置 REDIS_TEST_ADDR），\
             测试只使用 db {TEST_REDIS_DB}"
        ),
    }
}

/// A subject name (rate-limit identity, provider, …) that no other test — and
/// no earlier run against the same server — can be using.
///
/// This is how the suite stays safe on a shared Redis: it never clears keys, it
/// just never reuses a name.
pub(crate) fn unique_name(prefix: &str) -> String {
    format!("gwtest:{prefix}:{}", crate::rate_limiter::new_request_id())
}
