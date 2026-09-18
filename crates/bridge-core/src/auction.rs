//! The auction and its legality rules.

use crate::{Bid, Call, Contract, Seat, Vulnerability};

/// The sequence of calls of one board, together with the dealer and vulnerability.
///
/// Every constructor validates the calls against the Laws (17–19), so an `Auction` value is
/// always legal. Legality is this type's responsibility and never a system definition's: a
/// system that lists an illegal continuation is reported by the compiler and the bidder, never
/// silently accepted.
///
/// Seat `i` (0-based call index) is `dealer.offset(i % 4)`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Auction {
    dealer: Seat,
    vulnerability: Vulnerability,
    calls: Vec<Call>,
}

impl Auction {
    /// An empty auction.
    pub fn new(dealer: Seat, vulnerability: Vulnerability) -> Auction {
        Auction {
            dealer,
            vulnerability,
            calls: Vec::new(),
        }
    }

    /// Builds an auction from calls, rejecting the first illegal one.
    pub fn from_calls(
        dealer: Seat,
        vulnerability: Vulnerability,
        calls: impl IntoIterator<Item = Call>,
    ) -> Result<Auction, AuctionError> {
        todo!("phase 1")
    }

    /// Appends `call`, or returns an error and leaves the auction unchanged.
    pub fn push(&mut self, call: Call) -> Result<(), AuctionError> {
        todo!("phase 1")
    }

    /// A copy of this auction with `call` appended.
    pub fn with(&self, call: Call) -> Result<Auction, AuctionError> {
        let mut next = self.clone();
        next.push(call)?;
        Ok(next)
    }

    /// The dealer.
    pub fn dealer(&self) -> Seat {
        self.dealer
    }

    /// The vulnerability.
    pub fn vulnerability(&self) -> Vulnerability {
        self.vulnerability
    }

    /// The calls so far, dealer first.
    pub fn calls(&self) -> &[Call] {
        &self.calls
    }

    /// Number of calls so far.
    pub fn len(&self) -> usize {
        self.calls.len()
    }

    /// `true` when no call has been made.
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// The seat that made (or will make) the call at `index`.
    pub fn seat_at(&self, index: usize) -> Seat {
        self.dealer.offset((index % 4) as u8)
    }

    /// The seat to call next.
    pub fn next_seat(&self) -> Seat {
        self.seat_at(self.calls.len())
    }

    /// The calls made by `seat`, with their indices, in order.
    pub fn calls_by(&self, seat: Seat) -> CallsBy<'_> {
        CallsBy {
            auction: self,
            seat,
            next: 0,
        }
    }

    /// The last bid and its index, if any.
    pub fn last_bid(&self) -> Option<(usize, Bid)> {
        todo!("phase 1")
    }

    /// The last call that is not a pass, with its index, if any.
    pub fn last_non_pass(&self) -> Option<(usize, Call)> {
        todo!("phase 1")
    }

    /// Whether `call` may be made now.
    ///
    /// 1. Nothing is legal once the auction is complete.
    /// 2. `Pass` is always legal.
    /// 3. `Bid(b)` is legal iff there is no bid yet or `b` is higher than the last bid.
    /// 4. `Double` is legal iff the last non-pass call is a bid by the opponents.
    /// 5. `Redouble` is legal iff the last non-pass call is a double by the opponents.
    pub fn is_legal(&self, call: Call) -> bool {
        todo!("phase 1")
    }

    /// The calls that may be made now, in index order.
    pub fn legal_calls(&self) -> LegalCalls<'_> {
        LegalCalls {
            auction: self,
            next: 0,
        }
    }

    /// `true` when at least four calls have been made and the last three are passes.
    pub fn is_complete(&self) -> bool {
        todo!("phase 1")
    }

    /// `true` when the auction is complete and contains no bid.
    pub fn is_passed_out(&self) -> bool {
        todo!("phase 1")
    }

    /// The final contract, or `None` if the auction is incomplete or passed out.
    ///
    /// The declarer is the first player of the side that made the last bid to have named that
    /// bid's strain; the doubling state comes from the last non-pass call.
    pub fn contract(&self) -> Option<Contract> {
        todo!("phase 1")
    }

    /// Number of passes before the first bid (`0..=3`; `4` for a passed-out auction).
    pub fn leading_passes(&self) -> usize {
        todo!("phase 1")
    }

    /// The position (`1..=4`) in which `seat` would open: `1` for the dealer, `4` for the
    /// dealer's right-hand opponent. Used for `#SEAT` conditions in system definitions.
    pub fn position_of(&self, seat: Seat) -> u8 {
        ((seat.index() + 4 - self.dealer.index()) % 4) + 1
    }
}

/// Iterator over the calls of one seat (see [`Auction::calls_by`]).
#[derive(Clone, Debug)]
pub struct CallsBy<'a> {
    auction: &'a Auction,
    seat: Seat,
    next: usize,
}

impl Iterator for CallsBy<'_> {
    type Item = (usize, Call);

    fn next(&mut self) -> Option<(usize, Call)> {
        todo!("phase 1")
    }
}

/// Iterator over the legal calls at the current point (see [`Auction::legal_calls`]).
#[derive(Clone, Debug)]
pub struct LegalCalls<'a> {
    auction: &'a Auction,
    next: u8,
}

impl Iterator for LegalCalls<'_> {
    type Item = Call;

    fn next(&mut self) -> Option<Call> {
        todo!("phase 1")
    }
}

/// An illegal call was offered to an [`Auction`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum AuctionError {
    /// The call is not legal at the given position.
    #[error("call {call} is illegal at position {index}")]
    IllegalCall {
        /// The offending call.
        call: Call,
        /// Its 0-based position in the auction.
        index: usize,
    },
}
