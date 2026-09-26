//! Deal sampling: the output is a set of weighted deals, not a density.
//!
//! A [`Proposal`] draws deals consistent with the known cards; [`sample_deals`] corrects each
//! draw with an importance weight `w = L(deal) / π(deal)` where `L` is the bidding likelihood
//! (times the play constraints) and `π` the proposal density, both in the log domain, and
//! reports the effective sample size. Sample `i` is computed from an RNG derived from
//! `(seed, i)` only, so results are identical whatever the thread count.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod constraint_proposal;
mod proposal;
mod report;
mod rng;
mod uniform;
mod weights;

use core::ops::Range;

use bridge_bidding::{AuctionPolicy, Explanation, Interpretation, ResolutionKind};
use bridge_constraint::{HandConstraint, SampleOptions as ConstraintSampleOptions, Sampler};
use bridge_core::{Deal, Seat};

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
///
/// Slots `0, 1, 2, …` are processed in chunks of `n`; slot `i` uses only `rng_for(opts.seed,
/// i)` and retries up to `opts.max_attempts_per_sample` times, so the result does not depend on
/// how many chunks were needed or on the thread count (§2.3, §7 of `09-sample.md`). A rejected
/// attempt (`propose` returning `None`, or a non-finite log weight) is retried within the same
/// slot; a slot that never succeeds contributes nothing. Each chunk's slots are folded into
/// `deals`/`attempts` in slot order, stopping as soon as `deals.len()` reaches `n` or `attempts`
/// reaches the attempt budget `n × max_attempt_factor` (so the budget is honoured to within one
/// slot's `max_attempts_per_sample`; `SampleReport::budget_exhausted` records it): a chunk's
/// surplus slots (past the `n`-th accepted deal) are counted in neither, so a discarded surplus
/// deal never inflates `attempts` (and so understates `acceptance_rate`) without a matching
/// contribution to `produced`. Whether to run another chunk is still decided at a chunk boundary,
/// once `n` deals have been produced or `n × max_attempt_factor` attempts have been made in
/// total; either way, only slot order (never the thread count) decides which slots count, so the
/// result does not depend on how many chunks were needed or on the thread count (§2.3, §7 of
/// `09-sample.md`).
///
/// Before `proposal.prepare` is even called, every seat is probed against `ctx.known.pool()`
/// (§2.3 of `09-sample.md`):
///
/// - A seat already fully known (`needed(seat) == 0`: the viewer, or an exposed dummy) has
///   nothing to sample, but its fixed hand must still satisfy its own hard play constraint
///   (`ctx.play_constraints[s]`), or no deal exists at all: `Err(SampleError::EmptySupport)`,
///   zero attempts.
/// - Otherwise, if the hard play constraint admits no hand at all once the known cards are
///   fixed (via `bridge_constraint::Sampler`), no proposal could ever produce a deal, so sampling
///   returns `Err(SampleError::EmptySupport)` immediately, with zero attempts.
/// - If a seat's interpretation alternatives (`ctx.interpretation.seats[s]`) are all inconsistent
///   with the known cards (once each is AND-ed with the hard constraint just checked), the
///   auction's evidence for that seat cannot be used at all; sampling continues with that seat
///   dealt (and weighted) as `ANY` instead — its `seats[s]` becomes a single unconstrained
///   alternative and its `per_call` entries are dropped, so `Interpretation::likelihood` also
///   treats it as vacuous — and `SampleWarning::EmptySupport { seat }` records the fallback.
/// - Whichever `Sampler`s this probe prepares (the hard constraint's, and each alternative's
///   combined with it) are also checked for exactness: if any that survives with `count() > 0`
///   is not `Sampler::is_exact()` (a `Custom` node, a DNF residual, or any other rejection-sampled
///   term), `SampleWarning::CustomConstraint { seat }` records that `log_prob`'s density for that
///   seat is approximate — `Sampler`'s own rejection loop only ever under-estimates a term's true
///   mass, never over-estimates it, so `log_prob` and hence the importance weight can be biased.
///
/// This probing context (not `ctx` itself) is what `proposal.prepare`, `run_chunk` and
/// `ln_likelihood` below actually use.
pub fn sample_deals(
    ctx: &SampleContext<'_>,
    proposal: &dyn Proposal,
    n: usize,
    opts: &SampleOptions,
) -> Result<(Vec<WeightedDeal>, SampleReport), SampleError> {
    // `std::time::Instant::now()` panics on `wasm32-unknown-unknown` (no clock source), so wall
    // time is only measured on targets that actually have one; `elapsed` is excluded from every
    // determinism comparison in the design doc (§7 of `09-sample.md`), so reporting `Duration::
    // ZERO` on wasm32 costs nothing.
    #[cfg(not(target_arch = "wasm32"))]
    let start = std::time::Instant::now();

    KnownCards::new(ctx.known.known).map_err(|e| SampleError::Prepare(e.to_string()))?;
    if let Some(bidding) = &ctx.bidding {
        let calls = ctx.interpretation.per_call.len();
        let auction_len = bidding.auction.len();
        if calls != auction_len {
            return Err(SampleError::Prepare(format!(
                "interpretation has {calls} calls but the auction has {auction_len}"
            )));
        }
    }

    // §2.3 support checks. A `Sampler` here is used only as a support probe (`count() == 0`?);
    // its own internal weighting is irrelevant, so the constraint crate's own default options
    // are enough.
    let support_opts = ConstraintSampleOptions::default();
    let pool = ctx.known.pool();
    let mut warnings = Vec::new();
    let mut fallback_to_any = [false; 4];
    for seat in Seat::ALL {
        let idx = seat.index() as usize;
        let fixed = ctx.known.known[idx];
        let hard = &ctx.play_constraints[idx];

        if ctx.known.needed(seat) == 0 {
            // Already fully known (the viewer, or an exposed dummy): nothing to sample, but the
            // fixed hand must still satisfy this seat's own hard play constraint, or no deal
            // exists at all (§2.3 of `09-sample.md`) — the probe below (built around `Sampler`,
            // which needs at least one card to draw) can't check this case, so it is checked
            // directly instead.
            if !hard.satisfies(fixed) {
                return Err(SampleError::EmptySupport);
            }
            continue;
        }

        let hard_sampler = Sampler::prepare(hard, pool, fixed, &support_opts)
            .map_err(|e| SampleError::Prepare(e.to_string()))?;
        if hard_sampler.count() == 0 {
            return Err(SampleError::EmptySupport);
        }
        // `hard`'s own exactness matters even if this seat later falls back to `ANY` below,
        // since `ConstraintProposal` (and any other `Proposal`) still ANDs every alternative
        // with `hard`.
        let mut inexact = !hard_sampler.is_exact();

        let alternatives = &ctx.interpretation.seats[idx];
        if alternatives.is_empty() {
            // No calls at all for this seat: `Interpretation::likelihood` already treats this as
            // vacuously `ANY` (07-bidding.md §4.4 point 1), so there is nothing to fall back from.
            if inexact {
                warnings.push(SampleWarning::CustomConstraint { seat });
            }
            continue;
        }
        let mut any_consistent = false;
        for (constraint, _, _) in alternatives {
            let combined = constraint.clone().and(hard.clone());
            if let Ok(sampler) = Sampler::prepare(&combined, pool, fixed, &support_opts) {
                if sampler.count() > 0 {
                    any_consistent = true;
                    if !sampler.is_exact() {
                        inexact = true;
                    }
                }
            }
        }
        if !any_consistent {
            warnings.push(SampleWarning::EmptySupport { seat });
            fallback_to_any[idx] = true;
        } else if inexact {
            warnings.push(SampleWarning::CustomConstraint { seat });
        }
    }

    // Seats that fell back are dealt (and weighted) as `ANY` from here on: both `seats[s]` (what
    // `ConstraintProposal` and any other `Proposal` build alternatives from) and `per_call`
    // (what `Interpretation::likelihood` sums over in the `ctx.bidding.is_none()` branch below)
    // are overridden so that neither channel still constrains that seat.
    let fallback_interpretation;
    let effective_interpretation: &Interpretation = if fallback_to_any.iter().any(|&f| f) {
        let mut interpretation = ctx.interpretation.clone();
        for seat in Seat::ALL {
            if fallback_to_any[seat.index() as usize] {
                interpretation.seats[seat.index() as usize] = vec![(
                    HandConstraint::ANY,
                    1.0,
                    Explanation {
                        text: String::new(),
                        node: None,
                        resolution: ResolutionKind::Fallback,
                        parts: Vec::new(),
                    },
                )];
                interpretation.per_call.retain(|call| call.seat != seat);
            }
        }
        fallback_interpretation = interpretation;
        &fallback_interpretation
    } else {
        ctx.interpretation
    };
    let effective_ctx = SampleContext {
        known: ctx.known,
        interpretation: effective_interpretation,
        play_constraints: ctx.play_constraints,
        play_soft: ctx.play_soft,
        bidding: ctx.bidding,
    };
    let ctx = &effective_ctx;

    let prepared = proposal.prepare(ctx)?;

    // The bidding likelihood of this auction, prepared once for every deal of the run
    // (07-bidding.md §6.2's fast path; equal to `sequence_log_likelihood`).
    let policy = ctx
        .bidding
        .map(|b| AuctionPolicy::new(b.table, b.auction, b.ctx));
    let policy = policy.as_ref();

    let mut deals: Vec<WeightedDeal> = Vec::new();
    let mut attempts: u64 = 0;
    let max_attempts_total = (n as u64).saturating_mul(u64::from(opts.max_attempt_factor));
    if n > 0 && opts.max_attempts_per_sample > 0 {
        let mut chunk_start: usize = 0;
        loop {
            let chunk_end = chunk_start + n;
            let results = run_chunk(ctx, policy, prepared.as_ref(), opts, chunk_start..chunk_end);

            // A chunk always has exactly `n` slots, so it can push `deals.len()` from below `n`
            // to above it; once that happens, the remaining slots in this same chunk are surplus
            // and `deals.truncate(n)` below discards their deals regardless. Stop folding those
            // surplus slots' attempts into `attempts` (and stop pushing their deals) as soon as
            // the quota is reached, in slot order, so a thrown-away deal never inflates `attempts`
            // without a matching contribution to `produced` (which would otherwise understate
            // `acceptance_rate`). Slot order (not thread count) decides which slots are "surplus",
            // so this stays independent of `opts.threads` per §2.3/§7.
            //
            // The attempt budget is applied the same way: slots are folded in slot order only
            // while `attempts` is below the budget, so the reported attempts exceed the budget
            // by less than one slot's `max_attempts_per_sample`, and whichever slots count is
            // again a function of slot order alone.
            let mut chunk_attempts = 0u64;
            for (slot_attempts, result) in results {
                if deals.len() >= n || attempts >= max_attempts_total {
                    break;
                }
                chunk_attempts += slot_attempts;
                attempts += slot_attempts;
                if let Some(weighted) = result {
                    deals.push(weighted);
                }
            }
            chunk_start = chunk_end;

            if deals.len() >= n || attempts >= max_attempts_total || chunk_attempts == 0 {
                break;
            }
        }
    }

    deals.truncate(n);
    let produced = deals.len();
    if produced < n {
        warnings.push(SampleWarning::Truncated { produced });
    }
    let budget_exhausted = produced < n && attempts >= max_attempts_total;
    if budget_exhausted {
        warnings.push(SampleWarning::BudgetExhausted { attempts, produced });
    }

    let log_weights: Vec<f64> = deals.iter().map(|d| d.log_weight).collect();
    let ess = effective_sample_size(&log_weights);
    let ess_ratio = if n > 0 { ess / n as f64 } else { 0.0 };
    if n > 0 && ess < 0.5 * n as f64 {
        warnings.push(SampleWarning::LowEss { ess, requested: n });
    }
    let log_weight_max = log_weights
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let (acceptance_rate, ess_per_attempt) = if attempts > 0 {
        (produced as f64 / attempts as f64, ess / attempts as f64)
    } else {
        (0.0, 0.0)
    };

    let report = SampleReport {
        requested: n,
        produced,
        attempts,
        acceptance_rate,
        ess,
        ess_ratio,
        ess_per_attempt,
        log_weight_max,
        budget_exhausted,
        #[cfg(not(target_arch = "wasm32"))]
        elapsed: start.elapsed(),
        #[cfg(target_arch = "wasm32")]
        elapsed: std::time::Duration::ZERO,
        warnings,
    };

    tracing::info!(
        requested = report.requested,
        produced = report.produced,
        attempts = report.attempts,
        acceptance_rate = report.acceptance_rate,
        ess = report.ess,
        ess_ratio = report.ess_ratio,
        ess_per_attempt = report.ess_per_attempt,
        budget_exhausted = report.budget_exhausted,
        log_weight_max = report.log_weight_max,
        elapsed_us = report.elapsed.as_micros() as u64,
        "deal sampling finished"
    );

    Ok((deals, report))
}

