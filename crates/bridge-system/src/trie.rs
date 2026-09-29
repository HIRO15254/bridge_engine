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
use smallvec::{SmallVec, smallvec};

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
    ///
    /// `k = auction.leading_passes()` gives the opener's position (`k + 1`); an auction that is
    /// entirely leading passes (empty or passed-out) has no opener and yields `None`.
    pub fn for_auction(auction: &'a Auction, owner: Seat) -> Option<LookupKey<'a>> {
        let k = auction.leading_passes();
        if auction.calls().len() == k {
            return None;
        }
        let opener = auction.seat_at(k);
        let we_opened = opener.side() == owner.side();
        let calls = &auction.calls()[k..];
        let opener_pos = auction.position_of(opener);
        let vulnerability = auction.vulnerability();
        let vul = RelVul {
            we: vulnerability.is_vulnerable(owner),
            they: vulnerability.is_vulnerable(owner.next()),
        };
        Some(LookupKey {
            we_opened,
            calls,
            opener_pos,
            vul,
        })
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
    /// The trie node reached at `matched_depth - 1`: the position whose `children` are the
    /// siblings of the last matched call (the root when nothing matched, where it equals `end`).
    /// Tracked inside the walk at no extra cost, so a caller never has to re-resolve the
    /// one-call-shorter key to find the siblings; every attempt returned by
    /// [`AuctionTrie::resolve_lenient`] carries it too. Used with
    /// [`crate::exclusive::ExclusiveIndex`] to find the sibling group of a call.
    pub parent: TrieId,
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

/// One edge of an inserted path: either a concrete call, taken by either side, or an opponents'
/// interference class matched by predicate at lookup time (see [`OppClass::matches`]).
///
/// [`AuctionTrie::insert`] only ever needs [`Edge::Call`]; the expansion stage (which must also
/// insert wildcard opponents' steps such as `(D)` or `(2C+)`) uses
/// [`AuctionTrie::insert_path`] directly.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Edge {
    /// A concrete call.
    Call(Call),
    /// An opponents' interference class.
    Class(OppClass),
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

impl TrieNode {
    fn root() -> TrieNode {
        TrieNode {
            depth: 0,
            exact: Vec::new(),
            classes: Vec::new(),
            entries: Vec::new(),
        }
    }
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
        AuctionTrie {
            nodes: vec![TrieNode::root(), TrieNode::root()],
        }
    }

    /// Walks the trie; about 30 ns per call.
    ///
    /// At each depth an exact edge for the call always wins over a wildcard ([`Edge::Class`])
    /// edge, and among wildcard edges the first inserted one whose class matches wins. The walk
    /// never backtracks (`docs/design/06-system.md` §6.2): when the exact subtree has no
    /// continuation for a later call, the lookup stops there even if a wildcard sibling's
    /// subtree would have matched the rest (e.g. with both `1C-(1S)-` and `1C-(suit)-2C`
    /// defined, `1C (1S) 2C` stops after `(1S)`). An author who wants the wildcard's subtree
    /// under the exact interference too writes it there (or `#PASTE`s it).
    pub fn resolve(&self, key: &LookupKey<'_>) -> Lookup {
        let root = Self::root_id(key.we_opened);
        let mut cur = root;
        let mut end = root;
        let mut parent = root;
        let mut matched_depth = 0usize;
        let mut via_class = 0u8;
        let mut by_depth: SmallVec<[Option<NodeId>; 16]> = smallvec![None; key.calls.len()];

        for (i, &call) in key.calls.iter().enumerate() {
            let node = &self.nodes[cur.0 as usize];
            let idx = call.index();
            let found = match node.exact.binary_search_by_key(&idx, |&(k, _)| k) {
                Ok(pos) => Some((node.exact[pos].1, false)),
                Err(_) => node
                    .classes
                    .iter()
                    .find(|(class, _)| class.matches(call))
                    .map(|&(_, child)| (child, true)),
            };
            let Some((child_id, is_class)) = found else {
                break;
            };

            let child_node = &self.nodes[child_id.0 as usize];
            let selected = Self::best_entry(&child_node.entries, key.opener_pos, key.vul);
            by_depth[i] = selected;

            let ours = Self::is_ours(key.we_opened, i);
            if ours && selected.is_none() && !child_node.entries.is_empty() {
                // A real (non-implicit-pass) node exists here, but none of its entries hold for
                // this (opener_pos, vul); our side has no defined call, so the matched prefix
                // stops before this call (see 06-system.md §6.3).
                break;
            }

            parent = cur;
            cur = child_id;
            end = child_id;
            matched_depth = i + 1;
            if is_class {
                via_class += 1;
            }
        }

        Lookup {
            matched_depth,
            by_depth,
            end,
            parent,
            via_class,
        }
    }

    /// The candidate next calls at `at` whose conditions hold for `(opener_pos, vul)`.
    pub fn children(&self, at: TrieId, opener_pos: u8, vul: RelVul) -> Vec<(Call, NodeId)> {
        let node = &self.nodes[at.0 as usize];
        node.exact
            .iter()
            .filter_map(|&(idx, child)| {
                let child_node = &self.nodes[child.0 as usize];
                Self::best_entry(&child_node.entries, opener_pos, vul).map(|id| {
                    let call = Call::from_index(idx).expect("stored call index is valid");
                    (call, id)
                })
            })
            .collect()
    }

    /// Retries with up to `max_subst` unmatched opponents' calls replaced by `Pass`
    /// ("system on"); returns each alternative with the number of substitutions used.
    pub fn resolve_lenient(
        &self,
        key: &LookupKey<'_>,
        max_subst: u8,
    ) -> SmallVec<[(Lookup, u8); 4]> {
        let mut results: SmallVec<[(Lookup, u8); 4]> = SmallVec::new();
        let mut calls: SmallVec<[Call; 16]> = SmallVec::from_slice(key.calls);
        let mut subst = 0u8;

        loop {
            let attempt = LookupKey {
                we_opened: key.we_opened,
                calls: &calls,
                opener_pos: key.opener_pos,
                vul: key.vul,
            };
            let lookup = self.resolve(&attempt);
            let stalled_at = lookup.matched_depth;
            let exact = stalled_at == calls.len();
            results.push((lookup, subst));

            if exact || subst >= max_subst || results.len() >= 4 {
                break;
            }

            // The first unmatched *opponents'* call at or after the stall point becomes Pass.
            let next = (stalled_at..calls.len())
                .find(|&i| !Self::is_ours(key.we_opened, i) && calls[i] != Call::Pass);
            match next {
                Some(i) => {
                    calls[i] = Call::Pass;
                    subst += 1;
                }
                None => break,
            }
        }

        results
    }

    /// Inserts a node for the concrete path `calls` under the given conditions. Returns the
    /// existing entry's node if an entry with identical conditions is already present (first
    /// definition wins).
    ///
    /// All-exact convenience over [`AuctionTrie::insert_path`]; calls with `Class` steps (the
    /// opponents' interference wildcards) need `insert_path` directly.
    pub fn insert(
        &mut self,
        we_opened: bool,
        calls: &[Call],
        seat: SeatCond,
        vul: VulCond,
        node: NodeId,
    ) -> Result<(), NodeId> {
        let path: Vec<Edge> = calls.iter().copied().map(Edge::Call).collect();
        self.insert_path(we_opened, &path, seat, vul, node)
    }

    /// An already-inserted entry at the trie position reached by `path`, among those already
    /// present, whose `(seat, vul)` condition *covers* the given `(seat, vul)`: every
    /// `(opener_position, vulnerability)` that would satisfy the given condition also satisfies
    /// the existing entry's. Read-only: a `path` that does not exist yet in the trie yields
    /// `None`, the same as when no entry covers.
    ///
    /// Used by expansion to avoid inserting a needlessly more specific, empty-description entry
    /// (typically a history token re-traced under a table's own `#SEAT`/`#VUL`) that would
    /// shadow an already-defined node whose conditions already cover it
    /// (`docs/design/06-system.md` §4.2).
    pub(crate) fn covering_entry(
        &self,
        we_opened: bool,
        path: &[Edge],
        seat: SeatCond,
        vul: VulCond,
    ) -> Option<NodeId> {
        let mut cur = Self::root_id(we_opened);
        for edge in path {
            cur = match *edge {
                Edge::Call(call) => self.find_child_call(cur, call)?,
                Edge::Class(class) => self.find_child_class(cur, class)?,
            };
        }
        self.nodes[cur.0 as usize]
            .entries
            .iter()
            .find(|e| condition_covers(e.seat, e.vul, seat, vul))
            .map(|e| e.node)
    }

    /// The nodes of the entries at the trie position reached by `path` whose `(seat, vul)`
    /// condition is *covered by* the given one (every `(opener_position, vulnerability)` that
    /// satisfies theirs also satisfies `(seat, vul)`), excluding an identical condition. The
    /// mirror image of [`Self::covering_entry`]: used by expansion when a general definition is
    /// inserted *after* a more specific, empty-description placeholder for the same call
    /// (`docs/design/06-system.md` §4.2). Read-only; a missing `path` yields nothing.
    pub(crate) fn covered_entries(
        &self,
        we_opened: bool,
        path: &[Edge],
        seat: SeatCond,
        vul: VulCond,
    ) -> Vec<NodeId> {
        let mut cur = Self::root_id(we_opened);
        for edge in path {
            let next = match *edge {
                Edge::Call(call) => self.find_child_call(cur, call),
                Edge::Class(class) => self.find_child_class(cur, class),
            };
            match next {
                Some(id) => cur = id,
                None => return Vec::new(),
            }
        }
        self.nodes[cur.0 as usize]
            .entries
            .iter()
            .filter(|e| !(e.seat == seat && e.vul == vul))
            .filter(|e| condition_covers(seat, vul, e.seat, e.vul))
            .map(|e| e.node)
            .collect()
    }

    /// An existing entry at the trie position reached by `path` whose specificity equals the
    /// given `(seat, vul)`'s and which *overlaps* it (some `(opener_position, vulnerability)`
    /// satisfies both) without being identical to it (`insert_path` already returns `Err` for an
    /// identical condition -- first definition wins, no tie to report there). This is a genuine
    /// specificity tie: at lookup, either entry could match the same real auction, and only
    /// insertion order (via [`Self::best_entry`]'s `>=`) decides which one wins
    /// (`docs/design/06-system.md` §4.2/§9.3, `LintCode::ConditionTie`). Read-only, like
    /// [`Self::covering_entry`]: a `path` that does not exist yet yields `None`.
    pub(crate) fn tied_entry(
        &self,
        we_opened: bool,
        path: &[Edge],
        seat: SeatCond,
        vul: VulCond,
    ) -> Option<NodeId> {
        let mut cur = Self::root_id(we_opened);
        for edge in path {
            cur = match *edge {
                Edge::Call(call) => self.find_child_call(cur, call)?,
                Edge::Class(class) => self.find_child_class(cur, class)?,
            };
        }
        let specificity = seat.specificity() * 3 + vul.specificity();
        self.nodes[cur.0 as usize]
            .entries
            .iter()
            .find(|e| {
                e.specificity == specificity
                    && !(e.seat == seat && e.vul == vul)
                    && condition_overlaps(e.seat, e.vul, seat, vul)
            })
            .map(|e| e.node)
    }

    /// The existing child of `at` for `call`, without creating it.
    fn find_child_call(&self, at: TrieId, call: Call) -> Option<TrieId> {
        let idx = call.index();
        self.nodes[at.0 as usize]
            .exact
            .binary_search_by_key(&idx, |&(k, _)| k)
            .ok()
            .map(|pos| self.nodes[at.0 as usize].exact[pos].1)
    }

    /// The existing wildcard child of `at` for `class`, without creating it.
    fn find_child_class(&self, at: TrieId, class: OppClass) -> Option<TrieId> {
        self.nodes[at.0 as usize]
            .classes
            .iter()
            .find(|(c, _)| *c == class)
            .map(|&(_, id)| id)
    }

    /// Inserts a node for a path of edges, where an opponents' step may be a concrete [`Call`]
    /// or an [`OppClass`] wildcard. Returns the existing entry's node if an entry with identical
    /// conditions is already present at the resulting trie node (first definition wins).
    pub fn insert_path(
        &mut self,
        we_opened: bool,
        path: &[Edge],
        seat: SeatCond,
        vul: VulCond,
        node: NodeId,
    ) -> Result<(), NodeId> {
        let mut cur = Self::root_id(we_opened);
        for edge in path {
            cur = match *edge {
                Edge::Call(call) => self.child_for_call(cur, call),
                Edge::Class(class) => self.child_for_class(cur, class),
            };
        }

        let specificity = seat.specificity() * 3 + vul.specificity();
        let entries = &mut self.nodes[cur.0 as usize].entries;
        if let Some(existing) = entries.iter().find(|e| e.seat == seat && e.vul == vul) {
            return Err(existing.node);
        }
        entries.push(Entry {
            seat,
            vul,
            specificity,
            node,
        });
        Ok(())
    }

    /// Number of trie nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// `true` when only the two roots exist.
    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 2
    }

    /// `true` when `at` has at least one exact child edge (a candidate of
    /// [`AuctionTrie::children`] under some condition class).
    pub fn has_children(&self, at: TrieId) -> bool {
        !self.nodes[at.0 as usize].exact.is_empty()
    }

    /// `true` when some entry of an exact child of `at` carries a seat or vulnerability
    /// condition, i.e. [`AuctionTrie::children`] may depend on `(opener_pos, vul)`. When
    /// `false`, `children(at, ..)` is the same list for every condition.
    pub fn children_are_conditioned(&self, at: TrieId) -> bool {
        self.nodes[at.0 as usize].exact.iter().any(|&(_, child)| {
            self.nodes[child.0 as usize]
                .entries
                .iter()
                .any(|e| e.seat != SeatCond::Any || e.vul.specificity() != 0)
        })
    }

    fn root_id(we_opened: bool) -> TrieId {
        TrieId(if we_opened { 0 } else { 1 })
    }

    /// Whether the call at depth `i` (0-based, within `calls` after leading passes were
    /// stripped) belongs to the owner's side: sides strictly alternate starting from the
    /// opener's side at `i == 0`.
    fn is_ours(we_opened: bool, i: usize) -> bool {
        if i % 2 == 0 { we_opened } else { !we_opened }
    }

    /// The node whose conditions hold for `(opener_pos, vul)` with the highest specificity;
    /// ties keep the first-inserted entry.
    fn best_entry(entries: &[Entry], opener_pos: u8, vul: RelVul) -> Option<NodeId> {
        let mut best: Option<&Entry> = None;
        for entry in entries {
            if entry.seat.matches(opener_pos) && entry.vul.matches(vul.we, vul.they) {
                best = match best {
                    Some(b) if b.specificity >= entry.specificity => Some(b),
                    _ => Some(entry),
                };
            }
        }
        best.map(|e| e.node)
    }

    /// The existing child of `at` for `call`, creating it (with `depth + 1`) if absent.
    fn child_for_call(&mut self, at: TrieId, call: Call) -> TrieId {
        let idx = call.index();
        match self.nodes[at.0 as usize]
            .exact
            .binary_search_by_key(&idx, |&(k, _)| k)
        {
            Ok(pos) => self.nodes[at.0 as usize].exact[pos].1,
            Err(pos) => {
                let depth = self.nodes[at.0 as usize].depth + 1;
                let new_id = TrieId(self.nodes.len() as u32);
                self.nodes.push(TrieNode {
                    depth,
                    exact: Vec::new(),
                    classes: Vec::new(),
                    entries: Vec::new(),
                });
                self.nodes[at.0 as usize].exact.insert(pos, (idx, new_id));
                new_id
            }
        }
    }

    /// The existing wildcard child of `at` for `class`, creating it (with `depth + 1`) if
    /// absent.
    fn child_for_class(&mut self, at: TrieId, class: OppClass) -> TrieId {
        if let Some(&(_, id)) = self.nodes[at.0 as usize]
            .classes
            .iter()
            .find(|(c, _)| *c == class)
        {
            return id;
        }
        let depth = self.nodes[at.0 as usize].depth + 1;
        let new_id = TrieId(self.nodes.len() as u32);
        self.nodes.push(TrieNode {
            depth,
            exact: Vec::new(),
            classes: Vec::new(),
            entries: Vec::new(),
        });
        self.nodes[at.0 as usize].classes.push((class, new_id));
        new_id
    }
}

