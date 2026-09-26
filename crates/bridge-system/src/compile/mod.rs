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

mod expand;
mod meta;

use crate::{Lint, SystemIR, ast::Block, lexer::SourceLoader};

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
    // `Instant::now()` panics at runtime on `wasm32-unknown-unknown` ("time not implemented on
    // this platform"); this timing is only ever used for the tracing `elapsed_ms` field below, so
    // it is simply skipped there instead of pulling in a wasm-clock dependency.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    let started = Some(std::time::Instant::now());
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    let started: Option<std::time::Instant> = None;

    let loaded = crate::lexer::load(root_path, source, loader);
    let resolved_source: String = loaded.files.iter().map(|(_, t)| t.as_ref()).collect();
    let bml = crate::parser::parse(loaded);
    let mut lints = bml.lints.clone();

    let default_name = root_path
        .rsplit('/')
        .next()
        .unwrap_or(root_path)
        .to_string();
    let mut meta = meta::parse_meta(&bml.blocks, &default_name, &mut lints);
    meta.source_hash = source_hash(resolved_source.as_bytes());

    let tables: Vec<&crate::ast::BidTable> = bml
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::BidTable(t) => Some(t),
            _ => None,
        })
        .collect();
    let expansion = expand::expand_file(&tables, &meta, opts);
    lints.extend(expansion.lints);

    let mut ir = SystemIR {
        meta,
        rows: expansion.rows,
        nodes: expansion.nodes,
        index: expansion.trie,
        lints,
        exclusive_cell: Default::default(),
    };
    crate::lint::run_post_compile_checks(&mut ir, opts);

    let summary = crate::lint::LintSummary::of(&ir.lints);
    tracing::info!(
        target: "bridge_system::compile",
        name = %ir.meta.name,
        rows = ir.rows.len(),
        nodes = ir.nodes.len(),
        lints_error = summary.errors,
        lints_warn = summary.warnings,
        lints_info = summary.infos,
        elapsed_ms = started.map_or(0, |s| s.elapsed().as_millis() as u64),
        "compiled a BML system"
    );

    let lints_out = ir.lints.clone();
    (ir, lints_out)
}

/// blake3 of the resolved source, when the hasher is available (feature `cache`); otherwise the
/// zero hash, documented on [`crate::SystemMeta::source_hash`]'s only writer.
#[cfg(feature = "cache")]
fn source_hash(source: &[u8]) -> [u8; 32] {
    *blake3::hash(source).as_bytes()
}

/// Without the `cache` feature, `blake3` is not a dependency at all (see `Cargo.toml`), so
/// `SystemMeta::source_hash` is left at its all-zero default; enable `cache` for a real hash.
#[cfg(not(feature = "cache"))]
fn source_hash(_source: &[u8]) -> [u8; 32] {
    [0; 32]
}
