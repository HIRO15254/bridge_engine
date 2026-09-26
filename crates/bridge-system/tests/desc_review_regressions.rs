//! Regressions for the phase-3 integration review of the description compiler
//! (`compile/desc/**`, plus the per-seat HCP range `compile/expand.rs` feeds it). Each test names
//! the review finding it pins and uses the real-file text the finding quoted where possible.

use bridge_constraint::{Atom, HandConstraint};
use bridge_core::{Bid, Call, Hand, Holding, Rank, Side as TableSide, Strain, Suit};
use bridge_system::ast::{SeatCond, VulCond};
use bridge_system::compile::desc::context::RowContext;
use bridge_system::compile::desc::{Compiled, compile_description};
use bridge_system::lexer::MemLoader;
use bridge_system::{
    Alertability, Binding, CompileOptions, Forcing, LintCode, Node, NodeFlags, NodeId, Role, RowId,
    Side as PatternSide, SystemIR, SystemMeta,
};

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn holding_of(cards: &str) -> Holding {
    let mut h = Holding::EMPTY;
    for c in cards.chars() {
        let rank = match c.to_ascii_uppercase() {
            'A' => Rank::Ace,
            'K' => Rank::King,
            'Q' => Rank::Queen,
            'J' => Rank::Jack,
            'T' => Rank::Ten,
            '9' => Rank::Nine,
            '8' => Rank::Eight,
            '7' => Rank::Seven,
            '6' => Rank::Six,
            '5' => Rank::Five,
            '4' => Rank::Four,
            '3' => Rank::Three,
            '2' => Rank::Two,
            other => panic!("bad rank char {other}"),
        };
        h = h.with(rank);
    }
    h
}

/// `clubs, diamonds, hearts, spades`, asserting the result holds exactly 13 cards.
fn hand(clubs: &str, diamonds: &str, hearts: &str, spades: &str) -> Hand {
    let h = Hand::from_holdings(
        holding_of(clubs),
        holding_of(diamonds),
        holding_of(hearts),
        holding_of(spades),
    );
    assert_eq!(h.len(), 13, "test hand must hold 13 cards");
    h
}

fn bid(level: u8, strain: Strain) -> Call {
    Call::Bid(Bid::new(level, strain).expect("valid bid"))
}

fn ctx(binding: &Binding, call: Call, role: Role) -> RowContext<'_> {
    RowContext {
        call,
        side: TableSide::NS,
        level: call.bid().map_or(0, |b| b.level()),
        is_jump: false,
        binding,
        hash_suit: None,
        own_prev: None,
        partner_last: None,
        their_last_bid: None,
        agreed_suit: None,
        role,
        partner_hcp: None,
        own_hcp: None,
    }
}

fn node_with(call: Call, constraint: HandConstraint, artificial: bool) -> Node {
    Node {
        id: NodeId(0),
        row: RowId(0),
        side: PatternSide::Us,
        path: std::sync::Arc::from(Vec::new()),
        calls: Vec::new(),
        call,
        binding: Binding::default(),
        seat: SeatCond::default(),
        vul: VulCond::default(),
        constraint,
        branch_weights: None,
        priority: 0,
        volume_log2: 0,
        alertable: Alertability::default(),
        flags: NodeFlags {
            artificial,
            ..NodeFlags::default()
        },
        description: String::new(),
        children: Vec::new(),
    }
}

fn compile_as(text: &str, call: Call, role: Role) -> Compiled {
    let binding = Binding::default();
    let c = ctx(&binding, call, role);
    compile_description(text, &c, &SystemMeta::default())
}

fn compile_text(text: &str) -> Compiled {
    compile_as(text, Call::Pass, Role::Opener)
}

fn compile_system(source: &str) -> SystemIR {
    let opts = CompileOptions {
        coverage_samples: 0,
        ..CompileOptions::default()
    };
    let (ir, _) = bridge_system::compile("inline.bml", source, &MemLoader::default(), &opts);
    ir
}

