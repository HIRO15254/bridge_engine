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

fn sayc_ctx(table: &bridge_bidding::Table, policy: PolicyParams) -> BidContext<'_> {
    BidContext {
        scoring: Scoring::Imp,
        natural: Some(table.natural.as_ref()),
        implicit_pass: ImplicitPass::Complement,
        policy,
    }
}

/// Runs the property until `n` positions have actually been *checked* (a `NoCandidate` draw is
/// skipped and does not count towards `n`, since there is nothing to compare there -- but it must
/// not silently shrink the reported sample size either, per the review finding that the old
/// "10^5 positions" test actually checked only ~61% of that). Draws are capped at
/// `n * MAX_DRAW_FACTOR` so a system with too few `Chosen` positions fails loudly instead of
/// looping forever.
fn run_sayc_policy_check(n: u64, seed: u64, policy: PolicyParams) -> u64 {
    const MAX_DRAW_FACTOR: u64 = 20;
    let table = common::compile_sayc("sayc.bml");
    let ctx = sayc_ctx(&table, policy);
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

/// Non-`#[ignore]`d, debug-friendly version (task brief: 10^3 positions, 100% agreement), under
/// both presets (`δ < 1/2` keeps the argmax structural).
#[test]
fn sayc_policy_argmax_matches_choose_bid_1e3() {
    for policy in [PolicyParams::system_players(), PolicyParams::human()] {
        let checked = run_sayc_policy_check(1_000, 0x5A1C_1001, policy);
        assert_eq!(checked, 1_000);
    }
}

/// The 10^5-position release version (task brief), under both presets.
#[test]
#[ignore = "10^5 positions; run with `cargo test --release -- --ignored`"]
fn sayc_policy_argmax_matches_choose_bid_1e5() {
    for (name, policy) in [
        ("system_players", PolicyParams::system_players()),
        ("human", PolicyParams::human()),
    ] {
        let started = std::time::Instant::now();
        let checked = run_sayc_policy_check(100_000, 0x5A1C_1002, policy);
        eprintln!(
            "sayc_policy_argmax_matches_choose_bid_1e5 ({name}): {checked} position(s) checked in \
             {:?}",
            started.elapsed()
        );
        assert_eq!(checked, 100_000);
    }
}

// ================================================================================================
// `fast_likelihood_matches_reference` (07-bidding.md §6.2, §8): `AuctionPolicy::log_likelihood`
// equals the reference `sequence_log_likelihood` to 1e-5, on generated auctions (both presets)
// and corpus auctions (the human preset, natural-heavy).
// ================================================================================================

/// `(auctions checked, deals checked, max |Δ ln L|)` over `n_auctions` generated and
/// `n_auctions` corpus auctions, `deals` deals each (random deals plus the true deal).
fn run_fast_likelihood(n_auctions: usize, deals: usize, seed: u64) -> (usize, usize, f64) {
    use bridge_bidding::{AuctionPolicy, replay, sequence_log_likelihood};
    use rand_xoshiro::rand_core::Rng;

    let table = common::compile_sayc("sayc.bml");
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
    let corpus = common::corpus_auctions_with_deals(n_auctions);
    let (mut n_a, mut n_d, mut max_diff) = (0usize, 0usize, 0.0f64);
    for policy in [PolicyParams::system_players(), PolicyParams::human()] {
        let ctx = sayc_ctx(&table, policy);
        let mut auctions: Vec<(Auction, Option<bridge_core::Deal>)> = (0..n_auctions)
            .map(|i| {
                let deal = random_deal(&mut rng);
                let vul = Vulnerability::from_index((rng.next_u32() % 4) as u8);
                let a = replay(&table, &deal, Seat::ALL[i % 4], vul, &ctx).auction;
                (a, Some(deal))
            })
            .collect();
        if policy == PolicyParams::human() {
            auctions.extend(corpus.iter().cloned());
        }
        for (auction, deal) in &auctions {
            let fast = AuctionPolicy::new(&table, auction, &ctx);
            let mut ds: Vec<bridge_core::Deal> =
                (0..deals).map(|_| random_deal(&mut rng)).collect();
            if let Some(d) = deal {
                ds.push(*d);
            }
            for d in &ds {
                let want = sequence_log_likelihood(&table, d, auction, &ctx);
                let got = fast.log_likelihood(d);
                let diff = (got - want).abs();
                assert!(
                    diff <= 1e-5,
                    "|Δ ln L| = {diff} ({got} vs {want}) for {auction} under {policy:?}"
                );
                max_diff = max_diff.max(diff);
                n_d += 1;
            }
            n_a += 1;
        }
    }
    (n_a, n_d, max_diff)
}

/// Default suite: 50 generated auctions per preset plus 50 corpus auctions, 20 deals each.
#[test]
fn fast_likelihood_matches_reference() {
    let (a, d, max) = run_fast_likelihood(50, 20, 0xFA57_0001);
    eprintln!("fast_likelihood_matches_reference: {a} auctions, {d} deals, max |Δ ln L| {max:e}");
}

/// The full run: 50 auctions × 1000 deals per source and preset.
#[test]
#[ignore = "50 auctions x 1000 deals per source; run with `cargo test --release -- --ignored`"]
fn fast_likelihood_matches_reference_large() {
    let started = std::time::Instant::now();
    let (a, d, max) = run_fast_likelihood(50, 1000, 0xFA57_0002);
    eprintln!(
        "fast_likelihood_matches_reference_large: {a} auctions, {d} deals, max |Δ ln L| {max:e} \
         in {:?}",
        started.elapsed()
    );
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
/// kept candidate in `check_position`'s trials tied on priority, and the test could not
/// actually distinguish a priority-driven choice from an arbitrary one (it was written when the
/// policy was still the phase-3 priority softmax, where flipping the sign of priority or τ would
/// still have passed). This system gives two candidates at the same position distinct
/// priorities and checks that the policy's argmax follows `choose_bid`'s own priority order,
/// not merely membership in the legal-call set.
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
