//! `Ledger` — the Hold / Settle / Release state machine.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde_json::json;
use sqlx::{PgConnection, PgPool};

use crate::keys::{balance_key, holds_key, holds_ts_key, shortfall_resolve_reference};
use crate::scripts::{CACHE_MISS, GET_BALANCE_SCRIPT, HOLD_SCRIPT, INSUFFICIENT_BALANCE};
use crate::settlement::Settlement;
use crate::{LedgerError, log_type};

/// How long a cached balance stays in Redis before it is re-read from
/// Postgres.
pub const DEFAULT_BALANCE_TTL: Duration = Duration::from_secs(30);

/// The maximum lifetime of a reservation. Also the expiry cutoff both Lua
/// scripts use.
pub const DEFAULT_HOLD_TTL: Duration = Duration::from_secs(300);

/// Extra lifetime given to the hold sorted set and timestamp hash beyond the
/// hold TTL, so the keys self-expire once a user goes idle but never before
/// their last live hold could have expired.
const HOLD_KEY_TTL_MARGIN: Duration = Duration::from_secs(60);

/// How every money column is read back.
///
/// The historical schema built money columns as Postgres `numeric`, and sqlx
/// refuses to decode `NUMERIC` into `f64` — so the cast is mandatory, not
/// cosmetic. The column is also nullable (the schema only marks `not null`
/// where it was asked to) and a NULL is read back as `0`, so every call site
/// decodes an `Option<f64>` and defaults it. Dropping either half makes rows
/// a live deployment can read unreadable from newer code.
pub(crate) const BALANCE_EXPR: &str = "balance::float8";

/// What [`Ledger::settle_tx`] recorded, for a caller that needs to act on the
/// debt within the same transaction.
///
/// The shortfall is the only field of the original design; the two extra
/// fields here are additive. [`shortfall_log_id`](Self::shortfall_log_id) in
/// particular is what a compensating credit has to reference — see
/// [`shortfall_resolve_reference`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SettleOutcome {
    /// What actually came off the balance: `min(balance, actual)`.
    pub debited: f64,
    /// What the balance could not cover. Zero when fully covered.
    pub shortfall: f64,
    /// `balance_logs.id` of the zero-amount shortfall marker row, present iff
    /// [`shortfall`](Self::shortfall) is positive.
    pub shortfall_log_id: Option<i64>,
}

/// Coordinates persistent balance updates in Postgres with Redis pre-charge
/// holds.
///
/// Cheap to clone (a pool handle and a connection manager, both already
/// reference-counted); clone it rather than wrapping it in an `Arc`.
#[derive(Clone)]
pub struct Ledger {
    db: PgPool,
    redis: Option<ConnectionManager>,
    balance_ttl: Duration,
    hold_ttl: Duration,
}

/// Hand-written because `redis::aio::ConnectionManager` is not `Debug`. Only
/// whether Redis is wired is worth printing anyway — the pool's internals are
/// noise in a billing log line.
impl std::fmt::Debug for Ledger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ledger")
            .field("redis", &self.redis.is_some())
            .field("balance_ttl", &self.balance_ttl)
            .field("hold_ttl", &self.hold_ttl)
            .finish_non_exhaustive()
    }
}

impl Ledger {
    /// Builds a ledger with the default TTLs.
    ///
    /// `redis` may be `None`; the hold path then refuses to run
    /// ([`LedgerError::RedisNotConfigured`]) while credit/debit/get-balance
    /// still work straight against Postgres.
    #[must_use]
    pub fn new(db: PgPool, redis: Option<ConnectionManager>) -> Self {
        Self::with_config(db, redis, DEFAULT_BALANCE_TTL, DEFAULT_HOLD_TTL)
    }

    /// Builds a ledger with explicit TTLs, including the fallback: a zero TTL
    /// means "use the default" rather than "expire immediately".
    ///
    /// `hold_ttl` must match whatever the hold middleware was configured with
    /// — it is the cutoff both Lua scripts use to expire stale reservations.
    #[must_use]
    pub fn with_config(
        db: PgPool,
        redis: Option<ConnectionManager>,
        balance_ttl: Duration,
        hold_ttl: Duration,
    ) -> Self {
        Self {
            db,
            redis,
            balance_ttl: if balance_ttl.is_zero() {
                DEFAULT_BALANCE_TTL
            } else {
                balance_ttl
            },
            hold_ttl: if hold_ttl.is_zero() {
                DEFAULT_HOLD_TTL
            } else {
                hold_ttl
            },
        }
    }