/// The node whose full auction (`calls`, passes included) renders as `auction`
/// (e.g. `"1NT-Pass-2C"`).
fn node_at<'a>(ir: &'a SystemIR, auction: &str) -> &'a Node {
    ir.nodes
        .iter()
        .find(|n| {
            n.calls
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join("-")
                == auction
        })
        .unwrap_or_else(|| panic!("no node at {auction}"))
}

// ---------------------------------------------------------------------------------------------
// confirmed#12: partner/own HCP ranges come from the whole path, not only the last node
// ---------------------------------------------------------------------------------------------

const STAYMAN: &str = "\
1N = 15--17 HCP
  2C = !STAY
    2H = 4+!h
      3C = FG, 5+!c
      3S = S/T, 4+!h
      2S = INV, 5=!s
";

#[test]
fn review12_strength_words_use_partners_range_from_the_whole_path() {
    let ir = compile_system(STAYMAN);
    // Partner's last call (2H) states no HCP; the 15-17 shown by the 1NT opening still holds.
    // GF: 25 - 15 = 10; S/T: 31 - 17 = 14; INV: 22..24 - 15 = 7..9.
    assert_eq!(
        node_at(&ir, "1NT-Pass-2C-Pass-2H-Pass-3C")
            .constraint
            .hcp_range(),
        10..=37
    );
    assert_eq!(
        node_at(&ir, "1NT-Pass-2C-Pass-2H-Pass-3S")
            .constraint
            .hcp_range(),
        14..=37
    );
    assert_eq!(
        node_at(&ir, "1NT-Pass-2C-Pass-2H-Pass-2S")
            .constraint
            .hcp_range(),
        7..=9
    );
}

#[test]
fn review12_a_path_that_never_states_hcp_is_unknown_and_assumed() {
    // Neither partner call states any HCP: the §7.5 default applies.
    let ir = compile_system("1C = !ART\n  1D = !TRF\n    2N = INV\n");
    // Own INV over an assumed 12..=21 partner: 22 - 12 ..= 24 - 12.
    assert_eq!(
        node_at(&ir, "1C-Pass-1D-Pass-2NT").constraint.hcp_range(),
        10..=12
    );
    // A tracked range that is still the full 0..=37 is unknown, and flagged assumed.
    let binding = Binding::default();
    let partner = node_with(bid(1, Strain::Diamonds), HandConstraint::ANY, true);
    let mut c = ctx(&binding, bid(2, Strain::NoTrump), Role::Responder);
    c.partner_last = Some(&partner);
    c.partner_hcp = Some(0..=37);
    let compiled = compile_description("INV", &c, &SystemMeta::default());
    assert_eq!(compiled.constraint.hcp_range(), 10..=12);
    assert!(
        compiled
            .lints
            .iter()
            .any(|l| l.code == LintCode::AssumedContext)
    );
    // A tracked 15..=17 is known.
    c.partner_hcp = Some(15..=17);
    let known = compile_description("INV", &c, &SystemMeta::default());
    assert_eq!(known.constraint.hcp_range(), 7..=9);
    assert!(
        !known
            .lints
            .iter()
            .any(|l| l.code == LintCode::AssumedContext)
    );
}

// ---------------------------------------------------------------------------------------------
// confirmed#13: "A, B, or C" is one Or
// ---------------------------------------------------------------------------------------------

#[test]
fn review13_oxford_comma_list_is_an_or() {
    let c = compile_text("5+!s, or 4!s and 6+!c");
    assert!(c.constraint.satisfies(hand("AKQ432", "2", "32", "5432")));
    assert!(c.constraint.satisfies(hand("32", "432", "432", "AKQ32")));
    assert!(!c.constraint.satisfies(hand("AK32", "432", "432", "Q32")));
    assert!(
        !c.lints
            .iter()
            .any(|l| l.code == LintCode::UnrecognizedFragment && l.message.contains("\"\"")),
        "no empty Unrecognized fragment: {:?}",
        c.lints
    );

    let three = compile_as(
        "PRE 7+!c, INV+ 4+!d, or UNBAL FG",
        bid(2, Strain::NoTrump),
        Role::Responder,
    );
    assert!(three.constraint.is_satisfiable(), "{:?}", three.constraint);

    // A plain comma list without `or` stays a conjunction.
    let and = compile_text("5+!s, 12-14 hcp");
    assert!(!and.constraint.satisfies(hand("32", "432", "432", "65432")));
}

