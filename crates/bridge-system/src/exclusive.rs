//! Exclusive (rank-aware) regions of system calls: the derived index behind the policy mirror.
//!
//! `choose_bid` picks, among the satisfied legal candidates at a position, the first one in the
//! single rank order [`rank_cmp`] (priority descending, then `SystemMeta::tie_break`, then call
//! index ascending). Under that policy a hand makes call `c` exactly when the *first* satisfied
//! member of the position's sibling group has call `c`:
//!
//! ```text
//! X_c = ∪_{m ∈ G, call(m) = c} ( C_m ∧ ¬ ∪_{m' ∈ G ranked above m, call(m') ≠ c} C_m' )
//! ```
//!
//! (docs/design/15-phase4-plan.md, D19). `interpret` reads a system call as `X_c` instead of the
//! bare node constraint, so its non-`Fallback` pieces cover exactly the hands for which the
//! policy picks `c`.
//!
//! The sibling set of a trie position depends on the seat/vulnerability condition class (the
//! trie filters row entries by `(opener_pos, vul)`, see [`condition_class`]), so the
//! [`ExclusiveIndex`] is keyed by `(parent trie position, condition class)`, and positions with
//! identical sibling lists share one [`ExclusiveGroup`]. The index is *derived* data: it is not
//! serialised (`IR_FORMAT` and the postcard bytes are unchanged) and lives in the
//! [`ExclusiveCell`] of [`SystemIR`], built on first use by [`SystemIR::exclusive`].
//!
//! Pieces: every member contributes one [`ExclusivePiece`] per top-level `Or` branch of its
//! node's constraint. The branches of one node are disjointified (`b_j ∧ ¬∪_{k<j} b_k`), and each
//! piece also subtracts *every* higher-ranked member (same call or not). The pieces of a group
//! are therefore pairwise disjoint, and the union of the pieces of call `c` is exactly `X_c`
//! above (a hand in a lower same-call member's region that a higher same-call member also
//! contains is counted once, in the higher member's piece). A call whose pieces are all empty
//! is *shadowed*: the policy never makes it at this position.
//!
//! Build: [`crate::compile`] builds the index eagerly at the end of compilation (SAYC: about
//! 714 groups, 2.5k pieces, a handful of tree fallbacks, well under 15 ms release) and stores it
//! in the cell; a deserialised or hand-built IR builds it on the first [`SystemIR::exclusive`]
//! call. Per trie position the sibling list is computed once when no child entry carries a
//! seat/vulnerability condition (the common case), and once per condition class otherwise;
//! identical sibling lists share one group. [`ExclusiveIndex::stats`] reports the counts and
//! the lints `ShadowedBranch` / `OverlappingBranches` read the groups.

use core::cmp::Ordering;
use core::ops::RangeInclusive;
use std::collections::HashMap;
use std::sync::OnceLock;

use bridge_constraint::{Atom, DnfOptions, HandConstraint, Overflow};
use bridge_core::{Call, ShapeSet};

use crate::trie::{RelVul, TrieId};
use crate::{NodeId, SystemIR, TieBreak};

/// Cap on the atoms of one subtracted piece before [`subtract`] falls back to the
/// `And([base, Not(Or(minus))])` tree.
pub const MAX_EXCLUSIVE_ATOMS: usize = 48;

/// The rank-relevant data of one candidate at a position: its call, its priority and its system
/// node (`None` for a synthesised implicit `Pass` or a natural candidate).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RankKey {
    /// The call.
    pub call: Call,
    /// Its priority (a node's `{prio:N}`, `round(confidence·100)` for a natural candidate,
    /// `i16::MIN + 1` for the implicit pass).
    pub priority: i16,
    /// Its system node, if any.
    pub node: Option<NodeId>,
}

/// Compares two tie-break keys under `tie_break` (ascending: the smaller side ranks higher).
/// Rows and volumes of a node-less candidate rank after every node.
fn tie_break_cmp(sys: &SystemIR, tie_break: TieBreak, a: &RankKey, b: &RankKey) -> Ordering {
    match tie_break {
        TieBreak::RowOrder => {
            let ra = a.node.map(|n| sys.node(n).row.0);
            let rb = b.node.map(|n| sys.node(n).row.0);
            match (ra, rb) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        }
        TieBreak::Narrowest => {
            let va = a.node.map_or(i16::MAX, |n| sys.node(n).volume_log2);
            let vb = b.node.map_or(i16::MAX, |n| sys.node(n).volume_log2);
            va.cmp(&vb)
        }
        TieBreak::LowestCall => a.call.index().cmp(&b.call.index()),
        TieBreak::HighestCall => b.call.index().cmp(&a.call.index()),
    }
}

