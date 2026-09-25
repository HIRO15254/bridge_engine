//! Rejection sampling (2.6): a `Custom` predicate and a DNF term truncated into a residual both
//! make `Sampler::is_exact()` false, but every accepted sample still satisfies the whole
//! constraint, `tries` is reported, and `log_prob` stays finite on a satisfying hand.

use core::ops::RangeInclusive;
use std::sync::Arc;

use bridge_constraint::{
    Atom, CustomPred, DnfOptions, EvalRequirement, HandConstraint, LtcMethod, Metric,
    SampleOptions, Sampler,
};
use bridge_core::{Hand, Suit};
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;

/// Five independent metrics, each split into four buckets covering its whole domain: an `And` of
/// five `Or`s estimates `4^5 = 1024` terms, well past the default cap of 256, so `to_dnf` moves
/// one group into every surviving term's `residual` (same construction as `dnf_overflow.rs`).
fn overflow_constraint() -> HandConstraint {
    let groups: [(Metric, [RangeInclusive<u8>; 4]); 5] = [
        (Metric::Controls, [0..=2, 3..=5, 6..=8, 9..=12]),
        (Metric::QuickTricks, [0..=3, 4..=7, 8..=11, 12..=16]),
        (
            Metric::Losers(LtcMethod::Classic),
            [0..=5, 6..=11, 12..=17, 18..=24],
        ),
        (
            Metric::SuitQuality(Suit::Clubs),
            [0..=0, 1..=1, 2..=2, 3..=5],
        ),
        (
            Metric::SuitQuality(Suit::Diamonds),
            [0..=0, 1..=1, 2..=2, 3..=5],
        ),
    ];
    let ors = groups.map(|(metric, ranges)| {
        let atoms = ranges.map(|range| {
            HandConstraint::Atom(Atom::ANY.with_eval(EvalRequirement { metric, range }))
        });
        HandConstraint::Or(atoms.to_vec())
    });
    HandConstraint::And(ors.to_vec())
}

#[test]
fn truncated_dnf_residual_is_checked_by_rejection() {
    let c = overflow_constraint();
    let dnf = c
        .to_dnf(&DnfOptions::default())
        .expect("Overflow::Residual never errors");
    assert!(dnf.truncated, "5 groups of 4 (estimate 1024) exceeds 256");
    assert!(dnf.terms.iter().all(|t| t.residual.is_some()));

    let sampler = Sampler::prepare(&c, Hand::FULL, Hand::EMPTY, &SampleOptions::default()).unwrap();
    assert!(
        !sampler.is_exact(),
        "a term carrying a residual always needs rejection"
    );

    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0xBEEF);
    let mut accepted = 0;
    for _ in 0..500 {
        if let Some(sample) = sampler.sample(&mut rng) {
            assert!(
                c.satisfies(sample.hand),
                "accepted hand {:?} violates the constraint",
                sample.hand
            );
            assert!(sample.tries >= 1);
            assert!(
                sample.log_prob.is_finite(),
                "log_prob should be finite for an accepted (hence satisfying) hand"
            );
            accepted += 1;
        }
    }
    assert!(accepted > 0, "expected at least some accepted samples");
}

#[test]
fn custom_predicate_is_checked_by_rejection() {
    let pred = CustomPred {
        name: "even hcp, 5+ spades".to_string(),
        f: Arc::new(|h: Hand| bridge_eval::hcp(h) % 2 == 0 && h.holding(Suit::Spades).len() >= 5),
    };
    let c = HandConstraint::Custom(pred);
    assert!(!c.is_samplable());

    let sampler = Sampler::prepare(&c, Hand::FULL, Hand::EMPTY, &SampleOptions::default()).unwrap();
    assert!(!sampler.is_exact());

    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0xCAFE);
    let mut accepted = 0;
    for _ in 0..2_000 {
        if let Some(sample) = sampler.sample(&mut rng) {
            assert!(c.satisfies(sample.hand));
            assert!(sample.tries >= 1);
            assert!(sample.log_prob.is_finite());
            accepted += 1;
        }
    }
    assert!(accepted > 0, "expected at least some accepted samples");
}

#[test]
fn disallowing_rejection_reports_not_samplable() {
    let pred = CustomPred {
        name: "always true".to_string(),
        f: Arc::new(|_: Hand| true),
    };
    let c = HandConstraint::Custom(pred);
    let opts = SampleOptions {
        allow_rejection: false,
        ..SampleOptions::default()
    };
    match Sampler::prepare(&c, Hand::FULL, Hand::EMPTY, &opts) {
        Err(err) => assert_eq!(err, bridge_constraint::PrepareError::NotSamplable),
        Ok(_) => panic!("a Custom predicate cannot be sampled without rejection"),
    }
}
