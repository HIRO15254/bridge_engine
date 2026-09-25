//! One test per rule of `NaturalInference::infer`'s ordered table (06-system.md §8.3): each test
//! builds a `CallContext` for a realistic auction, checks that the expected rule (and only that
//! rule) fires, and checks `satisfies` on a hand that should match the constraint and one that
//! should not.

mod common;

use bridge_core::{Seat, Vulnerability};
use bridge_system::natural::{CallContext, Inference, NaturalInference, classify};
use common::{auction, hand};

fn infer(auction_str: &str, index: usize, owner: Seat) -> (CallContext, Inference) {
    let a = auction(Seat::North, Vulnerability::None, auction_str);
    let ctx = classify(&a, index, owner);
    let inf = NaturalInference::default().infer(&ctx);
    (ctx, inf)
}

/// Three low, worthless cards: a convenient filler suit.
const LOW: &str = "432";

#[test]
fn rule_open_1major() {
    let (_, inf) = infer("1S", 0, Seat::North);
    assert_eq!(inf.rule, "open_1M");
    let good = hand("32", "K32", LOW, "AKQJ2"); // 5 spades, 13 hcp
    let too_short = hand("AKQ2", "32", LOW, "AKQ2"); // only 4 spades
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_short));
}

#[test]
fn rule_open_1m() {
    let (_, inf) = infer("1C", 0, Seat::North);
    assert_eq!(inf.rule, "open_1m");
    let good = hand("AKQJ2", "K32", LOW, "32"); // 5 clubs, 13 hcp
    let too_weak = hand(LOW, LOW, LOW, "5432"); // 0 hcp
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_weak));
}

#[test]
fn rule_open_nt() {
    let (_, inf) = infer("1NT", 0, Seat::North);
    assert_eq!(inf.rule, "open_nt");
    // 4-3-4-2, 16 hcp, balanced.
    let good = hand("AKQJ", "432", "AQ32", "32");
    // 17 hcp but a 6-card suit: not balanced.
    let unbalanced = hand("AKQJ32", "AK2", "32", "32");
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(unbalanced));
}

#[test]
fn rule_open_weak2() {
    let (_, inf) = infer("2H", 0, Seat::North);
    assert_eq!(inf.rule, "open_weak2");
    let good = hand("32", LOW, "AJT832", "32"); // 6 hearts, 5 hcp
    let too_strong = hand("A32", "A32", "AKQJ32", "2"); // 18 hcp
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_strong));
}

#[test]
fn rule_open_2c() {
    let (_, inf) = infer("2C", 0, Seat::North);
    assert_eq!(inf.rule, "open_2c");
    let good = hand("AKQ2", "AKQ2", "AK2", "A2"); // 29 hcp
    let too_weak = hand("AKQ2", LOW, LOW, LOW); // 9 hcp
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_weak));
}

#[test]
fn rule_open_preempt() {
    let (_, inf) = infer("3H", 0, Seat::North);
    assert_eq!(inf.rule, "open_preempt");
    let good = hand("32", "32", "AJT9832", "32"); // 7 hearts, 5 hcp
    let too_short = hand("32", LOW, "AJT98", LOW); // only 5 hearts
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_short));
}

#[test]
fn rule_open_pass() {
    let (_, inf) = infer("P", 0, Seat::North);
    assert_eq!(inf.rule, "open_pass");
    let good = hand(LOW, LOW, LOW, "5432"); // 0 hcp
    let too_strong = hand("AKQ2", "AK3", LOW, LOW); // 16 hcp
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_strong));
}

#[test]
fn rule_overcall() {
    let (_, inf) = infer("1S 2H", 1, Seat::East);
    assert_eq!(inf.rule, "overcall");
    let good = hand(LOW, "32", "AKQJ32", "32"); // 6 hearts, 10 hcp
    let too_short = hand(LOW, "432", "AJ32", "AK2"); // only 4 hearts
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_short));
}

#[test]
fn rule_jump_overcall() {
    let (_, inf) = infer("1S 3H", 1, Seat::East);
    assert_eq!(inf.rule, "jump_overcall");
    let good = hand(LOW, "32", "AJT832", "32"); // 6 hearts, 5 hcp
    let too_strong = hand("A32", "A32", "AKQJ32", "2"); // 18 hcp
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_strong));
}

#[test]
fn rule_nt_overcall() {
    let (_, inf) = infer("1S 1NT", 1, Seat::East);
    assert_eq!(inf.rule, "nt_overcall");
    // 4-3-4-2, 18 hcp, balanced, spade ace as a stopper.
    let good = hand("AKQ2", LOW, "AJ32", "A2");
    // Same shape and strength but no spade stopper at all.
    let no_stopper = hand("AKQ2", LOW, "AQJ2", "32");
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(no_stopper));
}

#[test]
fn rule_takeout_x() {
    let (_, inf) = infer("1S X", 1, Seat::East);
    assert_eq!(inf.rule, "takeout_x");
    // 14 hcp, 2-card spades (their suit), two other suits with 4+.
    let good = hand("AK32", "AK32", LOW, "32");
    let too_weak = hand(LOW, "5432", "5432", "32"); // 0 hcp
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_weak));
}

#[test]
fn rule_penalty_x() {
    let (_, inf) = infer("1S 4H X", 2, Seat::South);
    assert_eq!(inf.rule, "penalty_x");
    let good = hand("32", "432", "AKQ2", "AK32"); // 4 hearts, 16 hcp
    let too_short = hand("A32", "AK32", "32", "5432"); // only 2 hearts
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_short));
}

