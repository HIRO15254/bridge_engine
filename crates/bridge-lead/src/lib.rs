//! Opening-lead advisor: samples deals consistent with the auction and ranks the leader's cards
//! by their double-dummy defence tricks (`docs/design/14-lead.md`, roadmap phase 6).
//!
//! [`advise`] ties together three lower layers, none of which it re-implements:
//!
//! - [`bridge_bidding::interpret`] turns the auction into a weighted disjunction of hand
//!   constraints per seat.
//! - [`bridge_sample::sample_deals`] draws deals consistent with the leader's known hand,
//!   weighted by [`bridge_bidding::sequence_log_likelihood`] (the bidding policy's likelihood of
//!   the auction actually having happened).
//! - [`bridge::dd::DoubleDummy::lead_scores`] scores every one of the leader's cards, on each
//!   sampled deal.
//!
//! This crate only aggregates those double-dummy scores under the sampler's self-normalised
//! importance weights (mean, standard error, a set-probability, and a grouping of cards that
//! scored identically in every sample) and ranks them. It holds no bidding or play judgement of
//! its own (the same principle as `bridge-bidding`'s module doc).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod advice;
mod aggregate;
mod options;
mod query;
mod scoring;

use bridge_bidding::{BidContext, ImplicitPass, Scoring, interpret};
use bridge_constraint::{HandConstraint, KnownCards};
use bridge_core::{Card, Contract, Seat, Strain};
use bridge_sample::{BiddingLikelihood, SampleContext, WeightedDeal, sample_deals};

pub use advice::{LeadAdvice, LeadScore};
pub use options::{LeadOptions, LeadScoring};
pub use query::LeadQuery;

// Re-exported so a caller of `advise` does not have to add every lower crate as its own
// dependency just to name the types in its signature.
pub use bridge::dd::{DdError, DoubleDummy};
pub use bridge_bidding::{InterpretOptions, PolicyParams, Table};
pub use bridge_sample::{ConstraintProposal, Proposal, SampleOptions, UniformProposal};

