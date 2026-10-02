//! The lead advisor's query.

use bridge_core::{Auction, Hand};

/// What to advise on: a completed auction and the opening leader's own hand.
#[derive(Clone, Copy, Debug)]
pub struct LeadQuery<'a> {
    /// The auction, which must be complete and not passed out.
    pub auction: &'a Auction,
    /// The opening leader's 13 cards.
    pub leader_hand: Hand,
}
