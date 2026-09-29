//! The policy mirror's regions (docs/design/15-phase4-plan.md D19; 07-bidding.md §4.1).
//!
//! For call `c` at position `P` (prefix, acting seat `s`, `n` legal calls) under the policy
//! `p(c|h) = (1 − ε)·[(1 − δ)·S + δ·M] + ε/n` (`policy.rs`), the hand space splits into pieces on
//! which `p(c|h)` is constant:
//!
//! | piece | set | raw weight |
//! | --- | --- | --- |
//! | `X_c` (per branch) | the system exclusive region: the first satisfied member has call `c` | `(1 − ε)(1 − δ)` |
//! | `N_sys` | no system candidate at all (only without the implicit pass) | `(1 − ε)(1 − δ)/n` |
//! | `Y_c` | the natural exclusive region (and the natural implicit pass for `Pass`) | `(1 − ε)δ`, or `(1 − ε)` off-system |
//! | `N_nat` | no natural candidate (only without the natural implicit pass) | `(1 − ε)δ/n`, or `(1 − ε)/n` off-system |
//! | `ANY` | every hand | `ε/n` |
//!
//! The system pieces come from the precomputed [`bridge_system::ExclusiveIndex`] (borrowed, no
//! allocation). When a system member ranked above one of `c`'s members is illegal after the
//! actual prefix (a lenient match, a wildcard subtree), `c`'s pieces are recomputed at run time
//! from the legal members only, with the same exact subtraction. The natural pieces are computed
//! at run time on the exact shape × HCP grid ([`HcpShapeGrid`]) in two forms: a flat
//! over-covering form (for the proposal; a higher-ranked candidate with `cards`/`eval` literals
//! is subtracted through its guaranteed subset `sub`) and, when that is not exact, an exact tree
//! form for membership and likelihood.
//!
//! [`Reader`] computes the per-call reading `partner_context` needs (07-bidding.md §4.1,
//! "パートナー文脈"): the shape/HCP summary of the call's system region, or of its natural
//! inference when the call has no system region. `choose_bid`'s natural branch and `interpret`
//! both go through it, so they fill `CallContext::partner_constraint`/`forcing_situation`
//! identically.

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

use bridge_constraint::grid::bounds;
use bridge_constraint::{Atom, HandConstraint, HcpShapeGrid};
use bridge_core::{Auction, Call, ShapeSet};
use bridge_system::exclusive::{
    ExclusivePiece, PieceSummary, branches_of, is_empty_or, subtract, subtract_tree,
};
use bridge_system::{Forcing, NaturalCandidate, NaturalInference, PartnerContext};
use smallvec::SmallVec;

use crate::choose::{Position, enumerate_position, ranked_legal};
use crate::memo;
use crate::{ImplicitPass, NodeId, PolicyParams, ResolutionKind, Table};

/// Cap on the atoms of a flat natural piece; a grid with more HCP runs is widened (merged runs,
/// over-covering) and keeps its exact tree form for membership.
const NATURAL_ATOM_CAP: usize = 32;

/// The role of a mirror piece.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PieceRole {
    /// A branch of the system exclusive region `X_c` (or the implicit-pass complement).
    System,
    /// `N_sys`: no system candidate.
    NoSystem,
    /// `Y_c`: the natural exclusive region.
    Natural,
    /// `N_nat`: no natural candidate.
    NoNatural,
    /// `ANY`: the uniform floor.
    Any,
}

impl PieceRole {
    /// Whether the piece is a `Fallback` piece (dropped under `strict`).
    pub(crate) fn is_fallback(self) -> bool {
        matches!(
            self,
            PieceRole::NoSystem | PieceRole::NoNatural | PieceRole::Any
        )
    }
}

/// One piece of a call's mirror.
#[derive(Clone, Debug)]
pub(crate) struct MirrorPiece<'a> {
    pub(crate) role: PieceRole,
    /// The system node of an `X` piece.
    pub(crate) node: Option<NodeId>,
    /// The raw weight (the policy probability on the piece, before normalisation).
    pub(crate) raw: f64,
    /// The proposal form: a superset of the exact region (equal to it unless `exact` is set).
    pub(crate) flat: Cow<'a, HandConstraint>,
    /// The exact region when `flat` over-covers it (kept only under [`MirrorSpec::membership`]).
    pub(crate) exact: Option<HandConstraint>,
    /// The exact region as a grid, when it is literal-free and was computed on the grid (kept
    /// only under [`MirrorSpec::membership`]).
    pub(crate) grid: Option<Box<HcpShapeGrid>>,
    /// Summary of `flat`.
    pub(crate) summary: Cow<'a, PieceSummary>,
}

