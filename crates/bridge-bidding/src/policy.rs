//! The probabilistic bidding policy used as the likelihood in importance sampling.
//!
//! For each distinct legal call `c` among the satisfying candidates,
//! `score(c) = logsumexp_{nodes with call c}(priority / τ)`; `softmax` over the scores; then
//! `p(c) = (1 − ε) · softmax(c) + ε / |legal calls|`. Every legal call has positive
//! probability, so no sampled deal ever gets weight zero; an off-system call costs `ln ε`.
//! As `τ → 0` the argmax equals `choose_bid`.

use bridge_core::{Auction, Call, Deal, Hand};

use crate::{BidContext, SystemIR, Table};

/// Softmax parameters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PolicyParams {
    /// Temperature (default 1.0).
    pub temperature: f32,
    /// Floor mass spread over all legal calls (default 1e-3).
    pub epsilon: f32,
}

impl Default for PolicyParams {
    fn default() -> PolicyParams {
        PolicyParams {
            temperature: 1.0,
            epsilon: 1e-3,
        }
    }
}

/// The distribution over legal calls for `hand` after `auction`.
pub fn call_distribution(
    system: &SystemIR,
    hand: Hand,
    auction: &Auction,
    ctx: &BidContext<'_>,
) -> Vec<(Call, f32)> {
    todo!("phase 3")
}

/// `Σ_j ln p_j(calls[j])` where `p_j` is the distribution of the seat that made call `j` given
/// its hand and the prefix. About 2–5 µs per deal.
pub fn sequence_log_likelihood(
    table: &Table,
    deal: &Deal,
    auction: &Auction,
    ctx: &BidContext<'_>,
) -> f64 {
    todo!("phase 3")
}
