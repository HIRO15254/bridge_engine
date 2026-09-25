//! Vocabulary-coverage tests for the description compiler (task 3.3, deliverable #2).
//!
//! Every v1 vocabulary row of `docs/design/06-system.md` §7.4 gets at least one example here,
//! compiled through the real, public [`compile_description`] entry point and checked either by
//! `HandConstraint::satisfies` on a hand-written 13-card hand (for rows that produce an `Atom`),
//! or by the derived flags / `hcp_range` (for rows that are context-dependent or carry no atom
//! at all, e.g. `Forcing`/`Convention`). Precedence, enumeration, negation, hedge and
//! context-dependent-word coverage each get a dedicated test too, using a synthetic
//! [`RowContext`] — no `is_satisfiable`/`Sampler` call anywhere in this file, per scope.

use bridge_constraint::HandConstraint;
use bridge_core::{Bid, Call, Hand, Holding, Rank, Side as TableSide, Strain, Suit};
use bridge_system::ast::{SeatCond, VulCond};
use bridge_system::compile::desc::compile_description;
use bridge_system::compile::desc::context::RowContext;
use bridge_system::{
    Alertability, Binding, Forcing, Node, NodeFlags, NodeId, Role, RowId, Side as PatternSide,
    SystemMeta,
};

// ---------------------------------------------------------------------------------------------
// Shared helpers (mirrors the pattern already used by `compile::desc::mod`'s own unit tests).
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

fn base_ctx(binding: &Binding, call: Call, role: Role) -> RowContext<'_> {
    RowContext {
        call,
        side: TableSide::NS,
        level: match call {
            Call::Bid(bid) => bid.level(),
            _ => 0,
        },
        is_jump: false,
        binding,
        hash_suit: None,
        own_prev: None,
        partner_last: None,
        their_last_bid: None,
        agreed_suit: None,
        role,
    }
}

/// A minimal fully-formed `Node` standing in for an ancestor on the path, carrying only an HCP
/// range (all this file's context tests need from `partner_last`/`own_prev`).
fn node_with_hcp(range: core::ops::RangeInclusive<u8>) -> Node {
    Node {
        id: NodeId(0),
        row: RowId(0),
        side: PatternSide::Us,
        path: std::sync::Arc::from(Vec::new()),
        calls: Vec::new(),
        call: Call::Pass,
        binding: Binding::default(),
        seat: SeatCond::default(),
        vul: VulCond::default(),
        constraint: HandConstraint::Atom(bridge_constraint::Atom {
            hcp: range,
            ..bridge_constraint::Atom::ANY
        }),
        branch_weights: None,
        priority: 0,
        volume_log2: 0,
        alertable: Alertability::default(),
        flags: NodeFlags::default(),
        description: String::new(),
        children: Vec::new(),
    }
}

fn bid(level: u8, strain: Strain) -> Call {
    Call::Bid(Bid::new(level, strain).expect("valid bid"))
}

// ---------------------------------------------------------------------------------------------
// Context-free tokens (Pass 1 vocabulary): one representative example per `Token` variant.
// ---------------------------------------------------------------------------------------------

#[test]
fn hcp_row() {
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();
    let compiled = compile_description("15-17 hcp", &c, &meta);
    // AK9(7) + Q43(2) + J43(1) + KQ43(5) = 15 hcp.
    assert!(
        compiled
            .constraint
            .satisfies(hand("AK9", "Q43", "J43", "KQ43"))
    );
    // 29(0) + Q43(2) + J43(1) + KQT43(6) = 9 hcp: below the range.
    assert!(
        !compiled
            .constraint
            .satisfies(hand("29", "Q43", "J43", "KQT43"))
    );
}

#[test]
fn points_row_total_points() {
    // "13+ points" (total points, default Goren 3-2-1): a flat 4333 hand has no distribution
    // points, so total points == hcp for it, keeping the check simple and unambiguous.
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();
    let compiled = compile_description("13+ points", &c, &meta);
    let flat_14 = hand("KQ2", "J43", "Q43", "AK92"); // 3+3+3+4=13 cards; A K Q J = 3+2+4+3=... see below
    // AK92 spades = A(4)+K(3)=7; KQ2 clubs = K(3)+Q(2)=5; J43 diamonds = J(1); Q43 hearts = Q(2).
    // hcp = 7+5+1+2 = 15, shape 3-3-3-4 (clubs3 diamonds3 hearts3 spades4) = 4333 class, flat.
    assert!(compiled.constraint.satisfies(flat_14));
    let flat_low = hand("432", "432", "432", "K432");
    assert!(!compiled.constraint.satisfies(flat_low));
}

