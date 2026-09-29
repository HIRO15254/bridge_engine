//! The phase-4 policy API (docs/design/15-phase4-plan.md D18/D19): the exact values of the new
//! `call_distribution` formula `p = (1 − ε)·[(1 − δ)·S + δ·M] + ε/n`, the legacy softmax path,
//! `AuctionPolicy` against the reference `sequence_log_likelihood`, `InterpretOptions` built from
//! a `BidContext`, and the single rank order shared by `choose_bid` and
//! `bridge_system::exclusive::rank_cmp_keys`.

mod common;

use std::sync::Arc;

use bridge_bidding::{
    AuctionPolicy, BidChoice, BidContext, ImplicitPass, InterpretMode, InterpretOptions,
    PolicyParams, Scoring, Table, call_distribution, choose_bid, sequence_log_likelihood,
};
use bridge_core::{Auction, Call, Seat, Strain, Vulnerability};
use bridge_system::ast::{SeatCond, VulCond};
use bridge_system::exclusive::{RankKey, rank_cmp_keys};
use common::*;
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;

const TOL: f32 = 1e-6;

/// Two openings at distinct priorities, both 0..40 HCP (so every hand satisfies both).
fn two_openings() -> Table {
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
        atom_hcp(12, 40),
        SeatCond::Any,
        VulCond::default(),
        "lower priority",
        1,
    );
    Table::uniform(
        Arc::new(b.build()),
        Arc::new(bridge_system::NaturalInference::default()),
    )
}

fn ctx(policy: PolicyParams, natural: Option<&bridge_system::NaturalInference>) -> BidContext<'_> {
    BidContext {
        scoring: Scoring::Imp,
        natural,
        implicit_pass: ImplicitPass::Never,
        policy,
    }
}

fn p_of(dist: &[(Call, f32)], call: Call) -> f32 {
    dist.iter()
        .find(|(c, _)| *c == call)
        .map(|(_, p)| *p)
        .unwrap()
}

#[test]
fn system_choice_gets_one_minus_epsilon_plus_the_floor() {
    let table = two_openings();
    let empty = Auction::new(Seat::North, Vulnerability::None);
    let policy = PolicyParams::system_players();
    let dist = call_distribution(&table, weak_hand(), &empty, &ctx(policy, None));
    let n = dist.len() as f32;
    let eps = policy.epsilon;
    let total: f32 = dist.iter().map(|(_, p)| p).sum();
    assert!((total - 1.0).abs() < 1e-4, "sum {total}");
    // The weak hand satisfies only 1C.
    assert!((p_of(&dist, bid(1, Strain::Clubs)) - ((1.0 - eps) + eps / n)).abs() < TOL);
    assert!((p_of(&dist, bid(1, Strain::Diamonds)) - eps / n).abs() < TOL);
    assert!((p_of(&dist, Call::Pass) - eps / n).abs() < TOL);
}

#[test]
fn a_hand_with_no_system_candidate_gets_the_uniform_distribution() {
    // Only 1D (12+ HCP) is defined; the weak hand satisfies nothing and ImplicitPass::Never
    // synthesises no pass, so S is uniform and so is p.
    let mut b = SystemBuilder::new();
    b.insert(
        true,
        &[bid(1, Strain::Diamonds)],
        bid(1, Strain::Diamonds),
        atom_hcp(12, 40),
        SeatCond::Any,
        VulCond::default(),
        "only",
        0,
    );
    let table = Table::uniform(
        Arc::new(b.build()),
        Arc::new(bridge_system::NaturalInference::default()),
    );
    let empty = Auction::new(Seat::North, Vulnerability::None);
    let dist = call_distribution(
        &table,
        weak_hand(),
        &empty,
        &ctx(PolicyParams::system_players(), None),
    );
    let n = dist.len() as f32;
    for (_, p) in &dist {
        assert!((p - 1.0 / n).abs() < TOL);
    }
}

#[test]
fn deviation_moves_delta_mass_to_the_natural_choice() {
    let table = two_openings();
    let empty = Auction::new(Seat::North, Vulnerability::None);
    let natural = table.natural.clone();
    let policy = PolicyParams {
        epsilon: 1e-3,
        deviation: 0.3,
        legacy_temperature: None,
    };
    let hand = weak_hand();
    let dist = call_distribution(&table, hand, &empty, &ctx(policy, Some(natural.as_ref())));
    let total: f32 = dist.iter().map(|(_, p)| p).sum();
    assert!((total - 1.0).abs() < 1e-4, "sum {total}");
    let n = dist.len() as f32;
    let (eps, delta) = (policy.epsilon, policy.deviation);
    // The system still picks 1C; the natural policy picks something (a 0-HCP hand passes
    // naturally), which is not 1C.
    let p_1c = p_of(&dist, bid(1, Strain::Clubs));
    assert!(
        (p_1c - ((1.0 - eps) * (1.0 - delta) + eps / n)).abs() < TOL,
        "{p_1c}"
    );
    let p_pass = p_of(&dist, Call::Pass);
    assert!(
        (p_pass - ((1.0 - eps) * delta + eps / n)).abs() < TOL,
        "{p_pass}"
    );
    // argmax is still choose_bid's call.
    let best = dist
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(c, _)| *c);
    assert_eq!(
        best,
        choose_bid(&table, hand, &empty, &ctx(policy, None)).call()
    );
}

