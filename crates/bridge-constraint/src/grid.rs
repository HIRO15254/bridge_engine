//! An exact set representation over the (shape, HCP) plane.
//!
//! A hand's ordered shape (one of 560) and its HCP (`0..=37`) decide every *literal-free*
//! [`Atom`] (one with `shapes` and `hcp` only, no `cards`/`eval` literals). Any Boolean
//! combination (`And`/`Or`/`Not`) of such atoms is therefore exactly a set of (shape, HCP) cells.
//! [`HcpShapeGrid`] stores that set as one [`ShapeSet`] per HCP value (`[ShapeSet; 38]`, 342
//! `u64` words), so every set operation is one pass over the words and is exact.
//!
//! Constraints with `cards`/`eval` literals or [`HandConstraint::Custom`] predicates are not
//! representable exactly; [`bounds`] returns a guaranteed subset (`sub`) and superset (`sup`) for
//! them instead, and `sub == sup` holds exactly when the constraint is literal-free.
//!
//! [`subtract_grid`] builds the over-covering flat "proposal form" of a region minus a grid, and
//! [`is_literal_free`] tells whether a constraint is exactly representable.
//!
//! Used by the exclusive index (`bridge-system`: emptiness proofs), the natural exclusion and the
//! run-time recomputation of exclusive regions in `bridge-bidding`
//! (docs/design/05-constraint.md §`grid.rs`). This module unifies the two phase-4 prototypes'
//! grids: a fixed `[ShapeSet; 38]` array (C) with sub/sup bounds (B).

use core::ops::RangeInclusive;
use std::sync::OnceLock;

use bridge_core::{Hand, MAX_HCP, MIN_HCP, SHAPES, ShapeSet};

use crate::{Atom, HandConstraint};

/// Highest HCP value of a 13-card hand.
const HCP_MAX: u8 = 37;
/// Number of HCP values (`0..=37`).
const HCP_VALUES: usize = 38;

/// A set of (shape, HCP) cells: `self.at(h)` is the set of shapes allowed at `h` HCP.
///
/// Equal sets have equal representations, so `==` is set equality. A cell may be infeasible
/// (no 13-card hand has that shape with that many HCP, e.g. a 13=0=0=0 shape with 37 HCP); the
/// set operations do not care, and [`HcpShapeGrid::feasible`] / [`HcpShapeGrid::is_empty_hands`]
/// are there when only realisable hands matter.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct HcpShapeGrid([ShapeSet; HCP_VALUES]);

impl core::fmt::Debug for HcpShapeGrid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut list = f.debug_list();
        for (h, shapes) in self.0.iter().enumerate() {
            if !shapes.is_empty() {
                list.entry(&(h, shapes.len()));
            }
        }
        list.finish()
    }
}

impl Default for HcpShapeGrid {
    /// The empty set.
    fn default() -> HcpShapeGrid {
        HcpShapeGrid::EMPTY
    }
}

impl HcpShapeGrid {
    /// The empty set.
    pub const EMPTY: HcpShapeGrid = HcpShapeGrid([ShapeSet::EMPTY; HCP_VALUES]);

    /// Every cell (every hand).
    pub const ALL: HcpShapeGrid = HcpShapeGrid([ShapeSet::ALL; HCP_VALUES]);

    /// `shapes × hcp` (the range is clamped to `0..=37`; an empty range or shape set gives the
    /// empty grid).
    pub fn from_box(shapes: ShapeSet, hcp: RangeInclusive<u8>) -> HcpShapeGrid {
        let mut g = HcpShapeGrid::EMPTY;
        let lo = *hcp.start() as usize;
        let hi = (*hcp.end()).min(HCP_MAX) as usize;
        if lo <= hi && !shapes.is_empty() {
            for cell in &mut g.0[lo..=hi] {
                *cell = shapes;
            }
        }
        g
    }

    /// The box of an atom's `shapes`/`hcp` (its `cards`/`eval` literals are ignored, so this is
    /// a superset of the atom, and equal to it when the atom is literal-free).
    pub fn of_atom_box(atom: &Atom) -> HcpShapeGrid {
        HcpShapeGrid::from_box(atom.shapes, atom.hcp.clone())
    }

