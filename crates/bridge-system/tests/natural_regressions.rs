//! Regression tests for the phase-3 integration review of `NaturalInference` (06-system.md §8):
//! each test names the auction the review used as evidence and checks the rule/constraint that
//! should fire instead.

mod common;

use bridge_core::{Seat, Vulnerability};
use bridge_system::natural::{
    CallContext, CallKind, DoubleKind, Inference, NaturalInference, classify,
};
use common::{auction, hand};

/// Classifies and infers the *last* call of `calls` (dealer North) for the seat that made it.
fn last(calls: &str) -> (CallContext, Inference) {
    let a = auction(Seat::North, Vulnerability::None, calls);
    let index = a.len() - 1;
    let ctx = classify(&a, index, a.seat_at(index));
    let inf = NaturalInference::default().infer(&ctx);
    (ctx, inf)
}

const LOW: &str = "432";

// --- open_pass (review: open_pass fires on the opener's later passes) --------------------------

#[test]
fn open_pass_only_before_anyone_has_bid() {
    // Still the opening pass.
    assert_eq!(last("P").1.rule, "open_pass");
    assert_eq!(last("P P").1.rule, "open_pass");
    // The opener's later passes are not "declines to open".
    for calls in [
        "1H P 2H P P",
        "1NT P 3NT P P",
        "1NT P 2C 2S P",
        "1H P 1S 2C P",
    ] {
        let (_, inf) = last(calls);
        assert_ne!(inf.rule, "open_pass", "{calls}");
        // A 15-HCP opener must be able to pass.
        let opener = hand("K32", "A32", "AQ32", "K32");
        assert!(inf.constraint.satisfies(opener), "{calls}: {}", inf.rule);
    }
}

/// With every call natural, a minimum 1H opener can pass partner's single raise.
#[test]
fn minimum_opener_can_pass_a_single_raise() {
    let a = auction(Seat::North, Vulnerability::None, "1H P 2H P");
    let min_opener = hand("32", "K32", "AQ654", "K32");
    let offered: Vec<_> = NaturalInference::default()
        .candidates(&a, Seat::North)
        .into_iter()
        .filter(|(_, c, _)| c.satisfies(min_opener))
        .map(|(call, _, _)| call)
        .collect();
    assert!(offered.contains(&bridge_core::Call::Pass), "{offered:?}");
}

// --- pass_default (review: caps every responder/advancer pass at 0-5) --------------------------

#[test]
fn pass_default_bounds_only_the_first_pass_of_partners_opening() {
    let six = hand(LOW, "Q432", "32", "KJ32"); // 6 hcp
    // Responder's first pass of a 1-level suit opening: 0-5.
    let (_, inf) = last("1H P P");
    assert_eq!(inf.rule, "pass_default");
    assert!(!inf.constraint.satisfies(six));
    assert!(inf.constraint.satisfies(hand(LOW, LOW, "5432", "Q32")));
    // Pass of partner's 1NT: up to 7 (25 - 17 - 1).
    let seven = hand("J32", "Q432", LOW, "KJ3"); // 7 hcp
    let eight = hand("Q32", "Q432", LOW, "KJ3"); // 8 hcp
    let (_, inf) = last("1NT P P");
    assert!(inf.constraint.satisfies(seven));
    assert!(!inf.constraint.satisfies(eight));
    // Passes of a weak two, or later passes by a responder who already bid, are unbounded.
    for calls in [
        "2H P P",
        "1H P 1S P 2H P P",
        "1H P 1S P 2C P P",
        "1D P 1S P 2H 3C P",
    ] {
        let (_, inf) = last(calls);
        assert!(inf.constraint.satisfies(six), "{calls}: {}", inf.rule);
    }
    // Advancer: first pass of partner's simple overcall is bounded, a later pass is not.
    let (_, inf) = last("1H 1S P P");
    assert!(!inf.constraint.satisfies(six));
    let twelve = hand("A32", "KQ32", "32", "K432");
    let (_, inf) = last("1H 1S P 2H P 2S P P");
    assert!(inf.constraint.satisfies(twelve), "{}", inf.rule);
}

