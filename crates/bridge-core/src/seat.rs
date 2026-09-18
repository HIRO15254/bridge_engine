//! Seats, sides and vulnerability.

/// A seat at the table, clockwise from North.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum Seat {
    North = 0,
    East = 1,
    South = 2,
    West = 3,
}

impl Seat {
    /// All seats clockwise from North.
    pub const ALL: [Seat; 4] = [Seat::North, Seat::East, Seat::South, Seat::West];

    /// The seat with index `i` (`0..4`).
    ///
    /// # Panics
    /// Panics if `i >= 4`.
    pub const fn from_index(i: u8) -> Seat {
        match i {
            0 => Seat::North,
            1 => Seat::East,
            2 => Seat::South,
            3 => Seat::West,
            _ => panic!("seat index out of range"),
        }
    }

    /// Index `0..4`.
    pub const fn index(self) -> u8 {
        self as u8
    }

    /// The seat `n` places clockwise from this one.
    pub const fn offset(self, n: u8) -> Seat {
        Seat::from_index((self as u8 + n) % 4)
    }

    /// Left-hand opponent (next to act).
    pub const fn next(self) -> Seat {
        self.offset(1)
    }

    /// Right-hand opponent.
    pub const fn prev(self) -> Seat {
        self.offset(3)
    }

    /// Partner.
    pub const fn partner(self) -> Seat {
        self.offset(2)
    }

    /// Left-hand opponent (alias of [`Seat::next`]).
    pub const fn lho(self) -> Seat {
        self.next()
    }

    /// Right-hand opponent (alias of [`Seat::prev`]).
    pub const fn rho(self) -> Seat {
        self.prev()
    }

    /// The partnership this seat belongs to.
    pub const fn side(self) -> Side {
        if self as u8 % 2 == 0 {
            Side::NS
        } else {
            Side::EW
        }
    }

    /// The dealer of a standard board number (board 1 → North, 2 → East, …).
    pub const fn dealer_of_board(number: u16) -> Seat {
        Seat::from_index(((number + 3) % 4) as u8)
    }

    /// PBN letter (`N E S W`).
    pub const fn letter(self) -> char {
        match self {
            Seat::North => 'N',
            Seat::East => 'E',
            Seat::South => 'S',
            Seat::West => 'W',
        }
    }
}

/// A partnership.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Side {
    /// North–South.
    NS = 0,
    /// East–West.
    EW = 1,
}

impl Side {
    /// The two seats of this side.
    pub const fn seats(self) -> [Seat; 2] {
        match self {
            Side::NS => [Seat::North, Seat::South],
            Side::EW => [Seat::East, Seat::West],
        }
    }

    /// The opposing side.
    pub const fn other(self) -> Side {
        match self {
            Side::NS => Side::EW,
            Side::EW => Side::NS,
        }
    }

    /// Whether `seat` belongs to this side.
    pub const fn contains(self, seat: Seat) -> bool {
        seat.side() as u8 == self as u8
    }
}

/// Which sides are vulnerable.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Vulnerability {
    /// Neither side ("Love" / "None").
    None = 0,
    /// North–South only.
    NS = 1,
    /// East–West only.
    EW = 2,
    /// Both sides ("All" / "Both").
    Both = 3,
}

impl Vulnerability {
    /// The vulnerability with index `i` (`0..4`).
    ///
    /// # Panics
    /// Panics if `i >= 4`.
    pub const fn from_index(i: u8) -> Vulnerability {
        match i {
            0 => Vulnerability::None,
            1 => Vulnerability::NS,
            2 => Vulnerability::EW,
            3 => Vulnerability::Both,
            _ => panic!("vulnerability index out of range"),
        }
    }

    /// Index `0..4`.
    pub const fn index(self) -> u8 {
        self as u8
    }

    /// The standard vulnerability of a board number (16-board cycle; board 0 behaves like 16).
    pub const fn from_board_number(number: u16) -> Vulnerability {
        let i = ((number + 15) % 16) as u8;
        Vulnerability::from_index((i / 4 + i % 4) % 4)
    }

    /// Whether `side` is vulnerable.
    pub const fn is_vulnerable_side(self, side: Side) -> bool {
        matches!(
            (self, side),
            (Vulnerability::Both, _)
                | (Vulnerability::NS, Side::NS)
                | (Vulnerability::EW, Side::EW)
        )
    }

    /// Whether `seat`'s side is vulnerable.
    pub const fn is_vulnerable(self, seat: Seat) -> bool {
        self.is_vulnerable_side(seat.side())
    }
}