    /// The exact grid of a literal-free constraint, or `None` when `c` contains an atom with
    /// `cards`/`eval` literals or a `Custom` predicate (use [`bounds`] then).
    pub fn of_exact(c: &HandConstraint) -> Option<HcpShapeGrid> {
        match c {
            HandConstraint::Atom(a) => {
                if a.cards.is_empty() && a.eval.is_empty() {
                    Some(HcpShapeGrid::of_atom_box(a))
                } else {
                    None
                }
            }
            HandConstraint::Or(children) => {
                let mut g = HcpShapeGrid::EMPTY;
                for child in children {
                    g = g.or(&HcpShapeGrid::of_exact(child)?);
                }
                Some(g)
            }
            HandConstraint::And(children) => {
                let mut g = HcpShapeGrid::ALL;
                for child in children {
                    g = g.and(&HcpShapeGrid::of_exact(child)?);
                }
                Some(g)
            }
            HandConstraint::Not(inner) => Some(HcpShapeGrid::of_exact(inner)?.not()),
            HandConstraint::Custom(_) => None,
        }
    }

    /// The cells whose shape can actually hold that many HCP in a 13-card hand (a shape's HCP
    /// range is the sum of the per-suit `MIN_HCP`/`MAX_HCP` of its lengths, capped at 37).
    /// Computed once.
    pub fn feasible() -> &'static HcpShapeGrid {
        static FEASIBLE: OnceLock<HcpShapeGrid> = OnceLock::new();
        FEASIBLE.get_or_init(|| {
            let mut g = HcpShapeGrid::EMPTY;
            for &shape in SHAPES.iter() {
                let lens = shape.lens();
                let lo: u8 = lens.iter().map(|&l| MIN_HCP[l as usize]).sum();
                let hi: u8 = lens
                    .iter()
                    .map(|&l| MAX_HCP[l as usize])
                    .sum::<u8>()
                    .min(HCP_MAX);
                for cell in &mut g.0[lo as usize..=hi as usize] {
                    *cell = cell.insert(shape);
                }
            }
            g
        })
    }

    /// The shapes allowed at `hcp` HCP (empty above 37).
    pub fn at(&self, hcp: u8) -> ShapeSet {
        self.0.get(hcp as usize).copied().unwrap_or(ShapeSet::EMPTY)
    }

    /// The 38 per-HCP shape sets, indexed by HCP.
    pub fn rows(&self) -> &[ShapeSet; 38] {
        &self.0
    }

    /// Whether `hand`'s (shape, HCP) cell is in the set. For a literal-free constraint `c`,
    /// `HcpShapeGrid::of_exact(&c).unwrap().contains(h) == c.satisfies(h)` for every hand.
    pub fn contains(&self, hand: Hand) -> bool {
        self.at(bridge_eval::hcp(hand)).contains(hand.shape())
    }

    /// `self ∩ other`.
    pub fn and(&self, other: &HcpShapeGrid) -> HcpShapeGrid {
        let mut g = *self;
        for (a, b) in g.0.iter_mut().zip(other.0.iter()) {
            *a = a.intersect(*b);
        }
        g
    }

    /// `self ∪ other`.
    pub fn or(&self, other: &HcpShapeGrid) -> HcpShapeGrid {
        let mut g = *self;
        for (a, b) in g.0.iter_mut().zip(other.0.iter()) {
            *a = a.union(*b);
        }
        g
    }

    /// `self ∖ other`.
    pub fn diff(&self, other: &HcpShapeGrid) -> HcpShapeGrid {
        let mut g = *self;
        for (a, b) in g.0.iter_mut().zip(other.0.iter()) {
            *a = a.difference(*b);
        }
        g
    }

    /// The complement (with respect to [`HcpShapeGrid::ALL`]).
    pub fn not(&self) -> HcpShapeGrid {
        let mut g = *self;
        for a in g.0.iter_mut() {
            *a = a.complement();
        }
        g
    }

    /// `true` when no cell is set (feasible or not).
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|s| s.is_empty())
    }

    /// `true` when no *feasible* cell is set, i.e. no 13-card hand lies in the set.
    pub fn is_empty_hands(&self) -> bool {
        self.and(HcpShapeGrid::feasible()).is_empty()
    }

    /// `self ⊆ other`.
    pub fn is_subset(&self, other: &HcpShapeGrid) -> bool {
        self.0
            .iter()
            .zip(other.0.iter())
            .all(|(a, b)| a.is_subset(*b))
    }

    /// Whether `self ∩ other` is non-empty (without building it).
    pub fn intersects(&self, other: &HcpShapeGrid) -> bool {
        self.0
            .iter()
            .zip(other.0.iter())
            .any(|(a, b)| !a.intersect(*b).is_empty())
    }

    /// Number of (shape, HCP) cells in the set.
    pub fn cell_count(&self) -> u32 {
        self.0.iter().map(|s| u32::from(s.len())).sum()
    }

    /// The smallest box containing the set: the union of the shape sets and the HCP span of the
    /// non-empty rows. `None` for the empty set.
    pub fn hull(&self) -> Option<(ShapeSet, RangeInclusive<u8>)> {
        let lo = self.0.iter().position(|s| !s.is_empty())?;
        let hi = self.0.iter().rposition(|s| !s.is_empty())?;
        let shapes = self.0[lo..=hi]
            .iter()
            .fold(ShapeSet::EMPTY, |acc, s| acc.union(*s));
        Some((shapes, lo as u8..=hi as u8))
    }

    /// The maximal HCP runs of equal, non-empty shape sets: `(shapes, lo..=hi)`, sorted by HCP
    /// and pairwise disjoint.
    pub fn runs(&self) -> Vec<(ShapeSet, RangeInclusive<u8>)> {
        let mut runs = Vec::new();
        let mut h = 0usize;
        while h < HCP_VALUES {
            let s = self.0[h];
            if s.is_empty() {
                h += 1;
                continue;
            }
            let mut e = h;
            while e + 1 < HCP_VALUES && self.0[e + 1] == s {
                e += 1;
            }
            runs.push((s, h as u8..=e as u8));
            h = e + 1;
        }
        runs
    }

    /// The set as atoms, one per HCP run ([`HcpShapeGrid::runs`]), each carrying `template`'s
    /// `cards`/`eval` literals and intersected with `template`'s own `shapes`/`hcp` (pass
    /// [`Atom::ANY`] for a plain literal-free result). Atoms that the intersection empties are
    /// dropped.
    ///
    /// With `template = Atom::ANY` and at most `cap` runs, the atoms are pairwise disjoint and
    /// their union is exactly the set. When there are more than `cap` runs, adjacent runs are
    /// merged (greedily, the merge adding the fewest cells first) into their shape-set union over
    /// the joint HCP span until `cap` remain: the result is then a *superset* of the set (it only
    /// ever widens) and the atoms stay pairwise disjoint. `cap == 0` is treated as 1.
    pub fn to_atoms(&self, template: &Atom, cap: usize) -> Vec<Atom> {
        let cap = cap.max(1);
        let mut runs: Vec<(ShapeSet, u8, u8)> = self
            .runs()
            .into_iter()
            .map(|(s, r)| (s, *r.start(), *r.end()))
            .collect();
        let cells = |r: &(ShapeSet, u8, u8)| u32::from(r.0.len()) * u32::from(r.2 - r.1 + 1);
        while runs.len() > cap {
            let mut best = (u32::MAX, 0usize);
            for i in 0..runs.len() - 1 {
                let merged = (runs[i].0.union(runs[i + 1].0), runs[i].1, runs[i + 1].2);
                let added = cells(&merged) - cells(&runs[i]) - cells(&runs[i + 1]);
                if added < best.0 {
                    best = (added, i);
                }
            }
            let i = best.1;
            let next = runs.remove(i + 1);
            runs[i] = (runs[i].0.union(next.0), runs[i].1, next.2);
        }
        runs.into_iter()
            .filter_map(|(shapes, lo, hi)| {
                let shapes = shapes.intersect(template.shapes);
                let lo = lo.max(*template.hcp.start());
                let hi = hi.min(*template.hcp.end());
                if shapes.is_empty() || lo > hi {
                    return None;
                }
                Some(Atom {
                    shapes,
                    hcp: lo..=hi,
                    cards: template.cards.clone(),
                    eval: template.eval.clone(),
                })
            })
            .collect()
    }

    /// [`HcpShapeGrid::to_atoms`] as a constraint: a single atom, or an `Or` of the atoms
    /// (`Or([])`, which no hand satisfies, when there are none).
    pub fn to_constraint(&self, template: &Atom, cap: usize) -> HandConstraint {
        let mut atoms: Vec<HandConstraint> = self
            .to_atoms(template, cap)
            .into_iter()
            .map(HandConstraint::Atom)
            .collect();
        if atoms.len() == 1 {
            atoms.pop().expect("one atom")
        } else {
            HandConstraint::Or(atoms)
        }
    }
}

