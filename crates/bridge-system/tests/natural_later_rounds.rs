//! Regression tests for the later-round limits of `NaturalInference` (06-system.md §8.3 and
//! §8.6; phase 4 lane D3). Each test names generated auctions where the natural engine used to
//! act at a position SAYC passes, with a call that is not sound bridge there, and checks the
//! rule that fires now (or that none does, so the natural policy passes).

mod common;

use std::ops::RangeInclusive;

use bridge_constraint::{Atom, HandConstraint};
use bridge_core::{Hand, Seat, Suit, Vulnerability};
use bridge_system::natural::{
    CallContext, CallKind, DoubleKind, Inference, NaturalInference, Role, classify,
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

/// Like [`last`], with partner's constraint filled in the way `interpret` does, so the level
/// floor (§8.6) applies.
fn last_with_partner(calls: &str, partner_hcp: RangeInclusive<u8>) -> Inference {
    let a = auction(Seat::North, Vulnerability::None, calls);
    let index = a.len() - 1;
    let mut ctx = classify(&a, index, a.seat_at(index));
    ctx.partner_constraint = Some(HandConstraint::Atom(Atom::ANY.with_hcp(partner_hcp)));
    NaturalInference::default().infer(&ctx)
}

/// Like [`last_with_partner`], with partner's whole constraint given.
fn last_with_partner_constraint(calls: &str, partner: Atom) -> Inference {
    let a = auction(Seat::North, Vulnerability::None, calls);
    let index = a.len() - 1;
    let mut ctx = classify(&a, index, a.seat_at(index));
    ctx.partner_constraint = Some(HandConstraint::Atom(partner));
    NaturalInference::default().infer(&ctx)
}

/// Like [`last_with_partner`], with partner's last call forcing (`forcing_situation`).
fn last_with_partner_forcing(calls: &str, partner_hcp: RangeInclusive<u8>) -> Inference {
    let a = auction(Seat::North, Vulnerability::None, calls);
    let index = a.len() - 1;
    let mut ctx = classify(&a, index, a.seat_at(index));
    ctx.partner_constraint = Some(HandConstraint::Atom(Atom::ANY.with_hcp(partner_hcp)));
    ctx.forcing_situation = true;
    NaturalInference::default().infer(&ctx)
}

fn min_hcp(inf: &Inference) -> u8 {
    *inf.constraint.hcp_range().start()
}

// --- first entries after the opponents' exchange (MAX_ENTRY_LEVEL_AFTER_EXCHANGE) -------------

#[test]
fn no_natural_entry_at_the_five_level_after_their_exchange() {
    for calls in [
        "1S P 3S P 4S P P 5H", // balancing over their game
        "1S P 4S 5H",          // direct, over their game raise
        "1D P 4D 5C",          // five-level first entry below their game
        "1D P 4D P P 5C",      // the same in the balancing seat
        "1H P 2NT P 4H 5C",    // a single jump is not described either
    ] {
        let (ctx, inf) = last(calls);
        assert_eq!(ctx.level, 5, "{calls}");
        assert!(ctx.their_bids >= 2, "{calls}");
        assert_eq!(inf.rule, "fallback", "{calls}");
    }
}

/// A hand with `long` in `suit` and the other three holdings, in suit order, in the other
/// suits.
fn hand_with(suit: Suit, long: &str, others: [&str; 3]) -> Hand {
    let mut others = others.into_iter();
    let mut holding = |s: Suit| {
        if s == suit {
            long
        } else {
            others.next().unwrap()
        }
    };
    hand(
        holding(Suit::Clubs),
        holding(Suit::Diamonds),
        holding(Suit::Hearts),
        holding(Suit::Spades),
    )
}

/// Asserts that the last call of `calls` fires `rule` with the four-level entry's description
/// (`four_level_entry`): 6+ cards in the bid suit and 12-16 HCP, or 9-16 when `balancing`.
fn assert_four_level_entry(calls: &str, rule: &str, balancing: bool) {
    let (ctx, inf) = last(calls);
    let suit = ctx.call.bid().unwrap().strain().suit().unwrap();
    assert_eq!(inf.rule, rule, "{calls}");
    assert_eq!(inf.constraint.suit_len(suit), 6..=13, "{calls}");
    let from = if balancing { 9 } else { 12 };
    assert_eq!(inf.constraint.hcp_range(), from..=16, "{calls}");
    // Six cards (8 hcp in the suit) with 12, 11, 9, 8, 16 and 17 hcp; five cards with 15.
    let six = |others| hand_with(suit, "AKJ432", others);
    let accepts = |h: Hand| inf.constraint.satisfies(h);
    assert!(accepts(six(["432", "A32", "2"])), "{calls}: 12");
    assert_eq!(accepts(six(["432", "Q32", "J"])), balancing, "{calls}: 11");
    assert_eq!(accepts(six(["432", "J32", "2"])), balancing, "{calls}: 9");
    assert!(!accepts(six(["432", "432", "2"])), "{calls}: 8");
    assert!(accepts(six(["A32", "A32", "2"])), "{calls}: 16");
    assert!(!accepts(six(["A32", "A32", "J"])), "{calls}: 17");
    let five_15 = hand_with(suit, "AKJ32", ["K32", "A32", "32"]);
    assert!(!accepts(five_15), "{calls}: five cards");
}

#[test]
fn four_level_entry_after_their_exchange_needs_six_cards_and_opening_values() {
    // Direct seat, below their game: over their limit raise, over their 2NT.
    assert_four_level_entry("1S P 3S 4H", "overcall", false);
    assert_four_level_entry("1H P 3H 4D", "overcall", false);
    // Over their game: four of a major over four of a major, any suit over 3NT.
    assert_four_level_entry("1H P 4H 4S", "overcall", false);
    assert_four_level_entry("1H P 3NT 4S", "overcall", false);
    assert_four_level_entry("1H P 3NT 4C", "overcall", false);
    // A single jump to the four level after the exchange.
    assert_four_level_entry("1H P 2NT 4C", "jump_overcall", false);
    // The balancing seat below their game: a king less.
    assert_four_level_entry("1S P 3S P P 4H", "overcall", true);
    assert_four_level_entry("1H P 3H P P 4D", "overcall", true);
    // The pass-out seat over their game is not a balance: opening values.
    assert_four_level_entry("1H P 4H P P 4S", "overcall", false);
    assert_four_level_entry("1NT P 2NT P 3NT P P 4H", "overcall", false);
}

#[test]
fn single_jump_to_the_four_level_shows_six_cards_and_opening_values() {
    for calls in ["2S 4H", "3C 4H", "3D 4S", "2H 4C"] {
        let (ctx, _) = last(calls);
        assert_eq!(ctx.their_bids, 1, "{calls}");
        assert_four_level_entry(calls, "jump_overcall", false);
    }
    assert_four_level_entry("2S P P 4H", "jump_overcall", true);
    // A single jump to the five level is not described.
    assert_eq!(last("3S 5C").1.rule, "fallback");
}

#[test]
fn ordinary_overcalls_are_unchanged() {
    assert_eq!(last("1S 2H").1.rule, "overcall");
    assert_eq!(last("1S 3H").1.rule, "jump_overcall");
    // After their raise, up to the three level: the ordinary five-card overcall.
    let (_, inf) = last("1S P 2S 3H");
    assert_eq!(inf.rule, "overcall");
    assert!(inf.constraint.satisfies(hand("K32", "A32", "AKJ32", "32")));
    // A non-jump four-level overcall of their single bid keeps the ordinary range.
    let (ctx, inf) = last("3S 4H");
    assert_eq!(ctx.their_bids, 1);
    assert_eq!(inf.rule, "overcall");
    assert_eq!(inf.constraint.hcp_range(), 10..=16);
    assert_eq!(inf.constraint.suit_len(Suit::Hearts), 5..=13);
    // After their exchange, a three-level entry (MAX_ENTRY_LEVEL_AFTER_EXCHANGE) keeps the
    // ordinary range; the four level is the four-level entry's (tested above).
    let (ctx, inf) = last("1H P 2H 3C");
    assert_eq!(ctx.their_bids, 2);
    assert_eq!(inf.rule, "overcall");
    assert_eq!(inf.constraint.hcp_range(), 10..=16);
    assert_eq!(inf.constraint.suit_len(Suit::Clubs), 5..=13);
    // The weak jump overcall up to the three level, also in the balancing seat.
    let (_, inf) = last("2H P P 3S");
    assert_eq!(inf.rule, "jump_overcall");
    assert_eq!(inf.constraint.suit_len(Suit::Spades), 6..=13);
    let (_, inf) = last("1S 3H");
    assert_eq!(inf.constraint.hcp_range(), 5..=10);
}

// --- bids past partner's game (SLAM_LEVEL, overrides_partners_game) ---------------------------

#[test]
fn overriding_partners_game_needs_slam_values() {
    // The weak-two opener pulling partner's 3NT after rebidding the spades, and opener bidding
    // on over partner's game raise: the six level's combined target (31) less partner's minimum
    // (12).
    for calls in ["2S P 2NT P 3S P 3NT P 4S", "1H P 1S P 2S P 4S P 5C"] {
        let inf = last_with_partner(calls, 12..=37);
        assert_eq!(min_hcp(&inf), 19, "{calls}: {}", inf.rule);
    }
    // Opener pulling 3NT to four of the suit already rebid: nothing partner did not know.
    let inf = last_with_partner("1H P 1S P 2H P 3NT P 4H", 10..=37);
    assert_eq!(inf.rule, "rebid_own");
    assert_eq!(min_hcp(&inf), 21);
    // Below game over partner's 3NT (a slam try, not a correction): the slam floor too.
    let inf = last_with_partner("1D P 3NT P 4D", 13..=15);
    assert_eq!(inf.rule, "rebid_own");
    assert_eq!(min_hcp(&inf), 18);
    // Once the right-hand opponent has bid or doubled, the same bid is competitive: the
    // ordinary level floor (five level: 26 - 12).
    for calls in ["1H P 1S P 2S P 4S X 5C", "1H P 1S P 2S P 4S 5D 5H"] {
        let inf = last_with_partner(calls, 12..=37);
        assert_eq!(min_hcp(&inf), 14, "{calls}: {}", inf.rule);
    }
    // Over a forcing game-level call partner has not chosen the contract: the ordinary floor.
    let inf = last_with_partner_forcing("1H P 1S P 2S P 4S P 5C", 12..=37);
    assert_eq!(min_hcp(&inf), 14, "{}", inf.rule);
}

#[test]
fn corrections_of_partners_3nt_keep_the_ordinary_floor() {
    // 1M-P-3NT-P-4M: SAYC's 3NT promises two hearts, so opener's sixth heart is news. The
    // ordinary four-level floor (22 - 15 = 7) leaves the rule's own 12-21 (it was 16+ under the
    // slam floor).
    let inf = last_with_partner("1H P 3NT P 4H", 15..=17);
    assert_eq!(inf.rule, "rebid_own");
    assert_eq!(inf.constraint.hcp_range(), 12..=21);
    let six_12 = hand("Q2", "Q32", "AKJ432", "32"); // 6 hearts, 12 hcp
    let five_12 = hand("Q32", "Q32", "AKJ32", "32"); // 5 hearts, 12 hcp
    assert!(inf.constraint.satisfies(six_12));
    assert!(!inf.constraint.satisfies(five_12));
    // A 1NT opener choosing the suit after a transfer and partner's 3NT: the notrump range and
    // an eight-card fit with partner's five spades (it was 21-21 with six spades under the slam
    // floor).
    let five_spades = Atom::ANY
        .with_hcp(10..=15)
        .with_suit_len(Suit::Spades, 5..=13);
    let inf = last_with_partner_constraint("1NT P 2H P 2S P 3NT P 4S", five_spades);
    assert_eq!(inf.rule, "rebid_own");
    assert_eq!(inf.constraint.hcp_range(), 15..=17);
    assert_eq!(inf.constraint.suit_len(Suit::Spades), 3..=5); // balanced
    assert!(inf.constraint.satisfies(hand("K32", "AQ2", "KJ32", "Q32"))); // 15, three spades
    assert!(!inf.constraint.satisfies(hand("K432", "AQ2", "KJ32", "Q2"))); // two spades
    // After Stayman partner's 3NT denies opener's major: no eight-card fit, no hand.
    let no_spades = Atom::ANY
        .with_hcp(10..=15)
        .with_suit_len(Suit::Spades, 0..=3);
    let inf = last_with_partner_constraint("1NT P 2C P 2S P 3NT P 4S", no_spades);
    assert_eq!(inf.rule, "rebid_own");
    assert!(!inf.constraint.is_satisfiable());
    // Preference to partner's suit: opener's raise, with the ordinary floor (22 - 10) and an
    // eight-card fit with partner's five hearts.
    let five_hearts = Atom::ANY
        .with_hcp(10..=37)
        .with_suit_len(Suit::Hearts, 5..=13);
    let inf = last_with_partner_constraint("1S P 2H P 2NT P 3NT P 4H", five_hearts);
    assert_eq!(inf.rule, "raise");
    assert_eq!(inf.constraint.hcp_range(), 12..=15);
    assert_eq!(inf.constraint.suit_len(Suit::Hearts), 3..=13);
    // Partner's one-level response showed four spades: a raise needs four.
    let four_spades = Atom::ANY
        .with_hcp(13..=37)
        .with_suit_len(Suit::Spades, 4..=13);
    let inf = last_with_partner_constraint("1C P 1S P 1NT P 3NT P 4S", four_spades);
    assert_eq!(inf.rule, "raise");
    assert_eq!(inf.constraint.suit_len(Suit::Spades), 4..=13);
    assert!(!inf.constraint.satisfies(hand("AQ32", "QJ2", "A72", "Q93"))); // three spades
    // Five of a minor is the cheapest game bid in the suit, not a jump rebid: the opening's
    // range with the ordinary five-level floor (26 - 13; the slam floor would have made it 18),
    // six diamonds, and a hand unsuited to notrump (a singleton or a void).
    let inf = last_with_partner("1D P 3NT P 5D", 13..=15);
    assert_eq!(inf.rule, "rebid_own");
    assert_eq!(inf.constraint.hcp_range(), 13..=21);
    assert!(inf.constraint.satisfies(hand("2", "AKJ432", "A32", "Q32"))); // 14, singleton
    assert!(!inf.constraint.satisfies(hand("32", "AKJ432", "A3", "Q32"))); // 14, no singleton
    // A notrump opener has no such hand: it never pulls 3NT to five of a minor.
    let inf = last_with_partner("1NT P 3C P 3D P 3NT P 5D", 10..=37);
    assert_eq!(inf.rule, "rebid_own");
    assert!(!inf.constraint.is_satisfiable());
}

#[test]
fn bids_over_partners_3nt_that_are_not_corrections_keep_the_slam_floor() {
    // The opponents' suit (partner's cue bid, which the natural rules read as a suit partner
    // bid first) is not a correction: the slam floor (31 - 12).
    let inf = last_with_partner("1H 1S X P 1NT P 2S P 2NT P 3NT P 4S", 12..=37);
    assert_eq!(inf.rule, "raise");
    assert_eq!(min_hcp(&inf), 19);
    // A suit nobody on our side has bid: the slam floor (31 - 10) over the jump shift's own 19.
    let inf = last_with_partner("1H P 3NT P 5C", 10..=37);
    assert_eq!(inf.rule, "rebid_new_suit");
    assert_eq!(min_hcp(&inf), 21);
    // A weak two or a preempt pulling partner's to-play 3NT (SAYC: 15+ over a weak two, 14+
    // over a preempt) to the suit it opened: the opening promised the long suit already. The
    // slam floor (31 - partner's minimum) is above the opening's range, so no hand bids it.
    for (calls, partner_min) in [
        ("2S P 3NT P 4S", 15),
        ("2H P 3NT P 4H", 15),
        ("3H P 3NT P 4H", 14),
    ] {
        let inf = last_with_partner(calls, partner_min..=37);
        assert_eq!(inf.rule, "rebid_own", "{calls}");
        assert_eq!(min_hcp(&inf), 31 - partner_min, "{calls}");
        assert!(!inf.constraint.is_satisfiable(), "{calls}");
    }
}

#[test]
fn answers_and_follow_ups_of_a_slam_ask_keep_the_ordinary_floor() {
    // Responder's answer to opener's 4NT: the ordinary five-level floor (26 - 12), not the slam
    // floor (31 - 12 = 19). The raise rule's own minimum at the game level is 13.
    let inf = last_with_partner("1H P 3H P 4NT P 5H", 12..=37);
    assert_eq!(inf.rule, "raise");
    assert_eq!(min_hcp(&inf), 14);
    // The asker signing off after the answer (partner's 5D is a game-level bid): the ordinary
    // five-level floor (26 - 10) under the re-raise's own 16-18, where the slam floor (21) left
    // no hand at all.
    let inf = last_with_partner("1H P 3H P 4NT P 5D P 5H", 10..=12);
    assert_eq!(inf.rule, "rebid_own");
    assert_eq!(inf.constraint.hcp_range(), 16..=18);
}

// --- opener's notrump rebid (is_openers_rebid) -------------------------------------------------

#[test]
fn notrump_range_rebid_is_openers_rebid_only() {
    for calls in [
        "1D P 1S P 1NT P 2S P 2NT", // the range is already shown
        "1C P 1D 1S P 2S P P 2NT",  // opener passed at the rebid
        "1C 1D P 1S 1NT",           // partner has not responded
        "1C 1NT 2H P 2NT",          // over their notrump
        "1C 1NT 2D X 2NT",
    ] {
        assert_ne!(last(calls).1.rule, "rebid_nt", "{calls}");
    }
    // The rebid itself, also over an overcall and partner's negative double.
    for calls in ["1C P 1H P 1NT", "1C 1S X P 1NT", "1D P 1S 2C 2NT"] {
        assert_eq!(last(calls).1.rule, "rebid_nt", "{calls}");
    }
    let (_, inf) = last("1C 1S X P 1NT");
    assert_eq!(inf.constraint.hcp_range(), 12..=14);
}

// --- responder's negative double (responders_first_turn_over_overcall) ------------------------

#[test]
fn negative_double_is_responders_first_turn_only() {
    for calls in ["1C 1H X", "P P 1D 2C X", "1H 2H X"] {
        let (ctx, inf) = last(calls);
        assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Negative), "{calls}");
        assert_eq!(inf.rule, "negative_x", "{calls}");
    }
    // Responder's later doubles are classified the same way but are not negative doubles.
    for calls in [
        "1C 1H X 2D P P X", // a second double
        "1H 1S P 2S P P X", // after a first pass
        "2D P P 2H P P X",  // after passing a weak two
        "1D 1H 1S 2H P P X",
    ] {
        let (ctx, inf) = last(calls);
        assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Negative), "{calls}");
        assert_ne!(inf.rule, "negative_x", "{calls}");
    }
}