#[test]
fn suit_len_row_fixed_suit() {
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();
    let compiled = compile_description("5+!s", &c, &meta);
    assert!(
        compiled
            .constraint
            .satisfies(hand("432", "432", "43", "AKQ32"))
    );
    assert!(
        !compiled
            .constraint
            .satisfies(hand("432", "432", "9432", "AKQ"))
    );
}

#[test]
fn shape_row_literal_digit_string() {
    // "4414" reads S H D C: spades=4, hearts=4, diamonds=1, clubs=4.
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();
    let compiled = compile_description("4414", &c, &meta);
    let matching = hand("A432", "9", "KQ32", "J432");
    assert!(compiled.constraint.satisfies(matching));
    let other_rotation = hand("A432", "J432", "9", "KQ32");
    assert!(!compiled.constraint.satisfies(other_rotation));
}

#[test]
fn balanced_semi_balanced_unbalanced_rows() {
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();

    let flat_4333 = hand("KQ2", "J43", "Q43", "AK92");
    let semi_5422 = hand("32", "43", "KQ43", "AK932");
    let unbal_6511 = hand("A", "K", "AKJ43", "KQJ982");

    let bal = compile_description("balanced", &c, &meta);
    assert!(bal.constraint.satisfies(flat_4333));
    assert!(!bal.constraint.satisfies(semi_5422));

    let semi = compile_description("semi-bal", &c, &meta);
    assert!(semi.constraint.satisfies(flat_4333));
    assert!(semi.constraint.satisfies(semi_5422));
    assert!(!semi.constraint.satisfies(unbal_6511));

    let unbal = compile_description("unbal", &c, &meta);
    assert!(!unbal.constraint.satisfies(flat_4333));
    assert!(!unbal.constraint.satisfies(semi_5422));
    assert!(unbal.constraint.satisfies(unbal_6511));
}

#[test]
fn quality_row_suit_quality_word() {
    // "solid" needs an "own suit" from the row's own call: open 1!s.
    let binding = Binding::default();
    let c = base_ctx(
        &binding,
        bid(1, Strain::from_suit(Suit::Spades)),
        Role::Opener,
    );
    let meta = SystemMeta::default();
    let compiled = compile_description("5+!s, solid", &c, &meta);
    let solid = hand("432", "432", "43", "AKQ32");
    assert!(compiled.constraint.satisfies(solid));
    let not_solid = hand("432", "432", "43", "AK432"); // A K but no Q among the top 3.
    assert!(!compiled.constraint.satisfies(not_solid));
}

#[test]
fn stopper_row() {
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();
    let compiled = compile_description("stopper in !h", &c, &meta);
    let has_stopper = hand("432", "432", "AQ2", "9432");
    assert!(compiled.constraint.satisfies(has_stopper));
    let no_stopper = hand("432", "43", "432", "AQ432");
    assert!(!compiled.constraint.satisfies(no_stopper));
}

#[test]
fn shortness_row() {
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();
    let compiled = compile_description("singleton !d", &c, &meta);
    let singleton_d = hand("A432", "K", "AQ32", "Q432");
    assert!(compiled.constraint.satisfies(singleton_d));
    let three_d = hand("A32", "K43", "AQ32", "Q32"); // 3 diamonds: not a singleton.
    assert!(!compiled.constraint.satisfies(three_d));
}

#[test]
fn support_row() {
    let binding = Binding::default();
    let mut c = base_ctx(&binding, Call::Pass, Role::Responder);
    c.agreed_suit = Some(Suit::Hearts);
    let meta = SystemMeta::default();
    let compiled = compile_description("fit", &c, &meta);
    let three_hearts = hand("A432", "432", "K32", "432"); // 3 hearts.
    assert!(compiled.constraint.satisfies(three_hearts));
    let two_hearts = hand("A432", "98432", "K3", "43"); // 2 hearts.
    assert!(!compiled.constraint.satisfies(two_hearts));
}

#[test]
fn controls_row() {
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();
    let compiled = compile_description("2+ controls", &c, &meta);
    // A (2 controls) + Q high in another suit: exactly 2 controls.
    let two_controls = hand("A32", "432", "432", "Q432");
    assert!(compiled.constraint.satisfies(two_controls));
    let zero_controls = hand("Q32", "432", "432", "Q432");
    assert!(!compiled.constraint.satisfies(zero_controls));
}

