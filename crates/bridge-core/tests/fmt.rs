//! `Display` / `FromStr` round trips and the specific strings of the format table.

use bridge_core::{
    Auction, Bid, Call, Card, Contract, Deal, Doubling, Hand, Holding, ParseError, Rank, Seat,
    Shape, ShapeClass, Strain, Suit, Vulnerability,
};
use proptest::prelude::*;

mod common;
use common::{arb_deal, arb_hand};

fn card(s: &str) -> Card {
    s.parse().unwrap()
}

#[test]
fn suit_and_rank() {
    for suit in Suit::ALL {
        assert_eq!(suit.to_string().parse::<Suit>().unwrap(), suit);
        assert_eq!(suit.symbol().to_string().parse::<Suit>().unwrap(), suit);
        assert_eq!(
            suit.letter()
                .to_ascii_lowercase()
                .to_string()
                .parse::<Suit>()
                .unwrap(),
            suit
        );
    }
    for rank in Rank::ALL {
        assert_eq!(rank.to_string().parse::<Rank>().unwrap(), rank);
    }
    assert_eq!("10".parse::<Rank>().unwrap(), Rank::Ten);
    assert_eq!("t".parse::<Rank>().unwrap(), Rank::Ten);
    assert_eq!("".parse::<Rank>(), Err(ParseError::Empty));
    assert_eq!("1".parse::<Rank>(), Err(ParseError::Rank('1')));
    assert_eq!("X".parse::<Suit>(), Err(ParseError::Suit('X')));
    assert_eq!("CD".parse::<Suit>(), Err(ParseError::Suit('D')));
}

#[test]
fn cards() {
    for i in 0..52 {
        let c = Card::from_index(i).unwrap();
        assert_eq!(c.to_string().parse::<Card>().unwrap(), c);
        assert_eq!(format!("{c:?}"), c.to_string());
    }
    let sa = Card::new(Suit::Spades, Rank::Ace);
    assert_eq!(sa.to_string(), "SA");
    assert_eq!(card("SA"), sa);
    assert_eq!(card("AS"), sa);
    assert_eq!(card("sa"), sa);
    assert_eq!(card(" as "), sa);
    assert_eq!(card("♠A"), sa);
    let ht = Card::new(Suit::Hearts, Rank::Ten);
    assert_eq!(card("HT"), ht);
    assert_eq!(card("H10"), ht);
    assert_eq!(card("10H"), ht);
    assert_eq!(card("TH"), ht);
    assert_eq!("".parse::<Card>(), Err(ParseError::Empty));
    assert_eq!("AX".parse::<Card>(), Err(ParseError::Suit('A')));
    assert_eq!("SZ".parse::<Card>(), Err(ParseError::Rank('Z')));
}

#[test]
fn holdings() {
    for bits in 0u16..8192 {
        let h = Holding::from_bits(bits).unwrap();
        assert_eq!(h.to_string().parse::<Holding>().unwrap(), h);
    }
    let akq = Holding::EMPTY
        .with(Rank::Ace)
        .with(Rank::King)
        .with(Rank::Queen);
    assert_eq!(akq.to_string(), "AKQ");
    assert_eq!("AKQ".parse::<Holding>().unwrap(), akq);
    assert_eq!("QKA".parse::<Holding>().unwrap(), akq);
    assert_eq!("akq".parse::<Holding>().unwrap(), akq);
    assert_eq!("-".parse::<Holding>().unwrap(), Holding::EMPTY);
    assert_eq!("".parse::<Holding>().unwrap(), Holding::EMPTY);
    assert_eq!(Holding::EMPTY.to_string(), "");
    assert_eq!(
        "T10".parse::<Holding>().unwrap(),
        Holding::EMPTY.with(Rank::Ten)
    );
    assert_eq!("AKQJT98765432".parse::<Holding>().unwrap(), Holding::FULL);
    assert_eq!("A1".parse::<Holding>(), Err(ParseError::Rank('1')));
}

