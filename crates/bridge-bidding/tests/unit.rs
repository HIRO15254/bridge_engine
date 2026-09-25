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
/// via `resolve_lenient`, with `eps_partial` mixed in (non-strict).
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
    let opts = InterpretOptions::default();
    let interp = interpret(&table, &a, &opts);

    let pc = &interp.per_call[2];
    assert_eq!(pc.seat, Seat::South);
    assert_eq!(pc.kind, ResolutionKind::Partial { matched_depth: 1 });
    // Call 0 (1H) and call 1 (1NT) both resolve Exact, each in its own seat's system; call 2 (2H)
    // is the first call anywhere in the auction that does not.
    assert_eq!(interp.divergence, Some(2));
    // eps_partial mixed in: the real alternative plus the ANY fallback.
    assert_eq!(pc.alternatives.len(), 2);
    let sum: f32 = pc.alternatives.iter().map(|(_, w, _)| *w).sum();
    assert!((sum - 1.0).abs() < TOL);
    let real_weight = pc
        .alternatives
        .iter()
        .find(|(_, _, ex)| ex.kind != ResolutionKind::Fallback)
        .unwrap()
        .1;
    assert!((real_weight - (1.0 - opts.eps_partial)).abs() < TOL);

    // `strict: true` removes the `Fallback` branch entirely (07-bidding.md §4.2), rather than
    // just shrinking its weight.
    let strict_opts = InterpretOptions {
        strict: true,
        ..InterpretOptions::default()
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
