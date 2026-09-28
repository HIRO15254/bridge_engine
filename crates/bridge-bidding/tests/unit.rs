//! Unit-style tests (07-bidding.md §8) against the hand-built system in `tests/common`.

mod common;

use bridge_bidding::{
    BidChoice, BidContext, ChoiceSource, ImplicitPass, InterpretOptions, PolicyParams,
    ResolutionKind, Scoring, Table, choose_bid, interpret,
};
use bridge_core::{Seat, Strain, Suit, Vulnerability};
use bridge_system::ast::{SeatCond, VulCond};
use common::*;
use rand_xoshiro::rand_core::SeedableRng;
use std::sync::Arc;

fn table_of(sys: &Sayc) -> Table {
    Table::uniform(
        sys.sys.clone(),
        Arc::new(bridge_system::NaturalInference::default()),
    )
}

fn default_ctx() -> BidContext<'static> {
    BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Never,
        policy: PolicyParams::default(),
    }
}

const TOL: f32 = 1e-4;

/// Every seat's `seats[s]` weights, and every call's `per_call[j]` weights, sum to 1.
#[test]
fn weights_sum_to_one() {
    let sys = sayc_system();
    let table = table_of(&sys);
    let a = auction(Seat::North, Vulnerability::None, &[bid(1, Strain::Spades)]);
    let interp = interpret(&table, &a, &InterpretOptions::default());

    assert_eq!(interp.per_call.len(), 1);
    let pc = &interp.per_call[0];
    // 1S has two `Or` branches (weighted 0.6/0.4) plus the epsilon fallback branch.
    assert_eq!(pc.alternatives.len(), 3);
    let sum: f32 = pc.alternatives.iter().map(|(_, w, _)| *w).sum();
    assert!((sum - 1.0).abs() < TOL, "per_call weights sum to {sum}");

    for seat in Seat::ALL {
        let seat_alts = &interp.seats[seat.index() as usize];
        let sum: f32 = seat_alts.iter().map(|(_, w, _)| *w).sum();
        assert!(
            (sum - 1.0).abs() < TOL,
            "seats[{seat:?}] weights sum to {sum}"
        );
    }
}

/// Two calls by the same seat whose constraints contradict (1C's 12-14 HCP vs. its own 3NT
/// rebid's 25-27 HCP) drop out of the cross product; the seat falls back to `ANY` with a warning
/// instead of an empty disjunction.
#[test]
fn and_combination_drops_contradictions() {
    let sys = sayc_system();
    let table = table_of(&sys);
    let a = auction(
        Seat::North,
        Vulnerability::None,
        &[
            bid(1, Strain::Clubs),
            PASS,
            bid(1, Strain::Diamonds),
            PASS,
            bid(3, Strain::NoTrump),
        ],
    );
    let opts = InterpretOptions {
        strict: true,
        ..InterpretOptions::default()
    };
    let interp = interpret(&table, &a, &opts);

    // Each call resolves cleanly on its own.
    assert_eq!(interp.per_call[0].kind, ResolutionKind::Exact);
    assert_eq!(interp.per_call[4].kind, ResolutionKind::Exact);

    let north = &interp.seats[Seat::North.index() as usize];
    assert_eq!(
        north.len(),
        1,
        "the contradiction collapses to a single ANY entry"
    );
    let (constraint, weight, explanation) = &north[0];
    assert!((*weight - 1.0).abs() < TOL);
    assert!(explanation.parts.is_empty());
    // `ANY`: every hand satisfies it.
    assert!(constraint.satisfies(weak_hand()));
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(1);
    for _ in 0..20 {
        assert!(constraint.satisfies(random_hand13(&mut rng)));
    }
}

/// A seat that made no calls interprets as `[(ANY, 1.0, empty)]`.
#[test]
fn seat_without_calls_is_any() {
    let sys = sayc_system();
    let table = table_of(&sys);
    let a = auction(Seat::North, Vulnerability::None, &[bid(1, Strain::Hearts)]);
    let interp = interpret(&table, &a, &InterpretOptions::default());

    for seat in [Seat::East, Seat::South, Seat::West] {
        let alts = &interp.seats[seat.index() as usize];
        assert_eq!(alts.len(), 1);
        let (constraint, weight, explanation) = &alts[0];
        assert!((*weight - 1.0).abs() < TOL);
        assert!(explanation.parts.is_empty());
        assert!(explanation.text.is_empty());
        let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(seat.index() as u64 + 1);
        for _ in 0..20 {
            assert!(constraint.satisfies(random_hand13(&mut rng)));
        }
    }
}