#[test]
fn hands_specific_strings() {
    let text = "AKQ.234.AKQ.2345";
    let hand: Hand = text.parse().unwrap();
    assert_eq!(hand.len(), 13);
    // Output is canonical: ranks descending.
    assert_eq!(hand.to_string(), "AKQ.432.AKQ.5432");
    assert_eq!("AKQ.432.AKQ.5432".parse::<Hand>().unwrap(), hand);
    assert_eq!(hand.holding(Suit::Spades), "AKQ".parse().unwrap());
    assert_eq!(hand.holding(Suit::Hearts), "432".parse().unwrap());
    assert_eq!(hand.holding(Suit::Diamonds), "AKQ".parse().unwrap());
    assert_eq!(hand.holding(Suit::Clubs), "5432".parse().unwrap());
    assert_eq!(hand.shape(), Shape::new(4, 3, 3, 3));
    assert_eq!(hand.shape().to_string(), "3=3=3=4");

    // Ranks in any order and `10` for ten.
    assert_eq!("QKA.432.QKA.5432".parse::<Hand>().unwrap(), hand);
    let ten: Hand = "10.-.-.-".parse().unwrap();
    assert_eq!(ten, Hand::EMPTY.with(Card::new(Suit::Spades, Rank::Ten)));
    assert_eq!(ten.to_string(), "T...");

    // Voids: `-` on input, empty field on output.
    let voids: Hand = "AKQJT98765432.-.-.-".parse().unwrap();
    assert_eq!(voids.to_string(), "AKQJT98765432...");
    assert_eq!("AKQJT98765432...".parse::<Hand>().unwrap(), voids);
    assert_eq!("...".parse::<Hand>().unwrap(), Hand::EMPTY);
    assert_eq!(Hand::EMPTY.to_string(), "...");

    // Partial hands are accepted.
    let partial: Hand = "A.K..".parse().unwrap();
    assert_eq!(partial.len(), 2);

    // Duplicates are rejected with the offending card.
    assert_eq!(
        "AA.234.AKQ.2345".parse::<Hand>(),
        Err(ParseError::DuplicateCard(card("SA")))
    );
    assert_eq!(
        "AKQ.234.AKQ.23455".parse::<Hand>(),
        Err(ParseError::DuplicateCard(card("C5")))
    );
    assert_eq!("AKQ.234.AKQ".parse::<Hand>(), Err(ParseError::SuitCount(3)));
    assert_eq!(
        "AKQ.234.AKQ.2.3".parse::<Hand>(),
        Err(ParseError::SuitCount(5))
    );
    assert_eq!("".parse::<Hand>(), Err(ParseError::Empty));
    assert_eq!(
        "AKQ.234.AKQ.234x".parse::<Hand>(),
        Err(ParseError::Rank('x'))
    );
}

#[test]
fn deals_specific_strings() {
    let text = "N:AKQ.234.AKQ.2345 J2.AKQJ.J.AKQJT9 T98.T98.T9876.87 76543.765.5432.6";
    let deal: Deal = text.parse().unwrap();
    let canonical = "N:AKQ.432.AKQ.5432 J2.AKQJ.J.AKQJT9 T98.T98.T9876.87 76543.765.5432.6";
    assert_eq!(deal.to_string(), canonical);
    assert_eq!(canonical.parse::<Deal>().unwrap(), deal);
    assert_eq!(deal.hand(Seat::North), "AKQ.234.AKQ.2345".parse().unwrap());
    assert_eq!(deal.hand(Seat::West), "76543.765.5432.6".parse().unwrap());
    assert_eq!(deal.owner(card("SA")), Seat::North);
    assert_eq!(deal.owner(card("C6")), Seat::West);

    // Any starting seat, clockwise.
    let rotated = "S:T98.T98.T9876.87 76543.765.5432.6 AKQ.234.AKQ.2345 J2.AKQJ.J.AKQJT9";
    assert_eq!(rotated.parse::<Deal>().unwrap(), deal);
    let lower = "east:J2.AKQJ.J.AKQJT9 T98.T98.T9876.87 76543.765.5432.6 AKQ.234.AKQ.2345";
    assert_eq!(lower.parse::<Deal>().unwrap(), deal);

    assert_eq!(
        "N:AKQ.234.AKQ.2345 J2.AKQJ.J.AKQJT9 T98.T98.T9876.87".parse::<Deal>(),
        Err(ParseError::HandCount(3))
    );
    assert_eq!(
        "N:AKQ.234.AKQ.234 J2.AKQJ.J.AKQJT9 T98.T98.T9876.87 76543.765.5432.6".parse::<Deal>(),
        Err(ParseError::HandSize {
            seat: Seat::North,
            count: 12
        })
    );
    assert_eq!(
        "N:AKQ.234.AKQ.2345 A.AKQJ.J.AKQJT9 T98.T98.T9876.87 76543.765.5432.6".parse::<Deal>(),
        Err(ParseError::DuplicateCard(card("SA")))
    );
    assert_eq!(
        "X:AKQ.234.AKQ.2345 J2.AKQJ.J.AKQJT9 T98.T98.T9876.87 76543.765.5432.6".parse::<Deal>(),
        Err(ParseError::Seat('X'))
    );
    assert_eq!("".parse::<Deal>(), Err(ParseError::Empty));
    assert_eq!(
        "AKQ.234.AKQ.2345".parse::<Deal>(),
        Err(ParseError::Seat('A'))
    );
}

