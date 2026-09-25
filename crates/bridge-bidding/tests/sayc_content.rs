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
