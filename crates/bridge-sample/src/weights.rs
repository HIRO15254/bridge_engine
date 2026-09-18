//! Importance weights and the effective sample size.

use bridge_core::Deal;

/// A sampled deal with its log importance weight.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WeightedDeal {
    /// The deal.
    pub deal: Deal,
    /// `ln w = ln L − ln π` (constant factors cancel under self-normalisation).
    pub log_weight: f64,
}

impl WeightedDeal {
    /// Self-normalised weights summing to 1.
    pub fn normalized_weights(deals: &[WeightedDeal]) -> Vec<f64> {
        todo!("phase 2")
    }
}

/// `m + ln Σ exp(x − m)` with `m = max x`; `-∞` for an empty slice.
pub fn log_sum_exp(xs: impl IntoIterator<Item = f64>) -> f64 {
    todo!("phase 2")
}

/// `ESS = (Σ w)² / Σ w² = exp(2·LSE(lw) − LSE(2·lw))`.
pub fn effective_sample_size(log_weights: &[f64]) -> f64 {
    todo!("phase 2")
}