// ---------------------------------------------------------------------------------------------
// confirmed#14: "(a)"/"(1)" enumeration markers
// ---------------------------------------------------------------------------------------------

#[test]
fn review14_parenthesised_enumeration_items_are_alternatives() {
    let text = "F, Polish Club:\n\
                (a) 12--14 HCP, 2--4!s, 2--4!h, 2--4!d, 2--4!c\n\
                (b) 11--17 HCP, 5+!c or 4414\n\
                (c) 18+ HCP, except the ones that qualify for 1!d";
    let c = compile_as(text, bid(1, Strain::Clubs), Role::Opener);
    assert!(c.constraint.is_satisfiable(), "{:?}", c.constraint);
    // (a) 13 HCP 3-3-4-3, (c) a 20 HCP hand; neither is (b).
    assert!(c.constraint.satisfies(hand("K32", "AQ32", "K32", "Q32")));
    assert!(c.constraint.satisfies(hand("AK2", "AQ32", "KQ2", "K32")));
    // `(1)` works the same way.
    let digits = compile_text("(1) 5+!s\n(2) 5+!h");
    assert!(
        digits
            .constraint
            .satisfies(hand("32", "432", "AK432", "432"))
    );
}

// ---------------------------------------------------------------------------------------------
// confirmed#15: a 4-digit number is a shape only when it fits 13 cards
// ---------------------------------------------------------------------------------------------

#[test]
fn review15_four_digit_numbers_that_are_not_shapes() {
    for text in ["RKCB 0314", "RKCB 1430", "4!d, no 3!s (changed 1-11-2017)"] {
        let c = compile_as(text, bid(4, Strain::Diamonds), Role::Responder);
        assert!(c.constraint.is_satisfiable(), "{text}: {:?}", c.constraint);
    }
    // The date is not read as an HCP range either (`1-11` HCP).
    let dated = compile_as(
        "4!d, no 3!s (changed 1-11-2017)",
        bid(4, Strain::Diamonds),
        Role::Responder,
    );
    assert_eq!(dated.constraint.hcp_range(), 0..=37);
    // A wildcard pattern whose fixed digits already exceed 13 cards is not a shape either
    // (the all-digit case was fixed by 072b7b1; this is the partial-pattern remainder).
    let over = compile_text("5(54)x");
    assert!(over.constraint.is_satisfiable(), "{:?}", over.constraint);
    // A real shape still is one.
    let shape = compile_text("4414");
    assert!(
        shape
            .constraint
            .satisfies(hand("AK32", "2", "5432", "5432"))
    );
}

// ---------------------------------------------------------------------------------------------
// confirmed#16: bare "6-5" / "5-4" / "5-5" are two-suiters, never HCP ranges
// ---------------------------------------------------------------------------------------------

#[test]
fn review16_bare_descending_pairs_are_two_suiters() {
    let c = compile_as("variant 2, 6-5", bid(4, Strain::NoTrump), Role::Responder);
    assert!(c.constraint.is_satisfiable(), "{:?}", c.constraint);
    assert!(c.constraint.satisfies(hand("2", "3", "AK5432", "Q5432")));
    assert!(!c.constraint.satisfies(hand("K32", "AQ32", "K32", "Q32")));
    // `5-5` is not "exactly 5 HCP".
    let five_five = compile_text("5-5");
    assert_eq!(five_five.constraint.hcp_range(), 0..=37);
    assert!(
        five_five
            .constraint
            .satisfies(hand("2", "32", "AKQJ2", "AKQ32"))
    );
    // An explicit metric keeps the numeric reading.
    assert_eq!(compile_text("5-7 hcp").constraint.hcp_range(), 5..=7);
    for text in ["5-4, good suits", "6-4, good suits, (mild) S/T"] {
        assert!(compile_text(text).constraint.is_satisfiable(), "{text}");
    }
}

