//! `interpret_play`: combining hard and soft constraints over a full deal.

use bridge_core::{Bid, Card, Contract, Doubling, Hand, PlayHistory, Seat, Strain};
use bridge_play::{
    HandConstraint, HonorLeads, LeadStyle, LeadTable, PlayAgreements, Polarity, SignalTable,
    SpotLead, interpret_play,
};

fn card(s: &str) -> Card {
    s.parse().unwrap()
}

fn hand(s: &str) -> Hand {
    s.parse().unwrap()
}

fn only(c: &str) -> Hand {
    Hand::EMPTY.with(card(c))
}

fn play_all(history: &mut PlayHistory, cards: &[&str]) {
    for c in cards {
        history.play(card(c), only(c)).unwrap();
    }
}

/// 4H by South (declarer South, West on lead, East defending alongside West).
fn contract() -> Contract {
    Contract {
        bid: Bid::new(4, Strain::Hearts).unwrap(),
        declarer: Seat::South,
        doubling: Doubling::Undoubled,
    }
}

fn no_agreements() -> PlayAgreements {
    PlayAgreements::default()
}

/// Trick 0: West leads a spot spade (fourth best); East, third hand, follows with a high spot
/// and does not win; South (declarer) wins with the ace.
fn scenario() -> (PlayHistory, [PlayAgreements; 4]) {
    let mut h = PlayHistory::new(Strain::Hearts, Seat::West);
    play_all(&mut h, &["S5", "S2", "S8", "SA"]);

    let mut west = no_agreements();
    west.leads = LeadTable {
        vs_suit: LeadStyle {
            spot: SpotLead::FourthBest,
            honors: HonorLeads::Unknown,
            confidence: 0.8,
        },
        vs_nt: LeadStyle {
            spot: SpotLead::Unknown,
            honors: HonorLeads::Unknown,
            confidence: 0.8,
        },
    };

    let mut east = no_agreements();
    east.signals = SignalTable {
        attitude: Polarity::Standard,
        count: Polarity::Unknown,
        confidence: 0.7,
    };

    let agreements = [no_agreements(), east, no_agreements(), west];
    (h, agreements)
}

#[test]
fn opening_lead_and_attitude_signal_combine() {
    let (h, agreements) = scenario();
    let interp = interpret_play(&h, &contract(), &agreements);

    assert!(interp.warnings.is_empty());

    let west = Seat::West.index() as usize;
    let east = Seat::East.index() as usize;

    assert_eq!(interp.events.len(), 2);
    assert_eq!(interp.events[0].seat, Seat::West);
    assert_eq!(interp.events[0].card, card("S5"));
    assert_eq!(interp.events[0].rule, "lead");
    assert_eq!(interp.events[1].seat, Seat::East);
    assert_eq!(interp.events[1].card, card("S8"));
    assert_eq!(interp.events[1].rule, "signal:attitude");

    // West's opening lead: fourth best from the S5 (len >= 4, exactly 3 cards above it).
    assert_eq!(interp.soft[west].len(), 2);
    let total: f32 = interp.soft[west].iter().map(|(_, w)| w).sum();
    assert!((total - 1.0).abs() < 1e-6);
    assert!((interp.soft[west][0].1 - 0.8).abs() < 1e-6);
    assert!(interp.soft[west][0].0.satisfies(hand("AKQ532.AKQ.43.32")));
    assert!(!interp.soft[west][0].0.satisfies(hand("AKQJ53.AKQ.43.32")));

    // East's attitude signal on the S8 (a high spot): shows an honour in spades (encourage).
    assert_eq!(interp.soft[east].len(), 2);
    assert!((interp.soft[east][0].1 - 0.7).abs() < 1e-6);
    assert!(interp.soft[east][0].0.satisfies(hand("K8642.AK.AKQ.AKQ")));
    assert!(!interp.soft[east][0].0.satisfies(hand("8642.AKQ.AKQ.AKQ")));

    // `into_seats` combines hard and soft: West's opening lead also carries the hard fact that
    // West has played (and so holds) at least one spade.
    let seats = interp.into_seats();
    assert_eq!(seats[west].len(), 2);
    assert!(seats[west][0].0.satisfies(hand("AKQ532.AKQ.43.32")));
    // A hand with the wrong number of cards above the led S5 fails even though it holds the
    // hard-constrained spade.
    assert!(!seats[west][0].0.satisfies(hand("AKQJ53.AKQ.43.32")));
}

/// Declarer and dummy never carry agreements or events, regardless of what happens at the table.
#[test]
fn declarer_side_has_no_events() {
    let (h, agreements) = scenario();
    let interp = interpret_play(&h, &contract(), &agreements);

    for seat in [Seat::North, Seat::South] {
        let si = seat.index() as usize;
        assert!(interp.events.iter().all(|e| e.seat != seat));
        assert_eq!(interp.soft[si].len(), 1);
        assert_eq!(interp.soft[si][0].1, 1.0);
        match &interp.soft[si][0].0 {
            HandConstraint::Atom(a) => assert_eq!(*a, bridge_constraint::Atom::ANY),
            other => panic!("expected ANY, got {other:?}"),
        }
    }
}
