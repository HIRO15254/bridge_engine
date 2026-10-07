//! `KnownCards` validation and `with_play`.

use bridge_constraint::{KnownCards, KnownCardsError};
use bridge_core::{Card, Hand, PlayHistory, Rank, Seat, Strain, Suit};

fn card(suit: Suit, rank: Rank) -> Card {
    Card::new(suit, rank)
}

#[test]
fn empty_is_valid_and_has_the_full_pool() {
    let known = KnownCards::EMPTY;
    assert_eq!(known.pool(), Hand::FULL);
    for seat in Seat::ALL {
        assert_eq!(known.needed(seat), 13);
    }
}

#[test]
fn from_viewer_and_with_dummy_compose() {
    let north_hand = Hand::EMPTY
        .with(card(Suit::Spades, Rank::Ace))
        .with(card(Suit::Spades, Rank::King));
    let south_hand = Hand::EMPTY.with(card(Suit::Hearts, Rank::Ace));

    let known =
        KnownCards::from_viewer(Seat::North, north_hand).with_dummy(Seat::South, south_hand);

    assert_eq!(known.known[Seat::North.index() as usize], north_hand);
    assert_eq!(known.known[Seat::South.index() as usize], south_hand);
    assert_eq!(known.needed(Seat::North), 13 - north_hand.len());
    assert_eq!(known.needed(Seat::South), 13 - south_hand.len());
    assert!(known.pool().is_disjoint(north_hand.union(south_hand)));
    assert_eq!(
        known.pool(),
        Hand::FULL.difference(north_hand.union(south_hand))
    );
}

#[test]
fn new_accepts_pairwise_disjoint_hands_of_at_most_13() {
    let mut hands = [Hand::EMPTY; 4];
    // Deal the 52 cards out 13 at a time in card-index order: pairwise disjoint by construction.
    for i in 0..52u8 {
        let c = Card::from_index(i).expect("valid index");
        hands[(i / 13) as usize] = hands[(i / 13) as usize].with(c);
    }
    let known = KnownCards::new(hands).expect("pairwise disjoint, 13 cards each");
    assert_eq!(known.pool(), Hand::EMPTY);
    for seat in Seat::ALL {
        assert_eq!(known.needed(seat), 0);
    }
}

#[test]
fn new_rejects_a_duplicate_card() {
    let ace_of_spades = card(Suit::Spades, Rank::Ace);
    let mut hands = [Hand::EMPTY; 4];
    hands[0] = hands[0].with(ace_of_spades);
    hands[1] = hands[1].with(ace_of_spades);
    let err = KnownCards::new(hands).expect_err("north and east share the ace of spades");
    assert_eq!(err, KnownCardsError::Duplicate(ace_of_spades));
}

#[test]
fn new_rejects_more_than_13_known_cards_for_one_seat() {
    let mut hands = [Hand::EMPTY; 4];
    for i in 0..14u8 {
        hands[0] = hands[0].with(Card::from_index(i).expect("valid index"));
    }
    let err = KnownCards::new(hands).expect_err("north has 14 known cards");
    assert_eq!(
        err,
        KnownCardsError::TooMany {
            seat: Seat::North,
            count: 14,
        }
    );
}

#[test]
fn with_play_adds_every_seat_s_played_cards() {
    // North leads a spade trick: N-E-S-W each play one card.
    let mut history = PlayHistory::new(Strain::NoTrump, Seat::North);
    let plays = [
        (Seat::North, card(Suit::Spades, Rank::Two)),
        (Seat::East, card(Suit::Spades, Rank::Three)),
        (Seat::South, card(Suit::Spades, Rank::Four)),
        (Seat::West, card(Suit::Spades, Rank::Five)),
    ];
    for (seat, c) in plays {
        let remaining = Hand::EMPTY.with(c);
        history.play(c, remaining).unwrap_or_else(|e| {
            panic!("{seat:?} playing {c:?} should be legal: {e}");
        });
    }

    let known = KnownCards::EMPTY.with_play(&history);
    for (seat, c) in plays {
        assert!(
            known.known[seat.index() as usize].contains(c),
            "{seat:?} should be known to have played {c:?}"
        );
    }
    assert_eq!(known.pool(), Hand::FULL.difference(history.played()));
}

#[test]
fn with_play_unions_with_existing_known_cards() {
    let north_hand = Hand::EMPTY.with(card(Suit::Clubs, Rank::Two));
    let mut history = PlayHistory::new(Strain::NoTrump, Seat::East);
    let east_card = card(Suit::Hearts, Rank::Seven);
    history
        .play(east_card, Hand::EMPTY.with(east_card))
        .expect("East may lead any card");

    let known = KnownCards::from_viewer(Seat::North, north_hand).with_play(&history);
    assert!(known.known[Seat::North.index() as usize].contains(card(Suit::Clubs, Rank::Two)));
    assert!(known.known[Seat::East.index() as usize].contains(east_card));
}
