//! `Atom::intersection_is_trivially_unsat` answers exactly what
//! `a.intersect(&b).is_trivially_unsat()` does, for normalized and raw atoms alike (card masks
//! drawn from a small pool so that both sides constrain the same mask often).

mod common;

use bridge_constraint::{Atom, CardRequirement};
use bridge_core::{Hand, Holding, Rank, ShapeSet, Suit};
use common::{arb_atom_safe, arb_shapeset};
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

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