/// Runs one chunk of slots `slots`, returning each slot's `(attempts, result)` in slot order.
/// With the `parallel` feature and [`Threads::Auto`], slots run on rayon's global pool
/// (`into_par_iter().map(..).collect()` preserves order); otherwise they run sequentially. Either
/// way every slot depends only on its own index, so the result is identical.
#[cfg(feature = "parallel")]
fn run_chunk(
    ctx: &SampleContext<'_>,
    policy: Option<&AuctionPolicy>,
    prepared: &(dyn PreparedProposal + Send + Sync),
    opts: &SampleOptions,
    slots: Range<usize>,
) -> Vec<(u64, Option<WeightedDeal>)> {
    use rayon::prelude::*;

    match opts.threads {
        Threads::Auto => slots
            .into_par_iter()
            .map(|slot| process_slot(ctx, policy, prepared, opts, slot))
            .collect(),
        Threads::Single => slots
            .map(|slot| process_slot(ctx, policy, prepared, opts, slot))
            .collect(),
    }
}

/// Sequential fallback when the `parallel` feature is disabled (`Threads::Auto` behaves like
/// `Threads::Single`).
#[cfg(not(feature = "parallel"))]
fn run_chunk(
    ctx: &SampleContext<'_>,
    policy: Option<&AuctionPolicy>,
    prepared: &dyn PreparedProposal,
    opts: &SampleOptions,
    slots: Range<usize>,
) -> Vec<(u64, Option<WeightedDeal>)> {
    slots
        .map(|slot| process_slot(ctx, policy, prepared, opts, slot))
        .collect()
}

