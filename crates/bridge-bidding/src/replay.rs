//! Replaying a deal through the systems.

use bridge_core::{Auction, Deal, Seat, Vulnerability};

use crate::{BidContext, Diagnostic, Table};

/// The result of [`replay`].
#[derive(Clone, Debug)]
pub struct Replay {
    /// The completed auction.
    pub auction: Auction,
    /// `(call index, seat)` where `NoCandidate` forced a pass.
    pub gaps: Vec<(usize, Seat)>,
    /// System-definition problems noticed on the way.
    pub diagnostics: Vec<Diagnostic>,
}

/// Bids the deal out with `choose_bid` at every seat until the auction is complete. A
/// `NoCandidate` becomes a pass and a recorded gap.
pub fn replay(
    table: &Table,
    deal: &Deal,
    dealer: Seat,
    vul: Vulnerability,
    ctx: &BidContext<'_>,
) -> Replay {
    todo!("phase 3")
}