impl MirrorPiece<'_> {
    /// The exact membership form.
    pub(crate) fn membership(&self) -> &HandConstraint {
        self.exact.as_ref().unwrap_or(&self.flat)
    }
}

/// The mirror of one call.
#[derive(Clone, Debug)]
pub(crate) struct CallMirror<'a> {
    /// The pieces (pairwise disjoint as exact regions, except `ANY`, which covers everything).
    pub(crate) pieces: SmallVec<[MirrorPiece<'a>; 4]>,
    /// How the call resolves: `Exact`/`Partial` at an on-system position where the call has a
    /// system reading (or is a shadowed system member), else `Natural`.
    pub(crate) kind: ResolutionKind,
    /// The policy never makes the call here (no `X` and no `Y` piece).
    pub(crate) shadowed: bool,
    /// The node the call's system reading is about (first member of the call), if any.
    pub(crate) node: Option<NodeId>,
    /// Explanation text of the non-`Fallback` pieces (node description or natural rule).
    pub(crate) text: String,
}

/// The system region of a call at a position.
enum XRegion<'a> {
    /// A member call: its exclusive pieces (possibly none: shadowed).
    Pieces(Cow<'a, [ExclusivePiece]>),
    /// The synthesised implicit pass: the complement of every legal member.
    ImplicitPass(Cow<'a, HandConstraint>, Cow<'a, PieceSummary>),
}

/// `call`'s exclusive pieces among the legal members of `pos`, recomputed at run time (a member
/// ranked above one of `call`'s members is illegal here).
fn recompute_pieces(pos: &Position<'_>, call: Call) -> Vec<ExclusivePiece> {
    let sys = pos.system;
    let members = ranked_legal(pos);
    let mut out = Vec::new();
    for (i, &(c, node)) in members.iter().enumerate() {
        if c != call {
            continue;
        }
        let above: Vec<&HandConstraint> = members[..i]
            .iter()
            .map(|&(_, id)| &sys.node(id).constraint)
            .collect();
        let branches = branches_of(&sys.node(node).constraint);
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
            let flat = match &constraint {
                HandConstraint::Atom(_) => true,
                HandConstraint::Or(v) => v.iter().all(|x| matches!(x, HandConstraint::Atom(_))),
                _ => false,
            };
            out.push(ExclusivePiece {
                node,
                branch: j as u16,
                constraint,
                flat,
                summary,
            });
        }
    }
    out
}

/// The complement of the legal members of `pos` (no system candidate satisfied), from the index
/// when every member is legal, else recomputed.
fn complement<'a>(pos: &Position<'a>) -> Option<(Cow<'a, HandConstraint>, Cow<'a, PieceSummary>)> {
    let group = pos.group()?;
    if !pos.any_illegal {
        return Some((
            Cow::Borrowed(&group.complement),
            Cow::Borrowed(&group.complement_summary),
        ));
    }
    let sys = pos.system;
    let legal: Vec<&HandConstraint> = pos
        .legal()
        .map(|(_, id)| &sys.node(id).constraint)
        .collect();
    let c = subtract(&HandConstraint::ANY, &legal);
    let s = PieceSummary::of(&c);
    Some((Cow::Owned(c), Cow::Owned(s)))
}

/// The system region of `call` at `pos` (`None` off-system, or when `call` is neither a legal
/// member nor the synthesised implicit pass).
fn system_x<'a>(pos: &Position<'a>, call: Call) -> Option<XRegion<'a>> {
    if !pos.on_system() {
        return None;
    }
    let group = pos.group()?;
    if pos.children.iter().any(|&(c, _, legal)| legal && c == call) {
        let borrowed = || XRegion::Pieces(Cow::Borrowed(group.pieces(call).unwrap_or(&[])));
        if !pos.any_illegal {
            return Some(borrowed());
        }
        let rank = |m: (Call, NodeId)| group.members.iter().position(|&x| x == m);
        let first_illegal = pos
            .children
            .iter()
            .filter(|&&(_, _, legal)| !legal)
            .filter_map(|&(c, n, _)| rank((c, n)))
            .min()
            .unwrap_or(usize::MAX);
        let last_own = group
            .members
            .iter()
            .rposition(|&(c, _)| c == call)
            .unwrap_or(0);
        if last_own < first_illegal {
            return Some(borrowed());
        }
        return Some(XRegion::Pieces(Cow::Owned(recompute_pieces(pos, call))));
    }
    if call == Call::Pass && pos.implicit_pass {
        let (c, s) = complement(pos)?;
        return Some(XRegion::ImplicitPass(c, s));
    }
    None
}