/// A system that lists an illegal continuation (a bid below the last one) is reported as a lint,
/// never a panic.
#[test]
fn illegal_call_is_lint() {
    let mut b = SystemBuilder::new();
    let opening = b.insert(
        true,
        &[bid(1, Strain::Hearts)],
        bid(1, Strain::Hearts),
        atom_hcp(12, 21),
        SeatCond::Any,
        VulCond::default(),
        "opening",
        0,
    );
    // A buggy row: "1H - (P) - 1D" (1D is lower than 1H and can never legally be bid here).
    let bogus = b.insert(
        true,
        &[bid(1, Strain::Hearts), PASS, bid(1, Strain::Diamonds)],
        bid(1, Strain::Diamonds),
        atom_hcp(6, 10),
        SeatCond::Any,
        VulCond::default(),
        "illegal by construction",
        0,
    );
    let sys = Arc::new(b.build());
    let table = Table::uniform(sys, Arc::new(bridge_system::NaturalInference::default()));

    // Next to call is South (partner): North opens, East passes.
    let a = auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::Hearts), PASS],
    );
    let ctx = default_ctx();
    let hand = weak_hand();
    let choice = choose_bid(&table, hand, &a, &ctx);

    match choice {
        BidChoice::NoCandidate(nc) => {
            assert!(
                nc.tried
                    .iter()
                    .any(|t| t.node == bogus && t.call == bid(1, Strain::Diamonds))
            );
            assert!(nc.diagnostics.iter().any(|d| matches!(
                d,
                bridge_bidding::Diagnostic::IllegalSystemCall { node, call }
                    if *node == bogus && *call == bid(1, Strain::Diamonds)
            )));
        }
        BidChoice::Chosen(_) => panic!("the only candidate is illegal; expected NoCandidate"),
    }
    let _ = opening;
}

/// A truncated system: past the point the trie covers exactly, resolution degrades to `Partial`
/// via `resolve_lenient`, with `eps_partial` mixed in (non-strict). Legacy mode
/// ([`InterpretOptions::legacy`]); `partial_position_mirror_pieces` is the mirror's version.
#[test]
fn partial_and_natural_epsilon() {
    let sys = sayc_system();
    let table = table_of(&sys);
    // 1H (N) - 1NT (E, an overcall not in NS's own sequence) - 2H (S, a raise as if E had
    // passed): resolves via `resolve_lenient` substituting E's 1NT with Pass.
    let a = auction(
        Seat::North,
        Vulnerability::None,
        &[
            bid(1, Strain::Hearts),
            bid(1, Strain::NoTrump),
            bid(2, Strain::Hearts),
        ],
    );
    let opts = InterpretOptions::legacy();
    let interp = interpret(&table, &a, &opts);

    let pc = &interp.per_call[2];
    assert_eq!(pc.seat, Seat::South);
    assert_eq!(pc.kind, ResolutionKind::Partial { matched_depth: 1 });
    // Call 0 (1H) and call 1 (1NT) both resolve Exact, each in its own seat's system; call 2 (2H)
    // is the first call anywhere in the auction that does not.
    assert_eq!(interp.divergence, Some(2));
    // eps_partial mixed in: the real alternative plus the ANY fallback, which also receives the
    // `lenient_decay` (rho) mass of the single substitution (07-bidding.md section 4.1 step 5.2).
    assert_eq!(pc.alternatives.len(), 2);
    let sum: f32 = pc.alternatives.iter().map(|(_, w, _)| *w).sum();
    assert!((sum - 1.0).abs() < TOL);
    let real_weight = pc
        .alternatives
        .iter()
        .find(|(_, _, ex)| ex.kind != ResolutionKind::Fallback)
        .unwrap()
        .1;
    assert!((real_weight - (1.0 - opts.eps_partial) * opts.lenient_decay).abs() < TOL);

    // `strict: true` removes the `Fallback` branch entirely (07-bidding.md §4.2), rather than
    // just shrinking its weight.
    let strict_opts = InterpretOptions {
        strict: true,
        ..InterpretOptions::legacy()
    };
    let strict_interp = interpret(&table, &a, &strict_opts);
    let strict_pc = &strict_interp.per_call[2];
    assert_eq!(strict_pc.alternatives.len(), 1);
    assert!(
        strict_pc
            .alternatives
            .iter()
            .all(|(_, _, ex)| ex.kind != ResolutionKind::Fallback)
    );
    let (_, strict_weight, _) = &strict_pc.alternatives[0];
    assert!((*strict_weight - 1.0).abs() < TOL);
}

