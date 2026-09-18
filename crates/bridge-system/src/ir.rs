//! The compiled, immutable system representation.

use core::ops::RangeInclusive;
use std::collections::BTreeMap;
use std::sync::Arc;

use bridge_constraint::HandConstraint;
use bridge_core::{Auction, Call, Seat, Strain, Suit};
use bridge_eval::DistMethod;

use crate::{
    Lint,
    ast::{SeatCond, Span, VulCond},
    natural::NaturalParams,
    pattern::{Binding, Side, SidedPattern},
    trie::{AuctionTrie, Lookup, LookupKey},
};

/// Index into [`SystemIR::nodes`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NodeId(pub u32);

/// Index into [`SystemIR::rows`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RowId(pub u32);

/// A compiled bidding system. Immutable; share through `Arc`.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SystemIR {
    /// Metadata and declared conventions.
    pub meta: SystemMeta,
    /// One entry per BML row (authoring provenance).
    pub rows: Vec<Row>,
    /// Concrete expansions; [`NodeId`] indexes this.
    pub nodes: Vec<Node>,
    /// Concrete auction sequence → node.
    pub index: AuctionTrie,
    /// Diagnostics produced at compile time.
    pub lints: Vec<Lint>,
}

impl SystemIR {
    /// The node with the given id.
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    /// The row with the given id.
    pub fn row(&self, id: RowId) -> &Row {
        &self.rows[id.0 as usize]
    }

    /// Resolves the auction from the point of view of `owner` (whose system this is).
    /// Returns `None` for an empty or passed-out auction.
    pub fn resolve(&self, auction: &Auction, owner: Seat) -> Option<Lookup> {
        let key = LookupKey::for_auction(auction, owner)?;
        Some(self.index.resolve(&key))
    }

    /// The candidate continuations for `owner`'s next call: `(call, node)` pairs whose
    /// conditions hold, or `None` when the prefix is off-system.
    pub fn continuations(&self, auction: &Auction, owner: Seat) -> Option<Vec<(Call, NodeId)>> {
        todo!("phase 3")
    }
}

/// One BML row after include and paste expansion.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Row {
    /// Id.
    pub id: RowId,
    /// Source location.
    pub span: Span,
    /// Pattern path including this row's own call.
    pub path: Arc<[SidedPattern]>,
    /// The description as written (before variable substitution).
    pub description_raw: String,
    /// Recognition statistics of the description compiler.
    pub recognition: Recognition,
    /// Concrete nodes expanded from this row.
    pub expansions: Vec<NodeId>,
}

/// One concrete expansion of a row: a specific auction sequence with a compiled constraint.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Node {
    /// Id.
    pub id: NodeId,
    /// Source row.
    pub row: RowId,
    /// Whose hand the constraint describes.
    pub side: Side,
    /// Pattern path (shared with the row).
    pub path: Arc<[SidedPattern]>,
    /// Concrete calls from the opening bid up to and including this call, implicit passes
    /// included.
    pub calls: Vec<Call>,
    /// This node's call.
    pub call: Call,
    /// Variable binding of this expansion.
    pub binding: Binding,
    /// Seat condition.
    pub seat: SeatCond,
    /// Vulnerability condition.
    pub vul: VulCond,
    /// The compiled constraint (never contains `HandConstraint::Custom`).
    pub constraint: HandConstraint,
    /// Weights of the top-level `Or` branches (`{w:X}` annotations); `None` = equal.
    pub branch_weights: Option<Vec<f32>>,
    /// `{prio:N}` annotation; default 0. Higher wins in `choose_bid`.
    pub priority: i16,
    /// `log2` of an estimate of the constraint's volume (for `TieBreak::Narrowest`).
    pub volume_log2: i16,
    /// Alert status.
    pub alertable: Alertability,
    /// Derived flags.
    pub flags: NodeFlags,
    /// Description after variable substitution (`4+M` → `4+!h`).
    pub description: String,
    /// Row-nodes one actual call deeper (implicit passes skipped).
    pub children: Vec<NodeId>,
}

/// Alert status of a call.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum Alertability {
    #[default]
    Unspecified,
    NotAlertable,
    Alertable,
    Announceable,
}

/// Forcing status derived from the description.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum Forcing {
    #[default]
    Unknown,
    NonForcing,
    OneRound,
    ToGame,
}

/// Flags derived from the description that are not constraints.
#[derive(Clone, PartialEq, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NodeFlags {
    /// `ART`, `(R)`, `TRF`, `PUP`, named conventions, or the `!` marker.
    pub artificial: bool,
    /// Forcing status.
    pub forcing: Forcing,
    /// Some fragment was hedged (`usually`, `may`, …).
    pub soft: bool,
    /// Transfer target.
    pub transfer_to: Option<Strain>,
    /// Agreed trump suit.
    pub agreed_suit: Option<Suit>,
    /// `S/O`, `T/P`.
    pub sign_off: bool,
}

