//! Targeted regression tests against the real, compiled `systems/sayc/sayc.bml`, one per SAYC
//! content fix made while closing out phase 3.10/3.12 (`scratchpad/sayc_review_findings.json`,
//! `scratchpad/p3/dropped.json` entries 10..19, `scratchpad/p3/confirmed.json` entry 8). Each test
//! names the finding it fixes in its doc comment and checks `choose_bid` on a concrete hand that
//! was previously mis-bid (or, for `1N-(2X)-`, unreachable), rather than only the aggregate
//! `tests/consistency.rs` harness -- so a future edit that reintroduces one of these regresses a
//! single, readable test instead of only nudging a violation count.

mod common;

use bridge_bidding::{BidContext, ImplicitPass, PolicyParams, Scoring, choose_bid};
use bridge_core::{Seat, Strain, Vulnerability};
use common::*;

fn ctx(table: &bridge_bidding::Table) -> BidContext<'_> {
    BidContext {
        scoring: Scoring::Imp,
        natural: Some(table.natural.as_ref()),
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::default(),
    }
}

/// dropped.json #14: "the simple overcall (prio 2) outranks Michaels, the unusual 2NT, the 1NT
/// overcall and weak jump overcalls". A hand that qualifies for a more specific, shapelier call
/// must get that call, not the plain one-level overcall that also happens to fit it.
#[test]
fn michaels_outranks_plain_overcall() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = ctx(&table);
    // 5 spades, 5 clubs, 10 hcp: qualifies for both Michaels (2H, over a 1H opening) and the
    // plain `1S` overcall (4+ spades, 8-16 hcp).
    let a = common::auction(Seat::North, Vulnerability::None, &[bid(1, Strain::Hearts)]);
    let h = common::hand("AJ432", "3", "32", "AJ432");
    let choice = choose_bid(&table, h, &a, &ctx);
    assert_eq!(
        choice.call(),
        Some(bid(2, Strain::Hearts)),
        "a 5-5 Michaels hand over 1H must bid Michaels (2H), not the plain 1S overcall: {choice:?}"
    );
}

/// dropped.json #14 (unusual notrump half): a 5+5+ two-suited hand in the two lowest unbid suits
/// must bid the unusual 2NT, not a plain one-level overcall of one of those same two suits.
#[test]
fn unusual_notrump_outranks_plain_overcall() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = ctx(&table);
    // Over 1C, the unusual 2NT shows 5+ diamonds and 5+ hearts. 10 hcp also fits the plain `1D`/
    // `1H` overcall (4+, 8-16 hcp).
    let a = common::auction(Seat::North, Vulnerability::None, &[bid(1, Strain::Clubs)]);
    let h = common::hand("3", "AJ432", "AJ432", "32");
    let choice = choose_bid(&table, h, &a, &ctx);
    assert_eq!(
        choice.call(),
        Some(bid(2, Strain::NoTrump)),
        "a 5-5 unusual-notrump hand over 1C must bid 2NT, not a plain one-suited overcall: \
         {choice:?}"
    );
}

/// dropped.json #14 (weak jump half): an exact 6-card suit within the weak jump overcall's own
/// range must jump, not settle for the plain one-level overcall the same hand also satisfies.
#[test]
fn weak_jump_overcall_outranks_plain_overcall() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = ctx(&table);
    // Over 1H, 6 spades and 9 hcp fits both the plain `1S` (4+, 8-16 hcp) and the weak jump `2S`
    // (6=, 5-11 hcp).
    let a = common::auction(Seat::North, Vulnerability::None, &[bid(1, Strain::Hearts)]);
    let h = common::hand("32", "32", "432", "AJ9432");
    let choice = choose_bid(&table, h, &a, &ctx);
    assert_eq!(
        choice.call(),
        Some(bid(2, Strain::Spades)),
        "a 6-card weak-jump-range spade hand over 1H must jump to 2S, not overcall 1S: {choice:?}"
    );
}

