//! Exhaustive small-pool tests (2.4-2.6): random constraints built from shape/HCP/card/eval
//! literals, an `Or` of overlapping atoms, and a `Not`, checked against a brute-force enumeration
//! of every completion of a 16-20 card pool plus a 0-6 card fixed part.

mod common;

use bridge_constraint::{Atom, HandConstraint, Metric, SampleOptions, Sampler};
use bridge_core::{Card, Hand};
use common::{arb_atom_dist_shape_only, arb_atom_safe};
use proptest::prelude::*;
use proptest::sample::Index;

/// A pool of 16-20 cards plus a disjoint fixed part of 0-6 cards, drawn from one shuffled deck.
fn arb_pool_fixed() -> impl Strategy<Value = (Hand, Hand)> {
    (
        16usize..=20,
        0usize..=6,
        prop::collection::vec(any::<Index>(), 52),
    )
        .prop_map(|(pool_size, fixed_size, indices)| {
            let mut cards: Vec<u8> = (0..52).collect();
            for (i, ix) in indices.iter().enumerate() {
                let j = i + ix.index(52 - i);
                cards.swap(i, j);
            }
            let mut pool = Hand::EMPTY;
            let mut fixed = Hand::EMPTY;
            for &c in &cards[..pool_size] {
                pool = pool.with(Card::from_index(c).expect("index < 52"));
            }
            for &c in &cards[pool_size..pool_size + fixed_size] {
                fixed = fixed.with(Card::from_index(c).expect("index < 52"));
            }
            (pool, fixed)
        })
}

/// The number of pool cards a completion still needs.
fn needed(fixed: Hand) -> usize {
    13 - fixed.len() as usize
}

/// Advances `idxs` (a strictly increasing `m`-subset of `0..n`) to the next combination in
/// lexicographic order; `false` once every combination has been visited.
fn next_combination(idxs: &mut [usize], n: usize) -> bool {
    let m = idxs.len();
    let mut i = m;
    loop {
        if i == 0 {
            return false;
        }
        i -= 1;
        if idxs[i] != i + n - m {
            idxs[i] += 1;
            for j in i + 1..m {
                idxs[j] = idxs[j - 1] + 1;
            }
            return true;
        }
    }
}

/// Every completion `fixed ∪ (m cards from pool)`, as a full 13-card `Hand`.
fn every_completion(pool: Hand, fixed: Hand) -> Vec<Hand> {
    let cards: Vec<Card> = pool.cards().collect();
    let n = cards.len();
    let m = needed(fixed);
    if m > n {
        return Vec::new();
    }
    let mut idxs: Vec<usize> = (0..m).collect();
    let mut out = Vec::new();
    loop {
        let mut hand = fixed;
        for &ix in &idxs {
            hand = hand.with(cards[ix]);
        }
        out.push(hand);
        if !next_combination(&mut idxs, n) {
            break;
        }
    }
    out
}

fn brute_force_count(pool: Hand, fixed: Hand, pred: impl Fn(Hand) -> bool) -> u64 {
    every_completion(pool, fixed)
        .into_iter()
        .filter(|&h| pred(h))
        .count() as u64
}

/// Number of literals `PreparedTerm::prepare` can offer at most one additive-feature slot to: a
/// multi-suit `CardRequirement`, or an eval requirement on `Controls`/`Losers`/`QuickTricks`. A
/// shape-only `DistPoints`/`TotalPoints` requirement (`GOREN_321`, `DUMMY_531`, `LongSuit`) is
/// routed through `dist_shape_filters`/`total_shape_shifts` instead and so does not count here.
/// Neither `arb_atom_safe` nor `arb_atom_dist_shape_only` ever produces `DistMethod::
/// BergenStarting` or a `Custom`, so an atom with at most one additive-feature candidate is
/// guaranteed to sample exactly (mirrors `sampler::term::classify`).
fn additive_candidate_count(atom: &Atom) -> usize {
    let cards = atom
        .cards
        .iter()
        .filter(|req| req.single_suit().is_none())
        .count();
    let eval = atom
        .eval
        .iter()
        .filter(|req| {
            matches!(
                req.metric,
                Metric::Controls | Metric::Losers(_) | Metric::QuickTricks
            )
        })
        .count();
    cards + eval
}

