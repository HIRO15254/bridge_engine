//! Phase 5 review of `bridge-play` against 08-play.md, table-driven: show-outs, revokes and
//! inconsistencies, `KnownCards::with_play`, the lead table (vs suit and vs NT through
//! `interpret_play`), attitude/count signals and first discards read off a real history,
//! defender-only application, and the §7.6 combination (prune against `hard`, K = 8).

use core::ops::RangeInclusive;

use bridge_constraint::{Atom, KnownCards};
use bridge_core::{Bid, Card, Contract, Doubling, Hand, PlayHistory, Seat, Strain, Suit};
use bridge_play::{
    DiscardTable, FirstDiscard, HandConstraint, HonorLeads, LeadStyle, LeadTable, PlayAgreements,
    PlayWarning, Polarity, SignalContext, SignalEvent, SignalKind, SignalTable, SpotLead,
    hard_constraints, interpret_play, lead_constraints, signal_constraints,
};

fn card(s: &str) -> Card {
    s.parse().unwrap()
}

fn hand(s: &str) -> Hand {
    s.parse().unwrap()
}

/// Plays `cards` in order. Each card is passed as the player's only holding, so the follow-suit
/// check never objects: that is what lets these tables record revokes on purpose.
fn history(trump: Strain, leader: Seat, cards: &[&str]) -> PlayHistory {
    let mut h = PlayHistory::new(trump, leader);
    for c in cards {
        h.play(card(c), Hand::EMPTY.with(card(c))).unwrap();
    }
    h
}

fn contract(level: u8, strain: Strain, declarer: Seat) -> Contract {
    Contract {
        bid: Bid::new(level, strain).unwrap(),
        declarer,
        doubling: Doubling::Undoubled,
    }
}

fn idx(seat: Seat) -> usize {
    seat.index() as usize
}

fn is_any(c: &HandConstraint) -> bool {
    matches!(c, HandConstraint::Atom(a) if *a == Atom::ANY)
}

fn total(alts: &[(HandConstraint, f32)]) -> f32 {
    alts.iter().map(|(_, w)| w).sum()
}

// ---------------------------------------------------------------------------------------------
// Hard constraints
// ---------------------------------------------------------------------------------------------

/// (description, trump, opening leader, cards, seat, suit, expected original length)
type ShowOutRow = (
    &'static str,
    Strain,
    Seat,
    &'static [&'static str],
    Seat,
    Suit,
    RangeInclusive<u8>,
);

#[test]
fn show_outs_fix_suit_lengths() {
    let rows: &[ShowOutRow] = &[
        (
            "void on the first round",
            Strain::NoTrump,
            Seat::North,
            &["S2", "H3", "S4", "S5"],
            Seat::East,
            Suit::Spades,
            0..=0,
        ),
        (
            "a seat that followed only gets a lower bound",
            Strain::NoTrump,
            Seat::North,
            &["S2", "H3", "S4", "S5"],
            Seat::West,
            Suit::Spades,
            1..=13,
        ),
        (
            "the discarded suit only gets a lower bound",
            Strain::NoTrump,
            Seat::North,
            &["S2", "H3", "S4", "S5"],
            Seat::East,
            Suit::Hearts,
            1..=13,
        ),
        (
            // T0: N S2, E S3, S H4 (South out), W S5 wins. T1: W S6, N S7, E D2 (East out after
            // one spade), S H5; North wins with the S7.
            "doubleton-free show-out after following once",
            Strain::NoTrump,
            Seat::North,
            &["S2", "S3", "H4", "S5", "S6", "S7", "D2", "H5"],
            Seat::East,
            Suit::Spades,
            1..=1,
        ),
        (
            "the same record: South is void",
            Strain::NoTrump,
            Seat::North,
            &["S2", "S3", "H4", "S5", "S6", "S7", "D2", "H5"],
            Seat::South,
            Suit::Spades,
            0..=0,
        ),
        (
            // 4H: West leads SA, North S2, East S3, South ruffs with the H2.
            "a ruff is a show-out of the suit led",
            Strain::Hearts,
            Seat::West,
            &["SA", "S2", "S3", "H2"],
            Seat::South,
            Suit::Spades,
            0..=0,
        ),
        (
            "the ruffing trump is a lower bound in trumps",
            Strain::Hearts,
            Seat::West,
            &["SA", "S2", "S3", "H2"],
            Seat::South,
            Suit::Hearts,
            1..=13,
        ),
        (
            // Design doc §6 worked example.
            "4H by South, East shows out of trumps",
            Strain::Hearts,
            Seat::West,
            &["SK", "S4", "S2", "SA", "HA", "H3", "H5", "C2"],
            Seat::East,
            Suit::Hearts,
            0..=0,
        ),
        (
            "a show-out in an incomplete trick still counts",
            Strain::NoTrump,
            Seat::North,
            &["S2", "H3"],
            Seat::East,
            Suit::Spades,
            0..=0,
        ),
    ];
    for (what, trump, leader, cards, seat, suit, expected) in rows {
        let h = history(*trump, *leader, cards);
        let (hard, _, warnings) = hard_constraints(&h);
        assert!(warnings.is_empty(), "{what}: {warnings:?}");
        assert_eq!(hard[idx(*seat)].suit_len(*suit), *expected, "{what}");
    }
}