// ---------------------------------------------------------------------------------------------
// confirmed#17: SPL short suit vs agreed suit, explicit short suit
// ---------------------------------------------------------------------------------------------

#[test]
fn review17_splinter_in_the_agreed_suit_does_not_demand_support_there() {
    // blue `1S-Pass-2C-Pass-4C | SPL, 0--1#, 4+!h`: agreed = short = clubs.
    let binding = Binding::default();
    let partner = node_with(bid(2, Strain::Clubs), HandConstraint::ANY, true);
    let mut c = ctx(&binding, bid(4, Strain::Clubs), Role::Opener);
    c.partner_last = Some(&partner);
    c.agreed_suit = Some(Suit::Clubs);
    let compiled = compile_description("SPL, 7--10 HCP, 0--1!c, 4+!h", &c, &SystemMeta::default());
    assert!(
        compiled.constraint.is_satisfiable(),
        "{:?}",
        compiled.constraint
    );
}

#[test]
fn review17_explicit_short_suit_after_spl() {
    // `1H-Pass-4C | 4!h, SPL !c, 13-15 HCP`.
    let binding = Binding::default();
    let partner = node_with(
        bid(1, Strain::Hearts),
        HandConstraint::Atom(Atom {
            hcp: 12..=21,
            shapes: bridge_core::ShapeSet::from_suit_len(Suit::Hearts, 5, 13),
            ..Atom::ANY
        }),
        false,
    );
    let mut c = ctx(&binding, bid(4, Strain::Clubs), Role::Responder);
    c.partner_last = Some(&partner);
    c.agreed_suit = Some(Suit::Hearts);
    let compiled = compile_description("4!h, SPL !c, 13-15 HCP", &c, &SystemMeta::default());
    assert!(compiled.constraint.is_satisfiable());
    assert_eq!(compiled.constraint.suit_len(Suit::Clubs), 0..=1);
    assert_eq!(compiled.constraint.hcp_range(), 13..=15);

    // `SPL !d` over a 4C call: the short suit is diamonds, not the call's clubs.
    let d = compile_description("SPL !d", &c, &SystemMeta::default());
    assert_eq!(d.constraint.suit_len(Suit::Diamonds), 0..=1);
    // Clubs (the call's own strain) are not made short.
    assert!(*d.constraint.suit_len(Suit::Clubs).end() > 1);
}

// ---------------------------------------------------------------------------------------------
// confirmed#18: forcing flag
// ---------------------------------------------------------------------------------------------

#[test]
fn review18_forcing_flag() {
    let f = |t: &str| compile_text(t).flags.forcing;
    assert_eq!(f("F, 5+!h"), Forcing::OneRound);
    assert_eq!(f("forcing"), Forcing::OneRound);
    assert_eq!(f("GF"), Forcing::ToGame);
    assert_eq!(f("F, GF"), Forcing::ToGame);
    assert_ne!(f("not GF"), Forcing::ToGame);
    assert_ne!(f("weak or GF"), Forcing::ToGame);
    assert_ne!(f("TRF, PRE 7+!c or FG 6+!c"), Forcing::ToGame);
    assert_eq!(f("Non forcing"), Forcing::NonForcing);
    assert_eq!(f("non-forcing"), Forcing::NonForcing);
    assert_eq!(f("not forcing"), Forcing::NonForcing);
    assert_eq!(f("NF"), Forcing::NonForcing);
    assert_eq!(f("5+!h"), Forcing::Unknown);
}

// ---------------------------------------------------------------------------------------------
// confirmed#19: call references are not suit lengths
// ---------------------------------------------------------------------------------------------

