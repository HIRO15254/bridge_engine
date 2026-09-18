//! Signal rules (attitude, count, first discard).

use bridge_constraint::HandConstraint;
use bridge_core::{Card, Seat};

use crate::{DiscardTable, SignalTable};

/// A defender's card in a position where a signal applies.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SignalEvent {
    /// The defender.
    pub seat: Seat,
    /// The card.
    pub card: Card,
    /// What kind of signal the position calls for.
    pub kind: SignalKind,
}

/// Signal positions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(missing_docs)]
pub enum SignalKind {
    Attitude,
    Count,
    FirstDiscard,
}

/// The weighted constraints implied by a signal.
pub fn signal_constraints(
    event: SignalEvent,
    signals: &SignalTable,
    discards: &DiscardTable,
) -> Vec<(HandConstraint, f32)> {
    todo!("phase 5")
}
