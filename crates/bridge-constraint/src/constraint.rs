//! Boolean combinations of atoms.

use core::ops::RangeInclusive;
use std::sync::Arc;

use bridge_core::{Hand, ShapeSet, Suit};

use crate::{Atom, Dnf, DnfError, DnfOptions, DnfTerm, Overflow, SampleOptions, Sampler};

/// A named predicate that cannot be sampled directly.
///
/// The name appears in `tracing` output so that slow (rejection-based) sampling can be traced
/// back to the constraint that caused it. The bidding-system compiler never produces one.
#[derive(Clone)]
pub struct CustomPred {
    /// Diagnostic name.
    pub name: String,
    /// The predicate.
    pub f: Arc<dyn Fn(Hand) -> bool + Send + Sync>,
}

impl core::fmt::Debug for CustomPred {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Custom({})", self.name)
    }
}

/// A constraint on a 13-card hand.
#[derive(Clone, Debug)]
pub enum HandConstraint {
    /// A conjunction of literals.
    Atom(Atom),
    /// Disjunction.
    Or(Vec<HandConstraint>),
    /// Conjunction.
    And(Vec<HandConstraint>),
    /// Negation.
    Not(Box<HandConstraint>),
    /// An opaque predicate; makes the constraint non-samplable (rejection only).
    Custom(CustomPred),
}

impl HandConstraint {
    /// The unconstrained constraint.
    pub const ANY: HandConstraint = HandConstraint::Atom(Atom::ANY);

    /// Evaluates the tree directly on `hand` (no normalisation needed).
    pub fn satisfies(&self, hand: Hand) -> bool {
        match self {
            HandConstraint::Atom(atom) => atom.satisfies(hand),
            HandConstraint::Or(children) => children.iter().any(|c| c.satisfies(hand)),
            HandConstraint::And(children) => children.iter().all(|c| c.satisfies(hand)),
            HandConstraint::Not(inner) => !inner.satisfies(hand),
            HandConstraint::Custom(pred) => (pred.f)(hand),
        }
    }

    /// `false` when a [`HandConstraint::Custom`] occurs anywhere; sampling then degrades to
    /// rejection and the sampler emits a warning.
    pub fn is_samplable(&self) -> bool {
        match self {
            HandConstraint::Atom(_) => true,
            HandConstraint::Or(children) | HandConstraint::And(children) => {
                children.iter().all(HandConstraint::is_samplable)
            }
            HandConstraint::Not(inner) => inner.is_samplable(),
            HandConstraint::Custom(_) => false,
        }
    }

    /// Disjunctive normal form. Done once before sampling; see [`DnfOptions`] for the blow-up cap.
    pub fn to_dnf(&self, opts: &DnfOptions) -> Result<Dnf, DnfError> {
        let nnf = Nnf::from_constraint(self, false);
        let (terms, truncated) = nnf.expand(opts)?;
        let mut terms: Vec<DnfTerm> = terms
            .into_iter()
            .map(|mut term| {
                term.atom.normalize();
                term
            })
            .filter(|term| !term.atom.is_trivially_unsat())
            .collect();
        dedup_exact_terms(&mut terms);
        Ok(Dnf { terms, truncated })
    }

    /// Summary: the union of the HCP ranges of the DNF terms.
    ///
    /// `Or` sums the branch ranges (min of the starts to max of the ends), `And` intersects them,
    /// `Not` and `Custom` fall back to the unconstrained range. An empty `Or` (unsatisfiable) is
    /// the empty range `1..=0`, matching [`Atom::suit_len`]'s convention.
    pub fn hcp_range(&self) -> RangeInclusive<u8> {
        match self {
            HandConstraint::Atom(atom) => atom.hcp_range(),
            HandConstraint::Or(children) => {
                let mut ranges = children.iter().map(HandConstraint::hcp_range);
                match ranges.next() {
                    None => RangeInclusive::new(1, 0),
                    Some(first) => ranges.fold(first, |acc, r| {
                        (*acc.start().min(r.start()))..=(*acc.end().max(r.end()))
                    }),
                }
            }
            HandConstraint::And(children) => children.iter().fold(0..=37, |acc, c| {
                let r = c.hcp_range();
                (*acc.start().max(r.start()))..=(*acc.end().min(r.end()))
            }),
            HandConstraint::Not(_) | HandConstraint::Custom(_) => 0..=37,
        }
    }

