//! serde round trips: JSON (human-readable, `Display` strings) and postcard (binary, raw
//! integers), plus the validation on deserialization.

#![cfg(feature = "serde")]

use bridge_core::{
    Auction, Bid, Call, Card, Contract, DdTable, Deal, Doubling, Hand, Holding, Rank, Seat, Shape,
    ShapeClass, ShapeSet, Strain, Suit, Vulnerability,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

fn json_round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(
    value: &T,
    expected_json: &str,
) {
    let json = serde_json::to_string(value).unwrap();
    assert_eq!(json, expected_json);
    let back: T = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, value);
}

fn postcard_round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(
    value: &T,
) -> Vec<u8> {
    let bytes = postcard::to_allocvec(value).unwrap();
    let back: T = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(&back, value);
    bytes
}

fn sample_deal() -> Deal {
    "N:AKQ.234.AKQ.2345 J2.AKQJ.J.AKQJT9 T98.T98.T9876.87 76543.765.5432.6"
        .parse()
        .unwrap()
}

#[test]
fn newtypes_json() {
    json_round_trip(&Card::new(Suit::Spades, Rank::Ace), "\"SA\"");
    json_round_trip(&"AKQ".parse::<Holding>().unwrap(), "\"AKQ\"");
    json_round_trip(&Holding::EMPTY, "\"\"");
    json_round_trip(
        &"AKQ.234.AKQ.2345".parse::<Hand>().unwrap(),
        "\"AKQ.432.AKQ.5432\"",
    );
    json_round_trip(
        &"AKQJT98765432...".parse::<Hand>().unwrap(),
        "\"AKQJT98765432...\"",
    );
    json_round_trip(&Shape::new(1, 3, 4, 5), "\"5=4=3=1\"");
    json_round_trip(&Bid::new(1, Strain::Clubs).unwrap(), "\"1C\"");
    json_round_trip(&Bid::new(7, Strain::NoTrump).unwrap(), "\"7NT\"");
    json_round_trip(
        &sample_deal(),
        "\"N:AKQ.432.AKQ.5432 J2.AKQJ.J.AKQJT9 T98.T98.T9876.87 76543.765.5432.6\"",
    );
}

#[test]
fn newtypes_postcard_are_raw_integers() {
    let sa = Card::new(Suit::Spades, Rank::Ace);
    assert_eq!(postcard_round_trip(&sa), vec![51]);
    let bytes = postcard_round_trip(&Holding::FULL);
    assert_eq!(bytes, postcard::to_allocvec(&0x1FFFu16).unwrap());
    let hand = "AKQ.234.AKQ.2345".parse::<Hand>().unwrap();
    let bytes = postcard_round_trip(&hand);
    assert_eq!(bytes, postcard::to_allocvec(&hand.bits()).unwrap());
    let shape = Shape::new(1, 3, 4, 5);
    let bytes = postcard_round_trip(&shape);
    assert_eq!(bytes, postcard::to_allocvec(&shape.bits()).unwrap());
    let bid = Bid::new(3, Strain::NoTrump).unwrap();
    assert_eq!(postcard_round_trip(&bid), vec![bid.index()]);
    let deal = sample_deal();
    let bytes = postcard_round_trip(&deal);
    assert_eq!(
        bytes,
        postcard::to_allocvec(&deal.hands().map(Hand::bits)).unwrap()
    );
}

#[test]
fn deserialization_validates() {
    assert!(postcard::from_bytes::<Card>(&[52]).is_err());
    assert!(postcard::from_bytes::<Bid>(&[35]).is_err());
    let bad_holding = postcard::to_allocvec(&0x2000u16).unwrap();
    assert!(postcard::from_bytes::<Holding>(&bad_holding).is_err());
    let bad_hand = postcard::to_allocvec(&(1u64 << 52)).unwrap();
    assert!(postcard::from_bytes::<Hand>(&bad_hand).is_err());
    let bad_shape = postcard::to_allocvec(&0x000Fu16).unwrap();
    assert!(postcard::from_bytes::<Shape>(&bad_shape).is_err());
    let bad_deal = postcard::to_allocvec(&[Hand::FULL.bits(), 0, 0, 0]).unwrap();
    assert!(postcard::from_bytes::<Deal>(&bad_deal).is_err());
    let mut words = [0u64; 9];
    words[8] = 1 << 48;
    let bad_set = postcard::to_allocvec(&words).unwrap();
    assert!(postcard::from_bytes::<ShapeSet>(&bad_set).is_err());

    assert!(serde_json::from_str::<Card>("\"SZ\"").is_err());
    assert!(serde_json::from_str::<Hand>("\"AA.234.AKQ.2345\"").is_err());
    assert!(serde_json::from_str::<Deal>("\"N:AKQ.234.AKQ.2345 J2.AKQJ.J.AKQJT9\"").is_err());
    assert!(serde_json::from_str::<Shape>("\"5=4=3\"").is_err());
    assert!(serde_json::from_str::<Bid>("\"8C\"").is_err());
}

