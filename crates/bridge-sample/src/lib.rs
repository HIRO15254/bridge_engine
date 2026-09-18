//! Deal sampling: the output is a set of weighted deals, not a density.
//!
//! A [`Proposal`] draws deals consistent with the known cards; [`sample_deals`] corrects each
//! draw with an importance weight `w = L(deal) / π(deal)` where `L` is the bidding likelihood
//! (times the play constraints) and `π` the proposal density, both in the log domain, and
//! reports the effective sample size. Sample `i` is computed from an RNG derived from
//! `(seed, i)` only, so results are identical whatever the thread count.
#![forbid(unsafe_code)]
#![warn(missing_docs)]
// Phase 0 skeleton: the public surface is final, bodies are `todo!()`.
#![allow(dead_code, unused_variables)]

mod constraint_proposal;
mod proposal;
mod report;
mod rng;
mod uniform;
mod weights;

pub use bridge_constraint::KnownCards;
pub use constraint_proposal::ConstraintProposal;
pub use proposal::{BiddingLikelihood, PreparedProposal, Proposal, SampleContext};
pub use report::{SampleOptions, SampleReport, SampleWarning, Threads};
pub use rng::{SampleRng, rng_for, splitmix64};
pub use uniform::UniformProposal;
pub use weights::{WeightedDeal, effective_sample_size, log_sum_exp};

/// Sampling failed as a whole (individual rejections are counted in the report instead).
#[derive(Clone, PartialEq, Debug, thiserror::Error)]
pub enum SampleError {
    /// The proposal could not be prepared for this context.
    #[error("proposal could not be prepared: {0}")]
    Prepare(String),
    /// No seat's alternatives are consistent with the known cards.
    #[error("empty support: no deal satisfies the constraints")]
    EmptySupport,
}

/// Draws `n` weighted deals from `proposal` for `ctx`.
pub fn sample_deals(
    ctx: &SampleContext<'_>,
    proposal: &dyn Proposal,
    n: usize,
    opts: &SampleOptions,
) -> Result<(Vec<WeightedDeal>, SampleReport), SampleError> {
    todo!("phase 2 (uniform) / phase 5 (constraint)")
}
