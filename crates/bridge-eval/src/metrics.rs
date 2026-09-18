//! Evaluation functions.

use bridge_core::{Hand, Holding, Rank, Suit};

use crate::{Half, LtcMethod, SUIT};

/// The 52-bit mask of `rank` in all four suits.
pub const fn rank_mask(rank: Rank) -> u64 {
    let b = 1u64 << rank.index();
    b | b << 13 | b << 26 | b << 39
}

/// All four aces.
pub const ACES: u64 = rank_mask(Rank::Ace);
/// All four kings.
pub const KINGS: u64 = rank_mask(Rank::King);
/// All four queens.
pub const QUEENS: u64 = rank_mask(Rank::Queen);
/// All four jacks.
pub const JACKS: u64 = rank_mask(Rank::Jack);
/// All four tens.
pub const TENS: u64 = rank_mask(Rank::Ten);

/// High-card points: A = 4, K = 3, Q = 2, J = 1. Four `popcnt`s; well under 10 ns.
pub const fn hcp(hand: Hand) -> u8 {
    let b = hand.bits();
    (4 * (b & ACES).count_ones()
        + 3 * (b & KINGS).count_ones()
        + 2 * (b & QUEENS).count_ones()
        + (b & JACKS).count_ones()) as u8
}

/// High-card points of one suit.
pub const fn holding_hcp(holding: Holding) -> u8 {
    SUIT.hcp[holding.bits() as usize]
}

/// Controls: A = 2, K = 1.
pub const fn controls(hand: Hand) -> u8 {
    let b = hand.bits();
    (2 * (b & ACES).count_ones() + (b & KINGS).count_ones()) as u8
}

/// Number of aces.
pub const fn aces(hand: Hand) -> u8 {
    (hand.bits() & ACES).count_ones() as u8
}

/// Number of kings.
pub const fn kings(hand: Hand) -> u8 {
    (hand.bits() & KINGS).count_ones() as u8
}

/// Number of queens.
pub const fn queens(hand: Hand) -> u8 {
    (hand.bits() & QUEENS).count_ones() as u8
}

/// Number of jacks.
pub const fn jacks(hand: Hand) -> u8 {
    (hand.bits() & JACKS).count_ones() as u8
}

/// Number of tens.
pub const fn tens(hand: Hand) -> u8 {
    (hand.bits() & TENS).count_ones() as u8
}

/// Classic losing-trick count of the whole hand (sum over suits).
pub fn losers(hand: Hand) -> Half {
    losers_with(hand, LtcMethod::Classic)
}

/// Losing-trick count with the chosen method.
pub fn losers_with(hand: Hand, method: LtcMethod) -> Half {
    let table: &[u8; 8192] = match method {
        LtcMethod::Classic => &SUIT.losers2,
        LtcMethod::New => &SUIT.nltc2,
    };
    let mut halves = 0u8;
    for suit in Suit::ALL {
        halves += table[hand.holding(suit).bits() as usize];
    }
    Half::from_halves(halves)
}

/// Quick tricks of the whole hand (sum over suits).
pub fn quick_tricks(hand: Hand) -> Half {
    let mut halves = 0u8;
    for suit in Suit::ALL {
        halves += SUIT.qt2[hand.holding(suit).bits() as usize];
    }
    Half::from_halves(halves)
}

/// Number of honours (A K Q J T) in a suit.
pub const fn honors(holding: Holding) -> u8 {
    SUIT.honors5[holding.bits() as usize]
}

/// Number of the `n` highest ranks (A, K, Q, …) held in a suit.
pub const fn top_honors(holding: Holding, n: u8) -> u8 {
    holding.intersect(Holding::top_ranks(n)).len()
}

/// Suit quality for "3 of the top 5" style tests: the number of honours among A K Q J T.
pub const fn suit_quality(holding: Holding) -> u8 {
    honors(holding)
}
