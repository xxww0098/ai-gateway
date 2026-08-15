use std::collections::HashSet;
use std::sync::Arc;

use super::*;
use crate::testsupport;

/// Settings with every scalar zeroed, i.e. what a `rate_limit:` block that only
/// sets `group_overrides` deserialises into.
fn zeroed() -> RateLimitSettings {
    RateLimitSettings {
        requests_per_min: 0,
        tokens_per_min: 0,
        max_concurrent: 0,
        burst_size: 0,
        global_request_cap: 0,
        global_token_cap: 0,
        group_overrides: HashMap::new(),
        model_token_limits: HashMap::new(),
    }
}

/// A zero in the config means "unset", so the resolved limiter must end up with
/// the same budgets as an untouched one — never with a budget of zero, which
/// would deny every request.
#[test]
fn zero_valued_settings_resolve_to_the_defaults() {
    assert_eq!(zeroed().with_defaults(), RateLimitSettings::default());

    let negative = RateLimitSettings {
        requests_per_min: -1,
        tokens_per_min: -1,
        max_concurrent: -1,
        burst_size: -1,
        global_request_cap: -1,
        global_token_cap: -1,
        ..zeroed()
    };
    assert_eq!(negative.with_defaults(), RateLimitSettings::default());
}

/// Configured budgets must survive defaulting untouched.
#[test]
fn positive_settings_are_left_alone() {
    let configured = RateLimitSettings {
        requests_per_min: 1,
        tokens_per_min: 2,
        max_concurrent: 3,
        burst_size: 4,
        global_request_cap: 5,
        global_token_cap: 6,
        ..zeroed()
    };
    assert_eq!(configured.clone().with_defaults(), configured);
}

/// With no group and no model, the effective limits are the plain defaults.
#[test]
fn effective_limits_fall_back_to_the_account_defaults() {
    let settings = RateLimitSettings::default();
    let limits = settings.effective_limits(None, "");
    assert_eq!(limits.max_requests, settings.requests_per_min);
    assert_eq!(limits.max_tokens, settings.tokens_per_min);
    assert_eq!(limits.max_concurrent, settings.max_concurrent);

    // A group with no override configured is the same as no group at all.
    assert_eq!(settings.effective_limits(Some(99), "gpt-4o"), limits);
}

/// A group override replaces only the fields it sets positively; the rest keep
/// falling back to the account defaults.
#[test]
fn group_override_replaces_only_its_positive_fields() {
    let mut settings = RateLimitSettings::default();
    settings.group_overrides.insert(
        "3".to_owned(),
        RateLimitOverride {
            requests_per_min: 5,
            tokens_per_min: 0,
            max_concurrent: -1,
            burst_size: 0,
        },
    );

    let limits = settings.effective_limits(Some(3), "");
    assert_eq!(limits.max_requests, 5);
    assert_eq!(
        limits.max_tokens, settings.tokens_per_min,
        "an unset override field must not zero the budget"
    );
    assert_eq!(limits.max_concurrent, settings.max_concurrent);
}

/// The per-model ceiling is resolved last, so it wins over a group override.
#[test]
fn model_token_limit_wins_over_group_and_default() {
    let mut settings = RateLimitSettings::default();
    settings.group_overrides.insert(
        "3".to_owned(),
        RateLimitOverride {
            tokens_per_min: 500,
            ..RateLimitOverride::default()
        },
    );
    settings
        .model_token_limits
        .insert("claude-opus-4".to_owned(), 250);

    assert_eq!(
        settings
            .effective_limits(Some(3), "claude-opus-4")
            .max_tokens,
        250
    );
    assert_eq!(
        settings.effective_limits(Some(3), "gpt-4o").max_tokens,
        500,
        "an unlisted model keeps the group budget"
    );
    assert_eq!(
        settings.effective_limits(None, "claude-opus-4").max_tokens,
        250
    );
}

/// A non-positive per-model limit is a config mistake, not an instruction to
/// deny everything for that model.
#[test]
fn non_positive_model_token_limit_is_ignored() {
    let mut settings = RateLimitSettings::default();
    settings.model_token_limits.insert("broken".to_owned(), 0);
    settings.model_token_limits.insert("worse".to_owned(), -10);

    assert_eq!(
        settings.effective_limits(None, "broken").max_tokens,
        settings.tokens_per_min
    );
    assert_eq!(
        settings.effective_limits(None, "worse").max_tokens,
        settings.tokens_per_min
    );
}

