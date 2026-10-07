//! Coverage for `Position::trick` (docs/design/10-dds.md §7): `position_deal`'s length and
//! duplicate-card checks, and clearing the trick's cards from their owner's holding before
//! calling `SolveBoard`. Every other FFI test (`differential.rs`, `concurrency.rs`, the facade's
//! own test) only ever passes `trick: &[]` (an opening-lead position), so none of them exercise
//! this code at all.

#![cfg(dds_vendored)]

use bridge_core::{Card, Deal, Hand, PlayHistory, Seat, Strain};
use bridge_dds::{Mode, Position, Solutions, Target, solve_board};

/// A fixed, valid deal (a formula shuffle, like `concurrency.rs`'s `sample_deals`; not a real
/// hand).
fn deal() -> Deal {
    let mut hands = [Hand::EMPTY; 4];
    for i in 0..52u8 {
        let card = Card::from_index(i).expect("i < 52");
        let seat = (i % 4) as usize;
        hands[seat] = hands[seat].with(card);
    }
    Deal::new(hands).expect("four 13-card hands covering the deck")
}

/// Plays `count` (1..=3) legal cards to the first trick led by `Seat::North`: the leader's
/// lowest card, then each following player's lowest card of the suit led if they hold it, else
/// their lowest card overall. Returns the played cards in play order.
fn play_first_trick(deal: &Deal, trump: Strain, count: usize) -> Vec<Card> {
    assert!((1..=3).contains(&count), "count must be 1..=3, got {count}");
    let mut history = PlayHistory::new(trump, Seat::North);
    let mut played = Vec::with_capacity(count);
    for _ in 0..count {
        let actor = history.next_to_play();
        let hand = deal.hand(actor);
        let remaining = hand.difference(history.played());
        let card = match history.led_suit() {
            Some(led) if !remaining.holding(led).is_empty() => remaining
                .cards()
                .filter(|c| c.suit() == led)
                .min()
                .expect("remaining holds the led suit"),
            _ => remaining.cards().min().expect("remaining is non-empty"),
        };
        history
            .play(card, hand)
            .unwrap_or_else(|e| panic!("playing {card} for {actor:?}: {e}"));
        played.push(card);
    }
    played
}

/// The seat to play next after `trick` (0..=3 cards) has been led by `leader`.
fn next_to_play(leader: Seat, trick: &[Card]) -> Seat {
    leader.offset(trick.len() as u8)
}

/// `solve_board` with 1, 2 or 3 cards already played to the first trick returns candidates that
/// are (a) actually held by the seat to play next and (b) not among the cards already played to
/// that trick: exactly the two things `position_deal`'s trick handling (clearing the played
/// cards' bits from `remainCards`) is supposed to guarantee, and that DDS's own fault checks
/// (`RETURN_SUIT_OR_RANK`) would not reject a well-formed but wrongly-converted trick.
#[test]
fn solve_board_with_partial_trick_is_consistent() {
    let deal = deal();
    let trump = Strain::NoTrump;
    let leader = Seat::North;

    for count in 1..=3 {
        let trick = play_first_trick(&deal, trump, count);
        let pos = Position {
            deal: &deal,
            trump,
            leader,
            trick: &trick,
        };
        let ft = solve_board(&pos, Target::Max, Solutions::AllRanked, Mode::Auto)
            .unwrap_or_else(|e| panic!("solve_board with {count}-card trick {trick:?}: {e}"));
        assert!(
            !ft.cards.is_empty(),
            "solve_board returned no candidates for {count}-card trick {trick:?}"
        );

        let actor = next_to_play(leader, &trick);
        for scored in &ft.cards {
            assert!(
                !trick.contains(&scored.card),
                "solve_board offered already-played card {} back as a candidate for \
                 {count}-card trick {trick:?}",
                scored.card
            );
            assert_eq!(
                deal.owner(scored.card),
                actor,
                "solve_board offered {} (held by {:?}) as a candidate for {actor:?} to play, \
                 {count}-card trick {trick:?}",
                scored.card,
                deal.owner(scored.card),
            );
        }
    }
}

/// `position_deal`'s length check: DDS itself only ever sees 0..=3 cards in a trick
/// (`currentTrickSuit`/`currentTrickRank` are `[c_int; 3]`), so a 4-card trick must be rejected
/// by this wrapper before any FFI call, with the same error DDS's own fault-checking code uses
/// for a malformed trick (`dll.h`'s `RETURN_SUIT_OR_RANK`, "currentTrickSuit or
/// currentTrickRank has wrong data").
#[test]
fn solve_board_rejects_a_four_card_trick() {
    let deal = deal();
    let trick = play_first_trick(&deal, Strain::NoTrump, 3);
    let mut trick = trick;
    // A fifth seat cannot exist, so the fourth card just has to be *some* unplayed card; which
    // one does not matter, only that `trick.len() == 4` is what must be rejected.
    let fourth = deal
        .hand(Seat::West)
        .cards()
        .find(|c| !trick.contains(c))
        .expect("West holds an unplayed card");
    trick.push(fourth);

    let pos = Position {
        deal: &deal,
        trump: Strain::NoTrump,
        leader: Seat::North,
        trick: &trick,
    };
    let err = solve_board(&pos, Target::Max, Solutions::AllRanked, Mode::Auto)
        .expect_err("a 4-card trick must be rejected");
    assert!(
        err.to_string().contains("currentTrickSuit"),
        "unexpected error for a 4-card trick: {err}"
    );
}

/// `position_deal`'s duplicate check: the same card cannot have been played twice in one trick
/// (`dll.h`'s `RETURN_DUPLICATE_CARDS`), and this wrapper must catch that itself rather than
/// build a `deal` DDS would fault on for an unrelated reason.
#[test]
fn solve_board_rejects_a_duplicate_card_in_the_trick() {
    let deal = deal();
    let led = deal
        .hand(Seat::North)
        .cards()
        .min()
        .expect("North holds a card");
    let trick = [led, led];

    let pos = Position {
        deal: &deal,
        trump: Strain::NoTrump,
        leader: Seat::North,
        trick: &trick,
    };
    let err = solve_board(&pos, Target::Max, Solutions::AllRanked, Mode::Auto)
        .expect_err("a duplicated card in the trick must be rejected");
    assert!(
        err.to_string().contains("duplicated"),
        "unexpected error for a duplicate card: {err}"
    );
}