/// The shape/HCP summary atom of a set of summaries (union of shapes, hull of HCP).
fn summary_atom<'s>(summaries: impl Iterator<Item = &'s PieceSummary>) -> Option<HandConstraint> {
    let mut shapes = ShapeSet::EMPTY;
    let mut lo = u8::MAX;
    let mut hi = 0u8;
    for s in summaries {
        if s.is_empty() {
            continue;
        }
        shapes = shapes.union(s.shapes);
        lo = lo.min(*s.hcp.start());
        hi = hi.max(*s.hcp.end());
    }
    (lo <= hi && !shapes.is_empty()).then(|| {
        HandConstraint::Atom(Atom {
            shapes,
            hcp: lo..=hi,
            cards: Vec::new(),
            eval: Vec::new(),
        })
    })
}

/// What one call tells partner (the input of `partner_context`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Reading {
    /// Shape/HCP summary of the call's system region, or of its natural inference; `None` when
    /// neither says anything (the natural `fallback` rule).
    pub(crate) summary: Option<HandConstraint>,
    /// The system node (for the forcing flag), when the reading is a system one.
    pub(crate) node: Option<NodeId>,
}

/// The reading of `call` from its system region, when it has a non-empty one.
fn system_reading(x: &XRegion<'_>) -> Option<Reading> {
    match x {
        XRegion::Pieces(pieces) => {
            let first = pieces.first()?;
            Some(Reading {
                summary: summary_atom(pieces.iter().map(|p| &p.summary)),
                node: Some(first.node),
            })
        }
        XRegion::ImplicitPass(_, s) => {
            if s.is_empty() {
                return None;
            }
            Some(Reading {
                summary: summary_atom(std::iter::once(s.as_ref())),
                node: None,
            })
        }
    }
}

/// The reading of a natural inference (`None` summary for the `fallback` rule).
fn natural_reading(cand: &NaturalCandidate) -> Reading {
    let summary = if cand.is_fallback() {
        None
    } else {
        summary_atom(std::iter::once(&PieceSummary::of(&cand.constraint)))
    };
    Reading {
        summary,
        node: None,
    }
}

/// Computes (and memoises) the per-call [`Reading`]s of an auction and the partner context of
/// each position, lazily: only the chain of partner calls a natural computation actually needs
/// is read.
pub(crate) struct Reader<'t> {
    table: &'t Table,
    natural: &'t NaturalInference,
    auction: &'t Auction,
    implicit_pass: ImplicitPass,
    memo: Vec<Option<Reading>>,
}

impl<'t> Reader<'t> {
    /// A reader of `auction`'s calls under `table`, with `natural` for calls that have no system
    /// region.
    pub(crate) fn new(
        table: &'t Table,
        natural: &'t NaturalInference,
        auction: &'t Auction,
        implicit_pass: ImplicitPass,
    ) -> Reader<'t> {
        Reader {
            table,
            natural,
            auction,
            implicit_pass,
            memo: vec![None; auction.len()],
        }
    }

    /// The prefix `auction[..j]`.
    fn prefix(&self, j: usize) -> Auction {
        Auction::from_calls(
            self.auction.dealer(),
            self.auction.vulnerability(),
            self.auction.calls()[..j].iter().copied(),
        )
        .expect("a prefix of a legal auction is legal")
    }

    /// The reading of call `j`.
    pub(crate) fn reading(&mut self, j: usize) -> Reading {
        if let Some(r) = &self.memo[j] {
            return r.clone();
        }
        let prefix = self.prefix(j);
        let call = self.auction.calls()[j];
        let pos = enumerate_position(self.table, &prefix, self.implicit_pass);
        let reading = match system_x(&pos, call).as_ref().and_then(system_reading) {
            Some(r) => r,
            None => {
                match peek_natural_position(self.table, self.natural, &prefix, self.implicit_pass) {
                    Some(np) => np
                        .ranked
                        .iter()
                        .find(|c| c.call == call)
                        .map(natural_reading)
                        .unwrap_or_default(),
                    None => {
                        let partner = self.partner_context(j);
                        let cand = self
                            .natural
                            .infer_batch(&prefix, pos.seat, &partner, &[call])
                            .pop()
                            .expect("one result per call");
                        natural_reading(&cand)
                    }
                }
            }
        };
        self.memo[j] = Some(reading.clone());
        reading
    }

    /// The partner context of the seat making call `j` (`j == auction.len()`: the next call):
    /// partner's most recent call's [`Reading`], and whether it is a forcing system call with no
    /// opponents' action since (an opponent's bid, double or redouble after partner's forcing
    /// call releases the obligation to bid).
    pub(crate) fn partner_context(&mut self, j: usize) -> PartnerContext {
        let seat = self.auction.seat_at(j);
        let partner = seat.partner();
        let Some(k) = (0..j).rev().find(|&k| self.auction.seat_at(k) == partner) else {
            return PartnerContext::default();
        };
        let intervened = (k + 1..j).any(|i| {
            self.auction.seat_at(i).side() != seat.side() && self.auction.calls()[i] != Call::Pass
        });
        let reading = self.reading(k);
        let forcing = match reading.node {
            Some(node) if !intervened => {
                let flags = &self.table.systems[partner.index() as usize]
                    .node(node)
                    .flags;
                matches!(flags.forcing, Forcing::OneRound | Forcing::ToGame)
            }
            _ => false,
        };
        PartnerContext {
            partner_constraint: reading.summary,
            forcing_situation: forcing,
        }
    }
}

