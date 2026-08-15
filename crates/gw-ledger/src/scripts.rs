//! The two atomic Lua scripts the hold accounting runs inside Redis.
//!
//! Both scripts are the only place where "available balance" is computed, and
//! they must stay verbatim: multiple binaries can share one Redis during a
//! cutover, so any divergence in the expiry cutoff or the summation would let
//! two processes admit holds against different views of the same balance.

use std::sync::LazyLock;

use redis::Script;

/// Atomic hold admission.
///
/// 1. read the cached balance from `KEYS[1]` (error `CACHE_MISS` if absent)
/// 2. drop holds from `KEYS[2]`/`KEYS[3]` older than `ARGV[3] - ARGV[4]`
/// 3. sum the surviving scores in `KEYS[2]`
/// 4. admit iff `balance - sum_holds >= ARGV[1]`
/// 5. on admission `ZADD` the amount and `HSET` the timestamp, reply `OK`
/// 6. otherwise reply the error `INSUFFICIENT_BALANCE:<available>`
///
/// ```text
/// KEYS[1] = cpa-gateway:billing:balance:{userID}   (cached balance string)
/// KEYS[2] = cpa-gateway:billing:holds:{userID}     (sorted set)
/// KEYS[3] = cpa-gateway:billing:holds:ts:{userID}  (hold timestamps hash)
/// ARGV[1] = hold amount (float as string)
/// ARGV[2] = request ID
/// ARGV[3] = current timestamp (unix seconds)
/// ARGV[4] = hold TTL seconds
/// ARGV[5] = idempotency key (empty string if none)
/// ```
///
/// Re-holding an existing request id is idempotent: the script replies `OK`
/// without touching the score, so a retried request cannot double-reserve.
pub(crate) static HOLD_SCRIPT: LazyLock<Script> = LazyLock::new(|| {
    Script::new(
        r"
-- Read cached balance
local balance_str = redis.call('GET', KEYS[1])
if not balance_str then
    return redis.error_reply('CACHE_MISS')
end
local balance = tonumber(balance_str)
if not balance then
    return redis.error_reply('INVALID_BALANCE')
end

local amount = tonumber(ARGV[1])
local request_id = ARGV[2]
local now = tonumber(ARGV[3])
local ttl_seconds = tonumber(ARGV[4])

-- Clean expired holds: remove entries whose timestamp is older than (now - ttl_seconds)
local cutoff = now - ttl_seconds
local ts_entries = redis.call('HGETALL', KEYS[3])
for i = 1, #ts_entries, 2 do
    local member = ts_entries[i]
    local ts = tonumber(ts_entries[i + 1])
    if ts and ts < cutoff then
        redis.call('ZREM', KEYS[2], member)
        redis.call('HDEL', KEYS[3], member)
    end
end

-- Check if this request ID already exists (idempotent hold)
local existing = redis.call('ZSCORE', KEYS[2], request_id)
if existing then
    return 'OK'
end

-- Sum all active hold scores
local holds = redis.call('ZRANGE', KEYS[2], 0, -1, 'WITHSCORES')
local sum_holds = 0
for i = 2, #holds, 2 do
    sum_holds = sum_holds + tonumber(holds[i])
end

-- Check available balance
local available = balance - sum_holds
if available < amount then
    return redis.error_reply('INSUFFICIENT_BALANCE:' .. tostring(available))
end

-- Add the new hold
redis.call('ZADD', KEYS[2], amount, request_id)
redis.call('HSET', KEYS[3], request_id, tostring(now))

return 'OK'
",
    )
});

/// Atomic available-balance read: expire stale holds, then reply
/// `cached_balance - sum(active holds)` as a string.
///
/// ```text
/// KEYS[1] = cpa-gateway:billing:balance:{userID}   (cached balance string)
/// KEYS[2] = cpa-gateway:billing:holds:{userID}     (sorted set)
/// KEYS[3] = cpa-gateway:billing:holds:ts:{userID}  (hold timestamps hash)
/// ARGV[1] = current timestamp (unix seconds)
/// ARGV[2] = hold TTL seconds
/// ```
///
/// The expiry cutoff here and in [`HOLD_SCRIPT`] must be computed from the
/// *same* configured hold TTL. Feeding this one a different TTL than the hold
/// path would let a read reclaim a reservation the writer still considers
/// live.
pub(crate) static GET_BALANCE_SCRIPT: LazyLock<Script> = LazyLock::new(|| {
    Script::new(
        r"
-- Read cached balance
local balance_str = redis.call('GET', KEYS[1])
if not balance_str then
    return redis.error_reply('CACHE_MISS')
end
local balance = tonumber(balance_str)
if not balance then
    return redis.error_reply('INVALID_BALANCE')
end

local now = tonumber(ARGV[1])
local ttl_seconds = tonumber(ARGV[2])

-- Clean expired holds
local cutoff = now - ttl_seconds
local ts_entries = redis.call('HGETALL', KEYS[3])
for i = 1, #ts_entries, 2 do
    local member = ts_entries[i]
    local ts = tonumber(ts_entries[i + 1])
    if ts and ts < cutoff then
        redis.call('ZREM', KEYS[2], member)
        redis.call('HDEL', KEYS[3], member)
    end
end

-- Sum all active hold scores
local holds = redis.call('ZRANGE', KEYS[2], 0, -1, 'WITHSCORES')
local sum_holds = 0
for i = 2, #holds, 2 do
    sum_holds = sum_holds + tonumber(holds[i])
end

-- Return available balance
local available = balance - sum_holds
return tostring(available)
",
    )
});

/// The Lua `error_reply` marker meaning "no cached balance for this user".
/// The caller reacts by loading the balance from Postgres and retrying.
pub(crate) const CACHE_MISS: &str = "CACHE_MISS";

/// The Lua `error_reply` marker meaning the hold was refused. The reply also
/// carries the available balance after the colon, which the caller discards.
pub(crate) const INSUFFICIENT_BALANCE: &str = "INSUFFICIENT_BALANCE";