/// The single rank order over candidates at one position, used by `choose_bid`'s sort, the
/// [`ExclusiveIndex`] and (through [`natural_rank_cmp`]) the natural candidate ranking:
/// priority descending, then `sys.meta.tie_break`, then call index ascending.
/// `Ordering::Less` means `a` ranks above (is preferred to) `b`.
pub fn rank_cmp_keys(sys: &SystemIR, a: &RankKey, b: &RankKey) -> Ordering {
    b.priority
        .cmp(&a.priority)
        .then_with(|| tie_break_cmp(sys, sys.meta.tie_break, a, b))
        .then_with(|| a.call.index().cmp(&b.call.index()))
}

/// [`rank_cmp_keys`] for two system candidates `(call, node)` of `sys` (priority read from the
/// node).
pub fn rank_cmp(sys: &SystemIR, a: (Call, NodeId), b: (Call, NodeId)) -> Ordering {
    rank_cmp_keys(
        sys,
        &RankKey {
            call: a.0,
            priority: sys.node(a.1).priority,
            node: Some(a.1),
        },
        &RankKey {
            call: b.0,
            priority: sys.node(b.1).priority,
            node: Some(b.1),
        },
    )
}

/// The natural candidate order: `priority = round(confidence·100)` descending, then
/// `tie_break` when it is `LowestCall`/`HighestCall` (`RowOrder`/`Narrowest` have nothing to
/// compare without nodes), then call index ascending. Identical to [`rank_cmp_keys`] on
/// node-less keys. `a`/`b` are `(call, priority)`.
pub fn natural_rank_cmp(tie_break: TieBreak, a: (Call, i16), b: (Call, i16)) -> Ordering {
    b.1.cmp(&a.1)
        .then_with(|| match tie_break {
            TieBreak::LowestCall => a.0.index().cmp(&b.0.index()),
            TieBreak::HighestCall => b.0.index().cmp(&a.0.index()),
            TieBreak::RowOrder | TieBreak::Narrowest => Ordering::Equal,
        })
        .then_with(|| a.0.index().cmp(&b.0.index()))
}

/// The top-level `Or` branches of `c` (what `interpret` turns into separate pieces), or `[c]`
/// for anything else (including an empty `Or`).
pub fn branches_of(c: &HandConstraint) -> Vec<&HandConstraint> {
    match c {
        HandConstraint::Or(branches) if !branches.is_empty() => branches.iter().collect(),
        other => vec![other],
    }
}

/// `true` for the empty `Or` (no hand satisfies it), which [`subtract`] returns when nothing is
/// left.
pub fn is_empty_or(c: &HandConstraint) -> bool {
    matches!(c, HandConstraint::Or(v) if v.is_empty())
}

/// The DNF atoms of `c` when its DNF is exact (no residual, no custom literal, no truncation).
fn exact_atoms(c: &HandConstraint) -> Option<Vec<Atom>> {
    let dnf = c
        .to_dnf(&DnfOptions {
            max_terms: 256,
            on_overflow: Overflow::Error,
        })
        .ok()?;
    if dnf.truncated || dnf.terms.iter().any(|t| !t.is_exact()) {
        return None;
    }
    Some(dnf.terms.into_iter().map(|t| t.atom).collect())
}

