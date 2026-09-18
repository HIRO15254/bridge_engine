//! The deal string: `N:AKQ.234.AKQ.2345 <E> <S> <W>`.
//!
//! Hands are listed clockwise from the named seat; each hand is spades-first with `.` between
//! suits, `T` for ten, and `-` for an unknown hand (which yields a [`PartialDeal`]).

use bridge_core::{Deal, Seat};

use crate::{ParseError, pbn::PartialDeal};

/// Parses a deal string, allowing unknown (`-`) hands.
pub fn parse(input: &str) -> Result<PartialDeal, ParseError> {
    todo!("phase 1")
}

/// Writes a deal string starting from `first`, ranks descending.
pub fn write(deal: &Deal, first: Seat) -> String {
    todo!("phase 1")
}