    /// The Postgres pool this ledger writes through, for callers that need to
    /// open the transaction [`settle_tx`](Self::settle_tx) joins.
    #[must_use]
    pub fn db(&self) -> &PgPool {
        &self.db
    }

    /// The configured reservation lifetime, i.e. the expiry cutoff the Lua
    /// scripts apply.
    #[must_use]
    pub fn hold_ttl(&self) -> Duration {
        self.hold_ttl
    }

    // ------------------------------------------------------------- balance

    /// Adds `amount` to a user's balance and writes a `credit` journal row in
    /// one transaction; invalidates the Redis balance cache after the commit.
    ///
    /// # Errors
    /// [`LedgerError::InvalidArgument`] for a non-positive amount,
    /// [`LedgerError::UserNotFound`] for an unknown user, otherwise the
    /// underlying query error.
    pub async fn credit(
        &self,
        user_id: i64,
        amount: f64,
        reference: &str,
    ) -> Result<(), LedgerError> {
        if amount <= 0.0 {
            return Err(LedgerError::InvalidArgument(
                "credit amount must be positive",
            ));
        }
        let mut tx = self.db.begin().await?;
        let balance = Self::lock_balance(&mut tx, user_id).await?;
        Self::set_balance(&mut tx, user_id, balance + amount).await?;
        Self::insert_balance_log(&mut tx, user_id, amount, log_type::CREDIT, reference, None)
            .await?;
        tx.commit().await?;

        self.invalidate_balance_cache(user_id).await;
        Ok(())
    }

    /// Subtracts `amount` from a user's balance and writes a `debit` journal
    /// row in one transaction; invalidates the Redis balance cache after the
    /// commit.
    ///
    /// Unlike settle, this refuses to overdraw: an amount the balance cannot
    /// cover fails with [`LedgerError::InsufficientBalance`] and changes
    /// nothing. Purchases must not be able to create debt.
    ///
    /// # Errors
    /// [`LedgerError::InvalidArgument`] for a non-positive amount,
    /// [`LedgerError::UserNotFound`] for an unknown user,
    /// [`LedgerError::InsufficientBalance`] when the balance is too low.
    pub async fn debit(
        &self,
        user_id: i64,
        amount: f64,
        reference: &str,
    ) -> Result<(), LedgerError> {
        if amount <= 0.0 {
            return Err(LedgerError::InvalidArgument(
                "debit amount must be positive",
            ));
        }
        let mut tx = self.db.begin().await?;
        let balance = Self::lock_balance(&mut tx, user_id).await?;
        if balance < amount {
            return Err(LedgerError::InsufficientBalance);
        }
        Self::set_balance(&mut tx, user_id, balance - amount).await?;
        Self::insert_balance_log(&mut tx, user_id, -amount, log_type::DEBIT, reference, None)
            .await?;
        tx.commit().await?;

        self.invalidate_balance_cache(user_id).await;
        Ok(())
    }

    /// The available balance: the cached persistent balance minus every live
    /// hold, computed atomically inside Redis (which also expires stale holds
    /// on the way through).
    ///
    /// Falls back to the raw Postgres balance when no Redis is configured.
    ///
    /// # Errors
    /// [`LedgerError::UserNotFound`] when the cache has to be primed for a
    /// user that does not exist; otherwise the underlying Redis or query
    /// error.
    pub async fn get_balance(&self, user_id: i64) -> Result<f64, LedgerError> {
        let Some(mut conn) = self.redis_conn() else {
            return self.db_balance(user_id).await;
        };

        let now = Self::now_unix();
        let hold_ttl_secs = self.hold_ttl.as_secs() as i64;

        let available = match run_get_balance(&mut conn, user_id, now, hold_ttl_secs).await {
            Ok(v) => v,
            Err(err) if is_cache_miss(&err) => {
                // No cached balance: load it from Postgres, cache it, retry.
                self.refresh_balance_cache(user_id).await?;
                run_get_balance(&mut conn, user_id, now, hold_ttl_secs).await?
            }
            Err(err) => return Err(err.into()),
        };

        available
            .parse::<f64>()
            .map_err(|e| LedgerError::Other(anyhow::anyhow!("parsing available balance: {e}")))
    }

