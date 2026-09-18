//! Errors of the text formats.

use crate::Card;

/// A text form of a core type could not be parsed.
///
/// These parsers are deliberately small (single cards, holdings, hands, deal strings, calls);
/// full PBN and LIN live in `bridge-format`.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum ParseError {
    /// Unknown suit symbol.
    #[error("unknown suit symbol {0:?}")]
    Suit(char),
    /// Unknown rank symbol.
    #[error("unknown rank symbol {0:?}")]
    Rank(char),
    /// Unknown seat symbol.
    #[error("unknown seat {0:?}")]
    Seat(char),
    /// A hand did not have four suit fields.
    #[error("expected 4 suits separated by '.', found {0}")]
    SuitCount(usize),
    /// A deal did not have four hands.
    #[error("expected 4 hands, found {0}")]
    HandCount(usize),
    /// The same card appeared twice.
    #[error("duplicate card {0}")]
    DuplicateCard(Card),
    /// Not a bid.
    #[error("invalid bid {0:?}")]
    Bid(String),
    /// Not a call.
    #[error("invalid call {0:?}")]
    Call(String),
    /// Not a vulnerability.
    #[error("invalid vulnerability {0:?}")]
    Vulnerability(String),
    /// Not a shape.
    #[error("invalid shape {0:?}")]
    Shape(String),
    /// Empty input.
    #[error("empty input")]
    Empty,
}
