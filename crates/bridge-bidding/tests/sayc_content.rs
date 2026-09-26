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

/// Lane `sayc-comp` (phase 3 SAYC completion: openings, responses, rebids and competition). Each
/// case is a constructed hand at one auction position with the call SAYC makes there; every case
/// was previously mis-bid, left without a system call (`NoCandidate`), or passed by an implicit
/// pass where the booklet requires a call. Auctions are written dealer-North, none vulnerable,
/// calls separated by spaces; hands are `S.H.D.C`.
mod sayc_comp {
    use super::*;
    use bridge_core::{Auction, Call, Hand};

    /// Asserts every `(auction, hand, expected call)` case against the compiled SAYC system.
    fn check(cases: &[(&str, &str, &str)]) {
        let table = common::compile_sayc("sayc.bml");
        let ctx = ctx(&table);
        let mut failures = Vec::new();
        for &(calls, hand, expected) in cases {
            let mut a = Auction::new(Seat::North, Vulnerability::None);
            for c in calls.split_whitespace() {
                let c: Call = c.parse().expect("valid call");
                a = a.with(c).expect("legal call");
            }
            let h: Hand = hand.parse().expect("valid hand");
            let expected: Call = expected.parse().expect("valid call");
            let choice = choose_bid(&table, h, &a, &ctx);
            if choice.call() != Some(expected) {
                failures.push(format!(
                    "  [{calls}] {hand}: expected {expected}, got {:?}",
                    choice.call().map(|c| c.to_string())
                ));
            }
        }
        assert!(failures.is_empty(), "wrong calls:\n{}", failures.join("\n"));
    }

    /// Opener's rebid after a forcing one-level response: every strength band has a call, so a
    /// maximum opener no longer passes a forcing new suit (the old tables stopped at 18 hcp).
    #[test]
    fn opener_rebid_after_one_level_response_covers_every_band() {
        check(&[
            // 19 hcp with four-card support: jump to game in responder's major.
            ("1H P 1S P", "AKQ98.AKQ875..J6", "4S"),
            // 20 hcp, six hearts, no fit: jump to game in the opening suit.
            ("1H P 1S P", "3.AKQJT4.AQ9.A54", "4H"),
            // 19 hcp with both majors after 1C-1D: jump shift into the lower major.
            ("1C P 1D P", "AQ62.AKT8.5.AQT6", "2H"),
            // 21 hcp with long clubs and no major: 3NT.
            ("1C P 1D P", "KQ2.AT.AJ.AKT943", "3NT"),
            // 21 hcp with six diamonds after 1D-1H: 3NT.
            ("1D P 1H P", "K96.A.AKT542.AK6", "3NT"),
            // 12 hcp 1C opener with four hearts and 5 clubs after 1C-1S: rebid clubs, 2H would be
            // a reverse.
            ("1C P 1S P", "Q6.AQJ5.J9.Q8752", "2C"),
        ]);
    }

    /// Opener's rebid after a two-over-one response (which promises another bid): a raise with
    /// four-card support, a new suit, 2NT with a balanced minimum, or the minimum rebid of the
    /// opening suit; none of these hands may pass.
    #[test]
    fn opener_rebid_after_two_over_one_is_never_pass() {
        check(&[
            ("1S P 2C P", "AJT82.KT9.Q.AQ62", "3C"),
            ("1S P 2C P", "QJ972.AK73.Q3.52", "2H"),
            ("1S P 2C P", "AK862.AT7.Q97.J2", "2NT"),
            // 14 hcp with four spades after 1H-2C: 2S would be a reverse, so rebid hearts.
            ("1H P 2C P", "KJ63.K8762.AK.T3", "2H"),
            ("1H P 2C P", "A74.AQT73.AJ62.8", "2D"),
            ("1D P 2C P", "AQ3.K2.KJ54.Q983", "3C"),
        ]);
    }

    /// dropped.json #19: rows that could never be chosen because an earlier-ranked sibling
    /// covered them -- 1M-3NT behind the 2/1 new suits, and the game-forcing jump preference
    /// 1S-2C-2H-3S behind the fourth-suit 3D. Also the limit raise covers 12 hcp, so a 12-count
    /// with a fit no longer falls into the gap between the limit raise and Jacoby 2NT, and a
    /// 3=4=3=3 13-count with three spades has the 2C response.
    #[test]
    fn response_rows_shadowed_by_siblings_are_reachable() {
        check(&[
            ("1H P", "AQ3.K2.KJ54.Q983", "3NT"),
            ("1S P", "K2.AQ3.KJ54.Q983", "3NT"),
            ("1S P", "KT98432.AJ3..A73", "3S"),
            ("1S P", "K32.AQ32.K32.Q32", "2C"),
            ("1S P 2C P 2H P", "KJ3.T2.A32.KQ432", "3S"),
        ]);
    }