/// dropped.json #11 / NOTES.md #20: advancing a takeout double must go by the hand's actual
/// shape, not always the cheapest unbid suit regardless of what advancer holds.
#[test]
fn takeout_double_advance_follows_shape_not_cheapest_suit() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = ctx(&table);
    // West opens 1C, North doubles for takeout, East passes; South (the advancer) has 4 hearts
    // and no diamonds at all, so `1D` (the cheapest unbid suit) must not be picked.
    let a = common::auction(
        Seat::West,
        Vulnerability::None,
        &[bid(1, Strain::Clubs), DBL, PASS],
    );
    let h = common::hand("5432", "", "AJ32", "Q9432");
    let choice = choose_bid(&table, h, &a, &ctx);
    assert_eq!(
        choice.call(),
        Some(bid(1, Strain::Hearts)),
        "advancing 1C-(D) with 4 hearts and no diamonds must bid 1H, not the cheaper-but-unheld \
         1D: {choice:?}"
    );
}

/// dropped.json #11 / NOTES.md #20: an invitational-quality 5+ card suit is shown by jumping,
/// not folded into a plain minimum advance in a different suit.
#[test]
fn takeout_double_advance_invitational_jump_outranks_other_minimum_suits() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = ctx(&table);
    let a = common::auction(
        Seat::West,
        Vulnerability::None,
        &[bid(1, Strain::Clubs), DBL, PASS],
    );
    // 5 spades, invitational values (11 hcp), and an incidental 4-card heart holding that also
    // fits the plain minimum `1H`.
    let h = common::hand("32", "32", "K432", "AKJ32");
    let choice = choose_bid(&table, h, &a, &ctx);
    assert_eq!(
        choice.call(),
        Some(bid(2, Strain::Spades)),
        "advancing 1C-(D) with 5 spades and invitational values must jump to 2S, not settle for \
         the plain minimum 1H: {choice:?}"
    );
}

/// dropped.json #11 / NOTES.md #20: `(1S)-D-` was missing outright.
#[test]
fn takeout_double_advance_of_1s_double_is_covered() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = ctx(&table);
    let a = common::auction(
        Seat::West,
        Vulnerability::None,
        &[bid(1, Strain::Spades), DBL, PASS],
    );
    let h = common::hand("432", "AJ32", "Q432", "32");
    let choice = choose_bid(&table, h, &a, &ctx);
    assert_eq!(
        choice.call(),
        Some(bid(2, Strain::Diamonds)),
        "advancing 1S-(D) with 4 diamonds (and 4 hearts, ranked below diamonds) must bid 2D: \
         {choice:?}"
    );
}

/// confirmed.json #8: `1N-(1X)-` was an impossible history (no 1-level call ranks above 1NT) and
/// silently expanded to nothing, so a natural response to a 2-level overcall of our own 1NT was
/// entirely off-system. Checks both halves of the fix: the natural suit ranked above the
/// overcall (reachable directly at the 2 level) and one ranked below it (needing the extra level
/// added at the 3 level, `NOTES.md` #21).
#[test]
fn natural_response_after_1nt_is_overcalled_is_on_system() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = ctx(&table);

    // 1NT-(2D)-?: spades (above diamonds) is directly reachable at 2S.
    let a = common::auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::NoTrump), bid(2, Strain::Diamonds)],
    );
    let h = common::hand("32", "32", "432", "AKQ432");
    let choice = choose_bid(&table, h, &a, &ctx);
    assert_eq!(
        choice.call(),
        Some(bid(2, Strain::Spades)),
        "1NT-(2D)- with 6 spades must be on-system (natural 2S), not a Fallback/NoCandidate: \
         {choice:?}"
    );

    // 1NT-(2S)-?: clubs (below spades) needs the extra level, at 3C.
    let a2 = common::auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::NoTrump), bid(2, Strain::Spades)],
    );
    let h2 = common::hand("AKQ432", "32", "432", "32");
    let choice2 = choose_bid(&table, h2, &a2, &ctx);
    assert_eq!(
        choice2.call(),
        Some(bid(3, Strain::Clubs)),
        "1NT-(2S)- with 6 clubs must be on-system (natural 3C), not a Fallback/NoCandidate: \
         {choice2:?}"
    );
}