/// Runs one sampling slot: up to `opts.max_attempts_per_sample` proposals from
/// `rng_for(opts.seed, slot)`, stopping at the first finite log weight. Returns the number of
/// attempts made and the produced deal, if any.
fn process_slot(
    ctx: &SampleContext<'_>,
    policy: Option<&AuctionPolicy>,
    prepared: &(impl PreparedProposal + ?Sized),
    opts: &SampleOptions,
    slot: usize,
) -> (u64, Option<WeightedDeal>) {
    let mut rng = rng_for(opts.seed, slot as u64);
    let mut attempts = 0u64;
    for _ in 0..opts.max_attempts_per_sample {
        attempts += 1;
        let Some(deal) = prepared.propose(&mut rng) else {
            continue;
        };
        let ln_pi = prepared.log_prob(&deal);
        let ln_l = ln_likelihood(ctx, policy, &deal);
        let log_weight = ln_l - ln_pi;
        if log_weight.is_finite() {
            return (attempts, Some(WeightedDeal { deal, log_weight }));
        }
    }
    (attempts, None)
}

/// `ln L(d)` (§3 of `09-sample.md`): `-∞` if any seat violates its hard play constraint;
/// otherwise the bidding term (`policy`, the [`AuctionPolicy`] built from `ctx.bidding`, when the
/// bidding is known, else the sum of `interpretation.likelihood`) plus, per seat with a non-empty
/// soft list, the log of its mixture mass (a seat with no soft alternatives contributes nothing,
/// not `-∞`).
fn ln_likelihood(ctx: &SampleContext<'_>, policy: Option<&AuctionPolicy>, deal: &Deal) -> f64 {
    for seat in Seat::ALL {
        if !ctx.play_constraints[seat.index() as usize].satisfies(deal.hand(seat)) {
            return f64::NEG_INFINITY;
        }
    }

    let mut ln_l = match policy {
        Some(policy) => policy.log_likelihood(deal),
        None => Seat::ALL
            .into_iter()
            .map(|seat| f64::from(ctx.interpretation.likelihood(seat, deal.hand(seat)).ln()))
            .sum(),
    };

    if let Some(soft) = ctx.play_soft {
        for seat in Seat::ALL {
            let alternatives = &soft[seat.index() as usize];
            if alternatives.is_empty() {
                continue;
            }
            let hand = deal.hand(seat);
            let mass: f32 = alternatives
                .iter()
                .filter(|(constraint, _)| constraint.satisfies(hand))
                .map(|(_, weight)| *weight)
                .sum();
            ln_l += f64::from(mass.ln());
        }
    }

    ln_l
}

