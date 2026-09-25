//! The auction and its legality rules.

use crate::{Bid, Call, Contract, Doubling, Seat, Vulnerability};

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
        let mut auction = Auction::new(dealer, vulnerability);
        for call in calls {
            auction.push(call)?;
        }
        Ok(auction)
    }

    /// Appends `call`, or returns an error and leaves the auction unchanged.
    pub fn push(&mut self, call: Call) -> Result<(), AuctionError> {
        if self.is_legal(call) {
            self.calls.push(call);
            Ok(())
        } else {
            Err(AuctionError::IllegalCall {
                call,
                index: self.calls.len(),
            })
        }
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
        self.calls
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, c)| c.bid().map(|b| (i, b)))
    }

    /// The last call that is not a pass, with its index, if any.
    pub fn last_non_pass(&self) -> Option<(usize, Call)> {
        self.calls
            .iter()
            .enumerate()
            .rev()
            .find(|(_, c)| **c != Call::Pass)
            .map(|(i, c)| (i, *c))
    }

    /// Whether the call at `index` was made by an opponent of the seat to call next.
    fn is_by_opponent(&self, index: usize) -> bool {
        self.seat_at(index).side() != self.next_seat().side()
    }

    /// Whether `call` may be made now.
    ///
    /// 1. Nothing is legal once the auction is complete.
    /// 2. `Pass` is always legal.
    /// 3. `Bid(b)` is legal iff there is no bid yet or `b` is higher than the last bid.
    /// 4. `Double` is legal iff the last non-pass call is a bid by the opponents.
    /// 5. `Redouble` is legal iff the last non-pass call is a double by the opponents.
    pub fn is_legal(&self, call: Call) -> bool {
        if self.is_complete() {
            return false;
        }
        match call {
            Call::Pass => true,
            Call::Bid(b) => self.last_bid().is_none_or(|(_, last)| b > last),
            Call::Double => {
                matches!(self.last_non_pass(), Some((i, Call::Bid(_))) if self.is_by_opponent(i))
            }
            Call::Redouble => {
                matches!(self.last_non_pass(), Some((i, Call::Double)) if self.is_by_opponent(i))
            }
        }
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
        let n = self.calls.len();
        n >= 4 && self.calls[n - 3..].iter().all(|c| *c == Call::Pass)
    }

    /// `true` when the auction is complete and contains no bid.
    pub fn is_passed_out(&self) -> bool {
        self.is_complete() && self.last_bid().is_none()
    }

    /// The final contract, or `None` if the auction is incomplete or passed out.
    ///
    /// The declarer is the first player of the side that made the last bid to have named that
    /// bid's strain; the doubling state comes from the last non-pass call.
    pub fn contract(&self) -> Option<Contract> {
        if !self.is_complete() {
            return None;
        }
        let (i, bid) = self.last_bid()?;
        let side = self.seat_at(i).side();
        let doubling = match self.last_non_pass() {
            Some((_, Call::Double)) => Doubling::Doubled,
            Some((_, Call::Redouble)) => Doubling::Redoubled,
            _ => Doubling::Undoubled,
        };
        let strain = bid.strain();
        let first = (0..=i).find(|&j| {
            self.seat_at(j).side() == side
                && self.calls[j].bid().is_some_and(|b| b.strain() == strain)
        })?;
        Some(Contract {
            bid,
            declarer: self.seat_at(first),
            doubling,
        })
    }

    /// Number of passes before the first bid (`0..=3`; `4` for a passed-out auction).
    pub fn leading_passes(&self) -> usize {
        self.calls.iter().take_while(|c| **c == Call::Pass).count()
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
        let auction = self.auction;
        // Call indices of `seat` are those congruent to its distance from the dealer mod 4.
        let residue = (self.seat.index() + 4 - auction.dealer.index()) as usize % 4;
        let i = self.next + (residue + 4 - self.next % 4) % 4;
        let call = *auction.calls.get(i)?;
        self.next = i + 1;
        Some((i, call))
    }
}

impl core::iter::FusedIterator for CallsBy<'_> {}

/// Iterator over the legal calls at the current point (see [`Auction::legal_calls`]).
#[derive(Clone, Debug)]
pub struct LegalCalls<'a> {
    auction: &'a Auction,
    next: u8,
}

impl Iterator for LegalCalls<'_> {
    type Item = Call;

    fn next(&mut self) -> Option<Call> {
        let auction = self.auction;
        if auction.is_complete() {
            self.next = 38;
            return None;
        }
        while self.next < 38 {
            let i = self.next;
            if i >= 3 {
                // Every bid above the last one is legal, so jump straight to it.
                let floor = match auction.last_bid() {
                    Some((_, b)) => 3 + b.index() + 1,
                    None => 3,
                };
                if i < floor {
                    self.next = floor;
                    continue;
                }
                self.next = i + 1;
                return Call::from_index(i);
            }
            self.next = i + 1;
            let call = Call::from_index(i)?;
            if auction.is_legal(call) {
                return Some(call);
            }
        }
        None
    }
}

impl core::iter::FusedIterator for LegalCalls<'_> {}

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
