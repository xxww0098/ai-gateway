use super::*;

fn key(expires_in: TimeDelta) -> CachedKey {
    CachedKey {
        user_id: 7,
        api_key_id: 11,
        group_id: Some(3),
        rate_mult: 1.5,
        status: "active".to_owned(),
        expires_at: Utc::now() + expires_in,
    }
}

/// Set → Get returns the same entry; Delete makes it a miss.
#[test]
fn api_key_entries_round_trip_until_deleted() {
    let cache = ApiKeyCache::new();
    let entry = key(TimeDelta::minutes(1));
    cache.set("hash-a", entry.clone());

    assert_eq!(cache.get("hash-a").as_deref(), Some(&entry));
    assert!(cache.get("hash-b").is_none(), "unknown hashes must miss");

    cache.delete("hash-a");
    assert!(cache.get("hash-a").is_none());
    assert!(cache.is_empty());
}

/// A past deadline is a miss *and* an eviction — Get deletes on the read path
/// so a stale entry cannot linger until the next sweep.
#[test]
fn expired_api_key_entries_are_evicted_on_read() {
    let cache = ApiKeyCache::new();
    cache.set("stale", key(TimeDelta::milliseconds(-1)));
    assert_eq!(cache.len(), 1, "the entry is stored before it is read");

    assert!(cache.get("stale").is_none());
    assert!(
        cache.is_empty(),
        "the expired entry must be gone from the map, not just hidden"
    );
}

/// Both caches are handles: `gw-proxy` and `gw-panel` hold clones and must see
/// each other's writes and invalidations.
#[test]
fn clones_share_one_set_of_entries() {
    let keys = ApiKeyCache::new();
    let statuses = UserStatusCache::new();
    let (keys_clone, statuses_clone) = (keys.clone(), statuses.clone());

    keys.set("shared", key(TimeDelta::minutes(1)));
    statuses.set(42, "active", Duration::from_secs(60));

    assert!(keys_clone.get("shared").is_some());
    assert!(statuses_clone.get(42).is_some());

    keys_clone.delete("shared");
    statuses_clone.invalidate_user(42);

    assert!(keys.get("shared").is_none());
    assert!(
        statuses.get(42).is_none(),
        "an admin status flip on one surface must be visible on the other"
    );
}

/// Hit, TTL-driven miss, and an immediate miss after invalidation.
#[test]
fn user_status_get_set_expire() {
    let cache = UserStatusCache::new();
    const USER: Id = 42;

    cache.set(USER, "active", Duration::from_secs(60));
    let got = cache.get(USER).expect("Get after Set must hit");
    assert_eq!(got.status, "active");
    assert!(got.expires_at > Utc::now(), "a fresh entry expires later");

    cache.set(USER, "active", Duration::from_millis(1));
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(cache.get(USER).is_none(), "an expired entry must miss");

    cache.set(USER, "active", Duration::from_secs(60));
    assert!(cache.get(USER).is_some());
    cache.invalidate_user(USER);
    assert!(
        cache.get(USER).is_none(),
        "InvalidateUser must miss even while the entry is fresh"
    );
}

/// A zero TTL must not poison the cache with an already-expired entry. (The
/// `ttl <= 0` guard; the negative half is unrepresentable in a `Duration`.)
#[test]
fn zero_ttl_is_a_no_op() {
    let cache = UserStatusCache::new();
    cache.set(1, "active", Duration::ZERO);
    assert!(cache.get(1).is_none());
    assert!(cache.is_empty(), "nothing may be stored at all");
}

/// No entry may outlive the API-key cache generation, however long a TTL the
/// caller asks for.
#[test]
fn ttl_is_clamped_to_the_cache_maximum() {
    let cache = UserStatusCache::new();
    cache.set(1, "active", Duration::from_secs(86_400));
    // Taken after the write, so the clamp is measured from an instant that
    // cannot precede the one `set` stamped the entry with.
    let ceiling = Utc::now()
        + TimeDelta::from_std(MAX_USER_STATUS_TTL).expect("the clamp fits in a TimeDelta");
    let got = cache.get(1).expect("a clamped entry is still cached");
    assert!(
        got.expires_at <= ceiling,
        "expires_at {} must not exceed the clamp {ceiling}",
        got.expires_at
    );
}

/// After the read-path eviction, re-populating must not be shadowed by a
/// residual entry.
#[test]
fn re_set_after_expiry_serves_the_new_status() {
    let cache = UserStatusCache::new();
    const USER: Id = 7;

    cache.set(USER, "active", Duration::from_millis(1));
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(cache.get(USER).is_none());

    cache.set(USER, "suspended", Duration::from_secs(60));
    assert_eq!(
        cache.get(USER).expect("re-Set must hit").status,
        "suspended"
    );
}

/// The background sweeper reclaims entries nobody reads. The interval is a
/// parameter here, so this is assertable.
#[tokio::test]
async fn sweeper_reclaims_expired_entries_without_a_reader() {
    let keys = ApiKeyCache::new();
    let statuses = UserStatusCache::new();
    keys.set("stale", key(TimeDelta::milliseconds(5)));
    statuses.set(1, "active", Duration::from_millis(5));

    let sweep_keys = keys.spawn_sweeper(Duration::from_millis(10));
    let sweep_statuses = statuses.spawn_sweeper(Duration::from_millis(10));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while (!keys.is_empty() || !statuses.is_empty()) && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(keys.is_empty(), "the api-key sweeper never ran");
    assert!(statuses.is_empty(), "the user-status sweeper never ran");

    // Live entries survive the sweep.
    statuses.set(2, "active", Duration::from_secs(60));
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(statuses.get(2).is_some(), "a fresh entry was swept away");

    sweep_keys.shutdown().await;
    sweep_statuses.shutdown().await;
}

/// Cancelling must make the task return, so shutdown does not leak it.
#[tokio::test]
async fn shutdown_stops_the_sweeper() {
    let cache = UserStatusCache::new();
    let handle = cache.spawn_sweeper(Duration::from_millis(10));
    assert!(!handle.is_finished(), "the sweeper starts out running");

    handle.shutdown().await;

    // A second handle proves shutdown did not poison the cache for new tasks.
    let handle = cache.spawn_sweeper(Duration::from_millis(10));
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!handle.is_finished());
    handle.shutdown().await;
}

/// Dropping the last cache handle must stop the task too, so a caller that
/// forgets the [`SweepHandle`] does not leak a task per dropped cache.
#[tokio::test]
async fn dropping_the_cache_stops_the_sweeper() {
    let cache = UserStatusCache::new();
    let handle = cache.spawn_sweeper(Duration::from_millis(5));
    drop(cache);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !handle.is_finished() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        handle.is_finished(),
        "the sweeper outlived the cache it was sweeping"
    );
}
