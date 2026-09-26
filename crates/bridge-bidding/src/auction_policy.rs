//! `AuctionPolicy`: the per-auction fast path of the policy likelihood (07-bidding.md §6.2).

use std::sync::Arc;

use bridge_core::{Auction, Deal};

use crate::{
    BidContext, ImplicitPass, NaturalInference, PolicyParams, Scoring, Table,
    sequence_log_likelihood,
};

/// The policy likelihood of one fixed auction, prepared once and evaluated per deal.
///
/// `AuctionPolicy::new` builds, once per auction, each call's pieces (membership form) and
/// `log_scale` under the policy of `ctx`; [`AuctionPolicy::log_likelihood`] then evaluates
/// `Σ_j [log_scale_j + ln D_j(h_{s_j})]` with membership tests only. It equals the reference
/// [`sequence_log_likelihood`] with `|Δ ln L| <= 1e-5`. The natural engine is `ctx.natural`, or
/// `table.natural` when `ctx.natural` is `None` (as in `sequence_log_likelihood`).
///
/// Owns everything it needs (the table's `Arc`s, the auction and the policy parameters), so it
/// can live in a cache next to the interpretation of the same auction.
///
/// Naive; replaced in phase 4 lane B: `log_likelihood` currently delegates to
/// `sequence_log_likelihood` on every call.
#[derive(Clone, Debug)]
pub struct AuctionPolicy {
    table: Table,
    auction: Auction,
    scoring: Scoring,
    implicit_pass: ImplicitPass,
    policy: PolicyParams,
    natural: Arc<NaturalInference>,
}

impl AuctionPolicy {
    /// Prepares the likelihood of `auction` under `table` and the policy of `ctx`.
    pub fn new(table: &Table, auction: &Auction, ctx: &BidContext<'_>) -> AuctionPolicy {
        let natural = match ctx.natural {
            Some(engine) if !std::ptr::eq(engine, table.natural.as_ref()) => {
                Arc::new(engine.clone())
            }
            _ => table.natural.clone(),
        };
        AuctionPolicy {
            table: table.clone(),
            auction: auction.clone(),
            scoring: ctx.scoring,
            implicit_pass: ctx.implicit_pass,
            policy: ctx.policy,
            natural,
        }
    }

    /// The auction this policy scores.
    pub fn auction(&self) -> &Auction {
        &self.auction
    }

    /// The policy parameters it was built with.
    pub fn policy(&self) -> PolicyParams {
        self.policy
    }

    /// `ln L(deal) = Σ_j ln p_j(calls[j] | hand of the caller of j)`: the same value as
    /// [`sequence_log_likelihood`] for this table, auction and context (`-∞` never occurs for
    /// `ε > 0`).
    pub fn log_likelihood(&self, deal: &Deal) -> f64 {
        let ctx = BidContext {
            scoring: self.scoring,
            natural: Some(self.natural.as_ref()),
            implicit_pass: self.implicit_pass,
            policy: self.policy,
        };
        sequence_log_likelihood(&self.table, deal, &self.auction, &ctx)
    }
}
