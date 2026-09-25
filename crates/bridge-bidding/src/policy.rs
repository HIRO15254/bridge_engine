//! The probabilistic bidding policy used as the likelihood in importance sampling.
//!
//! For each distinct legal call `c` among the satisfying candidates,
//! `score(c) = logsumexp_{nodes with call c}(priority / τ)`; `softmax` over the scores; then
//! `p(c) = (1 − ε) · softmax(c) + ε / |legal calls|`. Every legal call has positive
//! probability, so no sampled deal ever gets weight zero; an off-system call costs `ln ε`.
//! As `τ → 0` the argmax equals `choose_bid`.

use std::collections::HashMap;

use bridge_core::{Auction, Call, Deal, Hand};

use crate::choose::kept_priorities;
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

/// `ln Σ exp(x)`, computed with the usual max-subtraction for stability. `-∞` for an empty slice.
fn logsumexp(xs: &[f32]) -> f32 {
    let m = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if !m.is_finite() {
        return m;
    }
    let sum: f32 = xs.iter().map(|x| (x - m).exp()).sum();
    m + sum.ln()
}

/// The distribution over legal calls for `hand` after `auction`.
pub fn call_distribution(
    system: &SystemIR,
    hand: Hand,
    auction: &Auction,
    ctx: &BidContext<'_>,
) -> Vec<(Call, f32)> {
    let legal: Vec<Call> = auction.legal_calls().collect();
    if legal.is_empty() {
        return Vec::new();
    }
    let n_legal = legal.len() as f32;
    let eps = ctx.policy.epsilon;

    let kept = kept_priorities(system, hand, auction, ctx);
    if kept.is_empty() {
        return legal.into_iter().map(|c| (c, 1.0 / n_legal)).collect();
    }

    let tau = ctx.policy.temperature;
    let mut groups: HashMap<Call, Vec<f32>> = HashMap::new();
    for (call, priority) in kept {
        groups
            .entry(call)
            .or_default()
            .push(f32::from(priority) / tau);
    }
    let scores: HashMap<Call, f32> = groups
        .into_iter()
        .map(|(call, xs)| (call, logsumexp(&xs)))
        .collect();
    let all_scores: Vec<f32> = scores.values().copied().collect();
    let lse_all = logsumexp(&all_scores);

    legal
        .into_iter()
        .map(|c| {
            let softmax = scores.get(&c).map_or(0.0, |s| (s - lse_all).exp());
            let p = (1.0 - eps) * softmax + eps / n_legal;
            (c, p)
        })
        .collect()
}

/// `Σ_j ln p_j(calls[j])` where `p_j` is the distribution of the seat that made call `j` given
/// its hand and the prefix. About 2–5 µs per deal.
pub fn sequence_log_likelihood(
    table: &Table,
    deal: &Deal,
    auction: &Auction,
    ctx: &BidContext<'_>,
) -> f64 {
    let natural = ctx.natural.or(Some(table.natural.as_ref()));
    let ctx = BidContext {
        scoring: ctx.scoring,
        natural,
        implicit_pass: ctx.implicit_pass,
        policy: ctx.policy,
    };

    let n = auction.calls().len();
    let mut prefix = Auction::new(auction.dealer(), auction.vulnerability());
    let mut total = 0.0f64;

    for j in 0..n {
        let seat = auction.seat_at(j);
        let call = auction.calls()[j];
        let system = &table.systems[seat.index() as usize];
        let hand = deal.hand(seat);

        let dist = call_distribution(system, hand, &prefix, &ctx);
        let p = dist
            .iter()
            .find(|(c, _)| *c == call)
            .map_or(0.0, |(_, p)| *p);
        total += f64::from(p).ln();

        prefix
            .push(call)
            .expect("call from a valid Auction is legal at its own position");
    }

    total
}
