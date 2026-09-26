//! End to end: sampling in the middle of the play (phase 5, tasks 5.1 and 5.8).
//!
//! A real deal is played legally for seven tricks and one card of the eighth. On the way East
//! shows out of trumps and South shows out of hearts. `interpret_play` turns that history into
//! known cards, hard length constraints and soft lead/signal constraints. `ConstraintProposal`
//! then samples from the viewer's point of view (declarer, and a defender), and every sampled
//! deal must be consistent with the play so far:
//!
//! - each seat holds every card it has played, and the viewer's own hand and the dummy are fixed;
//! - a shown-out suit has exactly the length the seat has played of it;
//! - replaying the history against the sampled hands is legal (in particular nobody revoked);
//! - the hard constraints hold, and every importance weight is finite.

use bridge_bidding::{
    CallExplanation, CallInterpretation, Explanation, Interpretation, ResolutionKind,
};
use bridge_constraint::{Atom, HandConstraint, KnownCards, ShapeSet};
use bridge_core::{
    Bid, Call, Card, Contract, Deal, Doubling, Hand, PlayHistory, Seat, Strain, Suit,
};
use bridge_play::{
    DiscardTable, HonorLeads, LeadStyle, LeadTable, PlayAgreements, Polarity, SignalTable,
    SpotLead, interpret_play,
};
use bridge_sample::{ConstraintProposal, SampleContext, SampleOptions, Threads, sample_deals};

fn card(s: &str) -> Card {
    s.parse().unwrap()
}

fn hand(s: &str) -> Hand {
    s.parse().unwrap()
}

fn idx(seat: Seat) -> usize {
    seat.index() as usize
}

/// The deal (hands as `S.H.D.C`), 4S by South, West on lead.
///
/// ```text
///            North  KQ43.A72.KQ5.864
/// West  765.QJ96.J42.KQ3        East  -.T843.T986.JT975
///            South  AJT982.K5.A73.A2
/// ```
fn deal() -> Deal {
    let mut hands = [Hand::EMPTY; 4];
    hands[idx(Seat::North)] = hand("KQ43.A72.KQ5.864");
    hands[idx(Seat::East)] = hand("-.T843.T986.JT975");
    hands[idx(Seat::South)] = hand("AJT982.K5.A73.A2");
    hands[idx(Seat::West)] = hand("765.QJ96.J42.KQ3");
    Deal::new(hands).unwrap()
}

fn contract() -> Contract {
    Contract {
        bid: Bid::new(4, Strain::Spades).unwrap(),
        declarer: Seat::South,
        doubling: Doubling::Undoubled,
    }
}

/// The play so far, every card checked for legality against the real deal.
///
/// | Trick | Lead | Cards (in turn) | Notes |
/// | --- | --- | --- | --- |
/// | 0 | W | H6 H2 H3 HK | fourth best; East's H3 is an attitude signal (low: no honour) |
/// | 1 | S | SA S5 S3 C5 | East shows out of trumps |
/// | 2 | S | S2 S6 SK C7 | West's S5-S6 = odd (count in trumps) |
/// | 3 | N | DK D9 D3 D2 | |
/// | 4 | N | DQ D6 D7 D4 | East's D9-D6 = even, West's D2-D4 = odd (count) |
/// | 5 | N | HA H4 H5 H9 | |
/// | 6 | N | H7 HT C2 HJ | South shows out of hearts; West wins |
/// | 7 | W | CK C4 | in progress |
const PLAY: &[&str] = &[
    "H6", "H2", "H3", "HK", // 0
    "SA", "S5", "S3", "C5", // 1
    "S2", "S6", "SK", "C7", // 2
    "DK", "D9", "D3", "D2", // 3
    "DQ", "D6", "D7", "D4", // 4
    "HA", "H4", "H5", "H9", // 5
    "H7", "HT", "C2", "HJ", // 6
    "CK", "C4", // 7, in progress
];

fn play_history(deal: &Deal) -> PlayHistory {
    let mut h = PlayHistory::new(Strain::Spades, contract().leader());
    for c in PLAY {
        let seat = h.next_to_play();
        h.play(card(c), deal.hand(seat))
            .unwrap_or_else(|e| panic!("{c} by {seat:?} is illegal: {e:?}"));
    }
    h
}

