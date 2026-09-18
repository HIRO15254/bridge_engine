//! The auction trie: concrete call sequences to nodes.
//!
//! Leading passes are not part of a path; the position in which the opening bid was made is a
//! condition (`#SEAT`). The trie has two roots, one for "we opened" and one for "they opened".
//! Each trie node keeps its exact-call children in a sorted list (binary search) and its
//! wildcard (`OppClass`) children separately; the row nodes attached to an edge are filtered
//! by their seat/vulnerability conditions, the most specific one winning, so at most one node
//! is returned per depth.

use bridge_constraint::HandConstraint;
use bridge_core::{Auction, Call, Seat};
use smallvec::SmallVec;

use crate::{
    NodeId,
    ast::{SeatCond, VulCond},
    pattern::OppClass,
};

/// Index into the trie arena.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TrieId(pub u32);

/// Vulnerability relative to the system owner.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RelVul {
    /// We are vulnerable.
    pub we: bool,
    /// They are vulnerable.
    pub they: bool,
}

/// What the trie is asked about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LookupKey<'a> {
    /// Did the owner's partnership make the first non-pass call.
    pub we_opened: bool,
    /// The calls with the leading passes stripped.
    pub calls: &'a [Call],
    /// Position of the opener, `1..=4`.
    pub opener_pos: u8,
    /// Vulnerability relative to the owner.
    pub vul: RelVul,
}

impl<'a> LookupKey<'a> {
    /// Builds the key for `owner`'s system; `None` for an empty or passed-out auction.
    pub fn for_auction(auction: &'a Auction, owner: Seat) -> Option<LookupKey<'a>> {
        todo!("phase 3")
    }
}

/// The result of a resolution.
#[derive(Clone, Debug)]
pub struct Lookup {
    /// Number of calls matched (`== calls.len()` when exact).
    pub matched_depth: usize,
    /// The node for call `i`, or `None` for implicit passes and unmatched calls.
    pub by_depth: SmallVec<[Option<NodeId>; 16]>,
    /// The trie node reached at `matched_depth` (for `children`).
    pub end: TrieId,
    /// Number of wildcard edges taken (0 = pure exact match).
    pub via_class: u8,
}

impl Lookup {
    /// `true` when every call was matched.
    pub fn is_exact(&self, key: &LookupKey<'_>) -> bool {
        self.matched_depth == key.calls.len()
    }
}

/// How a call was resolved. `Natural` is produced by the bidding layer, never by the trie; the
/// enum lives here so that both layers share it.
#[derive(Clone, Debug)]
pub enum Resolution {
    /// The full sequence is in the trie.
    Exact(NodeId),
    /// Calls `1..=matched_depth` are interpreted by the system; the rest are off-system.
    Partial {
        /// The deepest node at or below `matched_depth`.
        node: NodeId,
        /// The matched prefix length.
        matched_depth: usize,
    },
    /// Natural inference.
    Natural(HandConstraint),
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
struct Entry {
    seat: SeatCond,
    vul: VulCond,
    specificity: u8,
    node: NodeId,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
struct TrieNode {
    depth: u8,
    /// `(Call::index(), child)`, sorted by call index.
    exact: Vec<(u8, TrieId)>,
    classes: Vec<(OppClass, TrieId)>,
    entries: Vec<Entry>,
}

/// The auction index of a [`SystemIR`](crate::SystemIR).
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AuctionTrie {
    /// Arena; index 0 is the "we opened" root, 1 the "they opened" root.
    nodes: Vec<TrieNode>,
}

impl AuctionTrie {
    /// An empty trie with its two roots.
    pub fn new() -> AuctionTrie {
        todo!("phase 3")
    }

    /// Walks the trie; about 30 ns per call.
    pub fn resolve(&self, key: &LookupKey<'_>) -> Lookup {
        todo!("phase 3")
    }

    /// The candidate next calls at `at` whose conditions hold for `(opener_pos, vul)`.
    pub fn children(&self, at: TrieId, opener_pos: u8, vul: RelVul) -> Vec<(Call, NodeId)> {
        todo!("phase 3")
    }

    /// Retries with up to `max_subst` unmatched opponents' calls replaced by `Pass`
    /// ("system on"); returns each alternative with the number of substitutions used.
    pub fn resolve_lenient(
        &self,
        key: &LookupKey<'_>,
        max_subst: u8,
    ) -> SmallVec<[(Lookup, u8); 4]> {
        todo!("phase 3")
    }

    /// Inserts a node for the concrete path `calls` under the given conditions. Returns the
    /// existing entry's node if an entry with identical conditions is already present (first
    /// definition wins).
    pub fn insert(
        &mut self,
        we_opened: bool,
        calls: &[Call],
        seat: SeatCond,
        vul: VulCond,
        node: NodeId,
    ) -> Result<(), NodeId> {
        todo!("phase 3")
    }

    /// Number of trie nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// `true` when only the two roots exist.
    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 2
    }
}

impl Default for AuctionTrie {
    fn default() -> AuctionTrie {
        AuctionTrie::new()
    }
}
