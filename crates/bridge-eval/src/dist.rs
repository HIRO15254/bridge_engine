//! Distribution points and losing-trick-count variants.

use bridge_core::{Hand, Shape, Suit};

use crate::{SUIT, aces, jacks, queens, tens};

/// Upper bound every [`DistMethod`] is assumed to stay within (`bridge_constraint::Metric::max`
/// hard-codes this same value for `DistPoints`, and `37 + MAX_DIST_POINTS` for `TotalPoints`).
/// `distribution_points` saturates a [`DistMethod::ShortSuit`] hand's score at this value, so an
/// unusually large custom weight cannot violate it.
pub const MAX_DIST_POINTS: u8 = 40;

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

/// `+1` when `aces + tens − queens − jacks >= 3`, `−1` when `<= −3`, else `0` (Bergen adjust-3).
fn adjust3(hand: Hand) -> i8 {
    let plus = aces(hand) as i16 + tens(hand) as i16;
    let minus = queens(hand) as i16 + jacks(hand) as i16;
    let delta = plus - minus;
    if delta >= 3 {
        1
    } else if delta <= -3 {
        -1
    } else {
        0
    }
}

/// Long-suit points of a [`Shape`]: `Σ_suit max(0, len − 4)`.
const fn long_suit_points(shape: Shape) -> i8 {
    let lens = shape.lens();
    let mut i = 0;
    let mut total: i8 = 0;
    while i < 4 {
        let len = lens[i];
        if len > 4 {
            total += (len - 4) as i8;
        }
        i += 1;
    }
    total
}

/// Short-suit points of a [`Shape`], saturating at [`MAX_DIST_POINTS`].
///
/// `void`/`singleton`/`doubleton` are caller-supplied (a bidding system can declare any custom
/// `DistMethod::ShortSuit`), so nothing bounds them individually; accumulating in `u16` and
/// saturating the sum (rather than adding each term straight into an `i8`) keeps the result
/// within the range every other part of the crate (and `bridge_constraint::Metric::max`) assumes
/// for a `DistPoints`/`TotalPoints` value, instead of overflowing or wrapping negative.
const fn short_suit_points(shape: Shape, void: u8, singleton: u8, doubleton: u8) -> i8 {
    let lens = shape.lens();
    let mut i = 0;
    let mut total: u16 = 0;
    while i < 4 {
        total += match lens[i] {
            0 => void as u16,
            1 => singleton as u16,
            2 => doubleton as u16,
            _ => 0,
        };
        i += 1;
    }
    if total > MAX_DIST_POINTS as u16 {
        MAX_DIST_POINTS as i8
    } else {
        total as i8
    }
}

/// Distribution points of `hand`. Signed because the Bergen adjust-3 correction can be negative.
pub fn distribution_points(hand: Hand, method: DistMethod) -> i8 {
    match method {
        DistMethod::ShortSuit {
            void,
            singleton,
            doubleton,
        } => short_suit_points(hand.shape(), void, singleton, doubleton),
        DistMethod::LongSuit => long_suit_points(hand.shape()),
        DistMethod::BergenStarting => {
            let quality_suits = Suit::ALL
                .into_iter()
                .filter(|&suit| {
                    let holding = hand.holding(suit);
                    holding.len() >= 4 && SUIT.honors5[holding.bits() as usize] >= 3
                })
                .count() as i8;
            long_suit_points(hand.shape()) + quality_suits + adjust3(hand)
        }
    }
}

/// Distribution points as a function of the shape alone, or `None` for methods that also look
/// at the cards (see [`DistMethod::is_shape_only`]).
pub fn shape_points(shape: Shape, method: DistMethod) -> Option<i8> {
    match method {
        DistMethod::ShortSuit {
            void,
            singleton,
            doubleton,
        } => Some(short_suit_points(shape, void, singleton, doubleton)),
        DistMethod::LongSuit => Some(long_suit_points(shape)),
        DistMethod::BergenStarting => None,
    }
}

/// `hcp + distribution_points`, saturating at zero.
pub fn total_points(hand: Hand, method: DistMethod) -> u8 {
    let total = crate::hcp(hand) as i16 + distribution_points(hand, method) as i16;
    total.max(0) as u8
}