/// Merges atoms that differ only in their shape set (union) or only in adjacent/overlapping HCP
/// ranges (hull). Both preserve the set union and the pairwise disjointness of the input.
fn merge_atoms(mut atoms: Vec<Atom>) -> Vec<Atom> {
    loop {
        let mut changed = false;
        let mut out: Vec<Atom> = Vec::with_capacity(atoms.len());
        'next: for a in atoms {
            for b in out.iter_mut() {
                if b.hcp == a.hcp && b.cards == a.cards && b.eval == a.eval {
                    b.shapes = b.shapes.union(a.shapes);
                    changed = true;
                    continue 'next;
                }
                if b.shapes == a.shapes && b.cards == a.cards && b.eval == a.eval {
                    let (alo, ahi) = (*a.hcp.start(), *a.hcp.end());
                    let (blo, bhi) = (*b.hcp.start(), *b.hcp.end());
                    if alo <= bhi.saturating_add(1) && blo <= ahi.saturating_add(1) {
                        b.hcp = alo.min(blo)..=ahi.max(bhi);
                        changed = true;
                        continue 'next;
                    }
                }
            }
            out.push(a);
        }
        atoms = out;
        if !changed {
            return atoms;
        }
    }
}

/// `true` when `a ∧ b` is certainly empty by the shape/HCP summaries alone.
fn summaries_disjoint(a: &HandConstraint, b: &HandConstraint) -> bool {
    if a.shapes().intersect(b.shapes()).is_empty() {
        return true;
    }
    let (ra, rb) = (a.hcp_range(), b.hcp_range());
    ra.start().max(rb.start()) > ra.end().min(rb.end())
}

/// Exact atom-level subtraction `base ∖ (m1 ∪ … ∪ mk)` through the disjoint `Atom::negate`
/// chain, or `None` when some constraint has no exact DNF or the result exceeds
/// [`MAX_EXCLUSIVE_ATOMS`].
fn subtract_exact(base: &HandConstraint, minus: &[&HandConstraint]) -> Option<Vec<Atom>> {
    let mut terms = exact_atoms(base)?;
    // Disjointify the base's own DNF terms first, so the flat result is a disjoint `Or`.
    let mut disjoint: Vec<Atom> = Vec::with_capacity(terms.len());
    for t in terms.drain(..) {
        let mut pieces = vec![t];
        for d in &disjoint {
            let negated = d.negate();
            let mut next = Vec::new();
            for p in pieces {
                if p.intersect(d).is_trivially_unsat() {
                    next.push(p);
                    continue;
                }
                for n in &negated {
                    let x = p.intersect(n);
                    if !x.is_trivially_unsat() {
                        next.push(x);
                    }
                }
            }
            pieces = next;
        }
        disjoint.extend(pieces);
        if disjoint.len() > MAX_EXCLUSIVE_ATOMS {
            return None;
        }
    }
    terms = merge_atoms(disjoint);
    for m in minus {
        let m_atoms = exact_atoms(m)?;
        for d in &m_atoms {
            let mut negated: Option<Vec<Atom>> = None;
            let mut next = Vec::with_capacity(terms.len());
            for t in terms {
                if t.intersect(d).is_trivially_unsat() {
                    next.push(t);
                    continue;
                }
                let pieces = negated.get_or_insert_with(|| d.negate());
                for piece in pieces.iter() {
                    let x = t.intersect(piece);
                    if !x.is_trivially_unsat() {
                        next.push(x);
                    }
                }
            }
            terms = merge_atoms(next);
            if terms.len() > MAX_EXCLUSIVE_ATOMS {
                return None;
            }
            if terms.is_empty() {
                return Some(terms);
            }
        }
    }
    Some(terms)
}

/// `base ∧ ¬(m1 ∨ … ∨ mk)`, the same set of hands as [`subtract_tree`], in the flattest form
/// available: a flat `Or` of pairwise-disjoint atoms (a single atom when one is left; the empty
/// `Or`, see [`is_empty_or`], when nothing is left) computed by exact atom-level subtraction with
/// the disjoint `Atom::negate` chain, capped at [`MAX_EXCLUSIVE_ATOMS`] atoms. When some
/// constraint has no exact DNF (a `Custom` predicate) or the cap is exceeded, the result is the
/// tree `And([base, Not(Or(minus))])`. Members of `minus` whose summaries are disjoint from
/// `base` are dropped first (they cannot remove anything); with nothing left to subtract, `base`
/// is returned unchanged.
pub fn subtract(base: &HandConstraint, minus: &[&HandConstraint]) -> HandConstraint {
    let kept: Vec<&HandConstraint> = minus
        .iter()
        .copied()
        .filter(|m| !summaries_disjoint(base, m))
        .collect();
    if kept.is_empty() {
        return base.clone();
    }
    match subtract_exact(base, &kept) {
        Some(atoms) if atoms.len() == 1 => {
            HandConstraint::Atom(atoms.into_iter().next().expect("one atom"))
        }
        Some(atoms) => HandConstraint::Or(atoms.into_iter().map(HandConstraint::Atom).collect()),
        None => subtract_tree(base, &kept),
    }
}

