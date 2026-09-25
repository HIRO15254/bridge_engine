//! `Atom::negate` is an exclusive, complementary decomposition (D4): for any hand, exactly one of
//! the original atom and its negation's atoms is satisfied, and no two negation atoms are ever
//! satisfied together.

mod common;

use bridge_constraint::{Atom, DistMethod, EvalRequirement, Metric};
use common::{arb_atom_safe, arb_dist_method, arb_hand13, arb_range};
use proptest::prelude::*;

proptest! {
    /// Exactly one of `atom.satisfies(h)` and "some atom of `atom.negate()` satisfies `h`" holds,
    /// and at most one negation atom ever holds for the same hand (pairwise disjoint).
    #[test]
    fn negation_is_exclusive_and_complementary(
        atom in arb_atom_safe(),
        hands in prop::collection::vec(arb_hand13(), 20),
    ) {
        let negated = atom.negate();
        for hand in hands {
            let positive = atom.satisfies(hand);
            let matches: usize = negated.iter().filter(|a| a.satisfies(hand)).count();
            prop_assert_eq!(positive, matches == 0);
            prop_assert!(matches <= 1, "hand satisfied {matches} negation atoms, expected at most 1");
        }
    }

    /// `(A ∩ B).satisfies(h) == A.satisfies(h) && B.satisfies(h)`.
    #[test]
    fn intersect_matches_conjunction(
        a in arb_atom_safe(),
        b in arb_atom_safe(),
        hands in prop::collection::vec(arb_hand13(), 20),
    ) {
        let both = a.intersect(&b);
        for hand in hands {
            prop_assert_eq!(both.satisfies(hand), a.satisfies(hand) && b.satisfies(hand));
        }
    }

    /// Same as `negation_is_exclusive_and_complementary`, for the two metrics that depend on
    /// `bridge_eval::distribution_points` (2.1, implemented by another lane in parallel).
    #[test]
    #[ignore = "needs bridge-eval 2.1 (distribution_points)"]
    fn negation_is_exclusive_and_complementary_dist_points(
        method in arb_dist_method(),
        range in arb_range(Metric::DistPoints(DistMethod::LongSuit).max()),
        hands in prop::collection::vec(arb_hand13(), 20),
    ) {
        let atom = Atom::ANY.with_eval(EvalRequirement { metric: Metric::DistPoints(method), range });
        let negated = atom.negate();
        for hand in hands {
            let positive = atom.satisfies(hand);
            let matches: usize = negated.iter().filter(|a| a.satisfies(hand)).count();
            prop_assert_eq!(positive, matches == 0);
            prop_assert!(matches <= 1);
        }
    }

    #[test]
    #[ignore = "needs bridge-eval 2.1 (distribution_points)"]
    fn negation_is_exclusive_and_complementary_total_points(
        method in arb_dist_method(),
        range in arb_range(Metric::TotalPoints(DistMethod::LongSuit).max()),
        hands in prop::collection::vec(arb_hand13(), 20),
    ) {
        let atom = Atom::ANY.with_eval(EvalRequirement { metric: Metric::TotalPoints(method), range });
        let negated = atom.negate();
        for hand in hands {
            let positive = atom.satisfies(hand);
            let matches: usize = negated.iter().filter(|a| a.satisfies(hand)).count();
            prop_assert_eq!(positive, matches == 0);
            prop_assert!(matches <= 1);
        }
    }
}
