//! A caller-owned memo of interpretations and policy likelihoods (the `SystemIR` itself never
//! caches, and `Table` carries no hidden cache).

use std::collections::HashMap;
use std::sync::Arc;

use bridge_core::{Auction, Call, Seat, Vulnerability};

use crate::{
    AuctionPolicy, BidContext, ImplicitPass, InterpretMode, InterpretOptions, Interpretation,
    Table, interpret,
};

/// The bits of the options that change an interpretation.
type OptionsKey = (usize, bool, InterpretMode, [u32; 3], bool, [u32; 4]);

/// The bits of the policy that change an [`AuctionPolicy`] (plus the natural engine's address).
type PolicyKey = ([u32; 3], bool, usize);

fn options_key(opts: &InterpretOptions) -> OptionsKey {
    (
        opts.max_alternatives,
        opts.strict,
        opts.mode,
        [
            opts.policy.epsilon.to_bits(),
            opts.policy.deviation.to_bits(),
            opts.policy
                .legacy_temperature
                .map_or(u32::MAX, f32::to_bits),
        ],
        opts.implicit_pass == ImplicitPass::Complement,
        [
            opts.eps_exact.to_bits(),
            opts.eps_partial.to_bits(),
            opts.eps_natural.to_bits(),
            opts.lenient_decay.to_bits(),
        ],
    )
}

type AuctionKey = (Seat, Vulnerability, Vec<Call>);

/// Memoises [`interpret`](crate::interpret) by `(dealer, vulnerability, calls, options)` and
/// [`AuctionPolicy::new`] by `(dealer, vulnerability, calls, policy)`. The table is not part of
/// the key: use one cache per table.
#[derive(Default)]
pub struct InterpretCache {
    map: HashMap<(AuctionKey, OptionsKey), Arc<Interpretation>>,
    policies: HashMap<(AuctionKey, PolicyKey), Arc<AuctionPolicy>>,
}

fn auction_key(auction: &Auction) -> AuctionKey {
    (
        auction.dealer(),
        auction.vulnerability(),
        auction.calls().to_vec(),
    )
}

impl InterpretCache {
    /// An empty cache.
    pub fn new() -> InterpretCache {
        InterpretCache::default()
    }

    /// Returns the cached interpretation or computes and stores it.
    pub fn get_or_interpret(
        &mut self,
        table: &Table,
        auction: &Auction,
        opts: &InterpretOptions,
    ) -> Arc<Interpretation> {
        let key = (auction_key(auction), options_key(opts));
        if let Some(existing) = self.map.get(&key) {
            return existing.clone();
        }
        let interpretation = Arc::new(interpret(table, auction, opts));
        self.map.insert(key, interpretation.clone());
        interpretation
    }

    /// Returns the cached [`AuctionPolicy`] of `auction` under `ctx`, or builds and stores it.
    pub fn get_or_policy(
        &mut self,
        table: &Table,
        auction: &Auction,
        ctx: &BidContext<'_>,
    ) -> Arc<AuctionPolicy> {
        let natural = ctx.natural.unwrap_or(table.natural.as_ref());
        let key = (
            auction_key(auction),
            (
                [
                    ctx.policy.epsilon.to_bits(),
                    ctx.policy.deviation.to_bits(),
                    ctx.policy.legacy_temperature.map_or(u32::MAX, f32::to_bits),
                ],
                ctx.implicit_pass == ImplicitPass::Complement,
                natural as *const _ as usize,
            ),
        );
        if let Some(existing) = self.policies.get(&key) {
            return existing.clone();
        }
        let policy = Arc::new(AuctionPolicy::new(table, auction, ctx));
        self.policies.insert(key, policy.clone());
        policy
    }

    /// Number of cached interpretations.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// `true` when no interpretation is cached.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}