/// `base ∧ ¬(m1 ∨ … ∨ mk)` as the tree `And([base, Not(Or(minus))])`, without any DNF work
/// (exact for `satisfies`; for run-time use where the flat subtraction would cost too much).
/// Members of `minus` whose summaries are disjoint from `base` are dropped first; with nothing
/// left, `base` is returned unchanged.
pub fn subtract_tree(base: &HandConstraint, minus: &[&HandConstraint]) -> HandConstraint {
    let mut kept = minus
        .iter()
        .copied()
        .filter(|m| !summaries_disjoint(base, m))
        .cloned();
    let Some(first) = kept.next() else {
        return base.clone();
    };
    let union = kept.fold(first, HandConstraint::or);
    HandConstraint::And(vec![base.clone(), union.not()])
}

/// The condition class of `(opener_pos, vul)`: `(opener_pos − 1) | we << 2 | they << 3`, one of
/// 16. It is exactly the unit by which `AuctionTrie::children` filters row entries by their
/// seat/vulnerability conditions, so one `(trie position, class)` pair has one sibling list.
pub fn condition_class(opener_pos: u8, vul: RelVul) -> u8 {
    (opener_pos.saturating_sub(1) & 3) | (u8::from(vul.we) << 2) | (u8::from(vul.they) << 3)
}

/// The inverse of [`condition_class`]: `(opener_pos, vul)` of a class `0..16`.
pub fn class_conditions(class: u8) -> (u8, RelVul) {
    (
        (class & 3) + 1,
        RelVul {
            we: class & 4 != 0,
            they: class & 8 != 0,
        },
    )
}

/// A precomputed shape/HCP summary of a piece: what Step B's summary-only pre-check and its
/// mass-ordered truncation read, so they never re-summarise a piece per combination.
#[derive(Clone, PartialEq, Debug)]
pub struct PieceSummary {
    /// `constraint.shapes()`.
    pub shapes: ShapeSet,
    /// `constraint.hcp_range()`.
    pub hcp: RangeInclusive<u8>,
    /// `ShapeSet::hcp_bounds` of `shapes`, or `None` when `shapes == ShapeSet::ALL` (every HCP
    /// value is then reachable by some shape).
    pub shape_hcp_bounds: Option<(u8, u8)>,
    /// `log2(|shapes| · |hcp|)`: the log size of the summary box in (shape, HCP) cells, the
    /// `volume_log2` of the mass-ordered truncation; `f32::NEG_INFINITY` for an empty box.
    pub volume_log2: f32,
}

impl PieceSummary {
    /// The summary of `c`.
    pub fn of(c: &HandConstraint) -> PieceSummary {
        let shapes = c.shapes();
        let hcp = c.hcp_range();
        let shape_hcp_bounds = (shapes != ShapeSet::ALL).then(|| shapes.hcp_bounds());
        let span = if hcp.is_empty() {
            0
        } else {
            u32::from(*hcp.end() - *hcp.start()) + 1
        };
        let cells = u32::from(shapes.len()) * span;
        let volume_log2 = if cells == 0 {
            f32::NEG_INFINITY
        } else {
            (cells as f32).log2()
        };
        PieceSummary {
            shapes,
            hcp,
            shape_hcp_bounds,
            volume_log2,
        }
    }

    /// `true` when the summary alone proves the piece empty: an empty shape set, an empty HCP
    /// range, or an HCP range no shape in the set can reach.
    pub fn is_empty(&self) -> bool {
        if self.shapes.is_empty() || self.hcp.is_empty() {
            return true;
        }
        match self.shape_hcp_bounds {
            Some((lo, hi)) => *self.hcp.start() > hi || *self.hcp.end() < lo,
            None => false,
        }
    }
}

