//! Card play: legality, revokes, trick winners and whole-deal walks.

use bridge_core::{Card, Deal, Hand, PlayError, PlayHistory, Seat, Side, Strain, Suit};
use proptest::prelude::*;
use proptest::sample::Index;

mod common;
use common::arb_deal;

fn card(s: &str) -> Card {
    s.parse().unwrap()
}

/// A hand holding just `c`: enough when only the trick mechanics are under test (a player
/// with no other card of the led suit may play anything).
fn only(c: &str) -> Hand {
    Hand::EMPTY.with(card(c))
}

fn play_all(history: &mut PlayHistory, cards: &[&str]) {
    for c in cards {
        history.play(card(c), only(c)).unwrap();
    }
}

#[test]
fn trump_beats_led_suit_and_off_suit_never_wins() {
    let mut h = PlayHistory::new(Strain::Hearts, Seat::North);
    play_all(&mut h, &["S5", "SA", "H2", "SK"]);
    assert_eq!(h.trick_winner(0), Some(Seat::South));
    assert_eq!(h.trick_leader(1), Seat::South);
    assert_eq!(h.next_to_play(), Seat::South);
    assert_eq!(h.tricks_won(Side::NS), 1);
    assert_eq!(h.tricks_won(Side::EW), 0);

    // Same cards at notrump: the ace of the led suit wins.
    let mut h = PlayHistory::new(Strain::NoTrump, Seat::North);
    play_all(&mut h, &["S5", "SA", "H2", "SK"]);
    assert_eq!(h.trick_winner(0), Some(Seat::East));
    assert_eq!(h.next_to_play(), Seat::East);

    // A high off-suit card never wins.
    let mut h = PlayHistory::new(Strain::NoTrump, Seat::North);
    play_all(&mut h, &["S5", "HA", "S6", "S2"]);
    assert_eq!(h.trick_winner(0), Some(Seat::South));

    // Higher trump beats lower trump; the led suit beats off-suit.
    let mut h = PlayHistory::new(Strain::Clubs, Seat::West);
    play_all(&mut h, &["D9", "C2", "CT", "DA"]);
    assert_eq!(h.trick_winner(0), Some(Seat::East));
    assert_eq!(h.trick_leader(1), Seat::East);
    play_all(&mut h, &["H3", "D5", "H4", "S2"]);
    assert_eq!(h.trick_winner(1), Some(Seat::West));
    assert_eq!(h.tricks_won(Side::EW), 2);
    assert_eq!(h.cards().len(), 8);
}

#[test]
fn revokes_are_rejected() {
    let mut h = PlayHistory::new(Strain::NoTrump, Seat::North);
    h.play(card("SA"), only("SA")).unwrap();
    assert_eq!(h.led_suit(), Some(Suit::Spades));
    let east: Hand = "K2.A..".parse().unwrap();
    assert!(!h.is_legal(card("HA"), east));
    assert_eq!(
        h.play(card("HA"), east),
        Err(PlayError::Revoke { led: Suit::Spades })
    );
    assert_eq!(h.cards().len(), 1);
    assert!(h.is_legal(card("S2"), east));
    h.play(card("S2"), east).unwrap();

    // Void in the suit led: any card goes.
    let south: Hand = ".KQ.J.".parse().unwrap();
    assert!(h.is_legal(card("HK"), south));
    h.play(card("DJ"), south).unwrap();

    // The follow-suit test ignores cards of `remaining` that were already played: a player
    // whose only spade has been played earlier may discard.
    let west: Hand = "A.5..".parse().unwrap(); // SA is already on the table
    assert!(h.is_legal(card("H5"), west));
    h.play(card("H5"), west).unwrap();
    assert_eq!(h.trick_winner(0), Some(Seat::North));
    assert_eq!(h.current_trick(), &[]);
    assert_eq!(h.led_suit(), None);
}

#[test]
fn other_errors() {
    let mut h = PlayHistory::new(Strain::Spades, Seat::East);
    let east: Hand = "AK.2..".parse().unwrap();
    assert_eq!(
        h.play(card("SQ"), east),
        Err(PlayError::NotHeld(card("SQ")))
    );
    h.play(card("SA"), east).unwrap();
    // The same card again, offered with the original (unreduced) hand.
    assert_eq!(
        h.play(card("SA"), east),
        Err(PlayError::AlreadyPlayed(card("SA")))
    );
    assert_eq!(h.next_to_play(), Seat::South);
    assert_eq!(h.seat_at(0), Seat::East);
    assert_eq!(h.seat_at(3), Seat::North);
    assert_eq!(
        PlayError::Revoke { led: Suit::Hearts }.to_string(),
        "must follow suit H"
    );
}

