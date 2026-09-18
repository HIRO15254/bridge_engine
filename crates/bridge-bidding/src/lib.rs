//! The interpreter and the generator. Both only read a [`SystemIR`]; neither holds judgement.
//!
//! - [`interpret`]: auction → weighted disjunction of constraints per seat.
//! - [`choose_bid`]: hand + auction → call (or `NoCandidate`, which is a result, not an error).
//!
//! The two are inverses of each other, and the bidirectional-consistency property test
//! (`tests/consistency.rs`) checks that a chosen call is always interpreted as satisfied by the
//! hand that chose it. Anything that looks like a bidding *judgement* (is this bid good, should
//! this hand be upgraded, how does the scoring change strategy) belongs in the system definition
//! or the application, never here.
#![forbid(unsafe_code)]
#![warn(missing_docs)]
// Phase 0 skeleton: the public surface is final, bodies are `todo!()`.
#![allow(dead_code, unused_variables)]

mod cache;
mod choose;
mod interpret;
mod policy;
mod replay;

use std::sync::Arc;

pub use bridge_system::{NaturalInference, NodeId, SystemIR};
pub use cache::InterpretCache;
pub use choose::{
    Alternative, BidChoice, ChoiceSource, Chosen, Diagnostic, NoCandidate, Rejected, Tried,
    choose_bid,
};
pub use interpret::{
    CallExplanation, CallInterpretation, Explanation, InterpretOptions, Interpretation,
    ResolutionKind, interpret,
};
pub use policy::{PolicyParams, call_distribution, sequence_log_likelihood};
pub use replay::{Replay, replay};

/// The four seats' systems. Opponents use their own system, so each seat has one.
#[derive(Clone, Debug)]
pub struct Table {
    /// Systems indexed by [`Seat`](bridge_core::Seat).
    pub systems: [Arc<SystemIR>; 4],
    /// Fallback when a sequence is off-system.
    pub natural: Arc<NaturalInference>,
}

impl Table {
    /// Both partnerships play `system`.
    pub fn uniform(system: Arc<SystemIR>, natural: Arc<NaturalInference>) -> Table {
        Table {
            systems: [system.clone(), system.clone(), system.clone(), system],
            natural,
        }
    }
}

/// Scoring form (used by system conditions through [`BidContext`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[allow(missing_docs)]
pub enum Scoring {
    Imp,
    Mp,
    Total,
}

/// What to do when no listed candidate applies but `Pass` is legal.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ImplicitPass {
    /// Report `NoCandidate` (property tests).
    #[default]
    Never,
    /// Synthesise `Pass` with the complement of the siblings' constraints (applications).
    Complement,
}

/// Context of one bidding decision.
#[derive(Clone, Copy, Debug)]
pub struct BidContext<'a> {
    /// Scoring.
    pub scoring: Scoring,
    /// Natural fallback when the prefix is off-system (`None` = `NoCandidate`).
    pub natural: Option<&'a NaturalInference>,
    /// Implicit-pass policy.
    pub implicit_pass: ImplicitPass,
    /// Parameters of the probabilistic policy.
    pub policy: PolicyParams,
}