#[test]
fn auction_json_and_postcard() {
    let calls: Vec<Call> = ["1C", "P", "1H", "X", "XX", "P", "P", "P"]
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
    let auction = Auction::from_calls(Seat::East, Vulnerability::NS, calls).unwrap();
    let json = serde_json::to_string(&auction).unwrap();
    assert_eq!(
        json,
        r#"{"dealer":"East","vulnerability":"NS","calls":[{"Bid":"1C"},"Pass",{"Bid":"1H"},"Double","Redouble","Pass","Pass","Pass"]}"#
    );
    let back: Auction = serde_json::from_str(&json).unwrap();
    assert_eq!(back, auction);
    postcard_round_trip(&auction);

    // An illegal auction cannot be deserialized.
    let illegal = r#"{"dealer":"North","vulnerability":"None","calls":["Double"]}"#;
    let err = serde_json::from_str::<Auction>(illegal).unwrap_err();
    assert!(err.to_string().contains("illegal"), "{err}");
    let illegal =
        r#"{"dealer":"North","vulnerability":"None","calls":[{"Bid":"1H"},{"Bid":"1C"}]}"#;
    assert!(serde_json::from_str::<Auction>(illegal).is_err());
    let illegal =
        r#"{"dealer":"North","vulnerability":"None","calls":["Pass","Pass","Pass","Pass","Pass"]}"#;
    assert!(serde_json::from_str::<Auction>(illegal).is_err());
}

#[test]
fn shape_set_json_forms() {
    // A product of ranges.
    let product = ShapeSet::from_suit_lens([(2, 5), (0, 13), (4, 4), (3, 13)]);
    let json = serde_json::to_string(&product).unwrap();
    assert_eq!(json, "\"C2-5 D0-4 H4-4 S3-7\"");
    assert_eq!(serde_json::from_str::<ShapeSet>(&json).unwrap(), product);
    // Loose ranges and missing suits are accepted on input.
    assert_eq!(
        serde_json::from_str::<ShapeSet>("\"C2-5 H4-4 S3-13\"").unwrap(),
        product
    );
    json_round_trip(&ShapeSet::ALL, "\"C0-13 D0-13 H0-13 S0-13\"");

    // A union of whole classes.
    json_round_trip(&ShapeSet::BALANCED, "[\"4-3-3-3\",\"4-4-3-2\",\"5-3-3-2\"]");
    json_round_trip(&ShapeSet::EMPTY, "[]");
    assert_eq!(
        serde_json::from_str::<ShapeSet>("[\"5-4-3-1\"]").unwrap(),
        ShapeSet::from_class(ShapeClass::C5431)
    );

    // Anything else: a 140-digit hex bitmap.
    let odd = ShapeSet::EMPTY
        .insert(Shape::new(1, 3, 4, 5))
        .insert(Shape::new(4, 4, 4, 1));
    let json = serde_json::to_string(&odd).unwrap();
    assert_eq!(json.len(), 142);
    assert!(json[1..141].bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(serde_json::from_str::<ShapeSet>(&json).unwrap(), odd);
    let ones = ShapeSet::EMPTY
        .insert(Shape::from_index(0))
        .insert(Shape::from_index(559));
    let json = serde_json::to_string(&ones).unwrap();
    assert_eq!(json.len(), 142);
    assert!(json.starts_with("\"8000"));
    assert!(json.ends_with("0001\""));
    assert_eq!(serde_json::from_str::<ShapeSet>(&json).unwrap(), ones);

    assert!(serde_json::from_str::<ShapeSet>("\"abc\"").is_err());
    assert!(serde_json::from_str::<ShapeSet>("\"X2-5\"").is_err());
    assert!(serde_json::from_str::<ShapeSet>("[\"4-3-3\"]").is_err());
    assert!(serde_json::from_str::<ShapeSet>("42").is_err());
}

#[test]
fn shape_set_postcard() {
    for set in [
        ShapeSet::EMPTY,
        ShapeSet::ALL,
        ShapeSet::BALANCED,
        ShapeSet::SEMI_BALANCED,
        ShapeSet::from_suit_len(Suit::Spades, 5, 13),
    ] {
        let bytes = postcard_round_trip(&set);
        assert_eq!(bytes, postcard::to_allocvec(&set.words()).unwrap());
    }
}

#[test]
fn derived_types() {
    json_round_trip(&Seat::North, "\"North\"");
    json_round_trip(&Suit::Hearts, "\"Hearts\"");
    json_round_trip(&Strain::NoTrump, "\"NoTrump\"");
    json_round_trip(&Vulnerability::Both, "\"Both\"");
    json_round_trip(
        &Call::Bid(Bid::new(2, Strain::Spades).unwrap()),
        "{\"Bid\":\"2S\"}",
    );
    json_round_trip(
        &Contract {
            bid: Bid::new(4, Strain::Spades).unwrap(),
            declarer: Seat::South,
            doubling: Doubling::Doubled,
        },
        "{\"bid\":\"4S\",\"declarer\":\"South\",\"doubling\":\"Doubled\"}",
    );
    postcard_round_trip(&ShapeClass::C5431);
    let table = DdTable::new([
        [7, 6, 7, 6],
        [8, 5, 8, 5],
        [10, 3, 10, 3],
        [10, 3, 10, 3],
        [9, 4, 9, 4],
    ]);
    postcard_round_trip(&table);
    let json = serde_json::to_string(&table).unwrap();
    assert_eq!(serde_json::from_str::<DdTable>(&json).unwrap(), table);
}

#[test]
fn struct_with_newtypes_derives() {
    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Record {
        hand: Hand,
        shapes: ShapeSet,
        contract: Contract,
    }
    let record = Record {
        hand: "AKQ.234.AKQ.2345".parse().unwrap(),
        shapes: ShapeSet::SEMI_BALANCED,
        contract: "3NT".parse().unwrap(),
    };
    let json = serde_json::to_string(&record).unwrap();
    assert_eq!(serde_json::from_str::<Record>(&json).unwrap(), record);
    postcard_round_trip(&record);
}
