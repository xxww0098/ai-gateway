use super::*;
use crate::testsupport;

fn hash(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// A zero in the config means "unset", so the breaker must end up with usable
/// thresholds instead of a 0.0 threshold that would trip on the first failure.
#[test]
fn zero_valued_settings_resolve_to_the_defaults() {
    let zeroed = CircuitBreakerSettings {
        failure_threshold: 0.0,
        window_seconds: 0,
        cooldown_seconds: 0,
    };
    assert_eq!(zeroed.with_defaults(), CircuitBreakerSettings::default());

    let negative = CircuitBreakerSettings {
        failure_threshold: -1.0,
        window_seconds: -30,
        cooldown_seconds: -30,
    };
    assert_eq!(negative.with_defaults(), CircuitBreakerSettings::default());

    let configured = CircuitBreakerSettings {
        failure_threshold: 0.25,
        window_seconds: 10,
        cooldown_seconds: 5,
    };
    assert_eq!(configured.with_defaults(), configured);
}

/// The counters must outlive the cooldown, or an open breaker would forget it
/// was ever opened and start admitting traffic early.
#[test]
fn state_outlives_its_own_cooldown() {
    let breaker = CircuitBreaker::new(
        None,
        CircuitBreakerSettings {
            failure_threshold: 0.5,
            window_seconds: 7,
            cooldown_seconds: 11,
        },
    );
    let ttl = breaker.ttl_seconds();
    assert!(
        ttl > i64::try_from(breaker.cooldown.as_secs()).unwrap(),
        "ttl {ttl}s does not outlast the {:?} cooldown",
        breaker.cooldown
    );
}

/// States round-trip through their stored spelling, and an unreadable value is
/// closed — never "stuck open".
#[test]
fn states_round_trip_and_unknown_reads_as_closed() {
    for state in [
        CircuitState::Closed,
        CircuitState::Open,
        CircuitState::HalfOpen,
    ] {
        assert_eq!(CircuitState::from_wire(state.as_str()), state);
    }
    assert_eq!(CircuitState::from_wire(""), CircuitState::Closed);
    assert_eq!(CircuitState::from_wire("tripped"), CircuitState::Closed);
    assert_eq!(CircuitState::default(), CircuitState::Closed);
}

/// A provider with no stored state, a closed one, and a half-open one all let
/// requests through.
#[test]
fn healthy_and_probing_states_allow() {
    let cooldown = Duration::from_secs(30);
    let now = 1_000;

    assert_eq!(
        allow_outcome(&hash(&[]), now, cooldown),
        AllowOutcome::Allow
    );
    assert_eq!(
        allow_outcome(&hash(&[("state", "closed")]), now, cooldown),
        AllowOutcome::Allow
    );
    assert_eq!(
        allow_outcome(&hash(&[("state", "half_open")]), now, cooldown),
        AllowOutcome::Allow
    );
    assert_eq!(
        allow_outcome(&hash(&[("failures", "3")]), now, cooldown),
        AllowOutcome::Allow,
        "counters without a state are not an open circuit"
    );
}

/// The cooldown boundary: rejected right up to it, probing from it onwards.
#[test]
fn open_rejects_until_the_cooldown_elapses() {
    let cooldown = Duration::from_secs(30);
    let opened_at = 1_000_i64;
    let cooldown_secs = i64::try_from(cooldown.as_secs()).unwrap();
    let fields = hash(&[("state", "open"), ("opened_at", &opened_at.to_string())]);

    for elapsed in [0, 1, cooldown_secs - 1] {
        assert_eq!(
            allow_outcome(&fields, opened_at + elapsed, cooldown),
            AllowOutcome::Reject,
            "{elapsed}s into a {cooldown:?} cooldown the circuit must stay shut"
        );
    }
    for elapsed in [cooldown_secs, cooldown_secs + 1, cooldown_secs * 100] {
        assert_eq!(
            allow_outcome(&fields, opened_at + elapsed, cooldown),
            AllowOutcome::ProbeAfterCooldown,
            "{elapsed}s into a {cooldown:?} cooldown a probe must be allowed"
        );
    }
}

/// A backwards clock (opened_at in the future) must not read as "cooldown
/// elapsed" — that would turn every skewed node into a probe generator.
#[test]
fn an_opened_at_in_the_future_still_rejects() {
    let fields = hash(&[("state", "open"), ("opened_at", "2000")]);
    assert_eq!(
        allow_outcome(&fields, 1_000, Duration::from_secs(30)),
        AllowOutcome::Reject
    );
}

/// An `opened_at` that cannot be read leaves the cooldown unevaluable, so the
/// breaker resets instead of rejecting forever.
#[test]
fn open_without_a_usable_timestamp_resets() {
    let cooldown = Duration::from_secs(30);
    for opened_at in ["", "not-a-timestamp", "12.5"] {
        assert_eq!(
            allow_outcome(
                &hash(&[("state", "open"), ("opened_at", opened_at)]),
                1_000,
                cooldown
            ),
            AllowOutcome::ResetThenAllow,
            "opened_at={opened_at:?} must clear the state"
        );
    }
    assert_eq!(
        allow_outcome(&hash(&[("state", "open")]), 1_000, cooldown),
        AllowOutcome::ResetThenAllow,
        "an open circuit with no opened_at at all must clear the state"
    );
}

/// Below the minimum sample size the breaker must not trip, however bad the
/// sample looks — one failed request is not an outage.
#[test]
fn a_small_sample_never_trips() {
    for total in 1..MIN_SAMPLES {
        assert!(
            !should_trip(total, total, 0.01),
            "{total} total observations are too few to trip on"
        );
    }
    assert!(
        should_trip(MIN_SAMPLES, MIN_SAMPLES, 0.01),
        "the minimum sample size must be usable, not one short"
    );
}

/// The threshold is inclusive, and exactly one failure below it is not enough.
#[test]
fn the_threshold_is_inclusive() {
    // 5 of 10 failures is exactly a 0.5 rate.
    assert!(should_trip(5, 10, 0.5));
    assert!(!should_trip(4, 10, 0.5));
    assert!(!should_trip(0, 10, 0.5));
    assert!(should_trip(10, 10, 1.0), "a total outage must trip");
}

/// More failures out of the same sample can never un-trip the breaker.
#[test]
fn tripping_is_monotone_in_the_failure_count() {
    for total in MIN_SAMPLES..=20 {
        for threshold_pct in (30..=80).step_by(5) {
            let threshold = f64::from(threshold_pct) / 100.0;
            let mut tripped = false;
            for failures in 0..=total {
                let now = should_trip(failures, total, threshold);
                assert!(
                    now || !tripped,
                    "{failures}/{total} at {threshold} un-tripped a tripped breaker"
                );
                tripped |= now;
            }
            assert!(
                tripped,
                "no failure count at all trips {total} samples at {threshold}"
            );
        }
    }
}

/// The smallest failure count that exceeds the threshold must trip.
#[test]
fn the_first_failure_count_past_the_threshold_trips() {
    for total in MIN_SAMPLES..=20 {
        for threshold_pct in 30..=80 {
            let threshold = f64::from(threshold_pct) / 100.0;
            #[expect(
                clippy::cast_possible_truncation,
                reason = "matches int(f64(total) * threshold) + 1"
            )]
            let min_failures = ((total as f64 * threshold) as i64 + 1).min(total);
            assert!(
                should_trip(min_failures, total, threshold),
                "{min_failures}/{total} failures did not trip at {threshold}"
            );
        }
    }
}

