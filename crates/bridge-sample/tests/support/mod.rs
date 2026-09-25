//! Shared fixtures for `bridge-sample`'s integration tests.
//!
//! Not a test binary itself: Cargo only auto-registers files directly under `tests/`, not
//! subdirectories, so this is pulled in with `mod support;` by the files that need it (each
//! integration test file is its own crate, so this module is compiled once per caller — hence
//! the blanket `#![allow(dead_code)]` below, since not every caller uses every item).

#![allow(dead_code)]

use bridge_bidding::{Explanation, Interpretation, ResolutionKind};
use bridge_constraint::{Atom, CardRequirement, HandConstraint, KnownCards, ShapeSet};
use bridge_core::{Card, Hand, Holding, Rank, Seat, Suit};

fn cards_to_hand(cards: &[Card]) -> Hand {
    cards.iter().fold(Hand::EMPTY, |h, &c| h.with(c))
}

fn empty_explanation() -> Explanation {
    Explanation {
        text: String::new(),
        node: None,
        resolution: ResolutionKind::Exact,
        parts: Vec::new(),
    }
}

fn atom_cards(cards: Vec<CardRequirement>) -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: ShapeSet::ALL,
        hcp: 0..=37,
        cards,
        eval: Vec::new(),
    })
}

fn atom_hcp(hcp: core::ops::RangeInclusive<u8>) -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: ShapeSet::ALL,
        hcp,
        cards: Vec::new(),
        eval: Vec::new(),
    })
}

/// A `SampleContext`-shaped fixture built to exercise two gaps a review found in
/// `tests/log_prob.rs`'s and `tests/determinism.rs`'s existing coverage: a re-prepared *middle*
/// seat (§6.4 (c) of `09-sample.md`) whose mixture has two genuinely overlapping components that
/// survive [`coarsen`](crate::constraint_proposal) unchanged (an HCP window, not a bare `cards`
/// atom, which coarsening would otherwise reduce to `ANY`), and a *last* seat that is `Sampled`
/// (not `Direct`) and can fail its own `satisfies` check for some residual pools, so `log_prob`'s
/// last-seat branch actually returns `-inf` for some deals instead of never being exercised (both
/// existing tests only ever left the last seat `Direct`/unconstrained).
///
/// Nine unknown cards: the ace, king and queen of spades, hearts and diamonds (clubs is a
/// complete suit fixed to North, `needed == 0`, and irrelevant to every constraint below). East
/// (`needed = 2`) must hold the spade ace; South (`needed = 3`) must hold 8-10 or 6-8 HCP among
/// its three pool cards (the windows overlap at 8, so both components can produce the same
/// hand); West (`needed = 4`, the residual seat) must hold the diamond ace. Every seat's own
/// fixed (non-pool) cards are worth 0 HCP (East's and South's fixed cards are plain spot cards;
/// West's fixed cards absorb the three jacks, irrelevant since West's own constraint is
/// card-identity, not HCP), so South's HCP constraint depends only on its own three drawn cards.
///
/// Mass ordering (§6.1 point 3, computed against the full 9-card pool): East `1.0 · C(8, 1) = 8`;
/// South `0.6 · 64 + 0.4 · 28 = 49.6` (64 and 28 are the counts of 3-card subsets of the 9-card
/// pool — three aces (4 pts), three kings (3 pts), three queens (2 pts) — with HCP in `8..=10`
/// and `6..=8` respectively); West `1.0 · C(8, 3) = 56`. East sorts first (cached), South second
/// (the re-prepared middle seat), West last.
pub struct MultiComponentContext {
    pub known: KnownCards,
    pub interpretation: Interpretation,
    pub spade_ace: Card,
    pub diamond_ace: Card,
}

pub fn multi_component_rejecting_last_seat() -> MultiComponentContext {
    let suits = [Suit::Spades, Suit::Hearts, Suit::Diamonds];
    let mut jacks = Hand::EMPTY;
    let mut zero_value = Hand::EMPTY;
    for &suit in &suits {
        let full = Hand::EMPTY.with_holding(suit, Holding::FULL);
        let top3 = Hand::EMPTY.with_holding(suit, Holding::top_ranks(3)); // A, K, Q
        let top4 = Hand::EMPTY.with_holding(suit, Holding::top_ranks(4)); // A, K, Q, J
        jacks = jacks.union(top4.difference(top3));
        zero_value = zero_value.union(full.difference(top4));
    }
    let clubs = Hand::EMPTY.with_holding(Suit::Clubs, Holding::FULL);

    let zero_cards: Vec<Card> = zero_value.cards().collect();
    assert_eq!(zero_cards.len(), 27);
    let south_fixed = cards_to_hand(&zero_cards[0..10]);
    let east_fixed = cards_to_hand(&zero_cards[10..21]);
    let west_zero = cards_to_hand(&zero_cards[21..27]);
    let west_fixed = west_zero.union(jacks);

    let known = KnownCards::new([clubs, east_fixed, south_fixed, west_fixed])
        .expect("the four fixed hands are pairwise disjoint by construction");
    assert_eq!(known.needed(Seat::North), 0);
    assert_eq!(known.needed(Seat::East), 2);
    assert_eq!(known.needed(Seat::South), 3);
    assert_eq!(known.needed(Seat::West), 4);
    assert_eq!(known.pool().len(), 9);

    let spade_ace = Card::new(Suit::Spades, Rank::Ace);
    let diamond_ace = Card::new(Suit::Diamonds, Rank::Ace);

    let holds_spade_ace = atom_cards(vec![CardRequirement::in_suit(
        Suit::Spades,
        Holding::top_ranks(1),
        1..=1,
    )]);
    let holds_diamond_ace = atom_cards(vec![CardRequirement::in_suit(
        Suit::Diamonds,
        Holding::top_ranks(1),
        1..=1,
    )]);
    let south_high = atom_hcp(8..=10);
    let south_low = atom_hcp(6..=8);

    let interpretation = Interpretation {
        seats: [
            Vec::new(),
            vec![(holds_spade_ace, 1.0, empty_explanation())],
            vec![
                (south_high, 0.6, empty_explanation()),
                (south_low, 0.4, empty_explanation()),
            ],
            vec![(holds_diamond_ace, 1.0, empty_explanation())],
        ],
        per_call: Vec::new(),
        divergence: None,
    };

    MultiComponentContext {
        known,
        interpretation,
        spade_ace,
        diamond_ace,
    }
}