/// dropped.json #13 / NOTES.md #22: Stayman after an opposing double of our 1NT must still
/// require a four-card major, exactly like the direct (uninterfered) Stayman row.
#[test]
fn stayman_after_double_of_1nt_requires_a_major() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = ctx(&table);
    let a = common::auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::NoTrump), DBL],
    );
    // 8 hcp, balanced, no four-card major: must not ask Stayman.
    let h = common::hand("QJ32", "KQ32", "32", "432");
    let choice = choose_bid(&table, h, &a, &ctx);
    assert_ne!(
        choice.call(),
        Some(bid(2, Strain::Clubs)),
        "1NT-(X)- with no four-card major must not bid Stayman (2C): {choice:?}"
    );
}

/// dropped.json #13 / NOTES.md #22: Stayman opposite our own 1NT overcall must also require a
/// four-card major.
#[test]
fn stayman_opposite_1nt_overcall_requires_a_major() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = ctx(&table);
    let a = common::auction(
        Seat::West,
        Vulnerability::None,
        &[bid(1, Strain::Clubs), bid(1, Strain::NoTrump)],
    );
    // 8 hcp, balanced, no four-card major: must not ask Stayman opposite partner's 1NT overcall.
    let h = common::hand("QJ32", "KQ32", "32", "432");
    let choice = choose_bid(&table, h, &a, &ctx);
    assert_ne!(
        choice.call(),
        Some(bid(2, Strain::Clubs)),
        "1C-1N- with no four-card major must not bid Stayman (2C): {choice:?}"
    );
}

/// dropped.json #15 / NOTES.md #23: in the balancing seat, the plain suit overcall must not be
/// swallowed by the double when the hand actually holds a real 4-card suit.
#[test]
fn balancing_suit_overcall_outranks_double() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = ctx(&table);
    // West opens 1C, North/East/South all pass; West's partner (East) already passed, so this is
    // the classic balancing seat for West's partner... rather, North reopens after 1C-P-P.
    let a = common::auction(
        Seat::West,
        Vulnerability::None,
        &[bid(1, Strain::Clubs), PASS, PASS],
    );
    // 9 hcp, 4 hearts (only): a real balancing overcall, not merely a takeout double's own 8+
    // hcp.
    let h = common::hand("432", "432", "AJ32", "KJ2");
    let choice = choose_bid(&table, h, &a, &ctx);
    assert_eq!(
        choice.call(),
        Some(bid(1, Strain::Hearts)),
        "1C-P-P- with 4 hearts and 9 hcp must reopen with 1H, not a takeout double: {choice:?}"
    );
}

/// dropped.json #15 / NOTES.md #23: the balancing jump overcall must be a genuinely lower-strength
/// preemptive jump, not simply "the same hand as the plain overcall, one card longer."
#[test]
fn balancing_jump_overcall_is_preemptive_not_full_strength() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = ctx(&table);
    let a = common::auction(
        Seat::West,
        Vulnerability::None,
        &[bid(1, Strain::Clubs), PASS, PASS],
    );
    // 14 hcp, 5 hearts: too strong for the preemptive jump; must overcall calmly at the one
    // level.
    let h = common::hand("32", "32", "AKQ32", "AJ32");
    let choice = choose_bid(&table, h, &a, &ctx);
    assert_eq!(
        choice.call(),
        Some(bid(1, Strain::Hearts)),
        "1C-P-P- with 14 hcp and 5 hearts must reopen with the plain 1H, not jump to 2H: \
         {choice:?}"
    );
}

/// dropped.json #16 / NOTES.md #24: a new suit ranked above a weak-two opening is reachable at
/// the cheap 2 level, not only as an unwarranted 3-level jump.
#[test]
fn weak_two_response_new_suit_above_opening_is_at_two_level() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = ctx(&table);
    let a = common::auction(
        Seat::North,
        Vulnerability::None,
        &[bid(2, Strain::Diamonds), PASS],
    );
    let h = common::hand("32", "32", "AKQ32", "K432");
    let choice = choose_bid(&table, h, &a, &ctx);
    assert_eq!(
        choice.call(),
        Some(bid(2, Strain::Hearts)),
        "2D-P- with 12 hcp and 5 hearts must bid the cheap 2H, not jump to 3H: {choice:?}"
    );
}

