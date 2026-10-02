//! Regression tests for the later-round limits of `NaturalInference` (06-system.md §8.3 and
//! §8.6; phase 4 lane D3). Each test names generated auctions where the natural engine used to
//! act at a position SAYC passes, with a call that is not sound bridge there, and checks the
//! rule that fires now (or that none does, so the natural policy passes).

mod common;

use std::ops::RangeInclusive;

use bridge_constraint::{Atom, HandConstraint};
use bridge_core::{Hand, Seat, Strain, Suit, Vulnerability};
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
        "1H P 2NT P 4H 5C",    // the cheapest club over their game (jump 0)
    ] {
        let (ctx, inf) = last(calls);
        assert_eq!(ctx.level, 5, "{calls}");
        assert!(ctx.their_bids >= 2, "{calls}");
        assert_eq!(inf.rule, "fallback", "{calls}");
    }
    // A single jump to the five level after their exchange is not described either.
    for calls in ["1H P 3H 5C", "1H P 3H P P 5C"] {
        let (ctx, inf) = last(calls);
        assert_eq!(ctx.level, 5, "{calls}");
        assert_eq!(ctx.their_bids, 2, "{calls}");
        assert!(
            matches!(ctx.kind, CallKind::Bid { jump: 1, .. }),
            "{calls}: {:?}",
            ctx.kind
        );
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
}