#[test]
fn losers_row() {
    // "N losers" (bare number, no `+`/`-`) is an exact match (`tokens::recognize`'s own
    // `controls_and_losers` test confirms `"7 losers"` recognises as `Token::Losers(7..=7)`), so
    // the description text is built from the classic-LTC value actually computed for each hand
    // rather than a hand-derived guess.
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();

    let strong = hand("AKQ", "AKQ", "AKQ", "AKQ4");
    let weak_ish = hand("432", "432", "432", "A432");
    let strong_losers = bridge_eval::losers_with(strong, bridge_eval::LtcMethod::Classic).halves();
    let weak_losers = bridge_eval::losers_with(weak_ish, bridge_eval::LtcMethod::Classic).halves();
    assert_ne!(strong_losers, weak_losers);

    let compiled = compile_description(&format!("{strong_losers} losers"), &c, &meta);
    assert!(compiled.constraint.satisfies(strong));
    assert!(!compiled.constraint.satisfies(weak_ish));
}

#[test]
fn no_bound_row_carries_no_constraint() {
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();
    let compiled = compile_description("unlimited", &c, &meta);
    assert!(matches!(
        compiled.constraint,
        HandConstraint::Atom(a) if a == bridge_constraint::Atom::ANY
    ));
}

#[test]
fn forcing_row_sets_flags_not_atoms() {
    // `Token::Forcing` (as opposed to the `Strength(GameForcing)` reading of the literal
    // words "GF"/"FG") carries no atom at all: use the plain forcing-marker abbreviations
    // (`F1`, `NF`) that `tokens::match_forcing` recognises directly.
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();

    let f1 = compile_description("F1", &c, &meta);
    assert_eq!(f1.flags.forcing, Forcing::OneRound);
    assert!(
        matches!(f1.constraint, HandConstraint::Atom(ref a) if *a == bridge_constraint::Atom::ANY)
    );

    let nf = compile_description("NF", &c, &meta);
    assert_eq!(nf.flags.forcing, Forcing::NonForcing);
    assert!(
        matches!(nf.constraint, HandConstraint::Atom(ref a) if *a == bridge_constraint::Atom::ANY)
    );
}

#[test]
fn convention_row_carries_no_constraint_but_is_artificial() {
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();
    let compiled = compile_description("STAY", &c, &meta);
    assert!(
        matches!(compiled.constraint, HandConstraint::Atom(a) if a == bridge_constraint::Atom::ANY)
    );
    assert!(compiled.flags.artificial);
    assert!(!compiled.recognition.constraint_bearing);
}

#[test]
fn natural_row_resolves_suit_length_from_the_call() {
    // Opener's natural 1!s: no explicit length, so the minimal natural rule (own role's
    // 1-major opening length, per `NaturalParams::default()`) applies: 5+ spades.
    let binding = Binding::default();
    let c = base_ctx(
        &binding,
        bid(1, Strain::from_suit(Suit::Spades)),
        Role::Opener,
    );
    let meta = SystemMeta::default();
    let compiled = compile_description("NAT", &c, &meta);
    let five_spades = hand("432", "432", "43", "AKQ32");
    assert!(compiled.constraint.satisfies(five_spades));
    let four_spades = hand("432", "432", "432", "AKQ2");
    assert!(!compiled.constraint.satisfies(four_spades));
    assert!(
        compiled
            .lints
            .iter()
            .any(|l| l.code == bridge_system::LintCode::AssumedContext),
        "NAT's minimal resolution is documented as an assumed default (open_issues: should \
         eventually route through NaturalInference)"
    );
}

// ---------------------------------------------------------------------------------------------
// Context-dependent strength words (`StrengthWord`), each with a synthetic `RowContext`.
// ---------------------------------------------------------------------------------------------

#[test]
fn strength_game_forcing() {
    let binding = Binding::default();
    let mut c = base_ctx(&binding, Call::Pass, Role::Responder);
    let partner = node_with_hcp(12..=14);
    c.partner_last = Some(&partner);
    let meta = SystemMeta::default();
    let compiled = compile_description("GF", &c, &meta);
    assert_eq!(compiled.constraint.hcp_range(), 13..=37); // 25 - 12
}