/// dropped.json #17 / NOTES.md #25: a plain 4-card major (not both majors 4-4, so no fit for the
/// negative double) must have a natural response, not be left with no call.
#[test]
fn negative_double_leaves_a_call_for_a_plain_four_card_major() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = ctx(&table);
    let a = common::auction(
        Seat::West,
        Vulnerability::None,
        &[bid(1, Strain::Clubs), bid(1, Strain::Diamonds)],
    );
    // 8 hcp, exactly 4 hearts, 3 spades: no fit for the negative double (needs both majors).
    let h = common::hand("432", "432", "AJ32", "K32");
    let choice = choose_bid(&table, h, &a, &ctx);
    assert_eq!(
        choice.call(),
        Some(bid(1, Strain::Hearts)),
        "1C-(1D)- with exactly 4 hearts and 8 hcp must bid 1H, not have no candidate: {choice:?}"
    );
}

/// dropped.json #14 (1NT-overcall half): a balanced hand with a stopper in the 15-18 hcp range
/// must bid the notrump overcall, not the plain one-level suit overcall a 4+ side suit also
/// satisfies.
#[test]
fn notrump_overcall_outranks_plain_overcall() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = ctx(&table);
    // Over 1D, a balanced 15-count with a solid diamond stopper and an incidental 4-card major
    // also fits the plain `1H` overcall (4+ hearts, 8-16 hcp).
    let a = common::auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::Diamonds)],
    );
    let h = common::hand("K32", "AQJ", "KQ32", "432");
    let choice = choose_bid(&table, h, &a, &ctx);
    assert_eq!(
        choice.call(),
        Some(bid(1, Strain::NoTrump)),
        "a balanced 15-18 hcp hand with a stopper over 1D must overcall 1NT, not a plain suit \
         overcall: {choice:?}"
    );
}

/// Regression tests for the notrump / strong 2C / weak-two / preempt lane (`notrump.bml`,
/// `strong-2c.bml`, `weak-twos.bml`, `preempts.bml`; `systems/sayc/NOTES.md` section "Notrump
/// lane"). Each test replays one sequence that the replay-based consistency harness
/// (`tests/consistency.rs`) or the review (`harness_review.json` #0/#3, `dropped.json` #10/#16)
/// found mis-bid or without any call, and checks the call `choose_bid` now makes for a
/// constructed hand. Auctions are written with North as dealer, hands as `S.H.D.C`.
mod notrump_lane {
    use super::ctx;
    use bridge_bidding::{BidChoice, ChoiceSource, Table, choose_bid};
    use bridge_core::{Auction, Call, Hand, Seat, Vulnerability};
    use std::sync::OnceLock;

