//! `policy_argmax_matches_choose_bid` (07-bidding.md §6.1, §8): `argmax_c call_distribution(c)`
//! equals `choose_bid(...).call()`. Under the phase-4 policy (docs/design/15-phase4-plan.md D18,
//! `PolicyParams::system_players()`) this is a structural identity for any `δ < 1/2`.

mod common;

use bridge_bidding::{
    BidChoice, BidContext, ImplicitPass, PolicyParams, Scoring, Table, call_distribution,
    choose_bid,
};
use bridge_core::{Auction, Seat, Strain, Vulnerability};
use bridge_system::ast::{SeatCond, VulCond};
use common::*;
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;
use std::sync::Arc;

fn table_of(sys: &Sayc) -> Table {
    Table::uniform(
        sys.sys.clone(),
        Arc::new(bridge_system::NaturalInference::default()),
    )
}

// ================================================================================================
// Phase 3.10: the same `policy_argmax_matches_choose_bid` property (07-bidding.md §6.1, §8), but
// over positions from the real, compiled `systems/sayc/sayc.bml` reached by replaying `choose_bid`
// itself (`common::random_sayc_position`, shared with `tests/consistency.rs`), rather than the
// two hand-picked auctions against the small hand-built system above, under the
// `system_players()` preset; the task brief's own gate is 100% agreement, so this asserts every
// position, not a tolerance band over a sampled fraction.
// ================================================================================================

fn sayc_ctx(table: &bridge_bidding::Table) -> BidContext<'_> {
    BidContext {
        scoring: Scoring::Imp,
        natural: Some(table.natural.as_ref()),
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::system_players(),
    }
}

/// Runs the property until `n` positions have actually been *checked* (a `NoCandidate` draw is
/// skipped and does not count towards `n`, since there is nothing to compare there -- but it must
/// not silently shrink the reported sample size either, per the review finding that the old
/// "10^5 positions" test actually checked only ~61% of that). Draws are capped at
/// `n * MAX_DRAW_FACTOR` so a system with too few `Chosen` positions fails loudly instead of
/// looping forever.
fn run_sayc_policy_check(n: u64, seed: u64) -> u64 {
    const MAX_DRAW_FACTOR: u64 = 20;
    let table = common::compile_sayc("sayc.bml");
    let ctx = sayc_ctx(&table);
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
    let mut checked = 0u64;
    let mut drawn = 0u64;
    let max_draws = n * MAX_DRAW_FACTOR;

    while checked < n {
        drawn += 1;
        assert!(
            drawn <= max_draws,
            "only {checked}/{n} positions had a Chosen candidate after {drawn} draws; the system \
             may have too many NoCandidate gaps to reach the requested sample size"
        );
        let (deal, auction) =
            std::iter::repeat_with(|| common::random_sayc_position(&mut rng, &table, &ctx))
                .find(|(_, auction)| !auction.is_complete())
                .expect("random_sayc_position eventually yields an incomplete auction");
        let seat = auction.next_seat();
        let hand = deal.hand(seat);

        let choice = choose_bid(&table, hand, &auction, &ctx);
        let Some(expected) = choice.call() else {
            continue; // NoCandidate: nothing to compare, redraw without counting it.
        };

        let dist = call_distribution(&table, hand, &auction, &ctx);
        let p_max = dist
            .iter()
            .map(|(_, p)| *p)
            .fold(f32::NEG_INFINITY, f32::max);
        let p_chosen = dist
            .iter()
            .find(|(c, _)| *c == expected)
            .map(|(_, p)| *p)
            .unwrap_or_else(|| {
                panic!("choose_bid picked {expected:?}, not among call_distribution's legal_calls")
            });
        assert!(
            (p_chosen - p_max).abs() < 1e-3,
            "choose_bid picked {expected:?} with p={p_chosen}, but max p over the distribution \
             is {p_max} for hand {hand:?} at {auction}"
        );
        checked += 1;
    }
    checked
}

/// Non-`#[ignore]`d, debug-friendly version (task brief: 10^3 positions, 100% agreement).
#[test]
fn sayc_policy_argmax_matches_choose_bid_1e3() {
    let checked = run_sayc_policy_check(1_000, 0x5A1C_1001);
    assert_eq!(checked, 1_000);
}

