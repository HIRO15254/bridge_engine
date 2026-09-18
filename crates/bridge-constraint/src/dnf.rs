//! Disjunctive normal form.

use crate::{Atom, CustomPred, HandConstraint};

/// One DNF term: an exact atom plus the literals the sampler can only reject on.
#[derive(Clone, Debug)]
pub struct DnfTerm {
    /// The exactly-samplable part.
    pub atom: Atom,
    /// Custom predicates and whether each is negated.
    pub custom: Vec<(CustomPred, bool)>,
    /// A sub-constraint that was not expanded because of the term cap; checked by `satisfies`.
    pub residual: Option<HandConstraint>,
}

impl DnfTerm {
    /// Whether `hand` satisfies the whole term (atom, custom literals and residual).
    pub fn satisfies(&self, hand: bridge_core::Hand) -> bool {
        todo!("phase 2")
    }

    /// `true` when the term has neither custom literals nor a residual.
    pub fn is_exact(&self) -> bool {
        self.custom.is_empty() && self.residual.is_none()
    }
}

/// A constraint in disjunctive normal form.
#[derive(Clone, Debug)]
pub struct Dnf {
    /// The terms (a hand satisfies the constraint iff it satisfies at least one term).
    pub terms: Vec<DnfTerm>,
    /// `true` when the term cap forced part of the constraint into `residual`s.
    pub truncated: bool,
}

/// What to do when expanding an `And` of `Or`s would exceed `max_terms`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overflow {
    /// Move the largest children into `residual` (checked by rejection) until the product fits;
    /// emits a `tracing::warn!`.
    Residual,
    /// Fail with [`DnfError::TooLarge`](crate::DnfError::TooLarge) (for CI checks of system definitions).
    Error,
}

/// Options for [`HandConstraint::to_dnf`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DnfOptions {
    /// Maximum number of terms (default 256).
    pub max_terms: usize,
    /// Overflow policy (default `Residual`).
    pub on_overflow: Overflow,
}

impl Default for DnfOptions {
    fn default() -> DnfOptions {
        DnfOptions {
            max_terms: 256,
            on_overflow: Overflow::Residual,
        }
    }
}