/// A [`advise`] call could not produce advice.
#[derive(Debug, thiserror::Error)]
pub enum LeadError {
    /// The auction is not yet complete.
    #[error("the auction is not complete")]
    IncompleteAuction,
    /// The auction was passed out (no contract, so no opening lead).
    #[error("the auction was passed out")]
    PassedOut,
    /// The leader's hand does not have exactly 13 cards.
    #[error("the leader's hand has {got} cards, not 13")]
    WrongHandSize {
        /// The number of cards actually given.
        got: usize,
    },
    /// Deal sampling failed.
    #[error("sampling failed: {0}")]
    Sample(#[from] bridge_sample::SampleError),
    /// Sampling completed without error but produced zero deals: either `opts.samples == 0`, or
    /// every proposal attempt was rejected (plausible with `ConstraintProposal` when the leader's
    /// own hand contradicts the auction's interpretation). Without at least one deal there is
    /// nothing to aggregate, so `advise` reports this rather than fabricating advice from an empty
    /// sample (`14-lead.md` §3 step 5).
    #[error("sampling produced no deals")]
    NoSamples {
        /// The (empty) sample report, for diagnostics.
        report: bridge_sample::SampleReport,
    },
    /// A double-dummy query failed.
    #[error("double-dummy solver failed: {0}")]
    Dd(#[from] DdError),
}

/// The deal proposal the lead advisor uses (`docs/design/14-lead.md` §3): [`ConstraintProposal`]
/// with residual rejection at an acceptance floor of 0.125 instead of the sampler's default 0.5.
///
/// Every produced deal costs a double-dummy solve, far more than a rejected attempt, so flatter
/// weights are worth more attempts here: on the corpus evaluation (100 eval-split boards, 100
/// samples) the median ESS is 86 at this floor, against 35 without residual rejection.
pub fn lead_proposal() -> ConstraintProposal {
    ConstraintProposal {
        residual_rejection: true,
        residual_min_acceptance: 0.125,
        ..ConstraintProposal::default()
    }
}

/// Advises on the opening lead for `query.auction`'s contract.
///
/// `table` provides the systems used to interpret the auction and to weight sampled deals by how
/// likely the auction was to have been bid (`docs/design/14-lead.md` §3). `proposal` draws the
/// deals (`bridge_sample::UniformProposal` needs no system compilation; `ConstraintProposal`
/// samples more efficiently from the interpreted constraints once it exists, phase 5). `dd`
/// solves each sampled deal's opening lead.
///
/// # Errors
/// See [`LeadError`]. In particular, the auction must be complete and not passed out, and
/// `query.leader_hand` must have exactly 13 cards.
pub fn advise(
    table: &Table,
    query: &LeadQuery<'_>,
    proposal: &dyn Proposal,
    dd: &dyn DoubleDummy,
    opts: &LeadOptions,
) -> Result<LeadAdvice, LeadError> {
    let auction = query.auction;
    if !auction.is_complete() {
        return Err(LeadError::IncompleteAuction);
    }
    let Some(contract) = auction.contract() else {
        return Err(LeadError::PassedOut);
    };
    let got = query.leader_hand.len() as usize;
    if got != 13 {
        return Err(LeadError::WrongHandSize { got });
    }

    let declarer = contract.declarer;
    let leader = contract.leader();
    let vulnerable = auction.vulnerability().is_vulnerable(declarer);

    let known = KnownCards::from_viewer(leader, query.leader_hand);
    // No play has happened yet, so every seat's hard constraint is unconstrained. `HandConstraint`
    // is not `Copy` (`bridge-constraint/src/constraint.rs`), hence the explicit 4-element literal
    // rather than `[HandConstraint::ANY; 4]`.
    let play_constraints = [
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
    ];

    // `scoring` is a pass-through in `bridge-bidding` v1 (07-bidding.md §5.1's "未決" item 4) and
    // `natural: None` is equivalent to `Some(&table.natural)` here, since
    // `sequence_log_likelihood` substitutes `table.natural` whenever `ctx.natural` is `None`
    // (`bridge-bidding/src/policy.rs`); `ImplicitPass::Complement` is the documented default for
    // applications (as opposed to property tests), which this crate is.
    let bid_ctx = BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Complement,
        policy: opts.policy,
    };
    // The interpretation is the mirror of the likelihood's own policy (D19), so the proposal
    // and the weights never disagree about which hands the auction allows.
    let mirror = InterpretOptions::for_context(&bid_ctx);
    let interpret_opts = InterpretOptions {
        policy: mirror.policy,
        implicit_pass: mirror.implicit_pass,
        ..opts.interpret
    };
    let interpretation = interpret(table, auction, &interpret_opts);
    let bidding = BiddingLikelihood {
        table,
        auction,
        ctx: &bid_ctx,
    };
    let ctx = SampleContext {
        known,
        interpretation: &interpretation,
        play_constraints: &play_constraints,
        play_soft: None,
        bidding: Some(bidding),
    };

    advise_with_context(&ctx, contract, vulnerable, proposal, dd, opts)
}

/// The shared tail of [`advise`]: sampling, double-dummy scoring and aggregation, given an
/// already-built [`SampleContext`].
///
/// `#[doc(hidden)]`, not part of the crate's public contract: it exists so a caller that needs to
/// compare against `advise` on equal footing — e.g. the corpus-evaluation harness's "no bidding
/// information" baseline (`14-lead.md` §4) — can reuse the exact same sampling-to-ranking
/// pipeline (including equivalence grouping) with its own [`SampleContext`] (typically the
/// vacuous interpretation and `bidding: None`) instead of re-implementing it.
///
/// `contract` and `vulnerable` are not derivable from `ctx` alone (a [`SampleContext`] does not
/// carry the auction when `bidding` is `None`), so the caller supplies them; `leader` and `trump`
/// are then derived from `contract` exactly as [`advise`] derives them, and the leader's known
/// hand (hence the cards ranked) comes from `ctx.known`.
#[doc(hidden)]
pub fn advise_with_context(
    ctx: &SampleContext<'_>,
    contract: Contract,
    vulnerable: bool,
    proposal: &dyn Proposal,
    dd: &dyn DoubleDummy,
    opts: &LeadOptions,
) -> Result<LeadAdvice, LeadError> {
    let declarer = contract.declarer;
    let leader = contract.leader();
    let trump = contract.bid.strain();
    // Tricks the defence needs to defeat the contract: declarer needs `6 + level`, and there are
    // 13 tricks in total, so the defence needs `13 - (6 + level) + 1 = 8 - level`
    // (`docs/design/14-lead.md` §3 step 7).
    let threshold = 8 - contract.bid.level();

    let sample_opts = SampleOptions {
        seed: opts.seed,
        ..opts.sample
    };
    let (deals, sample_report) = sample_deals(ctx, proposal, opts.samples, &sample_opts)?;
    if sample_report.produced == 0 {
        return Err(LeadError::NoSamples {
            report: sample_report,
        });
    }

    let per_deal = lead_scores_for_all(dd, &deals, trump, leader)?;

    let cards: Vec<Card> = ctx.known.known[leader.index() as usize].cards().collect();
    let weights = WeightedDeal::normalized_weights(&deals);
    let leads = aggregate::aggregate(
        &cards,
        &per_deal,
        &weights,
        sample_report.ess,
        threshold,
        contract,
        vulnerable,
        opts.scoring,
        opts.top_k,
    );

    Ok(LeadAdvice {
        contract,
        declarer,
        leader,
        leads,
        samples_used: sample_report.produced,
        ess: sample_report.ess,
        sample_report,
    })
}

/// Runs `dd.lead_scores` on every sampled deal, in order.
///
/// Under the `parallel` feature the calls run on rayon's global pool
/// (`par_iter().map(..).collect()`, which preserves the input order); each deal's solve is
/// independent of every other's, so the result does not depend on how the work was scheduled
/// (`docs/design/14-lead.md` §3 step 6).
#[cfg(feature = "parallel")]
fn lead_scores_for_all(
    dd: &dyn DoubleDummy,
    deals: &[WeightedDeal],
    trump: Strain,
    leader: Seat,
) -> Result<Vec<Vec<(Card, u8)>>, LeadError> {
    use rayon::prelude::*;

    deals
        .par_iter()
        .map(|weighted| {
            dd.lead_scores(&weighted.deal, trump, leader)
                .map_err(LeadError::from)
        })
        .collect()
}

/// Sequential fallback when the `parallel` feature is disabled.
#[cfg(not(feature = "parallel"))]
fn lead_scores_for_all(
    dd: &dyn DoubleDummy,
    deals: &[WeightedDeal],
    trump: Strain,
    leader: Seat,
) -> Result<Vec<Vec<(Card, u8)>>, LeadError> {
    deals
        .iter()
        .map(|weighted| {
            dd.lead_scores(&weighted.deal, trump, leader)
                .map_err(LeadError::from)
        })
        .collect()
}
