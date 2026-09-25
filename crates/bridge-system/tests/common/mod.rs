//! Shared helpers for `bridge-system` integration tests: building auctions and hands from short
//! text specs, so test bodies read as bidding sequences and PBN-style holdings.

#![allow(dead_code)]

use bridge_core::{Auction, Call, Hand, Holding, Rank, Seat, Vulnerability};

/// Builds an auction from a dealer, a vulnerability and a space-separated list of calls
/// (`"1S P 2S P"`, `"P P 1NT P"`, `"X"`, `"XX"`, …), as accepted by `Call`'s `FromStr`.
pub fn auction(dealer: Seat, vul: Vulnerability, calls: &str) -> Auction {
    let calls: Vec<Call> = calls
        .split_whitespace()
        .map(|c| c.parse().unwrap_or_else(|_| panic!("bad call {c:?}")))
        .collect();
    Auction::from_calls(dealer, vul, calls).expect("legal auction")
}

/// Builds a 13-card hand from four suit holdings (clubs, diamonds, hearts, spades), each a
/// string of rank characters (`"AKQJT98765432"`, case-insensitive, any subset, any order).
pub fn hand(clubs: &str, diamonds: &str, hearts: &str, spades: &str) -> Hand {
    Hand::from_holdings(
        holding(clubs),
        holding(diamonds),
        holding(hearts),
        holding(spades),
    )
}

/// Parses one suit's ranks (see [`hand`]).
pub fn holding(ranks: &str) -> Holding {
    ranks.chars().fold(Holding::EMPTY, |h, c| h.with(rank(c)))
}

fn rank(c: char) -> Rank {
    match c.to_ascii_uppercase() {
        'A' => Rank::Ace,
        'K' => Rank::King,
        'Q' => Rank::Queen,
        'J' => Rank::Jack,
        'T' => Rank::Ten,
        '9' => Rank::Nine,
        '8' => Rank::Eight,
        '7' => Rank::Seven,
        '6' => Rank::Six,
        '5' => Rank::Five,
        '4' => Rank::Four,
        '3' => Rank::Three,
        '2' => Rank::Two,
        other => panic!("not a rank: {other:?}"),
    }
}