#[test]
fn shapes_and_classes() {
    let s: Shape = "5=4=3=1".parse().unwrap();
    assert_eq!(s, Shape::new(1, 3, 4, 5));
    assert_eq!(s.to_string(), "5=4=3=1");
    assert_eq!(s.len(Suit::Spades), 5);
    assert_eq!(s.len(Suit::Clubs), 1);
    assert_eq!(
        "5=4=3".parse::<Shape>(),
        Err(ParseError::Shape("5=4=3".into()))
    );
    assert_eq!(
        "14=0=0=0".parse::<Shape>(),
        Err(ParseError::Shape("14=0=0=0".into()))
    );
    for i in 0..560 {
        let s = Shape::from_index(i);
        assert_eq!(s.to_string().parse::<Shape>().unwrap(), s);
    }

    let c: ShapeClass = "5-4-3-1".parse().unwrap();
    assert_eq!(c, ShapeClass::C5431);
    assert_eq!(c.to_string(), "5-4-3-1");
    assert_eq!("1-3-4-5".parse::<ShapeClass>().unwrap(), ShapeClass::C5431);
    assert_eq!(
        "5=4=3=1".parse::<ShapeClass>(),
        Err(ParseError::Shape("5=4=3=1".into()))
    );
    for i in 0..39 {
        let c = ShapeClass::from_index(i);
        assert_eq!(c.to_string().parse::<ShapeClass>().unwrap(), c);
    }
}

#[test]
fn seats_strains_bids() {
    for seat in Seat::ALL {
        assert_eq!(seat.to_string().parse::<Seat>().unwrap(), seat);
    }
    assert_eq!("n".parse::<Seat>().unwrap(), Seat::North);
    assert_eq!("North".parse::<Seat>().unwrap(), Seat::North);
    assert_eq!(" west ".parse::<Seat>().unwrap(), Seat::West);
    assert_eq!("X".parse::<Seat>(), Err(ParseError::Seat('X')));
    assert_eq!("".parse::<Seat>(), Err(ParseError::Empty));

    for strain in Strain::ALL {
        assert_eq!(strain.to_string().parse::<Strain>().unwrap(), strain);
    }
    assert_eq!("N".parse::<Strain>().unwrap(), Strain::NoTrump);
    assert_eq!("nt".parse::<Strain>().unwrap(), Strain::NoTrump);
    assert_eq!("s".parse::<Strain>().unwrap(), Strain::Spades);

    for i in 0..35 {
        let b = Bid::from_index(i).unwrap();
        assert_eq!(b.to_string().parse::<Bid>().unwrap(), b);
        assert_eq!(format!("{b:?}"), b.to_string());
    }
    assert_eq!(
        "1NT".parse::<Bid>().unwrap(),
        Bid::new(1, Strain::NoTrump).unwrap()
    );
    assert_eq!(
        "1N".parse::<Bid>().unwrap(),
        Bid::new(1, Strain::NoTrump).unwrap()
    );
    assert_eq!(
        "7n".parse::<Bid>().unwrap(),
        Bid::new(7, Strain::NoTrump).unwrap()
    );
    assert_eq!(
        "4s".parse::<Bid>().unwrap(),
        Bid::new(4, Strain::Spades).unwrap()
    );
    assert_eq!(Bid::new(7, Strain::NoTrump).unwrap().to_string(), "7NT");
    assert_eq!("8C".parse::<Bid>(), Err(ParseError::Bid("8C".into())));
    assert_eq!("0C".parse::<Bid>(), Err(ParseError::Bid("0C".into())));
    assert_eq!("1X".parse::<Bid>(), Err(ParseError::Bid("1X".into())));
    assert_eq!("C".parse::<Bid>(), Err(ParseError::Bid("C".into())));
    assert_eq!("".parse::<Bid>(), Err(ParseError::Empty));
}