/// Whether some `(opener_position, vulnerability)` satisfies both `(a_seat, a_vul)` and
/// `(b_seat, b_vul)` -- i.e. the two conditions can genuinely both match the same real auction.
/// Brute force over the same finite domain as [`condition_covers`], for the same reason.
fn condition_overlaps(a_seat: SeatCond, a_vul: VulCond, b_seat: SeatCond, b_vul: VulCond) -> bool {
    for position in 1..=4u8 {
        if !a_seat.matches(position) || !b_seat.matches(position) {
            continue;
        }
        for we in [true, false] {
            for they in [true, false] {
                if a_vul.matches(we, they) && b_vul.matches(we, they) {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether every `(opener_position, vulnerability)` satisfying `(b_seat, b_vul)` also satisfies
/// `(a_seat, a_vul)` -- i.e. an entry under `(a_seat, a_vul)` already covers whatever
/// `(b_seat, b_vul)` would match, so a new entry for `(b_seat, b_vul)` could only ever shadow it,
/// never add a case it does not already handle. Checked by brute force over the finite domain
/// (4 positions x 2 x 2 vulnerabilities): both condition types are small enums with no relation
/// between variants worth hand-encoding.
fn condition_covers(a_seat: SeatCond, a_vul: VulCond, b_seat: SeatCond, b_vul: VulCond) -> bool {
    for position in 1..=4u8 {
        if !b_seat.matches(position) {
            continue;
        }
        for &we in &[false, true] {
            for &they in &[false, true] {
                if b_vul.matches(we, they) && !(a_seat.matches(position) && a_vul.matches(we, they))
                {
                    return false;
                }
            }
        }
    }
    true
}

impl Default for AuctionTrie {
    fn default() -> AuctionTrie {
        AuctionTrie::new()
    }
}

#[cfg(test)]
mod tests {
    use bridge_core::{Bid, Call, Seat, Strain, Vulnerability};

    use super::*;

    fn bid(level: u8, strain: Strain) -> Call {
        Call::Bid(Bid::new(level, strain).unwrap())
    }

    const PASS: Call = Call::Pass;
    const DBL: Call = Call::Double;

    fn seat_vul(seat: SeatCond, vul: VulCond) -> (SeatCond, VulCond) {
        (seat, vul)
    }

    #[test]
    fn new_trie_has_two_empty_roots() {
        let trie = AuctionTrie::new();
        assert_eq!(trie.len(), 2);
        assert!(trie.is_empty());
    }

    #[test]
    fn exact_path_resolves() {
        let mut trie = AuctionTrie::new();
        let n1 = NodeId(1);
        let n2 = NodeId(2);
        trie.insert(
            true,
            &[bid(1, Strain::Clubs)],
            SeatCond::Any,
            VulCond::default(),
            n1,
        )
        .unwrap();
        trie.insert(
            true,
            &[bid(1, Strain::Clubs), bid(1, Strain::Hearts)],
            SeatCond::Any,
            VulCond::default(),
            n2,
        )
        .unwrap();

        let calls = [bid(1, Strain::Clubs), bid(1, Strain::Hearts)];
        let key = LookupKey {
            we_opened: true,
            calls: &calls,
            opener_pos: 1,
            vul: RelVul {
                we: false,
                they: false,
            },
        };
        let lookup = trie.resolve(&key);
        assert!(lookup.is_exact(&key));
        assert_eq!(lookup.matched_depth, 2);
        assert_eq!(lookup.by_depth.as_slice(), &[Some(n1), Some(n2)]);
        assert_eq!(lookup.via_class, 0);
    }

    #[test]
    fn wildcard_path_resolves_via_class() {
        let mut trie = AuctionTrie::new();
        let opening = NodeId(1);
        let overcall_resp = NodeId(2);
        trie.insert(
            true,
            &[bid(1, Strain::Clubs)],
            SeatCond::Any,
            VulCond::default(),
            opening,
        )
        .unwrap();
        trie.insert_path(
            true,
            &[
                Edge::Call(bid(1, Strain::Clubs)),
                Edge::Class(OppClass::AnySuitBid),
                Edge::Call(DBL),
            ],
            SeatCond::Any,
            VulCond::default(),
            overcall_resp,
        )
        .unwrap();

        let calls = [bid(1, Strain::Clubs), bid(1, Strain::Spades), DBL];
        let key = LookupKey {
            we_opened: true,
            calls: &calls,
            opener_pos: 1,
            vul: RelVul {
                we: false,
                they: false,
            },
        };
        let lookup = trie.resolve(&key);
        assert!(lookup.is_exact(&key));
        assert_eq!(lookup.via_class, 1);
        assert_eq!(lookup.by_depth[2], Some(overcall_resp));
    }

    #[test]
    fn seat_and_vul_conditions_pick_most_specific() {
        let mut trie = AuctionTrie::new();
        let general = NodeId(1);
        let first_seat = NodeId(2);
        let vul_specific = NodeId(3);

        trie.insert(
            true,
            &[bid(1, Strain::NoTrump)],
            SeatCond::Any,
            VulCond::default(),
            general,
        )
        .unwrap();
        trie.insert(
            true,
            &[bid(1, Strain::NoTrump)],
            SeatCond::First,
            VulCond::default(),
            first_seat,
        )
        .unwrap();
        trie.insert(
            true,
            &[bid(1, Strain::NoTrump)],
            SeatCond::Any,
            VulCond {
                we: crate::ast::Tri::Yes,
                they: crate::ast::Tri::Any,
            },
            vul_specific,
        )
        .unwrap();

        let calls = [bid(1, Strain::NoTrump)];
        let not_vul = RelVul {
            we: false,
            they: false,
        };
        let vulnerable = RelVul {
            we: true,
            they: false,
        };

        // Seat 1, not vulnerable: SeatCond::First (specificity 2*3=6) beats Any (0).
        let key = LookupKey {
            we_opened: true,
            calls: &calls,
            opener_pos: 1,
            vul: not_vul,
        };
        assert_eq!(trie.resolve(&key).by_depth[0], Some(first_seat));

        // Seat 2, not vulnerable: only the general entry matches.
        let key = LookupKey {
            we_opened: true,
            calls: &calls,
            opener_pos: 2,
            vul: not_vul,
        };
        assert_eq!(trie.resolve(&key).by_depth[0], Some(general));

        // Seat 2, we vulnerable: vul-specific (specificity 1) beats general (0), and there is
        // no seat-specific match since opener_pos != 1.
        let key = LookupKey {
            we_opened: true,
            calls: &calls,
            opener_pos: 2,
            vul: vulnerable,
        };
        assert_eq!(trie.resolve(&key).by_depth[0], Some(vul_specific));
    }

    #[test]
    fn first_definition_wins_on_identical_conditions() {
        let mut trie = AuctionTrie::new();
        let first = NodeId(1);
        let second = NodeId(2);
        let (seat, vul) = seat_vul(SeatCond::Any, VulCond::default());

        let ok = trie.insert(true, &[bid(1, Strain::Clubs)], seat, vul, first);
        assert!(ok.is_ok());

        let err = trie.insert(true, &[bid(1, Strain::Clubs)], seat, vul, second);
        assert_eq!(err, Err(first));
    }

    #[test]
    fn leading_passes_and_roots() {
        let mut trie = AuctionTrie::new();
        let they_open_1c = NodeId(1);
        trie.insert(
            false,
            &[bid(1, Strain::Clubs)],
            SeatCond::Any,
            VulCond::default(),
            they_open_1c,
        )
        .unwrap();

        // An auction dealt by North (index 0): South (index 2, North-South's side) opens 1C
        // after North/East pass. The owner sits East (East-West's side), so `we_opened` is
        // false and `opener_pos` is 3.
        let dealer = Seat::North;
        let auction = bridge_core::Auction::from_calls(
            dealer,
            Vulnerability::None,
            [PASS, PASS, bid(1, Strain::Clubs)],
        )
        .unwrap();
        let key = LookupKey::for_auction(&auction, Seat::East).unwrap();
        assert!(!key.we_opened);
        assert_eq!(key.opener_pos, 3);
        assert_eq!(key.calls, &[bid(1, Strain::Clubs)]);

        let lookup = trie.resolve(&key);
        assert!(lookup.is_exact(&key));
        assert_eq!(lookup.by_depth[0], Some(they_open_1c));
    }

    #[test]
    fn for_auction_is_none_when_empty_or_passed_out() {
        let dealer = Seat::North;
        let empty = bridge_core::Auction::new(dealer, Vulnerability::None);
        assert!(LookupKey::for_auction(&empty, Seat::North).is_none());

        let passed_out =
            bridge_core::Auction::from_calls(dealer, Vulnerability::None, [PASS, PASS, PASS, PASS])
                .unwrap();
        assert!(LookupKey::for_auction(&passed_out, Seat::North).is_none());
    }

    #[test]
    fn partial_depth_stops_before_unresolved_own_call() {
        let mut trie = AuctionTrie::new();
        let opening = NodeId(1);
        // Only #SEAT 1 is defined for our rebid; opener_pos 1 in this test, so it always
        // matches — instead we omit any entry for depth 2 entirely to force a stop there.
        trie.insert(
            true,
            &[bid(1, Strain::Clubs)],
            SeatCond::Any,
            VulCond::default(),
            opening,
        )
        .unwrap();
        // Create the depth-2 trie node (their pass) and depth-3 node (our rebid) without any
        // entry at depth 3, by inserting a *different*, unrelated deeper path first so the
        // node exists, then relying on there being no matching entry for our test key.
        trie.insert(
            true,
            &[bid(1, Strain::Clubs), PASS, bid(2, Strain::Clubs)],
            SeatCond::First,
            VulCond::default(),
            NodeId(2),
        )
        .unwrap();

        let calls = [bid(1, Strain::Clubs), PASS, bid(2, Strain::Clubs)];
        let key = LookupKey {
            we_opened: true,
            calls: &calls,
            opener_pos: 2, // does not satisfy SeatCond::First
            vul: RelVul {
                we: false,
                they: false,
            },
        };
        let lookup = trie.resolve(&key);
        assert!(!lookup.is_exact(&key));
        assert_eq!(lookup.matched_depth, 2);
        assert_eq!(lookup.by_depth[0], Some(opening));
        assert_eq!(lookup.by_depth[1], None); // their implicit pass has no row
        assert_eq!(lookup.by_depth[2], None); // condition mismatch on our rebid
    }

    #[test]
    fn implicit_pass_node_does_not_break_matched_depth() {
        let mut trie = AuctionTrie::new();
        let opening = NodeId(1);
        let rebid = NodeId(2);
        // 1C - (implicit pass) - 2C, with no row ever attached to the implicit pass depth.
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
            &[bid(1, Strain::Clubs), PASS, bid(2, Strain::Clubs)],
            SeatCond::Any,
            VulCond::default(),
            rebid,
        )
        .unwrap();

        let calls = [bid(1, Strain::Clubs), PASS, bid(2, Strain::Clubs)];
        let key = LookupKey {
            we_opened: true,
            calls: &calls,
            opener_pos: 1,
            vul: RelVul {
                we: false,
                they: false,
            },
        };
        let lookup = trie.resolve(&key);
        assert!(lookup.is_exact(&key));
        assert_eq!(
            lookup.by_depth.as_slice(),
            &[Some(opening), None, Some(rebid)]
        );
    }

    #[test]
    fn covering_entry_finds_a_general_entry_covering_a_specific_condition() {
        let mut trie = AuctionTrie::new();
        let general = NodeId(1);
        trie.insert(
            true,
            &[bid(1, Strain::Hearts)],
            SeatCond::Any,
            VulCond::default(),
            general,
        )
        .unwrap();

        // SeatCond::Any/VulCond::default() (the general entry) covers any more specific
        // condition, such as ThirdOrFourth/Any.
        let covering = trie.covering_entry(
            true,
            &[Edge::Call(bid(1, Strain::Hearts))],
            SeatCond::ThirdOrFourth,
            VulCond::default(),
        );
        assert_eq!(covering, Some(general));
    }

    #[test]
    fn tied_entry_finds_an_overlapping_equal_specificity_condition() {
        use crate::ast::Tri;

        let mut trie = AuctionTrie::new();
        let first = NodeId(1);
        trie.insert(
            true,
            &[bid(1, Strain::Clubs)],
            SeatCond::Any,
            VulCond {
                we: Tri::Yes,
                they: Tri::Any,
            },
            first,
        )
        .unwrap();

        // `we=Any, they=Yes` has the same specificity (1) as `we=Yes, they=Any`, and both match
        // a real "both vulnerable" auction: a genuine tie, not a duplicate.
        let tied = trie.tied_entry(
            true,
            &[Edge::Call(bid(1, Strain::Clubs))],
            SeatCond::Any,
            VulCond {
                we: Tri::Any,
                they: Tri::Yes,
            },
        );
        assert_eq!(tied, Some(first));
    }

    #[test]
    fn tied_entry_ignores_an_identical_condition() {
        // An identical condition is `insert_path`'s own `Err` case (`DuplicatePath`, first wins
        // outright) -- not a tie, so `tied_entry` must not also report it.
        let mut trie = AuctionTrie::new();
        trie.insert(
            true,
            &[bid(1, Strain::Clubs)],
            SeatCond::Any,
            VulCond::default(),
            NodeId(1),
        )
        .unwrap();

        let tied = trie.tied_entry(
            true,
            &[Edge::Call(bid(1, Strain::Clubs))],
            SeatCond::Any,
            VulCond::default(),
        );
        assert_eq!(tied, None);
    }

    #[test]
    fn tied_entry_ignores_a_less_specific_non_overlapping_condition() {
        let mut trie = AuctionTrie::new();
        trie.insert(
            true,
            &[bid(1, Strain::Clubs)],
            SeatCond::First,
            VulCond::default(),
            NodeId(1),
        )
        .unwrap();

        // `Second` has the same specificity as `First` but never overlaps it (no position
        // satisfies both): not a tie.
        let tied = trie.tied_entry(
            true,
            &[Edge::Call(bid(1, Strain::Clubs))],
            SeatCond::Second,
            VulCond::default(),
        );
        assert_eq!(tied, None);
    }

    #[test]
    fn covering_entry_is_none_when_no_entry_covers() {
        let mut trie = AuctionTrie::new();
        trie.insert(
            true,
            &[bid(1, Strain::Hearts)],
            SeatCond::First,
            VulCond::default(),
            NodeId(1),
        )
        .unwrap();

        // SeatCond::First only covers position 1, not every position ThirdOrFourth matches.
        let covering = trie.covering_entry(
            true,
            &[Edge::Call(bid(1, Strain::Hearts))],
            SeatCond::ThirdOrFourth,
            VulCond::default(),
        );
        assert_eq!(covering, None);
    }

    #[test]
    fn covering_entry_is_none_for_a_path_not_yet_in_the_trie() {
        let trie = AuctionTrie::new();
        let covering = trie.covering_entry(
            true,
            &[Edge::Call(bid(1, Strain::Hearts))],
            SeatCond::Any,
            VulCond::default(),
        );
        assert_eq!(covering, None);
    }

    #[test]
    fn children_lists_exact_edges_in_call_order() {
        let mut trie = AuctionTrie::new();
        trie.insert(
            true,
            &[bid(1, Strain::Hearts)],
            SeatCond::Any,
            VulCond::default(),
            NodeId(1),
        )
        .unwrap();
        trie.insert(
            true,
            &[bid(1, Strain::Clubs)],
            SeatCond::Any,
            VulCond::default(),
            NodeId(2),
        )
        .unwrap();
        trie.insert(
            true,
            &[bid(1, Strain::Spades)],
            SeatCond::First,
            VulCond::default(),
            NodeId(3),
        )
        .unwrap();

        let root = TrieId(0);
        let not_vul = RelVul {
            we: false,
            they: false,
        };
        // opener_pos 2: the 1S entry (SeatCond::First) does not hold, so it is skipped.
        let kids = trie.children(root, 2, not_vul);
        assert_eq!(
            kids,
            vec![
                (bid(1, Strain::Clubs), NodeId(2)),
                (bid(1, Strain::Hearts), NodeId(1)),
            ]
        );

        let kids = trie.children(root, 1, not_vul);
        assert_eq!(
            kids,
            vec![
                (bid(1, Strain::Clubs), NodeId(2)),
                (bid(1, Strain::Hearts), NodeId(1)),
                (bid(1, Strain::Spades), NodeId(3)),
            ]
        );
    }

    #[test]
    fn resolve_lenient_substitutes_opponents_calls_with_pass() {
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
        // The system is "on" over a pass from the opponents, i.e. as if they had passed.
        trie.insert(
            true,
            &[bid(1, Strain::Clubs), PASS, bid(1, Strain::Hearts)],
            SeatCond::Any,
            VulCond::default(),
            response,
        )
        .unwrap();

        // The opponent actually overcalled, which is not in the trie.
        let calls = [
            bid(1, Strain::Clubs),
            bid(1, Strain::Spades),
            bid(1, Strain::Hearts),
        ];
        let key = LookupKey {
            we_opened: true,
            calls: &calls,
            opener_pos: 1,
            vul: RelVul {
                we: false,
                they: false,
            },
        };

        let base = trie.resolve(&key);
        assert_eq!(base.matched_depth, 1);

        let attempts = trie.resolve_lenient(&key, 2);
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].1, 0);
        assert_eq!(attempts[0].0.matched_depth, 1);
        assert_eq!(attempts[1].1, 1);
        assert!(attempts[1].0.is_exact(&key));
        assert_eq!(attempts[1].0.by_depth[2], Some(response));
    }

    #[test]
    fn resolve_lenient_stops_when_nothing_left_to_substitute() {
        let trie = AuctionTrie::new();
        let calls = [bid(1, Strain::Clubs)];
        let key = LookupKey {
            we_opened: true,
            calls: &calls,
            opener_pos: 1,
            vul: RelVul {
                we: false,
                they: false,
            },
        };
        let attempts = trie.resolve_lenient(&key, 3);
        // Nothing was ever inserted, so there's no edge at all and no opponents' call to
        // substitute (the single call is ours).
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].1, 0);
        assert_eq!(attempts[0].0.matched_depth, 0);
    }

    #[cfg(feature = "cache")]
    #[test]
    fn trie_round_trips_through_serde() {
        let mut trie = AuctionTrie::new();
        trie.insert(
            true,
            &[bid(1, Strain::Clubs), bid(1, Strain::Hearts)],
            SeatCond::First,
            VulCond::default(),
            NodeId(1),
        )
        .unwrap();
        trie.insert_path(
            false,
            &[Edge::Class(OppClass::AnyBid), Edge::Call(DBL)],
            SeatCond::Any,
            VulCond::default(),
            NodeId(2),
        )
        .unwrap();

        let encoded = postcard::to_allocvec(&trie).unwrap();
        let decoded: AuctionTrie = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded.len(), trie.len());

        let calls = [bid(1, Strain::Clubs), bid(1, Strain::Hearts)];
        let key = LookupKey {
            we_opened: true,
            calls: &calls,
            opener_pos: 1,
            vul: RelVul {
                we: false,
                they: false,
            },
        };
        assert_eq!(decoded.resolve(&key).by_depth, trie.resolve(&key).by_depth);
    }
}