#[cfg(test)]
mod tests {
    use bridge_bidding::{
        CallExplanation, CallInterpretation, Explanation, Interpretation, ResolutionKind,
    };
    use bridge_constraint::{
        Atom, CardRequirement, CustomPred, HandConstraint, KnownCards, ShapeSet,
    };
    use bridge_core::{Bid, Call, Card, Hand, Rank, Strain, Suit};
    use std::sync::Arc;

    use super::*;

    /// North must hold a balanced 15-17 HCP hand; every other seat is unconstrained (no calls
    /// at all, which `Interpretation::likelihood` treats as vacuously satisfied).
    fn north_1nt_interpretation() -> (Interpretation, HandConstraint) {
        let strong_balanced = HandConstraint::Atom(Atom {
            shapes: ShapeSet::BALANCED,
            hcp: 15..=17,
            cards: Vec::new(),
            eval: Vec::new(),
        });
        let one_notrump = Bid::new(1, Strain::NoTrump).expect("1NT is a valid bid");
        let explanation = CallExplanation {
            call_index: 0,
            call: Call::Bid(one_notrump),
            node: None,
            kind: ResolutionKind::Exact,
            text: "1NT: 15-17 balanced".to_string(),
        };
        let call = CallInterpretation {
            call_index: 0,
            seat: Seat::North,
            call: explanation.call,
            kind: ResolutionKind::Exact,
            alternatives: vec![(strong_balanced.clone(), 1.0, explanation)],
            log_scale: 0.0,
            shadowed: false,
        };
        let interpretation = Interpretation {
            seats: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            per_call: vec![call],
            divergence: None,
        };
        (interpretation, strong_balanced)
    }