/// One piece of a call's exclusive region: branch `branch` of `node`'s constraint, minus the
/// earlier branches of the same node and every higher-ranked member of the group.
#[derive(Clone, Debug)]
pub struct ExclusivePiece {
    /// The member's node.
    pub node: NodeId,
    /// Index of the top-level `Or` branch of the node's constraint ([`branches_of`] order).
    pub branch: u16,
    /// The region: a flat `Or` of disjoint atoms (or a single atom) when [`subtract`] could
    /// flatten it, otherwise the equivalent tree.
    pub constraint: HandConstraint,
    /// `true` when `constraint` is flat (atoms only), `false` for the tree fallback.
    pub flat: bool,
    /// Precomputed summary of `constraint`.
    pub summary: PieceSummary,
}

/// The siblings of one position under one condition class.
#[derive(Clone, Debug)]
pub struct ExclusiveGroup {
    /// Members `(call, node)`, best rank first ([`rank_cmp`] order).
    pub members: Vec<(Call, NodeId)>,
    /// Per call, in the rank order of each call's first member: the pairwise-disjoint pieces
    /// whose union is `X_c`. An empty list means the call is shadowed (the policy never makes
    /// it here).
    pub per_call: Vec<(Call, Vec<ExclusivePiece>)>,
    /// `¬(C_1 ∨ … ∨ C_n)` over every member: the hands with no system candidate here (what an
    /// implicit, unlisted `Pass` shows), flattened like a piece when possible.
    pub complement: HandConstraint,
    /// Precomputed summary of `complement`.
    pub complement_summary: PieceSummary,
}

impl ExclusiveGroup {
    /// Builds the group of `siblings` (any order; they are sorted by [`rank_cmp`] here). Used by
    /// the index build and by callers that must recompute a group from only the *legal*
    /// siblings at run time (a higher-ranked sibling that is illegal after the actual prefix).
    ///
    /// Each piece is [`subtract`]`(branch_j, [branch_0..j, every higher-ranked member])`: exact
    /// atom-level subtraction into pairwise-disjoint atoms, with the tree fallback past
    /// [`MAX_EXCLUSIVE_ATOMS`] atoms or with a `Custom` literal. Empty pieces are dropped.
    pub fn build(sys: &SystemIR, siblings: &[(Call, NodeId)]) -> ExclusiveGroup {
        let mut members = siblings.to_vec();
        members.sort_by(|a, b| rank_cmp(sys, *a, *b));
        let mut per_call: Vec<(Call, Vec<ExclusivePiece>)> = Vec::new();
        for (i, &(call, node)) in members.iter().enumerate() {
            let above: Vec<&HandConstraint> = members[..i]
                .iter()
                .map(|&(_, id)| &sys.node(id).constraint)
                .collect();
            let branches = branches_of(&sys.node(node).constraint);
            let mut pieces = Vec::new();
            for (j, branch) in branches.iter().enumerate() {
                let mut minus: Vec<&HandConstraint> = branches[..j].to_vec();
                minus.extend(above.iter().copied());
                let constraint = subtract(branch, &minus);
                if is_empty_or(&constraint) {
                    continue;
                }
                let summary = PieceSummary::of(&constraint);
                if summary.is_empty() {
                    continue;
                }
                pieces.push(ExclusivePiece {
                    node,
                    branch: j as u16,
                    flat: is_flat(&constraint),
                    constraint,
                    summary,
                });
            }
            match per_call.iter_mut().find(|(c, _)| *c == call) {
                Some((_, existing)) => existing.extend(pieces),
                None => per_call.push((call, pieces)),
            }
        }
        let all: Vec<&HandConstraint> = members
            .iter()
            .map(|&(_, id)| &sys.node(id).constraint)
            .collect();
        let complement = subtract(&HandConstraint::ANY, &all);
        let complement_summary = PieceSummary::of(&complement);
        ExclusiveGroup {
            members,
            per_call,
            complement,
            complement_summary,
        }
    }

    /// The pieces of `call` (`Some(&[])` when the call is a shadowed member, `None` when it is
    /// not a member at all).
    pub fn pieces(&self, call: Call) -> Option<&[ExclusivePiece]> {
        self.per_call
            .iter()
            .find(|(c, _)| *c == call)
            .map(|(_, pieces)| pieces.as_slice())
    }

    /// The rank (index into `members`) of `call`'s first member.
    pub fn rank_of(&self, call: Call) -> Option<usize> {
        self.members.iter().position(|(c, _)| *c == call)
    }