    // ---------------------------------------------------------------- hold

    /// Reserves `amount` for `request_id`, admitted only if the available
    /// balance covers it. Atomic: the check and the reservation happen inside
    /// one Lua script, so two concurrent requests can never both be admitted
    /// against the same money.
    ///
    /// Re-holding a request id that already has a reservation is a no-op
    /// success, so a retry cannot double-reserve.
    ///
    /// **Signature and semantics are a hard project constraint**
    /// (`AGENTS.md`).
    ///
    /// The `ttl` argument is validated but deliberately *not* used as the
    /// expiry cutoff: the script is fed the ledger's configured
    /// [`hold_ttl`](Self::hold_ttl) instead. Get-balance's script uses that
    /// same cutoff, and a per-request value here would desync the two — a
    /// short-TTL request could evict a long-TTL request's reservation. The
    /// argument is kept for API stability; configure the ledger's hold TTL to
    /// match what callers pass.
    ///
    /// # Errors
    /// [`LedgerError::InsufficientBalance`] when the available balance does
    /// not cover the amount, [`LedgerError::RedisNotConfigured`] without
    /// Redis, [`LedgerError::InvalidArgument`] for a non-positive amount,
    /// empty request id, or zero TTL.
    pub async fn hold(
        &self,
        user_id: i64,
        amount: f64,
        request_id: &str,
        ttl: Duration,
    ) -> Result<(), LedgerError> {
        let Some(mut conn) = self.redis_conn() else {
            return Err(LedgerError::RedisNotConfigured);
        };
        if amount <= 0.0 {
            return Err(LedgerError::InvalidArgument("hold amount must be positive"));
        }
        if request_id.is_empty() {
            return Err(LedgerError::InvalidArgument("requestID is required"));
        }
        if ttl.is_zero() {
            return Err(LedgerError::InvalidArgument("hold ttl must be positive"));
        }

        let now = Self::now_unix();
        let hold_ttl_secs = self.hold_ttl.as_secs() as i64;
        // Shortest round-tripping decimal form, so the score Lua stores is
        // bit-identical across implementations.
        let amount_arg = amount.to_string();

        let outcome = match run_hold(
            &mut conn,
            user_id,
            &amount_arg,
            request_id,
            now,
            hold_ttl_secs,
        )
        .await
        {
            Ok(v) => Ok(v),
            Err(err) if is_cache_miss(&err) => {
                // No cached balance: load it from Postgres, cache it, retry.
                self.refresh_balance_cache(user_id).await?;
                run_hold(
                    &mut conn,
                    user_id,
                    &amount_arg,
                    request_id,
                    now,
                    hold_ttl_secs,
                )
                .await
            }
            Err(err) => Err(err),
        };

        match outcome {
            Ok(reply) if reply == "OK" => {}
            Ok(reply) => {
                return Err(LedgerError::Other(anyhow::anyhow!(
                    "unexpected hold script result: {reply}"
                )));
            }
            Err(err) if is_insufficient_balance(&err) => {
                return Err(LedgerError::InsufficientBalance);
            }
            Err(err) => return Err(err.into()),
        }

        // Let the hold keys self-expire once the user goes idle. Best-effort:
        // the reservation itself is already recorded, and the Lua cutoff — not
        // this TTL — is what governs correctness. 两条 EXPIRE 打进一条 pipeline，
        // 少一次 RTT。
        let key_ttl = (self.hold_ttl + HOLD_KEY_TTL_MARGIN).as_secs() as i64;
        let mut pipe = redis::pipe();
        pipe.expire(holds_key(user_id), key_ttl).ignore();
        pipe.expire(holds_ts_key(user_id), key_ttl).ignore();
        if let Err(err) = pipe.query_async::<()>(&mut conn).await {
            tracing::debug!(error = %err, user_id, "failed to set hold key expiry");
        }

        // 审计是 best-effort，不该挡在预扣成功之后、上游调用之前。
        let ledger = self.clone();
        let request_id = request_id.to_owned();
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::spawn(async move {
                ledger
                    .write_audit_log(user_id, amount, log_type::HOLD, &request_id, None)
                    .await;
            });
        } else {
            self.write_audit_log(user_id, amount, log_type::HOLD, &request_id, None)
                .await;
        }
        Ok(())
    }

    /// The amount currently reserved for `(user_id, request_id)`, or `None`
    /// when no reservation is live.
    ///
    /// Purely additive and read-only: the fallback settle path uses it as the
    /// lower bound `max(active_hold, estimate)` when the upstream reported no
    /// usage.
    ///
    /// # Errors
    /// [`LedgerError::InvalidArgument`] for an empty request id — a
    /// programmer error that must not be indistinguishable from "no hold" —
    /// and [`LedgerError::RedisNotConfigured`] without Redis.
    pub async fn active_hold_amount(
        &self,
        user_id: i64,
        request_id: &str,
    ) -> Result<Option<f64>, LedgerError> {
        if request_id.is_empty() {
            return Err(LedgerError::InvalidArgument("requestID is required"));
        }
        let Some(mut conn) = self.redis_conn() else {
            return Err(LedgerError::RedisNotConfigured);
        };
        Ok(conn.zscore(holds_key(user_id), request_id).await?)
    }

    /// Drops the reservation for `request_id` without touching the persistent
    /// balance, then invalidates the balance cache so the freed headroom is
    /// visible immediately.
    ///
    /// **Signature and semantics are a hard project constraint**
    /// (`AGENTS.md`). The Redis removals are best-effort — a release that
    /// cannot reach Redis still succeeds, and the reservation expires on its
    /// own via the Lua cutoff.
    ///
    /// # Errors
    /// [`LedgerError::InvalidArgument`] for an empty request id.
    pub async fn release(&self, user_id: i64, request_id: &str) -> Result<(), LedgerError> {
        if request_id.is_empty() {
            return Err(LedgerError::InvalidArgument("requestID is required"));
        }
        let Some(mut conn) = self.redis_conn() else {
            return Ok(());
        };

        if let Err(err) = Self::remove_hold(&mut conn, user_id, request_id).await {
            tracing::debug!(error = %err, user_id, request_id, "release: hold removal failed");
        }
        self.invalidate_balance_cache(user_id).await;
        self.write_audit_log(user_id, 0.0, log_type::RELEASE, request_id, None)
            .await;
        Ok(())
    }

    // -------------------------------------------------------------- settle

    /// Debits the actual cost from the persistent balance and clears the
    /// reservation.
    ///
    /// **Signature and semantics are a hard project constraint** (`AGENTS.md`).
    ///
    /// Partial-debit semantics:
    ///
    /// * `actual_amount <= 0` takes a fast path with no transaction at all:
    ///   clear the reservation, write a zero-amount `settle` audit row. This
    ///   is the "upstream attributed no cost" case.
    /// * Otherwise one transaction locks the user row, debits
    ///   `min(balance, actual_amount)`, writes the `settle` row for the
    ///   debited portion and — when the cost overran the balance — a second
    ///   zero-amount row carrying `metadata.shortfall_usd`. The Redis
    ///   reservation is cleared **only after that transaction commits**.
    ///
    /// A transaction failure returns the error and leaves the reservation in
    /// place for reconciliation. A post-commit Redis failure also returns an
    /// error, with the reservation still in place — an acceptable degraded
    /// state, since the debit is already durable and the hold expires on its
    /// own.
    ///
    /// Settle never returns [`LedgerError::InsufficientBalance`] for a
    /// shortfall. Callers that need to observe the debt read the
    /// `balance_logs` rows keyed by `reference = request_id`, or ask
    /// [`has_unresolved_shortfall`](Self::has_unresolved_shortfall).
    ///
    /// # Errors
    /// [`LedgerError::InvalidArgument`] for an empty request id;
    /// [`LedgerError::UserNotFound`] for an unknown user; otherwise the
    /// underlying query or Redis error.
    pub async fn settle(
        &self,
        user_id: i64,
        request_id: &str,
        actual_amount: f64,
    ) -> Result<(), LedgerError> {
        if request_id.is_empty() {
            return Err(LedgerError::InvalidArgument("requestID is required"));
        }

        // Fast path: nothing to debit, so keep zero-cost settlements cheap and
        // out of the database's transaction path entirely.
        if actual_amount <= 0.0 {
            if let Some(mut conn) = self.redis_conn()
                && let Err(err) = Self::remove_hold(&mut conn, user_id, request_id).await
            {
                tracing::debug!(error = %err, user_id, request_id, "settle: hold removal failed");
            }
            self.invalidate_balance_cache(user_id).await;
            self.write_audit_log(user_id, 0.0, log_type::SETTLE, request_id, None)
                .await;
            return Ok(());
        }

        // Partial-debit path. The persistent work is delegated to settle_tx so
        // the standalone settle and the transaction-aware caller (the usage
        // plugin, which also inserts the usage log) share one source of truth.
        let outcome = async {
            let mut tx = self.db.begin().await?;
            let outcome = self
                .settle_tx(&mut tx, user_id, request_id, actual_amount)
                .await?;
            tx.commit().await?;
            Ok::<_, LedgerError>(outcome)
        }
        .await;

        if let Err(err) = outcome {
            // The transaction failed. The reservation MUST survive so the
            // request can be reconciled or retried. Audit outside the failed
            // transaction; do not clear the hold.
            let reason = err.to_string();
            self.write_audit_log(
                user_id,
                0.0,
                log_type::SETTLE_FAILED,
                request_id,
                Some(json!({ "reason": reason })),
            )
            .await;
            return Err(err);
        }

        // Persistent writes committed; now — and only now — release Redis.
        self.clear_hold(user_id, request_id).await
    }

    /// The persistent half of a settle, run inside the caller's transaction.
    ///
    /// Locks the user row (`SELECT ... FOR UPDATE`), debits
    /// `min(balance, actual_amount)`, writes the `settle` row for the debited
    /// portion, and — when the cost overran the balance — a zero-amount
    /// shortfall marker row. It does **not** touch Redis.
    ///
    /// This exists so a caller can make the balance debit atomic with its own
    /// writes — the usage plugin's `usage_logs` insert and subscription
    /// accumulation — inside a single transaction. Settling separately would
    /// open a window where the balance moved but the usage log did not.
    ///
    /// After the caller's transaction commits it **must** call
    /// [`clear_hold`](Self::clear_hold) to release the reservation and
    /// invalidate the balance cache. On rollback it must not, so the hold
    /// survives for reconciliation.
    ///
    /// A non-positive `actual_amount` writes the zero-amount audit row
    /// (mirroring the standalone fast path) and settles nothing.
    ///
    /// # Errors
    /// [`LedgerError::InvalidArgument`] for an empty request id;
    /// [`LedgerError::UserNotFound`] for an unknown user; otherwise the
    /// underlying query error.
    pub async fn settle_tx(
        &self,
        conn: &mut PgConnection,
        user_id: i64,
        request_id: &str,
        actual_amount: f64,
    ) -> Result<SettleOutcome, LedgerError> {
        if request_id.is_empty() {
            return Err(LedgerError::InvalidArgument("requestID is required"));
        }

        if actual_amount <= 0.0 {
            Self::insert_balance_log(
                &mut *conn,
                user_id,
                0.0,
                log_type::SETTLE,
                request_id,
                Some(audit_metadata(user_id, None)),
            )
            .await?;
            return Ok(SettleOutcome::default());
        }

        let balance = Self::lock_balance(&mut *conn, user_id).await?;
        let split = Settlement::split(balance, actual_amount);

        Self::set_balance(&mut *conn, user_id, balance - split.debited).await?;

        // The debited portion. `amount` is negative-going like every other
        // balance-reducing row, and may be exactly zero when the user had
        // nothing left to take.
        Self::insert_balance_log(
            &mut *conn,
            user_id,
            -split.debited,
            log_type::SETTLE,
            request_id,
            Some(audit_metadata(
                user_id,
                Some(json!({ "actual_cost": split.debited })),
            )),
        )
        .await?;

        // The unpaid portion, as a zero-amount marker row. Downstream
        // reconciliation (has_unresolved_shortfall, the compensating credit)
        // pairs the debt with this request through it.
        let shortfall_log_id = if split.has_shortfall() {
            Some(
                Self::insert_balance_log(
                    &mut *conn,
                    user_id,
                    0.0,
                    log_type::SETTLE,
                    request_id,
                    Some(audit_metadata(
                        user_id,
                        Some(json!({
                            "shortfall_usd": split.shortfall,
                            "actual_cost": actual_amount,
                        })),
                    )),
                )
                .await?,
            )
        } else {
            None
        };

        Ok(SettleOutcome {
            debited: split.debited,
            shortfall: split.shortfall,
            shortfall_log_id,
        })
    }

    /// Removes the Redis reservation for `(user_id, request_id)` and
    /// invalidates the balance cache.
    ///
    /// Callers using [`settle_tx`](Self::settle_tx) **must** invoke this after
    /// their transaction commits, so the reservation is released only once the
    /// debit is durable. A ledger without Redis is a no-op success.
    ///
    /// Unlike [`release`](Self::release), a Redis failure here is reported
    /// rather than swallowed: the caller is mid-settle and needs to know the
    /// reservation is lingering (it will TTL-expire).
    ///
    /// # Errors
    /// The underlying Redis error.
    pub async fn clear_hold(&self, user_id: i64, request_id: &str) -> Result<(), LedgerError> {
        let Some(mut conn) = self.redis_conn() else {
            return Ok(());
        };
        Self::remove_hold(&mut conn, user_id, request_id).await?;
        self.invalidate_balance_cache(user_id).await;
        Ok(())
    }

    // ----------------------------------------------------------- shortfall

    /// Whether the user owns at least one `settle` row with a positive
    /// `metadata.shortfall_usd` that has not been paired with a compensating
    /// credit.
    ///
    /// A compensating credit is a `balance_logs` row where
    ///
    /// ```text
    /// type      = 'credit'
    /// reference = shortfall_resolve:<debit.reference>:<debit.id>
    /// ```
    ///
    /// (see [`shortfall_resolve_reference`]). Pinning the credit to the debt's
    /// `(reference, id)` pair is what makes an orphan credit — one naming a
    /// row that does not exist — incapable of clearing a real debt.
    ///
    /// One unresolved row is enough for a caller (the hold middleware's
    /// preflight, the subscription purchase preflight) to block billable work
    /// with HTTP 402 `outstanding_debt`.
    ///
    /// Read-only and independent of hold / settle / release. Callers must
    /// treat an error as "unknown" and fail open or closed per their policy;
    /// today's callers fail closed with a 500, matching the project's
    /// default-deny posture.
    ///
    /// This is Postgres-only, matching the production dialect.
    ///
    /// # Errors
    /// The underlying query error.
    pub async fn has_unresolved_shortfall(&self, user_id: i64) -> Result<bool, LedgerError> {
        // COALESCE guards against `->>` yielding NULL for a row whose metadata
        // predates the shortfall feature, so those rows drop out instead of
        // short-circuiting the comparison.
        let sql = "
SELECT EXISTS (
  SELECT 1 FROM balance_logs d
  WHERE d.user_id = $1
    AND d.type = $2
    AND COALESCE((d.metadata::jsonb ->> 'shortfall_usd')::float8, 0) > 0
    AND NOT EXISTS (
      SELECT 1 FROM balance_logs c
      WHERE c.user_id = d.user_id
        AND c.type = $3
        AND c.reference = 'shortfall_resolve:' || d.reference || ':' || d.id::text
    )
)";
        let unresolved: bool = sqlx::query_scalar(sql)
            .bind(user_id)
            .bind(log_type::SETTLE)
            .bind(log_type::CREDIT)
            .fetch_one(&self.db)
            .await?;
        Ok(unresolved)
    }

    /// Resolves one shortfall by crediting the user under the paired
    /// reference, which is what
    /// [`has_unresolved_shortfall`](Self::has_unresolved_shortfall) looks for.
    ///
    /// `shortfall_log_id` is the marker row's `balance_logs.id` — the value
    /// [`SettleOutcome::shortfall_log_id`] carries.
    ///
    /// Exposing this here keeps the one string format in one place.
    ///
    /// # Errors
    /// Same as [`credit`](Self::credit).
    pub async fn resolve_shortfall(
        &self,
        user_id: i64,
        request_id: &str,
        shortfall_log_id: i64,
        amount: f64,
    ) -> Result<(), LedgerError> {
        let reference = shortfall_resolve_reference(request_id, shortfall_log_id);
        self.credit(user_id, amount, &reference).await
    }

    // ------------------------------------------------------------ internals

    /// Loads the user's balance from Postgres and caches it in Redis.
    ///
    /// Public because operator tooling and the integration tests both need to
    /// prime the cache the Lua scripts read — a cold cache is otherwise only
    /// filled lazily, on the first hold.
    ///
    /// # Errors
    /// [`LedgerError::UserNotFound`], or the underlying query / Redis error.
    pub async fn refresh_balance_cache(&self, user_id: i64) -> Result<(), LedgerError> {
        let balance = self.db_balance(user_id).await?;
        let Some(mut conn) = self.redis_conn() else {
            return Ok(());
        };
        conn.set_ex::<_, _, ()>(
            balance_key(user_id),
            balance.to_string(),
            self.balance_ttl.as_secs(),
        )
        .await?;
        Ok(())
    }

    /// The persistent balance, straight from Postgres.
    async fn db_balance(&self, user_id: i64) -> Result<f64, LedgerError> {
        let balance: Option<f64> =
            sqlx::query_scalar(&format!("SELECT {BALANCE_EXPR} FROM users WHERE id = $1"))
                .bind(user_id)
                .fetch_optional(&self.db)
                .await?
                .ok_or(LedgerError::UserNotFound)?;
        Ok(balance.unwrap_or_default())
    }

    /// Drops the cached balance so the next read re-derives it from Postgres.
    /// Best-effort: a stale cache entry expires on its own within
    /// `balance_ttl`.
    async fn invalidate_balance_cache(&self, user_id: i64) {
        let Some(mut conn) = self.redis_conn() else {
            return;
        };
        if let Err(err) = conn.del::<_, ()>(balance_key(user_id)).await {
            tracing::debug!(error = %err, user_id, "failed to invalidate balance cache");
        }
    }

    /// Removes a reservation from both the sorted set and the timestamp hash.
    /// The pair must move together — a member left in either one alone would
    /// either freeze balance forever or resurrect as an ageless hold.
    async fn remove_hold(
        conn: &mut ConnectionManager,
        user_id: i64,
        request_id: &str,
    ) -> Result<(), redis::RedisError> {
        conn.zrem::<_, _, ()>(holds_key(user_id), request_id)
            .await?;
        conn.hdel::<_, _, ()>(holds_ts_key(user_id), request_id)
            .await?;
        Ok(())
    }

    /// Writes a `balance_logs` audit row, best-effort: a failure here is
    /// logged and swallowed, because losing an audit row must never fail the
    /// money operation that produced it.
    async fn write_audit_log(
        &self,
        user_id: i64,
        amount: f64,
        log_type: &str,
        reference: &str,
        extras: Option<serde_json::Value>,
    ) {
        let metadata = audit_metadata(user_id, extras);
        let mut conn = match self.db.acquire().await {
            Ok(conn) => conn,
            Err(err) => {
                tracing::warn!(error = %err, log_type, user_id, reference, "ledger: audit log connection failed");
                return;
            }
        };
        if let Err(err) = Self::insert_balance_log(
            &mut conn,
            user_id,
            amount,
            log_type,
            reference,
            Some(metadata),
        )
        .await
        {
            tracing::warn!(error = %err, log_type, user_id, reference, "ledger: failed to write audit log");
        }
    }

    /// `SELECT balance ... FOR UPDATE`: serializes concurrent movements on one
    /// user so a read-modify-write cannot lose an update.
    async fn lock_balance(conn: &mut PgConnection, user_id: i64) -> Result<f64, LedgerError> {
        let balance: Option<f64> = sqlx::query_scalar(&format!(
            "SELECT {BALANCE_EXPR} FROM users WHERE id = $1 FOR UPDATE"
        ))
        .bind(user_id)
        .fetch_optional(conn)
        .await?
        .ok_or(LedgerError::UserNotFound)?;
        Ok(balance.unwrap_or_default())
    }

    /// Writes the new balance. `updated_at` moves too, preserving the
    /// historical auto-stamping behaviour for this write.
    async fn set_balance(
        conn: &mut PgConnection,
        user_id: i64,
        balance: f64,
    ) -> Result<(), LedgerError> {
        sqlx::query("UPDATE users SET balance = $1, updated_at = NOW() WHERE id = $2")
            .bind(balance)
            .bind(user_id)
            .execute(conn)
            .await?;
        Ok(())
    }

    /// Appends one journal row and returns its id.
    async fn insert_balance_log(
        conn: &mut PgConnection,
        user_id: i64,
        amount: f64,
        log_type: &str,
        reference: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<i64, LedgerError> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO balance_logs (user_id, amount, type, reference, metadata, created_at) \
             VALUES ($1, $2, $3, $4, $5, NOW()) RETURNING id",
        )
        .bind(user_id)
        .bind(amount)
        .bind(log_type)
        .bind(reference)
        .bind(metadata)
        .fetch_one(conn)
        .await?;
        Ok(id)
    }

    /// A cloned Redis handle, or `None` when the ledger was built without one.
    /// `ConnectionManager` is a cheap, internally shared multiplexed handle.
    pub(crate) fn redis_conn(&self) -> Option<ConnectionManager> {
        self.redis.clone()
    }

    /// Wall-clock seconds. The Lua expiry cutoff is `now - hold_ttl`, so this
    /// must be the same clock the timestamps in the hash were written with.
    pub(crate) fn now_unix() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    }
}

