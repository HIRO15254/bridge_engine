//! The exact (shape, HCP) grid against `HandConstraint::satisfies` (docs/design/05-constraint.md
//! §2.6; lane-S acceptance "S, grid"): over 200 random constraints (100 literal-free, 100
//! with `cards`/`eval` literals or `Custom` predicates) and random 13-card hands,
//!
//! - a literal-free constraint's grid matches `satisfies` on every hand (0 mismatches), and its
//!   bounds are exact (`sub == sup == of_exact`);
//! - for every other constraint, `sub ⊆ C ⊆ sup` on every hand;
//! - `to_atoms` is exact under its cap and a superset above it;
//! - `subtract_grid` is a superset of `branch ∧ ¬minus`, exact for literal-free branches under
//!   the cap.
//!
//! The default suite uses 2 000 hands per constraint; the acceptance size (1e5 hands) is
//! `#[ignore]`d: `cargo test -p bridge-constraint --release --test grid -- --ignored`.

mod common;

use bridge_constraint::grid::bounds;
use bridge_constraint::{
    Atom, HandConstraint, HcpShapeGrid, ShapeSet, is_literal_free, subtract_grid,
};
use bridge_core::{Card, Hand};
use common::{arb_constraint, arb_constraint_no_custom};
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

fn runner(seed: u8) -> TestRunner {
    TestRunner::new_with_rng(
        Config::default(),
        TestRng::from_seed(RngAlgorithm::ChaCha, &[seed; 32]),
    )
}

/// Strips every atom's `cards`/`eval` literals.
fn strip_literals(c: HandConstraint) -> HandConstraint {
    match c {
        HandConstraint::Atom(mut a) => {
            a.cards.clear();
            a.eval.clear();
            HandConstraint::Atom(a)
        }
        HandConstraint::Or(v) => HandConstraint::Or(v.into_iter().map(strip_literals).collect()),
        HandConstraint::And(v) => HandConstraint::And(v.into_iter().map(strip_literals).collect()),
        HandConstraint::Not(inner) => HandConstraint::Not(Box::new(strip_literals(*inner))),
        custom @ HandConstraint::Custom(_) => custom,
    }
}

/// 100 literal-free and 100 literal-carrying constraints (depth <= 3), deterministic.
fn constraints() -> (Vec<HandConstraint>, Vec<HandConstraint>) {
    let mut run = runner(0x61);
    let free_strategy = arb_constraint_no_custom(3).prop_map(strip_literals);
    let mut free = Vec::new();
    while free.len() < 100 {
        free.push(free_strategy.new_tree(&mut run).unwrap().current());
    }
    let with_strategy = arb_constraint(3);
    let mut with = Vec::new();
    while with.len() < 100 {
        let c = with_strategy.new_tree(&mut run).unwrap().current();
        if !is_literal_free(&c) {
            with.push(c);
        }
    }
    (free, with)
}

/// A deterministic pseudo-random 13-card hand (splitmix64 Fisher-Yates).
fn random_hand(seed: &mut u64) -> Hand {
    let mut next = || {
        *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut cards: Vec<u8> = (0..52).collect();
    let mut hand = Hand::EMPTY;
    for i in 0..13 {
        let j = i + (next() % (52 - i as u64)) as usize;
        cards.swap(i, j);
        hand = hand.with(Card::from_index(cards[i]).expect("index < 52"));
    }
    hand
}

/// Counts of the grid check; every mismatch counter must be 0.
#[derive(Debug, Default)]
struct Report {
    checks: u64,
    satisfied: u64,
    literal_free_mismatches: u64,
    sub_violations: u64,
    sup_violations: u64,
}

fn check_grid(hands_per_constraint: usize) -> Report {
    let (free, with) = constraints();
    let mut report = Report::default();
    let mut seed = 0x6121_0001u64;
    for c in &free {
        assert!(is_literal_free(c));
        let exact = HcpShapeGrid::of_exact(c).expect("literal-free");
        let b = bounds(c);
        assert!(b.is_exact());
        assert_eq!(b.sub, exact);
        for _ in 0..hands_per_constraint {
            let hand = random_hand(&mut seed);
            let sat = c.satisfies(hand);
            report.checks += 1;
            report.satisfied += u64::from(sat);
            if exact.contains(hand) != sat {
                report.literal_free_mismatches += 1;
            }
        }
    }
    for c in &with {
        assert!(HcpShapeGrid::of_exact(c).is_none());
        let b = bounds(c);
        assert!(b.sub.is_subset(&b.sup));
        for _ in 0..hands_per_constraint {
            let hand = random_hand(&mut seed);
            let sat = c.satisfies(hand);
            report.checks += 1;
            report.satisfied += u64::from(sat);
            if b.sub.contains(hand) && !sat {
                report.sub_violations += 1;
            }
            if sat && !b.sup.contains(hand) {
                report.sup_violations += 1;
            }
        }
    }
    assert_eq!(report.literal_free_mismatches, 0, "{report:?}");
    assert_eq!(report.sub_violations, 0, "{report:?}");
    assert_eq!(report.sup_violations, 0, "{report:?}");
    report
}

#[test]
fn grid_matches_satisfies_on_random_constraints() {
    let report = check_grid(2_000);
    eprintln!("grid: {report:?}");
}

#[test]
#[ignore = "acceptance size (200 constraints x 1e5 hands); run in release with --ignored"]
fn grid_matches_satisfies_on_random_constraints_1e5() {
    let started = std::time::Instant::now();
    let report = check_grid(100_000);
    eprintln!("grid 1e5: {report:?} in {:?}", started.elapsed());
}

#[test]
fn to_atoms_round_trips_under_the_cap_and_widens_above_it() {
    let (free, _) = constraints();
    for c in &free {
        let g = HcpShapeGrid::of_exact(c).unwrap();
        let runs = g.runs().len();
        let exact = g.to_constraint(&Atom::ANY, runs.max(1));
        assert_eq!(HcpShapeGrid::of_exact(&exact), Some(g));
        let atoms = g.to_atoms(&Atom::ANY, 2);
        assert!(atoms.len() <= 2);
        let widened = atoms.iter().fold(HcpShapeGrid::EMPTY, |acc, a| {
            acc.or(&HcpShapeGrid::of_atom_box(a))
        });
        assert!(g.is_subset(&widened));
        // Pairwise disjoint.
        for (i, a) in atoms.iter().enumerate() {
            for b in &atoms[i + 1..] {
                assert!(!HcpShapeGrid::of_atom_box(a).intersects(&HcpShapeGrid::of_atom_box(b)));
            }
        }
    }
}

#[test]
fn subtract_grid_covers_the_exact_difference() {
    let (free, with) = constraints();
    let minus = HcpShapeGrid::from_box(ShapeSet::BALANCED, 12..=17);
    let mut seed = 0x6121_0002u64;
    for branch in free.iter().chain(&with) {
        let proposal = subtract_grid(branch, &minus, 8);
        let exact_under_cap =
            HcpShapeGrid::of_exact(branch).is_some_and(|g| g.diff(&minus).runs().len() <= 8);
        for _ in 0..300 {
            let hand = random_hand(&mut seed);
            let want = branch.satisfies(hand) && !minus.contains(hand);
            let got = proposal.satisfies(hand);
            assert!(!want || got, "under-cover: {branch:?} minus {minus:?}");
            if exact_under_cap {
                assert_eq!(want, got);
            }
        }
    }
}
