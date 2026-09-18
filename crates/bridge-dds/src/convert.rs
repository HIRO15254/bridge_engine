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

use bridge_core::{Deal, Holding, Seat, Strain, Suit, Vulnerability};

/// DDS suit index of a core suit.
pub const fn suit(s: Suit) -> i32 {
    3 - s.index() as i32
}

/// DDS strain index of a core strain.
pub const fn strain(s: Strain) -> i32 {
    match s {
        Strain::NoTrump => 4,
        s => 3 - s.index() as i32,
    }
}

/// DDS hand index of a seat.
pub const fn seat(s: Seat) -> i32 {
    s.index() as i32
}

/// DDS rank of a core rank index.
pub const fn rank(core_rank: u8) -> i32 {
    core_rank as i32 + 2
}

/// DDS card mask of a holding.
pub const fn holding(h: Holding) -> u32 {
    (h.bits() as u32) << 2
}

/// `remainCards[hand][dds_suit]` for a deal.
pub fn remain_cards(deal: &Deal) -> [[u32; 4]; 4] {
    todo!("phase 5")
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
