//! Shared `proptest` strategies for the integration tests.

#![allow(dead_code)]

use core::ops::RangeInclusive;
use std::collections::HashMap;
use std::sync::Arc;

use bridge_constraint::{
    Atom, CardRequirement, CustomPred, DistMethod, EvalRequirement, HandConstraint, LtcMethod,
    Metric, Sampler, ShapeSet,
};
use bridge_core::{Card, Hand, Suit};
use proptest::prelude::*;
use proptest::sample::Index;
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;

/// A uniformly random 13-card hand (partial Fisher-Yates shuffle of the deck).
pub fn arb_hand13() -> impl Strategy<Value = Hand> {
    prop::collection::vec(any::<Index>(), 13).prop_map(|indices| {
        let mut cards: Vec<u8> = (0..52).collect();
        let mut hand = Hand::EMPTY;
        for (i, ix) in indices.iter().enumerate() {
            let j = i + ix.index(52 - i);
            cards.swap(i, j);
            hand = hand.with(Card::from_index(cards[i]).expect("index < 52"));
        }
        hand
    })
}

pub fn arb_suit() -> impl Strategy<Value = Suit> {
    (0u8..4).prop_map(Suit::from_index)
}

pub fn arb_ltc_method() -> impl Strategy<Value = LtcMethod> {
    prop_oneof![Just(LtcMethod::Classic), Just(LtcMethod::New)]
}

/// The distribution-point methods.
pub fn arb_dist_method() -> impl Strategy<Value = DistMethod> {
    prop_oneof![
        Just(DistMethod::GOREN_321),
        Just(DistMethod::DUMMY_531),
        Just(DistMethod::LongSuit),
        Just(DistMethod::BergenStarting),
    ]
}

/// Metrics that never route through `DistPoints`/`TotalPoints` (so every literal they produce is
/// an additive-feature candidate, matching `sampler::term::classify`'s `Controls`/`Losers`/
/// `QuickTricks`/`SuitQuality` arms).
pub fn arb_metric_safe() -> impl Strategy<Value = Metric> {
    prop_oneof![
        Just(Metric::Controls),
        arb_ltc_method().prop_map(Metric::Losers),
        Just(Metric::QuickTricks),
        arb_suit().prop_map(Metric::SuitQuality),
    ]
}

/// The shape-only [`DistMethod`]s (`DistMethod::is_shape_only`): the exact sampler filters shapes
/// (`DistPoints`) or shifts the HCP window (`TotalPoints`) for these instead of falling back to
/// rejection. Excludes `BergenStarting`, which always needs rejection (see
/// `sampler_rejection.rs`).
pub fn arb_dist_method_shape_only() -> impl Strategy<Value = DistMethod> {
    prop_oneof![
        Just(DistMethod::GOREN_321),
        Just(DistMethod::DUMMY_531),
        Just(DistMethod::LongSuit),
    ]
}

/// [`arb_metric_safe`] plus `DistPoints`/`TotalPoints` with a shape-only [`DistMethod`]: every
/// metric this produces is still handled exactly by the sampler (via its `dist_shape_filters`/
/// `total_shape_shifts` routing, not the additive-feature slot).
pub fn arb_metric_dist_shape_only() -> impl Strategy<Value = Metric> {
    prop_oneof![
        arb_metric_safe(),
        arb_dist_method_shape_only().prop_map(Metric::DistPoints),
        arb_dist_method_shape_only().prop_map(Metric::TotalPoints),
    ]
}

/// A random inclusive sub-range of `0..=max` (`lo <= hi`).
pub fn arb_range(max: u8) -> impl Strategy<Value = RangeInclusive<u8>> {
    (0..=max, 0..=max).prop_map(|(a, b)| a.min(b)..=a.max(b))
}

pub fn arb_eval_requirement_safe() -> impl Strategy<Value = EvalRequirement> {
    arb_metric_safe()
        .prop_flat_map(|metric| arb_range(metric.max()).prop_map(move |range| (metric, range)))
        .prop_map(|(metric, range)| EvalRequirement { metric, range })
}

/// Same as [`arb_eval_requirement_safe`], but drawn from [`arb_metric_dist_shape_only`].
pub fn arb_eval_requirement_dist_shape_only() -> impl Strategy<Value = EvalRequirement> {
    arb_metric_dist_shape_only()
        .prop_flat_map(|metric| arb_range(metric.max()).prop_map(move |range| (metric, range)))
        .prop_map(|(metric, range)| EvalRequirement { metric, range })
}

pub fn arb_card_requirement() -> impl Strategy<Value = CardRequirement> {
    any::<u64>().prop_flat_map(|bits| {
        let mask = Hand::from_bits(bits & Hand::FULL.bits()).expect("masked to 52 bits");
        let popcount = mask.len();
        (0..=popcount, 0..=popcount).prop_map(move |(a, b)| CardRequirement {
            mask,
            count: a.min(b)..=a.max(b),
        })
    })
}