/// An allow reply hands back the very id that was reserved, so the caller can
/// release exactly that slot.
#[test]
fn allowed_reply_carries_the_reserved_slot_id() {
    let decision = parse_script_result("ALLOWED", "req-1");
    assert!(decision.is_allowed());
    assert_eq!(decision.release_id(), Some("req-1"));
    assert_eq!(decision.denied_dimension(), None);
}

/// Every dimension the script can deny on must arrive upstream distinguishable
/// — that is what lets the panel say *which* budget ran out.
#[test]
fn each_denial_payload_maps_to_its_own_dimension() {
    let seen: HashSet<DeniedDimension> = [
        DeniedDimension::RequestCount,
        DeniedDimension::TokenLimit,
        DeniedDimension::Concurrent,
        DeniedDimension::GlobalRequestCount,
        DeniedDimension::GlobalTokenLimit,
    ]
    .into_iter()
    .map(|dimension| {
        let decision = parse_script_result(&format!("DENIED:{dimension}"), "req-1");
        assert!(!decision.is_allowed(), "{dimension} must not be allowed");
        assert_eq!(decision.release_id(), None, "{dimension} reserved no slot");
        decision
            .denied_dimension()
            .expect("a denial has a dimension")
    })
    .collect();

    assert_eq!(seen.len(), 5, "two dimensions collapsed into one: {seen:?}");
    assert!(seen.iter().filter(|d| d.is_global()).count() == 2);
}

/// An unknown or malformed reply must deny, not fall through to allow: the one
/// direction that would silently disable the limiter.
#[test]
fn unrecognised_replies_deny() {
    for raw in ["DENIED:something_new", "DENIED:", "", "MAYBE"] {
        let decision = parse_script_result(raw, "req-1");
        assert_eq!(
            decision.denied_dimension(),
            Some(DeniedDimension::Unspecified),
            "reply {raw:?} must be an unspecified denial"
        );
    }
}

/// Dimensions round-trip through their wire spelling, so logs and parses agree.
#[test]
fn dimensions_round_trip_through_the_wire_spelling() {
    for dimension in [
        DeniedDimension::RequestCount,
        DeniedDimension::TokenLimit,
        DeniedDimension::Concurrent,
        DeniedDimension::GlobalRequestCount,
        DeniedDimension::GlobalTokenLimit,
        DeniedDimension::Unspecified,
    ] {
        assert_eq!(DeniedDimension::from_wire(dimension.as_str()), dimension);
    }
}

/// The three per-identity keys must stay distinct from each other and from
/// other identities', or one budget would consume another's window.
#[test]
fn keys_separate_identities_and_dimensions() {
    let mine = [
        request_key("user:1"),
        token_key("user:1"),
        concurrency_key("user:1"),
        global_request_key(),
        global_token_key(),
    ];
    let distinct: HashSet<&String> = mine.iter().collect();
    assert_eq!(distinct.len(), mine.len(), "keys collide: {mine:?}");
    assert!(mine.iter().all(|key| key.starts_with(KEY_PREFIX)));
    assert!(mine[..3].iter().all(|key| key.ends_with("user:1")));

    assert_ne!(request_key("user:1"), request_key("user:2"));
}

/// Request ids are the members of the token sorted set, whose count is parsed
/// off after the first colon — an id containing one would corrupt the tally.
#[test]
fn request_ids_are_unique_and_colon_free() {
    let ids: HashSet<String> = (0..10_000).map(|_| new_request_id()).collect();
    assert_eq!(ids.len(), 10_000, "request ids collided within one process");
    assert!(ids.iter().all(|id| !id.contains(':')));
    assert!(
        ids.iter()
            .all(|id| id.len() == ids.iter().next().unwrap().len())
    );
}

/// Without Redis the limiter must let everything through, and must not hand out
/// a release id there was never a slot for.
#[tokio::test]
async fn without_redis_the_limiter_fails_open() {
    let limiter = RateLimiter::new(None, RateLimitSettings::default());
    let decision = limiter.allow("nobody", 1, "gpt-4o", None).await;
    assert!(decision.is_allowed());
    assert_eq!(
        decision.release_id(),
        None,
        "no slot was reserved, so there is nothing to release"
    );
    limiter
        .release_conc("nobody", "")
        .await
        .expect("releasing without Redis is a no-op, not an error");
}

/// Defaulting happens at construction, so the limiter reports the budgets it
/// will actually enforce.
#[test]
fn constructor_applies_the_defaults() {
    let limiter = RateLimiter::new(None, zeroed());
    assert_eq!(limiter.settings(), &RateLimitSettings::default());
}

