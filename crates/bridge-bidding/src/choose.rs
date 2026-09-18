//! `choose_bid`: hand + auction → call.
//!
//! 1. Candidates: the system's continuations for the prefix, or the natural engine's when the
//!    prefix is off-system and `ctx.natural` is set.
//! 2. Illegal candidates (per `Auction::is_legal`) become `Tried { Illegal }` plus a
//!    `Diagnostic::IllegalSystemCall` (a system-definition lint, never a panic); candidates
//!    whose constraint the hand fails become `Tried { Unsatisfied }`.
//! 3. With `ImplicitPass::Complement`, a `Pass` with the complement of the siblings' constraints
//!    is synthesised when none is listed (the interpreter uses the same complement).
//! 4. Sort by priority descending, ties by `SystemMeta::tie_break`.
//! 5. Empty → `NoCandidate`; otherwise `Chosen` with every survivor in `alternatives`.

use bridge_core::{Auction, Call, Hand};

use crate::{BidContext, NodeId, SystemIR};

/// The outcome of a bidding decision. `NoCandidate` is information about the system's coverage,
/// not an error.
#[derive(Clone, Debug)]
pub enum BidChoice {
    /// A call was chosen.
    Chosen(Chosen),
    /// No listed call applies.
    NoCandidate(NoCandidate),
}

impl BidChoice {
    /// The chosen call, if any.
    pub fn call(&self) -> Option<Call> {
        match self {
            BidChoice::Chosen(c) => Some(c.call),
            BidChoice::NoCandidate(_) => None,
        }
    }

    /// `true` for [`BidChoice::Chosen`].
    pub fn is_chosen(&self) -> bool {
        matches!(self, BidChoice::Chosen(_))
    }
}

/// Where a chosen call came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(missing_docs)]
pub enum ChoiceSource {
    System,
    Natural,
    ImplicitPass,
}

/// A chosen call.
#[derive(Clone, Debug)]
pub struct Chosen {
    /// The call.
    pub call: Call,
    /// Its node (`None` for natural / implicit pass).
    pub node: Option<NodeId>,
    /// Source.
    pub source: ChoiceSource,
    /// Explanation text.
    pub explanation: String,
    /// Every satisfying legal candidate, sorted; the chosen one first.
    pub alternatives: Vec<Alternative>,
    /// System-definition problems noticed on the way.
    pub diagnostics: Vec<Diagnostic>,
}

/// A candidate that survived filtering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Alternative {
    /// The call.
    pub call: Call,
    /// Its node.
    pub node: Option<NodeId>,
    /// Its priority.
    pub priority: i16,
}

/// No candidate applied.
#[derive(Clone, Debug)]
pub struct NoCandidate {
    /// Every candidate and why it was rejected.
    pub tried: Vec<Tried>,
    /// System-definition problems noticed on the way.
    pub diagnostics: Vec<Diagnostic>,
}

/// A rejected candidate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tried {
    /// The node.
    pub node: NodeId,
    /// Its call.
    pub call: Call,
    /// Why it was rejected.
    pub reason: Rejected,
}

/// Rejection reasons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(missing_docs)]
pub enum Rejected {
    Unsatisfied,
    Illegal,
    NotApplicable,
}

/// A problem in the system definition found while bidding.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Diagnostic {
    /// The system lists a call that is illegal at this point.
    IllegalSystemCall {
        /// The node.
        node: NodeId,
        /// The call.
        call: Call,
    },
    /// Two nodes offer the same call at the same point.
    DuplicateCandidate {
        /// First node.
        node_a: NodeId,
        /// Second node.
        node_b: NodeId,
        /// The call.
        call: Call,
    },
    /// A node's constraint is unsatisfiable.
    UnsatisfiableNode {
        /// The node.
        node: NodeId,
    },
}

/// Chooses a call for `hand` after `auction` under `system`.
pub fn choose_bid(
    system: &SystemIR,
    hand: Hand,
    auction: &Auction,
    ctx: &BidContext<'_>,
) -> BidChoice {
    todo!("phase 3")
}