/// The mirror at a lenient (`Partial`) position: `X` pieces of the first full lenient match
/// with the system weight `(1 − ε)`, the no-candidate complement at `(1 − ε)/n`, and `ANY` at
/// `ε/n`; `log_scale` is `ln Σ raw`, and `strict` keeps only the `X` pieces.
#[test]
fn partial_position_mirror_pieces() {
    let sys = sayc_system();
    let table = table_of(&sys);
    let a = auction(
        Seat::North,
        Vulnerability::None,
        &[
            bid(1, Strain::Hearts),
            bid(1, Strain::NoTrump),
            bid(2, Strain::Hearts),
        ],
    );
    let opts = InterpretOptions::default();
    let interp = interpret(&table, &a, &opts);
    let pc = &interp.per_call[2];
    assert_eq!(pc.kind, ResolutionKind::Partial { matched_depth: 1 });
    assert_eq!(interp.divergence, Some(2));
    assert!(!pc.shadowed);
    let sum: f32 = pc.alternatives.iter().map(|(_, w, _)| *w).sum();
    assert!((sum - 1.0).abs() < TOL);
    let eps = f64::from(opts.policy.epsilon);
    let prefix = auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::Hearts), bid(1, Strain::NoTrump)],
    );
    let n = prefix.legal_calls().count() as f64;
    let scale = pc.log_scale.exp();
    let raw = |kind_ok: &dyn Fn(ResolutionKind) -> bool| -> f64 {
        pc.alternatives
            .iter()
            .filter(|(_, _, ex)| kind_ok(ex.kind))
            .map(|(_, w, _)| f64::from(*w) * scale)
            .sum()
    };
    let system = raw(&|k| matches!(k, ResolutionKind::Partial { .. }));
    assert!((system - (1.0 - eps)).abs() < 1e-5, "system raw {system}");
    let fallback = raw(&|k| k == ResolutionKind::Fallback);
    // The complement of the lenient siblings at (1 − ε)/n plus ANY at ε/n.
    assert!(
        (fallback - ((1.0 - eps) / n + eps / n)).abs() < 1e-5,
        "fallback raw {fallback}"
    );

    let strict_opts = InterpretOptions {
        strict: true,
        ..InterpretOptions::default()
    };
    let strict_pc = &interpret(&table, &a, &strict_opts).per_call[2];
    assert!(!strict_pc.alternatives.is_empty());
    assert!(
        strict_pc
            .alternatives
            .iter()
            .all(|(_, _, ex)| ex.kind != ResolutionKind::Fallback)
    );
    let strict_sum: f32 = strict_pc.alternatives.iter().map(|(_, w, _)| *w).sum();
    assert!((strict_sum - 1.0).abs() < TOL);
}

