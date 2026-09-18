//! Suits, ranks and cards.

/// One of the four suits in bridge ranking order: clubs < diamonds < hearts < spades.
///
/// The discriminant is the suit's index into every suit-ordered table and the position of the
/// suit's 13-bit block inside a [`Hand`](crate::Hand): clubs occupy bits `0..13`, spades
/// `39..52`.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Suit {
    /// ♣
    Clubs = 0,
    /// ♦
    Diamonds = 1,
    /// ♥
    Hearts = 2,
    /// ♠
    Spades = 3,
}

impl Suit {
    /// All suits in ascending (bit) order.
    pub const ALL: [Suit; 4] = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];

    /// The suit with index `i` (`0..4`).
    ///
    /// # Panics
    /// Panics if `i >= 4`. This is a contract violation, not an input error.
    pub const fn from_index(i: u8) -> Suit {
        match i {
            0 => Suit::Clubs,
            1 => Suit::Diamonds,
            2 => Suit::Hearts,
            3 => Suit::Spades,
            _ => panic!("suit index out of range"),
        }
    }

    /// Index `0..4`.
    pub const fn index(self) -> u8 {
        self as u8
    }

    /// Bit offset of this suit's block inside a [`Hand`](crate::Hand): `13 * index`.
    pub const fn shift(self) -> u8 {
        13 * (self as u8)
    }

    /// 52-bit mask covering this suit's block inside a [`Hand`](crate::Hand).
    pub const fn mask(self) -> u64 {
        0x1FFF_u64 << self.shift()
    }

    /// Unicode symbol (`♣ ♦ ♥ ♠`).
    pub const fn symbol(self) -> char {
        match self {
            Suit::Clubs => '♣',
            Suit::Diamonds => '♦',
            Suit::Hearts => '♥',
            Suit::Spades => '♠',
        }
    }

    /// PBN letter (`C D H S`).
    pub const fn letter(self) -> char {
        match self {
            Suit::Clubs => 'C',
            Suit::Diamonds => 'D',
            Suit::Hearts => 'H',
            Suit::Spades => 'S',
        }
    }
}

/// Card rank, ascending: Two = 0 … Ace = 12.
///
/// Ascending order makes the highest rank of a [`Holding`](crate::Holding) equal to
/// `15 - leading_zeros()`.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum Rank {
    Two = 0,
    Three = 1,
    Four = 2,
    Five = 3,
    Six = 4,
    Seven = 5,
    Eight = 6,
    Nine = 7,
    Ten = 8,
    Jack = 9,
    Queen = 10,
    King = 11,
    Ace = 12,
}

impl Rank {
    /// All ranks in ascending order.
    pub const ALL: [Rank; 13] = [
        Rank::Two,
        Rank::Three,
        Rank::Four,
        Rank::Five,
        Rank::Six,
        Rank::Seven,
        Rank::Eight,
        Rank::Nine,
        Rank::Ten,
        Rank::Jack,
        Rank::Queen,
        Rank::King,
        Rank::Ace,
    ];

    /// The rank with index `i` (`0..13`).
    ///
    /// # Panics
    /// Panics if `i >= 13`.
    pub const fn from_index(i: u8) -> Rank {
        match i {
            0 => Rank::Two,
            1 => Rank::Three,
            2 => Rank::Four,
            3 => Rank::Five,
            4 => Rank::Six,
            5 => Rank::Seven,
            6 => Rank::Eight,
            7 => Rank::Nine,
            8 => Rank::Ten,
            9 => Rank::Jack,
            10 => Rank::Queen,
            11 => Rank::King,
            12 => Rank::Ace,
            _ => panic!("rank index out of range"),
        }
    }

    /// Index `0..13`.
    pub const fn index(self) -> u8 {
        self as u8
    }

    /// High-card points of this rank alone (A = 4, K = 3, Q = 2, J = 1, else 0).
    pub const fn hcp(self) -> u8 {
        match self {
            Rank::Ace => 4,
            Rank::King => 3,
            Rank::Queen => 2,
            Rank::Jack => 1,
            _ => 0,
        }
    }

    /// PBN character (`2`..`9`, `T`, `J`, `Q`, `K`, `A`).
    pub const fn to_char(self) -> char {
        match self {
            Rank::Two => '2',
            Rank::Three => '3',
            Rank::Four => '4',
            Rank::Five => '5',
            Rank::Six => '6',
            Rank::Seven => '7',
            Rank::Eight => '8',
            Rank::Nine => '9',
            Rank::Ten => 'T',
            Rank::Jack => 'J',
            Rank::Queen => 'Q',
            Rank::King => 'K',
            Rank::Ace => 'A',
        }
    }
}

/// A single card, stored as its index `0..52` where `index = suit * 13 + rank`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Card(u8);

impl Card {
    /// The card of `suit` and `rank`.
    pub const fn new(suit: Suit, rank: Rank) -> Card {
        Card(suit as u8 * 13 + rank as u8)
    }

    /// The card with index `i`, or `None` if `i >= 52`.
    pub const fn from_index(i: u8) -> Option<Card> {
        if i < 52 { Some(Card(i)) } else { None }
    }

    /// Index `0..52`.
    pub const fn index(self) -> u8 {
        self.0
    }

    /// Suit of this card.
    pub const fn suit(self) -> Suit {
        Suit::from_index(self.0 / 13)
    }

    /// Rank of this card.
    pub const fn rank(self) -> Rank {
        Rank::from_index(self.0 % 13)
    }

    /// The single-bit 52-bit mask of this card inside a [`Hand`](crate::Hand).
    pub const fn bit(self) -> u64 {
        1u64 << self.0
    }
}

impl core::fmt::Debug for Card {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}{}", self.suit().letter(), self.rank().to_char())
    }
}
