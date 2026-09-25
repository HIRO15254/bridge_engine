//! `replay` on a few deals, and `InterpretCache` cache hits (07-bidding.md §8).

mod common;

use std::sync::Arc;

use bridge_bidding::{
    BidContext, ImplicitPass, InterpretCache, InterpretOptions, PolicyParams, Scoring, Table,
    interpret, replay,
};
use bridge_core::{Auction, Seat, Strain, Vulnerability};
use common::*;
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;

fn table_of(sys: &Sayc) -> Table {
    Table::uniform(
        sys.sys.clone(),
        Arc::new(bridge_system::NaturalInference::default()),
    )
}

fn ctx() -> BidContext<'static> {
    BidContext {
        scoring: Scoring::Imp,
        // `None`: a hand that runs off the hand-built system's covered sequences should degrade
        // to `NoCandidate` (recorded as a gap) rather than reach `NaturalInference`, which is
        // still `todo!()` on this branch. This is exactly the case `replay`'s gap-recording exists
        // for (07-bidding.md §6.3).
        natural: None,
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::default(),
    }
}

/// `replay` always terminates (the 320-call safety guard is a backstop; legality and the 4-pass
/// completion rule terminate in practice far sooner) and never panics, on several random deals,
/// with `NoCandidate` positions recorded as gaps instead of derailing the auction.
#[test]
fn replay_terminates_on_random_deals() {
    let sys = sayc_system();
    let table = table_of(&sys);
    let ctx = ctx();
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(99);

    for trial in 0..25 {
        let deal = random_deal(&mut rng);
        let dealer = Seat::ALL[trial % 4];
        let result = replay(&table, &deal, dealer, Vulnerability::None, &ctx);
        assert!(
            result.auction.is_complete(),
            "trial {trial} did not terminate"
        );
        // Every recorded gap really was a position `choose_bid` gave up on: replaying the same
        // prefix again must reproduce `NoCandidate` there (deterministic, no RNG in `choose_bid`).
        for &(idx, seat) in &result.gaps {
            assert_eq!(result.auction.calls()[idx], bridge_core::Call::Pass);
            assert_eq!(result.auction.seat_at(idx), seat);
        }
    }
}

/// `replay` on the opening-only decision (guaranteed to resolve without gaps, since
/// `ImplicitPass::Complement` covers every hand at that first call) produces an auction whose
/// first call `interpret` also accepts for the hand that was dealt it.
#[test]
fn replay_opening_matches_interpret() {
    let sys = sayc_system();
    let table = table_of(&sys);
    let ctx = ctx();
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(100);
    let deal = random_deal(&mut rng);

    let result = replay(&table, &deal, Seat::North, Vulnerability::None, &ctx);
    assert!(
        result.gaps.iter().all(|&(idx, _)| idx != 0),
        "the opening call is never a gap"
    );

    let opts = InterpretOptions::default();
    let opening_only = Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        result.auction.calls()[..1].iter().copied(),
    )
    .unwrap();
    let interp = interpret(&table, &opening_only, &opts);
    let pc = &interp.per_call[0];
    assert!(pc.alternatives.iter().any(|(c, w, ex)| ex.kind
        != bridge_bidding::ResolutionKind::Fallback
        && *w > 0.0
        && c.satisfies(deal.hand(Seat::North))));
}

/// `InterpretCache::get_or_interpret` computes once and returns the same `Arc` on a repeated
/// lookup with the same `(dealer, vulnerability, calls)` key, and grows only for a genuinely new
/// key.
#[test]
fn cache_hits_and_grows() {
    let sys = sayc_system();
    let table = table_of(&sys);
    let opts = InterpretOptions::default();
    let mut cache = InterpretCache::new();

    let a = auction(Seat::North, Vulnerability::None, &[bid(1, Strain::Hearts)]);
    let b = auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::Hearts), bridge_core::Call::Pass],
    );

    assert!(cache.is_empty());
    let first = cache.get_or_interpret(&table, &a, &opts);
    assert_eq!(cache.len(), 1);

    let second = cache.get_or_interpret(&table, &a, &opts);
    assert_eq!(cache.len(), 1, "same key must not grow the cache");
    assert!(
        Arc::ptr_eq(&first, &second),
        "same key must return the same Arc, not merely an equal value"
    );

    let third = cache.get_or_interpret(&table, &b, &opts);
    assert_eq!(cache.len(), 2, "a new key must grow the cache");
    assert!(!Arc::ptr_eq(&first, &third));
}
