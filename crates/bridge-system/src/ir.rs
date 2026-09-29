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
    /// Derived, lazily built index of exclusive regions ([`SystemIR::exclusive`]). Not
    /// serialised (`IR_FORMAT` and the serialised bytes do not depend on it); a struct literal
    /// sets it to `ExclusiveCell::default()`. After mutating `nodes` or `index` of an IR whose
    /// index was already built, call [`ExclusiveCell::clear`](crate::exclusive::ExclusiveCell::clear).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub exclusive_cell: crate::exclusive::ExclusiveCell,
}

impl SystemIR {
    /// The node with the given id.
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    /// The derived index of exclusive (rank-aware) regions of every position's candidates
    /// (docs/design/15-phase4-plan.md D19): built on first use and cached in
    /// [`SystemIR::exclusive_cell`].
    ///
    /// Naive; replaced in phase 4 lane S: lane S builds it eagerly at the end of `compile()`;
    /// a deserialised or hand-built IR still builds it on first access.
    pub fn exclusive(&self) -> &crate::exclusive::ExclusiveIndex {
        self.exclusive_cell.get_or_build(self)
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
        let key = LookupKey::for_auction(auction, owner)?;
        let lookup = self.index.resolve(&key);
        if !lookup.is_exact(&key) {
            return None;
        }
        Some(self.index.children(lookup.end, key.opener_pos, key.vul))
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
    /// Pattern path (shared with the row). Empty for the nodes the compiler synthesises for
    /// system stops ([`Node::is_synthesised`]); an empty path alone does not mark one (a
    /// hand-built IR may leave every path empty).
    pub path: Arc<[SidedPattern]>,
    /// Concrete calls from the opening bid up to and including this call, implicit passes
    /// included. An opponents' wildcard step (`(any)`/`(bid)`/`(suit)`, a trie
    /// [`Edge::Class`](crate::trie::Edge::Class)) has no concrete call and is stored as a `Pass`
    /// filler here (and as [`Node::call`] of the wildcard's own node); the trie path, not this
    /// vector, is what identifies such a position.
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

impl Node {
    /// `true` for a node the compiler synthesised rather than expanded from a row
    /// ([`NodeFlags::synthesised`]): the stop pass (ours, `Pass` with any hand at `{prio:-100}`,
    /// flagged [`NodeFlags::stop`]) and the opponents' `(any)` step before it, which the system
    /// stops under one `#SEAT`/`#VUL` condition share (`docs/design/06-system.md` §4.5). Such a
    /// node has no position of its own (an empty [`Node::path`] and [`Node::calls`]) and no
    /// parent; its row is a synthesised row too.
    pub fn is_synthesised(&self) -> bool {
        self.flags.synthesised
    }

    /// The description as shown to a user: [`Node::description`] without its `{prio:N}`,
    /// `{w:X}` and `{stop}` annotations, whose content is in [`Node::priority`],
    /// [`Node::branch_weights`] and [`NodeFlags::stop`]. So `P = {prio:-100} {stop} any hand`
    /// and the synthesised stop pass both explain themselves as `any hand`.
    pub fn explanation(&self) -> std::borrow::Cow<'_, str> {
        crate::compile::desc::normalize::strip_annotations(&self.description)
    }
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
    /// A system stop (`{stop}`, `#STOP`, `docs/design/06-system.md` §4.5): once this call is
    /// made, the partnership passes with any hand whatever the opponents call. Set on the nodes
    /// of a stop row and on the synthesised stop pass itself ([`Node::is_synthesised`]).
    /// Informational: resolution follows the trie edges the compiler grafted for the stop.
    pub stop: bool,
    /// The compiler synthesised this node for a system stop rather than expanding it from a row
    /// ([`Node::is_synthesised`]). Never set by a description.
    pub synthesised: bool,
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

impl Default for SystemMeta {
    /// Defaults for a system with no `#+KEY:` values at all: empty name, Goren 3-2-1
    /// distribution points, and SAYC-like natural inference.
    fn default() -> SystemMeta {
        SystemMeta {
            name: String::new(),
            description: String::new(),
            authors: Vec::new(),
            version: "0".to_string(),
            date: None,
            source_hash: [0; 32],
            compiler_version: crate::COMPILER_VERSION.to_string(),
            ir_format: crate::IR_FORMAT,
            dist_method: DistMethod::GOREN_321,
            tie_break: TieBreak::default(),
            strength: StrengthVocab::default(),
            balanced: BalancedDef::default(),
            natural: NaturalParams::default(),
            conventions: ConventionDefaults::default(),
            recognition_threshold: 0.5,
            extra: BTreeMap::new(),
        }
    }
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

#[cfg(test)]
mod tests {
    use bridge_core::{Auction, Bid, Vulnerability};

    use super::*;
    use crate::natural::{AdvanceParams, RebidParams, ResponseParams};

    /// Arbitrary but complete natural-bidding parameters.
    ///
    /// `NaturalParams::default()` is a phase-3 stub (`todo!`, owned by another lane), so tests
    /// that only need *some* valid value build one directly instead of depending on it.
    fn dummy_natural_params() -> NaturalParams {
        NaturalParams {
            opening_hcp: 12..=21,
            open_1m_len: 3,
            open_1major_len: 5,
            nt: vec![(1, 15..=17)],
            weak_two: (6, 5..=10),
            preempt: vec![(3, 7, 5..=10)],
            strong_two_c: 22,
            overcall: [(5, 8..=16), (5, 8..=16), (5, 8..=16)],
            nt_overcall: 15..=18,
            takeout_double: (12, 15, 3),
            response: ResponseParams {
                new_suit_1: (4, 6),
                new_suit_2: (5, 10),
                raise: (3, 6..=9),
                jump_raise: (4, 10..=12),
                nt: vec![(1, 6..=9)],
                jump_shift: 17,
            },
            rebid: RebidParams {
                reverse: 17,
                jump_rebid: 16..=18,
                nt_1: 12..=14,
                nt_2: 18..=19,
                raise: 16..=18,
                jump_raise: 19..=20,
            },
            advance: AdvanceParams {
                raise: (3, 6..=9),
                new_suit: (5, 8),
                cue: 10,
            },
            balancing_shift: -3,
            implicit_raise_support: true,
            level_floor: Default::default(),
        }
    }

    fn bid(level: u8, strain: Strain) -> Call {
        Call::Bid(Bid::new(level, strain).unwrap())
    }

    /// A minimal, otherwise-empty `SystemIR` wrapping the given trie, for exercising
    /// [`SystemIR::continuations`] without a compiled system.
    fn minimal_system(index: AuctionTrie) -> SystemIR {
        SystemIR {
            meta: SystemMeta {
                name: String::new(),
                description: String::new(),
                authors: Vec::new(),
                version: "0".to_string(),
                date: None,
                source_hash: [0; 32],
                compiler_version: String::new(),
                ir_format: 1,
                dist_method: DistMethod::GOREN_321,
                tie_break: TieBreak::default(),
                strength: StrengthVocab::default(),
                balanced: BalancedDef::default(),
                natural: dummy_natural_params(),
                conventions: ConventionDefaults::default(),
                recognition_threshold: 0.5,
                extra: BTreeMap::new(),
            },
            rows: Vec::new(),
            nodes: Vec::new(),
            index,
            lints: Vec::new(),
            exclusive_cell: Default::default(),
        }
    }

    /// Builds a trie with `1C` and `1C-(P)-1D` inserted (owner opened `1C`), matching the
    /// off-system regression reported against `SystemIR::continuations`.
    fn trie_with_opening_and_response() -> (AuctionTrie, NodeId, NodeId) {
        let mut trie = AuctionTrie::new();
        let opening = NodeId(1);
        let response = NodeId(2);
        trie.insert(
            true,
            &[bid(1, Strain::Clubs)],
            SeatCond::Any,
            VulCond::default(),
            opening,
        )
        .unwrap();
        trie.insert(
            true,
            &[bid(1, Strain::Clubs), Call::Pass, bid(1, Strain::Diamonds)],
            SeatCond::Any,
            VulCond::default(),
            response,
        )
        .unwrap();
        (trie, opening, response)
    }

    #[test]
    fn continuations_is_none_when_prefix_is_off_system() {
        let (trie, ..) = trie_with_opening_and_response();
        let ir = minimal_system(trie);

        // `1H` was never inserted anywhere: resolution stalls after the implicit pass
        // (`matched_depth == 2`), so the prefix (up to and including `1H`) is off-system and
        // `continuations` must return `None`, not the `1D` that happens to follow `1C-(P)` in
        // the trie.
        let auction = Auction::from_calls(
            Seat::North,
            Vulnerability::None,
            [bid(1, Strain::Clubs), Call::Pass, bid(1, Strain::Hearts)],
        )
        .unwrap();

        assert_eq!(ir.continuations(&auction, Seat::North), None);
    }

    #[test]
    fn continuations_is_some_when_prefix_is_exact() {
        let (trie, _, response) = trie_with_opening_and_response();
        let ir = minimal_system(trie);

        let auction = Auction::from_calls(
            Seat::North,
            Vulnerability::None,
            [bid(1, Strain::Clubs), Call::Pass],
        )
        .unwrap();

        let candidates = ir.continuations(&auction, Seat::North).unwrap();
        assert_eq!(candidates, vec![(bid(1, Strain::Diamonds), response)]);
    }
}