#[test]
fn strength_invitational_and_invitational_plus() {
    let binding = Binding::default();
    let mut c = base_ctx(&binding, Call::Pass, Role::Responder);
    let partner = node_with_hcp(12..=14);
    c.partner_last = Some(&partner);
    let meta = SystemMeta::default();

    let inv = compile_description("INV", &c, &meta);
    // inv_total 22..=24, partner_min 12: [10, 12].
    assert_eq!(inv.constraint.hcp_range(), 10..=12);

    let inv_plus = compile_description("INV+", &c, &meta);
    assert_eq!(*inv_plus.constraint.hcp_range().start(), 10);
}

#[test]
fn strength_min_max_split_previous_range() {
    let binding = Binding::default();
    let mut c = base_ctx(&binding, Call::Pass, Role::Opener);
    let prev = node_with_hcp(12..=21);
    c.own_prev = Some(&prev);
    let meta = SystemMeta::default();

    let min = compile_description("MIN", &c, &meta);
    assert!(*min.constraint.hcp_range().end() < 21);

    let max = compile_description("MAX", &c, &meta);
    assert!(*max.constraint.hcp_range().start() > 12);
}

#[test]
fn strength_weak_by_role() {
    let binding = Binding::default();
    let meta = SystemMeta::default();

    let responder_ctx = base_ctx(&binding, Call::Pass, Role::Responder);
    let responder_weak = compile_description("weak", &responder_ctx, &meta);
    assert_eq!(responder_weak.constraint.hcp_range(), 0..=9); // weak_max default 9.

    let opener_two_ctx = base_ctx(
        &binding,
        bid(2, Strain::from_suit(Suit::Hearts)),
        Role::Opener,
    );
    let opener_weak_two = compile_description("weak", &opener_two_ctx, &meta);
    assert_eq!(opener_weak_two.constraint.hcp_range(), 5..=10); // NaturalParams::default weak_two.
}

#[test]
fn strength_strong_and_preemptive() {
    let binding = Binding::default();
    let meta = SystemMeta::default();

    let ctx = base_ctx(&binding, Call::Pass, Role::Opener);
    let strong = compile_description("STR", &ctx, &meta);
    assert_eq!(strong.constraint.hcp_range(), 16..=37); // strong_min default.

    let preempt_ctx = base_ctx(
        &binding,
        bid(3, Strain::from_suit(Suit::Hearts)),
        Role::Opener,
    );
    let preempt = compile_description("PRE", &preempt_ctx, &meta);
    assert_eq!(preempt.constraint.hcp_range(), 5..=9); // preempt table, level 3.
}

#[test]
fn strength_slam_try_and_quantitative() {
    let binding = Binding::default();
    let mut c = base_ctx(&binding, Call::Pass, Role::Responder);
    let partner = node_with_hcp(15..=17);
    c.partner_last = Some(&partner);
    let meta = SystemMeta::default();

    let slam_try = compile_description("S/T", &c, &meta);
    // slam_total 31 - partner_max 17 = 14.
    assert_eq!(slam_try.constraint.hcp_range(), 14..=37);

    let quant = compile_description("QUANT", &c, &meta);
    // [gf_total(25) - partner_max(17) + 1, slam_total(31) - partner_min(15)] = [9, 16].
    assert_eq!(quant.constraint.hcp_range(), 9..=16);
}

#[test]
fn strength_negative_and_limit() {
    let binding = Binding::default();
    let mut c = base_ctx(&binding, Call::Pass, Role::Responder);
    let partner = node_with_hcp(12..=14);
    c.partner_last = Some(&partner);
    let meta = SystemMeta::default();

    let neg = compile_description("NEG", &c, &meta);
    assert_eq!(neg.constraint.hcp_range(), 0..=7); // neg_max default.

    let lim = compile_description("LIM", &c, &meta);
    // Same formula as INV in this implementation: [22-12, 24-12] = [10, 12].
    assert_eq!(lim.constraint.hcp_range(), 10..=12);
}

// ---------------------------------------------------------------------------------------------
// `SuitRef` context words: `#` (hash), own suit, agreed suit, opponents' suit.
// ---------------------------------------------------------------------------------------------

#[test]
fn suitref_hash_from_context() {
    let binding = Binding::default();
    let mut c = base_ctx(&binding, Call::Pass, Role::Responder);
    c.hash_suit = Some(Suit::Diamonds);
    let meta = SystemMeta::default();
    let compiled = compile_description("5+#", &c, &meta);
    let five_d = hand("432", "AKQ32", "43", "432");
    assert!(compiled.constraint.satisfies(five_d));
    let four_d = hand("9432", "AKQ2", "432", "43");
    assert!(!compiled.constraint.satisfies(four_d));
}