/// Whether `Sampler::prepare` is expected to report `is_exact()` for a lone `Atom`: trivially
/// unsatisfiable atoms drop out of the DNF entirely (zero terms, vacuously exact regardless of
/// their literals), otherwise it comes down to the additive-feature budget.
fn is_effectively_exact(atom: &Atom) -> bool {
    atom.is_trivially_unsat() || additive_candidate_count(atom) <= 1
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// A single atom (shape ranges, HCP window, single/multi-suit card requirements,
    /// controls/losers/quick-tricks ranges): `count()` matches brute force, `log_prob` sums to 1
    /// over the satisfying hands and is exactly `-inf` off the support, and `is_exact()` matches
    /// the literal budget.
    #[test]
    fn single_atom_matches_brute_force(
        (pool, fixed) in arb_pool_fixed(),
        atom in arb_atom_safe(),
    ) {
        let c = HandConstraint::Atom(atom.clone());
        let sampler = Sampler::prepare(&c, pool, fixed, &SampleOptions::default())
            .expect("pool/fixed are disjoint by construction");

        let expected_exact = is_effectively_exact(&atom);
        prop_assert_eq!(sampler.is_exact(), expected_exact);

        let all = every_completion(pool, fixed);
        let brute = all.iter().filter(|&&h| atom.satisfies(h)).count() as u64;
        if expected_exact {
            prop_assert_eq!(sampler.count(), brute);
        }

        let mut sum = 0.0f64;
        for &h in &all {
            let lp = sampler.log_prob(h);
            if atom.satisfies(h) {
                if expected_exact {
                    prop_assert!(lp.is_finite(), "expected finite log_prob for a satisfying hand");
                    sum += lp.exp();
                }
            } else {
                prop_assert_eq!(lp, f64::NEG_INFINITY, "expected -inf off the support");
            }
        }
        if expected_exact && brute > 0 {
            prop_assert!((sum - 1.0).abs() < 1e-9, "sum of exp(log_prob) = {sum}");
        }
    }

    /// Same as `single_atom_matches_brute_force`, but the atom's eval requirements may also draw a
    /// shape-only `DistPoints`/`TotalPoints` metric (`GOREN_321`, `DUMMY_531`, `LongSuit`): the
    /// sampler routes these through `dist_shape_filters`/`total_shape_shifts` (shape filtering and
    /// HCP-window shifting) rather than rejection, so they should sample exactly whenever the
    /// atom's other literals do.
    #[test]
    fn single_atom_dist_shape_only_matches_brute_force(
        (pool, fixed) in arb_pool_fixed(),
        atom in arb_atom_dist_shape_only(),
    ) {
        let c = HandConstraint::Atom(atom.clone());
        let sampler = Sampler::prepare(&c, pool, fixed, &SampleOptions::default())
            .expect("pool/fixed are disjoint by construction");

        let expected_exact = is_effectively_exact(&atom);
        prop_assert_eq!(sampler.is_exact(), expected_exact);

        let all = every_completion(pool, fixed);
        let brute = all.iter().filter(|&&h| atom.satisfies(h)).count() as u64;
        if expected_exact {
            prop_assert_eq!(sampler.count(), brute);
        }

        let mut sum = 0.0f64;
        for &h in &all {
            let lp = sampler.log_prob(h);
            if atom.satisfies(h) {
                if expected_exact {
                    prop_assert!(lp.is_finite(), "expected finite log_prob for a satisfying hand");
                    sum += lp.exp();
                }
            } else {
                prop_assert_eq!(lp, f64::NEG_INFINITY, "expected -inf off the support");
            }
        }
        if expected_exact && brute > 0 {
            prop_assert!((sum - 1.0).abs() < 1e-9, "sum of exp(log_prob) = {sum}");
        }
    }

    /// `Or` of two (possibly overlapping) atoms: `count()` is the sum of the branches' own
    /// brute-force counts (§8.2: overlapping `Or` terms are not deduplicated), and `log_prob`
    /// still sums to 1 over the *distinct* satisfying hands.
    #[test]
    fn or_of_overlapping_atoms(
        (pool, fixed) in arb_pool_fixed(),
        a in arb_atom_safe(),
        b in arb_atom_safe(),
    ) {
        let c = HandConstraint::Atom(a.clone()).or(HandConstraint::Atom(b.clone()));
        let sampler = Sampler::prepare(&c, pool, fixed, &SampleOptions::default())
            .expect("pool/fixed are disjoint by construction");

        let exact = is_effectively_exact(&a) && is_effectively_exact(&b);
        prop_assert_eq!(sampler.is_exact(), exact);
        if !exact {
            return Ok(());
        }

        let all = every_completion(pool, fixed);
        let count_a = all.iter().filter(|&&h| a.satisfies(h)).count() as u64;
        let count_b = all.iter().filter(|&&h| b.satisfies(h)).count() as u64;
        prop_assert_eq!(sampler.count(), count_a + count_b);

        let satisfying: Vec<Hand> = all
            .iter()
            .copied()
            .filter(|&h| a.satisfies(h) || b.satisfies(h))
            .collect();
        let sum: f64 = satisfying.iter().map(|&h| sampler.log_prob(h).exp()).sum();
        if !satisfying.is_empty() {
            prop_assert!((sum - 1.0).abs() < 1e-9, "sum of exp(log_prob) = {sum}");
        }
        for &h in &all {
            if !(a.satisfies(h) || b.satisfies(h)) {
                prop_assert_eq!(sampler.log_prob(h), f64::NEG_INFINITY);
            }
        }
    }

    /// `Not(atom)`: negation-derived terms are pairwise disjoint (D4), so `count()` matches a
    /// direct brute-force count of `!atom.satisfies(h)`.
    #[test]
    fn negation_matches_brute_force(
        (pool, fixed) in arb_pool_fixed(),
        atom in arb_atom_safe(),
    ) {
        let c = HandConstraint::Atom(atom.clone()).not();
        let sampler = Sampler::prepare(&c, pool, fixed, &SampleOptions::default())
            .expect("pool/fixed are disjoint by construction");

        // Every negation-derived term keeps a subset of the original atom's literals plus one
        // negated literal of the same kind, so it never needs more additive-feature slots than
        // the original atom did.
        if !is_effectively_exact(&atom) {
            return Ok(());
        }
        prop_assert!(sampler.is_exact());

        let expected = brute_force_count(pool, fixed, |h| !atom.satisfies(h));
        prop_assert_eq!(sampler.count(), expected);
    }
}
