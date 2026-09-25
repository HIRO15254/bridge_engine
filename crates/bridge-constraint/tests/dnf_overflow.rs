//! `to_dnf`'s blow-up cap (§4.3 step 3): an `And` of `Or`s whose product would exceed
//! `max_terms` is truncated into a rejection-checked residual under [`Overflow::Residual`], and
//! reported as [`DnfError::TooLarge`] under [`Overflow::Error`].

mod common;

use bridge_constraint::{
    Atom, DnfError, DnfOptions, EvalRequirement, HandConstraint, LtcMethod, Metric, Overflow,
};
use bridge_core::Suit;
use common::arb_hand13;
use core::ops::RangeInclusive;
use proptest::prelude::*;

/// Five independent metrics, each split into four non-overlapping buckets covering its whole
/// domain, so intersecting one atom per group never trips `Atom::is_trivially_unsat` (which
/// cannot see cross-metric interactions, only per-literal domain violations): `4^5 = 1024`
/// estimated terms, well above the default cap of 256.
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
fn residual_overflow_truncates() {
    let c = overflow_constraint();
    let dnf = c
        .to_dnf(&DnfOptions::default())
        .expect("Overflow::Residual never errors");
    assert!(
        dnf.truncated,
        "5 groups of 4 (estimate 1024) exceeds the default cap of 256"
    );
    assert_eq!(
        dnf.terms.len(),
        256,
        "one group (4) is moved to residual, leaving 4^4 = 256 exact combinations"
    );
    assert!(
        dnf.terms.iter().all(|t| t.residual.is_some()),
        "every surviving term should carry the moved-out group as a residual"
    );
}

#[test]
fn error_overflow_reports_too_large() {
    let c = overflow_constraint();
    let opts = DnfOptions {
        max_terms: 256,
        on_overflow: Overflow::Error,
    };
    let err = c
        .to_dnf(&opts)
        .expect_err("1024 estimated terms exceeds the 256 cap");
    assert_eq!(
        err,
        DnfError::TooLarge {
            estimated: 1024,
            max_terms: 256,
        }
    );
}

proptest! {
    /// Even truncated, the DNF stays equivalent to direct evaluation: rejection on the residual
    /// makes each term exact again.
    #[test]
    fn residual_overflow_stays_equivalent(hand in arb_hand13()) {
        let c = overflow_constraint();
        let dnf = c.to_dnf(&DnfOptions::default()).expect("Overflow::Residual never errors");
        let via_dnf = dnf.terms.iter().any(|term| term.satisfies(hand));
        prop_assert_eq!(c.satisfies(hand), via_dnf);
    }
}