/// A random product-of-ranges shape set (loose per-suit length bounds, tightened to the shapes
/// that actually total 13 cards).
pub fn arb_shapeset() -> impl Strategy<Value = ShapeSet> {
    (arb_range(13), arb_range(13), arb_range(13), arb_range(13)).prop_map(|(c, d, h, s)| {
        ShapeSet::from_suit_lens([
            (*c.start(), *c.end()),
            (*d.start(), *d.end()),
            (*h.start(), *h.end()),
            (*s.start(), *s.end()),
        ])
    })
}

/// A random atom using only the metrics [`arb_metric_safe`] covers.
pub fn arb_atom_safe() -> impl Strategy<Value = Atom> {
    (
        arb_shapeset(),
        arb_range(37),
        prop::collection::vec(arb_card_requirement(), 0..3),
        prop::collection::vec(arb_eval_requirement_safe(), 0..3),
    )
        .prop_map(|(shapes, hcp, cards, eval)| {
            let mut atom = Atom {
                shapes,
                hcp,
                cards,
                eval,
            };
            atom.normalize();
            atom
        })
}

/// Same as [`arb_atom_safe`], but its eval requirements may also draw a shape-only
/// `DistPoints`/`TotalPoints` metric (see [`arb_metric_dist_shape_only`]).
pub fn arb_atom_dist_shape_only() -> impl Strategy<Value = Atom> {
    (
        arb_shapeset(),
        arb_range(37),
        prop::collection::vec(arb_card_requirement(), 0..3),
        prop::collection::vec(arb_eval_requirement_dist_shape_only(), 0..3),
    )
        .prop_map(|(shapes, hcp, cards, eval)| {
            let mut atom = Atom {
                shapes,
                hcp,
                cards,
                eval,
            };
            atom.normalize();
            atom
        })
}

/// A handful of named, deterministic custom predicates for building [`HandConstraint::Custom`]
/// nodes in property tests.
pub fn custom_pred(i: usize) -> CustomPred {
    match i % 3 {
        0 => CustomPred {
            name: "5+ spades".to_string(),
            f: Arc::new(|h: Hand| h.holding(Suit::Spades).len() >= 5),
        },
        1 => CustomPred {
            name: "balanced".to_string(),
            f: Arc::new(|h: Hand| h.shape().is_balanced()),
        },
        _ => CustomPred {
            name: "no aces".to_string(),
            f: Arc::new(|h: Hand| bridge_eval::aces(h) == 0),
        },
    }
}

fn arb_leaf() -> impl Strategy<Value = HandConstraint> {
    prop_oneof![
        3 => arb_atom_safe().prop_map(HandConstraint::Atom),
        1 => (0usize..3).prop_map(|i| HandConstraint::Custom(custom_pred(i))),
    ]
}

/// A random constraint tree of `Atom`/`Custom` leaves combined with `Or`/`And`/`Not`, at most
/// `max_depth` levels deep.
pub fn arb_constraint(max_depth: u32) -> impl Strategy<Value = HandConstraint> {
    arb_leaf().prop_recursive(max_depth, 16, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 1..4).prop_map(HandConstraint::Or),
            prop::collection::vec(inner.clone(), 1..4).prop_map(HandConstraint::And),
            inner.prop_map(|c| HandConstraint::Not(Box::new(c))),
        ]
    })
}

/// Same as [`arb_constraint`], but never produces a `Custom` node (for serde round trips, which
/// cannot represent one).
pub fn arb_constraint_no_custom(max_depth: u32) -> impl Strategy<Value = HandConstraint> {
    arb_atom_safe()
        .prop_map(HandConstraint::Atom)
        .prop_recursive(max_depth, 16, 4, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 1..4).prop_map(HandConstraint::Or),
                prop::collection::vec(inner.clone(), 1..4).prop_map(HandConstraint::And),
                inner.prop_map(|c| HandConstraint::Not(Box::new(c))),
            ]
        })
}

// --- χ² uniformity check, shared by every test that draws real samples (not just `count()`) ---
//
// Same technique as `tests/sampler_chi_square.rs`'s standalone copy of this: a Lanczos `ln Gamma`
// plus the regularized incomplete gamma function give an exact (to float precision) upper-tail
// p-value for any degrees of freedom, so this needs no external stats crate.

/// Lanczos approximation to `ln(Gamma(x))` (g = 7, n = 9 coefficients), accurate to about
/// `1e-13` for `x > 0` - standard textbook constants (Numerical Recipes).
fn ln_gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const COEF: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_312e-7,
    ];
    if x < 0.5 {
        // Reflection formula (not hit by the `df/2 >= 1` inputs this module uses, kept for safety).
        (std::f64::consts::PI / (std::f64::consts::PI * x).sin()).ln() - ln_gamma(1.0 - x)
    } else {
        let x = x - 1.0;
        let t = x + G + 0.5;
        let mut a = COEF[0];
        for (i, &c) in COEF.iter().enumerate().skip(1) {
            a += c / (x + i as f64);
        }
        0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
    }
}

