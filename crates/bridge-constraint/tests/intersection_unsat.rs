//! `Atom::intersection_is_trivially_unsat` answers exactly what
//! `a.intersect(&b).is_trivially_unsat()` does, for normalized and raw atoms alike (card masks
//! drawn from a small pool so that both sides constrain the same mask often), and both agree
//! with `satisfies`: the intersection holds exactly where both atoms hold, and a pair that some
//! hand satisfies is never reported unsatisfiable.

mod common;

use bridge_constraint::{Atom, CardRequirement};
use bridge_core::{Hand, Holding, Rank, ShapeSet, Suit};
use common::{arb_atom_safe, arb_hand13, arb_shapeset};
use proptest::prelude::*;

fn mask_pool() -> Vec<Hand> {
    let top = |suit: Suit, ranks: &[Rank]| {
        Hand::EMPTY.with_holding(suit, ranks.iter().fold(Holding::EMPTY, |h, &r| h.with(r)))
    };
    let aces = Suit::ALL
        .into_iter()
        .fold(Hand::EMPTY, |h, s| h.union(top(s, &[Rank::Ace])));
    vec![
        top(Suit::Spades, &[Rank::Ace, Rank::King]),
        top(Suit::Spades, &[Rank::Ace, Rank::King, Rank::Queen]),
        top(Suit::Hearts, &[Rank::Ace]),
        aces,
    ]
}

fn arb_card_from_pool() -> impl Strategy<Value = CardRequirement> {
    (0..4usize, 0..=5u8, 0..=5u8).prop_map(|(i, a, b)| CardRequirement {
        mask: mask_pool()[i],
        // Unordered on purpose: an empty or over-long count range is a contradiction too.
        count: a..=b,
    })
}

fn arb_raw_atom() -> impl Strategy<Value = Atom> {
    (
        arb_shapeset(),
        0..=40u8,
        0..=40u8,
        prop::collection::vec(arb_card_from_pool(), 0..4),
    )
        .prop_map(|(shapes, lo, hi, cards)| Atom {
            shapes,
            hcp: lo..=hi,
            cards,
            eval: Vec::new(),
        })
}

/// A raw atom built around `anchor`, so that pairs of them often hold for the same hand: every
/// shape, shapes near the anchor's, or random ones; HCP bounds at 0 and 37 (the ends `normalize`
/// and the HCP reachability masks treat specially), near the anchor's HCP, or anywhere in
/// `0..=40` (unordered: an empty range too); and card requirements from the mask pool whose
/// count is the anchor's own (give or take one) or random. The anchor satisfies each literal
/// most of the time, and each literal is sometimes unsatisfiable or contradicts the other atom.
fn arb_raw_atom_near(anchor: Hand) -> impl Strategy<Value = Atom> {
    let points = bridge_eval::hcp(anchor);
    let lens = anchor.shape().lens();
    let shapes = prop_oneof![
        2 => Just(ShapeSet::ALL),
        2 => (0..=1u8, 0..=1u8).prop_map(move |(below, above)| {
            ShapeSet::from_suit_lens(lens.map(|l| (l.saturating_sub(below), (l + above).min(13))))
        }),
        1 => arb_shapeset(),
    ];
    let lo = prop_oneof![
        3 => Just(0u8),
        3 => (0..=2u8).prop_map(move |d| points.saturating_sub(d)),
        1 => Just(37u8),
        1 => 0..=40u8,
    ];
    let hi = prop_oneof![
        3 => Just(37u8),
        3 => (0..=2u8).prop_map(move |d| points + d),
        1 => Just(0u8),
        1 => 0..=40u8,
    ];
    let card = (0..4usize, 0..4u8, 0..=5u8, 0..=5u8).prop_map(move |(i, kind, a, b)| {
        let mask = mask_pool()[i];
        let count = if kind < 3 {
            let held = anchor.intersect(mask).len();
            held.saturating_sub(a % 2)..=held + b % 2
        } else {
            a..=b
        };
        CardRequirement { mask, count }
    });
    (shapes, lo, hi, prop::collection::vec(card, 0..4)).prop_map(|(shapes, lo, hi, cards)| Atom {
        shapes,
        hcp: lo..=hi,
        cards,
        eval: Vec::new(),
    })
}

/// Two atoms built around one random hand, and that hand with 19 more random ones.
fn arb_pair_and_hands() -> impl Strategy<Value = (Atom, Atom, Vec<Hand>)> {
    arb_hand13().prop_flat_map(|anchor| {
        (
            arb_raw_atom_near(anchor),
            arb_raw_atom_near(anchor),
            prop::collection::vec(arb_hand13(), 19),
        )
            .prop_map(move |(a, b, mut hands)| {
                hands.push(anchor);
                (a, b, hands)
            })
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// Raw and normalized atoms: `(a ∩ b).satisfies(h) == a.satisfies(h) && b.satisfies(h)`,
    /// `normalize` keeps `satisfies`, and a pair that some hand satisfies is never trivially
    /// unsatisfiable (the soundness the exclusive-region subtraction relies on).
    #[test]
    fn intersection_soundness_against_satisfies((a, b, hands) in arb_pair_and_hands()) {
        let (mut an, mut bn) = (a.clone(), b.clone());
        an.normalize();
        bn.normalize();
        for (x, y) in [(&a, &b), (&an, &bn)] {
            let both = x.intersect(y);
            let unsat = x.intersection_is_trivially_unsat(y);
            for &hand in &hands {
                let each = x.satisfies(hand) && y.satisfies(hand);
                prop_assert_eq!(both.satisfies(hand), each);
                prop_assert!(!(each && unsat), "{:?} and {:?} both hold for {:?}", x, y, hand);
            }
        }
        for &hand in &hands {
            prop_assert_eq!(an.satisfies(hand), a.satisfies(hand));
            prop_assert_eq!(bn.satisfies(hand), b.satisfies(hand));
        }
    }

    #[test]
    fn matches_the_built_intersection_on_raw_atoms(a in arb_raw_atom(), b in arb_raw_atom()) {
        prop_assert_eq!(
            a.intersection_is_trivially_unsat(&b),
            a.intersect(&b).is_trivially_unsat()
        );
    }

    #[test]
    fn matches_the_built_intersection_on_normalized_atoms(
        a in arb_atom_safe(),
        b in arb_atom_safe(),
    ) {
        prop_assert_eq!(
            a.intersection_is_trivially_unsat(&b),
            a.intersect(&b).is_trivially_unsat()
        );
    }
}

#[test]
fn any_meets_any_and_disjoint_shapes_do_not() {
    assert!(!Atom::ANY.intersection_is_trivially_unsat(&Atom::ANY));
    let a = Atom {
        shapes: ShapeSet::from_suit_len(Suit::Spades, 5, 13),
        ..Atom::ANY
    };
    let b = Atom {
        shapes: ShapeSet::from_suit_len(Suit::Spades, 0, 4),
        ..Atom::ANY
    };
    assert!(a.intersection_is_trivially_unsat(&b));
}