    /// Jacoby 2NT: opener shows shortness at the three level, otherwise strength (3M 18+, 3NT
    /// 15--17, 4M minimum).
    #[test]
    fn jacoby_2nt_rebids() {
        check(&[
            ("1H P 2NT P", "K32.AQJ75.K32.32", "4H"),
            ("1H P 2NT P", "K3.AQJ75.K32.Q32", "3NT"),
            ("1H P 2NT P", "K32.AQJ75.AK32.3", "3C"),
            ("1H P 2NT P", "AK3.AQJ75.K32.Q2", "3H"),
        ]);
    }
    /// harness_review.json #0: responder's actions after an overcall were missing, so the
    /// uncontested response table was reached by substituting the overcall with a pass
    /// (`resolve_lenient`), where a hand fitting none of its rows had no call at all. Every
    /// overcall level now has its own table (raises, cuebid limit raise, notrump with a
    /// stopper, negative or penalty double, natural new suits), and a hand with nothing to say
    /// passes.
    #[test]
    fn responder_acts_after_an_overcall() {
        check(&[
            // 1H-(1S): notrump with a spade stopper, 10 hcp.
            ("1H 1S", "KQJ875.Q9.Q9.865", "1NT"),
            // 1H-(1S): limit raise or better via the cuebid.
            ("1H 1S", "K3.KJ4.AQ32.8743", "2S"),
            // 1C-(1H): 2NT with a heart stopper and 12 hcp.
            ("1C 1H", "A8.K654.K742.Q94", "2NT"),
            // 1D-(2S) (weak jump overcall): negative double with long hearts.
            ("P P 1D 2S", "Q9.KQT85432.K3.J", "X"),
            // 1C-(2H): a 5-count passes.
            ("1C 2H", "A54.86.JT6532.94", "P"),
            // 1D-(2C): a new major at the two level, forcing.
            ("1D 2C", "AQ432.K2.Q32.432", "2S"),
            // 1H-(2C): negative double without heart support.
            ("1H 2C", "K32.A2.Q432.J432", "X"),
            // 1S-(2H): 3NT with a heart stopper and no spade fit.
            ("1S 2H", "Q2.K32.KJ32.A432", "3NT"),
            // 1S-(3H): game raise with three-card support.
            ("1S 3H", "K32.43.AQ32.K432", "4S"),
            // 1C-(1NT): a weak hand with a long suit escapes to the two level.
            ("1C 1NT", "QJ8742.85.T763.4", "2S"),
            // 1C-(1NT): penalty double.
            ("1C 1NT", "AQ87.J2.QJT3.A62", "X"),
        ]);
    }

    /// Responses after RHO doubles our opening: a single raise (missing before), 2NT as a limit
    /// raise or better with four trumps (it used to be `INV, 10+ hcp` with no fit, identical to
    /// the redouble and unreachable behind it), the redouble without a fit, and a jump raise
    /// with four trumps (it used to require six).
    #[test]
    fn responses_to_a_double_of_our_opening() {
        check(&[
            ("1H X", "K32.Q43.J432.432", "2H"),
            ("1H X", "K32.Q432.J432.32", "3H"),
            ("1S X", "K432.32.AQ32.K32", "2NT"),
            ("1C X", "KJ32.Q32.K32.A32", "XX"),
            ("1C X", "KJ32.Q2.K32.A432", "2NT"),
            ("1S X", "Q432.2.J9432.K32", "3S"),
        ]);
    }
    /// harness_review.json #4: advancing a takeout double with a weak hand and no four-card
    /// unbid suit bids the cheapest unbid suit with three cards instead of converting the
    /// double for penalties with a few small trumps.
    #[test]
    fn takeout_double_minimum_advance_without_a_four_card_suit() {
        check(&[
            ("1C X P", "K42.J72.Q93.8765", "1D"),
            ("1H X P", "K42.8765.Q93.J76", "1S"),
            ("1S X P", "Q432.J72.K93.765", "2C"),
        ]);
    }