/// Without Redis the breaker is inert: it never rejects and never errors.
#[tokio::test]
async fn without_redis_the_breaker_is_inert() {
    let breaker = CircuitBreaker::new(None, CircuitBreakerSettings::default());
    assert!(breaker.allow("openai").await);
    breaker.record_failure("openai").await.unwrap();
    breaker.record_success("openai").await.unwrap();
    assert_eq!(breaker.state("openai").await.unwrap(), CircuitState::Closed);
    assert!(breaker.allow("openai").await);
}

// ---------------------------------------------------------------------------
// Redis-backed behaviour.
//
//   cargo test -p gw-infra -- --ignored
//
// Needs a reachable Redis (REDIS_TEST_ADDR, default 127.0.0.1:6379, db 15).
// Each test uses a fresh provider name so the suite never clears shared keys.
// ---------------------------------------------------------------------------

/// Backdates `opened_at` so the cooldown reads as elapsed.
async fn backdate_opened_at(redis: &Redis, provider: &str, seconds_ago: i64) {
    let mut conn = redis.clone();
    let _: () = ::redis::AsyncCommands::hset(
        &mut conn,
        circuit_key(provider),
        "opened_at",
        Utc::now().timestamp() - seconds_ago,
    )
    .await
    .expect("backdating opened_at");
}