    /// Summary: the union of the shape sets of the DNF terms.
    ///
    /// Same shape as [`HandConstraint::hcp_range`]: `Or` unions, `And` intersects, `Not`/`Custom`
    /// fall back to [`ShapeSet::ALL`].
    pub fn shapes(&self) -> ShapeSet {
        match self {
            HandConstraint::Atom(atom) => atom.shapes,
            HandConstraint::Or(children) => children
                .iter()
                .fold(ShapeSet::EMPTY, |acc, c| acc.union(c.shapes())),
            HandConstraint::And(children) => children
                .iter()
                .fold(ShapeSet::ALL, |acc, c| acc.intersect(c.shapes())),
            HandConstraint::Not(_) | HandConstraint::Custom(_) => ShapeSet::ALL,
        }
    }

    /// Summary: projection of [`HandConstraint::shapes`] onto `suit`.
    pub fn suit_len(&self, suit: Suit) -> RangeInclusive<u8> {
        self.shapes()
            .suit_len(suit)
            .unwrap_or(RangeInclusive::new(1, 0))
    }

    /// `true` when some hand satisfies the constraint.
    ///
    /// Provisional (2.2/2.3): `to_dnf` already drops every trivially-unsatisfiable term (§5,
    /// stages 1-4), so this is `!dnf.terms.is_empty()`.
    /// TODO(2.4): switch to `Sampler::prepare(self, Hand::FULL, Hand::EMPTY, &opts).count() > 0`,
    /// which is also exact for the interactions `is_trivially_unsat` cannot see (stage 5).
    pub fn is_satisfiable(&self) -> bool {
        let dnf = self
            .to_dnf(&DnfOptions::default())
            .expect("DnfOptions::default uses Overflow::Residual, which never errors");
        !dnf.terms.is_empty()
    }

    /// `self ∧ other`, flattening nested conjunctions.
    pub fn and(self, other: HandConstraint) -> HandConstraint {
        match (self, other) {
            (HandConstraint::And(mut a), HandConstraint::And(b)) => {
                a.extend(b);
                HandConstraint::And(a)
            }
            (HandConstraint::And(mut a), other) => {
                a.push(other);
                HandConstraint::And(a)
            }
            (this, HandConstraint::And(mut b)) => {
                b.insert(0, this);
                HandConstraint::And(b)
            }
            (a, b) => HandConstraint::And(vec![a, b]),
        }
    }

    /// `self ∨ other`, flattening nested disjunctions.
    pub fn or(self, other: HandConstraint) -> HandConstraint {
        match (self, other) {
            (HandConstraint::Or(mut a), HandConstraint::Or(b)) => {
                a.extend(b);
                HandConstraint::Or(a)
            }
            (HandConstraint::Or(mut a), other) => {
                a.push(other);
                HandConstraint::Or(a)
            }
            (this, HandConstraint::Or(mut b)) => {
                b.insert(0, this);
                HandConstraint::Or(b)
            }
            (a, b) => HandConstraint::Or(vec![a, b]),
        }
    }

    /// `¬self`.
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> HandConstraint {
        HandConstraint::Not(Box::new(self))
    }

    /// Convenience sampler (spec signature): one hand from the full deck minus `excluded`.
    ///
    /// This prepares a [`Sampler`] on every call and is therefore O(prepare); repeated sampling
    /// must go through [`Sampler::prepare`] once and [`Sampler::sample`] many times.
    pub fn sample<R: rand_core::Rng + ?Sized>(&self, rng: &mut R, excluded: Hand) -> Option<Hand> {
        let sampler = Sampler::prepare(
            self,
            excluded.complement(),
            Hand::EMPTY,
            &SampleOptions::default(),
        )
        .ok()?;
        sampler.sample(rng).map(|s| s.hand)
    }
}

/// Externally-tagged mirror of [`HandConstraint`] without [`HandConstraint::Custom`], which is
/// not data and cannot round-trip.
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
enum HandConstraintRepr {
    Atom(Atom),
    Or(Vec<HandConstraintRepr>),
    And(Vec<HandConstraintRepr>),
    Not(Box<HandConstraintRepr>),
}

#[cfg(feature = "serde")]
fn to_repr(c: &HandConstraint) -> Result<HandConstraintRepr, &'static str> {
    Ok(match c {
        HandConstraint::Atom(atom) => HandConstraintRepr::Atom(atom.clone()),
        HandConstraint::Or(children) => {
            HandConstraintRepr::Or(children.iter().map(to_repr).collect::<Result<_, _>>()?)
        }
        HandConstraint::And(children) => {
            HandConstraintRepr::And(children.iter().map(to_repr).collect::<Result<_, _>>()?)
        }
        HandConstraint::Not(inner) => HandConstraintRepr::Not(Box::new(to_repr(inner)?)),
        HandConstraint::Custom(_) => {
            return Err("HandConstraint::Custom cannot be serialized (it is not data)");
        }
    })
}