    /// `true` when `call` is a member whose exclusive region is empty.
    pub fn is_shadowed(&self, call: Call) -> bool {
        self.pieces(call).is_some_and(<[ExclusivePiece]>::is_empty)
    }
}

/// `true` for an atom or an `Or` of atoms.
fn is_flat(c: &HandConstraint) -> bool {
    match c {
        HandConstraint::Atom(_) => true,
        HandConstraint::Or(v) => v.iter().all(|x| matches!(x, HandConstraint::Atom(_))),
        _ => false,
    }
}

/// The derived index of exclusive regions of a [`SystemIR`]: one [`ExclusiveGroup`] per
/// `(parent trie position, condition class)` with at least one candidate, groups with identical
/// member lists shared.
#[derive(Clone, Debug, Default)]
pub struct ExclusiveIndex {
    /// `(parent trie position, condition class, group)`, sorted by the first two.
    keys: Vec<(u32, u8, u32)>,
    /// Deduplicated groups.
    groups: Vec<ExclusiveGroup>,
}

/// Counts describing a built [`ExclusiveIndex`] (diagnostics, compile tracing and the lane-S
/// acceptance report).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ExclusiveStats {
    /// Distinct groups after dedup.
    pub groups: usize,
    /// `(trie position, condition class)` keys.
    pub keys: usize,
    /// Distinct trie positions with at least one key.
    pub positions: usize,
    /// Member slots over all distinct groups.
    pub members: usize,
    /// Top-level `Or` branches over all members of all distinct groups.
    pub branches: usize,
    /// Non-empty pieces over all distinct groups.
    pub pieces: usize,
    /// Pieces kept as a tree (the [`subtract`] fallback) rather than a flat atom list.
    pub tree_pieces: usize,
    /// Atoms over all flat pieces.
    pub flat_atoms: usize,
    /// Groups whose complement is a tree.
    pub tree_complements: usize,
    /// Member calls with no piece at all (shadowed calls), over all distinct groups.
    pub shadowed_calls: usize,
}

impl ExclusiveIndex {
    /// Builds the index for every trie position of `sys` and every condition class with at
    /// least one candidate (A's algorithm: exact atom-level subtraction per piece, grouping by
    /// `(parent trie position, condition class)`, dedup of identical sibling lists).
    ///
    /// A position whose child entries carry no seat/vulnerability condition has the same
    /// sibling list under all 16 classes; it is computed once and keyed 16 times.
    pub fn build(sys: &SystemIR) -> ExclusiveIndex {
        let mut keys = Vec::new();
        let mut groups: Vec<ExclusiveGroup> = Vec::new();
        let mut seen: HashMap<Vec<(Call, NodeId)>, u32> = HashMap::new();
        let mut intern = |mut children: Vec<(Call, NodeId)>| -> u32 {
            children.sort_by(|a, b| rank_cmp(sys, *a, *b));
            match seen.get(&children) {
                Some(&g) => g,
                None => {
                    groups.push(ExclusiveGroup::build(sys, &children));
                    let g = (groups.len() - 1) as u32;
                    seen.insert(children, g);
                    g
                }
            }
        };
        for pos in 0..sys.index.len() as u32 {
            let at = TrieId(pos);
            if !sys.index.has_children(at) {
                continue;
            }
            if !sys.index.children_are_conditioned(at) {
                let (opener_pos, vul) = class_conditions(0);
                let children = sys.index.children(at, opener_pos, vul);
                if children.is_empty() {
                    continue;
                }
                let g = intern(children);
                keys.extend((0u8..16).map(|class| (pos, class, g)));
                continue;
            }
            for class in 0u8..16 {
                let (opener_pos, vul) = class_conditions(class);
                let children = sys.index.children(at, opener_pos, vul);
                if children.is_empty() {
                    continue;
                }
                keys.push((pos, class, intern(children)));
            }
        }
        keys.sort_unstable();
        ExclusiveIndex { keys, groups }
    }

    /// Every key: `(parent trie position, condition class, group)`, sorted by position then
    /// class.
    pub fn entries(&self) -> impl Iterator<Item = (TrieId, u8, &ExclusiveGroup)> {
        self.keys
            .iter()
            .map(|&(p, c, g)| (TrieId(p), c, &self.groups[g as usize]))
    }

