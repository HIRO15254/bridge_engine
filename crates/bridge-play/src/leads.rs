//! Opening-lead rules.
//!
//! | Event | Agreement | Constraint on the leader's original hand (suit `u`, led rank `r`) | w |
//! | --- | --- | --- | --- |
//! | spot ≤ 9 | 4th best | `len[u] ≥ 4 ∧ exactly 3 cards above r` | 0.8 |
//! | spot | 3rd/5th | `(len 3 ∧ 2 above) ∨ (len ≥ 5 ∧ 4 above)` | 0.8 |
//! | spot | attitude | high (≥ 7): no honour in `u`; low (≤ 5): ≥ 1 honour | 0.7 |
//! | K vs suit | standard | `AK ∨ KQ` | 0.9 |
//! | K vs NT | standard | `KQ ∧ (J ∨ T)` (or AKJT branch) | 0.85 |
//! | K/Q/J/T/9 | Rusinow | holds the next higher honour | 0.9 |
//! | J | jack denies | no A, K, Q; T: `J ∧ (A ∨ K ∨ Q)` | 0.9 |

use bridge_constraint::HandConstraint;
use bridge_core::{Card, Contract};

use crate::LeadStyle;

/// The weighted constraints implied by an opening lead of `card` against `contract`.
pub fn lead_constraints(
    card: Card,
    contract: &Contract,
    style: &LeadStyle,
) -> Vec<(HandConstraint, f32)> {
    todo!("phase 5")
}
