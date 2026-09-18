//! One prepared DNF term.

use std::collections::HashMap;

use bridge_core::{Hand, Shape};

use super::suit_table::{PairConv, SuitTable};
use crate::DnfTerm;

/// A DNF term prepared for a fixed pool and fixed part.
pub(crate) struct PreparedTerm {
    pub(crate) suits: [SuitTable; 4],
    /// Pair convolutions for `(len_clubs, len_diamonds)` pairs used by some feasible shape.
    pub(crate) pair01: HashMap<(u8, u8), PairConv>,
    /// Pair convolutions for `(len_hearts, len_spades)` pairs used by some feasible shape.
    pub(crate) pair23: HashMap<(u8, u8), PairConv>,
    /// Feasible shapes with their weights and HCP windows.
    pub(crate) shapes: Vec<(Shape, u64, (u8, u8))>,
    pub(crate) cum: Vec<u64>,
    pub(crate) total: u64,
    /// The source term (for residual / custom checks).
    pub(crate) term: DnfTerm,
    /// Estimated acceptance rate of the rejected literals (`None` when exact).
    pub(crate) alpha: Option<f64>,
}

impl PreparedTerm {
    pub(crate) fn prepare(
        term: DnfTerm,
        pool: Hand,
        fixed: Hand,
        opts: &super::SampleOptions,
    ) -> PreparedTerm {
        todo!("phase 2")
    }

    /// Draws one hand from this term (before residual checks).
    pub(crate) fn draw<R: rand_core::Rng + ?Sized>(&self, rng: &mut R) -> Hand {
        todo!("phase 2")
    }
}