/// Regression: our own call can land exactly on an implicit-pass trie node that exists only
/// because some *deeper* row's path runs through it (06-system.md §4.3), not because our own call
/// has a listed row of its own — e.g. a competitive continuation like `1H-(P)-P-(1S)-X`. Before
/// the fix, Step A's `d == n_k` branch used `lookup.end` (which the walk had already advanced
/// *past* our own pass) to look for siblings, found none (its children are the calls after our
/// pass, not our pass's siblings), fell through to `NaturalInference` (still `todo!()` on this
/// branch, so this would panic), and marked `divergence` even though the auction is fully
/// on-system. The fix re-resolves the one-call-shorter key to find the *parent* position instead.
#[test]
fn implicit_pass_through_deeper_row() {
    let mut b = SystemBuilder::new();
    b.insert(
        true,
        &[bid(1, Strain::Hearts)],
        bid(1, Strain::Hearts),
        atom_hcp(12, 21),
        SeatCond::Any,
        VulCond::default(),
        "opening",
        0,
    );
    b.insert(
        true,
        &[bid(1, Strain::Hearts), PASS, bid(1, Strain::Spades)],
        bid(1, Strain::Spades),
        atom_suit_hcp(Suit::Spades, 4, 13, 6, 10),
        SeatCond::Any,
        VulCond::default(),
        "4+ spades, new suit",
        0,
    );
    b.insert(
        true,
        &[bid(1, Strain::Hearts), PASS, bid(2, Strain::Hearts)],
        bid(2, Strain::Hearts),
        atom_suit_hcp(Suit::Hearts, 3, 13, 6, 9),
        SeatCond::Any,
        VulCond::default(),
        "raise",
        0,
    );
    // The deeper competitive row 1H-(P)-P-(1S)-X: its path structurally creates South's own pass
    // at `[1H, Pass]` as a trie node with no row of its own (exactly the shape this test exists
    // to cover).
    b.insert(
        true,
        &[
            bid(1, Strain::Hearts),
            PASS,
            PASS,
            bid(1, Strain::Spades),
            DBL,
        ],
        DBL,
        atom_suit_hcp(Suit::Spades, 4, 13, 6, 21),
        SeatCond::Any,
        VulCond::default(),
        "negative double",
        0,
    );
    // East's own pass needs to resolve too (`Table::uniform` shares this system across all seats):
    // a real, direct row (not an implicit one), so this test's `divergence` assertions are about
    // South's resolution only, not entangled with East's.
    b.insert(
        false,
        &[bid(1, Strain::Hearts), PASS],
        PASS,
        bridge_constraint::HandConstraint::ANY,
        SeatCond::Any,
        VulCond::default(),
        "pass, nothing to say",
        0,
    );
    let sys = Arc::new(b.build());
    let table = Table::uniform(sys, Arc::new(bridge_system::NaturalInference::default()));
    let opts = InterpretOptions {
        strict: true,
        ..InterpretOptions::default()
    };
    let weak = weak_hand();

    // interpret([1H, Pass(E), Pass(S)]): South's own pass must resolve `Exact` (the complement of
    // 1S/2H), never fall through to `Natural`, and never mark `divergence`.
    let three_calls = auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::Hearts), PASS, PASS],
    );
    let interp = interpret(&table, &three_calls, &opts);
    assert!(interp.divergence.is_none());
    assert_eq!(interp.per_call[0].kind, ResolutionKind::Exact);
    assert_eq!(interp.per_call[1].kind, ResolutionKind::Exact);
    let south_pc = &interp.per_call[2];
    assert_eq!(south_pc.seat, Seat::South);
    assert_eq!(south_pc.kind, ResolutionKind::Exact);
    assert!(
        south_pc
            .alternatives
            .iter()
            .any(|(c, w, _)| *w > 0.0 && c.satisfies(weak))
    );

    // Bidirectional: `choose_bid` for South with a hand that fits neither 1S nor 2H must choose
    // the same implicit `Pass` that `interpret` just accepted.
    let two_calls = auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::Hearts), PASS],
    );
    let ctx = BidContext {
        implicit_pass: ImplicitPass::Complement,
        ..default_ctx()
    };
    let choice = choose_bid(&table, weak, &two_calls, &ctx);
    let BidChoice::Chosen(chosen) = choice else {
        panic!("weak_hand should satisfy the implicit-pass complement")
    };
    assert_eq!(chosen.call, bridge_core::Call::Pass);
    assert_eq!(chosen.source, ChoiceSource::ImplicitPass);
}

/// `classify`/`infer` (natural inference, `docs/design/07-bidding.md` §4.1 step 6) are now
/// implemented, so a scenario that falls all the way through to `ResolutionKind::Natural` can be
/// asserted directly (this test used to be `#[ignore]`d while that lane was still `todo!()`).
#[test]
fn partial_and_natural_epsilon_natural_gated() {
    let mut b = SystemBuilder::new();
    b.insert(
        true,
        &[bid(1, Strain::Hearts)],
        bid(1, Strain::Hearts),
        atom_hcp(12, 21),
        SeatCond::Any,
        VulCond::default(),
        "opening",
        0,
    );
    let sys = Arc::new(b.build());
    let table = Table::uniform(sys, Arc::new(bridge_system::NaturalInference::default()));
    // Nothing at all follows the opening, so responder's call falls through Partial (no lenient
    // match either) all the way to natural inference.
    let a = auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::Hearts), bid(1, Strain::Spades)],
    );
    let interp = interpret(&table, &a, &InterpretOptions::default());
    assert_eq!(interp.per_call[1].kind, ResolutionKind::Natural);
}

