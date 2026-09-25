//! `policy_argmax_matches_choose_bid` (07-bidding.md §6.1, §8): as `τ → 0`,
//! `argmax_c call_distribution(c)` equals `choose_bid(...).call()`.

mod common;

use bridge_bidding::{
    BidContext, ImplicitPass, PolicyParams, Scoring, call_distribution, choose_bid,
};
use bridge_core::{Auction, Seat, Strain, Vulnerability};
use common::*;
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;

const TRIALS_PER_POSITION: usize = 5_000;

fn ctx() -> BidContext<'static> {
    BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Never,
        policy: PolicyParams {
            temperature: 0.01,
            epsilon: 1e-3,
        },
    }
}

/// Runs the property at one auction prefix, over `TRIALS_PER_POSITION` random hands.
fn check_position(sys: &bridge_system::SystemIR, prefix: &Auction) {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x00C0_FFEE);
    let ctx = ctx();
    let mut checked = 0usize;

    for _ in 0..TRIALS_PER_POSITION {
        let hand = random_hand13(&mut rng);
        let choice = choose_bid(sys, hand, prefix, &ctx);
        let Some(expected) = choice.call() else {
            continue; // NoCandidate: nothing to compare (no legal call has positive priority).
        };
        let dist = call_distribution(sys, hand, prefix, &ctx);
        let p_max = dist
            .iter()
            .map(|(_, p)| *p)
            .fold(f32::NEG_INFINITY, f32::max);
        // `choose_bid` breaks ties (equal system priority) via `SystemMeta::tie_break`, which
        // `call_distribution` has no notion of (it only sums by *priority*, so tied candidates get
        // exactly equal probability). So rather than asserting our own untie-broken argmax equals
        // `choose_bid`'s pick, we assert the weaker, well-defined half of "argmax = choose_bid":
        // the call `choose_bid` picks always attains the distribution's maximum probability.
        let p_chosen = dist
            .iter()
            .find(|(c, _)| *c == expected)
            .map(|(_, p)| *p)
            .unwrap_or_else(|| panic!("choose_bid picked {expected:?}, not among legal_calls"));
        assert!(
            (p_chosen - p_max).abs() < 1e-3,
            "choose_bid picked {expected:?} with p={p_chosen}, but max p over the distribution is \
             {p_max} for hand {hand:?}"
        );
        checked += 1;
    }
    // Sanity: most random hands should have hit *some* candidate (openings alone cover a wide
    // HCP range), so this is not a vacuously-passing test.
    assert!(checked > TRIALS_PER_POSITION / 4, "checked only {checked}");
}

#[test]
fn policy_argmax_matches_choose_bid_opening() {
    let sys = sayc_system();
    let empty = Auction::new(Seat::North, Vulnerability::None);
    check_position(&sys.sys, &empty);
}

#[test]
fn policy_argmax_matches_choose_bid_response_to_1h() {
    let sys = sayc_system();
    let after_1h = auction(Seat::North, Vulnerability::None, &[bid(1, Strain::Hearts)]);
    check_position(&sys.sys, &after_1h);
}