/// With every call natural, a 6-HCP responder who bid 1S can pass opener's 2C rebid.
#[test]
fn responder_who_bid_can_pass_later() {
    let a = auction(Seat::North, Vulnerability::None, "1H P 1S P 2C P");
    let responder = hand(LOW, "Q432", "43", "KJ32");
    let offered: Vec<_> = NaturalInference::default()
        .candidates(&a, Seat::South)
        .into_iter()
        .filter(|(_, c, _)| c.satisfies(responder))
        .map(|(call, _, _)| call)
        .collect();
    assert!(offered.contains(&bridge_core::Call::Pass), "{offered:?}");
}

// --- overcall / jump_overcall / nt_overcall (review: accept cue bids and later bids) -----------

#[test]
fn cue_bids_are_not_overcalls() {
    for calls in ["1H 2H", "1H 3H", "1H P P 2H"] {
        let (ctx, inf) = last(calls);
        assert!(
            matches!(ctx.kind, CallKind::Bid { cue: true, .. }),
            "{calls}"
        );
        assert_eq!(inf.rule, "cue", "{calls}");
    }
}

#[test]
fn nt_overcall_is_the_cheapest_notrump_only() {
    assert_eq!(last("1H 1NT").1.rule, "nt_overcall");
    assert_eq!(last("2H 2NT").1.rule, "nt_overcall");
    assert_ne!(last("1H 2NT").1.rule, "nt_overcall");
    assert_ne!(last("1H 3NT").1.rule, "nt_overcall");
}

#[test]
fn overcallers_later_bids_are_not_fresh_overcalls() {
    // Raise of the advancer's suit: a raise, not "5+ clubs".
    let (_, inf) = last("1H 1S P 2C P 3C");
    assert_eq!(inf.rule, "raise");
    // Rebid of the overcaller's own suit: not a new overcall.
    let (_, inf) = last("1H 1S P 2C P 2S");
    assert_ne!(inf.rule, "overcall");
    assert_ne!(inf.rule, "jump_overcall");
}

// --- cue (review: advancer cue uses the GF formula) --------------------------------------------

#[test]
fn advancer_cue_keeps_advance_cue_even_with_partner_context() {
    let a = auction(Seat::North, Vulnerability::None, "1H 1S P 2H");
    let mut ctx = classify(&a, 3, Seat::West);
    // What interpret fills in: partner's 1S overcall, 8+ HCP.
    ctx.partner_constraint = Some(bridge_constraint::HandConstraint::Atom(
        bridge_constraint::Atom::ANY.with_hcp(8..=16),
    ));
    let inf = NaturalInference::default().infer(&ctx);
    assert_eq!(inf.rule, "cue");
    assert_eq!(*inf.constraint.hcp_range().start(), 10);
    // Responder's cue keeps the game-forcing formula (25 - partner's minimum).
    let a = auction(Seat::North, Vulnerability::None, "1H 1S 2S");
    let mut ctx = classify(&a, 2, Seat::South);
    ctx.partner_constraint = Some(bridge_constraint::HandConstraint::Atom(
        bridge_constraint::Atom::ANY.with_hcp(12..=21),
    ));
    let inf = NaturalInference::default().infer(&ctx);
    assert_eq!(inf.rule, "cue");
    assert_eq!(*inf.constraint.hcp_range().start(), 13);
}

// --- doubles (review: §8.2 differences) --------------------------------------------------------

#[test]
fn double_after_partners_notrump_is_penalty() {
    let (ctx, inf) = last("1NT 2H X");
    assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Penalty));
    assert_eq!(inf.rule, "penalty_x");
}

#[test]
fn reopening_double_after_own_overcall_is_takeout() {
    let (ctx, inf) = last("1H 1S 2H P P X");
    assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Takeout));
    assert_eq!(inf.rule, "takeout_x");
}

#[test]
fn double_after_agreeing_a_suit_is_penalty() {
    for calls in ["1H P 2H 2S X", "1H P 2H 3C X"] {
        let (ctx, inf) = last(calls);
        assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Penalty), "{calls}");
        assert_eq!(inf.rule, "penalty_x", "{calls}");
    }
    // Unchanged: negative, support and responsive doubles.
    assert_eq!(
        last("1D 1S X").0.kind,
        CallKind::Double(DoubleKind::Negative)
    );
    assert_eq!(
        last("1D P 1H 1S X").0.kind,
        CallKind::Double(DoubleKind::Support)
    );
    assert_eq!(
        last("P 1S X 2S X").0.kind,
        CallKind::Double(DoubleKind::Responsive)
    );
}

