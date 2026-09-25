//! Regression tests for `HandConstraint::is_satisfiable`: it must never report a genuinely
//! satisfiable constraint as unsatisfiable (the unsafe direction), even when the DNF term that
//! covers a satisfying hand needs rejection (a `Custom`, a residual, `DistMethod::BergenStarting`,
//! or more additive features than the sampler's slot allows) and its 256-draw burn-in probe
//! happens to find no hit.

use core::ops::RangeInclusive;

use bridge_constraint::{Atom, DistMethod, EvalRequirement, HandConstraint, LtcMethod, Metric};
use bridge_core::{Card, Hand, Holding, Rank, Suit};

/// `DistMethod::BergenStarting` always needs rejection (it is not shape-only, §6.2), so any atom
/// using `TotalPoints(BergenStarting)` puts a real hand's satisfaction behind a burn-in probe.
/// A hand this rare (Bergen-adjusted total points of 38) is well outside what a 256-draw probe
/// over the whole deck reliably finds, so before the fix `is_satisfiable` reported `false` even
/// though `satisfies` reports `true`.
#[test]
fn is_satisfiable_true_for_a_rare_bergen_total_points_hand() {
    let atom = Atom::ANY.with_eval(EvalRequirement {
        metric: Metric::TotalPoints(DistMethod::BergenStarting),
        range: 34..=77,
    });
    let c = HandConstraint::Atom(atom);

    // AKQ of every suit (12 cards) plus the spade jack: 37 HCP, Bergen total points 38.
    let mut hand = Hand::EMPTY;
    for suit in Suit::ALL {
        hand = hand.with_holding(suit, Holding::top_ranks(3));
    }
    hand = hand.with(Card::new(Suit::Spades, Rank::Jack));
    assert_eq!(hand.len(), 13);
    assert_eq!(bridge_eval::hcp(hand), 37);
    assert_eq!(
        bridge_eval::total_points(hand, DistMethod::BergenStarting),
        38
    );

    assert!(c.satisfies(hand), "the hand should satisfy the atom");
    assert!(
        c.is_satisfiable(),
        "a constraint with a known satisfying hand must never be reported unsatisfiable"
    );
}

/// Two additive per-suit features (`Controls` and `Losers`) in one atom: with the default
/// `extra_features = 1`, the second one always falls back to rejection (§6.2), with no `Custom`
/// and no `BergenStarting` involved. A hand whose controls/losers combination is rare enough that
/// the burn-in probe misses it reproduces the same false-negative bug.
#[test]
fn is_satisfiable_true_for_a_rare_two_additive_feature_hand() {
    let atom = Atom::ANY
        .with_eval(EvalRequirement {
            metric: Metric::Controls,
            range: 0..=0,
        })
        .with_eval(EvalRequirement {
            metric: Metric::Losers(LtcMethod::Classic),
            range: 0..=8,
        });
    let c = HandConstraint::Atom(atom);

    // Clubs Q down to 2 (11 cards, no ace/king anywhere) plus diamonds Q-J.
    let mut hand = Hand::EMPTY;
    hand = hand.with_holding(
        Suit::Clubs,
        Holding::FULL.without(Rank::Ace).without(Rank::King),
    );
    hand = hand.with_holding(
        Suit::Diamonds,
        Holding::EMPTY.with(Rank::Queen).with(Rank::Jack),
    );
    assert_eq!(hand.len(), 13);
    assert_eq!(bridge_eval::controls(hand), 0);
    assert_eq!(
        bridge_eval::losers_with(hand, LtcMethod::Classic).halves(),
        8
    );

    assert!(c.satisfies(hand), "the hand should satisfy the atom");
    assert!(
        c.is_satisfiable(),
        "a constraint with a known satisfying hand must never be reported unsatisfiable"
    );
}

/// A constraint whose DNF is truncated (a term carries a `residual`, checked only by rejection)
/// but which is logically equivalent to `ANY` (each `Or` group covers its metric's whole domain,
/// so the `And` of all five groups is always true). `is_satisfiable` must still report `true`.
#[test]
fn is_satisfiable_true_when_the_covering_term_needs_a_residual() {
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
    let c = HandConstraint::And(ors.to_vec());

    let dnf = c
        .to_dnf(&bridge_constraint::DnfOptions::default())
        .expect("Overflow::Residual (the default) never errors");
    assert!(dnf.truncated, "5 groups of 4 (estimate 1024) exceeds 256");
    assert!(
        dnf.terms.iter().any(|t| t.residual.is_some()),
        "the term cap should have moved some child into a residual"
    );

    assert!(
        c.is_satisfiable(),
        "the constraint is logically equivalent to ANY and must be reported satisfiable"
    );
}