#[test]
fn review19_call_references_are_not_lengths() {
    let trf = compile_text("6!s, TRF to 4!s");
    assert!(trf.constraint.satisfies(hand("32", "432", "32", "AQ5432")));
    let see = compile_text("see 1!h-1!s-2!c").constraint;
    assert_eq!(see.hcp_range(), 0..=37);
    for suit in [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades] {
        assert_eq!(see.suit_len(suit), 0..=13);
    }
    let at_least = compile_text("Transfer to 1!s. At least 4!s.");
    assert_eq!(at_least.constraint.suit_len(Suit::Spades), 4..=13);
    assert_eq!(
        compile_text("at most 3!h")
            .constraint
            .suit_len(Suit::Hearts),
        0..=3
    );
    assert!(
        compile_text("44 MM, min/max, no Texas 3!d/3!h")
            .constraint
            .is_satisfiable()
    );
    // A plain length is still a length.
    assert_eq!(compile_text("4!s").constraint.suit_len(Suit::Spades), 4..=4);
    // `5!h-4!s` shorthand is still two lengths, not an auction.
    let two = compile_text("5!h-4!s");
    assert_eq!(two.constraint.suit_len(Suit::Hearts), 5..=5);
    assert_eq!(two.constraint.suit_len(Suit::Spades), 4..=4);
}

// ---------------------------------------------------------------------------------------------
// confirmed#20: hedges
// ---------------------------------------------------------------------------------------------

#[test]
fn review20_possibility_hedges_contribute_no_literal() {
    let strong = compile_as("might be strong", Call::Pass, Role::Responder);
    assert_eq!(strong.constraint.hcp_range(), 0..=37);
    let muiderberg = compile_as(
        "Muiderberg (may be 6!h occasionally), see 2!h opening",
        bid(2, Strain::Hearts),
        Role::Overcaller,
    );
    assert!(muiderberg.constraint.is_satisfiable());
    assert!(muiderberg.flags.soft);
    let rarely = compile_text("5+!s, rarely 4!h");
    assert_eq!(*rarely.constraint.suit_len(Suit::Hearts).start(), 0);
    assert!(*rarely.constraint.suit_len(Suit::Hearts).end() > 4);
    // A probable hedge keeps its literal.
    let usually = compile_text("usually 5+!s");
    assert_eq!(usually.constraint.suit_len(Suit::Spades), 5..=13);
    assert!(usually.flags.soft);
}

// ---------------------------------------------------------------------------------------------
// confirmed#21: spaced group words
// ---------------------------------------------------------------------------------------------

#[test]
fn review21_two_suit_shapes_with_a_space_before_the_group() {
    let mm = compile_text("4+4+ MM");
    assert_eq!(mm.constraint.hcp_range(), 0..=37);
    assert_eq!(*mm.constraint.suit_len(Suit::Spades).start(), 4);
    assert_eq!(*mm.constraint.suit_len(Suit::Hearts).start(), 4);
    let exact = compile_text("44 MM");
    assert_eq!(exact.constraint.suit_len(Suit::Spades), 4..=4);
    assert_eq!(exact.constraint.suit_len(Suit::Hearts), 4..=4);
    let minors = compile_text("5-5 !d+!c");
    assert_eq!(minors.constraint.suit_len(Suit::Diamonds), 5..=5);
    assert_eq!(minors.constraint.suit_len(Suit::Clubs), 5..=5);
}

// ---------------------------------------------------------------------------------------------
// confirmed#22: MIN/MAX fallback, "min/max"
// ---------------------------------------------------------------------------------------------

