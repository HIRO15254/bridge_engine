//! `HandConstraint::to_dnf` must agree with direct evaluation for arbitrary constraint trees,
//! including `Not`, `Or`, `And` and a `Custom` predicate.

mod common;

use bridge_constraint::DnfOptions;
use common::{arb_constraint, arb_hand13};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// `c.satisfies(h) == dnf.terms.iter().any(|t| t.satisfies(h))` for a random tree of depth
    /// at most 3 (atoms, `Custom`, `Not`, `Or`, `And`) and a random 13-card hand.
    #[test]
    fn dnf_matches_direct_evaluation(c in arb_constraint(3), hand in arb_hand13()) {
        let dnf = c.to_dnf(&DnfOptions::default()).expect("default options never error");
        let via_dnf = dnf.terms.iter().any(|term| term.satisfies(hand));
        prop_assert_eq!(c.satisfies(hand), via_dnf);
    }
}