// ---------------------------------------------------------------------------
// Redis-backed behaviour.
//
//   cargo test -p gw-infra -- --ignored
//
// Needs a reachable Redis (REDIS_TEST_ADDR, default 127.0.0.1:6379, db 15).
// Each test uses a fresh identity instead of a fresh server, so the suite can
// run against a shared instance without clearing anyone's keys.
// ---------------------------------------------------------------------------

const REDIS_REQUIRED: &str = "需要本地 Redis（REDIS_TEST_ADDR，默认 127.0.0.1:6379 的 db 15）";

/// Budgets generous enough that only the dimension under test can deny.
fn only(dimension: DeniedDimension, limit: i64) -> RateLimitSettings {
    let mut settings = RateLimitSettings {
        requests_per_min: 100_000,
        tokens_per_min: 100_000_000,
        max_concurrent: 100_000,
        burst_size: 2,
        global_request_cap: 1_000_000,
        global_token_cap: 1_000_000_000,
        ..zeroed()
    };
    match dimension {
        DeniedDimension::RequestCount => settings.requests_per_min = limit,
        DeniedDimension::TokenLimit => settings.tokens_per_min = limit,
        DeniedDimension::Concurrent => settings.max_concurrent = limit,
        DeniedDimension::GlobalRequestCount => settings.global_request_cap = limit,
        DeniedDimension::GlobalTokenLimit => settings.global_token_cap = limit,
        DeniedDimension::Unspecified => unreachable!("not a configurable budget"),
    }
    settings
}

/// Property 10: however many callers race, the window admits at most its
/// budget.
#[tokio::test]
#[ignore = "需要本地 Redis（REDIS_TEST_ADDR，默认 127.0.0.1:6379 的 db 15）"]
async fn concurrent_callers_never_exceed_the_request_budget() {
    const MAX_REQ: i64 = 8;
    const CALLERS: i64 = 5;
    const ATTEMPTS_EACH: i64 = 8;

    let limiter = Arc::new(RateLimiter::new(
        Some(testsupport::test_redis().await),
        only(DeniedDimension::RequestCount, MAX_REQ),
    ));
    let identity = testsupport::unique_name("prop10");

    let callers: Vec<_> = (0..CALLERS)
        .map(|_| {
            let limiter = Arc::clone(&limiter);
            let identity = identity.clone();
            tokio::spawn(async move {
                let mut allowed = 0;
                for _ in 0..ATTEMPTS_EACH {
                    if limiter.allow(&identity, 1, "", None).await.is_allowed() {
                        allowed += 1;
                    }
                }
                allowed
            })
        })
        .collect();

    let mut total = 0;
    for caller in callers {
        total += caller.await.expect("caller task panicked");
    }

    assert!(
        total <= MAX_REQ,
        "{total} requests were admitted against a budget of {MAX_REQ}"
    );
    assert!(total > 0, "the limiter denied every single request");
}

/// Property 11: each budget denies on its own, with its own dimension, while
/// the others stay slack.
#[tokio::test]
#[ignore = "需要本地 Redis（REDIS_TEST_ADDR，默认 127.0.0.1:6379 的 db 15）"]
async fn each_budget_denies_on_its_own() {
    let redis = testsupport::test_redis().await;

    // Request count: fill the window, then the next one is denied.
    const MAX_REQ: i64 = 4;
    let limiter = RateLimiter::new(
        Some(redis.clone()),
        only(DeniedDimension::RequestCount, MAX_REQ),
    );
    let identity = testsupport::unique_name("prop11-req");
    for i in 0..MAX_REQ {
        assert!(
            limiter
                .allow(&identity, 1, "gpt-4o", None)
                .await
                .is_allowed(),
            "request {i} is still inside the budget of {MAX_REQ}"
        );
    }
    assert_eq!(
        limiter
            .allow(&identity, 1, "gpt-4o", None)
            .await
            .denied_dimension(),
        Some(DeniedDimension::RequestCount)
    );

    // Token consumption: two requests of just over half the budget.
    const MAX_TOK: i64 = 1_000;
    let limiter = RateLimiter::new(
        Some(redis.clone()),
        only(DeniedDimension::TokenLimit, MAX_TOK),
    );
    let identity = testsupport::unique_name("prop11-tok");
    let per_request = MAX_TOK / 2 + 1;
    assert!(
        limiter
            .allow(&identity, per_request, "gpt-4o", None)
            .await
            .is_allowed(),
        "the first request fits inside the token budget"
    );
    assert_eq!(
        limiter
            .allow(&identity, per_request, "gpt-4o", None)
            .await
            .denied_dimension(),
        Some(DeniedDimension::TokenLimit)
    );

    // Concurrency: fill the slots without releasing any.
    const MAX_CONC: i64 = 3;
    let limiter = RateLimiter::new(Some(redis), only(DeniedDimension::Concurrent, MAX_CONC));
    let identity = testsupport::unique_name("prop11-conc");
    for i in 0..MAX_CONC {
        assert!(
            limiter
                .allow(&identity, 1, "gpt-4o", None)
                .await
                .is_allowed(),
            "slot {i} is still free"
        );
    }
    assert_eq!(
        limiter
            .allow(&identity, 1, "gpt-4o", None)
            .await
            .denied_dimension(),
        Some(DeniedDimension::Concurrent)
    );
}