#[cfg(feature = "serde")]
fn from_repr(repr: HandConstraintRepr) -> HandConstraint {
    match repr {
        HandConstraintRepr::Atom(atom) => HandConstraint::Atom(atom),
        HandConstraintRepr::Or(children) => {
            HandConstraint::Or(children.into_iter().map(from_repr).collect())
        }
        HandConstraintRepr::And(children) => {
            HandConstraint::And(children.into_iter().map(from_repr).collect())
        }
        HandConstraintRepr::Not(inner) => HandConstraint::Not(Box::new(from_repr(*inner))),
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for HandConstraint {
    /// Serialises the tree; a [`HandConstraint::Custom`] node is an error (it is not data).
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let repr = to_repr(self).map_err(serde::ser::Error::custom)?;
        repr.serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for HandConstraint {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<HandConstraint, D::Error> {
        HandConstraintRepr::deserialize(deserializer).map(from_repr)
    }
}

/// A [`HandConstraint`] with every `Not` pushed down to the leaves (negation normal form): the
/// tree has no `Not` nodes, atoms are always positive (an atom's own negation was already expanded
/// into an `Or` of positive atoms by [`Atom::negate`]), and a negated [`CustomPred`] carries a
/// `true` flag instead.
enum Nnf {
    Atom(Atom),
    Or(Vec<Nnf>),
    And(Vec<Nnf>),
    Custom(CustomPred, bool),
}

impl Nnf {
    /// Converts `c` to negation normal form; `negated` is `true` while an odd number of `Not`
    /// ancestors have been crossed.
    fn from_constraint(c: &HandConstraint, negated: bool) -> Nnf {
        match c {
            HandConstraint::Atom(atom) => {
                if negated {
                    Nnf::Or(atom.negate().into_iter().map(Nnf::Atom).collect())
                } else {
                    Nnf::Atom(atom.clone())
                }
            }
            HandConstraint::Or(children) => {
                let children = children
                    .iter()
                    .map(|c| Nnf::from_constraint(c, negated))
                    .collect();
                if negated {
                    Nnf::And(children)
                } else {
                    Nnf::Or(children)
                }
            }
            HandConstraint::And(children) => {
                let children = children
                    .iter()
                    .map(|c| Nnf::from_constraint(c, negated))
                    .collect();
                if negated {
                    Nnf::Or(children)
                } else {
                    Nnf::And(children)
                }
            }
            HandConstraint::Not(inner) => Nnf::from_constraint(inner, !negated),
            HandConstraint::Custom(pred) => Nnf::Custom(pred.clone(), negated),
        }
    }

    /// Converts back to a [`HandConstraint`], for storing an unexpanded subtree in
    /// [`DnfTerm::residual`].
    fn to_constraint(&self) -> HandConstraint {
        match self {
            Nnf::Atom(atom) => HandConstraint::Atom(atom.clone()),
            Nnf::Or(children) => {
                HandConstraint::Or(children.iter().map(Nnf::to_constraint).collect())
            }
            Nnf::And(children) => {
                HandConstraint::And(children.iter().map(Nnf::to_constraint).collect())
            }
            Nnf::Custom(pred, false) => HandConstraint::Custom(pred.clone()),
            Nnf::Custom(pred, true) => {
                HandConstraint::Not(Box::new(HandConstraint::Custom(pred.clone())))
            }
        }
    }

    /// Expands this node into DNF terms, applying [`DnfOptions`]'s cap to every `And` node.
    fn expand(&self, opts: &DnfOptions) -> Result<(Vec<DnfTerm>, bool), DnfError> {
        match self {
            Nnf::Atom(atom) => Ok((
                vec![DnfTerm {
                    atom: atom.clone(),
                    custom: Vec::new(),
                    residual: None,
                }],
                false,
            )),
            Nnf::Custom(pred, negated) => Ok((
                vec![DnfTerm {
                    atom: Atom::ANY,
                    custom: vec![(pred.clone(), *negated)],
                    residual: None,
                }],
                false,
            )),
            Nnf::Or(children) => {
                let mut terms = Vec::new();
                let mut truncated = false;
                for child in children {
                    let (child_terms, child_truncated) = child.expand(opts)?;
                    terms.extend(child_terms);
                    truncated |= child_truncated;
                }
                Ok((terms, truncated))
            }
            Nnf::And(children) => expand_and(children, opts),
        }
    }
}

/// `Nnf::And`'s share of [`Nnf::expand`]: expands every child, then estimates the product of
/// their term counts (§4.3 step 3). Within the cap, the terms are the literal cartesian product
/// (pairwise [`Atom::intersect`]ed). Over the cap, [`Overflow::Error`] fails outright; otherwise
/// the largest children are moved into a shared [`DnfTerm::residual`] (checked by rejection)
/// until the remaining product fits, and a `tracing::warn!` records the truncation.
fn expand_and(children: &[Nnf], opts: &DnfOptions) -> Result<(Vec<DnfTerm>, bool), DnfError> {
    let mut truncated = false;
    let mut expanded: Vec<(Vec<DnfTerm>, &Nnf)> = Vec::with_capacity(children.len());
    for child in children {
        let (terms, child_truncated) = child.expand(opts)?;
        truncated |= child_truncated;
        if terms.is_empty() {
            // This conjunct is unsatisfiable, so the whole conjunction is.
            return Ok((Vec::new(), truncated));
        }
        expanded.push((terms, child));
    }

    let sizes: Vec<usize> = expanded.iter().map(|(terms, _)| terms.len()).collect();
    let estimate = saturating_product(sizes.iter().copied());
    if estimate > opts.max_terms {
        if opts.on_overflow == Overflow::Error {
            return Err(DnfError::TooLarge {
                estimated: estimate,
                max_terms: opts.max_terms,
            });
        }
        truncated = true;
    }

    // Move the largest children into `residual` until the remaining product fits (or nothing is
    // left to move).
    let mut kept: Vec<usize> = (0..expanded.len()).collect();
    let mut residual: Vec<usize> = Vec::new();
    while !kept.is_empty() && saturating_product(kept.iter().map(|&i| sizes[i])) > opts.max_terms {
        let largest = *kept
            .iter()
            .max_by_key(|&&i| sizes[i])
            .expect("kept is non-empty");
        kept.retain(|&i| i != largest);
        residual.push(largest);
    }

    if !residual.is_empty() {
        tracing::warn!(
            estimated = estimate,
            max_terms = opts.max_terms,
            moved = residual.len(),
            "DNF And node exceeded the term cap; moving children to a rejection-checked residual"
        );
    }

    let kept_lists: Vec<&Vec<DnfTerm>> = kept.iter().map(|&i| &expanded[i].0).collect();
    let mut terms = cartesian(&kept_lists);

    if !residual.is_empty() {
        let extra = residual
            .iter()
            .map(|&i| expanded[i].1.to_constraint())
            .reduce(HandConstraint::and)
            .expect("residual is non-empty");
        for term in &mut terms {
            term.residual = Some(match term.residual.take() {
                Some(existing) => existing.and(extra.clone()),
                None => extra.clone(),
            });
        }
    }

    Ok((terms, truncated))
}

/// The product of `sizes`, saturating at `usize::MAX` instead of overflowing (only the comparison
/// against `max_terms` matters, not the exact value once it is already far past the cap).
fn saturating_product(sizes: impl Iterator<Item = usize>) -> usize {
    sizes.fold(1usize, |acc, n| acc.saturating_mul(n))
}

/// Pairwise [`Atom::intersect`]s two terms' atoms, concatenates their custom literals, and joins
/// their residuals with `and`.
fn combine_terms(a: &DnfTerm, b: &DnfTerm) -> DnfTerm {
    let atom = a.atom.intersect(&b.atom);
    let mut custom = a.custom.clone();
    custom.extend(b.custom.iter().cloned());
    let residual = match (a.residual.clone(), b.residual.clone()) {
        (None, None) => None,
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (Some(x), Some(y)) => Some(x.and(y)),
    };
    DnfTerm {
        atom,
        custom,
        residual,
    }
}

/// The cartesian product of the term lists, combined pairwise with [`combine_terms`]. The product
/// of zero lists is the single unconstrained term (the identity for `And`).
fn cartesian(lists: &[&Vec<DnfTerm>]) -> Vec<DnfTerm> {
    let mut acc = vec![DnfTerm {
        atom: Atom::ANY,
        custom: Vec::new(),
        residual: None,
    }];
    for list in lists {
        let mut next = Vec::with_capacity(acc.len() * list.len().max(1));
        for a in &acc {
            for b in list.iter() {
                next.push(combine_terms(a, b));
            }
        }
        acc = next;
    }
    acc
}

/// Removes exact-duplicate terms (identical, custom- and residual-free atoms). Terms carrying a
/// custom literal or a residual are never compared (`CustomPred` and `HandConstraint` have no
/// `PartialEq`) and are always kept.
fn dedup_exact_terms(terms: &mut Vec<DnfTerm>) {
    let mut seen: Vec<Atom> = Vec::new();
    terms.retain(|term| {
        if term.is_exact() {
            if seen.contains(&term.atom) {
                return false;
            }
            seen.push(term.atom.clone());
        }
        true
    });
}
