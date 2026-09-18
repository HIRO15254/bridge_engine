//! Strains, bids, calls and contracts.

use crate::{Seat, Suit};

/// A denomination: one of the four suits or notrump, in bidding order.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum Strain {
    Clubs = 0,
    Diamonds = 1,
    Hearts = 2,
    Spades = 3,
    NoTrump = 4,
}

impl Strain {
    /// All strains in bidding order.
    pub const ALL: [Strain; 5] = [
        Strain::Clubs,
        Strain::Diamonds,
        Strain::Hearts,
        Strain::Spades,
        Strain::NoTrump,
    ];

    /// The strain with index `i` (`0..5`).
    ///
    /// # Panics
    /// Panics if `i >= 5`.
    pub const fn from_index(i: u8) -> Strain {
        match i {
            0 => Strain::Clubs,
            1 => Strain::Diamonds,
            2 => Strain::Hearts,
            3 => Strain::Spades,
            4 => Strain::NoTrump,
            _ => panic!("strain index out of range"),
        }
    }

    /// Index `0..5`.
    pub const fn index(self) -> u8 {
        self as u8
    }

    /// The strain naming `suit`.
    pub const fn from_suit(suit: Suit) -> Strain {
        Strain::from_index(suit.index())
    }

    /// The trump suit, or `None` for notrump.
    pub const fn suit(self) -> Option<Suit> {
        match self {
            Strain::NoTrump => None,
            s => Some(Suit::from_index(s as u8)),
        }
    }

    /// `true` for hearts and spades.
    pub const fn is_major(self) -> bool {
        matches!(self, Strain::Hearts | Strain::Spades)
    }

    /// `true` for clubs and diamonds.
    pub const fn is_minor(self) -> bool {
        matches!(self, Strain::Clubs | Strain::Diamonds)
    }
}

/// A bid, stored as `0..35` with `1♣ = 0` and `7NT = 34`.
///
/// The single-integer encoding makes "higher bid" an integer comparison and lets system tables
/// be indexed directly.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Bid(u8);

impl Bid {
    /// The bid of `level` (`1..=7`) in `strain`, or `None` for an invalid level.
    pub const fn new(level: u8, strain: Strain) -> Option<Bid> {
        if level >= 1 && level <= 7 {
            Some(Bid((level - 1) * 5 + strain as u8))
        } else {
            None
        }
    }

    /// The bid with index `i`, or `None` if `i >= 35`.
    pub const fn from_index(i: u8) -> Option<Bid> {
        if i < 35 { Some(Bid(i)) } else { None }
    }

    /// Index `0..35`.
    pub const fn index(self) -> u8 {
        self.0
    }

    /// Level `1..=7`.
    pub const fn level(self) -> u8 {
        self.0 / 5 + 1
    }

    /// Denomination.
    pub const fn strain(self) -> Strain {
        Strain::from_index(self.0 % 5)
    }

    /// Tricks needed to make this bid (`6 + level`).
    pub const fn tricks_required(self) -> u8 {
        6 + self.level()
    }
}

impl core::fmt::Debug for Bid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(self, f)
    }
}

/// A call in the auction.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Call {
    /// Pass.
    Pass,
    /// Double.
    Double,
    /// Redouble.
    Redouble,
    /// A bid.
    Bid(Bid),
}

impl Call {
    /// Index `0..38` (Pass = 0, Double = 1, Redouble = 2, bids at `3 + bid.index()`), used by
    /// system tries and call-distribution tables.
    pub const fn index(self) -> u8 {
        match self {
            Call::Pass => 0,
            Call::Double => 1,
            Call::Redouble => 2,
            Call::Bid(b) => 3 + b.index(),
        }
    }

    /// The call with index `i`, or `None` if `i >= 38`.
    pub const fn from_index(i: u8) -> Option<Call> {
        match i {
            0 => Some(Call::Pass),
            1 => Some(Call::Double),
            2 => Some(Call::Redouble),
            _ => match Bid::from_index(i - 3) {
                Some(b) => Some(Call::Bid(b)),
                None => None,
            },
        }
    }

    /// The bid, if this call is one.
    pub const fn bid(self) -> Option<Bid> {
        match self {
            Call::Bid(b) => Some(b),
            _ => None,
        }
    }

    /// `true` for [`Call::Bid`].
    pub const fn is_bid(self) -> bool {
        matches!(self, Call::Bid(_))
    }
}

/// Whether the final contract is doubled or redoubled.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum Doubling {
    Undoubled,
    Doubled,
    Redoubled,
}

/// The final contract of a completed auction.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Contract {
    /// The last bid of the auction.
    pub bid: Bid,
    /// The first player of the declaring side to have named the contract's strain.
    pub declarer: Seat,
    /// Doubled state.
    pub doubling: Doubling,
}

impl Contract {
    /// The opening leader: declarer's left-hand opponent.
    pub const fn leader(self) -> Seat {
        self.declarer.next()
    }

    /// The dummy: declarer's partner.
    pub const fn dummy(self) -> Seat {
        self.declarer.partner()
    }
}
