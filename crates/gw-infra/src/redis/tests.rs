use std::time::Instant;

use super::*;

/// The address forms operators actually put in `config.yaml`, plus the bare
/// host and empty host conveniences.
#[test]
fn addresses_split_into_host_and_port() {
    let cases = [
        ("127.0.0.1:6379", "127.0.0.1", 6379),
        ("redis.internal:6380", "redis.internal", 6380),
        ("redis", "redis", DEFAULT_PORT),
        (" 127.0.0.1:6379 ", "127.0.0.1", 6379),
        (":6379", "127.0.0.1", 6379),
        ("[::1]:6379", "::1", 6379),
        ("[fe80::1]", "fe80::1", DEFAULT_PORT),
        ("::1", "::1", DEFAULT_PORT),
    ];
    for (addr, host, port) in cases {
        let parsed = parse_addr(addr).unwrap_or_else(|err| panic!("{addr:?}: {err}"));
        assert_eq!(parsed, (host.to_owned(), port), "parsing {addr:?}");
    }
}

/// A malformed address must be reported, not silently turned into a connection
/// to some other server.
#[test]
fn malformed_addresses_are_rejected() {
    for addr in [
        "",
        "   ",
        "host:",
        "host:0",
        "host:65536",
        "host:redis",
        "[::1",
    ] {
        assert!(
            parse_addr(addr).is_err(),
            "{addr:?} should not parse into a connection address"
        );
    }
}

/// An empty `redis.addr` disables Redis without any connection attempt — the
/// deployment knob that lets the gateway run Redis-less.
#[tokio::test]
async fn empty_address_disables_redis_immediately() {
    let started = Instant::now();
    assert!(init_redis("", "", 0).await.is_none());
    assert!(init_redis("   ", "", 0).await.is_none());
    assert!(
        started.elapsed() < PING_TIMEOUT,
        "an empty address must short-circuit before the ping budget"
    );
}

/// An unreachable server is fail-soft: `None` within the ping budget rather
/// than an error propagated into startup.
#[tokio::test]
async fn unreachable_server_fails_soft_within_the_ping_budget() {
    let started = Instant::now();
    // Port 1 (tcpmux) is a reserved port nothing in this repo binds.
    let conn = init_redis("127.0.0.1:1", "", 0).await;
    assert!(
        conn.is_none(),
        "an unreachable Redis must not yield a handle"
    );
    assert!(
        started.elapsed() < PING_TIMEOUT * 2,
        "fail-soft took {:?}, well past the {PING_TIMEOUT:?} budget",
        started.elapsed()
    );
}

/// The happy path, against a real server. Ignored because it needs one.
#[tokio::test]
#[ignore = "需要本地 Redis（REDIS_TEST_ADDR，默认 127.0.0.1:6379）"]
async fn init_redis_pings_a_live_server() {
    let addr = crate::testsupport::test_redis_addr();
    let conn = init_redis(&addr, "", crate::testsupport::TEST_REDIS_DB).await;
    assert!(
        conn.is_some(),
        "Redis 未就绪：请在 {addr} 启动 redis-server（或设置 REDIS_TEST_ADDR）"
    );
}