fn agreements() -> [PlayAgreements; 4] {
    let defenders = PlayAgreements {
        leads: LeadTable {
            vs_suit: LeadStyle {
                spot: SpotLead::FourthBest,
                honors: HonorLeads::Standard,
                confidence: 0.8,
            },
            vs_nt: LeadStyle::default(),
        },
        signals: SignalTable {
            attitude: Polarity::Standard,
            count: Polarity::Standard,
            confidence: 0.7,
        },
        discards: DiscardTable::default(),
    };
    let mut a: [PlayAgreements; 4] = Default::default();
    a[idx(Seat::East)] = defenders.clone();
    a[idx(Seat::West)] = defenders;
    a
}

fn explanation() -> Explanation {
    Explanation {
        text: String::new(),
        node: None,
        resolution: ResolutionKind::Exact,
        parts: Vec::new(),
    }
}

/// South opened (12-21 HCP, 2% defensive `ANY`); the other seats carry no auction information.
fn interpretation() -> Interpretation {
    let opening = HandConstraint::Atom(Atom {
        shapes: ShapeSet::ALL,
        hcp: 12..=21,
        cards: Vec::new(),
        eval: Vec::new(),
    });
    let weighted = vec![(opening, 0.98f32), (HandConstraint::ANY, 0.02)];
    let mut seats: [Vec<(HandConstraint, f32, Explanation)>; 4] = Default::default();
    seats[idx(Seat::South)] = weighted
        .iter()
        .map(|(c, w)| (c.clone(), *w, explanation()))
        .collect();
    let alternatives = weighted
        .into_iter()
        .map(|(c, w)| {
            let e = CallExplanation {
                call_index: 0,
                call: Call::Pass,
                node: None,
                kind: ResolutionKind::Exact,
                text: String::new(),
            };
            (c, w, e)
        })
        .collect();
    Interpretation {
        seats,
        per_call: vec![CallInterpretation {
            call_index: 0,
            seat: Seat::South,
            call: Call::Pass,
            kind: ResolutionKind::Exact,
            alternatives,
        }],
        divergence: None,
    }
}

/// `(seat, suit)` pairs a seat has shown out of: it did not follow when the suit was led.
fn show_outs(h: &PlayHistory) -> Vec<(Seat, Suit)> {
    let mut out = Vec::new();
    for trick in h.tricks() {
        let Some(led) = trick.cards[0].map(|c| c.suit()) else {
            continue;
        };
        for i in 1..4u8 {
            if let Some(c) = trick.cards[i as usize] {
                let seat = trick.leader.offset(i);
                if c.suit() != led && !out.contains(&(seat, led)) {
                    out.push((seat, led));
                }
            }
        }
    }
    out
}

