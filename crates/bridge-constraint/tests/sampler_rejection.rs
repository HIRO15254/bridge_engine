//! Rejection sampling (2.6): a `Custom` predicate and a DNF term truncated into a residual both
//! make `Sampler::is_exact()` false, but every accepted sample still satisfies the whole
//! constraint, `tries` is reported, and `log_prob` stays finite on a satisfying hand.

use core::ops::RangeInclusive;
use std::sync::Arc;

use bridge_constraint::{
    Atom, CustomPred, DistMethod, DnfOptions, EvalRequirement, HandConstraint, LtcMethod, Metric,
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

/// `DistMethod::BergenStarting` is the one `DistMethod` that is not shape-only
/// (`DistMethod::is_shape_only`): unlike `GOREN_321`/`DUMMY_531`/`LongSuit`, its "quality suit" and
/// adjust-3 terms also look at the cards, not just the shape, so `sampler::term::classify` cannot
/// route it through `dist_shape_filters`/`total_shape_shifts` and always falls back to rejection.
#[test]
fn bergen_starting_total_points_is_checked_by_rejection() {
    let atom = Atom::ANY.with_eval(EvalRequirement {
        metric: Metric::TotalPoints(DistMethod::BergenStarting),
        range: 10..=20,
    });
    let c = HandConstraint::Atom(atom);
    assert!(
        c.is_samplable(),
        "no Custom node, only a plain atom literal"
    );

    let sampler = Sampler::prepare(&c, Hand::FULL, Hand::EMPTY, &SampleOptions::default()).unwrap();
    assert!(
        !sampler.is_exact(),
        "BergenStarting is not shape-only, so it always needs rejection"
    );

    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x0B00B1E5);
    let mut accepted = 0;
    for _ in 0..2_000 {
        if let Some(sample) = sampler.sample(&mut rng) {
            assert!(
                c.satisfies(sample.hand),
                "accepted hand {:?} violates the constraint",
                sample.hand
            );
            assert!(sample.tries >= 1);
            assert!(sample.log_prob.is_finite());
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

/// A minimal `tracing::Subscriber` that only records whether *some* `WARN`-level event fired
/// while it was the default subscriber - just enough to check `Sampler::prepare` emits one for a
/// `Custom` literal (05-constraint.md §8.2 step 3), without depending on the `tracing-subscriber`
/// crate (not a workspace dependency).
struct WarnRecorder(Arc<std::sync::atomic::AtomicBool>);

impl tracing::Subscriber for WarnRecorder {
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if *event.metadata().level() == tracing::Level::WARN {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    fn enter(&self, _span: &tracing::span::Id) {}
    fn exit(&self, _span: &tracing::span::Id) {}
}

fn prepare_and_check_warn(c: &HandConstraint) -> bool {
    let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let subscriber = WarnRecorder(fired.clone());
    let _ = tracing::subscriber::with_default(subscriber, || {
        Sampler::prepare(c, Hand::FULL, Hand::EMPTY, &SampleOptions::default())
    });
    fired.load(std::sync::atomic::Ordering::SeqCst)
}

/// `is_samplable() == false` (a `Custom` literal is present) makes `Sampler::prepare` emit a
/// `tracing::warn!`, as `HandConstraint::is_samplable`'s doc comment says (05-constraint.md §8.2
/// step 3): before this fix, the doc comment made this same claim but `prepare` never actually
/// emitted anything, so nothing observing `tracing` output could tell rejection-degraded sampling
/// had occurred. A plain, `Custom`-free atom must not warn.
#[test]
fn custom_predicate_makes_prepare_warn() {
    let pred = CustomPred {
        name: "5+ spades".to_string(),
        f: Arc::new(|h: Hand| h.holding(Suit::Spades).len() >= 5),
    };
    let with_custom = HandConstraint::Custom(pred);
    assert!(prepare_and_check_warn(&with_custom));

    let plain = HandConstraint::Atom(Atom::ANY.with_hcp(10..=17));
    assert!(!prepare_and_check_warn(&plain));
}

/// A rejection-only term's burn-in probe (256 draws, a seed fixed by the atom/pool/fixed alone)
/// can find zero hits even when the true acceptance rate is far from zero, because the probe's
/// draw sequence is the same for every `Custom` predicate sharing the same atom/pool/fixed (here,
/// `Atom::ANY` on the full deck with nothing fixed): whichever suit's honour-quad the 256 fixed
/// draws happen not to contain gives a zero-hit burn-in for that suit's predicate. Before the fix,
/// storing `alpha = 0.0` in that case made `log_prob` divide by (a clamped) zero, so an accepted
/// sample's reported probability was above 1 (`ln P > 0`). Regression: `log_prob` must never
/// exceed `0.0` (`P(hand) <= 1`) for any sample this sampler actually returns.
#[test]
fn log_prob_never_exceeds_zero_even_when_the_burn_in_probe_finds_no_hit() {
    for suit in Suit::ALL {
        let pred = CustomPred {
            name: format!("AKQJ of {suit:?}"),
            f: Arc::new(move |h: Hand| {
                let top4 = bridge_core::Holding::top_ranks(4);
                h.holding(suit).intersect(top4) == top4
            }),
        };
        let c = HandConstraint::Custom(pred);
        let sampler =
            Sampler::prepare(&c, Hand::FULL, Hand::EMPTY, &SampleOptions::default()).unwrap();
        assert!(!sampler.is_exact());

        let mut rng = Xoshiro256PlusPlus::seed_from_u64(0xABCD);
        let mut accepted = 0;
        for _ in 0..2_000 {
            if let Some(sample) = sampler.sample(&mut rng) {
                assert!(c.satisfies(sample.hand));
                assert!(
                    sample.log_prob <= 0.0,
                    "log_prob must never exceed 0 (P(hand) <= 1); suit={suit:?} got {}",
                    sample.log_prob
                );
                accepted += 1;
            }
        }
        assert!(
            accepted > 0,
            "expected at least some accepted samples for {suit:?}"
        );
    }
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
