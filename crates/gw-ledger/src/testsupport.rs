//! Deterministic randomness for the property-style unit tests.
//!
//! This workspace pins no property-testing crate, so the tests below drive
//! the same invariants with a seeded SplitMix64. Seeded, not sampled from the
//! clock: a failure reproduces by re-running the test.

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
}
