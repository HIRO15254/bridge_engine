//! Text formats (`Display` / `FromStr`) for the core types.
//!
//! | Type | `Display` | `FromStr` accepts |
//! | --- | --- | --- |
//! | [`Suit`] | `C D H S` | also `♣♦♥♠`, lowercase |
//! | [`Rank`] | `2`..`9 T J Q K A` | also `10`, lowercase |
//! | [`Card`] | `SA` (suit first, PBN play style) | `SA` and `AS` |
//! | [`Holding`] | ranks descending, `AKQ`; empty for a void | any order; `-` for a void |
//! | [`Hand`] | PBN order spades-first `AKQ.234.AKQ.2345`; empty field for a void | `-` for a void; any rank order; partial hands allowed; duplicates rejected |
//! | [`Deal`] | `N:AKQ.234.AKQ.2345 <E> <S> <W>` clockwise from the named seat | any starting seat |
//! | [`Shape`] | `5=4=3=1` (spades first, `=` = fixed order) | |
//! | [`ShapeClass`] | `5-4-3-1` | |
//! | [`Seat`] | `N E S W` | lowercase, full names |
//! | [`Strain`] | `C D H S NT` | `N` |
//! | [`Bid`] | `1C`..`7NT` | `7N`, lowercase |
//! | [`Call`] | `Pass X XX 1C` (PBN) | `P Dbl Rdbl D R` |
//! | [`Contract`] | `4SX` (declarer printed separately, as PBN does) | `4SXX`, `3NT` |
//! | [`Vulnerability`] | `None NS EW All` (PBN) | `Love Both - NONE` |
//! | [`Auction`] | calls separated by spaces, dealer first | |
//!
//! Storage is clubs-first (bit layout) while text is spades-first (PBN). Only this module knows
//! about the two orders.

use core::fmt::{Display, Formatter, Result as FmtResult};
use core::str::FromStr;

use crate::{
    Auction, Bid, Call, Card, Contract, Deal, Hand, Holding, ParseError, Rank, Seat, Shape,
    ShapeClass, Strain, Suit, Vulnerability,
};

impl Display for Suit {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(match self {
            Suit::Clubs => "C",
            Suit::Diamonds => "D",
            Suit::Hearts => "H",
            Suit::Spades => "S",
        })
    }
}

impl FromStr for Suit {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Suit, ParseError> {
        todo!("phase 1")
    }
}

impl Display for Rank {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.to_char())
    }
}

impl FromStr for Rank {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Rank, ParseError> {
        todo!("phase 1")
    }
}

impl Display for Card {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}{}", self.suit().letter(), self.rank().to_char())
    }
}

impl FromStr for Card {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Card, ParseError> {
        todo!("phase 1")
    }
}

impl Display for Holding {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        todo!("phase 1")
    }
}

impl FromStr for Holding {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Holding, ParseError> {
        todo!("phase 1")
    }
}

impl Display for Hand {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        todo!("phase 1")
    }
}

impl FromStr for Hand {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Hand, ParseError> {
        todo!("phase 1")
    }
}

impl Display for Deal {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        todo!("phase 1")
    }
}

impl FromStr for Deal {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Deal, ParseError> {
        todo!("phase 1")
    }
}

impl Display for Shape {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let l = self.lens();
        write!(f, "{}={}={}={}", l[3], l[2], l[1], l[0])
    }
}

impl FromStr for Shape {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Shape, ParseError> {
        todo!("phase 1")
    }
}

impl Display for ShapeClass {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let l = self.lens();
        write!(f, "{}-{}-{}-{}", l[0], l[1], l[2], l[3])
    }
}

impl FromStr for ShapeClass {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<ShapeClass, ParseError> {
        todo!("phase 1")
    }
}

impl Display for Seat {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.letter())
    }
}

impl FromStr for Seat {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Seat, ParseError> {
        todo!("phase 1")
    }
}

impl Display for Strain {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(match self {
            Strain::Clubs => "C",
            Strain::Diamonds => "D",
            Strain::Hearts => "H",
            Strain::Spades => "S",
            Strain::NoTrump => "NT",
        })
    }
}

impl FromStr for Strain {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Strain, ParseError> {
        todo!("phase 1")
    }
}

impl Display for Bid {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}{}", self.level(), self.strain())
    }
}

impl FromStr for Bid {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Bid, ParseError> {
        todo!("phase 1")
    }
}

impl Display for Call {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Call::Pass => f.write_str("Pass"),
            Call::Double => f.write_str("X"),
            Call::Redouble => f.write_str("XX"),
            Call::Bid(b) => Display::fmt(b, f),
        }
    }
}

impl FromStr for Call {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Call, ParseError> {
        todo!("phase 1")
    }
}

impl Display for Contract {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        todo!("phase 1")
    }
}

impl FromStr for Contract {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Contract, ParseError> {
        todo!("phase 1")
    }
}

impl Display for Vulnerability {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(match self {
            Vulnerability::None => "None",
            Vulnerability::NS => "NS",
            Vulnerability::EW => "EW",
            Vulnerability::Both => "All",
        })
    }
}

impl FromStr for Vulnerability {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Vulnerability, ParseError> {
        todo!("phase 1")
    }
}

impl Display for Auction {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        todo!("phase 1")
    }
}
