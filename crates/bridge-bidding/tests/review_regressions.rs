//! Regression tests for the phase-3 integration review of `interpret`/`choose_bid`
//! (07-bidding.md §4.1, §5.2), each against a small hand-built system.

mod common;

use std::sync::Arc;

use bridge_bidding::{
    BidChoice, BidContext, ChoiceSource, ImplicitPass, InterpretOptions, PolicyParams,
    ResolutionKind, Scoring, Table, choose_bid, interpret,
};
use bridge_core::{Call, Seat, Strain, Suit, Vulnerability};
use bridge_system::ast::{SeatCond, VulCond};
use bridge_system::{Forcing, NaturalInference};
use common::*;

fn table(sys: SystemBuilder) -> Table {
    Table::uniform(Arc::new(sys.build()), Arc::new(NaturalInference::default()))
}

fn strict() -> InterpretOptions {
    InterpretOptions {
        strict: true,
        ..InterpretOptions::default()
    }
}

fn natural_ctx(natural: &NaturalInference) -> BidContext<'_> {
    BidContext {
        scoring: Scoring::Imp,
        natural: Some(natural),
        implicit_pass: ImplicitPass::Never,
        policy: PolicyParams::default(),
    }
}

fn opening_1h(b: &mut SystemBuilder) {
    b.insert(
        true,
        &[bid(1, Strain::Hearts)],
        bid(1, Strain::Hearts),
        atom_suit_hcp(Suit::Hearts, 5, 13, 12, 21),
        SeatCond::Any,
        VulCond::default(),
        "1H opening",
        0,
    );
}

/// Review: "Implicit-pass fallthrough builds our Pass's complement from the opponents' next
/// calls". Only a deeper row `1H-(X)-P-(2S)-X` runs through South's pass after `1H-(X)`, so that
/// pass lands on an implicit-pass trie node whose parent offers no sibling. Step A used to fall
/// into §4.1 step 5.1 at `lookup.end` (the position *after* our pass) and complement West's 2S
/// node; it must resolve naturally instead, and `choose_bid` (which used to return
/// `NoCandidate` there, dropped review finding 7) must offer the same natural pass.
#[test]
fn implicit_pass_node_without_siblings_is_natural_on_both_sides() {
    let mut b = SystemBuilder::new();
    opening_1h(&mut b);
    // West's 2S, as a history row of the competitive table (a node on an opponents' depth).
    b.insert(
        true,
        &[bid(1, Strain::Hearts), DBL, PASS, bid(2, Strain::Spades)],
        bid(2, Strain::Spades),
        atom_suit_hcp(Suit::Spades, 5, 13, 0, 37),
        SeatCond::Any,
        VulCond::default(),
        "5+ spades",
        0,
    );
    b.insert(
        true,
        &[
            bid(1, Strain::Hearts),
            DBL,
            PASS,
            bid(2, Strain::Spades),
            DBL,
        ],
        DBL,
        atom_hcp(10, 37),
        SeatCond::Any,
        VulCond::default(),
        "penalty",
        0,
    );
    let t = table(b);

    let with_pass = auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::Hearts), DBL, PASS],
    );
    let interp = interpret(&t, &with_pass, &strict());
    let south = &interp.per_call[2];
    assert_eq!(south.seat, Seat::South);
    assert_eq!(south.kind, ResolutionKind::Natural);
    // Nothing about spades: a spade-less weak hand is a legal pass.
    let spadeless = hand("5432", "5432", "5432", "3");
    assert!(interp.satisfied_by(Seat::South, spadeless));

    let natural = NaturalInference::default();
    let before = auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::Hearts), DBL],
    );
    let BidChoice::Chosen(chosen) = choose_bid(&t, spadeless, &before, &natural_ctx(&natural))
    else {
        panic!("a weak hand must find the natural pass");
    };
    assert_eq!(chosen.call, Call::Pass);
    assert_eq!(chosen.source, ChoiceSource::Natural);
}

/// Review (dropped 7): an exact prefix with no child row (a leaf of the system) never fell back
/// to natural in `choose_bid`, although `interpret` resolves every call there as `Natural`.
///
/// The prefix `1H-(P)` is exact only because a deeper competitive row `1H-(P)-P-(1S)-X` puts
/// East's pass in the trie; South's own continuations there are just that row's implicit pass,
/// which has no entry, so `children` is empty.
#[test]
fn leaf_position_falls_back_to_natural() {
    let mut b = SystemBuilder::new();
    opening_1h(&mut b);
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
        atom_hcp(6, 37),
        SeatCond::Any,
        VulCond::default(),
        "reopening double",
        0,
    );
    let t = table(b);
    let natural = NaturalInference::default();
    let prefix = auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::Hearts), PASS],
    );
    // A 6-HCP hand with five spades: the natural 1S response.
    let responder = hand("432", "Q32", "32", "KJ432");
    let BidChoice::Chosen(chosen) = choose_bid(&t, responder, &prefix, &natural_ctx(&natural))
    else {
        panic!("natural candidates must answer at a system leaf");
    };
    assert_eq!(chosen.source, ChoiceSource::Natural);
    let after = prefix.with(chosen.call).unwrap();
    let interp = interpret(&t, &after, &strict());
    assert_eq!(interp.per_call[2].kind, ResolutionKind::Natural);
    assert!(interp.satisfied_by(Seat::South, responder));
}