/// The partner context of the seat about to call after `auction` (see [`Reader`]).
pub(crate) fn partner_context(
    table: &Table,
    natural: &NaturalInference,
    auction: &Auction,
    implicit_pass: ImplicitPass,
) -> PartnerContext {
    Reader::new(table, natural, auction, implicit_pass).partner_context(auction.len())
}

/// A natural piece in both forms.
struct NaturalPiece {
    flat: HandConstraint,
    exact: Option<HandConstraint>,
    grid: Option<Box<HcpShapeGrid>>,
    summary: PieceSummary,
}

/// `grid` as a flat constraint (one atom per HCP run, carrying `own`'s literals when `own` is a
/// literal atom), whether that is exact, and the summary; `None` when the grid holds no hand.
fn grid_piece(
    grid: &HcpShapeGrid,
    exact: bool,
    own: Option<&HandConstraint>,
    tree: impl FnOnce() -> HandConstraint,
) -> Option<NaturalPiece> {
    if grid.is_empty_hands() {
        return None;
    }
    let runs = grid.runs();
    let widened = runs.len() > NATURAL_ATOM_CAP;
    let template = match own {
        Some(HandConstraint::Atom(a)) => a.clone(),
        _ => Atom::ANY,
    };
    let atoms = if widened {
        grid.to_atoms(&template, NATURAL_ATOM_CAP)
    } else {
        runs.into_iter()
            .filter_map(|(shapes, hcp)| {
                let shapes = shapes.intersect(template.shapes);
                let lo = (*hcp.start()).max(*template.hcp.start());
                let hi = (*hcp.end()).min(*template.hcp.end());
                (!shapes.is_empty() && lo <= hi).then(|| Atom {
                    shapes,
                    hcp: lo..=hi,
                    cards: template.cards.clone(),
                    eval: template.eval.clone(),
                })
            })
            .collect()
    };
    let mut flat = match atoms.len() {
        0 => return None,
        1 => HandConstraint::Atom(atoms.into_iter().next().expect("one atom")),
        _ => HandConstraint::Or(atoms.into_iter().map(HandConstraint::Atom).collect()),
    };
    // An own constraint with literals that is not a single atom stays a conjunct.
    if let Some(own) = own {
        if !matches!(own, HandConstraint::Atom(_)) {
            flat = HandConstraint::And(vec![flat, own.clone()]);
        }
    }
    let exact_form = (!exact || widened).then(tree);
    let summary = PieceSummary::of(&flat);
    let grid = (exact && !widened && own.is_none()).then(|| Box::new(*grid));
    Some(NaturalPiece {
        flat,
        exact: exact_form,
        grid,
        summary,
    })
}

/// `Or` of `cs` (`None` for an empty list).
fn or_of(cs: &[&HandConstraint]) -> Option<HandConstraint> {
    match cs.len() {
        0 => None,
        1 => Some(cs[0].clone()),
        _ => Some(HandConstraint::Or(cs.iter().map(|&c| c.clone()).collect())),
    }
}