/// Runs [`GET_BALANCE_SCRIPT`] and returns its raw string reply.
async fn run_get_balance(
    conn: &mut ConnectionManager,
    user_id: i64,
    now: i64,
    hold_ttl_secs: i64,
) -> Result<String, redis::RedisError> {
    let mut inv = GET_BALANCE_SCRIPT.prepare_invoke();
    inv.key(balance_key(user_id));
    inv.key(holds_key(user_id));
    inv.key(holds_ts_key(user_id));
    inv.arg(now);
    inv.arg(hold_ttl_secs);
    inv.invoke_async(conn).await
}

/// Runs [`HOLD_SCRIPT`] and returns its raw string reply (`OK` on admission).
async fn run_hold(
    conn: &mut ConnectionManager,
    user_id: i64,
    amount: &str,
    request_id: &str,
    now: i64,
    hold_ttl_secs: i64,
) -> Result<String, redis::RedisError> {
    let mut inv = HOLD_SCRIPT.prepare_invoke();
    inv.key(balance_key(user_id));
    inv.key(holds_key(user_id));
    inv.key(holds_ts_key(user_id));
    inv.arg(amount);
    inv.arg(request_id);
    inv.arg(now);
    inv.arg(hold_ttl_secs);
    inv.arg(""); // idempotency key (unused for now, kept for parity)
    inv.invoke_async(conn).await
}