    /// Counts describing the index (`sys` is the IR it was built from, for the branch counts).
    pub fn stats(&self, sys: &SystemIR) -> ExclusiveStats {
        let mut s = ExclusiveStats {
            groups: self.groups.len(),
            keys: self.keys.len(),
            ..ExclusiveStats::default()
        };
        let mut last = None;
        for &(p, _, _) in &self.keys {
            if last != Some(p) {
                s.positions += 1;
                last = Some(p);
            }
        }
        for g in &self.groups {
            s.members += g.members.len();
            s.branches += g
                .members
                .iter()
                .map(|&(_, node)| branches_of(&sys.node(node).constraint).len())
                .sum::<usize>();
            for (_, pieces) in &g.per_call {
                if pieces.is_empty() {
                    s.shadowed_calls += 1;
                }
                s.pieces += pieces.len();
                for p in pieces {
                    if p.flat {
                        s.flat_atoms += match &p.constraint {
                            HandConstraint::Or(v) => v.len(),
                            _ => 1,
                        };
                    } else {
                        s.tree_pieces += 1;
                    }
                }
            }
            if !is_flat(&g.complement) && !is_empty_or(&g.complement) {
                s.tree_complements += 1;
            }
        }
        s
    }

    /// The group at trie position `parent` for condition class `class` (see
    /// [`condition_class`]), `None` when that position has no candidate under the class.
    pub fn group(&self, parent: TrieId, class: u8) -> Option<&ExclusiveGroup> {
        self.keys
            .binary_search_by(|&(p, c, _)| (p, c).cmp(&(parent.0, class)))
            .ok()
            .map(|i| &self.groups[self.keys[i].2 as usize])
    }

    /// [`ExclusiveIndex::group`] for `(opener_pos, vul)`.
    pub fn group_for(
        &self,
        parent: TrieId,
        opener_pos: u8,
        vul: RelVul,
    ) -> Option<&ExclusiveGroup> {
        self.group(parent, condition_class(opener_pos, vul))
    }

    /// The pieces of `call` at `parent` under `(opener_pos, vul)`: typically `parent` is
    /// `Lookup::parent` of the lookup that matched `call`. `Some(&[])` for a shadowed call,
    /// `None` when there is no group or `call` is not a member.
    pub fn pieces(
        &self,
        parent: TrieId,
        opener_pos: u8,
        vul: RelVul,
        call: Call,
    ) -> Option<&[ExclusivePiece]> {
        self.group_for(parent, opener_pos, vul)?.pieces(call)
    }

    /// Number of distinct groups.
    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// Number of `(position, class)` keys.
    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// `true` when there is no group (a system without rows).
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The distinct groups.
    pub fn groups(&self) -> impl Iterator<Item = &ExclusiveGroup> {
        self.groups.iter()
    }
}

/// The lazily built [`ExclusiveIndex`] slot of a [`SystemIR`] (a `OnceLock`). Not serialised:
/// a deserialised or hand-built IR starts empty and builds on the first
/// [`SystemIR::exclusive`] call. Construct it with `ExclusiveCell::default()` in a `SystemIR`
/// struct literal.
#[derive(Clone, Default)]
pub struct ExclusiveCell(OnceLock<ExclusiveIndex>);

impl core::fmt::Debug for ExclusiveCell {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0.get() {
            Some(index) => write!(f, "ExclusiveCell(built, {} groups)", index.group_count()),
            None => f.write_str("ExclusiveCell(not built)"),
        }
    }
}

impl ExclusiveCell {
    /// The index if it has been built.
    pub fn get(&self) -> Option<&ExclusiveIndex> {
        self.0.get()
    }

    /// The index, building it from `sys` on first use. `sys` must be the IR that owns this cell.
    pub fn get_or_build(&self, sys: &SystemIR) -> &ExclusiveIndex {
        self.0.get_or_init(|| ExclusiveIndex::build(sys))
    }

    /// Stores an index built elsewhere (the eager build at the end of `compile()`); returns it
    /// back when the cell was already filled.
    pub fn set(&self, index: ExclusiveIndex) -> Result<(), ExclusiveIndex> {
        self.0.set(index)
    }