fn check_viewer(viewer: Seat) {
    let real = deal();
    let history = play_history(&real);
    let c = contract();
    let interp = interpret_play(&history, &c, &agreements());
    assert!(interp.warnings.is_empty(), "{:?}", interp.warnings);

    // The play fires the lead, East's attitude and three count signals.
    let rules: Vec<(Seat, &str)> = interp.events.iter().map(|e| (e.seat, e.rule)).collect();
    assert_eq!(
        rules,
        vec![
            (Seat::West, "lead"),
            (Seat::East, "signal:attitude"),
            (Seat::West, "signal:count"), // S5-S6 in trumps: odd
            (Seat::East, "signal:count"), // D9-D6: even
            (Seat::West, "signal:count"), // D2-D4: odd
        ]
    );
    // The show-outs fix exact lengths.
    let outs = show_outs(&history);
    assert_eq!(
        outs,
        vec![(Seat::East, Suit::Spades), (Seat::South, Suit::Hearts)]
    );
    assert_eq!(interp.hard[idx(Seat::East)].suit_len(Suit::Spades), 0..=0);
    assert_eq!(interp.hard[idx(Seat::South)].suit_len(Suit::Hearts), 2..=2);
    // The real deal satisfies everything the play says (hard) and the soft rules' main branch.
    for seat in Seat::ALL {
        assert!(
            interp.hard[idx(seat)].satisfies(real.hand(seat)),
            "{seat:?}"
        );
        assert!(
            interp.soft[idx(seat)][0].0.satisfies(real.hand(seat)),
            "{seat:?}"
        );
    }

    // The viewer's knowledge: own hand, the dummy (exposed after the opening lead), the play.
    let dummy = c.dummy();
    let known = KnownCards::from_viewer(viewer, real.hand(viewer))
        .with_dummy(dummy, real.hand(dummy))
        .with_play(&history);
    let known = KnownCards::new(known.known).unwrap();
    assert_eq!(known.known[idx(viewer)], real.hand(viewer));
    assert_eq!(known.known[idx(dummy)], real.hand(dummy));
    // `interpret_play`'s own known cards are the played cards: a subset of the viewer's.
    for seat in Seat::ALL {
        let played = interp.known.known[idx(seat)];
        assert_eq!(known.known[idx(seat)].intersect(played), played);
    }

    let interpretation = interpretation();
    let ctx = SampleContext {
        known,
        interpretation: &interpretation,
        play_constraints: &interp.hard,
        play_soft: Some(&interp.soft),
        bidding: None,
    };
    let n = 300;
    let opts = SampleOptions {
        seed: 0x5eed_0000 + u64::from(viewer.index()),
        threads: Threads::Single,
        ..SampleOptions::default()
    };
    let (deals, report) = sample_deals(&ctx, &ConstraintProposal::default(), n, &opts).unwrap();
    assert_eq!(deals.len(), n, "{report:?}");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    // The soft play constraints are folded into the proposal, so the weights stay even (measured
    // 0.67 for the declarer's view and 0.82 for the defender's at these seeds; the phase 5
    // criterion is 0.5).
    assert!(report.ess_ratio >= 0.5, "{report:?}");

    let mut hidden_hands = std::collections::HashSet::new();
    for wd in &deals {
        let d = wd.deal;
        assert!(wd.log_weight.is_finite(), "{d:?}");
        for seat in Seat::ALL {
            let h = d.hand(seat);
            // Known cards (own hand, dummy, every card played) are where they belong.
            let k = known.known[idx(seat)];
            assert_eq!(h.intersect(k), k, "{seat:?} lost a known card in {d:?}");
            assert_eq!(
                h.intersect(history.played_by(seat)),
                history.played_by(seat)
            );
            assert!(
                interp.hard[idx(seat)].satisfies(h),
                "{seat:?} breaks hard in {d:?}"
            );
        }
        // Show-outs: the seat held exactly what it has played of that suit.
        for &(seat, suit) in &outs {
            assert_eq!(
                d.hand(seat).holding(suit),
                history.played_by(seat).holding(suit),
                "{seat:?} in {suit:?}: {d:?}"
            );
        }
        // Replaying the play against the sampled hands is legal throughout.
        let mut replay = PlayHistory::new(history.trump(), history.leader());
        for &c in history.cards() {
            let seat = replay.next_to_play();
            replay
                .play(c, d.hand(seat))
                .unwrap_or_else(|e| panic!("{c:?} by {seat:?} illegal in {d:?}: {e:?}"));
        }
        let hidden: Vec<Hand> = Seat::ALL
            .into_iter()
            .filter(|&s| s != viewer && s != dummy)
            .map(|s| d.hand(s))
            .collect();
        hidden_hands.insert(hidden);
    }
    // The sampler really is sampling: the hidden hands vary.
    assert!(
        hidden_hands.len() > 10,
        "only {} distinct",
        hidden_hands.len()
    );

    // Consequences of the show-outs for the hidden hands.
    for wd in &deals {
        let d = wd.deal;
        if viewer == Seat::South {
            // East is void in trumps, so the one unseen trump is West's.
            assert!(d.hand(Seat::West).contains(card("S7")), "{d:?}");
        }
        if viewer == Seat::East {
            // South had exactly the HK and H5, so the unseen HQ is West's.
            assert!(d.hand(Seat::West).contains(card("HQ")), "{d:?}");
            assert_eq!(d.hand(Seat::South).holding(Suit::Hearts).len(), 2);
        }
    }
}

/// Declarer's view: dummy North exposed, East and West hidden.
#[test]
fn sampled_deals_are_consistent_with_the_play_declarer_view() {
    check_viewer(Seat::South);
}

/// A defender's view: dummy North exposed, South and West hidden.
#[test]
fn sampled_deals_are_consistent_with_the_play_defender_view() {
    check_viewer(Seat::East);
}