    #[test]
    fn ln_likelihood_is_neg_infinity_exactly_for_violating_deals() {
        let (interpretation, constraint) = north_1nt_interpretation();
        let play_constraints = [
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
        ];
        let ctx = SampleContext {
            known: KnownCards::EMPTY,
            interpretation: &interpretation,
            play_constraints: &play_constraints,
            play_soft: None,
            bidding: None,
        };

        let cards: Vec<_> = Hand::FULL.cards().collect();
        let mut satisfied = false;
        let mut violated = false;
        // 13! permutations is way too many to enumerate; walk a handful of RNG-shuffled deals
        // and check both sides of the constraint show up and behave as expected.
        for i in 0..500u64 {
            if satisfied && violated {
                break;
            }
            let mut rng = rng_for(99, i);
            let mut deck = cards.clone();
            uniform_shuffle_for_test(&mut deck, &mut rng);
            let mut hands = [Hand::EMPTY; 4];
            for (hand, chunk) in hands.iter_mut().zip(deck.chunks(13)) {
                for &card in chunk {
                    *hand = hand.with(card);
                }
            }
            let deal = Deal::new(hands).expect("52 cards split into four 13-card hands");
            let ln_l = ln_likelihood(&ctx, None, &deal);
            if constraint.satisfies(deal.hand(Seat::North)) {
                satisfied = true;
                assert!(ln_l.is_finite(), "satisfying North hand got ln_l = {ln_l}");
            } else {
                violated = true;
                assert_eq!(
                    ln_l,
                    f64::NEG_INFINITY,
                    "violating North hand got ln_l = {ln_l}"
                );
            }
        }
        assert!(
            satisfied,
            "500 random deals never satisfied the 15-17 balanced constraint"
        );
        assert!(
            violated,
            "500 random deals always satisfied the 15-17 balanced constraint"
        );
    }

    /// A minimal Fisher-Yates, independent of `uniform.rs`'s private one, so this test does not
    /// depend on that module's internals.
    fn uniform_shuffle_for_test(cards: &mut [bridge_core::Card], rng: &mut SampleRng) {
        use rand_core::Rng;
        for i in (1..cards.len()).rev() {
            let j = (rng.next_u64() % (i as u64 + 1)) as usize;
            cards.swap(i, j);
        }
    }

    /// North must hold a balanced hand (any HCP, roughly 40% of deals - moderate, neither
    /// vanishingly rare nor a near-certainty); every other seat is unconstrained.
    fn north_balanced_interpretation() -> (Interpretation, HandConstraint) {
        let balanced = HandConstraint::Atom(Atom {
            shapes: ShapeSet::BALANCED,
            hcp: 0..=37,
            cards: Vec::new(),
            eval: Vec::new(),
        });
        let one_notrump = Bid::new(1, Strain::NoTrump).expect("1NT is a valid bid");
        let explanation = CallExplanation {
            call_index: 0,
            call: Call::Bid(one_notrump),
            node: None,
            kind: ResolutionKind::Exact,
            text: "1NT: balanced".to_string(),
        };
        let call = CallInterpretation {
            call_index: 0,
            seat: Seat::North,
            call: explanation.call,
            kind: ResolutionKind::Exact,
            alternatives: vec![(balanced.clone(), 1.0, explanation)],
            log_scale: 0.0,
            shadowed: false,
        };
        let interpretation = Interpretation {
            seats: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            per_call: vec![call],
            divergence: None,
        };
        (interpretation, balanced)
    }