#[test]
fn legacy_temperature_restores_the_priority_softmax() {
    let table = two_openings();
    let empty = Auction::new(Seat::North, Vulnerability::None);
    // A 13-HCP hand satisfies both openings.
    let strong = hand("AKQ2", "A32", "432", "432");
    let policy = PolicyParams::legacy(10.0);
    let dist = call_distribution(&table, strong, &empty, &ctx(policy, None));
    let n = dist.len() as f32;
    let eps = policy.epsilon;
    // softmax over priorities 10/τ and 1/τ with τ = 10: e^1 : e^0.1.
    let z = 1f32.exp() + 0.1f32.exp();
    let want_1c = (1.0 - eps) * 1f32.exp() / z + eps / n;
    let want_1d = (1.0 - eps) * 0.1f32.exp() / z + eps / n;
    assert!((p_of(&dist, bid(1, Strain::Clubs)) - want_1c).abs() < 1e-5);
    assert!((p_of(&dist, bid(1, Strain::Diamonds)) - want_1d).abs() < 1e-5);
}

#[test]
fn presets_are_distinct_and_default_is_system_players() {
    assert_eq!(PolicyParams::default(), PolicyParams::system_players());
    let h = PolicyParams::human();
    assert!(h.deviation > 0.0 && h.deviation < 0.5);
    assert!(h.epsilon > 0.0);
    assert_eq!(h.legacy_temperature, None);
    assert_eq!(PolicyParams::legacy(1.0).legacy_temperature, Some(1.0));
}

#[test]
fn interpret_options_follow_the_bid_context() {
    let natural = bridge_system::NaturalInference::default();
    let bid_ctx = BidContext {
        scoring: Scoring::Imp,
        natural: Some(&natural),
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::human(),
    };
    let opts = InterpretOptions::for_context(&bid_ctx);
    assert_eq!(opts.policy, PolicyParams::human());
    assert_eq!(opts.implicit_pass, ImplicitPass::Complement);
    assert_eq!(opts.mode, InterpretMode::Mirror);
    assert!(!opts.strict);
    assert_eq!(opts.max_alternatives, 8);
    let legacy = InterpretOptions::legacy();
    assert_eq!(legacy.mode, InterpretMode::Legacy);
    assert_eq!(legacy.eps_exact, 0.02);
    assert_eq!(InterpretOptions::default().mode, InterpretMode::Mirror);
}

#[test]
fn auction_policy_matches_the_reference_likelihood() {
    let table = compile_sayc("sayc.bml");
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0xA0C7_1001);
    for policy in [PolicyParams::system_players(), PolicyParams::human()] {
        let bid_ctx = BidContext {
            scoring: Scoring::Imp,
            natural: Some(table.natural.as_ref()),
            implicit_pass: ImplicitPass::Complement,
            policy,
        };
        for _ in 0..10 {
            let deal = random_deal(&mut rng);
            let auction =
                bridge_bidding::replay(&table, &deal, Seat::North, Vulnerability::None, &bid_ctx)
                    .auction;
            let fast = AuctionPolicy::new(&table, &auction, &bid_ctx);
            assert_eq!(fast.auction(), &auction);
            for _ in 0..5 {
                let other = random_deal(&mut rng);
                let want = sequence_log_likelihood(&table, &other, &auction, &bid_ctx);
                let got = fast.log_likelihood(&other);
                assert!((got - want).abs() <= 1e-5, "{got} vs {want} for {auction}");
            }
            // The true deal replays to the auction, so every call is the policy's choice.
            let own = fast.log_likelihood(&deal);
            assert!(own.is_finite());
        }
    }
}

/// `rank_order_shared` (07-bidding.md §8): `choose_bid`'s `alternatives` are sorted by the one
/// rank comparator.
#[test]
fn choose_bid_alternatives_follow_rank_cmp() {
    let table = compile_sayc("sayc.bml");
    let bid_ctx = BidContext {
        scoring: Scoring::Imp,
        natural: Some(table.natural.as_ref()),
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::system_players(),
    };
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0xA0C7_1002);
    let mut checked = 0;
    for _ in 0..400 {
        let (deal, auction) = random_sayc_position(&mut rng, &table, &bid_ctx);
        if auction.is_complete() {
            continue;
        }
        let seat = auction.next_seat();
        let BidChoice::Chosen(chosen) = choose_bid(&table, deal.hand(seat), &auction, &bid_ctx)
        else {
            continue;
        };
        let system = &table.systems[seat.index() as usize];
        for w in chosen.alternatives.windows(2) {
            let key = |a: &bridge_bidding::Alternative| RankKey {
                call: a.call,
                priority: a.priority,
                node: a.node,
            };
            assert_ne!(
                rank_cmp_keys(system, &key(&w[0]), &key(&w[1])),
                std::cmp::Ordering::Greater,
                "alternatives out of rank order at {auction}"
            );
        }
        checked += 1;
    }
    assert!(checked > 100, "only {checked} positions checked");
}
