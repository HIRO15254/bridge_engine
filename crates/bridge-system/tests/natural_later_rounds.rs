//! Regression tests for the later-round limits of `NaturalInference` (06-system.md §8.3 and
//! §8.6; phase 4 lane D3). Each test names generated auctions where the natural engine used to
//! act at a position SAYC passes, with a call that is not sound bridge there, and checks the
//! rule that fires now (or that none does, so the natural policy passes).

mod common;

use std::ops::RangeInclusive;

use bridge_constraint::{Atom, HandConstraint};
use bridge_core::{Seat, Vulnerability};
use bridge_system::natural::{CallContext, Inference, NaturalInference, classify};
use common::{auction, hand};

/// Classifies and infers the *last* call of `calls` (dealer North) for the seat that made it.
fn last(calls: &str) -> (CallContext, Inference) {
    let a = auction(Seat::North, Vulnerability::None, calls);
    let index = a.len() - 1;
    let ctx = classify(&a, index, a.seat_at(index));
    let inf = NaturalInference::default().infer(&ctx);
    (ctx, inf)
}

/// Like [`last`], with partner's constraint filled in the way `interpret` does, so the level
/// floor (§8.6) applies.
fn last_with_partner(calls: &str, partner_hcp: RangeInclusive<u8>) -> Inference {
    let a = auction(Seat::North, Vulnerability::None, calls);
    let index = a.len() - 1;
    let mut ctx = classify(&a, index, a.seat_at(index));
    ctx.partner_constraint = Some(HandConstraint::Atom(Atom::ANY.with_hcp(partner_hcp)));
    NaturalInference::default().infer(&ctx)
}

fn min_hcp(inf: &Inference) -> u8 {
    *inf.constraint.hcp_range().start()
}

// --- first entries after the opponents' exchange (MAX_ENTRY_LEVEL_AFTER_EXCHANGE) -------------

#[test]
fn no_natural_entry_over_their_game_after_their_exchange() {
    for calls in [
        "1S P 3S P 4S P P 5H", // balancing over their game
        "1S P 4S 5H",          // direct, over their game raise
        "1H P 3NT 4S",         // over their 3NT
        "1H P 2H P P 5C",      // five-level first entry below their game
    ] {
        let (_, inf) = last(calls);
        assert!(
            !matches!(inf.rule, "overcall" | "jump_overcall"),
            "{calls}: {}",
            inf.rule
        );
    }
}

#[test]
fn four_level_entry_below_their_game_needs_six_cards_and_opening_values() {
    let six_12 = hand("432", "A32", "AKJ432", "2"); // 6 hearts, 12 hcp
    let six_10 = hand("432", "Q32", "AKJ432", "2"); // 6 hearts, 10 hcp
    let six_8 = hand("432", "432", "AKJ432", "2"); // 6 hearts, 8 hcp
    let five_15 = hand("K32", "A32", "AKJ32", "32"); // 5 hearts, 15 hcp

    // Direct seat over their limit raise.
    let (_, inf) = last("1S P 3S 4H");
    assert_eq!(inf.rule, "overcall");
    assert!(inf.constraint.satisfies(six_12));
    assert!(!inf.constraint.satisfies(six_10));
    assert!(!inf.constraint.satisfies(five_15));

    // The balancing seat: a king less.
    let (_, inf) = last("1S P 3S P P 4H");
    assert_eq!(inf.rule, "overcall");
    assert!(inf.constraint.satisfies(six_10));
    assert!(!inf.constraint.satisfies(six_8));
    assert!(!inf.constraint.satisfies(five_15));
}

#[test]
fn ordinary_overcalls_are_unchanged() {
    assert_eq!(last("1S 2H").1.rule, "overcall");
    assert_eq!(last("1S 3H").1.rule, "jump_overcall");
    // After their raise, up to the three level: the ordinary five-card overcall.
    let (_, inf) = last("1S P 2S 3H");
    assert_eq!(inf.rule, "overcall");
    assert!(inf.constraint.satisfies(hand("K32", "A32", "AKJ32", "32")));
}

#[test]
fn weak_jump_overcall_stops_at_the_three_level() {
    // A single jump to the four level (over a weak two, or over their 2NT) is not a weak-two
    // hand.
    for calls in ["2H 4C", "1H P 2NT 4C"] {
        assert_ne!(last(calls).1.rule, "jump_overcall", "{calls}");
    }
    assert_eq!(last("2H P P 3S").1.rule, "jump_overcall");
}

// --- bids past partner's game (SLAM_LEVEL) -----------------------------------------------------

#[test]
fn a_bid_past_partners_game_needs_slam_values() {
    // The weak-two opener pulling partner's 3NT, and opener bidding on over partner's game
    // raise: the six level's combined target (31) less partner's minimum (12).
    for calls in ["2S P 2NT P 3S P 3NT P 4S", "1H P 1S P 2S P 4S P 5C"] {
        let inf = last_with_partner(calls, 12..=37);
        assert!(
            min_hcp(&inf) >= 19,
            "{calls}: {} {}",
            inf.rule,
            min_hcp(&inf)
        );
    }
    // Once the right-hand opponent has bid or doubled, the same bid is competitive: the
    // ordinary level floor (five level: 26 - 12).
    let inf = last_with_partner("1H P 1S P 2S P 4S X 5C", 12..=37);
    assert_eq!(min_hcp(&inf), 14, "{}", inf.rule);
}
