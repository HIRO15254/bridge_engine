//! Replaying a deal through the systems.

use bridge_core::{Auction, Call, Deal, Seat, Vulnerability};

use crate::{BidChoice, BidContext, Diagnostic, Table, choose_bid};

/// Safety cap on the number of calls `replay` will make, in case bidding never terminates
/// (07-bidding.md §6.3); legality guarantees termination in practice long before this.
const MAX_CALLS: usize = 320;

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
    let mut auction = Auction::new(dealer, vul);
    let mut gaps = Vec::new();
    let mut diagnostics = Vec::new();

    while !auction.is_complete() && auction.calls().len() < MAX_CALLS {
        let seat = auction.next_seat();
        let system = &table.systems[seat.index() as usize];
        let hand = deal.hand(seat);
        let call = match choose_bid(system, hand, &auction, ctx) {
            BidChoice::Chosen(chosen) => {
                diagnostics.extend(chosen.diagnostics.iter().copied());
                chosen.call
            }
            BidChoice::NoCandidate(no_candidate) => {
                diagnostics.extend(no_candidate.diagnostics.iter().copied());
                gaps.push((auction.calls().len(), seat));
                Call::Pass
            }
        };
        auction
            .push(call)
            .expect("choose_bid returns a legal call, and Pass is always legal while incomplete");
    }

    Replay {
        auction,
        gaps,
        diagnostics,
    }
}