/// Adds the exact region of `c` to `boxes` as `(hcp lo, hcp hi, shapes)` boxes when `c` is a
/// literal-free atom, an `And` of literal-free atoms (one box: the shapes intersected, the HCP
/// ranges intersected), or an `Or` of such constraints; returns `false` (and adds nothing)
/// otherwise. Boxes with the same HCP range are merged by uniting their shapes. This is the
/// common form of natural candidates (a rule's atom, conjoined with the level floor's HCP atom
/// at high levels), and it avoids building and intersecting per-candidate grids.
fn push_boxes(c: &HandConstraint, boxes: &mut SmallVec<[(u8, u8, ShapeSet); 8]>) -> bool {
    fn push(boxes: &mut SmallVec<[(u8, u8, ShapeSet); 8]>, lo: u8, hi: u8, shapes: ShapeSet) {
        if lo > hi || shapes.is_empty() {
            return;
        }
        match boxes.iter_mut().find(|b| b.0 == lo && b.1 == hi) {
            Some(b) => b.2 = b.2.union(shapes),
            None => boxes.push((lo, hi, shapes)),
        }
    }
    let literal_free = |a: &Atom| a.cards.is_empty() && a.eval.is_empty();
    match c {
        HandConstraint::Atom(a) if literal_free(a) => {
            push(boxes, *a.hcp.start(), *a.hcp.end(), a.shapes);
            true
        }
        HandConstraint::And(children) => {
            let (mut lo, mut hi, mut shapes) = (0u8, u8::MAX, ShapeSet::ALL);
            for child in children {
                match child {
                    HandConstraint::Atom(a) if literal_free(a) => {
                        lo = lo.max(*a.hcp.start());
                        hi = hi.min(*a.hcp.end());
                        shapes = shapes.intersect(a.shapes);
                    }
                    _ => return false,
                }
            }
            push(boxes, lo, hi, shapes);
            true
        }
        HandConstraint::Or(children) => {
            let mut own = SmallVec::new();
            if !children.iter().all(|child| push_boxes(child, &mut own)) {
                return false;
            }
            for (lo, hi, shapes) in own {
                push(boxes, lo, hi, shapes);
            }
            true
        }
        _ => false,
    }
}

/// The grid of `boxes` (see [`push_boxes`]) united with `grid`.
fn or_boxes(grid: HcpShapeGrid, boxes: &[(u8, u8, ShapeSet)]) -> HcpShapeGrid {
    boxes.iter().fold(grid, |g, &(lo, hi, shapes)| {
        g.or(&HcpShapeGrid::from_box(shapes, lo..=hi))
    })
}

/// The union of the guaranteed subsets (`sub` of [`bounds`]) of `cs`, and whether every one is
/// exact. Literal-free atoms and `And`/`Or` combinations of them (almost every natural
/// inference) are merged per HCP range into one box each ([`push_boxes`]), so no per-candidate
/// grid is built.
fn union_sub<'c>(cs: impl IntoIterator<Item = &'c HandConstraint>) -> (HcpShapeGrid, bool) {
    let mut boxes: SmallVec<[(u8, u8, ShapeSet); 8]> = SmallVec::new();
    let mut grid: Option<HcpShapeGrid> = None;
    let mut exact = true;
    for c in cs {
        if push_boxes(c, &mut boxes) {
            continue;
        }
        match c {
            // An atom with literals: `sub = ∅`.
            HandConstraint::Atom(_) => exact = false,
            _ => {
                let b = bounds(c);
                exact &= b.is_exact();
                grid = Some(match grid {
                    Some(g) => g.or(&b.sub),
                    None => b.sub,
                });
            }
        }
    }
    (or_boxes(grid.unwrap_or(HcpShapeGrid::EMPTY), &boxes), exact)
}

/// The guaranteed superset (`sup` of [`bounds`]) of `c`, and whether it is exact.
fn sup_of(c: &HandConstraint) -> (HcpShapeGrid, bool) {
    let mut boxes = SmallVec::new();
    if push_boxes(c, &mut boxes) {
        return (or_boxes(HcpShapeGrid::EMPTY, &boxes), true);
    }
    match c {
        HandConstraint::Atom(a) => (
            HcpShapeGrid::of_atom_box(a),
            a.cards.is_empty() && a.eval.is_empty(),
        ),
        _ => {
            let b = bounds(c);
            let exact = b.is_exact();
            (b.sup, exact)
        }
    }
}