/// `ImplicitPass::Complement`: a hand that satisfies neither listed reaction to the opponents'
/// double is chosen to `Pass`, and the interpreter's implicit-pass complement is satisfied by
/// that same hand (07-bidding.md §4.1.5.1 / §5.2 step 3, bidirectional by construction since both
/// use `interpret::complement_of` against the same siblings).
#[test]
fn implicit_pass_bidirectional() {
    let sys = sayc_system();
    let table = table_of(&sys);
    let hand = weak_hand();

    let before = auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::Hearts), bridge_core::Call::Double],
    );
    let ctx = BidContext {
        implicit_pass: ImplicitPass::Complement,
        ..default_ctx()
    };
    let choice = choose_bid(&table, hand, &before, &ctx);
    let chosen = match choice {
        BidChoice::Chosen(c) => c,
        BidChoice::NoCandidate(_) => {
            panic!("weak_hand should satisfy the implicit-pass complement")
        }
    };
    assert_eq!(chosen.call, bridge_core::Call::Pass);
    assert_eq!(chosen.source, ChoiceSource::ImplicitPass);

    let after = before.with(chosen.call).unwrap();
    let interp = interpret(&table, &after, &InterpretOptions::default());
    let pc = &interp.per_call[2];
    assert_eq!(pc.seat, Seat::South);
    assert!(
        pc.alternatives
            .iter()
            .any(|(c, w, ex)| ex.kind != ResolutionKind::Fallback && *w > 0.0 && c.satisfies(hand))
    );
}

