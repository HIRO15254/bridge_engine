//! Core domain types for contract bridge.
//!
//! This crate is layer L0 of the `bridge` workspace: it has no dependencies (apart from an
//! optional `serde` feature), is meant to be immutable once released, and every other crate
//! builds on it.
//!
//! # Representation
//!
//! - [`Card`] is an index `0..52` with `index = suit * 13 + rank` (♣2 = 0, ♠A = 51).
//! - [`Holding`] is a 13-bit set of ranks in one suit (bit `rank`; Two = bit 0, Ace = bit 12).
//! - [`Hand`] is a 52-bit set of cards (bit `card.index()`); clubs occupy the low bits.
//! - [`Shape`] packs the four suit lengths into nibbles (clubs in the low nibble).
//! - [`ShapeClass`] is an order-free pattern such as 5-4-3-1 (39 classes for 13 cards).
//! - [`ShapeSet`] is a 560-bit set over every ordered 13-card shape.
//! - [`Bid`] is `0..35` (1♣ = 0, 7NT = 34) so that level comparison is integer comparison.
//!
//! # Text formats
//!
//! Text follows PBN: hands are written spades-first (`AKQ.234.AKQ.2345`), which is the reverse
//! of the internal bit order. Only the `Display`/`FromStr` implementations in [`fmt`] know this.
//!
//! # Invariants and panics
//!
//! Public constructors validate their input and return `Result`/`Option`. Panics are reserved for
//! contract violations that are checked with `debug_assert!` (for example a [`Shape`] that does
//! not total 13 cards being asked for its [`Shape::index`]).
#![forbid(unsafe_code)]
#![warn(missing_docs)]
// Phase 0 skeleton: the public surface is final, most bodies are `todo!()`.
// Remove these allows as the bodies are implemented.
#![allow(dead_code, unused_variables)]

mod auction;
mod call;
mod card;
mod dd_table;
mod deal;
mod error;
pub mod fmt;
mod hand;
mod holding;
mod play;
mod seat;
#[cfg(feature = "serde")]
mod serde_impls;
mod shape;

pub use auction::{Auction, AuctionError, CallsBy, LegalCalls};
pub use call::{Bid, Call, Contract, Doubling, Strain};
pub use card::{Card, Rank, Suit};
pub use dd_table::DdTable;
pub use deal::{Board, Deal, DealError};
pub use error::ParseError;
pub use hand::{Hand, HandCards};
pub use holding::{Holding, HoldingRanks, Submasks};
pub use play::{PlayError, PlayHistory, Trick, Tricks};
pub use seat::{Seat, Side, Vulnerability};
pub use shape::{
    CLASS_OF, CLASSES, MAX_HCP, MIN_HCP, SHAPES, Shape, ShapeClass, ShapeSet, ShapeSetIter,
    shape_index,
};