#[test]
fn tricks_view_and_played_by() {
    let mut h = PlayHistory::new(Strain::Diamonds, Seat::South);
    assert_eq!(h.tricks().count(), 0);
    play_all(&mut h, &["C3", "C4", "C5"]);
    let tricks: Vec<_> = h.tricks().collect();
    assert_eq!(tricks.len(), 1);
    assert_eq!(tricks[0].leader, Seat::South);
    assert_eq!(
        tricks[0].cards,
        [Some(card("C3")), Some(card("C4")), Some(card("C5")), None]
    );
    assert_eq!(tricks[0].winner, None);
    assert_eq!(h.trick_winner(0), None);
    assert_eq!(h.current_trick().len(), 3);
    h.play(card("D2"), only("D2")).unwrap();
    let tricks: Vec<_> = h.tricks().collect();
    assert_eq!(tricks.len(), 1);
    assert_eq!(tricks[0].winner, Some(Seat::East));
    play_all(&mut h, &["HA"]);
    assert_eq!(h.tricks().count(), 2);
    assert_eq!(h.tricks().nth(1).unwrap().leader, Seat::East);
    assert_eq!(h.played_by(Seat::South), Hand::EMPTY.with(card("C3")));
    assert_eq!(
        h.played_by(Seat::East),
        Hand::EMPTY.with(card("D2")).with(card("HA"))
    );
    assert_eq!(h.played_by(Seat::West), Hand::EMPTY.with(card("C4")));
    assert_eq!(
        h.played(),
        "..2.543".parse::<Hand>().unwrap().with(card("HA"))
    );
    assert_eq!(h.trump(), Strain::Diamonds);
    assert_eq!(h.leader(), Seat::South);
}

/// Plays a whole deal with random legal cards.
fn play_random(deal: Deal, trump: Strain, leader: Seat, picks: &[Index]) -> PlayHistory {
    let mut h = PlayHistory::new(trump, leader);
    for ix in picks.iter().take(52) {
        let seat = h.next_to_play();
        let remaining = deal.hand(seat).difference(h.played_by(seat));
        let legal: Vec<Card> = remaining
            .cards()
            .filter(|c| h.is_legal(*c, remaining))
            .collect();
        assert!(!legal.is_empty(), "no legal card for {seat}");
        // Following suit must be possible whenever the suit is held.
        if let Some(led) = h.led_suit() {
            if !remaining.holding(led).is_empty() {
                assert!(legal.iter().all(|c| c.suit() == led));
            }
        }
        h.play(legal[ix.index(legal.len())], remaining).unwrap();
    }
    h
}

proptest! {
    #[test]
    fn random_deals_play_to_the_end(
        deal in arb_deal(),
        trump in 0u8..5,
        leader in 0u8..4,
        picks in prop::collection::vec(any::<Index>(), 52),
    ) {
        let trump = Strain::from_index(trump);
        let leader = Seat::from_index(leader);
        let h = play_random(deal, trump, leader, &picks);
        prop_assert_eq!(h.cards().len(), 52);
        prop_assert_eq!(h.tricks().count(), 13);
        prop_assert!(h.tricks().all(|t| t.winner.is_some()));
        prop_assert_eq!(h.tricks_won(Side::NS) + h.tricks_won(Side::EW), 13);
        prop_assert_eq!(h.played(), Hand::FULL);
        for seat in Seat::ALL {
            prop_assert_eq!(h.played_by(seat), deal.hand(seat));
        }
        for t in 0..13 {
            let winner = h.trick_winner(t).unwrap();
            if t < 12 {
                prop_assert_eq!(h.trick_leader(t + 1), winner);
            }
            prop_assert_eq!(h.seat_at(4 * t), h.trick_leader(t));
        }
        prop_assert_eq!(h.next_to_play(), h.trick_winner(12).unwrap());
        prop_assert_eq!(h.trick_leader(0), leader);
        let any = Card::from_index(0).unwrap();
        prop_assert_eq!(h.clone().play(any, Hand::FULL), Err(PlayError::Complete));
        prop_assert!(!h.is_legal(any, Hand::FULL));
    }
}
