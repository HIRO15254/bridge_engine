//! A set of ranks in one suit.

use crate::Rank;

/// The cards of one suit, as a 13-bit set: bit `r` is set when rank `r` is held.
///
/// Because ranks are ascending (Ace = bit 12), `highest()` is a `leading_zeros` and the numeric
/// value of the holding is a valid index into 8192-entry evaluation tables.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Holding(u16);

impl Holding {
    /// No cards.
    pub const EMPTY: Holding = Holding(0);
    /// All thirteen cards of the suit.
    pub const FULL: Holding = Holding(0x1FFF);

    /// Builds a holding from its bit pattern, or `None` if any bit above 12 is set.
    pub const fn from_bits(bits: u16) -> Option<Holding> {
        if bits & !0x1FFF == 0 {
            Some(Holding(bits))
        } else {
            None
        }
    }

    /// The raw 13-bit pattern.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Number of cards held.
    pub const fn len(self) -> u8 {
        self.0.count_ones() as u8
    }

    /// `true` when no card is held.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether `rank` is held.
    pub const fn contains(self, rank: Rank) -> bool {
        (self.0 >> rank.index()) & 1 == 1
    }

    /// This holding plus `rank`.
    pub const fn with(self, rank: Rank) -> Holding {
        Holding(self.0 | (1 << rank.index()))
    }

    /// This holding minus `rank`.
    pub const fn without(self, rank: Rank) -> Holding {
        Holding(self.0 & !(1 << rank.index()))
    }

    /// The highest rank held, or `None` when empty.
    pub const fn highest(self) -> Option<Rank> {
        if self.0 == 0 {
            None
        } else {
            Some(Rank::from_index((15 - self.0.leading_zeros()) as u8))
        }
    }

    /// The lowest rank held, or `None` when empty.
    pub const fn lowest(self) -> Option<Rank> {
        if self.0 == 0 {
            None
        } else {
            Some(Rank::from_index(self.0.trailing_zeros() as u8))
        }
    }

    /// The `n` highest ranks (A, K, Q, …) as a holding; `n` is clamped to 13.
    pub const fn top_ranks(n: u8) -> Holding {
        let n = if n > 13 { 13 } else { n };
        if n == 0 {
            Holding(0)
        } else {
            Holding((0x1FFF >> (13 - n)) << (13 - n))
        }
    }

    /// Set union.
    pub const fn union(self, other: Holding) -> Holding {
        Holding(self.0 | other.0)
    }

    /// Set intersection.
    pub const fn intersect(self, other: Holding) -> Holding {
        Holding(self.0 & other.0)
    }

    /// Set difference `self \ other`.
    pub const fn difference(self, other: Holding) -> Holding {
        Holding(self.0 & !other.0)
    }

    /// Complement within the suit (always masked to 13 bits).
    pub const fn complement(self) -> Holding {
        Holding(!self.0 & 0x1FFF)
    }

    /// `true` when `self ⊆ other`.
    pub const fn is_subset(self, other: Holding) -> bool {
        self.0 & !other.0 == 0
    }

    /// Ranks held, highest first.
    pub fn ranks(self) -> HoldingRanks {
        HoldingRanks { bits: self.0 }
    }

    /// Every sub-holding of `self` (including `EMPTY` and `self`), in descending numeric order.
    ///
    /// This is the `sub = (sub - 1) & self` walk used by the constraint sampler to enumerate
    /// the candidate holdings of a suit.
    pub fn submasks(self) -> Submasks {
        Submasks {
            mask: self.0,
            current: Some(self.0),
        }
    }
}

/// Iterator over the ranks of a [`Holding`], highest first.
#[derive(Clone, Debug)]
pub struct HoldingRanks {
    bits: u16,
}

impl Iterator for HoldingRanks {
    type Item = Rank;

    #[inline]
    fn next(&mut self) -> Option<Rank> {
        if self.bits == 0 {
            return None;
        }
        let r = (15 - self.bits.leading_zeros()) as u8;
        self.bits &= !(1 << r);
        Some(Rank::from_index(r))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.bits.count_ones() as usize;
        (n, Some(n))
    }
}

impl ExactSizeIterator for HoldingRanks {}
impl core::iter::FusedIterator for HoldingRanks {}

/// Iterator over every sub-holding of a [`Holding`] (see [`Holding::submasks`]).
#[derive(Clone, Debug)]
pub struct Submasks {
    mask: u16,
    current: Option<u16>,
}

impl Iterator for Submasks {
    type Item = Holding;

    #[inline]
    fn next(&mut self) -> Option<Holding> {
        let current = self.current?;
        self.current = if current == 0 {
            None
        } else {
            Some((current - 1) & self.mask)
        };
        Some(Holding(current))
    }
}

impl core::iter::FusedIterator for Submasks {}

impl core::ops::BitOr for Holding {
    type Output = Holding;
    fn bitor(self, rhs: Holding) -> Holding {
        self.union(rhs)
    }
}

impl core::ops::BitAnd for Holding {
    type Output = Holding;
    fn bitand(self, rhs: Holding) -> Holding {
        self.intersect(rhs)
    }
}

impl core::ops::Sub for Holding {
    type Output = Holding;
    fn sub(self, rhs: Holding) -> Holding {
        self.difference(rhs)
    }
}

impl core::ops::Not for Holding {
    type Output = Holding;
    fn not(self) -> Holding {
        self.complement()
    }
}

impl core::fmt::Debug for Holding {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(self, f)
    }
}
