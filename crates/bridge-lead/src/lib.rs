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

use bridge_bidding::{BidContext, ImplicitPass, PolicyParams, Scoring, interpret};
use bridge_constraint::{HandConstraint, KnownCards};
use bridge_core::{Card, Seat, Strain};
use bridge_sample::{BiddingLikelihood, SampleContext, WeightedDeal, sample_deals};

pub use advice::{LeadAdvice, LeadScore};
pub use options::{LeadOptions, LeadScoring};
pub use query::LeadQuery;

// Re-exported so a caller of `advise` does not have to add every lower crate as its own
// dependency just to name the types in its signature.
pub use bridge::dd::{DdError, DoubleDummy};
pub use bridge_bidding::{InterpretOptions, Table};
pub use bridge_sample::{Proposal, SampleOptions, UniformProposal};

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
    /// A double-dummy query failed.
    #[error("double-dummy solver failed: {0}")]
    Dd(#[from] DdError),
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
    let trump = contract.bid.strain();
    let vulnerable = auction.vulnerability().is_vulnerable(declarer);
    // Tricks the defence needs to defeat the contract: declarer needs `6 + level`, and there are
    // 13 tricks in total, so the defence needs `13 - (6 + level) + 1 = 8 - level`
    // (`docs/design/14-lead.md` §3 step 7).
    let threshold = 8 - contract.bid.level();

    let known = KnownCards::from_viewer(leader, query.leader_hand);
    let interpretation = interpret(table, auction, &opts.interpret);
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
        policy: PolicyParams::default(),
    };
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

    let sample_opts = SampleOptions {
        seed: opts.seed,
        ..opts.sample
    };
    let (deals, sample_report) = sample_deals(&ctx, proposal, opts.samples, &sample_opts)?;

    let per_deal = lead_scores_for_all(dd, &deals, trump, leader)?;

    let cards: Vec<Card> = query.leader_hand.cards().collect();
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
