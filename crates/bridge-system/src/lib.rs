//! Bidding-system definitions: from BML source to an executable [`SystemIR`].
//!
//! ```text
//! BML file ──parse──▶ AST ──expand──▶ concrete nodes + AuctionTrie ──compile descriptions──▶ SystemIR
//!                                                                                        │
//!                                                        interpret ◀────── read only ─────┤
//!                                                        choose_bid ◀───── read only ─────┘
//! ```
//!
//! The authoring format is BML (Bridge Bidding Markup Language) as published by gpaulissen/bml;
//! this crate parses it, expands pattern calls (`1M`, `2X`, `1CD`, `1step`) into concrete
//! auction sequences exactly like the reference `bss.py`, compiles the natural-language
//! descriptions into [`HandConstraint`](bridge_constraint::HandConstraint)s, and reports what
//! it could not understand instead of silently dropping it. The IR is immutable after
//! construction and shared through `Arc`; every cache lives in the caller.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ast;
#[cfg(feature = "cache")]
pub mod cache;
pub mod compile;
pub mod exclusive;
pub mod lexer;
pub mod lint;
pub mod natural;
pub mod parser;
pub mod pattern;
pub mod trie;

mod ir;

pub use compile::{CompileOptions, compile};
pub use exclusive::{ExclusiveGroup, ExclusiveIndex, ExclusivePiece};
pub use ir::{
    Alertability, BalancedDef, ConventionDefaults, Forcing, Node, NodeFlags, NodeId, Recognition,
    Row, RowId, StrengthVocab, SystemIR, SystemMeta, TieBreak,
};
pub use lint::{Lint, LintCode, Severity};
pub use natural::{
    CallContext, CallKind, Inference, LevelFloor, NaturalCandidate, NaturalInference,
    NaturalParams, PartnerContext, Role,
};
pub use pattern::{Binding, CallPattern, Level, OppClass, Side, SidedPattern, StrainSet, Var};
pub use trie::{AuctionTrie, Lookup, LookupKey, RelVul, Resolution};

/// Compiler version stamped into every [`SystemMeta`].
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Bumped on every breaking change of the serialised IR.
///
/// 1: phase 3. 2: phase 4 (`NodeFlags::stop`; the trie links system stops to shared detached
/// nodes). 3: `NodeFlags::synthesised`.
pub const IR_FORMAT: u32 = 3;
/// Revision of what [`crate::compile()`] produces for a given source and options, bumped
/// whenever that output changes without a format change (so without an [`IR_FORMAT`] bump):
/// new lints, a different expansion. It is part of the `cache::SystemCache` key (feature
/// `cache`), so an entry written by an older compiler of the same crate version is a miss
/// rather than a silently stale IR.
///
/// 1: phase 3. 2: phase 4 (the exclusive-index lints `ShadowedBranch` and
/// `OverlappingBranches` are stored in `SystemIR::lints`). 3: system stops (`#STOP`, `{stop}`).
/// 4: stops under different `#SEAT`/`#VUL` conditions that meet at one edge share a loop
/// carrying every condition's entries. 5: the synthesised stop pass's description is the row it
/// stands for, `{prio:-100} {stop} any hand`. 6: every node's description is stored without its
/// `{prio:N}` / `{w:X}` / `{stop}` annotations (`Row::description_raw` keeps them).
pub const COMPILE_REVISION: u32 = 6;
