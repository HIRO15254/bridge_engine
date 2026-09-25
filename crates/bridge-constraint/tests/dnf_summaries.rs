//! `HandConstraint::hcp_range` and `HandConstraint::shapes` are over-approximations: every hand
//! that satisfies the constraint has its HCP and shape inside the summary, though the converse
//! need not hold.

mod common;

use common::{arb_constraint, arb_hand13};
use proptest::prelude::*;

proptest! {
    #[test]
    fn summaries_are_supersets(
        c in arb_constraint(3),
        hands in prop::collection::vec(arb_hand13(), 30),
    ) {
        let hcp_range = c.hcp_range();
        let shapes = c.shapes();
        for hand in hands {
            if c.satisfies(hand) {
                prop_assert!(
                    hcp_range.contains(&bridge_eval::hcp(hand)),
                    "hcp_range {hcp_range:?} does not contain a satisfying hand's HCP"
                );
                prop_assert!(
                    shapes.contains(hand.shape()),
                    "shapes summary does not contain a satisfying hand's shape"
                );
            }
        }
    }
}
