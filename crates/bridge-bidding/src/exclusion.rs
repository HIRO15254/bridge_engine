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

use bridge_constraint::grid::bounds;
use bridge_constraint::{Atom, GridBounds, HandConstraint, HcpShapeGrid};
use bridge_core::{Auction, Call, ShapeSet};
use bridge_system::exclusive::{
    ExclusivePiece, PieceSummary, branches_of, is_empty_or, subtract, subtract_tree,
};
use bridge_system::{Forcing, NaturalCandidate, NaturalInference, PartnerContext};

use crate::choose::{Position, enumerate_position, ranked_legal};
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
    /// The exact region when `flat` over-covers it.
    pub(crate) exact: Option<HandConstraint>,
    /// The exact region as a grid, when it is literal-free and was computed on the grid.
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
    pub(crate) pieces: Vec<MirrorPiece<'a>>,
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
                let partner = self.partner_context(j);
                let cand = self
                    .natural
                    .infer_batch(&prefix, pos.seat, &partner, &[call])
                    .pop()
                    .expect("one result per call");
                natural_reading(&cand)
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

/// The natural regions of `call` among `ranked` (rank order): `Y_c` and, without the natural
/// implicit pass, `N_nat`.
fn natural_regions(
    ranked: &[NaturalCandidate],
    call: Call,
    implicit_pass: ImplicitPass,
) -> (Option<NaturalPiece>, Option<NaturalPiece>) {
    let bounds_of: Vec<GridBounds> = ranked.iter().map(|c| bounds(&c.constraint)).collect();
    let union_sub =
        |range: &[GridBounds]| range.iter().fold(HcpShapeGrid::EMPTY, |g, b| g.or(&b.sub));
    let all_exact = |range: &[GridBounds]| range.iter().all(GridBounds::is_exact);
    let constraints: Vec<&HandConstraint> = ranked.iter().map(|c| &c.constraint).collect();
    let idx = ranked.iter().position(|c| c.call == call);
    let natural_pass = implicit_pass == ImplicitPass::Complement;

    let y = if call == Call::Pass && natural_pass {
        // The natural implicit pass: the first satisfied candidate is `Pass`, or none is.
        let (above, own, below) = match idx {
            Some(i) => (&bounds_of[..i], Some(&bounds_of[i]), &bounds_of[i + 1..]),
            None => (&bounds_of[..], None, &bounds_of[..0]),
        };
        let none_below = union_sub(below).not();
        let inner = match own {
            Some(b) => b.sup.or(&none_below),
            None => none_below,
        };
        let grid = union_sub(above).not().and(&inner);
        let exact = all_exact(&bounds_of);
        grid_piece(&grid, exact, None, || {
            let (above_c, below_c) = match idx {
                Some(i) => (&constraints[..i], &constraints[i + 1..]),
                None => (&constraints[..], &constraints[..0]),
            };
            let not_below = or_of(below_c).map(|u| u.not());
            let inner = match (idx, not_below) {
                (Some(i), Some(nb)) => Some(HandConstraint::Or(vec![constraints[i].clone(), nb])),
                (Some(i), None) => Some(constraints[i].clone()),
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
        let own_b = &bounds_of[i];
        let grid = own_b.sup.and(&union_sub(&bounds_of[..i]).not());
        let exact_above = all_exact(&bounds_of[..i]);
        let own_c = &ranked[i].constraint;
        let own = (!own_b.is_exact()).then_some(own_c);
        grid_piece(&grid, exact_above, own, || {
            subtract_tree(own_c, &constraints[..i])
        })
    } else {
        None
    };

    let n = if natural_pass {
        None
    } else {
        let grid = union_sub(&bounds_of).not();
        grid_piece(&grid, all_exact(&bounds_of), None, || {
            or_of(&constraints).map_or(HandConstraint::ANY, |u| u.not())
        })
    };
    (y, n)
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
    let mut pieces: Vec<MirrorPiece<'t>> = Vec::new();
    let mut kind = ResolutionKind::Natural;
    let mut node: Option<NodeId> = None;
    let mut text = String::new();
    let mut has_x = false;
    let mut has_y = false;

    let on_system = pos.on_system();
    let (nat_w, nat_none_w) = if on_system {
        let raw_sys = (1.0 - eps) * (1.0 - delta);
        let x = system_x(&pos, call);
        if let Some(r) = x.as_ref().and_then(system_reading) {
            reader.memo[j] = Some(r);
        }
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
        let partner = reader.partner_context(j);
        let tie_break = pos.system.meta.tie_break;
        let ranked = spec
            .natural
            .ranked_candidates(prefix, pos.seat, &partner, tie_break);
        if !on_system && reader.memo[j].is_none() {
            let r = match ranked.iter().find(|c| c.call == call) {
                Some(cand) => natural_reading(cand),
                None => Reading::default(),
            };
            reader.memo[j] = Some(r);
        }
        let (y, none) = natural_regions(&ranked, call, spec.implicit_pass);
        if let Some(y) = y {
            has_y = true;
            pieces.push(MirrorPiece {
                role: PieceRole::Natural,
                node: None,
                raw: nat_w,
                flat: Cow::Owned(y.flat),
                exact: y.exact,
                grid: y.grid,
                summary: Cow::Owned(y.summary),
            });
            if spec.want_text && !has_x {
                text = natural_text(spec.natural, prefix, call, &partner);
            }
        }
        if !spec.strict {
            if let Some(none) = none {
                pieces.push(MirrorPiece {
                    role: PieceRole::NoNatural,
                    node: None,
                    raw: nat_none_w,
                    flat: Cow::Owned(none.flat),
                    exact: none.exact,
                    grid: none.grid,
                    summary: Cow::Owned(none.summary),
                });
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
            None => {
                let partner = reader.partner_context(j);
                natural_text(spec.natural, prefix, call, &partner)
            }
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
