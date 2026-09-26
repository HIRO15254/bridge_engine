//! The probabilistic bidding policy used as the likelihood in importance sampling
//! (docs/design/15-phase4-plan.md D18; 07-bidding.md §6.1).
//!
//! At a position `P` (prefix `auction`, acting seat `s`) with legal calls `L`, `n = |L|`:
//!
//! - `s_P(h)`: when `P` is on-system (the exact resolve, or the first full lenient match, has at
//!   least one legal child), `choose_bid`'s system choice (the implicit `Pass` included); `⊥`
//!   when no system candidate is satisfied.
//! - `m_P(h)`: the natural policy's choice: the first satisfied candidate of
//!   `NaturalInference::ranked_candidates`, else the natural implicit `Pass` (only under
//!   `ImplicitPass::Complement`), else `⊥`.
//! - `S(c|h) = 1[s_P(h) = c]`, or `1/n` when `s_P(h) = ⊥`; `M` likewise from `m_P`.
//! - `π(c|h) = (1 − δ)·S + δ·M` on-system, `M` off-system.
//! - **`p(c|h) = (1 − ε)·π(c|h) + ε/n`**.
//!
//! For `δ < 1/2` the argmax of `p` is `choose_bid`'s call whenever `choose_bid` chooses one:
//! a structural identity, independent of any temperature. With `δ = 0` the natural candidates
//! are not evaluated at on-system positions.
//!
//! `PolicyParams::legacy_temperature = Some(τ)` selects the retired phase-3 policy instead
//! (priority/τ logsumexp per call, softmax, then the ε floor), kept only for the phase-6
//! comparison; `interpret`'s mirror is not calibrated to it.

use bridge_core::{Auction, Call, Deal, Hand};

use crate::choose::{enumerate_position, gather, natural_choice, natural_ranked, system_choice};
use crate::{BidContext, Table};

/// Parameters of the bidding policy `p(c|h) = (1 − ε)·[(1 − δ)·S + δ·M] + ε/n`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PolicyParams {
    /// The uniform floor `ε`: mass spread over all legal calls, so every legal call has
    /// positive probability (default 1e-3).
    pub epsilon: f32,
    /// The deviation `δ` from the system to the natural policy at on-system positions (default
    /// 0). Keep it below 1/2 so the argmax stays `choose_bid`'s call.
    pub deviation: f32,
    /// `Some(τ)` selects the retired priority softmax at temperature `τ` (comparison only; to
    /// be removed after the phase-6 lead evaluation). `interpret`'s mirror is not calibrated to
    /// it. Default `None`.
    pub legacy_temperature: Option<f32>,
}

impl PolicyParams {
    /// Players who bid exactly the system: `ε = 1e-3`, `δ = 0`, no legacy temperature. Used for
    /// SAYC-generated auctions. Equal to `PolicyParams::default()`.
    pub const fn system_players() -> PolicyParams {
        PolicyParams {
            epsilon: 1e-3,
            deviation: 0.0,
            legacy_temperature: None,
        }
    }

    /// Human players, who leave the system for natural calls: `(ε, δ)` fitted by maximum
    /// likelihood on the corpus tune split (docs/design/15-phase4-plan.md D18, D20). Used for
    /// corpus auctions and the lead advisor.
    ///
    /// Placeholder values (`ε = 0.01`, `δ = 0.3`) until lane D's MLE lands at the phase-4
    /// integration; the fitted values are recorded in 12-roadmap.
    pub const fn human() -> PolicyParams {
        PolicyParams {
            epsilon: 0.01,
            deviation: 0.3,
            legacy_temperature: None,
        }
    }

    /// The retired phase-3 policy (priority softmax at temperature `temperature`, `ε = 1e-3`),
    /// for before/after comparisons only.
    pub const fn legacy(temperature: f32) -> PolicyParams {
        PolicyParams {
            epsilon: 1e-3,
            deviation: 0.0,
            legacy_temperature: Some(temperature),
        }
    }
}

