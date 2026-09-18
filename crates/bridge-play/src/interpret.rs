//! `interpret_play`: history → constraints per seat.

use bridge_constraint::{HandConstraint, KnownCards};
use bridge_core::{Card, Contract, PlayHistory, Seat, Suit};

use crate::PlayAgreements;

/// The result of [`interpret_play`].
#[derive(Clone, Debug)]
pub struct PlayInterpretation {
    /// Cards now known to belong to each seat's original hand.
    pub known: KnownCards,
    /// Hard length constraints per seat.
    pub hard: [HandConstraint; 4],
    /// Soft weighted alternatives per seat (combined from every rule that fired).
    pub soft: [Vec<(HandConstraint, f32)>; 4],
    /// Which rule fired on which card.
    pub events: Vec<PlayEvent>,
    /// Inconsistencies found.
    pub warnings: Vec<PlayWarning>,
}

impl PlayInterpretation {
    /// The spec's return type: `hard ∧ each soft branch` per seat.
    pub fn into_seats(self) -> [Vec<(HandConstraint, f32)>; 4] {
        todo!("phase 5")
    }
}

/// An audit-trail entry.
#[derive(Clone, PartialEq, Debug)]
pub struct PlayEvent {
    /// The seat.
    pub seat: Seat,
    /// The card.
    pub card: Card,
    /// The rule name.
    pub rule: &'static str,
}

/// An inconsistency in the record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlayWarning {
    /// The minimum lengths in `suit` exceed 13 (revoke or bad record).
    Inconsistent {
        /// The suit.
        suit: Suit,
    },
    /// A seat played a suit it had shown out of.
    RevokeSuspected {
        /// Trick index.
        trick: usize,
        /// The seat.
        seat: Seat,
    },
}

/// Derives hard and soft constraints from the play so far. `agreements` is indexed by seat;
/// rules apply to defenders only.
pub fn interpret_play(
    history: &PlayHistory,
    contract: &Contract,
    agreements: &[PlayAgreements; 4],
) -> PlayInterpretation {
    todo!("phase 5")
}