// --- opener re-raising an agreed suit (review: read as a raise of partner's suit) --------------

#[test]
fn opener_reraise_of_own_agreed_suit_is_not_a_raise() {
    let (ctx, inf) = last("1H P 2H P 3H");
    assert!(matches!(
        ctx.kind,
        CallKind::Bid {
            raise: false,
            rebid_own: true,
            ..
        }
    ));
    assert_eq!(inf.rule, "rebid_own");
    // A game try: 16-18 with the opening's length.
    let try_hand = hand("32", "AK2", "AQJ54", "K32"); // 17 hcp, 5 hearts
    let minimum = hand("32", "K32", "AQ654", "K32"); // 12 hcp
    assert!(inf.constraint.satisfies(try_hand));
    assert!(!inf.constraint.satisfies(minimum));
    // Responder raising opener's suit is still a raise.
    assert_eq!(last("1H P 2H").1.rule, "raise");
}

// --- rule-table gaps (review: common natural calls fall to fallback) ---------------------------

#[test]
fn opener_notrump_rebids_are_natural() {
    let (_, inf) = last("1C P 1H P 1NT");
    assert_eq!(inf.rule, "rebid_nt");
    assert_eq!(inf.constraint.hcp_range(), 12..=14);
    let (_, inf) = last("1C P 1H P 2NT");
    assert_eq!(inf.rule, "rebid_nt");
    assert_eq!(inf.constraint.hcp_range(), 18..=19);
    // Not once a suit is agreed.
    assert_ne!(last("1H P 2H P 2NT").1.rule, "rebid_nt");
}

#[test]
fn opener_new_suit_rebid_is_natural() {
    let (_, inf) = last("1H P 1S P 2C");
    assert_eq!(inf.rule, "rebid_new_suit");
    let good = hand("AJ32", "32", "AKJ32", "32"); // 4 clubs, 5 hearts, 13 hcp
    let short = hand("A32", "32", "AKJ32", "K32"); // 3 clubs
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(short));
    let (_, inf) = last("1H P 1S P 3C");
    assert_eq!(inf.rule, "rebid_new_suit");
    assert_eq!(*inf.constraint.hcp_range().start(), 19);
}

#[test]
fn advancer_new_suit_is_natural() {
    let (_, inf) = last("1H 1S P 2C");
    assert_eq!(inf.rule, "advance_new_suit");
    let good = hand("KQJ32", "Q32", "32", "432"); // 5 clubs, 8 hcp
    let short = hand("KQJ2", "Q432", "32", "432"); // 4 clubs
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(short));
}

/// An unbounded pass (anything but the limited first pass of `pass_default_max_hcp`) must rank
/// below every bid the hand satisfies; at equal priority `choose_bid` takes the lowest call, so
/// the opener's `ANY` pass used to shadow `reverse` and `rebid_new_suit` completely.
#[test]
fn unbounded_pass_ranks_below_satisfied_bids() {
    let engine = NaturalInference::default();
    let cases = [
        ("1C P 1S P", hand("AKQ32", "32", "AKJ2", "32"), "2H"), // reverse, 17 hcp
        ("1H P 1S P", hand("AJ32", "32", "AKJ32", "32"), "2C"), // new suit rebid
    ];
    for (calls, h, expected) in cases {
        let a = auction(Seat::North, Vulnerability::None, calls);
        let candidates = engine.candidates(&a, a.next_seat());
        let priority = |call: &str| {
            let call: bridge_core::Call = call.parse().unwrap();
            candidates
                .iter()
                .find(|(c, k, _)| *c == call && k.satisfies(h))
                .map(|(_, _, p)| *p)
        };
        let pass = priority("P").expect("the pass is always available");
        let bid = priority(expected).expect("the hand satisfies the natural bid");
        assert!(bid > pass, "{calls}: {expected} {bid} vs pass {pass}");
    }
    // The limited first pass keeps its 0.4 priority.
    let a = auction(Seat::North, Vulnerability::None, "1H P");
    let pass = engine
        .candidates(&a, Seat::South)
        .into_iter()
        .find(|(c, _, _)| *c == bridge_core::Call::Pass)
        .unwrap();
    assert_eq!(pass.2, 40);
}