impl Default for PolicyParams {
    /// [`PolicyParams::system_players`].
    fn default() -> PolicyParams {
        PolicyParams::system_players()
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

/// One slot per `Call::index()` value (`0..38`, Pass/Double/Redouble plus every `Bid`); see
/// `Call::index`.
const N_CALLS: usize = 38;

/// Adds `mass` to `choice`'s slot, or spreads it uniformly over `legal` when there is no choice.
fn add_choice(pi: &mut [f32; N_CALLS], choice: Option<Call>, mass: f32, legal: &[Call]) {
    if mass <= 0.0 {
        return;
    }
    match choice {
        Some(call) => pi[call.index() as usize] += mass,
        None => {
            let share = mass / legal.len() as f32;
            for c in legal {
                pi[c.index() as usize] += share;
            }
        }
    }
}

/// The distribution over legal calls for `hand` after `auction` (docs/design/15-phase4-plan.md
/// D18; the module doc has the formula). Values are in `Call` legal order and sum to 1 (up to
/// rounding).
///
/// Grouped by `Call::index()` in a fixed-size array, not a `HashMap` (D12/09-sample §7): the
/// summation order is fixed, so `sequence_log_likelihood` is bit-for-bit deterministic between
/// single- and multi-threaded runs.
pub fn call_distribution(
    table: &Table,
    hand: Hand,
    auction: &Auction,
    ctx: &BidContext<'_>,
) -> Vec<(Call, f32)> {
    let legal: Vec<Call> = auction.legal_calls().collect();
    if legal.is_empty() {
        return Vec::new();
    }
    if let Some(tau) = ctx.policy.legacy_temperature {
        return legacy_distribution(table, hand, auction, ctx, &legal, tau);
    }
    let n_legal = legal.len() as f32;
    let eps = ctx.policy.epsilon;
    let delta = ctx.policy.deviation;

    let pos = enumerate_position(table, auction, ctx.implicit_pass);
    let mut pi = [0.0f32; N_CALLS];
    if pos.on_system() {
        add_choice(&mut pi, system_choice(&pos, hand), 1.0 - delta, &legal);
        if delta > 0.0 {
            let m = ctx.natural.and_then(|natural| {
                let ranked = natural_ranked(table, &pos, auction, natural, ctx.implicit_pass);
                natural_choice(&ranked, hand, ctx.implicit_pass)
            });
            add_choice(&mut pi, m, delta, &legal);
        }
    } else {
        let m = ctx.natural.and_then(|natural| {
            let ranked = natural_ranked(table, &pos, auction, natural, ctx.implicit_pass);
            natural_choice(&ranked, hand, ctx.implicit_pass)
        });
        add_choice(&mut pi, m, 1.0, &legal);
    }

    legal
        .into_iter()
        .map(|c| (c, (1.0 - eps) * pi[c.index() as usize] + eps / n_legal))
        .collect()
}

/// The retired phase-3 policy: for each distinct call among the satisfied candidates,
/// `score(c) = logsumexp_{candidates with call c}(priority / τ)`, softmax over the scores, then
/// the `ε/n` floor; uniform when no candidate is satisfied.
fn legacy_distribution(
    table: &Table,
    hand: Hand,
    auction: &Auction,
    ctx: &BidContext<'_>,
    legal: &[Call],
    tau: f32,
) -> Vec<(Call, f32)> {
    let n_legal = legal.len() as f32;
    let eps = ctx.policy.epsilon;
    let kept = gather(table, hand, auction, ctx).kept;
    if kept.is_empty() {
        return legal.iter().map(|&c| (c, 1.0 / n_legal)).collect();
    }
    let mut scores_by_call: [Vec<f32>; N_CALLS] = std::array::from_fn(|_| Vec::new());
    for k in &kept {
        scores_by_call[k.call.index() as usize].push(f32::from(k.priority) / tau);
    }
    let mut scores = [f32::NEG_INFINITY; N_CALLS];
    for (i, xs) in scores_by_call.iter().enumerate() {
        if !xs.is_empty() {
            scores[i] = logsumexp(xs);
        }
    }
    let all_scores: Vec<f32> = scores.iter().copied().filter(|s| s.is_finite()).collect();
    let lse_all = logsumexp(&all_scores);
    legal
        .iter()
        .map(|&c| {
            let s = scores[c.index() as usize];
            let softmax = if s.is_finite() {
                (s - lse_all).exp()
            } else {
                0.0
            };
            (c, (1.0 - eps) * softmax + eps / n_legal)
        })
        .collect()
}

/// `Σ_j ln p_j(calls[j])` where `p_j` is [`call_distribution`] of the seat that made call `j`
/// given its hand and the prefix. The natural engine is `ctx.natural`, or `table.natural` when
/// `ctx.natural` is `None`. This is the reference implementation; `AuctionPolicy` is the fast
/// path and must agree with it to `|Δ ln L| <= 1e-5`.
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
        let hand = deal.hand(seat);

        let dist = call_distribution(table, hand, &prefix, &ctx);
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
