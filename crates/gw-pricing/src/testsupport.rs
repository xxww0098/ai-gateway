//! Helpers shared by the unit tests: deterministic randomness, and a
//! constructor for the entity row the cache stores.
//!
//! This workspace pins no property-testing crate, so the tests below drive
//! their invariants with a seeded SplitMix64. Seeded, not sampled from the
//! clock: a failure reproduces by re-running the test.

use chrono::{DateTime, Utc};
use gw_model::ModelPrice;

/// One `model_prices` row with the four prices set and everything else at a
/// fixed placeholder.
///
/// The cache reads none of the remaining columns, but [`ModelPrice`] is the
/// whole entity, so the timestamps have to be *some* value. Pinning them to
/// the epoch keeps the tests independent of the clock.
pub(crate) fn priced(
    model_id: &str,
    input: f64,
    output: f64,
    cached: f64,
    reasoning: f64,
) -> ModelPrice {
    ModelPrice {
        id: 1,
        model_id: model_id.to_string(),
        input_price_per_1m: input,
        output_price_per_1m: output,
        cached_input_price_per_1m: cached,
        reasoning_price_per_1m: reasoning,
        created_at: DateTime::<Utc>::UNIX_EPOCH,
        updated_at: DateTime::<Utc>::UNIX_EPOCH,
    }
}

/// SplitMix64 — small, fast, and good enough for spraying inputs across a
/// range. Not cryptographic, and deliberately not used for anything but tests.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        // 53 significant bits is exactly an f64 mantissa, so this is uniform
        // over representable values rather than biased by rounding.
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `[lo, hi)`.
    pub(crate) fn f64_range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + self.unit() * (hi - lo)
    }

    /// Uniform in `[lo, hi]`, inclusive on both ends.
    pub(crate) fn i64_range(&mut self, lo: i64, hi: i64) -> i64 {
        debug_assert!(lo <= hi);
        let span = (hi - lo) as u64 + 1;
        lo + (self.next_u64() % span) as i64
    }

    pub(crate) fn bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}
