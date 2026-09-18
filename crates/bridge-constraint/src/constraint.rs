//! Boolean combinations of atoms.

use core::ops::RangeInclusive;
use std::sync::Arc;

use bridge_core::{Hand, ShapeSet, Suit};

use crate::{Atom, Dnf, DnfError, DnfOptions, SampleOptions, Sampler};

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
        todo!("phase 2")
    }

    /// `false` when a [`HandConstraint::Custom`] occurs anywhere; sampling then degrades to
    /// rejection and the sampler emits a warning.
    pub fn is_samplable(&self) -> bool {
        todo!("phase 2")
    }

    /// Disjunctive normal form. Done once before sampling; see [`DnfOptions`] for the blow-up cap.
    pub fn to_dnf(&self, opts: &DnfOptions) -> Result<Dnf, DnfError> {
        todo!("phase 2")
    }

    /// Summary: the union of the HCP ranges of the DNF terms.
    pub fn hcp_range(&self) -> RangeInclusive<u8> {
        todo!("phase 2")
    }

    /// Summary: the union of the shape sets of the DNF terms.
    pub fn shapes(&self) -> ShapeSet {
        todo!("phase 2")
    }

    /// Summary: projection of [`HandConstraint::shapes`] onto `suit`.
    pub fn suit_len(&self, suit: Suit) -> RangeInclusive<u8> {
        self.shapes()
            .suit_len(suit)
            .unwrap_or(RangeInclusive::new(1, 0))
    }

    /// `true` when some hand satisfies the constraint (every DNF term is checked with the exact
    /// sampler's `count()`; unsatisfiable is a result, not an error).
    pub fn is_satisfiable(&self) -> bool {
        todo!("phase 2")
    }

    /// `self ∧ other`, flattening nested conjunctions.
    pub fn and(self, other: HandConstraint) -> HandConstraint {
        todo!("phase 2")
    }

    /// `self ∨ other`, flattening nested disjunctions.
    pub fn or(self, other: HandConstraint) -> HandConstraint {
        todo!("phase 2")
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

#[cfg(feature = "serde")]
impl serde::Serialize for HandConstraint {
    /// Serialises the tree; a [`HandConstraint::Custom`] node is an error (it is not data).
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        todo!("phase 3")
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for HandConstraint {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<HandConstraint, D::Error> {
        todo!("phase 3")
    }
}