/// The 10^5-position release version (task brief).
#[test]
#[ignore = "10^5 positions; run with `cargo test --release -- --ignored`"]
fn sayc_policy_argmax_matches_choose_bid_1e5() {
    let started = std::time::Instant::now();
    let checked = run_sayc_policy_check(100_000, 0x5A1C_1002);
    eprintln!(
        "sayc_policy_argmax_matches_choose_bid_1e5: {checked} position(s) checked in {:?}",
        started.elapsed()
    );
    assert_eq!(checked, 100_000);
}

const TRIALS_PER_POSITION: usize = 5_000;

fn ctx() -> BidContext<'static> {
    BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Never,
        policy: PolicyParams::system_players(),
    }
}

/// Runs the property at one auction prefix, over `TRIALS_PER_POSITION` random hands.
fn check_position(table: &Table, prefix: &Auction) {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x00C0_FFEE);
    let ctx = ctx();
    let mut checked = 0usize;

    for _ in 0..TRIALS_PER_POSITION {
        let hand = random_hand13(&mut rng);
        let choice = choose_bid(table, hand, prefix, &ctx);
        let Some(expected) = choice.call() else {
            continue; // NoCandidate: nothing to compare (no legal call has positive priority).
        };
        let dist = call_distribution(table, hand, prefix, &ctx);
        let p_max = dist
            .iter()
            .map(|(_, p)| *p)
            .fold(f32::NEG_INFINITY, f32::max);
        // The call `choose_bid` picks must attain the distribution's maximum probability (under
        // the phase-4 policy it is the unique maximum, since `call_distribution` uses the same
        // rank order, tie-break included).
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
    let table = table_of(&sys);
    let empty = Auction::new(Seat::North, Vulnerability::None);
    check_position(&table, &empty);
}

#[test]
fn policy_argmax_matches_choose_bid_response_to_1h() {
    let sys = sayc_system();
    let table = table_of(&sys);
    let after_1h = auction(Seat::North, Vulnerability::None, &[bid(1, Strain::Hearts)]);
    check_position(&table, &after_1h);
}

/// Regression: every node in `sayc_system()` has `priority: 0` (see `tests/common`), so every
/// kept candidate in `check_position`'s trials tied on priority, and `call_distribution` fell
/// back to summing equal scores — the test could not actually distinguish a priority-driven
/// softmax from a uniform one (flipping the sign of priority or τ would still have passed). This
/// system gives two candidates at the same position distinct priorities and checks that the
/// policy's argmax follows `choose_bid`'s own priority order, not merely membership in the
/// legal-call set.
#[test]
fn policy_argmax_respects_distinct_priorities() {
    let mut b = SystemBuilder::new();
    b.insert(
        true,
        &[bid(1, Strain::Clubs)],
        bid(1, Strain::Clubs),
        atom_hcp(0, 40),
        SeatCond::Any,
        VulCond::default(),
        "higher priority",
        10,
    );
    b.insert(
        true,
        &[bid(1, Strain::Diamonds)],
        bid(1, Strain::Diamonds),
        atom_hcp(0, 40),
        SeatCond::Any,
        VulCond::default(),
        "lower priority",
        1,
    );
    let sys = Arc::new(b.build());
    let table = Table::uniform(sys, Arc::new(bridge_system::NaturalInference::default()));
    let empty = Auction::new(Seat::North, Vulnerability::None);
    let ctx = BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Never,
        policy: PolicyParams::system_players(),
    };
    // `weak_hand` (0 HCP) satisfies both `atom_hcp(0, 40)` nodes, so both are legal, satisfied
    // candidates and priority alone must decide between them.
    let hand = weak_hand();

    let choice = choose_bid(&table, hand, &empty, &ctx);
    let BidChoice::Chosen(chosen) = choice else {
        panic!("both candidates are satisfied; expected Chosen")
    };
    assert_eq!(chosen.call, bid(1, Strain::Clubs));

    let dist = call_distribution(&table, hand, &empty, &ctx);
    let p = |call| dist.iter().find(|(c, _)| *c == call).unwrap().1;
    let p_1c = p(bid(1, Strain::Clubs));
    let p_1d = p(bid(1, Strain::Diamonds));
    assert!(
        p_1c > p_1d,
        "the higher-priority call must have higher probability: 1C={p_1c}, 1D={p_1d}"
    );
    let (best_call, _) =
        dist.iter()
            .cloned()
            .fold((chosen.call, f32::NEG_INFINITY), |acc, (c, prob)| {
                if prob > acc.1 { (c, prob) } else { acc }
            });
    assert_eq!(best_call, chosen.call);
}