/// A guaranteed subset and superset of a constraint's satisfying set on the (shape, HCP) plane:
/// for every hand `h`, `sub.contains(h) ⇒ c.satisfies(h) ⇒ sup.contains(h)`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GridBounds {
    /// Cells every hand of which satisfies the constraint.
    pub sub: HcpShapeGrid,
    /// Cells containing every hand that satisfies the constraint.
    pub sup: HcpShapeGrid,
}

impl GridBounds {
    /// Whether the constraint is represented exactly (`sub == sup`, which holds for every
    /// literal-free constraint).
    pub fn is_exact(&self) -> bool {
        self.sub == self.sup
    }
}

/// [`GridBounds`] of `c`, by structural recursion:
///
/// - a literal-free atom: `sub = sup = its box`;
/// - an atom with `cards`/`eval` literals: `sub = ∅`, `sup = its box`;
/// - `Custom`: `sub = ∅`, `sup = everything`;
/// - `And`/`Or`: the intersection/union of the children's bounds, componentwise;
/// - `Not`: `sub = ¬sup(inner)`, `sup = ¬sub(inner)` (the two swap).
///
/// Exact (`sub == sup`) for every literal-free constraint.
pub fn bounds(c: &HandConstraint) -> GridBounds {
    match c {
        HandConstraint::Atom(a) => {
            let sup = HcpShapeGrid::of_atom_box(a);
            let sub = if a.cards.is_empty() && a.eval.is_empty() {
                sup
            } else {
                HcpShapeGrid::EMPTY
            };
            GridBounds { sub, sup }
        }
        HandConstraint::Or(children) => {
            let mut sub = HcpShapeGrid::EMPTY;
            let mut sup = HcpShapeGrid::EMPTY;
            for child in children {
                let b = bounds(child);
                sub = sub.or(&b.sub);
                sup = sup.or(&b.sup);
            }
            GridBounds { sub, sup }
        }
        HandConstraint::And(children) => {
            let mut sub = HcpShapeGrid::ALL;
            let mut sup = HcpShapeGrid::ALL;
            for child in children {
                let b = bounds(child);
                sub = sub.and(&b.sub);
                sup = sup.and(&b.sup);
            }
            GridBounds { sub, sup }
        }
        HandConstraint::Not(inner) => {
            let b = bounds(inner);
            GridBounds {
                sub: b.sup.not(),
                sup: b.sub.not(),
            }
        }
        HandConstraint::Custom(_) => GridBounds {
            sub: HcpShapeGrid::EMPTY,
            sup: HcpShapeGrid::ALL,
        },
    }
}

