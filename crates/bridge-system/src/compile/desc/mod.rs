//! Description → constraint compiler.
//!
//! 1. [`normalize`]: strip the alert marker and `{prio:N}` / `{w:X}` annotations, map `!c` to a
//!    suit sentinel, `--` to `-`, collapse whitespace but keep line breaks.
//! 2. [`clause`]: parse into fragments with byte spans; precedence `and` > `or`/`/` > `,`/`;`;
//!    enumerations `a) b)` form an `Or` group; unrecognised spans are kept.
//! 3. [`tokens`]: context-free fragments (HCP, lengths, shapes, balance, cards, metrics) to
//!    partial atoms.
//! 4. [`context`]: context-dependent fragments (`GF INV MIN MAX weak PRE STR S/T QUANT NAT SPL
//!    fit TRF #`) resolved from the parent chain, the binding and `SystemMeta`, with provenance.
//! 5. Assemble into a [`HandConstraint`], compute
//!    [`Recognition`], emit `tracing` events.

pub mod clause;
pub mod context;
pub mod normalize;
pub mod recognition;
pub mod tokens;

use bridge_constraint::HandConstraint;

use crate::{Lint, NodeFlags, Recognition, SystemMeta};

/// The output of compiling one description.
#[derive(Clone, Debug)]
pub struct Compiled {
    /// The constraint (`ANY` for an empty or fully unrecognised description).
    pub constraint: HandConstraint,
    /// Weights of top-level `Or` branches from `{w:X}`.
    pub branch_weights: Option<Vec<f32>>,
    /// `{prio:N}`.
    pub priority: i16,
    /// Derived flags.
    pub flags: NodeFlags,
    /// Recognition statistics.
    pub recognition: Recognition,
    /// Diagnostics.
    pub lints: Vec<Lint>,
}

/// Compiles one substituted description in its context.
pub fn compile_description(
    text: &str,
    ctx: &context::RowContext<'_>,
    meta: &SystemMeta,
) -> Compiled {
    todo!("phase 3")
}