/// The natural regions of `call` among `ranked` (rank order): `Y_c` and, without the natural
/// implicit pass, `N_nat`.
fn natural_regions(
    ranked: &[NaturalCandidate],
    call: Call,
    implicit_pass: ImplicitPass,
) -> (Option<NaturalPiece>, Option<NaturalPiece>) {
    let constraints: Vec<&HandConstraint> = ranked.iter().map(|c| &c.constraint).collect();
    let idx = ranked.iter().position(|c| c.call == call);
    let natural_pass = implicit_pass == ImplicitPass::Complement;

    let y = if call == Call::Pass && natural_pass {
        // The natural implicit pass: the first satisfied candidate is `Pass`, or none is.
        let (above_c, own_c, below_c) = match idx {
            Some(i) => (
                &constraints[..i],
                Some(constraints[i]),
                &constraints[i + 1..],
            ),
            None => (&constraints[..], None, &constraints[..0]),
        };
        let (above, above_exact) = union_sub(above_c.iter().copied());
        let (below, below_exact) = union_sub(below_c.iter().copied());
        let none_below = below.not();
        let (inner, own_exact) = match own_c {
            Some(c) => {
                let (sup, exact) = sup_of(c);
                (sup.or(&none_below), exact)
            }
            None => (none_below, true),
        };
        let grid = above.not().and(&inner);
        let exact = above_exact && own_exact && below_exact;
        grid_piece(&grid, exact, None, || {
            let not_below = or_of(below_c).map(|u| u.not());
            let inner = match (own_c, not_below) {
                (Some(c), Some(nb)) => Some(HandConstraint::Or(vec![c.clone(), nb])),
                (Some(c), None) => Some(c.clone()),
                (None, nb) => nb,
            };
            let not_above = or_of(above_c).map(|u| u.not());
            match (not_above, inner) {
                (Some(a), Some(b)) => HandConstraint::And(vec![a, b]),
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (None, None) => HandConstraint::ANY,
            }
        })
    } else if let Some(i) = idx {
        let own_c = &ranked[i].constraint;
        let (own_sup, own_exact) = sup_of(own_c);
        let (above, exact_above) = union_sub(constraints[..i].iter().copied());
        let grid = own_sup.and(&above.not());
        let own = (!own_exact).then_some(own_c);
        grid_piece(&grid, exact_above, own, || {
            subtract_tree(own_c, &constraints[..i])
        })
    } else {
        None
    };

    let n = if natural_pass {
        None
    } else {
        let (all, exact) = union_sub(constraints.iter().copied());
        grid_piece(&all.not(), exact, None, || {
            or_of(&constraints).map_or(HandConstraint::ANY, |u| u.not())
        })
    };
    (y, n)
}

/// The hand-independent natural data of one position (prefix): the ranked natural candidates
/// under the partner context of the acting seat, and, per call, its natural regions and
/// explanation text (computed on first use). Shared by `choose_bid`'s natural branch,
/// `call_distribution` and the mirror through [`natural_position`].
pub(crate) struct NaturalPos {
    /// The natural candidates in rank order.
    pub(crate) ranked: Vec<NaturalCandidate>,
    /// The partner context they were inferred under.
    pub(crate) partner: PartnerContext,
    implicit_pass: ImplicitPass,
    regions: RefCell<Vec<(Call, Rc<NaturalCall>)>>,
    texts: RefCell<Vec<(Call, String)>>,
}

/// The natural regions of one call at a [`NaturalPos`].
struct NaturalCall {
    y: Option<NaturalPiece>,
    n: Option<NaturalPiece>,
}

impl NaturalPos {
    fn compute(
        table: &Table,
        natural: &NaturalInference,
        prefix: &Auction,
        implicit_pass: ImplicitPass,
        partner: PartnerContext,
    ) -> NaturalPos {
        let seat = prefix.next_seat();
        let tie_break = table.systems[seat.index() as usize].meta.tie_break;
        let ranked = natural.ranked_candidates(prefix, seat, &partner, tie_break);
        NaturalPos {
            ranked,
            partner,
            implicit_pass,
            regions: RefCell::new(Vec::new()),
            texts: RefCell::new(Vec::new()),
        }
    }

    /// `Y_c` and `N_nat` of `call` (memoised).
    fn regions(&self, call: Call) -> Rc<NaturalCall> {
        if let Some((_, r)) = self.regions.borrow().iter().find(|(c, _)| *c == call) {
            return r.clone();
        }
        let (y, n) = natural_regions(&self.ranked, call, self.implicit_pass);
        let r = Rc::new(NaturalCall { y, n });
        self.regions.borrow_mut().push((call, r.clone()));
        r
    }

