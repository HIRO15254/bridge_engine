//! Encoding conversion between `bridge-core` and DDS.
//!
//! | Concept | core | DDS | conversion |
//! | --- | --- | --- | --- |
//! | Seat | N=0 E=1 S=2 W=3 | same | identity |
//! | Suit | C=0 D=1 H=2 S=3 | S=0 H=1 D=2 C=3 | `3 − core` |
//! | Strain | C=0..S=3, NT=4 | S=0..C=3, NT=4 | `NT → 4`, else `3 − core` |
//! | Holding | bit `rank` (Two = bit 0) | bits `2..=14` (deuce = bit 2) | `bits << 2` |
//! | Rank | Two=0..Ace=12 | 2..14 | `+2` |
//! | Vulnerability (par) | enum | 0 none, 1 all, 2 NS, 3 EW | table |

use bridge_core::{Card, Deal, Holding, Rank, Seat, Strain, Suit, Vulnerability};

#[cfg(dds_vendored)]
use crate::sys;

/// DDS suit index of a core suit.
pub const fn suit(s: Suit) -> i32 {
    3 - s.index() as i32
}

/// Core suit of a DDS suit index (`0..4`; not `4`, which is no-trump and has no core `Suit`).
///
/// # Panics
/// Panics if `dds_suit` is not in `0..4`.
pub const fn suit_from_dds(dds_suit: i32) -> Suit {
    Suit::from_index((3 - dds_suit) as u8)
}

/// DDS strain index of a core strain.
pub const fn strain(s: Strain) -> i32 {
    match s {
        Strain::NoTrump => 4,
        s => 3 - s.index() as i32,
    }
}

/// Core strain of a DDS strain index (`0..5`).
///
/// # Panics
/// Panics if `dds_strain` is not in `0..5`.
pub const fn strain_from_dds(dds_strain: i32) -> Strain {
    if dds_strain == 4 {
        Strain::NoTrump
    } else {
        Strain::from_index((3 - dds_strain) as u8)
    }
}

/// DDS hand index of a seat.
pub const fn seat(s: Seat) -> i32 {
    s.index() as i32
}

/// Core seat of a DDS hand index (`0..4`; the encoding is identity, but this documents intent
/// and panics on an out-of-range value instead of silently misinterpreting DDS output).
///
/// # Panics
/// Panics if `dds_hand` is not in `0..4`.
pub const fn seat_from_dds(dds_hand: i32) -> Seat {
    Seat::from_index(dds_hand as u8)
}

/// DDS rank of a core rank index.
pub const fn rank(core_rank: u8) -> i32 {
    core_rank as i32 + 2
}

/// DDS card mask of a holding.
pub const fn holding(h: Holding) -> u32 {
    (h.bits() as u32) << 2
}

/// Core holding of a DDS card mask (bits `2..=14`).
pub const fn holding_from_dds(bits: u32) -> Holding {
    // `Holding::from_bits` only fails above bit 12 (13 ranks); DDS masks never set those, so
    // the shifted-down value is always in range.
    match Holding::from_bits((bits >> 2) as u16) {
        Some(h) => h,
        None => panic!("DDS card mask has bits set above rank Ace"),
    }
}

/// The core card named by a DDS `(suit, rank)` pair, e.g. from `futureTricks`.
///
/// # Panics
/// Panics if `dds_suit` is not in `0..4` or `dds_rank` not in `2..=14`.
pub const fn card_from_dds(dds_suit: i32, dds_rank: i32) -> Card {
    Card::new(
        suit_from_dds(dds_suit),
        Rank::from_index((dds_rank - 2) as u8),
    )
}

/// `remainCards[hand][dds_suit]` for a whole deal (used by `calc_dd_table(s)`; `solve_board`
/// additionally removes the cards of the current trick from their owner, see `lib.rs`).
pub fn remain_cards(deal: &Deal) -> [[u32; 4]; 4] {
    let mut out = [[0u32; 4]; 4];
    for s in Seat::ALL {
        let hand = deal.hand(s);
        for suit_id in Suit::ALL {
            out[seat(s) as usize][suit(suit_id) as usize] = holding(hand.holding(suit_id));
        }
    }
    out
}

/// DDS vulnerability code for `DealerParBin`.
pub const fn vulnerability(v: Vulnerability) -> i32 {
    match v {
        Vulnerability::None => 0,
        Vulnerability::Both => 1,
        Vulnerability::NS => 2,
        Vulnerability::EW => 3,
    }
}

/// Core vulnerability of a DDS vulnerability code (`0..4`; `DealerPar.cpp`: "vulnerable 0: None
/// 1: Both 2: NS 3: EW" — note this is *not* the same numbering as
/// [`Vulnerability::from_index`](bridge_core::Vulnerability::from_index)).
///
/// # Panics
/// Panics if `dds_vul` is not in `0..4`.
pub const fn vulnerability_from_dds(dds_vul: i32) -> Vulnerability {
    match dds_vul {
        0 => Vulnerability::None,
        1 => Vulnerability::Both,
        2 => Vulnerability::NS,
        3 => Vulnerability::EW,
        _ => panic!("DDS vulnerability code out of range"),
    }
}

/// `contractType.denom`: **not** the `Suit`/`Strain` encoding used everywhere else in DDS.
/// `dll.h`: `0 = No Trumps, 1 = Spades, 2 = Hearts, 3 = Diamonds, 4 = Clubs`.
#[cfg(dds_vendored)]
const fn par_denom_letter(denom: i32) -> char {
    match denom {
        0 => 'N',
        1 => 'S',
        2 => 'H',
        3 => 'D',
        4 => 'C',
        _ => '?',
    }
}

/// `contractType.seats`: `0 N, 1 E, 2 S, 3 W, 4 NS, 5 EW`.
#[cfg(dds_vendored)]
fn par_seats_label(seats: i32) -> &'static str {
    match seats {
        0 => "N",
        1 => "E",
        2 => "S",
        3 => "W",
        4 => "NS",
        5 => "EW",
        _ => "?",
    }
}

/// Formats one `contractType` as e.g. `"NS 4S"`, `"EW 4Sx-1"` (sacrifice, doubled, down one) or
/// `"NS 3N+1"` (making with one overtrick); the same algorithm as the vendored
/// `ConvertToDealerTextFormat` (`Par.cpp`), reimplemented because that function writes into a
/// fixed `char[]` buffer that this crate has no matching allocation for.
#[cfg(dds_vendored)]
pub fn format_par_contract(c: &sys::contractType) -> String {
    let mut s = format!(
        "{} {}{}",
        par_seats_label(c.seats),
        c.level,
        par_denom_letter(c.denom)
    );
    if c.underTricks > 0 {
        s.push_str(&format!("x-{}", c.underTricks));
    } else if c.overTricks > 0 {
        s.push_str(&format!("+{}", c.overTricks));
    }
    s
}
