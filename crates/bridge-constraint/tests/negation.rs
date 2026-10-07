//! `Atom::negate` is an exclusive, complementary decomposition (D4): for any hand, exactly one of
//! the original atom and its negation's atoms is satisfied, and no two negation atoms are ever
//! satisfied together.
//!
//! `HandConstraint::And`'s own negation (via `Nnf::from_constraint`) must keep the same property:
//! `¬(C1∧…∧Cn)`'s DNF terms have to be pairwise disjoint, or `Sampler::count()` (which just sums
//! every term's count) over- or double-counts hands that violate more than one `Cj`.

mod common;

use bridge_constraint::{
    Atom, DistMethod, EvalRequirement, HandConstraint, Metric, SampleOptions, Sampler,
};
use bridge_core::Hand;
use common::{arb_atom_safe, arb_dist_method, arb_hand13, arb_range, arb_shapeset};
use proptest::prelude::*;

/// `C(52, 13)`: every 13-card hand out of a 52-card deck (see `sampler/term.rs`'s `binomial` doc).
const FULL_DECK_HANDS: u64 = 635_013_559_600;

/// An atom with only `shapes`/`hcp` (no `cards`, no `eval`): every DNF term built from these
/// stays exact (`sampler::term::classify` only sets `needs_full_check` for a multi-suit `cards`
/// mask, and only `eval` competes for the additive-feature budget), so this test is purely about
/// `And`'s negation logic, not about rejection-path or additive-feature-budget concerns (already
/// covered elsewhere).
fn arb_atom_exact() -> impl Strategy<Value = Atom> {
    (arb_shapeset(), arb_range(37)).prop_map(|(shapes, hcp)| Atom {
        shapes,
        hcp,
        ..Atom::ANY
    })
}

/// A random tree of atoms combined with (only) `And` — no `Or`, no `Custom`. `Or`'s own DNF
/// terms are not necessarily disjoint even when its children are exact (`Or(a, b)`'s terms
/// double-count a hand that satisfies both `a` and `b`, if `a` and `b` overlap), so `count()` on
/// an `Or` is not in general the exact set size; that is a separate, pre-existing property of
/// `Or`, not what this test is about. Restricting to nested `And` keeps every positive node's
/// `count()` exact and isolates the property this test checks: `And`'s own negation (the only
/// place an `Or` appears here) must itself decompose into pairwise-disjoint terms.
fn arb_atom_tree(max_depth: u32) -> impl Strategy<Value = HandConstraint> {
    let leaf = arb_atom_exact().prop_map(HandConstraint::Atom);
    leaf.prop_recursive(max_depth, 16, 4, |inner| {
        prop::collection::vec(inner, 2..4).prop_map(HandConstraint::And)
    })
}

fn exact_count(c: &HandConstraint) -> u64 {
    let sampler = Sampler::prepare(c, Hand::FULL, Hand::EMPTY, &SampleOptions::default())
        .expect("Hand::FULL/Hand::EMPTY never overlap, and the default options allow rejection");
    assert!(
        sampler.is_exact(),
        "an atom/And/Or-only tree never needs rejection"
    );
    sampler.count()
}

/// The specific two-atom example the finding is about: `a ∧ b` where `a` and `b` overlap (so
/// plain De Morgan's `¬a ∨ ¬b` is not a disjoint decomposition of `¬(a∧b)`).
#[test]
fn and_of_two_overlapping_atoms_negates_to_an_exact_complement() {
    let a = Atom {
        hcp: 12..=17,
        ..Atom::ANY
    };
    let b = Atom {
        shapes: bridge_constraint::ShapeSet::BALANCED,
        ..Atom::ANY
    };
    let c = HandConstraint::Atom(a).and(HandConstraint::Atom(b));
    let not_c = c.clone().not();

    // Both `a` and `b` alone hold for plenty of hands (neither is trivially false), so `¬a` and
    // `¬b` are not disjoint: a hand outside `a`'s HCP window that is also unbalanced satisfies
    // both. Plain De Morgan (`¬a ∨ ¬b`) would count every such hand twice.
    assert_eq!(exact_count(&c) + exact_count(&not_c), FULL_DECK_HANDS);
}

proptest! {
    /// General form of the same property: for any atom/`And`/`Or` tree (no `Custom`, so
    /// `count()` is exact on both sides), `count(c) + count(¬c) == C(52, 13)`.
    #[test]
    fn and_negation_is_exact_and_complementary(c in arb_atom_tree(3)) {
        let not_c = c.clone().not();
        let total = exact_count(&c) + exact_count(&not_c);
        prop_assert_eq!(total, FULL_DECK_HANDS);
    }
}

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