#[test]
fn calls_and_contracts() {
    let one_club = Call::Bid(Bid::new(1, Strain::Clubs).unwrap());
    assert_eq!(Call::Pass.to_string(), "Pass");
    assert_eq!(Call::Double.to_string(), "X");
    assert_eq!(Call::Redouble.to_string(), "XX");
    assert_eq!(one_club.to_string(), "1C");
    for (text, call) in [
        ("P", Call::Pass),
        ("Pass", Call::Pass),
        ("pass", Call::Pass),
        ("X", Call::Double),
        ("D", Call::Double),
        ("Dbl", Call::Double),
        ("double", Call::Double),
        ("XX", Call::Redouble),
        ("R", Call::Redouble),
        ("Rdbl", Call::Redouble),
        ("xx", Call::Redouble),
        ("1C", one_club),
        ("1c", one_club),
        (" 1C ", one_club),
        ("1NT", Call::Bid(Bid::new(1, Strain::NoTrump).unwrap())),
        ("1N", Call::Bid(Bid::new(1, Strain::NoTrump).unwrap())),
    ] {
        assert_eq!(text.parse::<Call>().unwrap(), call, "{text}");
    }
    for i in 0..38 {
        let c = Call::from_index(i).unwrap();
        assert_eq!(c.to_string().parse::<Call>().unwrap(), c);
    }
    assert_eq!("XXX".parse::<Call>(), Err(ParseError::Call("XXX".into())));
    assert_eq!("".parse::<Call>(), Err(ParseError::Empty));

    let four_spades = Bid::new(4, Strain::Spades).unwrap();
    let c: Contract = "4SX".parse().unwrap();
    assert_eq!(c.bid, four_spades);
    assert_eq!(c.doubling, Doubling::Doubled);
    assert_eq!(c.to_string(), "4SX");
    let c: Contract = "4SXX".parse().unwrap();
    assert_eq!(c.doubling, Doubling::Redoubled);
    assert_eq!(c.to_string(), "4SXX");
    let c: Contract = "3NT".parse().unwrap();
    assert_eq!(c.bid, Bid::new(3, Strain::NoTrump).unwrap());
    assert_eq!(c.doubling, Doubling::Undoubled);
    assert_eq!(c.to_string(), "3NT");
    assert_eq!("4hx".parse::<Contract>().unwrap().to_string(), "4HX");
    assert_eq!(
        "4SXXX".parse::<Contract>(),
        Err(ParseError::Call("XXX".into()))
    );
    assert_eq!("9SX".parse::<Contract>(), Err(ParseError::Bid("9S".into())));
    assert_eq!("".parse::<Contract>(), Err(ParseError::Empty));
    let full = Contract {
        bid: four_spades,
        declarer: Seat::South,
        doubling: Doubling::Doubled,
    };
    assert_eq!(full.to_string(), "4SX");
    assert_eq!(full.leader(), Seat::West);
    assert_eq!(full.dummy(), Seat::North);
}

#[test]
fn vulnerability() {
    assert_eq!(Vulnerability::None.to_string(), "None");
    assert_eq!(Vulnerability::NS.to_string(), "NS");
    assert_eq!(Vulnerability::EW.to_string(), "EW");
    assert_eq!(Vulnerability::Both.to_string(), "All");
    for (text, v) in [
        ("None", Vulnerability::None),
        ("NONE", Vulnerability::None),
        ("Love", Vulnerability::None),
        ("-", Vulnerability::None),
        ("NS", Vulnerability::NS),
        ("ns", Vulnerability::NS),
        ("EW", Vulnerability::EW),
        ("All", Vulnerability::Both),
        ("Both", Vulnerability::Both),
        ("both", Vulnerability::Both),
    ] {
        assert_eq!(text.parse::<Vulnerability>().unwrap(), v, "{text}");
    }
    for i in 0..4 {
        let v = Vulnerability::from_index(i);
        assert_eq!(v.to_string().parse::<Vulnerability>().unwrap(), v);
    }
    assert_eq!(
        "NW".parse::<Vulnerability>(),
        Err(ParseError::Vulnerability("NW".into()))
    );
    assert_eq!("".parse::<Vulnerability>(), Err(ParseError::Empty));
}

#[test]
fn auction_display() {
    let calls: Vec<Call> = ["1C", "P", "1H", "P"]
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
    let auction = Auction::from_calls(Seat::North, Vulnerability::None, calls).unwrap();
    assert_eq!(auction.to_string(), "1C Pass 1H Pass");
    assert_eq!(Auction::new(Seat::East, Vulnerability::NS).to_string(), "");
}

#[test]
fn error_messages_mention_the_card() {
    let err = ParseError::DuplicateCard(card("SA"));
    assert_eq!(err.to_string(), "duplicate card SA");
    let err = ParseError::HandSize {
        seat: Seat::East,
        count: 12,
    };
    assert_eq!(err.to_string(), "E holds 12 cards, expected 13");
}

proptest! {
    #[test]
    fn hand_round_trip(hand in arb_hand()) {
        let text = hand.to_string();
        prop_assert_eq!(text.parse::<Hand>().unwrap(), hand);
        prop_assert_eq!(text.split('.').count(), 4);
    }

    #[test]
    fn deal_round_trip(deal in arb_deal()) {
        let text = deal.to_string();
        prop_assert!(text.starts_with("N:"));
        prop_assert_eq!(text.parse::<Deal>().unwrap(), deal);
    }

    #[test]
    fn holding_display_is_descending(bits in 0u16..8192) {
        let h = Holding::from_bits(bits).unwrap();
        let ranks: Vec<Rank> = h.ranks().collect();
        prop_assert_eq!(ranks.len(), h.len() as usize);
        prop_assert!(ranks.windows(2).all(|w| w[0] > w[1]));
        let text = h.to_string();
        prop_assert_eq!(text.chars().count(), h.len() as usize);
    }
}