/// The regularized lower incomplete gamma function `P(a, x)` by its series expansion (valid for
/// `x < a + 1`; Numerical Recipes §6.2).
fn gamma_p_series(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let gln = ln_gamma(a);
    let mut ap = a;
    let mut sum = 1.0 / a;
    let mut del = sum;
    for _ in 0..200 {
        ap += 1.0;
        del *= x / ap;
        sum += del;
        if del.abs() < sum.abs() * 1e-14 {
            break;
        }
    }
    sum * (-x + a * x.ln() - gln).exp()
}

/// The regularized upper incomplete gamma function `Q(a, x)` by its continued fraction (valid for
/// `x >= a + 1`; Numerical Recipes §6.2).
fn gamma_q_cf(a: f64, x: f64) -> f64 {
    let gln = ln_gamma(a);
    let fpmin = 1e-300;
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / fpmin;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..200 {
        let an = -(f64::from(i)) * (f64::from(i) - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < fpmin {
            d = fpmin;
        }
        c = b + an / c;
        if c.abs() < fpmin {
            c = fpmin;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-14 {
            break;
        }
    }
    (-x + a * x.ln() - gln).exp() * h
}

/// `Q(a, x) = 1 - P(a, x)`, the regularized upper incomplete gamma function, picking whichever of
/// the series or the continued fraction converges quickly for the given `(a, x)`.
fn gamma_q(a: f64, x: f64) -> f64 {
    if x < a + 1.0 {
        1.0 - gamma_p_series(a, x)
    } else {
        gamma_q_cf(a, x)
    }
}

/// The upper-tail p-value of a chi-square statistic with `df` degrees of freedom:
/// `P(X > chi2) = Q(df/2, chi2/2)`, exact (to float precision) for any `df`, small or large.
pub fn chi_square_p_value(chi2: f64, df: f64) -> f64 {
    gamma_q(df / 2.0, chi2 / 2.0)
}

/// Draws `draws` samples from `sampler` and checks (a) every drawn hand is one of `satisfying`
/// and (b) the draws land uniformly across `satisfying` (a χ² goodness-of-fit test against the
/// uniform distribution). `satisfying` must already be `sampler.count()`'s own satisfying set
/// (e.g. from a brute-force enumeration): this exercises the sampler's actual `draw`, not just
/// `count()`/`log_prob()`, which are computed from the same weight tables `draw` samples from and
/// so would not necessarily catch a bug specific to `draw` itself (e.g. picking shapes or
/// suit-pair splits with the wrong weight, or the wrong holding within a bucket).
///
/// A no-op when `satisfying` has fewer than 2 hands (a χ² test needs at least 1 degree of freedom,
/// and a singleton satisfying set is uniform by construction).
pub fn assert_samples_are_uniform_and_satisfy(
    sampler: &Sampler,
    satisfying: &[Hand],
    seed: u64,
    draws: u64,
) {
    if satisfying.len() < 2 {
        return;
    }
    let mut index_of: HashMap<Hand, usize> = HashMap::new();
    for &h in satisfying {
        let next = index_of.len();
        index_of.insert(h, next);
    }
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
    let mut observed = vec![0u64; satisfying.len()];
    for _ in 0..draws {
        let sample = sampler
            .sample(&mut rng)
            .expect("an exact term's draw always finds an accepting hand");
        let idx = *index_of.get(&sample.hand).unwrap_or_else(|| {
            panic!(
                "drew a hand outside the enumerated satisfying set: {:?}",
                sample.hand
            )
        });
        observed[idx] += 1;
    }
    let expected = draws as f64 / satisfying.len() as f64;
    let chi2: f64 = observed
        .iter()
        .map(|&o| {
            let d = o as f64 - expected;
            d * d / expected
        })
        .sum();
    let df = (satisfying.len() - 1) as f64;
    let p = chi_square_p_value(chi2, df);
    // A looser threshold than `tests/sampler_chi_square.rs`'s standalone `1e-3` (used there for a
    // handful of fixed, single-shot checks): this runs once per proptest case (24+ per test by
    // default, each with a fixed RNG seed derived from the case's own draw count), so a `1e-3`
    // threshold would have a non-negligible chance of a spurious failure somewhere across a full
    // proptest run. `1e-6` is still tight enough to reject a sampler that always returns the same
    // hand, or one that draws a shape/suit-pair split uniformly instead of by weight (both give a
    // chi2 orders of magnitude beyond what any reasonable threshold here would accept).
    assert!(
        p > 1e-6,
        "chi2={chi2}, df={df}, p={p} (uniformity rejected), n={}",
        satisfying.len()
    );
}