/// (description, trump, leader, cards, expected warnings in any order)
type WarningRow = (
    &'static str,
    Strain,
    Seat,
    &'static [&'static str],
    &'static [PlayWarning],
);

#[test]
fn revokes_and_inconsistencies_are_flagged() {
    let rows: &[WarningRow] = &[
        (
            "a clean record",
            Strain::NoTrump,
            Seat::North,
            &["S2", "S3", "S4", "S5", "S6", "S7", "S8", "S9"],
            &[],
        ),
        (
            // T0: East out of spades; West wins. T1: West H2, North H6, East SK (revoke).
            "a shown-out suit played again",
            Strain::NoTrump,
            Seat::North,
            &["S2", "H3", "S4", "S5", "H2", "H6", "SK"],
            &[PlayWarning::RevokeSuspected {
                trick: 1,
                seat: Seat::East,
            }],
        ),
        (
            // T0: N S2, E H2 (East out), S S3, W S4 wins. T1: W S5, N H3 (North out), E D2, S H5;
            // West wins with the only spade. T2: W H6, N H7, E H8, S H9; South wins. T3: S S7
            // (South had shown out in T1: revoke), W D3 (West out), N D4, E D5. All four seats
            // are now held to exactly the spades they played (1 + 0 + 2 + 2 = 5 < 13).
            "all four seats shown out of one suit",
            Strain::NoTrump,
            Seat::North,
            &[
                "S2", "H2", "S3", "S4", "S5", "H3", "D2", "H5", "H6", "H7", "H8", "H9", "S7", "D3",
                "D4", "D5",
            ],
            &[
                PlayWarning::RevokeSuspected {
                    trick: 3,
                    seat: Seat::South,
                },
                PlayWarning::Inconsistent { suit: Suit::Spades },
            ],
        ),
        (
            // T0: N S2, E H2 (East out of spades), S S3, W S4 wins. T1: W H3, N H4, E C2 (out of
            // hearts), S H5 wins. T2: S C3, W C4, N C5, E D2 (out of clubs); North wins. T3: N
            // D3, E S5 (out of diamonds, and a spade after showing out: revoke), S D4, W D5.
            // East is now held to exactly 1 card in every suit: 4 cards, not 13.
            "one seat shown out of every suit",
            Strain::NoTrump,
            Seat::North,
            &[
                "S2", "H2", "S3", "S4", "H3", "H4", "C2", "H5", "C3", "C4", "C5", "D2", "D3", "S5",
                "D4", "D5",
            ],
            &[
                PlayWarning::RevokeSuspected {
                    trick: 3,
                    seat: Seat::East,
                },
                PlayWarning::Inconsistent { suit: Suit::Clubs },
                PlayWarning::Inconsistent {
                    suit: Suit::Diamonds,
                },
                PlayWarning::Inconsistent { suit: Suit::Hearts },
                PlayWarning::Inconsistent { suit: Suit::Spades },
            ],
        ),
    ];
    for (what, trump, leader, cards, expected) in rows {
        let h = history(*trump, *leader, cards);
        let (hard, _, warnings) = hard_constraints(&h);
        assert_eq!(warnings.len(), expected.len(), "{what}: {warnings:?}");
        for w in *expected {
            assert!(
                warnings.contains(w),
                "{what}: missing {w:?} in {warnings:?}"
            );
        }
        // The same warnings reach `interpret_play`.
        let c = contract(3, Strain::NoTrump, h.leader().offset(3));
        let interp = interpret_play(&h, &c, &Default::default());
        assert_eq!(interp.warnings, warnings, "{what}");
        // Every flagged record leaves some seat or the deal without a consistent shape:
        // the per-seat shape sets alone show it for the "every suit" row.
        if what.contains("every suit") {
            assert!(hard[idx(Seat::East)].shapes().is_empty(), "{what}");
        }
    }
}