/// The regression test for the concurrency leak. A released slot must be
/// reusable immediately, not after the 10-minute TTL.
#[tokio::test]
#[ignore = "需要本地 Redis（REDIS_TEST_ADDR，默认 127.0.0.1:6379 的 db 15）"]
async fn releasing_a_slot_admits_the_next_request() {
    let limiter = RateLimiter::new(
        Some(testsupport::test_redis().await),
        only(DeniedDimension::Concurrent, 2),
    );
    let identity = testsupport::unique_name("release");

    let first = limiter.allow(&identity, 1, "gpt-4o", None).await;
    let second = limiter.allow(&identity, 1, "gpt-4o", None).await;
    let first_slot = first
        .release_id()
        .expect("an admitted request holds a slot");
    assert!(second.is_allowed());

    let third = limiter.allow(&identity, 1, "gpt-4o", None).await;
    assert_eq!(third.denied_dimension(), Some(DeniedDimension::Concurrent));
    assert_eq!(third.release_id(), None, "a denial must not reserve a slot");

    limiter
        .release_conc(&identity, first_slot)
        .await
        .expect(REDIS_REQUIRED);

    assert!(
        limiter
            .allow(&identity, 1, "gpt-4o", None)
            .await
            .is_allowed(),
        "the freed slot was not reusable"
    );
}

/// The global caps deny even when the caller's own budgets are untouched.
#[tokio::test]
#[ignore = "需要本地 Redis（REDIS_TEST_ADDR，默认 127.0.0.1:6379 的 db 15）"]
async fn the_global_cap_denies_across_identities() {
    // The global window is shared with every other client of this Redis, so
    // this test only asserts that *some* request is denied globally once the
    // cap is reached — not which one.
    let mut settings = only(DeniedDimension::GlobalRequestCount, 1);
    settings.requests_per_min = 100_000;
    let limiter = RateLimiter::new(Some(testsupport::test_redis().await), settings);

    let mut denials = Vec::new();
    for i in 0..4 {
        let identity = testsupport::unique_name(&format!("global-{i}"));
        if let Some(dimension) = limiter
            .allow(&identity, 1, "", None)
            .await
            .denied_dimension()
        {
            denials.push(dimension);
        }
    }

    assert!(
        denials.contains(&DeniedDimension::GlobalRequestCount),
        "a cap of one request per window admitted four different identities"
    );
}

/// `gw_config` and this crate each carry the defaults; if either side drifts
/// the gateway would silently enforce budgets nobody configured.
#[test]
fn config_defaults_match_the_limiter_defaults() {
    let from_config = RateLimitSettings::from(&gw_config::RateLimitConfig::default());
    assert_eq!(from_config, RateLimitSettings::default());
}

/// A group override arrives with its zeros intact — `gw_config` documents them
/// as "no override", and this crate is where that is interpreted.
#[test]
fn config_group_overrides_survive_the_lift() {
    let mut cfg = gw_config::RateLimitConfig::default();
    cfg.group_overrides.insert(
        "3".to_owned(),
        gw_config::RateLimitOverride {
            requests_per_min: 5,
            tokens_per_min: 0,
            max_concurrent: 0,
            burst_size: 0,
        },
    );
    cfg.model_token_limits.insert("gpt-4o".to_owned(), 42);

    let settings = RateLimitSettings::from(&cfg);
    let limits = settings.effective_limits(Some(3), "gpt-4o");
    assert_eq!(limits.max_requests, 5);
    assert_eq!(limits.max_tokens, 42);
    assert_eq!(
        limits.max_concurrent, settings.max_concurrent,
        "a zero in the override must not become a zero budget"
    );
}
