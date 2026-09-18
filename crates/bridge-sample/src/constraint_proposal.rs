//! The v1 proposal: hierarchical sampling from the constraints.
//!
//! **prepare.** Per seat, alternatives = `interpretation.seats[s] ⊗ play_soft[s]`, each AND-ed
//! with `play_constraints[s]` and put in DNF. Restrictiveness `mass_s = Σ w_i · count(term)`
//! orders the seats (most constrained first); the first seat's samplers are cached. Seats whose
//! constraint is unconstrained are dealt combinatorially without a sampler.
//!
//! **propose.** Seats 1..=3 in order: re-prepare on the shrinking pool, draw a component ∝
//! `w_i · count_i`, draw a hand uniformly within it; the last seat gets the remainder and is
//! checked (rejected attempts are retried within the sample's own RNG stream).
//!
//! **log_prob.** Replay the same order and pools; `π_k(h) = Σ_{components ∋ h} u / count`,
//! summing over every component that could have produced `h`; the last seat contributes 0.

use bridge_core::Deal;

use crate::{PreparedProposal, Proposal, SampleContext, SampleError};

/// Hierarchical constraint sampling.
#[derive(Clone, Debug)]
pub struct ConstraintProposal {
    /// Retries per proposed deal before giving up (default 16).
    pub max_retries: u32,
}

impl Default for ConstraintProposal {
    fn default() -> ConstraintProposal {
        ConstraintProposal { max_retries: 16 }
    }
}

impl Proposal for ConstraintProposal {
    fn prepare<'c>(
        &self,
        ctx: &'c SampleContext<'c>,
    ) -> Result<Box<dyn PreparedProposal + Send + Sync + 'c>, SampleError> {
        todo!("phase 5")
    }
}

struct PreparedConstraint<'c> {
    ctx: &'c SampleContext<'c>,
    max_retries: u32,
}

impl PreparedProposal for PreparedConstraint<'_> {
    fn propose(&self, rng: &mut dyn rand_core::Rng) -> Option<Deal> {
        todo!("phase 5")
    }

    fn log_prob(&self, deal: &Deal) -> f64 {
        todo!("phase 5")
    }
}