    /// `attempts`/`acceptance_rate` must count exactly the slots (in slot order, from 0) needed to
    /// produce the first `n` accepted deals, never the whole final chunk of `n` slots when the
    /// n-th acceptance falls before that chunk's last slot. With `max_attempts_per_sample == 1`,
    /// slot `i` always costs exactly one attempt (`process_slot` returns after its single try
    /// regardless of outcome), so the correct total is `1 + (the 0-based slot index of the n-th
    /// accepted deal)` - computed here independently of `sample_deals`, by re-running the same
    /// `process_slot` slot by slot. Before the fix, `sample_deals` summed every slot in the whole
    /// final chunk (rounding the correct total up to the next multiple of `n`) whenever that
    /// chunk's surplus successes (beyond the n-th) got discarded by the trailing `truncate(n)`.
    #[test]
    fn attempts_counts_only_slots_used_toward_the_produced_deals_not_a_discarded_surplus() {
        let (interpretation, constraint) = north_balanced_interpretation();
        let play_constraints = [
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
        ];
        let ctx = SampleContext {
            known: KnownCards::EMPTY,
            interpretation: &interpretation,
            play_constraints: &play_constraints,
            play_soft: None,
            bidding: None,
        };
        let opts = SampleOptions {
            seed: 7,
            max_attempts_per_sample: 1,
            max_attempt_factor: 500,
            threads: Threads::Single,
        };
        let n = 5usize;

        // Independently recompute, slot by slot, how many slots (from 0) it takes to accept `n`
        // deals whose North hand satisfies `constraint` - the same acceptance test `sample_deals`
        // applies via `ln_likelihood`'s -infinity branch, just walked by hand here.
        let prepared = UniformProposal.prepare(&ctx).expect("prepares");
        let mut accepted = 0usize;
        let mut expected_attempts = 0u64;
        for slot in 0.. {
            let (slot_attempts, result) = process_slot(&ctx, None, prepared.as_ref(), &opts, slot);
            expected_attempts += slot_attempts;
            if result.is_some() {
                accepted += 1;
                if accepted == n {
                    break;
                }
            }
            assert!(
                slot < 10_000,
                "sanity: north_balanced_interpretation should accept far more often than this"
            );
        }
        // Sanity: the chosen seed/n must actually exercise the bug, i.e. the n-th acceptance must
        // not fall exactly on a chunk boundary (a multiple of `n` slots) - otherwise the buggy and
        // fixed code would coincide by chance and this test would not be a regression test at all.
        assert_ne!(
            expected_attempts % n as u64,
            0,
            "test setup does not exercise a discarded surplus; pick a different seed"
        );

        let (deals, report) =
            sample_deals(&ctx, &UniformProposal, n, &opts).expect("sampling succeeds");

        assert_eq!(report.produced, n);
        assert_eq!(deals.len(), n);
        assert_eq!(
            report.attempts, expected_attempts,
            "attempts must equal the slots actually needed to reach n accepted deals, not the \
             whole final chunk rounded up to a multiple of n"
        );
        assert_eq!(report.acceptance_rate, n as f64 / expected_attempts as f64);
        for weighted in &deals {
            assert!(constraint.satisfies(weighted.deal.hand(Seat::North)));
        }
    }

    /// A proposal whose every draw is rejected unless `rng`'s next word falls in the lowest
    /// `1 / accept_one_in` of its range: a stand-in for residual rejection with a known,
    /// hand-independent acceptance probability.
    struct RarelyAccepting {
        accept_one_in: u64,
    }

