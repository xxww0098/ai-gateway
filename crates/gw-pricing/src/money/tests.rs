//! Properties of the checked newtypes.
//!
//! The interesting property is a *negative* one — "these values can never
//! exist inside the type" — so the tests sample a hostile value space rather
//! than restating the constructor's `if`s. Every hostile value below is
//! *computed* (`0.0 / 0.0`, `-tiny`, `-huge`), not spelled out to match a
//! literal in the implementation.

use super::*;

/// A deterministic value stream. Not cryptography — the only requirement is
/// that it walks a wide range of magnitudes without the test hard-coding them.
struct Lcg(u64);

impl Lcg {
    fn new() -> Self {
        Self(0x2545_F491_4F6C_DD1D)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    /// A finite magnitude spanning roughly 1e-9 .. 1e9.
    fn next_magnitude(&mut self) -> f64 {
        let mantissa = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        let exponent = (self.next_u64() % 19) as i32 - 9;
        mantissa * 10f64.powi(exponent)
    }
}

/// Every value a checked constructor must refuse, built by arithmetic rather
/// than transcribed from the implementation.
fn hostile_values() -> Vec<f64> {
    let zero = 0.0f64;
    let one = 1.0f64;
    let mut out = vec![
        zero / zero,
        one / zero,
        -one / zero,
        f64::MAX * 2.0,
        -(f64::MAX * 2.0),
        -f64::MIN_POSITIVE,
        -f64::EPSILON,
    ];
    let mut rng = Lcg::new();
    for _ in 0..64 {
        out.push(-rng.next_magnitude());
    }
    out
}

#[test]
fn money_rejects_every_hostile_value() {
    for value in hostile_values() {
        assert!(
            Money::new(value).is_err(),
            "Money accepted {value:?}, which is NaN, infinite or negative"
        );
    }
}

#[test]
fn unit_price_rejects_every_hostile_value() {
    for value in hostile_values() {
        assert!(
            UnitPrice::new(value).is_err(),
            "UnitPrice accepted {value:?}"
        );
    }
}

#[test]
fn rate_multiplier_rejects_every_hostile_value() {
    for value in hostile_values() {
        assert!(
            RateMultiplier::new(value).is_err(),
            "RateMultiplier accepted {value:?}"
        );
    }
}

#[test]
fn token_count_rejects_every_hostile_value() {
    for value in hostile_values() {
        assert!(
            TokenCount::from_f64(value).is_err(),
            "TokenCount::from_f64 accepted {value:?}"
        );
    }
    // The integer constructor has no NaN to refuse, only the sign.
    let mut rng = Lcg::new();
    for _ in 0..64 {
        let negative = -((rng.next_u64() % i64::MAX as u64) as i64) - 1;
        assert!(
            TokenCount::new(negative).is_err(),
            "TokenCount::new accepted {negative}"
        );
    }
}

#[test]
fn zero_is_a_valid_amount_price_multiplier_and_count() {
    // A free model, an unpriced column, a zero-rate group and an empty
    // response are all real states; only negative and non-finite are not.
    assert!(Money::new(0.0).is_ok());
    assert!(UnitPrice::new(0.0).is_ok());
    assert!(RateMultiplier::new(0.0).is_ok());
    assert!(TokenCount::new(0).is_ok());
    assert!(TokenCount::from_f64(0.0).is_ok());
}

#[test]
fn every_finite_non_negative_value_round_trips() {
    let mut rng = Lcg::new();
    for _ in 0..256 {
        let value = rng.next_magnitude();
        assert_eq!(Money::new(value).expect("finite").get(), value);
        assert_eq!(UnitPrice::new(value).expect("finite").get(), value);
        assert_eq!(RateMultiplier::new(value).expect("finite").get(), value);
    }
    for _ in 0..256 {
        let count = (rng.next_u64() % (1 << 40)) as i64;
        assert_eq!(TokenCount::new(count).expect("non-negative").get(), count);
    }
}

#[test]
fn arithmetic_cannot_produce_a_value_the_constructor_would_reject() {
    let mut rng = Lcg::new();
    for _ in 0..256 {
        let a = Money::new(rng.next_magnitude()).expect("finite");
        let b = Money::new(rng.next_magnitude()).expect("finite");
        let rate = RateMultiplier::new(rng.next_magnitude()).expect("finite");

        for produced in [
            a.checked_add(b).expect("finite sum"),
            a.checked_mul(rate).expect("finite product"),
            a.max(b),
            a.saturating_sub(b),
            b.saturating_sub(a),
        ] {
            assert!(
                Money::new(produced.get()).is_ok(),
                "arithmetic produced {produced:?}, which the constructor rejects"
            );
        }
    }
}

#[test]
fn saturating_sub_never_credits_the_tenant() {
    let mut rng = Lcg::new();
    for _ in 0..256 {
        let small = Money::new(rng.next_magnitude()).expect("finite");
        let large = small
            .checked_add(Money::new(rng.next_magnitude()).expect("finite"))
            .expect("finite");
        assert_eq!(small.saturating_sub(large), Money::ZERO);
        assert!(large.saturating_sub(small).get() >= 0.0);
    }
}

#[test]
fn checked_arithmetic_reports_overflow_instead_of_producing_infinity() {
    let huge = Money::new(f64::MAX).expect("finite");
    assert!(huge.checked_add(huge).is_err());
    let doubling = RateMultiplier::new(2.0).expect("finite");
    assert!(huge.checked_mul(doubling).is_err());
}

#[test]
fn or_identity_maps_every_unusable_multiplier_onto_one() {
    // A bad `groups.rate_multiplier` must not take the request down, and it
    // must not silently make the request free either.
    for value in hostile_values().into_iter().chain([0.0]) {
        assert_eq!(
            RateMultiplier::or_identity(value),
            RateMultiplier::ONE,
            "or_identity({value:?}) did not fall back to the identity"
        );
    }
    let mut rng = Lcg::new();
    for _ in 0..64 {
        let usable = rng.next_magnitude();
        if usable > 0.0 {
            assert_eq!(RateMultiplier::or_identity(usable).get(), usable);
        }
    }
}

#[test]
fn clamped_token_counts_are_never_negative_and_never_inflate() {
    let mut rng = Lcg::new();
    for _ in 0..256 {
        let raw = rng.next_u64() as i64;
        let clamped = TokenCount::clamped(raw);
        assert!(clamped.get() >= 0);
        // Clamping may only move a value *up to* zero, never up from a
        // legitimate count — otherwise it would over-bill.
        assert!(clamped.get() <= raw.max(0));
    }
}

#[test]
fn from_f64_refuses_values_beyond_the_integer_range_instead_of_saturating() {
    // `value as i64` saturates silently, so a 1e30 token count would become
    // `i64::MAX` — an astronomically over-priced request rather than an error.
    let beyond = i64::MAX as f64 * 4.0;
    assert!(matches!(
        TokenCount::from_f64(beyond),
        Err(ValueError::NotRepresentable { .. })
    ));
}
