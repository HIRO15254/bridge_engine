//! Deal sampling: the output is a set of weighted deals, not a density.
//!
//! A [`Proposal`] draws deals consistent with the known cards; [`sample_deals`] corrects each
//! draw with an importance weight `w = L(deal) / π(deal)` where `L` is the bidding likelihood
//! (times the play constraints) and `π` the proposal density, both in the log domain, and
//! reports the effective sample size. Sample `i` is computed from an RNG derived from
//! `(seed, i)` only, so results are identical whatever the thread count.
#![forbid(unsafe_code)]
#![warn(missing_docs)]
// `constraint_proposal.rs` (phase 5, out of this lane's scope) still has `todo!()` bodies whose
// unused parameters and never-constructed `PreparedConstraint` would otherwise warn; keeping the
// crate-level allow was checked to still be necessary for that reason alone (removing it and
// re-running clippy only surfaces warnings from that file, none from the phase-2 code here).
#![allow(dead_code, unused_variables)]

mod constraint_proposal;
mod proposal;
mod report;
mod rng;
mod uniform;
mod weights;

use core::ops::Range;

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
/// slot; a slot that never succeeds contributes nothing. Sampling stops at a chunk boundary once
/// `n` deals have been produced or `n × max_attempt_factor` attempts have been made in total,
/// and the first `n` produced deals (in slot order) are returned.
///
/// Not yet checked here (tracked as an open issue): whether a seat's hard play constraint, or
/// its interpretation alternatives, admit any hand at all given the known cards. That check
/// needs `bridge_constraint::Sampler`, which is still `todo!()`; until then, an unsatisfiable
/// seat simply never contributes a finite-weight deal, which surfaces as `Truncated`/`LowEss`
/// rather than as `SampleError::EmptySupport` or `SampleWarning::EmptySupport`.
pub fn sample_deals(
    ctx: &SampleContext<'_>,
    proposal: &dyn Proposal,
    n: usize,
    opts: &SampleOptions,
) -> Result<(Vec<WeightedDeal>, SampleReport), SampleError> {
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

    let prepared = proposal.prepare(ctx)?;

    let mut warnings = Vec::new();
    for seat in Seat::ALL {
        let hard_samplable = ctx.play_constraints[seat.index() as usize].is_samplable();
        let alternatives_samplable = ctx.interpretation.seats[seat.index() as usize]
            .iter()
            .all(|(constraint, _, _)| constraint.is_samplable());
        if !hard_samplable || !alternatives_samplable {
            warnings.push(SampleWarning::CustomConstraint { seat });
        }
    }

    let mut deals: Vec<WeightedDeal> = Vec::new();
    let mut attempts: u64 = 0;
    if n > 0 && opts.max_attempts_per_sample > 0 {
        let max_attempts_total = (n as u64).saturating_mul(u64::from(opts.max_attempt_factor));
        let mut chunk_start: usize = 0;
        loop {
            let chunk_end = chunk_start + n;
            let results = run_chunk(ctx, prepared.as_ref(), opts, chunk_start..chunk_end);

            let mut chunk_attempts = 0u64;
            for (slot_attempts, result) in results {
                chunk_attempts += slot_attempts;
                if let Some(weighted) = result {
                    deals.push(weighted);
                }
            }
            attempts += chunk_attempts;
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
    let acceptance_rate = if attempts > 0 {
        produced as f64 / attempts as f64
    } else {
        0.0
    };

    let report = SampleReport {
        requested: n,
        produced,
        attempts,
        acceptance_rate,
        ess,
        ess_ratio,
        log_weight_max,
        elapsed: start.elapsed(),
        warnings,
    };

    tracing::info!(
        requested = report.requested,
        produced = report.produced,
        attempts = report.attempts,
        acceptance_rate = report.acceptance_rate,
        ess = report.ess,
        ess_ratio = report.ess_ratio,
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
    prepared: &(dyn PreparedProposal + Send + Sync),
    opts: &SampleOptions,
    slots: Range<usize>,
) -> Vec<(u64, Option<WeightedDeal>)> {
    use rayon::prelude::*;

    match opts.threads {
        Threads::Auto => slots
            .into_par_iter()
            .map(|slot| process_slot(ctx, prepared, opts, slot))
            .collect(),
        Threads::Single => slots
            .map(|slot| process_slot(ctx, prepared, opts, slot))
            .collect(),
    }
}

/// Sequential fallback when the `parallel` feature is disabled (`Threads::Auto` behaves like
/// `Threads::Single`).
#[cfg(not(feature = "parallel"))]
fn run_chunk(
    ctx: &SampleContext<'_>,
    prepared: &dyn PreparedProposal,
    opts: &SampleOptions,
    slots: Range<usize>,
) -> Vec<(u64, Option<WeightedDeal>)> {
    slots
        .map(|slot| process_slot(ctx, prepared, opts, slot))
        .collect()
}

/// Runs one sampling slot: up to `opts.max_attempts_per_sample` proposals from
/// `rng_for(opts.seed, slot)`, stopping at the first finite log weight. Returns the number of
/// attempts made and the produced deal, if any.
fn process_slot(
    ctx: &SampleContext<'_>,
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
        let ln_l = ln_likelihood(ctx, &deal);
        let log_weight = ln_l - ln_pi;
        if log_weight.is_finite() {
            return (attempts, Some(WeightedDeal { deal, log_weight }));
        }
    }
    (attempts, None)
}

/// `ln L(d)` (§3 of `09-sample.md`): `-∞` if any seat violates its hard play constraint;
/// otherwise the bidding term (`sequence_log_likelihood` when `ctx.bidding` is known, else the
/// sum of `interpretation.likelihood`) plus, per seat with a non-empty soft list, the log of its
/// mixture mass (a seat with no soft alternatives contributes nothing, not `-∞`).
fn ln_likelihood(ctx: &SampleContext<'_>, deal: &Deal) -> f64 {
    for seat in Seat::ALL {
        if !ctx.play_constraints[seat.index() as usize].satisfies(deal.hand(seat)) {
            return f64::NEG_INFINITY;
        }
    }

    let mut ln_l = match &ctx.bidding {
        Some(bidding) => bridge_bidding::sequence_log_likelihood(
            bidding.table,
            deal,
            bidding.auction,
            bidding.ctx,
        ),
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
    use bridge_bidding::{CallExplanation, CallInterpretation, Interpretation, ResolutionKind};
    use bridge_constraint::{Atom, HandConstraint, KnownCards, ShapeSet};
    use bridge_core::{Bid, Call, Hand, Strain};

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
            let ln_l = ln_likelihood(&ctx, &deal);
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
}