/// Recognition statistics of one description.
#[derive(Clone, PartialEq, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Recognition {
    /// Words covered by a recognised fragment.
    pub covered: u16,
    /// Words in the denominator (stopwords excluded unless covered).
    pub total: u16,
    /// `covered / total` (1.0 for an empty description).
    pub ratio: f32,
    /// Byte spans of unrecognised text.
    pub unrecognized: Vec<(u16, u16)>,
    /// Whether any fragment produced a constraint literal.
    pub constraint_bearing: bool,
    /// Fragments resolved with an assumed (not stated) context value.
    pub assumed: u8,
    /// Hedged fragments.
    pub soft: u8,
}

/// System metadata: `#+KEY:` values plus the compiler's stamps.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SystemMeta {
    /// `#+TITLE`.
    pub name: String,
    /// `#+DESCRIPTION`.
    pub description: String,
    /// `#+AUTHOR` (split on `,` and ` and `).
    pub authors: Vec<String>,
    /// `#+VERSION` (author-controlled; default `"0"`).
    pub version: String,
    /// `#+DATE`.
    pub date: Option<String>,
    /// blake3 of the resolved source.
    pub source_hash: [u8; 32],
    /// The compiler that produced this IR.
    pub compiler_version: String,
    /// The IR format version.
    pub ir_format: u32,
    /// `#+DISTPOINTS: 321 | bergen | none`.
    pub dist_method: DistMethod,
    /// `#+TIEBREAK`.
    pub tie_break: TieBreak,
    /// `#+STRENGTH`.
    pub strength: StrengthVocab,
    /// `#+BALANCED`.
    pub balanced: BalancedDef,
    /// `#+NATURAL`.
    pub natural: NaturalParams,
    /// `#+CONVENTION`.
    pub conventions: ConventionDefaults,
    /// `#+RECOGNITION` threshold below which a row is reported (default 0.5).
    pub recognition_threshold: f32,
    /// Unknown `#+KEY`s, preserved.
    pub extra: BTreeMap<String, String>,
}

/// Point thresholds that give meaning to `GF`, `INV`, `weak`, … (`#+STRENGTH:`).
#[derive(Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StrengthVocab {
    /// Combined points for game (default 25).
    pub gf_total: u8,
    /// Combined points for an invitation (default 22..=24).
    pub inv_total: RangeInclusive<u8>,
    /// Combined points for a slam try (default 31).
    pub slam_total: u8,
    /// `STR`: minimum for "strong" (default 16).
    pub strong_min: u8,
    /// `weak`: maximum for "weak" (default 9).
    pub weak_max: u8,
    /// `NEG`: maximum for a negative response (default 7).
    pub neg_max: u8,
    /// Minimum opening strength (default 12).
    pub opening_min: u8,
    /// Absolute maximum (37).
    pub hcp_max: u8,
}

impl Default for StrengthVocab {
    fn default() -> StrengthVocab {
        StrengthVocab {
            gf_total: 25,
            inv_total: 22..=24,
            slam_total: 31,
            strong_min: 16,
            weak_max: 9,
            neg_max: 7,
            opening_min: 12,
            hcp_max: 37,
        }
    }
}

/// How `choose_bid` breaks priority ties (`#+TIEBREAK:`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TieBreak {
    /// First definition in the file wins (BML semantics).
    #[default]
    RowOrder,
    /// Smallest estimated constraint volume wins.
    Narrowest,
    /// Lowest call wins.
    LowestCall,
    /// Highest call wins.
    HighestCall,
}

/// What `bal` / `semi-bal` mean (`#+BALANCED:`).
#[derive(Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BalancedDef {
    /// Classes counted as balanced (default 4333, 4432, 5332).
    pub balanced: Vec<bridge_core::ShapeClass>,
    /// Extra classes counted as semi-balanced (default 5422, 6322).
    pub semi_balanced: Vec<bridge_core::ShapeClass>,
}

impl Default for BalancedDef {
    fn default() -> BalancedDef {
        use bridge_core::ShapeClass as C;
        BalancedDef {
            balanced: vec![C::C4333, C::C4432, C::C5332],
            semi_balanced: vec![C::C5422, C::C6322],
        }
    }
}

/// Default constraints for named conventions (`#+CONVENTION:`).
#[derive(Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ConventionDefaults {
    /// Minimum length shown by a transfer (default 5).
    pub transfer_len: u8,
    /// Stayman shows a 4-card major (default true).
    pub stayman_major: bool,
    /// Support shown by a splinter (default 4).
    pub splinter_support: u8,
}

impl Default for ConventionDefaults {
    fn default() -> ConventionDefaults {
        ConventionDefaults {
            transfer_len: 5,
            stayman_major: true,
            splinter_support: 4,
        }
    }
}