#[test]
fn known_cards_from_play_match_the_record() {
    let records: &[(Strain, Seat, &[&str])] = &[
        (Strain::NoTrump, Seat::North, &[]),
        (Strain::NoTrump, Seat::North, &["S2", "H3"]),
        (
            Strain::Hearts,
            Seat::West,
            &["SK", "S4", "S2", "SA", "HA", "H3", "H5", "C2"],
        ),
        (
            Strain::NoTrump,
            Seat::North,
            &["S2", "S3", "H4", "S5", "S6", "S7", "D2", "H5", "SA"],
        ),
    ];
    for (trump, leader, cards) in records {
        let h = history(*trump, *leader, cards);
        let (_, known, _) = hard_constraints(&h);
        assert_eq!(known, KnownCards::EMPTY.with_play(&h));
        for seat in Seat::ALL {
            assert_eq!(
                known.known[idx(seat)],
                h.played_by(seat),
                "{cards:?} {seat:?}"
            );
        }
        // Played cards are disjoint and within 13 per seat, so `KnownCards::new` accepts them.
        assert_eq!(KnownCards::new(known.known).unwrap(), known);
        assert_eq!(known.pool().len() as usize, 52 - cards.len());
        // Each card is credited to the seat `seat_at` says played it.
        for (i, c) in cards.iter().enumerate() {
            assert!(known.known[idx(h.seat_at(i))].contains(card(c)));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Leads
// ---------------------------------------------------------------------------------------------

fn style(spot: SpotLead, honors: HonorLeads) -> LeadStyle {
    LeadStyle {
        spot,
        honors,
        confidence: 0.8,
    }
}

/// (led card, strain, spot convention, honour convention, Some((satisfying, violating)) or None
/// when no rule applies). West is on lead against South.
type LeadRow = (
    &'static str,
    Strain,
    SpotLead,
    HonorLeads,
    Option<(&'static str, &'static str)>,
);

#[test]
fn lead_table() {
    use HonorLeads as H;
    use SpotLead as S;
    let rows: &[LeadRow] = &[
        // 4th best: length >= 4 and exactly 3 cards above the card led.
        (
            "S5",
            Strain::Hearts,
            S::FourthBest,
            H::Unknown,
            Some(("KJ85.AK2.AK2.K32", "K85.AK2.AK32.K32")),
        ),
        (
            "S5",
            Strain::NoTrump,
            S::FourthBest,
            H::Unknown,
            Some(("KJ852.AK.AK2.K32", "KJT95.AK.AK2.K32")),
        ),
        (
            "S2",
            Strain::NoTrump,
            S::FourthBest,
            H::Unknown,
            Some(("KJ52.AK2.AK2.K32", "KJ652.AK.AK2.K32")),
        ),
        // 3rd/5th: the 4-card branch is not in v1 (§11 item 1).
        (
            "S5",
            Strain::Hearts,
            S::ThirdFifth,
            H::Unknown,
            Some(("K85.AK32.AK2.K32", "KJ85.AK2.AK2.K32")),
        ),
        (
            "S5",
            Strain::NoTrump,
            S::ThirdFifth,
            H::Unknown,
            Some(("KJ985.AK.AK2.K32", "KJ85.AK2.AK2.K32")),
        ),
        // Attitude: high denies an honour, low (including the 6) shows one.
        (
            "S8",
            Strain::Hearts,
            S::Attitude,
            H::Unknown,
            Some(("T8632.AK.AK2.K32", "Q8632.AK.AK2.K32")),
        ),
        (
            "S6",
            Strain::Hearts,
            S::Attitude,
            H::Unknown,
            Some(("J8632.AK.AK2.K32", "T8632.AK.AK2.K32")),
        ),
        (
            "S3",
            Strain::NoTrump,
            S::Attitude,
            H::Unknown,
            Some(("A8632.AK.AK2.K32", "T8632.AK.AK2.K32")),
        ),
        // Standard king: AK or KQ vs suit; KQJ/KQT or AKJT vs NT.
        (
            "SK",
            Strain::Hearts,
            S::Unknown,
            H::Standard,
            Some(("AK2.QJ32.K32.432", "KJ2.QJ32.K32.432")),
        ),
        (
            "SK",
            Strain::Hearts,
            S::Unknown,
            H::Standard,
            Some(("KQ2.QJ32.K32.432", "K32.AQJ3.K32.432")),
        ),
        (
            "SK",
            Strain::NoTrump,
            S::Unknown,
            H::Standard,
            Some(("KQT2.QJ3.K32.432", "AKQ2.QJ3.K32.432")),
        ),
        (
            "SK",
            Strain::NoTrump,
            S::Unknown,
            H::Standard,
            Some(("AKJT.QJ3.K32.432", "AKJ2.QJ3.K32.432")),
        ),
        ("SQ", Strain::Hearts, S::Unknown, H::Standard, None),
        ("SA", Strain::NoTrump, S::FourthBest, H::Standard, None),
        // Rusinow: the next higher card.
        (
            "SK",
            Strain::Hearts,
            S::Unknown,
            H::Rusinow,
            Some(("AK2.QJ32.K32.432", "KQ2.QJ32.K32.432")),
        ),
        (
            "SQ",
            Strain::NoTrump,
            S::Unknown,
            H::Rusinow,
            Some(("KQ2.QJ32.K32.432", "AQJ.QJ32.K32.432")),
        ),
        (
            "SJ",
            Strain::Hearts,
            S::Unknown,
            H::Rusinow,
            Some(("QJ2.QJ32.K32.432", "KJT.QJ32.K32.432")),
        ),
        (
            "ST",
            Strain::NoTrump,
            S::Unknown,
            H::Rusinow,
            Some(("JT2.QJ32.K32.432", "QT9.QJ32.K32.432")),
        ),
        (
            "S9",
            Strain::Hearts,
            S::FourthBest,
            H::Rusinow,
            Some(("T92.QJ32.K32.432", "J98.QJ32.K32.432")),
        ),
        // Jack denies: J denies A/K/Q; T shows J + a higher honour, or T9 denying all four.
        (
            "SJ",
            Strain::NoTrump,
            S::Unknown,
            H::JackDenies,
            Some(("JT92.QJ3.K32.432", "QJT2.QJ3.K32.432")),
        ),
        (
            "ST",
            Strain::Hearts,
            S::Unknown,
            H::JackDenies,
            Some(("KJT2.QJ3.K32.432", "JT32.QJ3.K32.432")),
        ),
        (
            "ST",
            Strain::NoTrump,
            S::Unknown,
            H::JackDenies,
            Some(("T932.QJ3.K32.432", "AT92.QJ3.K32.432")),
        ),
        // The nine is a spot lead unless the honour convention claims it (Rusinow); the ten is
        // never a spot.
        ("ST", Strain::Hearts, S::FourthBest, H::Standard, None),
        (
            "S9",
            Strain::Hearts,
            S::FourthBest,
            H::JackDenies,
            Some(("KJT9.AK2.AK2.K32", "K92.AK2.AK32.K32")),
        ),
        ("S5", Strain::Hearts, S::Unknown, H::Rusinow, None),
    ];
    let west_on_lead = Seat::South;
    for (led, strain, spot, honors, expected) in rows {
        let c = contract(4, *strain, west_on_lead);
        let s = style(*spot, *honors);
        let alts = lead_constraints(card(led), &c, &s);
        let what = format!("{led} vs {strain:?} ({spot:?}, {honors:?})");
        match expected {
            None => assert!(alts.is_empty(), "{what}: expected no rule"),
            Some((good, bad)) => {
                assert_eq!(alts.len(), 2, "{what}");
                assert!((total(&alts) - 1.0).abs() < 1e-6, "{what}");
                assert!((alts[0].1 - 0.8).abs() < 1e-6, "{what}");
                assert!(
                    alts[0].0.satisfies(hand(good)),
                    "{what}: {good} should satisfy"
                );
                assert!(!alts[0].0.satisfies(hand(bad)), "{what}: {bad} should not");
            }
        }

        // Through `interpret_play`, the table picks `vs_nt` for notrump and `vs_suit` otherwise:
        // put the style under test in the right slot and a disabled one in the other.
        let (vs_suit, vs_nt) = if *strain == Strain::NoTrump {
            (LeadStyle::default(), s.clone())
        } else {
            (s.clone(), LeadStyle::default())
        };
        let swapped = PlayAgreements {
            leads: LeadTable {
                vs_suit: vs_nt.clone(),
                vs_nt: vs_suit.clone(),
            },
            ..Default::default()
        };
        let west = PlayAgreements {
            leads: LeadTable { vs_suit, vs_nt },
            ..Default::default()
        };
        let h = history(*strain, Seat::West, &[led]);
        let mut agreements: [PlayAgreements; 4] = Default::default();
        agreements[idx(Seat::West)] = west;
        let interp = interpret_play(&h, &c, &agreements);
        assert_eq!(
            interp.events.len(),
            usize::from(expected.is_some()),
            "{what}"
        );
        if let Some((good, bad)) = expected {
            let w = idx(Seat::West);
            assert!(interp.soft[w][0].0.satisfies(hand(good)), "{what}");
            assert!(!interp.soft[w][0].0.satisfies(hand(bad)), "{what}");
        }
        // The other table is never consulted.
        agreements[idx(Seat::West)] = swapped;
        let interp = interpret_play(&h, &c, &agreements);
        assert!(interp.events.is_empty(), "{what}: the wrong table fired");
    }
}

// ---------------------------------------------------------------------------------------------
// Signals and discards read off a history
// ---------------------------------------------------------------------------------------------

fn defenders_agreement(
    attitude: Polarity,
    count: Polarity,
    discard: FirstDiscard,
) -> PlayAgreements {
    PlayAgreements {
        leads: LeadTable {
            vs_suit: style(SpotLead::FourthBest, HonorLeads::Standard),
            vs_nt: style(SpotLead::FourthBest, HonorLeads::Standard),
        },
        signals: SignalTable {
            attitude,
            count,
            confidence: 0.7,
        },
        discards: DiscardTable {
            first: discard,
            polarity: attitude,
        },
    }
}

/// Every seat, declarer's side included, carries the same agreements: only defenders may fire.
fn everyone(a: PlayAgreements) -> [PlayAgreements; 4] {
    [a.clone(), a.clone(), a.clone(), a]
}

/// 4S by South, West on lead. T0: W H5, N H2, E H6 (attitude, the mid card), S HA wins. T1: S
/// D4, W D2, N DK wins, E D9. T2: N DA, E D3 (East high-low: count), S D5, W D6 (West low-high:
/// count). T3: N DQ, E C9 (East's first discard, out of diamonds), S D7, W D8.
const SIGNAL_PLAY: &[&str] = &[
    "H5", "H2", "H6", "HA", "D4", "D2", "DK", "D9", "DA", "D3", "D5", "D6", "DQ", "C9", "D7", "D8",
];

#[test]
fn signals_fire_on_the_right_cards() {
    let h = history(Strain::Spades, Seat::West, SIGNAL_PLAY);
    let c = contract(4, Strain::Spades, Seat::South);
    let a = everyone(defenders_agreement(
        Polarity::Standard,
        Polarity::Standard,
        FirstDiscard::Attitude,
    ));
    let interp = interpret_play(&h, &c, &a);
    assert!(interp.warnings.is_empty());
    let rules: Vec<(Seat, Card, &str)> = interp
        .events
        .iter()
        .map(|e| (e.seat, e.card, e.rule))
        .collect();
    assert_eq!(
        rules,
        vec![
            (Seat::West, card("H5"), "lead"),
            (Seat::East, card("H6"), "signal:attitude"),
            (Seat::East, card("D3"), "signal:count"),
            (Seat::West, card("D6"), "signal:count"),
            (Seat::East, card("C9"), "signal:discard"),
        ]
    );

    let e = idx(Seat::East);
    let w = idx(Seat::West);
    // East: diamonds exactly 2 (hard); high-low = even (consistent); H6 splits; C9 high =
    // a club honour. 3 x 2 x 2 = 12 combinations, capped at K = 8, summing to 1.
    assert_eq!(interp.hard[e].suit_len(Suit::Diamonds), 2..=2);
    assert_eq!(interp.soft[e].len(), 8);
    assert!((total(&interp.soft[e]) - 1.0).abs() < 1e-5);
    // The heaviest East branch is everything that fired, with the H6's "high" half first: a heart
    // honour (0.35), even diamonds (0.7) and a club honour (0.6).
    assert!(interp.soft[e].windows(2).all(|p| p[0].1 >= p[1].1));
    let top = &interp.soft[e][0].0;
    assert!(top.satisfies(hand("K3.Q632.93.AQ932")));
    assert!(!top.satisfies(hand("K3.Q632.932.AQ93")), "odd diamonds");
    assert!(!top.satisfies(hand("K3.T632.93.AQ932")), "no heart honour");
    assert!(!top.satisfies(hand("K3.Q632.93.T9832")), "no club honour");
    // West: 4th best H5 and low-high diamonds (odd): 2 x 2 = 4 branches.
    assert_eq!(interp.soft[w].len(), 4);
    assert!(interp.soft[w][0].0.satisfies(hand("32.KJ85.862.KJ32")));
    assert!(
        !interp.soft[w][0].0.satisfies(hand("32.KJ85.8762.KJ3")),
        "even diamonds"
    );
}

/// Regression: the count signal must read a seat's first two cards of the suit. East's diamonds
/// are J, 9, 3 (three cards, odd): the J comes first (an honour), so the 9-3 that follows is not
/// a high-low from the top of the holding. The rule used to fire on the first two *spot* follows
/// and read "even", which the hard constraint does not contradict until East shows out.
#[test]
fn count_needs_the_first_two_cards_of_the_suit() {
    // 4S by South, West leads the H5. T0: W H5, N H2, E H6, S HA. T1: S D4, W D2, N DA wins, E
    // DJ. T2: N DK, E D9, S D5, W D6; North wins. T3: N DQ, E D3, S D7, W D8.
    let h = history(
        Strain::Spades,
        Seat::West,
        &[
            "H5", "H2", "H6", "HA", "D4", "D2", "DA", "DJ", "DK", "D9", "D5", "D6", "DQ", "D3",
            "D7", "D8",
        ],
    );
    let c = contract(4, Strain::Spades, Seat::South);
    let a = everyone(defenders_agreement(
        Polarity::Unknown,
        Polarity::Standard,
        FirstDiscard::Unknown,
    ));
    let interp = interpret_play(&h, &c, &a);
    let east_counts: Vec<Card> = interp
        .events
        .iter()
        .filter(|e| e.seat == Seat::East && e.rule == "signal:count")
        .map(|e| e.card)
        .collect();
    assert!(east_counts.is_empty(), "fired on {east_counts:?}");
    // West's D2 then D6 are its first two diamonds: that count does fire (low-high = odd).
    assert!(
        interp
            .events
            .iter()
            .any(|e| e.seat == Seat::West && e.rule == "signal:count" && e.card == card("D6"))
    );
}

/// (description, cards, contract strain, expected (seat, card, rule) events for the defenders)
type EventRow = (
    &'static str,
    Strain,
    &'static [&'static str],
    &'static [(Seat, &'static str, &'static str)],
);

#[test]
fn signal_positions_table() {
    let rows: &[EventRow] = &[
        (
            // 3rd hand wins the trick with a spot: no attitude.
            "third hand wins",
            Strain::Spades,
            &["H5", "H2", "H9", "H3"],
            &[(Seat::West, "H5", "lead")],
        ),
        (
            // 3rd hand plays an honour that loses: not a spot, no attitude.
            "third hand plays a losing honour",
            Strain::Spades,
            &["H5", "H2", "HJ", "HA"],
            &[(Seat::West, "H5", "lead")],
        ),
        (
            // Declarer's side led: no attitude, and one spot is not yet count.
            "a single follow to declarer's lead",
            Strain::Spades,
            &["H5", "H2", "HQ", "HA", "D4", "D2", "DA", "D9"],
            &[(Seat::West, "H5", "lead")],
        ),
        (
            // T0 South ruffs: a ruff is not a discard, and declarer's cards never fire.
            // T1: S D2, W D3, N DA, E C2 (East discards, first time).
            "a ruff is not a discard; the next off-suit card is",
            Strain::Spades,
            &["H5", "H2", "H9", "S2", "D2", "D3", "DA", "C2"],
            &[
                (Seat::West, "H5", "lead"),
                (Seat::East, "H9", "signal:attitude"),
                (Seat::East, "C2", "signal:discard"),
            ],
        ),
        (
            // Only the first discard: East discards C2 then C3 (West's D3-D5 is a count).
            "only the first discard fires",
            Strain::Spades,
            &[
                "H5", "H2", "HQ", "HA", "D2", "D3", "DA", "C2", "DK", "C3", "D4", "D5",
            ],
            &[
                (Seat::West, "H5", "lead"),
                (Seat::East, "C2", "signal:discard"),
                (Seat::West, "D5", "signal:count"),
            ],
        ),
        (
            // A defender who ruffs is not discarding: East ruffs T1 with a trump.
            "a defender's ruff is not a discard",
            Strain::Spades,
            &["H5", "H2", "HQ", "HA", "D2", "D3", "DA", "S2"],
            &[(Seat::West, "H5", "lead")],
        ),
    ];
    for (what, strain, cards, expected) in rows {
        let h = history(*strain, Seat::West, cards);
        let c = contract(4, *strain, Seat::South);
        let a = everyone(defenders_agreement(
            Polarity::Standard,
            Polarity::Standard,
            FirstDiscard::Attitude,
        ));
        let interp = interpret_play(&h, &c, &a);
        let got: Vec<(Seat, Card, &str)> = interp
            .events
            .iter()
            .map(|e| (e.seat, e.card, e.rule))
            .collect();
        let want: Vec<(Seat, Card, &str)> =
            expected.iter().map(|(s, c, r)| (*s, card(c), *r)).collect();
        assert_eq!(got, want, "{what}");
    }
}

fn discard_event(trump: Strain, led: Suit, c: &str) -> SignalEvent {
    SignalEvent {
        seat: Seat::East,
        card: card(c),
        kind: SignalKind::FirstDiscard,
        context: SignalContext::Discard { trump, led },
    }
}

/// (first-discard convention, polarity, trump, led, discard, Some((satisfying, violating)) of
/// the first branch, or None when the first branch is `ANY`)
type DiscardRow = (
    FirstDiscard,
    Polarity,
    Strain,
    Suit,
    &'static str,
    Option<(&'static str, &'static str)>,
);

/// Regression (Lavinthal): the discarded suit is the one the defender does not want, so it is
/// never the suit a preference discard points at. Before the fix, with spades trump and hearts
/// led, a high diamond "showed" a diamond honour (the discarded suit itself), and against notrump
/// the lowest and highest of three candidate suits were used, which could again be the discarded
/// one.
#[test]
fn first_discard_table() {
    use FirstDiscard as F;
    use Polarity as P;
    let rows: &[DiscardRow] = &[
        // Attitude in the discarded suit.
        (
            F::Attitude,
            P::Standard,
            Strain::Spades,
            Suit::Hearts,
            "D8",
            Some(("AKQ.32.K973.AKQJ", "AKQ.32.9743.AKQJ")),
        ),
        (
            F::Attitude,
            P::Standard,
            Strain::NoTrump,
            Suit::Hearts,
            "D2",
            Some(("AKQ.32.9743.AKQJ", "AKQ.32.K973.AKQJ")),
        ),
        (
            F::Attitude,
            P::UpsideDown,
            Strain::Spades,
            Suit::Hearts,
            "D8",
            Some(("AKQ.32.9743.AKQJ", "AKQ.32.K973.AKQJ")),
        ),
        (
            F::Attitude,
            P::UpsideDown,
            Strain::Spades,
            Suit::Hearts,
            "D2",
            Some(("AKQ.32.K973.AKQJ", "AKQ.32.9743.AKQJ")),
        ),
        // Lavinthal vs NT, hearts led, diamond discarded: clubs (low) vs spades (high).
        (
            F::Lavinthal,
            P::Standard,
            Strain::NoTrump,
            Suit::Hearts,
            "D8",
            Some(("K973.32.9743.T98", "9873.32.KQJ3.T98")),
        ),
        (
            F::Lavinthal,
            P::Standard,
            Strain::NoTrump,
            Suit::Hearts,
            "D2",
            Some(("9873.32.9743.AT9", "K973.32.KQJ3.T98")),
        ),
        (
            F::Lavinthal,
            P::UpsideDown,
            Strain::NoTrump,
            Suit::Hearts,
            "D2",
            Some(("K973.32.9743.T98", "9873.32.9743.AT9")),
        ),
        // Spades trump, spades (trump) led, diamond discarded: clubs vs hearts.
        (
            F::Lavinthal,
            P::Standard,
            Strain::Spades,
            Suit::Spades,
            "D8",
            Some(("987.K932.9743.T9", "987.9832.KQJ3.T9")),
        ),
        (
            F::Lavinthal,
            P::Standard,
            Strain::Spades,
            Suit::Spades,
            "D2",
            Some(("987.9832.9743.A9", "987.K932.KQJ3.T9")),
        ),
        // Spades trump, hearts led, diamond discarded: clubs vs hearts, and East is void in
        // hearts, so "prefers hearts" carries no honour inference.
        (
            F::Lavinthal,
            P::Standard,
            Strain::Spades,
            Suit::Hearts,
            "D8",
            None,
        ),
        (
            F::Lavinthal,
            P::Standard,
            Strain::Spades,
            Suit::Hearts,
            "D2",
            Some(("987.9832.9743.A9", "987.9832.KQJ3.T9")),
        ),
        // Odd-even: odd shows an honour in the suit, even is Lavinthal.
        (
            F::OddEven,
            P::Standard,
            Strain::NoTrump,
            Suit::Hearts,
            "D7",
            Some(("AKQ.32.K973.AKQJ", "AKQ.32.9743.AKQJ")),
        ),
        (
            F::OddEven,
            P::Standard,
            Strain::NoTrump,
            Suit::Hearts,
            "D8",
            Some(("K973.32.9743.T98", "9873.32.KQJ3.T98")),
        ),
        (
            F::OddEven,
            P::Standard,
            Strain::NoTrump,
            Suit::Hearts,
            "D2",
            Some(("9873.32.9743.AT9", "K973.32.KQJ3.T98")),
        ),
    ];
    let signals = SignalTable::default();
    for (first, polarity, trump, led, c, expected) in rows {
        let d = DiscardTable {
            first: *first,
            polarity: *polarity,
        };
        let alts = signal_constraints(discard_event(*trump, *led, c), &signals, &d);
        let what = format!("{first:?}/{polarity:?} {c} ({trump:?}, {led:?} led)");
        assert_eq!(alts.len(), 2, "{what}");
        assert!((total(&alts) - 1.0).abs() < 1e-6, "{what}");
        assert!((alts[0].1 - 0.6).abs() < 1e-6, "{what}");
        match expected {
            None => assert!(is_any(&alts[0].0), "{what}: {:?}", alts[0].0),
            Some((good, bad)) => {
                assert!(
                    alts[0].0.satisfies(hand(good)),
                    "{what}: {good} should satisfy"
                );
                assert!(!alts[0].0.satisfies(hand(bad)), "{what}: {bad} should not");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Defenders only; combination
// ---------------------------------------------------------------------------------------------

/// Whoever declares, declarer and dummy never fire a rule, even carrying full agreements.
#[test]
fn soft_constraints_only_for_defenders() {
    for declarer in Seat::ALL {
        let c = contract(4, Strain::Spades, declarer);
        let leader = c.leader();
        // Rotate SIGNAL_PLAY so that `leader` plays West's part.
        let h = history(Strain::Spades, leader, SIGNAL_PLAY);
        let a = everyone(defenders_agreement(
            Polarity::Standard,
            Polarity::Standard,
            FirstDiscard::Attitude,
        ));
        let interp = interpret_play(&h, &c, &a);
        assert_eq!(interp.events.len(), 5, "declarer {declarer:?}");
        for e in &interp.events {
            assert!(
                !declarer.side().contains(e.seat),
                "declarer {declarer:?}: {e:?}"
            );
        }
        for seat in [declarer, c.dummy()] {
            let s = &interp.soft[idx(seat)];
            assert_eq!(s.len(), 1);
            assert!(is_any(&s[0].0));
        }
    }
}

/// A soft branch that contradicts the hard constraint is pruned (§7.6 step 2): East plays D3 then
/// D9 (low-high = odd) and then shows out, so East had exactly 2 diamonds; the odd branch dies and
/// East's soft constraint is back to `ANY`.
#[test]
fn soft_branches_contradicting_hard_are_pruned() {
    let h = history(
        Strain::Spades,
        Seat::West,
        &[
            "H5", "H2", "H6", "HA", "D4", "D2", "DK", "D3", "DA", "D9", "D5", "D6", "DQ", "C9",
            "D7", "D8",
        ],
    );
    let c = contract(4, Strain::Spades, Seat::South);
    let a = everyone(defenders_agreement(
        Polarity::Unknown,
        Polarity::Standard,
        FirstDiscard::Unknown,
    ));
    let interp = interpret_play(&h, &c, &a);
    let e = idx(Seat::East);
    assert!(
        interp
            .events
            .iter()
            .any(|ev| ev.seat == Seat::East && ev.rule == "signal:count")
    );
    assert_eq!(interp.hard[e].suit_len(Suit::Diamonds), 2..=2);
    assert_eq!(interp.soft[e].len(), 1, "{:?}", interp.soft[e]);
    assert!(is_any(&interp.soft[e][0].0));
    assert!((interp.soft[e][0].1 - 1.0).abs() < 1e-6);
}

/// `into_seats` is `hard ∧ soft_i` with the soft weights, for every seat.
#[test]
fn into_seats_ands_hard_into_every_branch() {
    let h = history(Strain::Spades, Seat::West, SIGNAL_PLAY);
    let c = contract(4, Strain::Spades, Seat::South);
    let a = everyone(defenders_agreement(
        Polarity::Standard,
        Polarity::Standard,
        FirstDiscard::Attitude,
    ));
    let interp = interpret_play(&h, &c, &a);
    let soft = interp.soft.clone();
    let seats = interp.into_seats();
    for seat in Seat::ALL {
        let i = idx(seat);
        assert_eq!(seats[i].len(), soft[i].len());
        for (k, (combined, w)) in seats[i].iter().enumerate() {
            assert_eq!(*w, soft[i][k].1);
            // East: every branch now fixes diamonds at exactly 2.
            if seat == Seat::East {
                assert_eq!(combined.suit_len(Suit::Diamonds), 2..=2);
            }
        }
    }
}