/// `true` when every atom of `c` is literal-free (shapes and HCP only) and `c` has no `Custom`
/// predicate: exactly the constraints [`HcpShapeGrid::of_exact`] represents (without building
/// the grid).
pub fn is_literal_free(c: &HandConstraint) -> bool {
    match c {
        HandConstraint::Atom(a) => a.cards.is_empty() && a.eval.is_empty(),
        HandConstraint::Or(children) | HandConstraint::And(children) => {
            children.iter().all(is_literal_free)
        }
        HandConstraint::Not(inner) => is_literal_free(inner),
        HandConstraint::Custom(_) => false,
    }
}

/// An over-covering flat form of `branch ∧ ¬minus` for a proposal (the "proposal form" of the
/// natural exclusion, docs/design/07-bidding.md §4.1): `branch` is taken by its superset bound,
/// `minus` is the region to remove (typically the union of the *subset* bounds of the
/// higher-ranked candidates, so the removal never takes away a hand the exact set keeps).
///
/// - A literal-free `branch`: the flat atoms of `grid(branch) ∖ minus` ([`HcpShapeGrid::to_atoms`]
///   with `cap`), exact when at most `cap` runs remain.
/// - An atom with `cards`/`eval` literals: the same atoms, each carrying the atom's literals (so
///   the result is `branch ∧ ¬minus` exactly when at most `cap` runs remain).
/// - Anything else (literals under `And`/`Or`/`Not`, or `Custom`): `And([branch, flat])` where
///   `flat` is the atoms of `sup(branch) ∖ minus`; `branch` is returned unchanged when `minus`
///   removes nothing.
///
/// Always a superset of `branch ∧ ¬minus`; `Or([])` (no hand) when no feasible cell is left.
pub fn subtract_grid(branch: &HandConstraint, minus: &HcpShapeGrid, cap: usize) -> HandConstraint {
    let (region, template, conjoin) = match branch {
        HandConstraint::Atom(a) => (
            HcpShapeGrid::of_atom_box(a),
            Atom {
                shapes: ShapeSet::ALL,
                hcp: 0..=HCP_MAX,
                cards: a.cards.clone(),
                eval: a.eval.clone(),
            },
            false,
        ),
        other => match HcpShapeGrid::of_exact(other) {
            Some(g) => (g, Atom::ANY, false),
            None => (bounds(other).sup, Atom::ANY, true),
        },
    };
    let rest = region.diff(minus);
    if rest.is_empty_hands() {
        return HandConstraint::Or(Vec::new());
    }
    if conjoin && rest == region {
        return branch.clone();
    }
    let flat = rest.to_constraint(&template, cap);
    if conjoin {
        HandConstraint::And(vec![branch.clone(), flat])
    } else {
        flat
    }
}