/// Review (dropped 6): a leading pass ignored an explicit opening `Pass` row and interpreted the
/// pass as the complement of all opening rows, the `Pass` row included.
#[test]
fn leading_pass_uses_an_explicit_pass_row() {
    let mut b = SystemBuilder::new();
    opening_1h(&mut b);
    let pass_node = b.insert(
        true,
        &[PASS],
        PASS,
        atom_hcp(0, 11),
        SeatCond::Any,
        VulCond::default(),
        "no opening: 0-11",
        0,
    );
    let t = table(b);
    let a = auction(
        Seat::North,
        Vulnerability::None,
        &[PASS, bid(1, Strain::Hearts)],
    );
    let interp = interpret(&t, &a, &strict());
    let north = &interp.per_call[0];
    assert_eq!(north.kind, ResolutionKind::Exact);
    assert!(
        north
            .alternatives
            .iter()
            .all(|(_, _, ex)| ex.node == Some(pass_node))
    );
    let eight = hand("432", "Q32", "K432", "KJ3");
    let fifteen = hand("A32", "KQ2", "K432", "KJ3");
    assert!(interp.satisfied_by(Seat::North, eight));
    assert!(!interp.satisfied_by(Seat::North, fifteen));
}

/// Review (dropped 3): after partner's forcing call, an opponent's intervention releases the
/// obligation to bid, so a pass is not `pass_forcing` (0..=0) any more.
#[test]
fn pass_after_intervention_over_a_forcing_call_is_not_pass_forcing() {
    let mut b = SystemBuilder::new();
    opening_1h(&mut b);
    let two_c = b.insert(
        true,
        &[bid(1, Strain::Hearts), PASS, bid(2, Strain::Clubs)],
        bid(2, Strain::Clubs),
        atom_hcp(10, 37),
        SeatCond::Any,
        VulCond::default(),
        "2C, forcing",
        0,
    );
    b.set_forcing(two_c, Forcing::OneRound);
    let t = table(b);
    let opener = hand("32", "K32", "AQ654", "K32"); // 12 hcp

    let intervened = auction(
        Seat::North,
        Vulnerability::None,
        &[
            bid(1, Strain::Hearts),
            PASS,
            bid(2, Strain::Clubs),
            bid(3, Strain::Diamonds),
            PASS,
        ],
    );
    let interp = interpret(&t, &intervened, &strict());
    assert_eq!(interp.per_call[4].kind, ResolutionKind::Natural);
    assert!(
        !interp.per_call[4].alternatives[0]
            .2
            .text
            .contains("pass_forcing"),
        "{}",
        interp.per_call[4].alternatives[0].2.text
    );
    assert!(interp.satisfied_by(Seat::North, opener));

    // Without intervention the pass is still the contradictory `pass_forcing`.
    let uncontested = auction(
        Seat::North,
        Vulnerability::None,
        &[
            bid(1, Strain::Hearts),
            PASS,
            bid(2, Strain::Clubs),
            PASS,
            PASS,
        ],
    );
    let interp = interpret(&t, &uncontested, &strict());
    assert!(
        interp.per_call[4].alternatives[0]
            .2
            .text
            .contains("pass_forcing")
    );
}

/// Review (dropped 8): `lenient_decay` had no effect, because normalising a single lenient
/// node's alternatives removed `ρ^subst`. The decayed mass now goes to the `Fallback` branch.
#[test]
fn lenient_decay_lowers_the_partial_weight() {
    let sys = sayc_system();
    let t = Table::uniform(sys.sys.clone(), Arc::new(NaturalInference::default()));
    // 1H - (1NT) - 2H resolves by substituting East's 1NT with Pass (one substitution).
    let a = auction(
        Seat::North,
        Vulnerability::None,
        &[
            bid(1, Strain::Hearts),
            bid(1, Strain::NoTrump),
            bid(2, Strain::Hearts),
        ],
    );
    let real_weight = |decay: f32| {
        let opts = InterpretOptions {
            lenient_decay: decay,
            ..InterpretOptions::default()
        };
        let interp = interpret(&t, &a, &opts);
        let pc = &interp.per_call[2];
        assert_eq!(pc.kind, ResolutionKind::Partial { matched_depth: 1 });
        let sum: f32 = pc.alternatives.iter().map(|(_, w, _)| *w).sum();
        assert!((sum - 1.0).abs() < 1e-4, "weights sum to {sum}");
        pc.alternatives
            .iter()
            .filter(|(_, _, ex)| ex.kind != ResolutionKind::Fallback)
            .map(|(_, w, _)| *w)
            .sum::<f32>()
    };
    let eps = InterpretOptions::default().eps_partial;
    assert!((real_weight(0.5) - (1.0 - eps) * 0.5).abs() < 1e-4);
    assert!((real_weight(0.9) - (1.0 - eps) * 0.9).abs() < 1e-4);
}