/// Property 21a: a failure rate past the threshold opens the circuit.
#[tokio::test]
#[ignore = "需要本地 Redis（REDIS_TEST_ADDR，默认 127.0.0.1:6379 的 db 15）"]
async fn a_failure_rate_past_the_threshold_opens_the_circuit() {
    let redis = testsupport::test_redis().await;
    let breaker = CircuitBreaker::new(
        Some(redis),
        CircuitBreakerSettings {
            failure_threshold: 0.5,
            window_seconds: 30,
            cooldown_seconds: 30,
        },
    );
    let provider = testsupport::unique_name("cb-open");

    // 2 successes then 3 failures: 3/5 is past a 0.5 threshold.
    for _ in 0..2 {
        breaker.record_success(&provider).await.unwrap();
    }
    assert_eq!(
        breaker.state(&provider).await.unwrap(),
        CircuitState::Closed,
        "successes alone must not open the circuit"
    );
    for _ in 0..3 {
        breaker.record_failure(&provider).await.unwrap();
    }

    assert_eq!(breaker.state(&provider).await.unwrap(), CircuitState::Open);
}

/// Property 21b: while open, every request is rejected without an upstream call.
#[tokio::test]
#[ignore = "需要本地 Redis（REDIS_TEST_ADDR，默认 127.0.0.1:6379 的 db 15）"]
async fn an_open_circuit_rejects_every_request() {
    let breaker = CircuitBreaker::new(
        Some(testsupport::test_redis().await),
        CircuitBreakerSettings {
            failure_threshold: 0.5,
            window_seconds: 30,
            // Long enough that the cooldown cannot expire mid-test.
            cooldown_seconds: 600,
        },
    );
    let provider = testsupport::unique_name("cb-reject");

    for _ in 0..MIN_SAMPLES {
        breaker.record_failure(&provider).await.unwrap();
    }
    assert_eq!(breaker.state(&provider).await.unwrap(), CircuitState::Open);

    for i in 0..10 {
        assert!(
            !breaker.allow(&provider).await,
            "request {i} slipped through an open circuit"
        );
    }
    assert_eq!(
        breaker.state(&provider).await.unwrap(),
        CircuitState::Open,
        "rejecting must not change the state"
    );
}

/// Property 21c: once the cooldown has elapsed, the next request becomes the
/// half-open probe.
#[tokio::test]
#[ignore = "需要本地 Redis（REDIS_TEST_ADDR，默认 127.0.0.1:6379 的 db 15）"]
async fn the_cooldown_promotes_the_circuit_to_half_open() {
    let redis = testsupport::test_redis().await;
    let cooldown_seconds = 5;
    let breaker = CircuitBreaker::new(
        Some(redis.clone()),
        CircuitBreakerSettings {
            failure_threshold: 0.5,
            window_seconds: 30,
            cooldown_seconds,
        },
    );
    let provider = testsupport::unique_name("cb-halfopen");

    for _ in 0..MIN_SAMPLES {
        breaker.record_failure(&provider).await.unwrap();
    }
    assert!(
        !breaker.allow(&provider).await,
        "the circuit must reject before its cooldown expires"
    );

    backdate_opened_at(&redis, &provider, cooldown_seconds + 1).await;

    assert!(
        breaker.allow(&provider).await,
        "the probe must be admitted once the cooldown has elapsed"
    );
    assert_eq!(
        breaker.state(&provider).await.unwrap(),
        CircuitState::HalfOpen
    );
}

/// Property 21d: a successful probe closes the circuit and clears the counters,
/// so the next failure starts a fresh sample.
#[tokio::test]
#[ignore = "需要本地 Redis（REDIS_TEST_ADDR，默认 127.0.0.1:6379 的 db 15）"]
async fn a_successful_probe_closes_the_circuit() {
    let redis = testsupport::test_redis().await;
    let cooldown_seconds = 5;
    let breaker = CircuitBreaker::new(
        Some(redis.clone()),
        CircuitBreakerSettings {
            failure_threshold: 0.5,
            window_seconds: 30,
            cooldown_seconds,
        },
    );
    let provider = testsupport::unique_name("cb-recovery");

    for _ in 0..MIN_SAMPLES {
        breaker.record_failure(&provider).await.unwrap();
    }
    backdate_opened_at(&redis, &provider, cooldown_seconds + 1).await;
    assert!(breaker.allow(&provider).await);

    breaker.record_success(&provider).await.unwrap();

    assert_eq!(
        breaker.state(&provider).await.unwrap(),
        CircuitState::Closed
    );
    assert!(breaker.allow(&provider).await);

    // The counters were cleared, so one fresh failure is nowhere near a trip.
    breaker.record_failure(&provider).await.unwrap();
    assert_eq!(
        breaker.state(&provider).await.unwrap(),
        CircuitState::Closed,
        "the old failure count survived the reset"
    );
}

/// `gw_config` and this crate each carry the defaults; if either side drifts
/// the breaker would trip at a threshold nobody configured.
#[test]
fn config_defaults_match_the_breaker_defaults() {
    let from_config = CircuitBreakerSettings::from(&gw_config::CircuitBreakerConfig::default());
    assert_eq!(from_config, CircuitBreakerSettings::default());
}