#[cfg(test)]
mod tests {
    use bridge_core::{Holding, Suit};

    use super::*;
    use crate::CardRequirement;

    fn atom(shapes: ShapeSet, lo: u8, hi: u8) -> HandConstraint {
        HandConstraint::Atom(Atom {
            shapes,
            hcp: lo..=hi,
            cards: Vec::new(),
            eval: Vec::new(),
        })
    }

    #[test]
    fn boolean_ops_are_exact_on_boxes() {
        let a = HcpShapeGrid::from_box(ShapeSet::BALANCED, 15..=17);
        let b = HcpShapeGrid::from_box(ShapeSet::ALL, 12..=21);
        assert_eq!(a.and(&b), a);
        assert_eq!(a.or(&b), b);
        let d = b.diff(&a);
        assert!(!d.intersects(&a));
        assert_eq!(d.or(&a), b);
        assert!(a.is_subset(&b));
        assert!(!b.is_subset(&a));
        assert!(b.not().and(&b).is_empty());
        assert_eq!(b.not().or(&b), HcpShapeGrid::ALL);
        // 12-14 all, 15-17 unbalanced, 18-21 all.
        assert_eq!(d.runs().len(), 3);
        assert_eq!(d.hull(), Some((ShapeSet::ALL, 12..=21)));
    }

    #[test]
    fn bounds_are_exact_for_literal_free_constraints() {
        let c = atom(ShapeSet::BALANCED, 15, 17)
            .or(atom(ShapeSet::ALL, 20, 21))
            .and(atom(ShapeSet::ALL, 16, 37).not());
        let b = bounds(&c);
        assert!(b.is_exact());
        assert_eq!(Some(b.sub), HcpShapeGrid::of_exact(&c));
        assert_eq!(b.sub, HcpShapeGrid::from_box(ShapeSet::BALANCED, 15..=15));
    }

