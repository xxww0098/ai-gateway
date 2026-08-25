//! The atomic Lua scripts the hold accounting runs inside Redis.
//!
//! [`HOLD_SCRIPT`] and [`GET_BALANCE_SCRIPT`] are the only place where
//! "available balance" is computed, and
//! they must stay verbatim: multiple binaries can share one Redis during a
//! cutover, so any divergence in the expiry cutoff or the summation would let
//! two processes admit holds against different views of the same balance.
//!
//! [`RENEW_SCRIPT`] deliberately computes **nothing**: it is not a second
//! admission, so it must not grow a third copy of that summation.

use std::sync::LazyLock;

use redis::Script;

/// Atomic hold admission.
///
/// 1. read the cached balance from `KEYS[1]` (error `CACHE_MISS` if absent)
/// 2. drop holds from `KEYS[2]`/`KEYS[3]` older than `ARGV[3] - ARGV[4]`
/// 3. sum the surviving scores in `KEYS[2]`
/// 4. admit iff `balance - sum_holds >= ARGV[1]`
/// 5. on admission `ZADD` the amount, `HSET` the timestamp, `EXPIRE` both
///    hold keys (so idle users do not leave immortal sets), reply `OK`
/// 6. otherwise reply the error `INSUFFICIENT_BALANCE:<available>`
///
/// The expiry cutoff (`now - ttl`) and the summation are the cutover
/// contract: two binaries may share one Redis, and they must admit against
/// the same view of the balance. `EXPIRE` and the optional floor (`ARGV[7]`)
/// are additive — a new script SHA can coexist with an old one; they do not
/// change who is admitted for a given `(balance, holds, amount)`.
///
/// ```text
/// KEYS[1] = ai-gateway:billing:balance:{userID}   (cached balance string)
/// KEYS[2] = ai-gateway:billing:holds:{userID}     (sorted set)
/// KEYS[3] = ai-gateway:billing:holds:ts:{userID}  (hold timestamps hash)
/// ARGV[1] = hold amount (float as string)
/// ARGV[2] = request ID
/// ARGV[3] = current timestamp (unix seconds)
/// ARGV[4] = hold TTL seconds
/// ARGV[5] = idempotency key (empty string if none)
/// ARGV[6] = hold-key EXPIRE seconds (hold TTL + margin)
/// ARGV[7] = min available (floor); defaults to ARGV[1] when absent/invalid
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
local key_ttl = tonumber(ARGV[6])
local min_available = tonumber(ARGV[7])
if not min_available then
    min_available = amount
end
-- The reserved amount is still ARGV[1]. The floor only gates admission, so a
-- caller can refuse when available covers the hold but not the pre-flight
-- upper bound, without over-holding.
if min_available < amount then
    min_available = amount
end

local function touch_ttl()
    if key_ttl and key_ttl > 0 then
        redis.call('EXPIRE', KEYS[2], key_ttl)
        redis.call('EXPIRE', KEYS[3], key_ttl)
    end
end

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
    touch_ttl()
    return 'OK'
end

-- Sum all active hold scores
local holds = redis.call('ZRANGE', KEYS[2], 0, -1, 'WITHSCORES')
local sum_holds = 0
for i = 2, #holds, 2 do
    sum_holds = sum_holds + tonumber(holds[i])
end

-- Check available balance against the floor (and therefore against amount)
local available = balance - sum_holds
if available < min_available then
    return redis.error_reply('INSUFFICIENT_BALANCE:' .. tostring(available))
end

-- Add the new hold
redis.call('ZADD', KEYS[2], amount, request_id)
redis.call('HSET', KEYS[3], request_id, tostring(now))
touch_ttl()

return 'OK'
",
    )
});

/// Atomic available-balance read: expire stale holds, then reply
/// `cached_balance - sum(active holds)` as a string.
///
/// ```text
/// KEYS[1] = ai-gateway:billing:balance:{userID}   (cached balance string)
/// KEYS[2] = ai-gateway:billing:holds:{userID}     (sorted set)
/// KEYS[3] = ai-gateway:billing:holds:ts:{userID}  (hold timestamps hash)
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

/// 只把租约的时间戳往后推一格。
///
/// 1. `ZSCORE` 取分数 —— 成员不在就回 `HOLD_NOT_FOUND`，
///    **绝不 `ZADD` 把它造出来**：续租不是准入，它不许凭空产生一笔预留。
/// 2. `HSET` 时间戳为 `now`，于是两个清理脚本的 `now - ttl` 截止线追不上它。
/// 3. `EXPIRE` 两个 hold 键，与预扣时同样的余量。
/// 4. 原样回那个分数 —— 调用方据此确认**金额没变**。
///
/// 分数一个字节不动，余额一次不读。
///
/// ```text
/// KEYS[1] = ai-gateway:billing:holds:{userID}     (sorted set)
/// KEYS[2] = ai-gateway:billing:holds:ts:{userID}  (hold timestamps hash)
/// ARGV[1] = billing operation id
/// ARGV[2] = current timestamp (unix seconds)
/// ARGV[3] = hold-key EXPIRE seconds (hold TTL + margin)
/// ```
pub(crate) static RENEW_SCRIPT: LazyLock<Script> = LazyLock::new(|| {
    Script::new(
        r"
local score = redis.call('ZSCORE', KEYS[1], ARGV[1])
if not score then
    return redis.error_reply('HOLD_NOT_FOUND')
end

redis.call('HSET', KEYS[2], ARGV[1], ARGV[2])

local key_ttl = tonumber(ARGV[3])
if key_ttl and key_ttl > 0 then
    redis.call('EXPIRE', KEYS[1], key_ttl)
    redis.call('EXPIRE', KEYS[2], key_ttl)
end

return score
",
    )
});

/// The Lua `error_reply` marker meaning "no cached balance for this user".
/// The caller reacts by loading the balance from Postgres and retrying.
pub(crate) const CACHE_MISS: &str = "CACHE_MISS";

/// The Lua `error_reply` marker meaning the hold was refused. The reply also
/// carries the available balance after the colon, which the caller discards.
pub(crate) const INSUFFICIENT_BALANCE: &str = "INSUFFICIENT_BALANCE";

/// [`RENEW_SCRIPT`] 的拒绝标记：这个成员已经不在 zset 里了。
pub(crate) const HOLD_NOT_FOUND: &str = "HOLD_NOT_FOUND";
