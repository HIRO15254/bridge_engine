//! The v0 baseline: deal the unknown cards uniformly.

use bridge_core::Deal;

use crate::{PreparedProposal, Proposal, SampleContext, SampleError};

/// Fisher–Yates over the pool; every constraint is handled by the likelihood.
/// `log_prob` is the constant `−ln(|pool|! / Π_s needed(s)!)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct UniformProposal;

impl Proposal for UniformProposal {
    fn prepare<'c>(
        &self,
        ctx: &'c SampleContext<'c>,
    ) -> Result<Box<dyn PreparedProposal + Send + Sync + 'c>, SampleError> {
        todo!("phase 2")
    }
}

struct PreparedUniform<'c> {
    ctx: &'c SampleContext<'c>,
    log_prob: f64,
}

impl PreparedProposal for PreparedUniform<'_> {
    fn propose(&self, rng: &mut dyn rand_core::Rng) -> Option<Deal> {
        todo!("phase 2")
    }

    fn log_prob(&self, deal: &Deal) -> f64 {
        self.log_prob
    }
}