#[test]
fn responders_later_double_after_own_call_shows_extra_values() {
    // After responder's own call (a negative double, a new suit): a king more than a negative
    // double at that level (6 + 2 x (level - 1)).
    for (calls, min) in [
        ("1C 1H X 2D P P X", 11),  // a second double, over a two-level bid
        ("1D 1H 1S 2H P P X", 11), // after a one-level response
        ("1C P 1H 1S P P X", 9),   // over a one-level bid
    ] {
        let (ctx, inf) = last(calls);
        assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Negative), "{calls}");
        assert!(ctx.owner_acted, "{calls}");
        assert_eq!(inf.rule, "competitive_x", "{calls}");
        assert_eq!(inf.constraint.hcp_range(), min..=37, "{calls}");
    }
    let (_, inf) = last("1C 1H X 2D P P X");
    assert!(inf.constraint.satisfies(hand("K32", "A32", "Q432", "Q32"))); // 11 hcp
    assert!(!inf.constraint.satisfies(hand("K32", "A32", "J432", "Q32"))); // 10 hcp
    // After responder's first pass no rule describes the double.
    for calls in ["1H 1S P 2S P P X", "2D P P 2H P P X"] {
        let (ctx, inf) = last(calls);
        assert!(!ctx.owner_acted, "{calls}");
        assert_eq!(inf.rule, "fallback", "{calls}");
    }
}

