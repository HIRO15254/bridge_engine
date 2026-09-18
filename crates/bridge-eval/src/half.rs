//! Half-unit quantities (losers, quick tricks).

/// A non-negative quantity in units of one half: `value = halves / 2`.
///
/// Used for losing-trick counts and quick tricks, which are conventionally counted in halves.
/// The derived `Ord` on the inner count is the correct numeric order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Half(u8);

impl Half {
    /// Zero.
    pub const ZERO: Half = Half(0);

    /// `n / 2`.
    pub const fn from_halves(n: u8) -> Half {
        Half(n)
    }

    /// `n` (whole units).
    pub const fn from_whole(n: u8) -> Half {
        Half(n * 2)
    }

    /// Number of halves.
    pub const fn halves(self) -> u8 {
        self.0
    }

    /// Whole part.
    pub const fn whole(self) -> u8 {
        self.0 / 2
    }

    /// `true` when there is a fractional half.
    pub const fn is_half(self) -> bool {
        self.0 & 1 == 1
    }

    /// As a float.
    pub const fn as_f32(self) -> f32 {
        self.0 as f32 * 0.5
    }

    /// Saturating sum.
    pub const fn add(self, other: Half) -> Half {
        Half(self.0.saturating_add(other.0))
    }
}

impl core::ops::Add for Half {
    type Output = Half;
    fn add(self, rhs: Half) -> Half {
        Half::add(self, rhs)
    }
}

impl core::iter::Sum for Half {
    fn sum<I: Iterator<Item = Half>>(iter: I) -> Half {
        iter.fold(Half::ZERO, Half::add)
    }
}

impl core::fmt::Display for Half {
    /// `2`, `2.5` (ASCII).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.is_half() {
            write!(f, "{}.5", self.whole())
        } else {
            write!(f, "{}", self.whole())
        }
    }
}
