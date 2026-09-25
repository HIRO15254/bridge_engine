//! Shared `proptest` strategies for the integration tests.

#![allow(dead_code)]

use core::ops::RangeInclusive;
use std::sync::Arc;

use bridge_constraint::{
    Atom, CardRequirement, CustomPred, DistMethod, EvalRequirement, HandConstraint, LtcMethod,
    Metric, ShapeSet,
};
use bridge_core::{Card, Hand, Suit};
use proptest::prelude::*;
use proptest::sample::Index;

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

/// The distribution-point methods; usable only in tests marked `#[ignore]` (they call
/// `bridge_eval::distribution_points`, which is `todo!()` until bridge-eval 2.1 lands).
pub fn arb_dist_method() -> impl Strategy<Value = DistMethod> {
    prop_oneof![
        Just(DistMethod::GOREN_321),
        Just(DistMethod::DUMMY_531),
        Just(DistMethod::LongSuit),
        Just(DistMethod::BergenStarting),
    ]
}

/// Metrics that do not depend on `bridge_eval::distribution_points`.
pub fn arb_metric_safe() -> impl Strategy<Value = Metric> {
    prop_oneof![
        Just(Metric::Controls),
        arb_ltc_method().prop_map(Metric::Losers),
        Just(Metric::QuickTricks),
        arb_suit().prop_map(Metric::SuitQuality),
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