// --- a defender's second takeout double (SECOND_TAKEOUT_DOUBLE_EXTRA) --------------------------

#[test]
fn defenders_second_takeout_double_shows_extra_values() {
    // First takeout doubles keep their minimum.
    assert_eq!(min_hcp(&last("1S X").1), 12);
    assert_eq!(min_hcp(&last("1S P P X").1), 9);
    // The overcaller doubling after the overcall, the balancer doubling again: 15+, 12+ in
    // the balancing seat.
    let (ctx, inf) = last("1C 1S X P 2H X");
    assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Takeout));
    assert_eq!(inf.rule, "takeout_x");
    assert_eq!(min_hcp(&inf), 15);
    let (_, inf) = last("1S P P X 2S P P X");
    assert_eq!(inf.rule, "takeout_x");
    assert_eq!(min_hcp(&inf), 12);
    // Opener's reopening double is not a defender's: a minimum opening.
    let (_, inf) = last("1D 1S P P X");
    assert_eq!(inf.rule, "takeout_x");
    assert_eq!(min_hcp(&inf), 12);
    // Nor is the advancer's after a forced answer to partner's takeout double: the ordinary
    // minimum.
    for calls in ["1H X P 1S P P 2H X", "1C X P 1H P P 2C X"] {
        let (ctx, inf) = last(calls);
        assert_eq!(ctx.role, Role::Advancer, "{calls}");
        assert!(ctx.owner_acted, "{calls}");
        assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Takeout), "{calls}");
        assert_eq!(inf.rule, "takeout_x", "{calls}");
        assert_eq!(min_hcp(&inf), 12, "{calls}");
    }
}
