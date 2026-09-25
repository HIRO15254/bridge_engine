//! Shared notation for the lead, signal and discard rule tables (design doc §7.1).
//!
//! Kept private: these are building blocks for [`crate::leads`] and [`crate::signals`], not part
//! of the crate's public surface.

use core::ops::RangeInclusive;

use bridge_constraint::{Atom, CardRequirement, HandConstraint};
use bridge_core::{Card, Hand, Holding, Rank, ShapeSet, Suit};

/// The cards of suit `u` ranked strictly above `r` (used by the 4th-best / 3rd-5th tables:
/// "exactly N cards above the led rank").
pub(crate) fn above(u: Suit, r: Rank) -> Hand {
    Hand::EMPTY.with_holding(u, Holding::top_ranks(12 - r.index()))
}

/// `{A, K, Q, J}` of suit `u`. Ten is deliberately excluded (design doc §11 item 5).
pub(crate) fn honors(u: Suit) -> Hand {
    Hand::EMPTY.with_holding(u, Holding::top_ranks(4))
}

/// A single card as a one-card [`Hand`] mask.
pub(crate) fn one(u: Suit, r: Rank) -> Hand {
    Hand::EMPTY.with(Card::new(u, r))
}

/// A card-count requirement: `popcount(hand ∩ mask) ∈ count`.
pub(crate) fn req(mask: Hand, count: RangeInclusive<u8>) -> CardRequirement {
    CardRequirement { mask, count }
}

/// Shapes whose length in `u` lies in `lo..=hi`.
pub(crate) fn len(u: Suit, lo: u8, hi: u8) -> ShapeSet {
    ShapeSet::from_suit_len(u, lo, hi)
}

/// Union of the single-length shape sets for each length in `set` (used for "even"/"odd" length
/// conditions, which are not a single contiguous range).
pub(crate) fn lens(u: Suit, set: &[u8]) -> ShapeSet {
    set.iter().fold(ShapeSet::EMPTY, |acc, &l| {
        acc.union(ShapeSet::from_suit_len(u, l, l))
    })
}

/// A single non-`ANY` atom as a [`HandConstraint`].
pub(crate) fn atom(cards: Vec<CardRequirement>) -> HandConstraint {
    HandConstraint::Atom(Atom { cards, ..Atom::ANY })
}

/// A single atom restricted to `shapes` as well as `cards`.
pub(crate) fn shaped_atom(shapes: ShapeSet, cards: Vec<CardRequirement>) -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes,
        cards,
        ..Atom::ANY
    })
}

/// One fired rule as `[(C, w), (ANY, 1 - w)]` (§7.1: "each rule returns a `Vec` summing to 1 for
/// one seat and one event, the rest always going to `ANY`").
pub(crate) fn branch(c: HandConstraint, w: f32) -> Vec<(HandConstraint, f32)> {
    vec![(c, w), (HandConstraint::ANY, 1.0 - w)]
}

/// The numeric rank (`Two = 2` .. `Ace = 14`); spot cards run `2..=9`.
pub(crate) fn numeric(r: Rank) -> u8 {
    r.index() + 2
}

/// Whether `r` is a spot card (`2..=9`), as opposed to an honour (`T`, `J`, `Q`, `K`, `A`).
pub(crate) fn is_spot(r: Rank) -> bool {
    numeric(r) <= 9
}

/// The three-way spot-card height used by the signal and discard tables (§7.1: "high" spots
/// (`>= 7`) and "low" spots (`<= 5`) are unambiguous; `Six` is the undecided middle case).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Height {
    High,
    Low,
    Mid,
}

pub(crate) fn height(r: Rank) -> Height {
    let n = numeric(r);
    if n >= 7 {
        Height::High
    } else if n <= 5 {
        Height::Low
    } else {
        Height::Mid
    }
}