    /// Advancing a one-level overcall had only the cuebid, so every other hand passed: raises,
    /// notrump with a stopper, and the cuebid as a limit raise or better. After the cuebid the
    /// overcaller always has a call.
    #[test]
    fn advances_of_an_overcall_and_the_overcallers_rebid() {
        check(&[
            ("1C 1H P", "K32.Q32.K432.432", "2H"),
            ("1C 1H P", "K32.Q2.K432.Q432", "1NT"),
            ("1C 1S P", "KJ32.A2.K432.432", "2C"),
            ("1C 1H P 2C P", "K2.AJ832.Q32.432", "2H"),
            ("1C 1H P 2C P", "A2.AKJ32.K32.432", "3H"),
            ("1C 1H X 2C P", "K2.AJ832.Q32.432", "2H"),
        ]);
    }

    /// The sandwich position (both opponents have bid, partner passed) resolved against the
    /// balancing table "as if" responder had passed, so most hands had no call: a natural
    /// overcall between the opponents' suits, a takeout double of the two unbid suits, and a
    /// pass otherwise.
    #[test]
    fn sandwich_position_after_opening_and_response() {
        check(&[
            ("P 1D P 1S", "32.AQJ732.K2.K32", "2H"),
            ("1C P 1H", "AQ32.32.KJ32.K32", "X"),
            ("P P P 1S P 2C", "T72.K96.QT75.743", "P"),
            ("1S P 3S", "2.AJ62.KQ86.A753", "X"),
            ("1S P 3S", "98.AT6532.54.Q76", "P"),
        ]);
    }

    /// Interference the file does not model reverts to natural bidding (the booklet's own rule)
    /// instead of being read against the uncontested table, where a weak hand had no call.
    /// Also the natural defense to a 1NT opening.
    #[test]
    fn unmodeled_interference_is_natural_and_1nt_defense() {
        check(&[
            ("1C 1D 2C", "T53.983.K8.J7632", "P"),
            ("1NT", "AQJ32.K32.432.32", "2S"),
            ("1NT", "AQ3.KJ3.AQ32.K32", "X"),
        ]);
    }
    /// dropped.json #19 (minor single raise): 1NT (6--9) and the single raise of a minor
    /// (6--10) shared a range and 1NT came first, so a 6--9 hand with a fit never raised.
    #[test]
    fn minor_single_raise_is_reachable() {
        check(&[
            ("1C P", "Q3.963.64.KJ9763", "2C"),
            ("1D P", "T85..K6432.KT965", "2D"),
            ("1D P", "J62.T64.872.KQT9", "1NT"),
        ]);
    }

    /// Responder's second call after opener's 1NT rebid: the only rows were a few new suits,
    /// so a hand with game values or an invitation passed 1NT via the implicit pass.
    #[test]
    fn responder_rebid_after_opener_rebids_1nt() {
        check(&[
            ("1C P 1H P 1NT P", "A9.AJ43.AQ2.KQ86", "3NT"),
            ("1H P 1S P 1NT P", "AQ975.K2.A43.Q86", "3NT"),
            ("1H P 1S P 1NT P", "KQ85.Q4.KJ32.T84", "2NT"),
            ("1H P 1S P 1NT P", "KJ9854.Q2.K73.84", "2S"),
            ("1C P 1S P 1NT P", "KQ854.J972.83.J5", "2H"),
            ("1D P 1S P 1NT P", "AJ75.7.8632.JT92", "2D"),
            ("1D P 1S P 1NT P", "AJ75.Q72.832.T92", "P"),
        ]);
    }

    /// Responses to a minor opening that had no call: the invitational 3=3=3=4 hand over 1C
    /// (no four-card suit to show, too strong for 1NT, too weak for 2NT) raises with four
    /// clubs, and a hand above the 16--18 3NT with no four-card major bids 3NT instead of
    /// passing partner's opening.
    #[test]
    fn minor_opening_responses_cover_every_strength() {
        check(&[
            ("1C P", "K74.Q53.875.AQJ2", "3C"),
            ("1C P", "AT8.A5.A9.AKT762", "3NT"),
            ("1D P", "QJ.A64.AKQ82.A72", "3NT"),
        ]);
    }