    #[test]
    fn bounds_of_a_literal_atom_bracket_it_and_not_swaps_them() {
        let with_ace = HandConstraint::Atom(Atom {
            shapes: ShapeSet::ALL,
            hcp: 10..=20,
            cards: vec![CardRequirement::in_suit(
                Suit::Spades,
                Holding::EMPTY.with(bridge_core::Rank::Ace),
                1..=1,
            )],
            eval: Vec::new(),
        });
        let b = bounds(&with_ace);
        assert!(!b.is_exact());
        assert!(b.sub.is_empty());
        assert_eq!(b.sup, HcpShapeGrid::from_box(ShapeSet::ALL, 10..=20));
        let n = bounds(&with_ace.clone().not());
        assert_eq!(n.sub, b.sup.not());
        assert_eq!(n.sup, HcpShapeGrid::ALL);
        assert_eq!(HcpShapeGrid::of_exact(&with_ace), None);
    }

    #[test]
    fn to_atoms_is_exact_under_the_cap_and_a_superset_above_it() {
        let g = HcpShapeGrid::from_box(ShapeSet::ALL, 12..=21)
            .diff(&HcpShapeGrid::from_box(ShapeSet::BALANCED, 15..=17));
        let exact = g.to_constraint(&Atom::ANY, 8);
        assert_eq!(HcpShapeGrid::of_exact(&exact), Some(g));
        let capped = g.to_constraint(&Atom::ANY, 1);
        let capped_grid = HcpShapeGrid::of_exact(&capped).unwrap();
        assert!(g.is_subset(&capped_grid));
        assert_eq!(capped_grid, HcpShapeGrid::from_box(ShapeSet::ALL, 12..=21));
        assert!(matches!(
            HcpShapeGrid::EMPTY.to_constraint(&Atom::ANY, 4),
            HandConstraint::Or(v) if v.is_empty()
        ));
    }

    /// A deterministic pseudo-random 13-card hand (splitmix64 Fisher-Yates).
    fn hand_from_seed(seed: &mut u64) -> Hand {
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
            hand = hand.with(bridge_core::Card::from_index(cards[i]).expect("index < 52"));
        }
        hand
    }

    #[test]
    fn grid_membership_matches_satisfies_on_random_hands() {
        let literal_free = atom(ShapeSet::BALANCED, 12, 14)
            .or(atom(ShapeSet::from_suit_len(Suit::Hearts, 5, 13), 8, 16))
            .and(atom(ShapeSet::from_suit_len(Suit::Spades, 4, 13), 0, 37).not());
        let with_literal = literal_free.clone().and(HandConstraint::Atom(Atom {
            shapes: ShapeSet::ALL,
            hcp: 0..=37,
            cards: vec![CardRequirement::in_suit(
                Suit::Hearts,
                Holding::EMPTY
                    .with(bridge_core::Rank::Ace)
                    .with(bridge_core::Rank::King),
                1..=2,
            )],
            eval: Vec::new(),
        }));
        let exact = HcpShapeGrid::of_exact(&literal_free).expect("literal-free");
        let b = bounds(&with_literal);
        let nb = bounds(&with_literal.clone().not());
        let mut seed = 0x5EED;
        for _ in 0..5_000 {
            let hand = hand_from_seed(&mut seed);
            assert_eq!(exact.contains(hand), literal_free.satisfies(hand));
            let sat = with_literal.satisfies(hand);
            assert!(!b.sub.contains(hand) || sat);
            assert!(!sat || b.sup.contains(hand));
            assert!(!nb.sub.contains(hand) || !sat);
            assert!(sat || nb.sup.contains(hand));
            assert!(HcpShapeGrid::feasible().contains(hand));
        }
    }

    #[test]
    fn feasible_cells_exclude_impossible_hcp() {
        let feasible = HcpShapeGrid::feasible();
        // Every shape can hold 10 HCP; a 13-card suit holds exactly 10.
        assert_eq!(feasible.at(10), ShapeSet::ALL);
        assert!(!HcpShapeGrid::from_box(ShapeSet::ALL, 0..=37).is_empty_hands());
        assert!(!feasible.at(37).is_empty());
        assert!(
            HcpShapeGrid::from_box(ShapeSet::from_suit_len(Suit::Spades, 13, 13), 11..=37)
                .is_empty_hands()
        );
    }
}
