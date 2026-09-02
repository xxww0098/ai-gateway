//! Checked numeric newtypes for the billing path.
//!
//! Every number that can become money passes through one of these on its way
//! in. The point is not documentation — it is that `f64` silently accepts
//! three values a price, an amount, a multiplier or a token count can never
//! legitimately hold:
//!
//! * `NaN`, which poisons every later comparison (`NaN < balance` is `false`,
//!   so a NaN cost slips past the "can the balance cover it" gate *and*
//!   past the "is it positive" gate),
//! * `+/-inf`, which turns one bad price row into an unbounded hold,
//! * a negative, which turns a *charge* into a *credit*.
//!
//! Construction is the only checkpoint: once a [`Money`] exists it is finite
//! and non-negative, so the ledger's arithmetic does not re-validate.
//!
//! Zero is accepted everywhere — a free model, a zero-token column and a
//! zero-cost settlement are all real states. What is rejected is
//! *non-finite* and *negative*.
//!
//! The representation stays `f64` (and `i64` for counts) this phase: the
//! Postgres money columns are `numeric` read back through `::float8`, and
//! migrating the whole workspace to `Decimal` is a separate change. These
//! types are the seam that migration would go through.

/// Why a checked value could not be constructed.
///
/// `kind` names the newtype so one error type serves all four without the
/// caller having to match on which constructor failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    /// `NaN`, `+inf` or `-inf`.
    #[error("{kind} must be finite")]
    NotFinite { kind: &'static str },
    /// Finite, but below zero.
    #[error("{kind} must not be negative")]
    Negative { kind: &'static str },
    /// An `f64` that cannot be represented as the target integer.
    #[error("{kind} is not representable as an integer count")]
    NotRepresentable { kind: &'static str },
    /// 一个自相矛盾的 usage 信封：某个**子集**列大过了它所属的总量
    /// （缓存输入大过输入，或思考 ⊂ 输出的方言下思考大过输出）。
    ///
    /// 拒绝而不是截断 —— 见 [`crate::normalize`]。
    #[error("{kind} exceeds the total it is part of")]
    Inconsistent { kind: &'static str },
}

/// Shared gate: finite and `>= 0`.
fn checked(kind: &'static str, value: f64) -> Result<f64, ValueError> {
    if !value.is_finite() {
        return Err(ValueError::NotFinite { kind });
    }
    if value < 0.0 {
        return Err(ValueError::Negative { kind });
    }
    Ok(value)
}

/// Declares one `f64`-backed non-negative newtype.
///
/// A macro rather than four hand-written copies because the *only* thing that
/// differs is the name in the error message; hand-copying invites one of them
/// to drift out of the check.
macro_rules! checked_f64 {
    ($(#[$meta:meta])* $name:ident, $kind:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
        pub struct $name(f64);

        impl $name {
            #[doc = concat!("The additive identity. Always constructible.")]
            pub const ZERO: Self = Self(0.0);

            #[doc = concat!("Constructs a checked `", stringify!($name), "`.")]
            ///
            /// # Errors
            /// [`ValueError::NotFinite`] for `NaN` / infinity,
            /// [`ValueError::Negative`] for a value below zero.
            pub fn new(value: f64) -> Result<Self, ValueError> {
                checked($kind, value).map(Self)
            }

            /// The underlying `f64`, guaranteed finite and non-negative.
            #[must_use]
            pub const fn get(self) -> f64 {
                self.0
            }

            /// Whether this is exactly zero.
            #[must_use]
            pub fn is_zero(self) -> bool {
                self.0 == 0.0
            }
        }

        impl TryFrom<f64> for $name {
            type Error = ValueError;
            fn try_from(value: f64) -> Result<Self, ValueError> {
                Self::new(value)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

checked_f64!(
    /// A USD amount: a hold, a debit, a shortfall, a computed cost.
    Money,
    "money"
);

checked_f64!(
    /// A per-1M-token price from one `model_prices` column.
    UnitPrice,
    "unit price"
);

checked_f64!(
    /// A group's linear price scaling. Zero means "this group pays nothing",
    /// which is a real configuration, so it is allowed.
    RateMultiplier,
    "rate multiplier"
);

impl Money {
    /// Adds two amounts, rejecting a sum that overflows to infinity.
    ///
    /// # Errors
    /// [`ValueError::NotFinite`] when the sum is not finite.
    pub fn checked_add(self, other: Self) -> Result<Self, ValueError> {
        Self::new(self.0 + other.0)
    }

    /// Scales an amount by a multiplier.
    ///
    /// # Errors
    /// [`ValueError::NotFinite`] when the product overflows to infinity.
    pub fn checked_mul(self, rate: RateMultiplier) -> Result<Self, ValueError> {
        Self::new(self.0 * rate.get())
    }

    /// The larger of two amounts. Total because both are known finite.
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        if other.0 > self.0 { other } else { self }
    }

    /// Saturating difference: never negative, so it cannot flip a debit into
    /// a credit. This is the shape every shortfall computation needs.
    #[must_use]
    pub fn saturating_sub(self, other: Self) -> Self {
        Self((self.0 - other.0).max(0.0))
    }
}

impl RateMultiplier {
    /// The identity multiplier — what an unconfigured group gets.
    pub const ONE: Self = Self(1.0);

    /// Sanitizes a stored multiplier: a non-finite, negative or zero value
    /// becomes [`ONE`](Self::ONE).
    ///
    /// This is the *reading* counterpart of [`new`](Self::new): a bad row in
    /// `groups.rate_multiplier` must not take the request down, and billing at
    /// the un-discounted rate is the safe direction to fail.
    #[must_use]
    pub fn or_identity(value: f64) -> Self {
        match Self::new(value) {
            Ok(rate) if !rate.is_zero() => rate,
            _ => Self::ONE,
        }
    }
}

/// A non-negative token count.
///
/// Backed by `i64` because that is what the `usage_logs` columns are
/// (`bigint`, per `CONTRACT.md` §3.5); the checking is against negatives,
/// which upstreams do occasionally report and which would otherwise produce a
/// negative cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct TokenCount(i64);

impl TokenCount {
    /// No tokens at all.
    pub const ZERO: Self = Self(0);

    /// Constructs a checked count.
    ///
    /// # Errors
    /// [`ValueError::Negative`] below zero.
    pub fn new(value: i64) -> Result<Self, ValueError> {
        if value < 0 {
            return Err(ValueError::Negative {
                kind: "token count",
            });
        }
        Ok(Self(value))
    }

    /// Constructs a count from a float, as JSON usage envelopes deliver it.
    ///
    /// # Errors
    /// [`ValueError::NotFinite`] for `NaN` / infinity,
    /// [`ValueError::Negative`] below zero, and
    /// [`ValueError::NotRepresentable`] when the value exceeds `i64`.
    pub fn from_f64(value: f64) -> Result<Self, ValueError> {
        let value = checked("token count", value)?;
        // `as` saturates in Rust, so an out-of-range float would silently
        // become `i64::MAX` — check the range instead of trusting the cast.
        if value > i64::MAX as f64 {
            return Err(ValueError::NotRepresentable {
                kind: "token count",
            });
        }
        Ok(Self(value.trunc() as i64))
    }

    /// Clamps an unchecked count instead of failing.
    ///
    /// Upstream usage envelopes are not ours to validate mid-stream: a
    /// negative token count means the upstream is wrong, and treating it as
    /// zero bills nothing for that column rather than *crediting* the tenant.
    #[must_use]
    pub const fn clamped(value: i64) -> Self {
        Self(if value < 0 { 0 } else { value })
    }

    /// The underlying count, guaranteed non-negative.
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }

    /// The count as an `f64` for the pricing arithmetic.
    #[must_use]
    pub fn as_f64(self) -> f64 {
        self.0 as f64
    }
}

impl TryFrom<i64> for TokenCount {
    type Error = ValueError;
    fn try_from(value: i64) -> Result<Self, ValueError> {
        Self::new(value)
    }
}

impl std::fmt::Display for TokenCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests;
