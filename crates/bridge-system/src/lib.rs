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
// Phase 0 skeleton: the public surface is final, bodies are `todo!()`.
#![allow(dead_code, unused_variables)]

pub mod ast;
#[cfg(feature = "cache")]
pub mod cache;
pub mod compile;
pub mod lexer;
pub mod lint;
pub mod natural;
pub mod parser;
pub mod pattern;
pub mod trie;

mod ir;

pub use compile::{CompileOptions, compile};
pub use ir::{
    Alertability, BalancedDef, ConventionDefaults, Forcing, Node, NodeFlags, NodeId, Recognition,
    Row, RowId, StrengthVocab, SystemIR, SystemMeta, TieBreak,
};
pub use lint::{Lint, LintCode, Severity};
pub use natural::{CallContext, CallKind, Inference, NaturalInference, NaturalParams, Role};
pub use pattern::{Binding, CallPattern, Level, OppClass, Side, SidedPattern, StrainSet, Var};
pub use trie::{AuctionTrie, Lookup, LookupKey, RelVul, Resolution};

/// Compiler version stamped into every [`SystemMeta`].
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Bumped on every breaking change of the serialised IR.
pub const IR_FORMAT: u32 = 1;
