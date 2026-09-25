//! Table-driven tests for `bridge_system::natural::classify` over realistic auctions (06-system.md
//! §8.2). Each case checks the fields that matter for that scenario; unchecked fields are left to
//! the more targeted assertions below.

mod common;

use bridge_core::{Call, Seat, Strain, Vulnerability};
use bridge_system::natural::{CallKind, DoubleKind, Role, classify};
use common::auction;

/// The very first call of the auction, before anyone has bid, is `Role::Opener` for every seat
/// (§8.2.1: "nobody has bid yet, every seat is still a candidate opener").
#[test]
fn classify_role_pre_opening_is_opener_for_every_seat() {
    let a = auction(Seat::North, Vulnerability::None, "P P P P");
    for (i, seat) in [Seat::North, Seat::East, Seat::South, Seat::West]
        .into_iter()
        .enumerate()
    {
        let ctx = classify(&a, i, seat);
        assert_eq!(ctx.role, Role::Opener, "seat {seat:?} at index {i}");
        assert_eq!(ctx.call, Call::Pass);
    }
}

#[test]
fn classify_role_opener_and_responder() {
    let a = auction(Seat::North, Vulnerability::None, "1S P 1NT P");
    let opener = classify(&a, 0, Seat::North);
    assert_eq!(opener.role, Role::Opener);
    assert_eq!(opener.position, 1);
    assert!(!opener.passed_hand);

    let responder = classify(&a, 2, Seat::South);
    assert_eq!(responder.role, Role::Responder);
    assert_eq!(responder.partner_last, Some("1S".parse().unwrap()));
}

#[test]
fn classify_role_overcaller_and_advancer() {
    // N opens 1S, E overcalls 2H, S passes, W (E's partner) raises: W is the advancer.
    let a = auction(Seat::North, Vulnerability::None, "1S 2H P 3H");
    let overcaller = classify(&a, 1, Seat::East);
    assert_eq!(overcaller.role, Role::Overcaller);

    let advancer = classify(&a, 3, Seat::West);
    assert_eq!(advancer.role, Role::Advancer);
}

/// The classic balancing seat: an opponent's opening, two passes, and now the fourth seat.
#[test]
fn classify_role_balancer() {
    let a = auction(Seat::North, Vulnerability::None, "1S P P P");
    let ctx = classify(&a, 3, Seat::West);
    assert_eq!(ctx.role, Role::Balancer);
}

/// A pass right after two other passes but with no bid at all is *not* a balancing seat: nobody
/// has opened, so every seat is still `Opener` (this is simply the auction passing out).
#[test]
fn classify_role_all_pass_is_not_balancing() {
    let a = auction(Seat::North, Vulnerability::None, "P P P P");
    let ctx = classify(&a, 3, Seat::West);
    assert_eq!(ctx.role, Role::Opener);
}

#[test]
fn classify_kind_new_suit_raise_and_rebid_own() {
    // 1S (opener, new suit) - P - 2H (responder, new suit) - P - 3S (opener, raise? no: rebid own).
    let a = auction(Seat::North, Vulnerability::None, "1S P 2H P 2S");
    let opening = classify(&a, 0, Seat::North);
    match opening.kind {
        CallKind::Bid {
            new_suit,
            jump,
            nt,
            rebid_own,
            ..
        } => {
            assert!(new_suit);
            assert_eq!(jump, 0);
            assert!(!nt);
            assert!(!rebid_own);
        }
        other => panic!("expected a bid, got {other:?}"),
    }

    let response = classify(&a, 2, Seat::South);
    match response.kind {
        CallKind::Bid { new_suit, .. } => assert!(new_suit),
        other => panic!("expected a bid, got {other:?}"),
    }

    let rebid = classify(&a, 4, Seat::North);
    match rebid.kind {
        CallKind::Bid {
            rebid_own, raise, ..
        } => {
            assert!(rebid_own);
            assert!(!raise);
        }
        other => panic!("expected a bid, got {other:?}"),
    }
}

#[test]
fn classify_kind_raise_of_partners_suit() {
    // 1S (opener) - P - 2S (responder raises opener's suit).
    let a = auction(Seat::North, Vulnerability::None, "1S P 2S");
    let raise = classify(&a, 2, Seat::South);
    match raise.kind {
        CallKind::Bid {
            raise, new_suit, ..
        } => {
            assert!(raise);
            assert!(!new_suit);
        }
        other => panic!("expected a bid, got {other:?}"),
    }
    assert_eq!(raise.agreed_suit, None); // not agreed until *both* have bid it.
}

#[test]
fn classify_kind_jump_is_levels_skipped() {
    // 1S - 3H: cheapest legal heart bid over 1S is 2H (hearts rank below spades), so 3H is one
    // level higher than necessary.
    let a = auction(Seat::North, Vulnerability::None, "1S 3H");
    let ctx = classify(&a, 1, Seat::East);
    match ctx.kind {
        CallKind::Bid { jump, .. } => assert_eq!(jump, 1),
        other => panic!("expected a bid, got {other:?}"),
    }

    // 1S - 2H: this *is* the cheapest legal heart bid, so no jump.
    let a2 = auction(Seat::North, Vulnerability::None, "1S 2H");
    let ctx2 = classify(&a2, 1, Seat::East);
    match ctx2.kind {
        CallKind::Bid { jump, .. } => assert_eq!(jump, 0),
        other => panic!("expected a bid, got {other:?}"),
    }
}