#[test]
fn review22_min_max_fallbacks() {
    // Opener with no range of its own yet: §7.5's [opening_min, opening_min+2] / [+3, 21].
    assert_eq!(
        compile_as("MIN", bid(1, Strain::Spades), Role::Opener)
            .constraint
            .hcp_range(),
        12..=14
    );
    assert_eq!(
        compile_as("MAX", bid(1, Strain::Spades), Role::Opener)
            .constraint
            .hcp_range(),
        15..=21
    );
    // A responder's first call has no base range: unconstrained, flagged assumed.
    let resp = compile_as(
        "F, 4+!d, possibly MIN 5=M",
        bid(1, Strain::Diamonds),
        Role::Responder,
    );
    assert_eq!(resp.constraint.hcp_range(), 0..=37);
    assert!(
        compile_as("MIN FG", bid(2, Strain::NoTrump), Role::Responder)
            .lints
            .iter()
            .any(|l| l.code == LintCode::AssumedContext)
    );
    // `min/max` is either end: no bound.
    assert_eq!(
        compile_text("Splinter, MIN/MAX").constraint.hcp_range(),
        compile_text("Splinter").constraint.hcp_range()
    );
    assert_eq!(compile_text("min/max").constraint.hcp_range(), 0..=37);
}

// ---------------------------------------------------------------------------------------------
// confirmed#23: QUANT is a slam invitation
// ---------------------------------------------------------------------------------------------

#[test]
fn review23_quant_is_a_slam_invite() {
    let binding = Binding::default();
    let partner = node_with(
        bid(1, Strain::NoTrump),
        HandConstraint::Atom(Atom {
            hcp: 15..=17,
            ..Atom::ANY
        }),
        false,
    );
    let mut c = ctx(&binding, bid(4, Strain::NoTrump), Role::Responder);
    c.partner_last = Some(&partner);
    let meta = SystemMeta::default();
    // Small-slam total 33: 33 - 17 ..= 33 - 15 - 1.
    for text in ["F QUANT", "QUANT INV to 6NT, NF"] {
        assert_eq!(
            compile_description(text, &c, &meta).constraint.hcp_range(),
            16..=17,
            "{text}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// confirmed#24: stopper and honour runs
// ---------------------------------------------------------------------------------------------

#[test]
fn review24_stopper_follows_the_section_7_4_definition() {
    let stop = compile_text("stopper in !h");
    // `hearts` plus five low spades, the rest diamonds (at most 7 of them), no clubs.
    let with_hearts = |hearts: &str| {
        let diamonds: String = "AKQJT98".chars().take(8 - hearts.len()).collect();
        hand("", &diamonds, hearts, "65432")
    };
    assert!(stop.constraint.satisfies(with_hearts("A")));
    assert!(stop.constraint.satisfies(with_hearts("K2")));
    assert!(!stop.constraint.satisfies(with_hearts("K")));
    assert!(!stop.constraint.satisfies(with_hearts("Q2")));
    assert!(stop.constraint.satisfies(with_hearts("Q32")));
    assert!(stop.constraint.satisfies(with_hearts("J432")));
    assert!(!stop.constraint.satisfies(with_hearts("J32")));
    let no_stop = compile_text("no stopper in !h");
    assert!(no_stop.constraint.satisfies(with_hearts("Q2")));
}

#[test]
fn review24_honour_runs_require_the_listed_honours_and_length() {
    let binding = Binding::default();
    let c = ctx(&binding, bid(3, Strain::Spades), Role::Opener);
    let meta = SystemMeta::default();
    let run = compile_description("QJ10xx", &c, &meta);
    assert!(run.constraint.satisfies(hand("32", "432", "432", "QJT98")));
    // Holds two of the top three but not the written Q-J-10.
    assert!(!run.constraint.satisfies(hand("32", "432", "432", "AK982")));
    // Too short for the five written cards.
    assert!(!run.constraint.satisfies(hand("32", "5432", "432", "QJT9")));
    let long = compile_description("KQJ109x", &c, &meta);
    assert!(
        !long
            .lints
            .iter()
            .any(|l| l.code == LintCode::UnrecognizedFragment),
        "{:?}",
        long.lints
    );
    assert_eq!(long.constraint.suit_len(Suit::Spades), 6..=13);
}