/// `rank_order_shared` (07-bidding.md §8): over 10^4 SAYC positions, `choose_bid`'s
/// `alternatives` follow the one rank comparator (`rank_cmp_keys`), and their system members
/// appear in the order of the exclusive index's sibling group (the order `interpret`'s mirror
/// and `call_distribution` use), whenever the position resolves exactly.
#[test]
fn rank_order_shared() {
    use bridge_system::exclusive::{RankKey, rank_cmp_keys};
    use bridge_system::{LookupKey, RelVul};

    let table = compile_sayc("sayc.bml");
    let ctx = BidContext {
        scoring: Scoring::Imp,
        natural: Some(table.natural.as_ref()),
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::system_players(),
    };
    let n: u64 = std::env::var("RANK_ORDER_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(0x5A1C_7001);
    let (mut checked, mut grouped) = (0u64, 0u64);
    while checked < n {
        let (deal, auction) = random_sayc_position(&mut rng, &table, &ctx);
        if auction.is_complete() {
            continue;
        }
        let seat = auction.next_seat();
        let BidChoice::Chosen(chosen) = choose_bid(&table, deal.hand(seat), &auction, &ctx) else {
            continue;
        };
        checked += 1;
        let system = &table.systems[seat.index() as usize];
        let key = |a: &bridge_bidding::Alternative| RankKey {
            call: a.call,
            priority: a.priority,
            node: a.node,
        };
        for w in chosen.alternatives.windows(2) {
            assert_ne!(
                rank_cmp_keys(system, &key(&w[0]), &key(&w[1])),
                std::cmp::Ordering::Greater,
                "alternatives out of rank order at {auction}"
            );
        }
        let vulnerability = auction.vulnerability();
        let vul = RelVul {
            we: vulnerability.is_vulnerable(seat),
            they: vulnerability.is_vulnerable(seat.next()),
        };
        let lk = match LookupKey::for_auction(&auction, seat) {
            Some(k) => k,
            None => LookupKey {
                we_opened: true,
                calls: &[],
                opener_pos: auction.position_of(seat),
                vul,
            },
        };
        let lookup = system.index.resolve(&lk);
        if lookup.matched_depth != lk.calls.len() {
            continue;
        }
        let Some(group) = system
            .exclusive()
            .group_for(lookup.end, lk.opener_pos, lk.vul)
        else {
            continue;
        };
        let ranks: Vec<usize> = chosen
            .alternatives
            .iter()
            .filter_map(|a| a.node.map(|n| (a.call, n)))
            .map(|m| {
                group
                    .members
                    .iter()
                    .position(|&x| x == m)
                    .unwrap_or_else(|| panic!("{m:?} is not a group member at {auction}"))
            })
            .collect();
        assert!(
            ranks.windows(2).all(|w| w[0] < w[1]),
            "choose_bid order {ranks:?} differs from the index group order at {auction}"
        );
        grouped += 1;
    }
    eprintln!("rank_order_shared: {checked} positions, {grouped} compared with the index group");
    assert!(grouped > n / 4, "only {grouped} positions resolved exactly");
}

/// The run-time recompute of `X_c` (15-phase4-plan D19): at a lenient position where a
/// higher-ranked sibling is illegal after the actual prefix, the call's exclusive region comes
/// from the legal siblings only, not from the index (where the illegal sibling shadows it), and
/// the mirror still equals `call_distribution`.
///
/// System: `1C` opening; responses `1C-P-1D` (any hand, priority 10) and `1C-P-1H` (4+ hearts,
/// priority 5). In `1C-(1D)-1H` the lenient match substitutes East's `1D` by `Pass`, so the
/// position is `1C-P` with `1D` illegal. The index reads `1H` as shadowed by `1D`; the run-time
/// region is `1H`'s own constraint.
#[test]
fn recomputed_region_when_a_higher_sibling_is_illegal() {
    use bridge_bidding::call_distribution;
    use rand_xoshiro::Xoshiro256PlusPlus;

    let mut b = SystemBuilder::new();
    b.insert(
        true,
        &[bid(1, Strain::Clubs)],
        bid(1, Strain::Clubs),
        atom_hcp(12, 21),
        SeatCond::Any,
        VulCond::default(),
        "opening",
        0,
    );
    b.insert(
        true,
        &[bid(1, Strain::Clubs), PASS, bid(1, Strain::Diamonds)],
        bid(1, Strain::Diamonds),
        atom_hcp(0, 37),
        SeatCond::Any,
        VulCond::default(),
        "any response",
        10,
    );
    b.insert(
        true,
        &[bid(1, Strain::Clubs), PASS, bid(1, Strain::Hearts)],
        bid(1, Strain::Hearts),
        atom_suit_hcp(Suit::Hearts, 4, 13, 0, 37),
        SeatCond::Any,
        VulCond::default(),
        "four hearts",
        5,
    );
    let table = Table::uniform(
        Arc::new(b.build()),
        Arc::new(bridge_system::NaturalInference::default()),
    );
    let ctx = BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::human(),
    };
    let prefix = auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::Clubs), bid(1, Strain::Diamonds)],
    );
    let a = prefix.with(bid(1, Strain::Hearts)).expect("legal");
    let interp = interpret(&table, &a, &InterpretOptions::for_context(&ctx));
    let pc = &interp.per_call[2];
    assert_eq!(pc.kind, ResolutionKind::Partial { matched_depth: 1 });
    assert!(!pc.shadowed, "1H is shadowed only by the illegal 1D");
    let system_pieces: Vec<_> = pc
        .alternatives
        .iter()
        .filter(|(_, _, ex)| matches!(ex.kind, ResolutionKind::Partial { .. }))
        .collect();
    assert_eq!(system_pieces.len(), 1, "{:?}", pc.alternatives);

    // The mirror equals the policy on every hand (with and without four hearts).
    let scale = pc.log_scale.exp();
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x4EC0);
    let (mut with_hearts, mut without) = (0, 0);
    for _ in 0..400 {
        let hand = random_hand13(&mut rng);
        if hand.holding(Suit::Hearts).len() >= 4 {
            with_hearts += 1;
        } else {
            without += 1;
        }
        let p = f64::from(
            call_distribution(&table, hand, &prefix, &ctx)
                .iter()
                .find(|(c, _)| *c == bid(1, Strain::Hearts))
                .map(|(_, p)| *p)
                .expect("1H is legal"),
        );
        let m: f64 = scale
            * pc.alternatives
                .iter()
                .filter(|(c, _, _)| c.satisfies(hand))
                .map(|(_, w, _)| f64::from(*w))
                .sum::<f64>();
        assert!(
            (m - p).abs() <= 1e-4 * p,
            "hand {hand:?}: mirror {m:e}, policy {p:e}"
        );
    }
    assert!(with_hearts > 0 && without > 0);
}
