//! Proposal distributions.

use bridge_bidding::{BidContext, Interpretation, Table};
use bridge_constraint::{HandConstraint, KnownCards};
use bridge_core::{Auction, Deal};

use crate::SampleError;

/// A proposal distribution over deals. Prepared once per context; the prepared object is what
/// draws, because the first seat's sampler and the DNFs are context-invariant and must not be
/// recomputed per deal.
pub trait Proposal: Send + Sync {
    /// Prepares for `ctx`.
    fn prepare<'c>(
        &self,
        ctx: &'c SampleContext<'c>,
    ) -> Result<Box<dyn PreparedProposal + Send + Sync + 'c>, SampleError>;
}

/// A proposal prepared for one context.
pub trait PreparedProposal {
    /// Proposes one deal; `None` is a rejected attempt.
    fn propose(&self, rng: &mut dyn rand_core::Rng) -> Option<Deal>;

    /// `ln π(deal)`; `-∞` outside the support.
    fn log_prob(&self, deal: &Deal) -> f64;
}

/// What the sampler knows about the position.
#[derive(Clone, Copy)]
pub struct SampleContext<'a> {
    /// Known cards per seat (viewer's hand, dummy, played cards).
    pub known: KnownCards,
    /// The auction's interpretation.
    pub interpretation: &'a Interpretation,
    /// Hard constraints from the play (AND-ed into everything).
    pub play_constraints: &'a [HandConstraint; 4],
    /// Soft weighted constraints from the play, if any.
    pub play_soft: Option<&'a [Vec<(HandConstraint, f32)>; 4]>,
    /// The bidding likelihood; `None` uses `interpretation.likelihood` instead.
    pub bidding: Option<BiddingLikelihood<'a>>,
}

/// What is needed to evaluate the bidding likelihood of a deal.
#[derive(Clone, Copy)]
pub struct BiddingLikelihood<'a> {
    /// The systems.
    pub table: &'a Table,
    /// The auction.
    pub auction: &'a Auction,
    /// Policy context.
    pub ctx: &'a BidContext<'a>,
}
