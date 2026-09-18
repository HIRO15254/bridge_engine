//! Deals and boards.

use crate::{Card, Hand, Seat, Vulnerability};

/// Four hands of thirteen cards covering the whole deck.
///
/// The invariant (each hand has 13 cards, the hands are disjoint, their union is the deck) is
/// checked once by [`Deal::new`]; afterwards it can be relied on without further checks.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Deal {
    hands: [Hand; 4],
}

impl Deal {
    /// Validates and builds a deal. `hands` is indexed by [`Seat`].
    ///
    /// The check is O(1): each hand must have 13 cards and the bitwise OR of the four hands must
    /// equal the full deck (which, given the counts, implies disjointness).
    pub fn new(hands: [Hand; 4]) -> Result<Deal, DealError> {
        todo!("phase 1")
    }

    /// The hand of `seat`.
    pub const fn hand(&self, seat: Seat) -> Hand {
        self.hands[seat.index() as usize]
    }

    /// All four hands indexed by [`Seat`].
    pub const fn hands(&self) -> [Hand; 4] {
        self.hands
    }

    /// The seat holding `card`.
    pub fn owner(&self, card: Card) -> Seat {
        todo!("phase 1")
    }
}

/// A numbered board: deal plus dealer and vulnerability.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Board {
    /// Board number (1-based; 0 is accepted and treated like 16 for conditions).
    pub number: u16,
    /// Dealer.
    pub dealer: Seat,
    /// Vulnerability.
    pub vulnerability: Vulnerability,
    /// The cards.
    pub deal: Deal,
}

impl Board {
    /// A board whose dealer and vulnerability follow the standard schedule for `number`.
    pub fn new(number: u16, deal: Deal) -> Board {
        Board {
            number,
            dealer: Seat::dealer_of_board(number),
            vulnerability: Vulnerability::from_board_number(number),
            deal,
        }
    }

    /// A board with explicit conditions (PBN records may override the schedule).
    pub fn with_conditions(
        number: u16,
        dealer: Seat,
        vulnerability: Vulnerability,
        deal: Deal,
    ) -> Board {
        Board {
            number,
            dealer,
            vulnerability,
            deal,
        }
    }
}

/// The hands offered to [`Deal::new`] do not form a deal.
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum DealError {
    /// A hand does not have exactly 13 cards.
    #[error("{seat} holds {count} cards, expected 13")]
    HandSize {
        /// The seat.
        seat: Seat,
        /// Its card count.
        count: u8,
    },
    /// A card appears in two hands.
    #[error("card {0} is held by two seats")]
    Duplicate(Card),
}
