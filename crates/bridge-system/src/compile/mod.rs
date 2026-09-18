//! Expansion of the AST into concrete nodes and the trie, and compilation of descriptions.
//!
//! Expansion is a depth-first walk per table (port of the reference `bss.py`):
//!
//! 1. Expand the history row left to right; variables bound there are visible to every row.
//! 2. Among siblings, exact rows come first, then pattern rows; within each group the first
//!    definition wins.
//! 3. Generate the candidate calls of a row (`Strains`: C, D, H, S(, N) order; `Var`: the domain
//!    filtered by "not yet bid by either side", sufficiency and `X < Y < Z`, binding the
//!    variable for the subtree; `Step`: last bid plus `n`; `Class`: a wildcard edge).
//! 4. For each candidate: check legality (`IllegalCall` drops the subtree), insert an implicit
//!    opponents' pass when two consecutive calls are on the same side, substitute variables in
//!    the description, compile the description in the context of the concrete path, create the
//!    node, insert it into the trie (duplicates: first wins, `DuplicatePath`).
//! 5. Recurse into the children; unbind on return.

pub mod desc;

use crate::{Lint, SystemIR, lexer::SourceLoader};

/// Compiler options.
#[derive(Clone, Debug)]
pub struct CompileOptions {
    /// Sample count for the coverage lints (0 disables them).
    pub coverage_samples: u32,
    /// Fail (instead of warn) when a DNF expansion exceeds its cap.
    pub strict_dnf: bool,
    /// Maximum number of nodes before expansion is aborted with a lint.
    pub max_nodes: usize,
}

impl Default for CompileOptions {
    fn default() -> CompileOptions {
        CompileOptions {
            coverage_samples: 10_000,
            strict_dnf: false,
            max_nodes: 50_000,
        }
    }
}

/// Compiles a BML system from source. Never fails as a whole: problems are returned as lints
/// (and also stored in `SystemIR::lints`).
pub fn compile(
    root_path: &str,
    source: &str,
    loader: &dyn SourceLoader,
    opts: &CompileOptions,
) -> (SystemIR, Vec<Lint>) {
    todo!("phase 3")
}
