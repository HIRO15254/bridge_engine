//! A set of cards.

use crate::{Card, Holding, Shape, Suit};

/// A set of cards as a 52-bit mask: bit `card.index()` is set when the card is held.
///
/// A `Hand` is not required to hold 13 cards: it also represents the remaining cards of a
/// player mid-play, the cards already played, or the pool of unknown cards for a sampler.
/// Only [`Deal::new`](crate::Deal::new) enforces four hands of 13.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Hand(u64);

impl Hand {
    /// No cards.
    pub const EMPTY: Hand = Hand(0);
    /// All 52 cards.
    pub const FULL: Hand = Hand((1u64 << 52) - 1);

    /// Builds a hand from its bit pattern, or `None` if any bit above 51 is set.
    pub const fn from_bits(bits: u64) -> Option<Hand> {
        if bits >> 52 == 0 {
            Some(Hand(bits))
        } else {
            None
        }
    }

    /// The raw 52-bit pattern.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Builds a hand from four holdings given in suit order (clubs first).
    pub const fn from_holdings(
        clubs: Holding,
        diamonds: Holding,
        hearts: Holding,
        spades: Holding,
    ) -> Hand {
        Hand(
            clubs.bits() as u64
                | (diamonds.bits() as u64) << 13
                | (hearts.bits() as u64) << 26
                | (spades.bits() as u64) << 39,
        )
    }

    /// The cards of `suit`.
    pub const fn holding(self, suit: Suit) -> Holding {
        match Holding::from_bits(((self.0 >> suit.shift()) & 0x1FFF) as u16) {
            Some(h) => h,
            None => unreachable!(),
        }
    }

    /// This hand with the cards of `suit` replaced by `holding`.
    pub const fn with_holding(self, suit: Suit, holding: Holding) -> Hand {
        Hand((self.0 & !suit.mask()) | (holding.bits() as u64) << suit.shift())
    }

    /// Number of cards held.
    pub const fn len(self) -> u8 {
        self.0.count_ones() as u8
    }

    /// `true` when no card is held.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The four suit lengths as a [`Shape`].
    pub const fn shape(self) -> Shape {
        Shape::new(
            (self.0 & Suit::Clubs.mask()).count_ones() as u8,
            (self.0 & Suit::Diamonds.mask()).count_ones() as u8,
            (self.0 & Suit::Hearts.mask()).count_ones() as u8,
            (self.0 & Suit::Spades.mask()).count_ones() as u8,
        )
    }

    /// Whether `card` is held.
    pub const fn contains(self, card: Card) -> bool {
        self.0 & card.bit() != 0
    }

    /// This hand plus `card`.
    pub const fn with(self, card: Card) -> Hand {
        Hand(self.0 | card.bit())
    }

    /// This hand minus `card`.
    pub const fn without(self, card: Card) -> Hand {
        Hand(self.0 & !card.bit())
    }

    /// Set union.
    pub const fn union(self, other: Hand) -> Hand {
        Hand(self.0 | other.0)
    }

    /// Set intersection.
    pub const fn intersect(self, other: Hand) -> Hand {
        Hand(self.0 & other.0)
    }

    /// Set difference `self \ other`.
    pub const fn difference(self, other: Hand) -> Hand {
        Hand(self.0 & !other.0)
    }

    /// Complement within the deck (always masked to 52 bits).
    pub const fn complement(self) -> Hand {
        Hand(!self.0 & Hand::FULL.0)
    }

    /// `true` when the two hands share no card.
    pub const fn is_disjoint(self, other: Hand) -> bool {
        self.0 & other.0 == 0
    }

    /// `true` when `self ⊆ other`.
    pub const fn is_subset(self, other: Hand) -> bool {
        self.0 & !other.0 == 0
    }

    /// Cards held, in ascending index order (clubs first, low ranks first).
    pub fn cards(self) -> HandCards {
        HandCards { bits: self.0 }
    }
}

/// Iterator over the cards of a [`Hand`] in ascending index order.
#[derive(Clone, Debug)]
pub struct HandCards {
    bits: u64,
}

impl Iterator for HandCards {
    type Item = Card;

    fn next(&mut self) -> Option<Card> {
        todo!("phase 1")
    }
}

impl core::ops::BitOr for Hand {
    type Output = Hand;
    fn bitor(self, rhs: Hand) -> Hand {
        self.union(rhs)
    }
}

impl core::ops::BitAnd for Hand {
    type Output = Hand;
    fn bitand(self, rhs: Hand) -> Hand {
        self.intersect(rhs)
    }
}

impl core::ops::Sub for Hand {
    type Output = Hand;
    fn sub(self, rhs: Hand) -> Hand {
        self.difference(rhs)
    }
}

impl core::ops::Not for Hand {
    type Output = Hand;
    fn not(self) -> Hand {
        self.complement()
    }
}

impl core::fmt::Debug for Hand {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(self, f)
    }
}