/// Builds the JSON payload for a `balance_logs.metadata` column: always
/// `user_id` + `timestamp`, plus whatever the caller merges in.
fn audit_metadata(user_id: i64, extras: Option<serde_json::Value>) -> serde_json::Value {
    let mut base = json!({
        "user_id": user_id,
        "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    });
    if let (Some(serde_json::Value::Object(extras)), Some(obj)) = (extras, base.as_object_mut()) {
        for (k, v) in extras {
            obj.insert(k, v);
        }
    }
    base
}

/// Whether a Lua reply was the `CACHE_MISS` marker, meaning the user's balance
/// is not cached and must be loaded from Postgres first.
///
/// Checks the parsed error code *and* the rendered message, so a change in
/// how redis-rs classifies a custom `error_reply` cannot silently turn a
/// retryable miss into a hard failure.
fn is_cache_miss(err: &redis::RedisError) -> bool {
    marks(err, CACHE_MISS)
}

/// Whether a Lua reply was the `INSUFFICIENT_BALANCE:<available>` refusal.
fn is_insufficient_balance(err: &redis::RedisError) -> bool {
    marks(err, INSUFFICIENT_BALANCE)
}

fn marks(err: &redis::RedisError, marker: &str) -> bool {
    err.code().is_some_and(|c| c.contains(marker)) || err.to_string().contains(marker)
}

#[cfg(test)]
mod tests;
