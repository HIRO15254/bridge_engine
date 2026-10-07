//! `hard_constraints`: show-out sequences, dummy exposure, revoke detection.

use bridge_core::{Card, Hand, PlayHistory, Seat, Strain, Suit};
use bridge_play::{PlayWarning, hard_constraints};

fn card(s: &str) -> Card {
    s.parse().unwrap()
}

/// A hand holding just `c`: enough to drive the trick mechanics without a real 13-card deal
/// (same trick bridge-core's own `tests/play.rs` uses).
fn only(c: &str) -> Hand {
    Hand::EMPTY.with(card(c))
}

fn play_all(history: &mut PlayHistory, cards: &[&str]) {
    for c in cards {
        history.play(card(c), only(c)).unwrap();
    }
}

/// The design doc §6 worked example: 4H by South, West on lead.
#[test]
fn hard_constraints_from_showout() {
    let mut h = PlayHistory::new(Strain::Hearts, Seat::West);
    play_all(&mut h, &["SK", "S4", "S2", "SA"]); // South wins with the ace, leads next.
    play_all(&mut h, &["HA", "H3", "H5", "C2"]); // East cannot follow hearts: shows out.

    let (hard, known, warnings) = hard_constraints(&h);
    assert!(warnings.is_empty());

    // East is now confirmed void in hearts (played 0, shown out).
    assert_eq!(
        hard[Seat::East.index() as usize].suit_len(Suit::Hearts),
        0..=0
    );
    // Every seat has played at least one spade and (except East) at least one heart.
    for seat in [Seat::North, Seat::South, Seat::West] {
        assert_eq!(
            *hard[seat.index() as usize].suit_len(Suit::Hearts).start(),
            1
        );
    }
    for seat in Seat::ALL {
        assert_eq!(
            *hard[seat.index() as usize].suit_len(Suit::Spades).start(),
            1
        );
    }

    // The known cards are exactly what has been played so far.
    let expected_east = Hand::EMPTY.with(card("S2")).with(card("C2"));
    assert_eq!(known.known[Seat::East.index() as usize], expected_east);
    assert_eq!(known.pool().len(), 52 - 8);
}

/// A shown-out suit fixes the original length exactly, not just a lower bound.
#[test]
fn showout_fixes_exact_length_not_just_a_minimum() {
    let mut h = PlayHistory::new(Strain::NoTrump, Seat::North);
    play_all(&mut h, &["S2", "H3", "S4", "S5"]); // East cannot follow spades.

    let (hard, _known, warnings) = hard_constraints(&h);
    assert!(warnings.is_empty());
    assert_eq!(
        hard[Seat::East.index() as usize].suit_len(Suit::Spades),
        0..=0
    );
    // A seat that HAS followed suit only gets a lower bound, not an exact length.
    assert_eq!(
        hard[Seat::North.index() as usize].suit_len(Suit::Spades),
        1..=13
    );
}

/// A seat that plays a suit it had earlier shown out of is flagged as a suspected revoke; the
/// constraints are still returned (the caller treats the result as `EmptySupport`).
#[test]
fn revoke_suspected_when_a_shown_out_suit_is_replayed() {
    let mut h = PlayHistory::new(Strain::NoTrump, Seat::North);
    // Trick 0: East cannot follow spades (led by North), so `shown_out[East][Spades]`.
    play_all(&mut h, &["S2", "H3", "S4", "S5"]);
    // Trick 1 (West on lead, having won trick 0 with the highest spade): East now plays a spade
    // again, contradicting the earlier show-out.
    h.play(card("H2"), only("H2")).unwrap();
    h.play(card("H6"), only("H6")).unwrap();
    h.play(card("SK"), only("SK")).unwrap(); // East, revoke.

    let (hard, _known, warnings) = hard_constraints(&h);
    assert_eq!(
        warnings,
        vec![PlayWarning::RevokeSuspected {
            trick: 1,
            seat: Seat::East
        }]
    );
    // The mechanically-computed (self-contradictory) length still comes back: East is credited
    // with exactly the one spade played after the show-out.
    assert_eq!(
        hard[Seat::East.index() as usize].suit_len(Suit::Spades),
        1..=1
    );
}

/// `KnownCards::with_play`'s result composes with the viewer's own hand and the exposed dummy
/// (design doc §5 item 2), which `hard_constraints` itself does not add.
#[test]
fn known_cards_compose_with_viewer_and_dummy() {
    let mut h = PlayHistory::new(Strain::Spades, Seat::West);
    play_all(&mut h, &["S2", "S3", "S4", "S5"]);

    let (_hard, known, _warnings) = hard_constraints(&h);
    // Before this call, `known` only has the played cards.
    for seat in Seat::ALL {
        assert_eq!(known.known[seat.index() as usize].len(), 1);
    }

    // North's original hand must include the S3 it has already played.
    let north_hand: Hand = "AKQ3.AKQ.AKQ.AKQ".parse().unwrap();
    let composed = known
        .with_dummy(Seat::North, north_hand)
        .with_dummy(Seat::North, north_hand); // idempotent (union)
    assert_eq!(composed.known[Seat::North.index() as usize], north_hand);
    // West's own played card is untouched by adding the dummy.
    assert_eq!(
        composed.known[Seat::West.index() as usize],
        Hand::EMPTY.with(card("S2"))
    );
}