    /// The natural explanation of `call` after `prefix` (this position), memoised.
    pub(crate) fn text(&self, natural: &NaturalInference, prefix: &Auction, call: Call) -> String {
        if let Some((_, t)) = self.texts.borrow().iter().find(|(c, _)| *c == call) {
            return t.clone();
        }
        let t = natural_text(natural, prefix, call, &self.partner);
        self.texts.borrow_mut().push((call, t.clone()));
        t
    }
}

/// The memoised [`NaturalPos`] of the position after `prefix`, if any (never computes).
fn peek_natural_position(
    table: &Table,
    natural: &NaturalInference,
    prefix: &Auction,
    implicit_pass: ImplicitPass,
) -> Option<Rc<NaturalPos>> {
    if !std::ptr::eq(natural, table.natural.as_ref()) {
        return None;
    }
    let entry = memo::peek(table, prefix, implicit_pass)?;
    entry.natural.borrow().clone()
}

/// The [`NaturalPos`] of `pos` (the position after `prefix`): memoised with the position
/// ([`crate::memo`]) when `natural` is the table's own engine, else computed. `partner()` must
/// be `partner_context(table, natural, prefix, implicit_pass)`; it is only called on a miss.
pub(crate) fn natural_position(
    table: &Table,
    pos: &Position<'_>,
    natural: &NaturalInference,
    prefix: &Auction,
    implicit_pass: ImplicitPass,
    partner: impl FnOnce() -> PartnerContext,
) -> Rc<NaturalPos> {
    let own = std::ptr::eq(natural, table.natural.as_ref());
    if own {
        if let Some(np) = pos.entry().natural.borrow().as_ref() {
            return np.clone();
        }
    }
    // Computed outside any borrow: the partner context reads other entries.
    let np = Rc::new(NaturalPos::compute(
        table,
        natural,
        prefix,
        implicit_pass,
        partner(),
    ));
    if own {
        *pos.entry().natural.borrow_mut() = Some(np.clone());
    }
    np
}

/// What [`mirror_call`] needs besides the position.
pub(crate) struct MirrorSpec<'t> {
    pub(crate) table: &'t Table,
    pub(crate) natural: &'t NaturalInference,
    pub(crate) policy: PolicyParams,
    pub(crate) implicit_pass: ImplicitPass,
    /// Drop the `Fallback` pieces (`N_sys`, `N_nat`, `ANY`).
    pub(crate) strict: bool,
    /// Build the explanation text.
    pub(crate) want_text: bool,
    /// Keep the exact membership forms of the natural pieces (their exact trees and grids, for
    /// [`crate::AuctionPolicy`]); without it a piece carries its flat (proposal) form only.
    pub(crate) membership: bool,
}

/// The natural explanation text of `call` after `prefix` with `partner`'s context.
fn natural_text(
    natural: &NaturalInference,
    prefix: &Auction,
    call: Call,
    partner: &PartnerContext,
) -> String {
    let Ok(next) = prefix.with(call) else {
        return String::new();
    };
    let mut ctx = bridge_system::natural::classify(&next, prefix.len(), prefix.next_seat());
    ctx.partner_constraint = partner.partner_constraint.clone();
    ctx.forcing_situation = partner.forcing_situation;
    let inf = natural.infer(&ctx);
    format!("{} ({})", inf.explanation, inf.rule)
}