    impl Proposal for RarelyAccepting {
        fn prepare<'c>(
            &self,
            ctx: &'c SampleContext<'c>,
        ) -> Result<Box<dyn PreparedProposal + Send + Sync + 'c>, SampleError> {
            Ok(Box::new(PreparedRarely {
                inner: UniformProposal.prepare(ctx)?,
                accept_one_in: self.accept_one_in,
            }))
        }
    }

    struct PreparedRarely<'c> {
        inner: Box<dyn PreparedProposal + Send + Sync + 'c>,
        accept_one_in: u64,
    }

    impl PreparedProposal for PreparedRarely<'_> {
        fn propose(&self, rng: &mut dyn rand_core::Rng) -> Option<Deal> {
            if rng.next_u64() % self.accept_one_in != 0 {
                return None;
            }
            self.inner.propose(rng)
        }

        fn log_prob(&self, deal: &Deal) -> f64 {
            self.inner.log_prob(deal)
        }
    }

    /// The attempt budget `n × max_attempt_factor` is honoured (to within one slot's
    /// `max_attempts_per_sample`) whatever the chunking, and running out of it is reported both
    /// as `budget_exhausted` and as a `BudgetExhausted` warning; a run that finishes within the
    /// budget reports neither. `ess_per_attempt` is `ess / attempts`.
    #[test]
    fn attempt_budget_is_honoured_and_reported() {
        let (interpretation, _) = north_1nt_interpretation();
        let play_constraints = [
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
        ];
        let ctx = SampleContext {
            known: KnownCards::EMPTY,
            interpretation: &interpretation,
            play_constraints: &play_constraints,
            play_soft: None,
            bidding: None,
        };
        let n = 50usize;
        for threads in [Threads::Single, Threads::Auto] {
            let opts = SampleOptions {
                seed: 11,
                max_attempts_per_sample: 4,
                max_attempt_factor: 20,
                threads,
            };
            // About one draw in 100 passes the proposal and about one in 20 of those the 1NT
            // likelihood: 20n = 1000 attempts give about 0.5 deals.
            let (deals, report) =
                sample_deals(&ctx, &RarelyAccepting { accept_one_in: 100 }, n, &opts)
                    .expect("sampling runs");
            let budget = (n * 20) as u64;
            assert!(report.produced < n, "the budget should run out first");
            assert_eq!(report.produced, deals.len());
            assert!(
                report.attempts >= budget && report.attempts < budget + 4,
                "attempts {} not within one slot of the budget {budget}",
                report.attempts
            );
            assert!(report.budget_exhausted);
            assert!(report.warnings.contains(&SampleWarning::BudgetExhausted {
                attempts: report.attempts,
                produced: report.produced,
            }));
            assert!((report.ess_per_attempt - report.ess / report.attempts as f64).abs() < 1e-12);

            // Accepting every draw (only the 1NT likelihood rejects, about 19 in 20): a budget of
            // 400n is reached long after the quota.
            let generous = SampleOptions {
                max_attempts_per_sample: 64,
                max_attempt_factor: 400,
                ..opts
            };
            let (_, report) =
                sample_deals(&ctx, &RarelyAccepting { accept_one_in: 1 }, n, &generous)
                    .expect("sampling runs");
            assert_eq!(report.produced, n);
            assert!(!report.budget_exhausted);
            assert!(
                !report
                    .warnings
                    .iter()
                    .any(|w| matches!(w, SampleWarning::BudgetExhausted { .. }))
            );
        }
    }

    #[test]
    fn sample_deals_never_produces_a_violating_deal_and_report_is_consistent() {
        let (interpretation, constraint) = north_1nt_interpretation();
        let play_constraints = [
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
        ];
        let ctx = SampleContext {
            known: KnownCards::EMPTY,
            interpretation: &interpretation,
            play_constraints: &play_constraints,
            play_soft: None,
            bidding: None,
        };
        let opts = SampleOptions {
            seed: 123,
            max_attempts_per_sample: 64,
            max_attempt_factor: 200,
            threads: Threads::Single,
        };

        let n = 20;
        let (deals, report) =
            sample_deals(&ctx, &UniformProposal, n, &opts).expect("sampling succeeds");

        assert_eq!(report.requested, n);
        assert_eq!(report.produced, deals.len());
        assert!(report.produced <= n);
        assert!(report.attempts >= report.produced as u64);
        assert!(report.acceptance_rate >= 0.0 && report.acceptance_rate <= 1.0);
        assert!(report.ess >= 0.0);
        assert!(report.ess <= report.produced as f64 + 1e-9);

        for weighted in &deals {
            assert!(
                constraint.satisfies(weighted.deal.hand(Seat::North)),
                "a produced deal violated the 15-17 balanced constraint"
            );
            assert!(weighted.log_weight.is_finite());
        }

        // Every alternative here has the same weight (1.0) and North is the only seat with any
        // calls, so every produced deal's raw likelihood (and hence its log weight, since the
        // uniform proposal's log_prob is a context-wide constant) is identical; ESS should equal
        // the produced count exactly, up to floating-point slop.
        if report.produced > 0 {
            assert!((report.ess - report.produced as f64).abs() < 1e-6);
        }
    }

    /// A hard play constraint that admits no hand at all (an atom with an empty `ShapeSet`)
    /// must fail before any attempt is made, whatever the known cards are (§2.3 of
    /// `09-sample.md`).
    #[test]
    fn empty_support_early_return() {
        let unsatisfiable = HandConstraint::Atom(Atom {
            shapes: ShapeSet::EMPTY,
            hcp: 0..=37,
            cards: Vec::new(),
            eval: Vec::new(),
        });
        let play_constraints = [
            unsatisfiable,
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
        ];
        let interpretation = Interpretation {
            seats: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            per_call: Vec::new(),
            divergence: None,
        };
        let ctx = SampleContext {
            known: KnownCards::EMPTY,
            interpretation: &interpretation,
            play_constraints: &play_constraints,
            play_soft: None,
            bidding: None,
        };

        let result = sample_deals(&ctx, &UniformProposal, 10, &SampleOptions::default());
        assert!(
            matches!(result, Err(SampleError::EmptySupport)),
            "expected Err(SampleError::EmptySupport), got {result:?}"
        );
    }

    /// North's fixed card (the club ace) contradicts North's only bidding alternative ("never
    /// holds the club ace"); the hard play constraint alone is fine (`ANY`). Sampling must warn
    /// and fall back to dealing North as `ANY` rather than failing every attempt (§2.3).
    #[test]
    fn seat_fallback_warns() {
        let club_ace = Card::new(Suit::Clubs, Rank::Ace);
        let north_fixed = Hand::EMPTY.with(club_ace);
        let known = KnownCards::from_viewer(Seat::North, north_fixed);
        assert_eq!(known.needed(Seat::North), 12);

        let never_club_ace = HandConstraint::Atom(Atom {
            shapes: ShapeSet::ALL,
            hcp: 0..=37,
            cards: vec![CardRequirement {
                mask: north_fixed,
                count: 0..=0,
            }],
            eval: Vec::new(),
        });
        let explanation = Explanation {
            text: "never the club ace".to_string(),
            node: None,
            resolution: ResolutionKind::Exact,
            parts: Vec::new(),
        };
        let interpretation = Interpretation {
            seats: [
                vec![(never_club_ace, 1.0, explanation)],
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ],
            per_call: Vec::new(),
            divergence: None,
        };
        let play_constraints = [
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
        ];
        let ctx = SampleContext {
            known,
            interpretation: &interpretation,
            play_constraints: &play_constraints,
            play_soft: None,
            bidding: None,
        };
        let opts = SampleOptions {
            seed: 7,
            max_attempts_per_sample: 32,
            max_attempt_factor: 100,
            threads: Threads::Single,
        };

        let n = 20;
        let (deals, report) = sample_deals(&ctx, &UniformProposal, n, &opts)
            .expect("the ANY fallback lets every attempt succeed");

        assert_eq!(
            report.produced, n,
            "the fallback should let every slot succeed, same as an unconstrained seat"
        );
        assert!(
            report
                .warnings
                .contains(&SampleWarning::EmptySupport { seat: Seat::North }),
            "warnings = {:?}",
            report.warnings
        );
        for weighted in &deals {
            assert!(
                weighted.deal.hand(Seat::North).contains(club_ace),
                "the known club ace must still be in North's hand"
            );
        }
    }

    /// A fully-known seat (`needed == 0`) whose fixed hand violates its own hard play constraint
    /// makes no deal possible at all, whatever the other seats hold — this must be caught
    /// up front (§2.3 of `09-sample.md`), not discovered by exhausting the attempt budget.
    #[test]
    fn known_seat_hard_violation() {
        // North is fully known (all clubs — no spades at all), but its hard play constraint
        // requires at least one spade.
        let north_hand = Hand::EMPTY.with_holding(Suit::Clubs, bridge_core::Holding::FULL);
        let known = KnownCards::from_viewer(Seat::North, north_hand);
        assert_eq!(known.needed(Seat::North), 0);

        let requires_a_spade = HandConstraint::Atom(Atom {
            shapes: ShapeSet::ALL,
            hcp: 0..=37,
            cards: vec![CardRequirement {
                mask: Hand::EMPTY.with_holding(Suit::Spades, bridge_core::Holding::FULL),
                count: 1..=13,
            }],
            eval: Vec::new(),
        });
        let play_constraints = [
            requires_a_spade,
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
        ];
        let interpretation = Interpretation {
            seats: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            per_call: Vec::new(),
            divergence: None,
        };
        let ctx = SampleContext {
            known,
            interpretation: &interpretation,
            play_constraints: &play_constraints,
            play_soft: None,
            bidding: None,
        };

        let result = sample_deals(&ctx, &UniformProposal, 20, &SampleOptions::default());
        assert!(
            matches!(result, Err(SampleError::EmptySupport)),
            "expected Err(SampleError::EmptySupport), got {result:?}"
        );
    }

    /// A seat whose only alternative is a `HandConstraint::Custom` predicate is not
    /// `Sampler::is_exact()` — `log_prob`'s density for it is only approximate — and that must be
    /// surfaced as `SampleWarning::CustomConstraint`, not silently dropped (the §2.3 probe used to
    /// check `HandConstraint::is_samplable()` instead, which is blind to inexactness that isn't a
    /// bare `Custom` node, and in any case never ran the check for a seat with no calls at all).
    #[test]
    fn custom_constraint_warns_for_inexact_alternative() {
        let never_void_in_spades = HandConstraint::Custom(CustomPred {
            name: "never void in spades".to_string(),
            f: Arc::new(|hand: Hand| !hand.holding(Suit::Spades).is_empty()),
        });
        let interpretation = Interpretation {
            seats: [
                vec![(never_void_in_spades, 1.0, empty_explanation())],
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ],
            per_call: Vec::new(),
            divergence: None,
        };
        let play_constraints = [
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
        ];
        let ctx = SampleContext {
            known: KnownCards::EMPTY,
            interpretation: &interpretation,
            play_constraints: &play_constraints,
            play_soft: None,
            bidding: None,
        };

        let (_, report) = sample_deals(&ctx, &UniformProposal, 20, &SampleOptions::default())
            .expect("a Custom predicate is still satisfiable by most deals, just not exactly");
        assert!(
            report
                .warnings
                .contains(&SampleWarning::CustomConstraint { seat: Seat::North }),
            "warnings = {:?}",
            report.warnings
        );
    }

    fn empty_explanation() -> Explanation {
        Explanation {
            text: String::new(),
            node: None,
            resolution: ResolutionKind::Exact,
            parts: Vec::new(),
        }
    }
}