    /// Drops the built index, for a caller that mutates `nodes`/`index` after the first use.
    pub fn clear(&mut self) {
        self.0.take();
    }
}

#[cfg(test)]
mod tests {
    use bridge_constraint::Atom;
    use bridge_core::{Bid, Strain};

    use super::*;

    fn hcp(lo: u8, hi: u8) -> HandConstraint {
        HandConstraint::Atom(Atom::ANY.with_hcp(lo..=hi))
    }

    fn bal(lo: u8, hi: u8) -> HandConstraint {
        HandConstraint::Atom(Atom {
            shapes: ShapeSet::BALANCED,
            hcp: lo..=hi,
            cards: Vec::new(),
            eval: Vec::new(),
        })
    }

    fn bid(level: u8, strain: Strain) -> Call {
        Call::Bid(Bid::new(level, strain).unwrap())
    }

    #[test]
    fn subtract_is_exact_and_flat_for_literal_free_constraints() {
        let base = hcp(12, 21);
        let minus = bal(15, 17);
        let x = subtract(&base, &[&minus]);
        assert!(is_flat(&x));
        let g = bridge_constraint::HcpShapeGrid::of_exact(&x).unwrap();
        let want = bridge_constraint::HcpShapeGrid::of_exact(&base)
            .unwrap()
            .diff(&bridge_constraint::HcpShapeGrid::of_exact(&minus).unwrap());
        assert_eq!(g, want);
        // Pairwise-disjoint atoms.
        if let HandConstraint::Or(atoms) = &x {
            for (i, a) in atoms.iter().enumerate() {
                for b in &atoms[i + 1..] {
                    let (ga, gb) = (
                        bridge_constraint::HcpShapeGrid::of_exact(a).unwrap(),
                        bridge_constraint::HcpShapeGrid::of_exact(b).unwrap(),
                    );
                    assert!(!ga.intersects(&gb));
                }
            }
        }
    }

    #[test]
    fn subtract_of_a_covering_sibling_is_empty_and_disjoint_minus_is_a_no_op() {
        assert!(is_empty_or(&subtract(&bal(15, 17), &[&hcp(10, 20)])));
        let base = hcp(0, 11);
        let x = subtract(&base, &[&hcp(12, 37)]);
        assert!(matches!(x, HandConstraint::Atom(a) if a.hcp == (0..=11)));
        let t = subtract_tree(&base, &[&hcp(20, 37)]);
        assert!(matches!(t, HandConstraint::Atom(_)));
        let t = subtract_tree(&hcp(10, 20), &[&hcp(15, 37)]);
        assert!(matches!(t, HandConstraint::And(_)));
    }

    #[test]
    fn natural_rank_order_is_priority_then_tie_break_then_call_index() {
        let (a, b) = (bid(1, Strain::Clubs), bid(1, Strain::Spades));
        // Higher priority first.
        assert_eq!(
            natural_rank_cmp(TieBreak::RowOrder, (b, 60), (a, 50)),
            Ordering::Less
        );
        // Equal priority: RowOrder/Narrowest fall through to call index ascending.
        assert_eq!(
            natural_rank_cmp(TieBreak::RowOrder, (a, 50), (b, 50)),
            Ordering::Less
        );
        assert_eq!(
            natural_rank_cmp(TieBreak::Narrowest, (b, 50), (a, 50)),
            Ordering::Greater
        );
        // HighestCall reverses the call order.
        assert_eq!(
            natural_rank_cmp(TieBreak::HighestCall, (b, 50), (a, 50)),
            Ordering::Less
        );
        assert_eq!(
            natural_rank_cmp(TieBreak::LowestCall, (a, 50), (b, 50)),
            Ordering::Less
        );
    }

    #[test]
    fn condition_class_round_trips() {
        for class in 0u8..16 {
            let (pos, vul) = class_conditions(class);
            assert_eq!(condition_class(pos, vul), class);
        }
    }

    #[test]
    fn piece_summary_detects_empty_boxes() {
        assert!(PieceSummary::of(&HandConstraint::Or(Vec::new())).is_empty());
        let s = PieceSummary::of(&bal(15, 17));
        assert!(!s.is_empty());
        assert!(s.volume_log2 > 0.0);
    }
}
