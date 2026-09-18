//! Distribution points and losing-trick-count variants.

use bridge_core::{Hand, Shape};

/// Which losing-trick count to use.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LtcMethod {
    /// `min(len, 3) − A − [K ∧ len ≥ 2] − [Q ∧ len ≥ 3]` per suit.
    Classic,
    /// Missing A = 1.5, missing K = 1, missing Q = 0.5 per suit (while the suit is long enough).
    New,
}

/// How distribution is converted to points. A bidding system declares which method it assumes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DistMethod {
    /// Short-suit points: `Σ_suit [len = 0]·void + [len = 1]·singleton + [len = 2]·doubleton`.
    ShortSuit {
        /// Points for a void.
        void: u8,
        /// Points for a singleton.
        singleton: u8,
        /// Points for a doubleton.
        doubleton: u8,
    },
    /// Long-suit points: `Σ_suit max(0, len − 4)`.
    LongSuit,
    /// Bergen "starting points": long-suit points, +1 per suit of 4+ cards with 3+ honours, and
    /// the adjust-3 correction (+1 when `aces + tens − queens − jacks ≥ 3`, −1 when `≤ −3`).
    BergenStarting,
}

impl DistMethod {
    /// Goren 3-2-1 (void 3, singleton 2, doubleton 1).
    pub const GOREN_321: DistMethod = DistMethod::ShortSuit {
        void: 3,
        singleton: 2,
        doubleton: 1,
    };
    /// Dummy points 5-3-1.
    pub const DUMMY_531: DistMethod = DistMethod::ShortSuit {
        void: 5,
        singleton: 3,
        doubleton: 1,
    };

    /// `true` when the value depends on the [`Shape`] alone (`ShortSuit`, `LongSuit`), which lets
    /// the constraint sampler treat the metric exactly by filtering shapes.
    pub const fn is_shape_only(self) -> bool {
        !matches!(self, DistMethod::BergenStarting)
    }
}

/// Distribution points of `hand`. Signed because the Bergen adjust-3 correction can be negative.
pub fn distribution_points(hand: Hand, method: DistMethod) -> i8 {
    todo!("phase 2")
}

/// Distribution points as a function of the shape alone, or `None` for methods that also look
/// at the cards (see [`DistMethod::is_shape_only`]).
pub fn shape_points(shape: Shape, method: DistMethod) -> Option<i8> {
    todo!("phase 2")
}

/// `hcp + distribution_points`, saturating at zero.
pub fn total_points(hand: Hand, method: DistMethod) -> u8 {
    let total = crate::hcp(hand) as i16 + distribution_points(hand, method) as i16;
    total.max(0) as u8
}