#[test]
fn suitref_hash_missing_is_assumed() {
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Responder);
    let meta = SystemMeta::default();
    let compiled = compile_description("5+#", &c, &meta);
    assert!(
        compiled
            .lints
            .iter()
            .any(|l| l.code == bridge_system::LintCode::AssumedContext)
    );
}

#[test]
fn suitref_agreed_suit_via_support_and_shortness() {
    let binding = Binding::default();
    let mut c = base_ctx(&binding, Call::Pass, Role::Responder);
    c.agreed_suit = Some(Suit::Spades);
    let meta = SystemMeta::default();
    // A void in an unnamed suit while agreeing spades: shortness token still needs its own
    // explicit suit reference, so this exercises `Agreed` only through `Support`/`fit` above;
    // here we instead confirm the agreed suit feeds `Token::Support`'s minimum length directly.
    let compiled = compile_description("fit", &c, &meta);
    let four_spades = hand("432", "432", "432", "AK32");
    assert!(compiled.constraint.satisfies(four_spades));
}

// ---------------------------------------------------------------------------------------------
// Precedence, enumeration, negation and hedges.
// ---------------------------------------------------------------------------------------------

#[test]
fn precedence_and_binds_tighter_than_or_and_comma() {
    // "5+!c and 4!h or 4!s, 11-15 hcp" parses as: ((5+!c AND 4!h) OR 4!s) AND (11-15 hcp).
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();
    let compiled = compile_description("5+!c and 4=!h or 4=!s, 11-15 hcp", &c, &meta);

    // Clubs+hearts branch: 5 clubs (AK432, 7 hcp), 4 hearts (KQ32, 5 hcp), 12 hcp total.
    let branch_a = hand("AK432", "43", "KQ32", "43");
    assert!(compiled.constraint.satisfies(branch_a));

    // Spades branch alone (3 clubs, 3 hearts: neither 5+!c nor 4=!h holds): 4 spades, 12 hcp.
    let branch_b = hand("432", "432", "K32", "AKQ2");
    assert!(compiled.constraint.satisfies(branch_b));

    // Neither branch (4 clubs, 3 spades): still 12 hcp — fails the Or.
    let neither = hand("AK32", "432", "KQ2", "432");
    assert!(!compiled.constraint.satisfies(neither));

    // Second branch matches (exactly 4 spades) but hcp is out of range (19, too high).
    let out_of_hcp = hand("AKQ2", "432", "43", "AKQJ");
    assert!(!compiled.constraint.satisfies(out_of_hcp));
}

#[test]
fn enumeration_numbered_items_form_or_group() {
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();
    let compiled = compile_description("1) 5+!c\n2) 5+!d", &c, &meta);
    let clubs = hand("AK432", "432", "432", "43");
    assert!(compiled.constraint.satisfies(clubs));
    let diamonds = hand("432", "AK432", "432", "43");
    assert!(compiled.constraint.satisfies(diamonds));
    let neither = hand("432", "432", "AK432", "43");
    assert!(!compiled.constraint.satisfies(neither));
}

#[test]
fn negation_word_denies() {
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();
    let compiled = compile_description("denies stopper in !h", &c, &meta);
    let has_stopper = hand("432", "432", "AQ2", "9432");
    assert!(!compiled.constraint.satisfies(has_stopper));
    let no_stopper = hand("432", "43", "432", "AQ432");
    assert!(compiled.constraint.satisfies(no_stopper));
}

#[test]
fn hedge_word_may_still_sets_soft_without_loosening() {
    let binding = Binding::default();
    let c = base_ctx(&binding, Call::Pass, Role::Opener);
    let meta = SystemMeta::default();
    let compiled = compile_description("may have 5+!s", &c, &meta);
    assert!(compiled.flags.soft);
    assert!(
        compiled
            .lints
            .iter()
            .any(|l| l.code == bridge_system::LintCode::SoftConstraint)
    );
    let five_spades = hand("432", "432", "43", "AKQ32");
    assert!(compiled.constraint.satisfies(five_spades));
    let four_spades = hand("432", "432", "432", "AKQ2");
    // Still an exact 5+ requirement: the hedge does not widen it.
    assert!(!compiled.constraint.satisfies(four_spades));
}
