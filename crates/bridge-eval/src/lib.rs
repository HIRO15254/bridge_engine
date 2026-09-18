//! Hand evaluation: pure functions from a [`Hand`] (or [`Holding`]) to a number.
//!
//! Whole-hand linear metrics (`hcp`, `controls`, honour counts) are computed with rank masks and
//! `popcnt` and need no tables. Per-suit metrics that are not linear in the cards (losing trick
//! count, quick tricks) are looked up in [`SUIT`], a `static` table indexed by the 13-bit
//! [`Holding`] and built at compile time.
//!
//! Everything here is a pure function of the bits; nothing knows about auctions or systems.
//! Which distribution-point method a bidding system assumes is declared in the system's
//! metadata and passed in as a [`DistMethod`].
#![forbid(unsafe_code)]
#![warn(missing_docs)]
// Phase 0 skeleton: the public surface is final, some bodies are `todo!()`.
#![allow(dead_code, unused_variables)]

mod dist;
mod half;
mod metrics;
mod tables;

pub use bridge_core::{Hand, Holding, Shape};
pub use dist::{DistMethod, LtcMethod, distribution_points, shape_points, total_points};
pub use half::Half;
pub use metrics::{
    ACES, JACKS, KINGS, QUEENS, TENS, aces, controls, hcp, holding_hcp, honors, jacks, kings,
    losers, losers_with, queens, quick_tricks, rank_mask, suit_quality, tens, top_honors,
};
pub use tables::{SUIT, SuitTables};