    /// Advancing a natural two-level overcall of the opponents' 1NT had no table, and natural
    /// inference offered nothing to a hand of middling strength (`NoCandidate`): a raise with
    /// a fit, game with an opening hand, and a raise or penalty double after opener's partner
    /// competes.
    #[test]
    fn advances_of_an_overcall_of_their_1nt() {
        check(&[
            ("1NT 2C P", "K43.76.K82.Q8654", "3C"),
            ("1NT 2C P", "AQ87.J2.QJT3.A62", "3NT"),
            ("1NT 2H P", "A3.K952.AQ832.95", "4H"),
            ("1NT 2C P", "KT832.QT5.7.JT96", "P"),
            ("1NT 2D 2S", "K43.76.KQ82.Q865", "3D"),
            ("1NT 2S 3C", "A3.K952.AQ832.95", "X"),
        ]);
    }

    /// Opener's rebid after a forcing new suit by responder over an overcall had no table, and
    /// natural inference had no call for many openers (`NoCandidate`): a one-level response
    /// now uses the uncontested rebid table for the same two suits, a two-level one a table of
    /// its own (raise, notrump with a stopper, rebid of the opening suit), and a further bid by
    /// the opponents reverts to natural bidding.
    #[test]
    fn opener_rebid_after_a_new_suit_over_an_overcall() {
        check(&[
            ("1C 1D 1S P", "T3.AKQ3.QJ.K8763", "2C"),
            ("1C 1D 1H P", "65.AK.Q653.AKJ94", "2D"),
            ("1D 1H 1S P", "A3.K952.AQ832.95", "2D"),
            ("1C 1H 2D P", "AQ86.KQJ5.5.J932", "2NT"),
            ("1D 1S 2H P", "A854.8.AKT94.J74", "2NT"),
            ("1D 1H 2C P", "3.KQ65.K752.KJ82", "3C"),
            ("1C 1D 1S 3D", "KT87.KQ.Q7.Q9763", "3S"),
        ]);
    }

    /// Opener's call after responder's game-forcing fourth suit (1S-2C-2H-3D) or jump
    /// preference (1S-2C-2H-3S) had no table, so opener could pass a game force.
    #[test]
    fn opener_continues_after_responders_game_force() {
        check(&[
            ("1S P 2C P 2H P 3D P", "KQ965.AJT54.9.A4", "3H"),
            ("1S P 2C P 2H P 3D P", "AQJ76.K87.Q72.K5", "3NT"),
            ("1S P 2C P 2H P 3D P", "AKJ76.Q87.72.KQ5", "4C"),
            ("1S P 2C P 2H P 3S P", "KQ965.AJT54.9.A4", "4S"),
            ("1S P 2C P 2H P 3S P", "AKQ76.AK87.72.K5", "4C"),
        ]);
    }

    /// Advancing an overcall after opener's partner has bid (a new suit, 2NT, the cuebid, a
    /// jump raise), after a 1NT overcall is taken out, after a negative double of a two-level
    /// overcall, and after a minor-suit Michaels cuebid: each had no table, and natural
    /// inference had no call for a hand of middling strength (`NoCandidate`).
    #[test]
    fn advances_after_opener_s_partner_bids() {
        check(&[
            ("1D 1H 2NT", "K865.J974.K75.96", "3H"),
            ("1H 1S 3H", "K865.974.K75.965", "3S"),
            ("1D 1H 2C", "AK65.974.K75.965", "2H"),
            ("1C 1D 2H", "65.974.KJ75.Q965", "3D"),
            ("1C 1NT 2D", "Q865.97.K75.K965", "X"),
            ("1C 1NT 2D", "J865.97.K75.Q965", "P"),
            ("1D 2C X", "65.974.KJ75.Q965", "3C"),
            ("1C 2C P", "8654.Q74.K75.965", "2S"),
            ("1C 2C P", "865.Q74.K753.965", "2H"),
            ("1C 2C P", "K86.A2.KQ75.9652", "3S"),
        ]);
    }

    /// Advancing a sandwich-seat overcall after the opponents bid again: a raise with
    /// three-card support.
    #[test]
    fn advances_of_a_sandwich_overcall() {
        check(&[
            ("1C P 1D 1H 2D", "K865.974.K75.965", "2H"),
            ("1D P 1S 2H 2S", "K65.974.KJ75.965", "3H"),
            ("1D P 1H 2C 2H", "K65.97.KJ75.9652", "3C"),
            ("1H P 2H 2S 3H", "K65.97.KJ75.9652", "3S"),
        ]);
    }
}
