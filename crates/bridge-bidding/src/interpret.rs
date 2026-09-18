//! `interpret`: auction → constraints.
//!
//! **Step A (per call).** For call `j` by seat `s`, resolve in `table.systems[s]`. `Exact`
//! yields one alternative per top-level `Or` branch (weights from `branch_weights` or equal);
//! `Partial` first tries `resolve_lenient`, then the node's own constraint; `Natural` asks the
//! natural engine. Every alternative is scaled by `1 − ε` and a defensive branch `(ANY, ε,
//! Fallback)` is appended, with `ε` depending on the resolution kind. This is how lower
//! confidence is represented: more mass on the unconstrained alternative, never an ad-hoc
//! loosening of the constraint; the sampler's importance weights correct the mixture afterwards.
//!
//! **Step B (per seat).** The alternatives of a seat's calls are combined by cross product
//! (`and`, unsatisfiable combinations dropped, deduplicated by node/kind, truncated to `K` by
//! weight, renormalised). Each call contributes only its own node's constraint; calls before
//! the divergence point keep their `Exact` confidence, which is the operational meaning of
//! "weaken later constraints, not earlier ones".

use bridge_constraint::HandConstraint;
use bridge_core::{Auction, Call, Hand, Seat};

use crate::{NodeId, Table};

/// How a call was resolved. Ordered from most to least confident.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ResolutionKind {
    /// The sequence is in the system.
    Exact,
    /// A prefix is in the system.
    Partial {
        /// Matched prefix length.
        matched_depth: usize,
    },
    /// Natural inference.
    Natural,
    /// The defensive `ANY` branch.
    Fallback,
}

/// Explanation of one call.
#[derive(Clone, Debug)]
pub struct CallExplanation {
    /// Index of the call in the auction.
    pub call_index: usize,
    /// The call.
    pub call: Call,
    /// The node, if any.
    pub node: Option<NodeId>,
    /// Resolution kind.
    pub kind: ResolutionKind,
    /// The node's description or the natural rule text; empty for `Fallback`.
    pub text: String,
}

/// Explanation of one alternative for one seat.
#[derive(Clone, Debug)]
pub struct Explanation {
    /// The parts joined with ` / `.
    pub text: String,
    /// The node of the seat's most recent call.
    pub node: Option<NodeId>,
    /// The least confident kind among the parts.
    pub resolution: ResolutionKind,
    /// One part per call of this seat.
    pub parts: Vec<CallExplanation>,
}

/// The weighted disjunction for one call, before combination.
#[derive(Clone, Debug)]
pub struct CallInterpretation {
    /// Index of the call.
    pub call_index: usize,
    /// Its seat.
    pub seat: Seat,
    /// The call.
    pub call: Call,
    /// Resolution kind.
    pub kind: ResolutionKind,
    /// Alternatives; weights sum to 1.
    pub alternatives: Vec<(HandConstraint, f32, CallExplanation)>,
}

/// The result of [`interpret`].
#[derive(Clone, Debug)]
pub struct Interpretation {
    /// Per seat: weighted alternatives summing to 1.
    pub seats: [Vec<(HandConstraint, f32, Explanation)>; 4],
    /// Per call, before combination (for display and likelihoods).
    pub per_call: Vec<CallInterpretation>,
    /// The first call index that was not resolved `Exact`, if any.
    pub divergence: Option<usize>,
}

impl Interpretation {
    /// Whether `hand` satisfies at least one non-`Fallback` alternative of `seat` (the strict
    /// check used by the consistency test).
    pub fn satisfied_by(&self, seat: Seat, hand: Hand) -> bool {
        todo!("phase 3")
    }

    /// `Σ w_i · [C_i ∋ hand]`: the set-membership mass of `hand` under the mixture. This is not
    /// the bidding-policy likelihood (see `sequence_log_likelihood`).
    pub fn likelihood(&self, seat: Seat, hand: Hand) -> f32 {
        todo!("phase 3")
    }
}

/// Options for [`interpret`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct InterpretOptions {
    /// Maximum alternatives kept per seat (default 8).
    pub max_alternatives: usize,
    /// Fallback mass for `Exact` resolutions (default 0.02).
    pub eps_exact: f32,
    /// Fallback mass for `Partial` resolutions (default 0.15).
    pub eps_partial: f32,
    /// Fallback mass for `Natural` resolutions (default 0.30).
    pub eps_natural: f32,
    /// No fallback branches at all (property tests).
    pub strict: bool,
    /// Weight multiplier per opponents'-call substitution in `resolve_lenient` (default 0.5).
    pub lenient_decay: f32,
}

impl Default for InterpretOptions {
    fn default() -> InterpretOptions {
        InterpretOptions {
            max_alternatives: 8,
            eps_exact: 0.02,
            eps_partial: 0.15,
            eps_natural: 0.30,
            strict: false,
            lenient_decay: 0.5,
        }
    }
}

/// Interprets `auction` under the four systems of `table`.
pub fn interpret(table: &Table, auction: &Auction, opts: &InterpretOptions) -> Interpretation {
    todo!("phase 3")
}
