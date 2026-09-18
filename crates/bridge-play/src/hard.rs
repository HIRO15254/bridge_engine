//! Hard constraints from the play.
//!
//! 1. Walk the tricks; `seat_at` gives each card's player.
//! 2. Maintain `played[s]` and `shown_out[s][suit]` (a seat that did not follow the led suit).
//! 3. `hard[s]` = shapes whose length in a shown-out suit equals the number of that suit's cards
//!    the seat has played, and whose length in every other suit is at least the number played.
//! 4. Consistency: per suit, the minimum lengths must sum to at most 13; otherwise a warning
//!    (revoke or bad record). The constraints are still returned.
//!
//! Only lengths go into the constraints; card identities go into [`KnownCards`] (the sampler's
//! `fixed`), which is cheaper than card requirements.

use bridge_constraint::{HandConstraint, KnownCards};
use bridge_core::PlayHistory;

use crate::PlayWarning;

/// Length constraints and known cards implied by the play so far.
pub fn hard_constraints(
    history: &PlayHistory,
) -> ([HandConstraint; 4], KnownCards, Vec<PlayWarning>) {
    todo!("phase 5")
}