#[test]
fn classify_kind_cue_bid_of_their_suit() {
    // P (N) - 1D (E, opponent opens) - 1S (S, our overcall) - P (W) - 2D (N, cue of E's suit).
    let a = auction(Seat::North, Vulnerability::None, "P 1D 1S P 2D");
    let ctx = classify(&a, 4, Seat::North);
    match ctx.kind {
        CallKind::Bid {
            cue,
            new_suit,
            raise,
            rebid_own,
            ..
        } => {
            assert!(cue);
            assert!(!new_suit);
            assert!(!raise);
            assert!(!rebid_own);
        }
        other => panic!("expected a bid, got {other:?}"),
    }
}

#[test]
fn classify_kind_reverse() {
    // 1C (opener) - P - 1S (responder) - P - 2H (opener reverses: hearts outrank clubs, opener's
    // first suit, at the cheapest available level after a 1-level response).
    let a = auction(Seat::North, Vulnerability::None, "1C P 1S P 2H");
    let ctx = classify(&a, 4, Seat::North);
    match ctx.kind {
        CallKind::Bid { reverse, .. } => assert!(reverse),
        other => panic!("expected a bid, got {other:?}"),
    }

    // 1H (opener) - P - 1S (responder) - P - 2C: clubs do *not* outrank hearts, so this is not a
    // reverse even though it is opener's second suit at the 2-level.
    let not_reverse = auction(Seat::North, Vulnerability::None, "1H P 1S P 2C");
    let ctx2 = classify(&not_reverse, 4, Seat::North);
    match ctx2.kind {
        CallKind::Bid { reverse, .. } => assert!(!reverse),
        other => panic!("expected a bid, got {other:?}"),
    }
}

#[test]
fn classify_double_kind_takeout() {
    let a = auction(Seat::North, Vulnerability::None, "1S X");
    let ctx = classify(&a, 1, Seat::East);
    assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Takeout));
}

#[test]
fn classify_double_kind_negative() {
    // 1D (opener) - 1S (overcall) - X (responder: negative).
    let a = auction(Seat::North, Vulnerability::None, "1D 1S X");
    let ctx = classify(&a, 2, Seat::South);
    assert_eq!(ctx.role, Role::Responder);
    assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Negative));
}

#[test]
fn classify_double_kind_penalty_high_level() {
    // 1S (opener) - 4H (overcall) - X (responder doubles a game-level overcall: penalty).
    let a = auction(Seat::North, Vulnerability::None, "1S 4H X");
    let ctx = classify(&a, 2, Seat::South);
    assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Penalty));
}

#[test]
fn classify_double_kind_penalty_of_notrump() {
    let a = auction(Seat::North, Vulnerability::None, "1NT X");
    let ctx = classify(&a, 1, Seat::East);
    assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Penalty));
}

#[test]
fn classify_double_kind_responsive() {
    // P (N) - 1S (E, opponent opens) - X (S, takeout) - 2S (W, raise) - X (N, responsive double).
    let a = auction(Seat::North, Vulnerability::None, "P 1S X 2S X");
    let ctx = classify(&a, 4, Seat::North);
    assert_eq!(ctx.role, Role::Advancer);
    assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Responsive));
}

#[test]
fn classify_double_kind_support() {
    // 1D (opener) - P - 1H (responder, new suit) - 1S (opponent intervenes) - X (opener: support).
    let a = auction(Seat::North, Vulnerability::None, "1D P 1H 1S X");
    let ctx = classify(&a, 4, Seat::North);
    assert_eq!(ctx.role, Role::Opener);
    assert_eq!(ctx.kind, CallKind::Double(DoubleKind::Support));
}

#[test]
fn classify_last_bid_and_agreed_suit() {
    // Both opener and responder bid spades: an agreed suit.
    let a = auction(Seat::North, Vulnerability::None, "1S P 2S P 3S");
    let ctx = classify(&a, 4, Seat::North);
    assert_eq!(ctx.agreed_suit, Some(bridge_core::Suit::Spades));
    assert_eq!(ctx.last_bid.map(|b| b.strain()), Some(Strain::Spades));
}

#[test]
fn classify_vulnerability() {
    let a = auction(Seat::North, Vulnerability::NS, "1S 2H");
    let ctx = classify(&a, 1, Seat::East);
    // East/West are "we" here, North/South (vulnerable) are "they".
    assert_eq!(ctx.vul, (false, true));
}

#[test]
fn classify_competitive_requires_both_sides_to_have_bid_already() {
    // Before East's very first call, only North/South have bid: not yet competitive from East's
    // point of view (competitive describes the auction *so far*, not this call).
    let a = auction(Seat::North, Vulnerability::None, "1S P");
    let ctx = classify(&a, 1, Seat::East);
    assert!(!ctx.competitive);

    // By West's turn here, both sides have bid a strain.
    let a2 = auction(Seat::North, Vulnerability::None, "1S 2H P 3H");
    let ctx2 = classify(&a2, 3, Seat::West);
    assert!(ctx2.competitive);
}