/// The mirror of call `j` of `reader`'s auction, made after `prefix` (`== auction[..j]`).
pub(crate) fn mirror_call<'t>(
    spec: &MirrorSpec<'t>,
    reader: &mut Reader<'t>,
    prefix: &Auction,
    call: Call,
) -> CallMirror<'t> {
    let j = prefix.len();
    let pos = enumerate_position(spec.table, prefix, spec.implicit_pass);
    let n = pos.n_legal.max(1) as f64;
    let eps = f64::from(spec.policy.epsilon);
    let delta = f64::from(spec.policy.deviation);
    let mut pieces: SmallVec<[MirrorPiece<'t>; 4]> = SmallVec::new();
    let mut kind = ResolutionKind::Natural;
    let mut node: Option<NodeId> = None;
    let mut text = String::new();
    let mut has_x = false;
    let mut has_y = false;

    let on_system = pos.on_system();
    let (nat_w, nat_none_w) = if on_system {
        let raw_sys = (1.0 - eps) * (1.0 - delta);
        // The call's reading (`Reader::reading`) is computed lazily, only when a later natural
        // position needs it as partner context and its natural data is not memoised yet.
        let x = system_x(&pos, call);
        let is_member = pos.children.iter().any(|&(c, _, legal)| legal && c == call);
        if is_member || x.is_some() {
            kind = pos.system_kind();
        }
        if is_member {
            node = pos
                .group()
                .and_then(|g| g.members.iter().find(|&&(c, _)| c == call))
                .map(|&(_, id)| id);
        }
        match x {
            Some(XRegion::Pieces(p)) if !p.is_empty() => {
                has_x = true;
                match p {
                    Cow::Borrowed(p) => {
                        for piece in p {
                            pieces.push(MirrorPiece {
                                role: PieceRole::System,
                                node: Some(piece.node),
                                raw: raw_sys,
                                flat: Cow::Borrowed(&piece.constraint),
                                exact: None,
                                grid: None,
                                summary: Cow::Borrowed(&piece.summary),
                            });
                        }
                    }
                    Cow::Owned(p) => {
                        for piece in p {
                            pieces.push(MirrorPiece {
                                role: PieceRole::System,
                                node: Some(piece.node),
                                raw: raw_sys,
                                flat: Cow::Owned(piece.constraint),
                                exact: None,
                                grid: None,
                                summary: Cow::Owned(piece.summary),
                            });
                        }
                    }
                }
            }
            Some(XRegion::ImplicitPass(c, s)) if !(is_empty_or(&c) || s.is_empty()) => {
                has_x = true;
                pieces.push(MirrorPiece {
                    role: PieceRole::System,
                    node: None,
                    raw: raw_sys,
                    flat: c,
                    exact: None,
                    grid: None,
                    summary: s,
                });
            }
            _ => {}
        }
        if spec.want_text {
            if let Some(id) = node {
                text = pos.system.node(id).description.clone();
            } else if has_x {
                text = if prefix.calls().iter().all(|&c| c == Call::Pass) {
                    "no opening bid".to_string()
                } else {
                    "implicit pass".to_string()
                };
            }
        }
        // `N_sys`: no system candidate satisfied and no implicit pass to catch it.
        if !pos.implicit_pass && !spec.strict {
            if let Some((c, s)) = complement(&pos) {
                if !(is_empty_or(&c) || s.is_empty()) {
                    pieces.push(MirrorPiece {
                        role: PieceRole::NoSystem,
                        node: None,
                        raw: raw_sys / n,
                        flat: c,
                        exact: None,
                        grid: None,
                        summary: s,
                    });
                }
            }
        }
        ((1.0 - eps) * delta, (1.0 - eps) * delta / n)
    } else {
        ((1.0 - eps), (1.0 - eps) / n)
    };

    if nat_w > 0.0 {
        let np = natural_position(
            spec.table,
            &pos,
            spec.natural,
            prefix,
            spec.implicit_pass,
            || reader.partner_context(j),
        );
        let regions = np.regions(call);
        let piece = |role: PieceRole, raw: f64, p: &NaturalPiece| MirrorPiece {
            role,
            node: None,
            raw,
            flat: Cow::Owned(p.flat.clone()),
            exact: if spec.membership {
                p.exact.clone()
            } else {
                None
            },
            grid: if spec.membership {
                p.grid.clone()
            } else {
                None
            },
            summary: Cow::Owned(p.summary.clone()),
        };
        if let Some(y) = &regions.y {
            has_y = true;
            pieces.push(piece(PieceRole::Natural, nat_w, y));
            if spec.want_text && !has_x {
                text = np.text(spec.natural, prefix, call);
            }
        }
        if !spec.strict {
            if let Some(none) = &regions.n {
                pieces.push(piece(PieceRole::NoNatural, nat_none_w, none));
            }
        }
    }

    if !spec.strict && eps > 0.0 {
        pieces.push(MirrorPiece {
            role: PieceRole::Any,
            node: None,
            raw: eps / n,
            flat: Cow::Owned(HandConstraint::ANY),
            exact: None,
            grid: None,
            summary: Cow::Owned(PieceSummary::of(&HandConstraint::ANY)),
        });
    }

    let shadowed = !has_x && !has_y;
    if shadowed && spec.want_text {
        let base = match node {
            Some(id) => pos.system.node(id).description.clone(),
            None => natural_position(
                spec.table,
                &pos,
                spec.natural,
                prefix,
                spec.implicit_pass,
                || reader.partner_context(j),
            )
            .text(spec.natural, prefix, call),
        };
        text = format!("{base} [never chosen by the policy here]");
    }

    CallMirror {
        pieces,
        kind,
        shadowed,
        node,
        text,
    }
}