    fn table() -> &'static Table {
        static TABLE: OnceLock<Table> = OnceLock::new();
        TABLE.get_or_init(|| super::common::compile_sayc("sayc.bml"))
    }

    /// `choose_bid` for `hand` (`S.H.D.C`) after `calls` (space-separated, North dealer).
    fn choose(calls: &str, hand: &str) -> BidChoice {
        let table = table();
        let calls: Vec<Call> = calls
            .split_whitespace()
            .map(|c| c.parse().expect("test call parses"))
            .collect();
        let auction = Auction::from_calls(Seat::North, Vulnerability::None, calls)
            .expect("test auction is legal");
        let hand: Hand = hand.parse().expect("test hand parses");
        choose_bid(table, hand, &auction, &ctx(table))
    }

    /// Asserts that the chosen call is `expected` and that it came from the system itself (a
    /// row, not natural inference); `Pass` may also come from the implicit pass when
    /// `implicit_pass_ok` is set.
    fn assert_call(calls: &str, hand: &str, expected: &str, implicit_pass_ok: bool, why: &str) {
        let choice = choose(calls, hand);
        let expected: Call = expected.parse().expect("expected call parses");
        let BidChoice::Chosen(chosen) = &choice else {
            panic!("[{calls}] {hand}: expected {expected}, got NoCandidate ({why}): {choice:?}");
        };
        assert_eq!(chosen.call, expected, "[{calls}] {hand}: {why}: {choice:?}");
        let source_ok = chosen.source == ChoiceSource::System
            || (implicit_pass_ok && chosen.source == ChoiceSource::ImplicitPass);
        assert!(
            source_ok,
            "[{calls}] {hand}: {expected} must come from the system, not {:?} ({why})",
            chosen.source
        );
    }

    /// The unconstrained `4C = !BW` row over 1NT swallowed every hand without another call.
    #[test]
    fn weak_balanced_hand_passes_1nt_instead_of_gerber() {
        assert_call(
            "1NT P",
            "Q32.Q32.J32.5432",
            "P",
            true,
            "a 5 hcp balanced hand passes 1NT; it is not a 4C Gerber ask",
        );
    }

    /// The same catch-all over 2NT: 4+ hcp raises to game.
    #[test]
    fn game_values_raise_2nt_to_3nt() {
        assert_call(
            "2NT P",
            "Q32.Q32.J32.5432",
            "3NT",
            false,
            "5 hcp opposite 20-21 raises to 3NT",
        );
    }

    /// And over 2C-2D-2NT (22-24): 3+ hcp raises to game.
    #[test]
    fn game_values_raise_2c_2d_2nt_to_3nt() {
        assert_call(
            "2C P 2D P 2NT P",
            "Q32.Q32.J32.5432",
            "3NT",
            false,
            "5 hcp opposite 22-24 raises to 3NT",
        );
    }

    /// Stayman continuation: a 4-4 heart fit with game values bids game in the major, not the
    /// natural fallback's 2NT.
    #[test]
    fn stayman_heart_fit_with_game_values_bids_4h() {
        assert_call(
            "1NT P 2C P 2H P",
            "KQ32.K432.A2.432",
            "4H",
            false,
            "12 hcp with four hearts after 1NT-2C-2H bids 4H",
        );
    }

    /// Stayman continuation: invitational values without a heart fit bid 2NT.
    #[test]
    fn stayman_invitation_without_fit_bids_2nt() {
        assert_call(
            "1NT P 2C P 2H P",
            "KQ32.Q32.J32.432",
            "2NT",
            false,
            "8 hcp with four spades (no heart fit) invites with 2NT",
        );
    }

    /// Stayman continuation after the 2D denial: game values bid 3NT.
    #[test]
    fn stayman_denial_with_game_values_bids_3nt() {
        assert_call(
            "1NT P 2C P 2D P",
            "KQ32.K432.A2.432",
            "3NT",
            false,
            "12 hcp after 1NT-2C-2D bids 3NT",
        );
    }

    /// Opener's answer to 2NT after 1NT-2C-2H: responder has four spades, so a minimum with
    /// four spades bids 3S.
    #[test]
    fn opener_shows_spades_after_stayman_2nt() {
        assert_call(
            "1NT P 2C P 2H P 2NT P",
            "AK32.KQ32.Q2.J32",
            "3S",
            false,
            "a 15 hcp opener with both majors shows the spade fit",
        );
    }

    /// A 16-17 hcp transfer hand with five spades had no call at all (NoCandidate).
    #[test]
    fn strong_transfer_hand_makes_quantitative_4nt() {
        assert_call(
            "1NT P 2H P 2S P",
            "AT974.KQ3.AK.J43",
            "4NT",
            false,
            "16-17 hcp with exactly five spades invites slam with 4NT",
        );
    }

    /// An 18+ hcp transfer hand with five hearts bids 6NT.
    #[test]
    fn very_strong_transfer_hand_bids_6nt() {
        assert_call(
            "1NT P 2D P 2H P",
            "A6.KQT32.KQ.AT76",
            "6NT",
            false,
            "18+ hcp with exactly five hearts bids 6NT",
        );
    }

    /// 1NT-3NT is a system position: opener's closing Pass is written, not left to natural
    /// inference.
    #[test]
    fn opener_passes_1nt_3nt_explicitly() {
        assert_call(
            "1NT P 3NT P",
            "AK4.AKT7.QJ63.T4",
            "P",
            false,
            "opener passes 1NT-3NT",
        );
    }

    /// After a game-forcing new suit in a transfer auction, opener with three-card support
    /// bids game in responder's major (it had no call before).
    #[test]
    fn opener_raises_transfer_major_after_new_suit() {
        assert_call(
            "1NT P 2D P 2H P 3D P",
            "QJ97.K86.64.AKQ4",
            "4H",
            false,
            "opener with three hearts bids 4H over 3D",
        );
    }

    /// 3NT after a transfer: opener with three-card support corrects to 4 of the major.
    #[test]
    fn opener_corrects_transfer_3nt_with_support() {
        assert_call(
            "1NT P 2H P 2S P 3NT P",
            "K32.KQ2.AQ32.K32",
            "4S",
            false,
            "opener with three spades corrects 3NT to 4S",
        );
    }

    /// Opener competes over an overcall of the transfer with four-card support.
    #[test]
    fn opener_competes_over_overcalled_transfer_with_fit() {
        assert_call(
            "1NT P 2D 3D",
            "A32.AJ32.J4.AJ32",
            "3H",
            false,
            "four hearts compete to 3H after 1NT-2D-(3D)",
        );
    }

    /// A jump overcall of 1NT used to map back onto the uncontested table (lenient matching)
    /// and leave responder with no call.
    #[test]
    fn responder_bids_3nt_with_stopper_over_jump_overcall() {
        assert_call(
            "P 1NT 3S",
            "AQ5.KQ85.Q976.AT",
            "3NT",
            false,
            "17 hcp with a spade stopper bids 3NT over (3S)",
        );
    }

    /// Weak twos: the `unlimited` 4-level raise took every hand, so responder never passed.
    #[test]
    fn weak_hand_passes_weak_two() {
        assert_call(
            "2D P",
            "8742.T9843.7.J9",
            "P",
            true,
            "a 1 hcp hand passes 2D",
        );
    }

    /// Weak twos: a strong balanced hand now reaches the 2NT ask instead of the game raise.
    #[test]
    fn strong_hand_asks_with_2nt_over_weak_two() {
        assert_call(
            "2S P",
            "J42.KQ3.AQ65.KQ9",
            "2NT",
            false,
            "17 hcp with three spades asks with 2NT",
        );
    }

    /// Weak twos: feature rebids are reachable; the stopper suit is shown.
    #[test]
    fn weak_two_maximum_shows_its_feature() {
        assert_call(
            "2S P 2NT P",
            "KQJ982.32.A52.32",
            "3D",
            false,
            "a maximum with the diamond ace shows the 3D feature",
        );
    }

    /// Weak twos: opener raises a forcing new suit with three-card support (no call before).
    #[test]
    fn weak_two_opener_raises_forcing_new_suit() {
        assert_call(
            "2H P 2S P",
            "J32.AKQJ98.T5.52",
            "3S",
            false,
            "opener with three spades raises 2S to 3S",
        );
    }

    /// Weak twos: an overcall no longer maps onto the uncontested table; a hand without a
    /// listed action passes.
    #[test]
    fn responder_passes_overcalled_weak_two_without_values() {
        assert_call(
            "P 2H 3H",
            "J94.QT7.Q32.AQ84",
            "P",
            true,
            "11 hcp passes after 2H-(3H)",
        );
    }

    /// Preempts: the `unlimited` raise took every hand, so a 19 hcp hand raised to 4C and a
    /// 1 hcp hand bid too.
    #[test]
    fn preempt_responses_have_real_ranges() {
        assert_call(
            "3C P",
            "AJ42.KQ3.AQ65.K9",
            "4NT",
            false,
            "19 hcp with club support asks for aces",
        );
        assert_call(
            "3C P",
            "8742.T9843.7.J9",
            "P",
            true,
            "a 1 hcp hand passes 3C",
        );
        assert_call(
            "4H P",
            "87432.T9.8765.J9",
            "P",
            true,
            "a 1 hcp hand passes 4H instead of bidding Blackwood",
        );
    }

    /// Strong 2C: opener raises a positive response with three-card support (no call before).
    #[test]
    fn strong_2c_opener_raises_positive_response() {
        assert_call(
            "2C P 2H P",
            "QJ764.KQ3.AKJ.AQ",
            "3H",
            false,
            "opener with three hearts raises the 2H positive",
        );
    }

    /// Strong 2C: responder may not pass opener's forcing major rebid.
    #[test]
    fn strong_2c_responder_keeps_forcing_rebid_alive() {
        assert_call(
            "2C P 2D P 2H P",
            "432.32.J432.5432",
            "2NT",
            false,
            "a bust without three hearts makes the waiting 2NT",
        );
    }

    /// Strong 2C: a weak responder passes an overcall (no call before).
    #[test]
    fn strong_2c_weak_responder_passes_overcall() {
        assert_call(
            "2C 2S",
            "74.QJ743.T3.J983",
            "P",
            false,
            "4 hcp passes after 2C-(2S)",
        );
    }
}