#[test]
fn rule_negative_x() {
    let (_, inf) = infer("1D 1S X", 2, Seat::South);
    assert_eq!(inf.rule, "negative_x");
    let good = hand("65432", "32", "AQ32", "32"); // 4 hearts (the unbid major), 6 hcp
    let no_major = hand("65432", "43", "32", "AJ32"); // 4 *spades* is their suit, not an unbid major
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(no_major));
}

#[test]
fn rule_raise() {
    let (_, inf) = infer("1S P 2S", 2, Seat::South);
    assert_eq!(inf.rule, "raise");
    let good = hand(LOW, "K32", "J432", "Q43"); // 3-card support, 6 hcp
    let too_short = hand("6432", "K432", LOW, "Q4"); // only 2 spades
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_short));
}

#[test]
fn rule_new_suit_resp_1() {
    let (_, inf) = infer("1C P 1H", 2, Seat::South);
    assert_eq!(inf.rule, "new_suit_resp_1");
    let good = hand(LOW, "K32", "AJ32", LOW); // 4 hearts, 8 hcp
    let too_short = hand(LOW, "K32", "AJ2", "5432"); // only 3 hearts
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_short));
}

#[test]
fn rule_new_suit_resp_2() {
    // 1S (a suit that outranks clubs) - P - 2C is a plain (non-jump) 2-level new suit.
    let (_, inf) = infer("1S P 2C", 2, Seat::South);
    assert_eq!(inf.rule, "new_suit_resp_2");
    let good = hand("AKQJ2", "32", LOW, LOW); // 5 clubs, 10 hcp
    let too_weak = hand("AJ432", "32", LOW, LOW); // 5 clubs but only 5 hcp
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_weak));
}

#[test]
fn rule_resp_nt() {
    let (_, inf) = infer("1S P 1NT", 2, Seat::South);
    assert_eq!(inf.rule, "resp_nt");
    let good = hand(LOW, LOW, LOW, "AQJ2"); // 7 hcp (within 6-10)
    let too_strong = hand("AK32", "32", LOW, "AQ32"); // 13 hcp
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_strong));
}

#[test]
fn rule_rebid_own() {
    let (_, inf) = infer("1S P 2H P 2S", 4, Seat::North);
    assert_eq!(inf.rule, "rebid_own");
    let good = hand(LOW, "K32", "3", "AKQJ32"); // 6 spades, 13 hcp
    let too_short = hand(LOW, "K32", "32", "AKQJ2"); // only 5 spades
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_short));
}

#[test]
fn rule_reverse() {
    let (_, inf) = infer("1C P 1S P 2H", 4, Seat::North);
    assert_eq!(inf.rule, "reverse");
    // 5+ clubs, 4+ hearts, 17 hcp.
    let good = hand("AKQ32", LOW, "AKJ32", "");
    // Same shape but only 9 hcp.
    let too_weak = hand("KQ432", LOW, "KJ432", "");
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_weak));
    // Only 2 clubs (opener's real first suit) but 5 spades (responder's suit, already in
    // `our_suits`) and 4 hearts, 17 hcp: must NOT satisfy, even though a naive check that ORs
    // over every suit our side has bid so far would let responder's 5-card spade suit qualify.
    let short_in_first_suit = hand("32", "32", "AK32", "AKQJT");
    assert!(!inf.constraint.satisfies(short_in_first_suit));
}

#[test]
fn rule_cue() {
    let (_, inf) = infer("P 1D 1S P 2D", 4, Seat::North);
    assert_eq!(inf.rule, "cue");
    let good = hand("AKQJ", LOW, LOW, LOW); // 10 hcp (advance.cue default)
    let too_weak = hand(LOW, LOW, LOW, "5432"); // 0 hcp
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_weak));
}

#[test]
fn rule_pass_forcing() {
    let a = auction(Seat::North, Vulnerability::None, "1S P 2S P");
    let mut ctx = classify(&a, 3, Seat::West);
    ctx.forcing_situation = true;
    let inf = NaturalInference::default().infer(&ctx);
    assert_eq!(inf.rule, "pass_forcing");
    // Near-unsatisfiable: only a 0-hcp hand matches.
    let zero = hand(LOW, LOW, LOW, "5432");
    let any_points = hand("A32", LOW, LOW, "5432");
    assert!(inf.constraint.satisfies(zero));
    assert!(!inf.constraint.satisfies(any_points));
}

#[test]
fn rule_pass_default() {
    // Responder passing with a minimum hand: no earlier rule matches a plain pass by a
    // non-opener, non-forcing-situation role.
    let (ctx, inf) = infer("1S P 2S P P", 4, Seat::South);
    assert_eq!(ctx.call, bridge_core::Call::Pass);
    assert_eq!(inf.rule, "pass_default");
    let good = hand(LOW, LOW, LOW, "5432"); // 0 hcp
    let too_strong = hand("AKQ2", "32", LOW, "5432"); // 9 hcp, above the response threshold
    assert!(inf.constraint.satisfies(good));
    assert!(!inf.constraint.satisfies(too_strong));
}

#[test]
fn rule_fallback() {
    // Opener's 1-level notrump *rebid* (as opposed to opening 1NT) matches nothing in the v1
    // table (06-system.md §8.3 does not wire `rebid.nt_1`/`nt_2` into any rule yet), so it falls
    // through to `fallback`.
    let (_, inf) = infer("1C P 1H P 1NT", 4, Seat::North);
    assert_eq!(inf.rule, "fallback");
    // `ANY`: every hand satisfies it.
    assert!(inf.constraint.satisfies(hand(LOW, LOW, LOW, "5432")));
    assert!(inf.constraint.satisfies(hand("AKQJ", "AKQJ", "AKQJ", "A")));
}