#[test]
fn entry_over_their_game_after_a_pass_is_not_described() {
    // A player who passed earlier, after their opening, where an overcall of the same suit was
    // available (1S over 1H, 3H over 2NT), enters over their game: six cards and opening values
    // are what that pass denied. No rule describes the entry, in the pass-out seat or in the
    // direct seat.
    for (calls, role) in [
        ("1H P 4H P P 4S", Role::Balancer),
        ("1NT P 2NT P 3NT P P 4H", Role::Balancer),
        ("1H P 3NT P P 4S", Role::Balancer),
        ("1H P 2H P 4H 4S", Role::Overcaller),
        ("1H P 2H P 3H P 4H 4S", Role::Overcaller),
        ("1NT P 2C P 2H P 3NT 4S", Role::Overcaller),
    ] {
        let (ctx, inf) = last(calls);
        assert_eq!(ctx.role, role, "{calls}");
        assert!(ctx.passed_after_opening, "{calls}");
        assert_eq!(inf.rule, "fallback", "{calls}");
    }
    // A first chance over their game keeps the four-level entry (tested above): no pass after
    // their opening. A pass before it (`P-(1H)-P-(4H)-4S`) is not keyed (a known limit).
    for calls in ["1H P 4H 4S", "1NT P 3NT 4H", "P 1H P 4H 4S"] {
        let (ctx, inf) = last(calls);
        assert!(!ctx.passed_after_opening, "{calls}");
        assert_eq!(inf.rule, "overcall", "{calls}");
    }
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
fn a_one_level_opener_pulling_3nt_to_its_rebid_suit_corrects() {
    // After a minimum rebid of the opened suit (SAYC: 12-15), the pull of partner's 3NT is a
    // choice of game: a seventh card, in the minimum's range. The ordinary floor (22 - 10,
    // 22 - 13) is below it (it was 21+ and 18+ under the slam floor).
    for (calls, partner_min) in [
        ("1H P 1S P 2H P 3NT P 4H", 10),
        ("1S P 2C P 2S P 3NT P 4S", 13),
    ] {
        let inf = last_with_partner(calls, partner_min..=37);
        let suit = inf_suit(calls);
        assert_eq!(inf.rule, "rebid_own", "{calls}");
        assert_eq!(inf.constraint.hcp_range(), 12..=15, "{calls}");
        let accepts = |long, others| inf.constraint.satisfies(hand_with(suit, long, others));
        assert!(
            accepts("AKJ5432", ["32", "K2", "Q2"]),
            "{calls}: seven, 13 hcp"
        );
        assert!(
            !accepts("AKJ543", ["2", "K32", "Q32"]),
            "{calls}: six and a singleton"
        );
        assert!(
            !accepts("AKJ543", ["32", "K32", "Q2"]),
            "{calls}: six, 6-3-2-2"
        );
        assert!(
            !accepts("AKQ5432", ["32", "K2", "A2"]),
            "{calls}: seven, 16 hcp"
        );
    }
    // After a jump rebid the pull shows the jump rebid's range (16-18).
    let inf = last_with_partner("1S P 1NT P 3S P 3NT P 4S", 8..=37);
    assert_eq!(inf.rule, "rebid_own");
    assert_eq!(inf.constraint.hcp_range(), 16..=18);
    // A weak two's rebid suit is not corrected this way: the slam floor (above).
    let inf = last_with_partner("2S P 2NT P 3S P 3NT P 4S", 12..=37);
    assert!(!inf.constraint.is_satisfiable());
}

#[test]
fn a_pull_after_an_unlimited_call_shows_openers_strongest_range() {
    // After a reverse, a jump shift or a 2NT rebid, a later non-jump rebid of the opened suit
    // does not limit opener to a minimum. The pull of partner's 3NT is still a correction (seven
    // cards, the ordinary floor) and shows the range of opener's strongest earlier call: a
    // reverse 17-21, a jump shift 19-21, a 2NT rebid 18-19.
    for (calls, partner_min, hcp) in [
        ("1D P 1S P 2H P 2NT P 3D P 3NT P 5D", 6, 20..=21), // reverse; floor 26 - 6
        ("1C P 1H P 2S P 2NT P 3C P 3NT P 5C", 8, 18..=21), // a jump reverse; floor 26 - 8
        ("1H P 1S P 3C P 3D P 3H P 3NT P 4H", 6, 19..=21),  // jump shift
        ("1H P 1S P 2NT P 3C P 3H P 3NT P 4H", 6, 18..=19), // 2NT rebid
        ("1D P 1H P 2NT P 3C P 3D P 3NT P 5D", 8, 18..=19),
    ] {
        let (ctx, _) = last(calls);
        assert!(!ctx.opener_other_calls.is_empty(), "{calls}");
        let inf = last_with_partner(calls, partner_min..=37);
        assert_eq!(inf.rule, "rebid_own", "{calls}");
        assert_eq!(inf.constraint.hcp_range(), hcp, "{calls}");
        assert_eq!(inf.constraint.suit_len(inf_suit(calls)), 7..=13, "{calls}");
        assert!(inf.constraint.is_satisfiable(), "{calls}");
    }
    let (ctx, inf) = last("1D P 1S P 2H P 2NT P 3D P 3NT P 5D");
    assert!(ctx.opener_other_calls.reverse);
    assert_eq!(inf.constraint.hcp_range(), 17..=21);
    // 21 hcp, seven diamonds and the reverse's four hearts, unsuited to notrump.
    assert!(inf.constraint.satisfies(hand("2", "AKQ5432", "AKJ2", "A")));
    assert!(
        last("1H P 1S P 3C P 3D P 3H P 3NT P 4H")
            .0
            .opener_other_calls
            .jump_shift
    );
    assert!(
        last("1H P 1S P 2NT P 3C P 3H P 3NT P 4H")
            .0
            .opener_other_calls
            .jump_nt_rebid
    );
    // A non-jump new suit (12-18) does not limit the hand to a minimum either.
    let (ctx, inf) = last("1H P 1S P 2C P 2D P 2H P 3NT P 4H");
    assert!(ctx.opener_other_calls.new_suit);
    assert_eq!(inf.constraint.hcp_range(), 12..=18);
    // While the rebid limits the hand (only the opening and bids of the opened suit) the field
    // is empty, also in competition.
    for calls in [
        "1S P 2C P 2S P 3NT P 4S",
        "1S P 1NT P 3S P 3NT P 4S",
        "1H 1S 2C 2S 3H P 3NT P 4H",
    ] {
        assert!(last(calls).0.opener_other_calls.is_empty(), "{calls}");
    }
}

#[test]
fn every_suit_both_partners_bid_is_agreed() {
    // Responder bid hearts and clubs, opener clubs and hearts: both are agreed, not only the
    // lowest-ranking one. Responder's 4H over opener's 3NT bids a suit opener raised, so it is
    // not the correction of an unsupported suit (it was 6+ hearts, 7+ hcp), the same as when
    // hearts are the only agreed suit.
    let (ctx, _) = last("1C P 1H P 2H P 3C P 3NT P 4H");
    assert!(ctx.agreed_suits.contains(Strain::Clubs));
    assert!(ctx.agreed_suits.contains(Strain::Hearts));
    assert_eq!(ctx.agreed_suit, Some(Suit::Clubs));
    for calls in [
        "1C P 1H P 2H P 3C P 3NT P 4H",
        "1D P 1H P 2H P 3C P 3NT P 4H",
    ] {
        let inf = last_with_partner(calls, 15..=37);
        assert_eq!(inf.rule, "fallback", "{calls}");
    }
    // Opener: diamonds, clubs and hearts are all agreed. The pull of 3NT to 5D re-raises an
    // agreed suit (opening values, the ordinary floor 26 - 10) rather than rebidding an
    // unsupported opened suit (it was 16-15, unsatisfiable).
    let calls = "1D P 2C P 2D P 2H P 3C P 3D P 3H P 3NT P 5D";
    let (ctx, _) = last(calls);
    for strain in [Strain::Clubs, Strain::Diamonds, Strain::Hearts] {
        assert!(ctx.agreed_suits.contains(strain), "{strain:?}");
    }
    let partner = Atom::ANY
        .with_hcp(10..=37)
        .with_suit_len(Suit::Diamonds, 3..=13);
    let inf = last_with_partner_constraint(calls, partner);
    assert_eq!(inf.rule, "rebid_own");
    assert_eq!(inf.constraint.hcp_range(), 16..=21);
    assert_eq!(inf.constraint.suit_len(Suit::Diamonds), 5..=13);
    // 17 hcp, five diamonds (an eight-card fit) and a singleton.
    assert!(inf.constraint.satisfies(hand("K32", "AKJ54", "AQ32", "2")));
}

#[test]
fn a_pull_after_a_reraise_game_try_is_a_choice_of_game() {
    // 1S-P-2S-P-3S invites (16-18); partner's 3NT accepts and offers a choice of game, which
    // the pull to four of the major makes: an opening with the ordinary floor (22 - 8) and an
    // eight-card fit, not a slam move (it was 23-18, unsatisfiable).
    assert_eq!(
        last("1S P 2S P 3S").1.constraint.hcp_range(),
        16..=18,
        "the game try itself"
    );
    for (calls, suit) in [
        ("1S P 2S P 3S P 3NT P 4S", Suit::Spades),
        ("1H P 2H P 3H P 3NT P 4H", Suit::Hearts),
    ] {
        let partner = Atom::ANY.with_hcp(8..=37).with_suit_len(suit, 3..=13);
        let inf = last_with_partner_constraint(calls, partner);
        assert_eq!(inf.rule, "rebid_own", "{calls}");
        assert_eq!(inf.constraint.hcp_range(), 14..=21, "{calls}");
        assert_eq!(inf.constraint.suit_len(suit), 5..=13, "{calls}");
        assert!(inf.constraint.is_satisfiable(), "{calls}");
    }
    // In a minor: five of it with the ordinary floor (26 - 6) and a hand unsuited to notrump
    // (it was 25-21).
    let partner = Atom::ANY
        .with_hcp(6..=37)
        .with_suit_len(Suit::Diamonds, 4..=13);
    let inf = last_with_partner_constraint("1D P 2D P 3D P 3NT P 5D", partner);
    assert_eq!(inf.rule, "rebid_own");
    assert_eq!(inf.constraint.hcp_range(), 20..=21);
    assert!(inf.constraint.satisfies(hand("A2", "AKQ5432", "AK2", "2")));
    assert!(!inf.constraint.satisfies(hand("A2", "AKQ54", "AK2", "432")));
}

/// The suit of the last bid of `calls`.
fn inf_suit(calls: &str) -> Suit {
    let (ctx, _) = last(calls);
    ctx.call.bid().unwrap().strain().suit().unwrap()
}

#[test]
fn responders_corrections_of_openers_3nt_keep_responders_range() {
    // Responder's preference to opener's suit over opener's 3NT is a choice of game, not a game
    // raise (13+): the range of responder's first call and an eight-card fit with opener's five.
    // After `1H-1S` that is the one-level response's 6+.
    let opener = |suit, hcp| Atom::ANY.with_hcp(hcp).with_suit_len(suit, 5..=13);
    let inf = last_with_partner_constraint("1H P 1S P 3NT P 4H", opener(Suit::Hearts, 19..=21));
    assert_eq!(inf.rule, "raise");
    assert_eq!(inf.constraint.hcp_range(), 6..=37);
    assert_eq!(inf.constraint.suit_len(Suit::Hearts), 3..=13);
    for (spades, hearts, hcp) in [("KJ32", "K32", 9), ("Q432", "Q32", 6), ("KJ32", "A32", 10)] {
        let h = hand("432", "Q32", hearts, spades);
        assert!(inf.constraint.satisfies(h), "{hcp} hcp, three hearts");
    }
    assert!(!inf.constraint.satisfies(hand("5432", "Q32", "K2", "KJ32"))); // two hearts
    // After a two-level response (SAYC 3NT: 18-19 balanced) the two-level response's 10+.
    let inf = last_with_partner_constraint("1S P 2C P 3NT P 4S", opener(Suit::Spades, 18..=19));
    assert_eq!(inf.rule, "raise");
    assert_eq!(inf.constraint.hcp_range(), 10..=37);
    assert!(inf.constraint.satisfies(hand("AQJ32", "K32", "J2", "432"))); // 11, three spades
    assert!(!inf.constraint.satisfies(hand("AQJ32", "K432", "J2", "32"))); // two spades
    // Responder's own suit, six cards or more, in the first call's range.
    let inf = last_with_partner("1C P 1S P 3NT P 4S", 19..=21);
    assert_eq!(inf.rule, "rebid_own");
    assert_eq!(inf.constraint.hcp_range(), 6..=37);
    assert!(inf.constraint.satisfies(hand("32", "432", "Q2", "KJ5432"))); // 6 spades, 6 hcp
    assert!(!inf.constraint.satisfies(hand("32", "5432", "Q2", "KJ432"))); // five spades
}

#[test]
fn responders_unlimited_first_calls_keep_their_open_ended_minimum() {
    let opener = |suit, hcp| Atom::ANY.with_hcp(hcp).with_suit_len(suit, 5..=13);
    // A non-jump new suit at the three level in competition (SAYC: forcing, 11+) is no weaker
    // than the two-level one: 10+, open-ended (it was the simple raise's 6-9, so the minor
    // correction described no hand). The floor (22 - 15, 26 - 15) is below or at it.
    let inf = last_with_partner_constraint("1S 2H 3C P 3NT P 4S", opener(Suit::Spades, 15..=19));
    assert_eq!(inf.rule, "raise");
    assert_eq!(inf.constraint.hcp_range(), 10..=37);
    assert!(inf.constraint.satisfies(hand("AQJ32", "K32", "32", "Q32"))); // 12, three spades
    let inf = last_with_partner("1S 2H 3C P 3NT P 5C", 15..=37);
    assert_eq!(inf.rule, "rebid_own");
    assert_eq!(inf.constraint.hcp_range(), 11..=37);
    assert_eq!(inf.constraint.suit_len(Suit::Clubs), 6..=13);
    assert!(inf.constraint.satisfies(hand("AKJ432", "K32", "2", "Q32"))); // 13, singleton
    // A negative double: its own minimum at the level it doubled, open-ended (8+ over 2H).
    let inf = last_with_partner_constraint("1S 2H X P 3NT P 4S", opener(Suit::Spades, 15..=19));
    assert_eq!(inf.rule, "raise");
    assert_eq!(inf.constraint.hcp_range(), 8..=37);
    let (ctx, _) = last("1S 2H X P 3NT P 4S");
    assert_eq!(ctx.owner_first_negative_double, Some(2));
    // A redouble: 10+ (it was 6-9 against the redouble's 10+ and responder's own 2NT 11-12).
    let inf = last_with_partner_constraint(
        "1H X XX 1S P P 2NT P 3NT P 4H",
        opener(Suit::Hearts, 15..=21),
    );
    assert_eq!(inf.rule, "raise");
    assert_eq!(inf.constraint.hcp_range(), 10..=37);
    // A first call no rule ranges (a penalty double of their 1NT), and no non-pass call before
    // the correction (responder passed 1H): no rule describes the correction.
    for calls in ["1H 1NT X 2C 3NT P 4H", "1H 1S P 2S 3NT P 4H"] {
        let inf = last_with_partner_constraint(calls, opener(Suit::Hearts, 15..=21));
        assert_eq!(inf.rule, "fallback", "{calls}");
    }
    assert_eq!(
        last("1H 1NT X 2C 3NT P 4H").0.owner_first_negative_double,
        None
    );
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
    // slam floor (31 - partner's minimum) is above the opening's range, so no hand bids it. In a
    // minor, five over 3NT is read as the cheapest game bid (the opening's range), not as a
    // 16-18 jump rebid the slam floor would leave satisfiable.
    for (calls, partner_min) in [
        ("2S P 3NT P 4S", 15),
        ("2H P 3NT P 4H", 15),
        ("3H P 3NT P 4H", 14),
        ("2D P 3NT P 5D", 15),
        ("3C P 3NT P 5C", 14),
        ("3D P 3NT P 5D", 14),
    ] {
        let inf = last_with_partner(calls, partner_min..=37);
        assert_eq!(inf.rule, "rebid_own", "{calls}");
        assert_eq!(min_hcp(&inf), 31 - partner_min, "{calls}");
        assert!(*inf.constraint.hcp_range().end() <= 10, "{calls}");
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
    // After a limited first response (a raise, notrump) the double is competitive within that
    // response's range (a maximum raise): no rule describes it.
    for calls in [
        "1H 1S 2H 2S P P X",
        "1D 1S 1NT 2S P P X",
        "1H P 2H 2S P P X",
    ] {
        let (ctx, inf) = last(calls);
        assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Negative), "{calls}");
        assert!(ctx.owner_acted, "{calls}");
        assert_eq!(inf.rule, "fallback", "{calls}");
    }
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
    // The same advancer in the pass-out seat is classified `Balancer`: by its history (the
    // forced answer) it keeps the ordinary balancing minimum, 9+.
    for calls in ["1H X P 1S 2H P P X", "1C X P 1H 2C P P X"] {
        let (ctx, inf) = last(calls);
        assert_eq!(ctx.role, Role::Balancer, "{calls}");
        assert!(ctx.owner_answered_partners_double, "{calls}");
        assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Takeout), "{calls}");
        assert_eq!(inf.rule, "takeout_x", "{calls}");
        assert_eq!(min_hcp(&inf), 9, "{calls}");
    }
    // A balancer whose earlier call was its own (a balancing double) still adds the +3.
    let (ctx, _) = last("1S P P X 2S P P X");
    assert!(!ctx.owner_answered_partners_double);
}
