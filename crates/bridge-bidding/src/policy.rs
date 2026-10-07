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
//! a structural identity. With `δ = 0` the natural candidates are not evaluated at on-system
//! positions.
//!
//! The phase-3 priority softmax (a temperature `τ`, `PolicyParams::legacy_temperature`) was
//! deleted after the phase-6 lead evaluation (docs/design/13-decisions.md D18).

use bridge_core::{Auction, Call, Deal, Hand};

use crate::choose::{enumerate_position, natural_choice, natural_ranked, system_choice};
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
}

impl PolicyParams {
    /// Players who bid exactly the system: `ε = 1e-3`, `δ = 0`. Used for SAYC-generated
    /// auctions. Equal to `PolicyParams::default()`.
    pub const fn system_players() -> PolicyParams {
        PolicyParams {
            epsilon: 1e-3,
            deviation: 0.0,
        }
    }

    /// Human players, who leave the system for natural calls: `(ε, δ)` fitted by maximum
    /// likelihood on the corpus tune split (docs/design/15-phase4-plan.md D18, D20). Used for
    /// corpus auctions and the lead advisor.
    ///
    /// `ε = 0.3404`, `δ = 0.3959`: the grid MLE of `cargo xtask coverage` (`corpus.mle`) on the
    /// corpus tune split (even enumeration index, 4,135 calls), rounded to 4 decimals. The
    /// fine grid steps `ε` by a factor of `10^0.002` (about 0.0016 near 0.34) and `δ` by
    /// 0.0001, so `ε` is resolved only to about ±0.0008 and its 4th decimal is not
    /// significant: maximising the same counts over an additive 0.0001 grid gives `ε ≈ 0.3411`
    /// at the same `δ`, with ln L higher by 0.004. Fitted at the phase-4 integration on the
    /// SAYC of wip/p4int 8669ffd (lanes D3, len and perf merged; coverage `system_hash` of the
    /// SAYC sources `fnv1a64:bac068b4d7749231`, `COMPILE_REVISION` 9), where ln L = −7295.9 on
    /// the tune split (−1.764 per call) and −6663.2 on the eval split at the MLE. The phase-4
    /// placeholder (`ε = 0.01`, `δ = 0.3`) gives −10973.3 on the same tune split. Lane guard's
    /// `#EXACTPASS` rewrite of the same SAYC (`COMPILE_REVISION` 10) left every coverage
    /// metric, this fit included, unchanged, and revision 11 compiles that SAYC to the same IR.
    /// The natural-inference fix lanes N, N2 and N3 moved the grid MLE at the phase-4 head
    /// (wip/p4int 5aa82e6, revision 12 after lane X2) to `ε = 0.3420`, `δ = 0.3943` (ln L
    /// −7307.07 on the tune split). These values were kept on purpose: they give −7307.09
    /// there, 0.02 below the MLE. This `ε` is one fine-grid step (about 0.0016) below the new
    /// grid MLE, which accounts for almost all of the gap; the point is far inside the 1.92
    /// likelihood-ratio interval (`δ` 0.35..=0.44 at the MLE's `ε`). On the eval split they
    /// give −6675.07, above the MLE's −6675.70. Refit when the SAYC data or the natural
    /// engine moves the fit by more than that; the fits are recorded in
    /// docs/design/12-roadmap.md (the phase-4 integration after lanes D3, len and perf, and
    /// the phase-4 completion).
    pub const fn human() -> PolicyParams {
        PolicyParams {
            epsilon: 0.3404,
            deviation: 0.3959,
        }
    }
}

impl Default for PolicyParams {
    /// [`PolicyParams::system_players`].
    fn default() -> PolicyParams {
        PolicyParams::system_players()
    }
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
/// The natural engine of `M` is `ctx.natural`, or `table.natural` when `ctx.natural` is `None`
/// (the same substitution as [`sequence_log_likelihood`] and [`crate::AuctionPolicy`], so all
/// three evaluate one policy; see [`BidContext::natural`]).
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
    // The policy's natural engine is always defined (`BidContext::natural`).
    let ctx = &BidContext {
        natural: Some(ctx.natural.unwrap_or(table.natural.as_ref())),
        ..*ctx
    };
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
