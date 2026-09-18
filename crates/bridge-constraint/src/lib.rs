//! The hand-constraint language and the exact constraint-satisfying sampler.
//!
//! A [`HandConstraint`] is a boolean combination of [`Atom`]s. An atom is a conjunction of
//! literals over the ORIGINAL 13-card hand: a [`ShapeSet`] (the single source of truth for every
//! length condition), an HCP range, card requirements ("♦ has A or K") and evaluation
//! requirements (controls, losers, …). Constraints must be
//!
//! - **composable** (`and` / `or` / `not`),
//! - **decidable** ([`HandConstraint::satisfies`]),
//! - **samplable** ([`Sampler`]: exact, rejection-free for shape + HCP + one additive metric,
//!   with the set size known so that `log_prob` is exact), and
//! - **summarisable** ([`HandConstraint::hcp_range`], [`HandConstraint::shapes`], …).
//!
//! [`HandConstraint::Custom`] is the escape hatch: a closure that can only be rejection-sampled;
//! [`HandConstraint::is_samplable`] reports it so that slow sampling can be traced to its cause.
#![forbid(unsafe_code)]
#![warn(missing_docs)]
// Phase 0 skeleton: the public surface is final, bodies are `todo!()`.
#![allow(dead_code, unused_variables)]

mod atom;
mod constraint;
mod dnf;
mod error;
mod known;
pub mod sampler;

pub use atom::{Atom, CardRequirement, EvalRequirement, Metric};
pub use bridge_core::{Hand, Shape, ShapeClass, ShapeSet};
pub use bridge_eval::{DistMethod, Half, LtcMethod};
pub use constraint::{CustomPred, HandConstraint};
pub use dnf::{Dnf, DnfOptions, DnfTerm, Overflow};
pub use error::{DnfError, KnownCardsError, PrepareError};
pub use known::KnownCards;
pub use sampler::{Sample, SampleOptions, Sampler};
