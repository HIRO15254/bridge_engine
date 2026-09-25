//! `interpret`: auction → constraints.
//!
//! **Step A (per call).** For call `j` by seat `s`, resolve in `table.systems[s]`. `Exact`
//! yields one alternative per top-level `Or` branch (weights from `branch_weights` or equal);
//! `Partial` first tries `resolve_lenient`, then falls back to natural inference. Every
//! alternative is scaled by `1 − ε` and a defensive branch `(ANY, ε, Fallback)` is appended, with
//! `ε` depending on the resolution kind. This is how lower confidence is represented: more mass
//! on the unconstrained alternative, never an ad-hoc loosening of the constraint; the sampler's
//! importance weights correct the mixture afterwards.
//!
//! **Step B (per seat).** The alternatives of a seat's calls are combined by cross product
//! (`and`, unsatisfiable combinations dropped by a summary-only pre-check, deduplicated by
//! node/kind/branch, truncated to `K` by weight, renormalised). Each call contributes only its
//! own node's constraint; calls before the divergence point keep their `Exact` confidence, which
//! is the operational meaning of "weaken later constraints, not earlier ones".

use bridge_constraint::HandConstraint;
use bridge_core::{Auction, Call, Hand, Seat};
use bridge_system::natural::classify;
use bridge_system::trie::{LookupKey, RelVul, TrieId};
use bridge_system::{CallContext, Forcing, SystemIR};

use crate::{NodeId, Table};

/// Upper bound on opponents'-call substitutions passed to `resolve_lenient` (07-bidding.md §3;
/// not an option, a fixed implementation constant).
pub(crate) const LENIENT_MAX_SUBST: u8 = 2;

/// How a call was resolved. Ordered from most to least confident.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ResolutionKind {
    /// The sequence is in the system.
    Exact,
    /// A prefix is in the system.
    Partial {
        /// Matched prefix length.
        matched_depth: usize,
    },
    /// Natural inference.
    Natural,
    /// The defensive `ANY` branch.
    Fallback,
}

/// Explanation of one call.
#[derive(Clone, Debug)]
pub struct CallExplanation {
    /// Index of the call in the auction.
    pub call_index: usize,
    /// The call.
    pub call: Call,
    /// The node, if any.
    pub node: Option<NodeId>,
    /// Resolution kind.
    pub kind: ResolutionKind,
    /// The node's description or the natural rule text; empty for `Fallback`.
    pub text: String,
}

/// Explanation of one alternative for one seat.
#[derive(Clone, Debug)]
pub struct Explanation {
    /// The parts joined with ` / `.
    pub text: String,
    /// The node of the seat's most recent call.
    pub node: Option<NodeId>,
    /// The least confident kind among the parts.
    pub resolution: ResolutionKind,
    /// One part per call of this seat.
    pub parts: Vec<CallExplanation>,
}

impl Explanation {
    /// The explanation of a seat that has not called at all.
    fn empty() -> Explanation {
        Explanation {
            text: String::new(),
            node: None,
            resolution: ResolutionKind::Exact,
            parts: Vec::new(),
        }
    }

    /// Builds a seat's explanation from the parts contributed by each of its calls (04-bidding.md
    /// §4.4.5): `text` joins the non-empty part texts with ` / `, `node` is the last part's node,
    /// `resolution` is the least confident kind among the parts.
    fn from_parts(parts: Vec<CallExplanation>) -> Explanation {
        let text = parts
            .iter()
            .map(|p| p.text.as_str())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" / ");
        let node = parts.last().and_then(|p| p.node);
        let resolution = parts
            .iter()
            .map(|p| p.kind)
            .max()
            .unwrap_or(ResolutionKind::Exact);
        Explanation {
            text,
            node,
            resolution,
            parts,
        }
    }
}

/// The weighted disjunction for one call, before combination.
#[derive(Clone, Debug)]
pub struct CallInterpretation {
    /// Index of the call.
    pub call_index: usize,
    /// Its seat.
    pub seat: Seat,
    /// The call.
    pub call: Call,
    /// Resolution kind.
    pub kind: ResolutionKind,
    /// Alternatives; weights sum to 1.
    pub alternatives: Vec<(HandConstraint, f32, CallExplanation)>,
}

/// The result of [`interpret`].
#[derive(Clone, Debug)]
pub struct Interpretation {
    /// Per seat: weighted alternatives summing to 1.
    pub seats: [Vec<(HandConstraint, f32, Explanation)>; 4],
    /// Per call, before combination (for display and likelihoods).
    pub per_call: Vec<CallInterpretation>,
    /// The first call index that was not resolved `Exact`, if any.
    pub divergence: Option<usize>,
}

impl Interpretation {
    /// Whether `hand` satisfies at least one non-`Fallback` alternative of `seat` (the strict
    /// check used by the consistency test).
    ///
    /// Defined over `per_call` rather than the truncated `seats` mixture: a seat's true
    /// disjunction is the union, over its calls, of the AND of one alternative per call, and
    /// membership in that union is equivalent to every call having some non-fallback,
    /// positive-weight alternative that contains `hand`. Checking against `seats` instead would
    /// false-positive on nodes with many branches, since `seats` is capped to `K` alternatives.
    /// A seat with no calls is vacuously satisfied.
    pub fn satisfied_by(&self, seat: Seat, hand: Hand) -> bool {
        self.per_call
            .iter()
            .filter(|call| call.seat == seat)
            .all(|call| {
                call.alternatives
                    .iter()
                    .any(|(constraint, weight, explanation)| {
                        *weight > 0.0
                            && explanation.kind != ResolutionKind::Fallback
                            && constraint.satisfies(hand)
                    })
            })
    }

    /// `Π_{j ∈ calls(seat)} Σ_i w_{j,i} · [C_{j,i} ∋ hand]`: the set-membership mass of `hand`
    /// under the mixture, one factor per call (including its `Fallback` branch). This is not
    /// the bidding-policy likelihood (see `sequence_log_likelihood`); it matches the
    /// pre-truncation mass and needs no renormalisation. A seat with no calls has likelihood 1.
    pub fn likelihood(&self, seat: Seat, hand: Hand) -> f32 {
        self.per_call
            .iter()
            .filter(|call| call.seat == seat)
            .map(|call| {
                call.alternatives
                    .iter()
                    .filter(|(constraint, _, _)| constraint.satisfies(hand))
                    .map(|(_, weight, _)| *weight)
                    .sum::<f32>()
            })
            .product()
    }
}

/// Options for [`interpret`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct InterpretOptions {
    /// Maximum alternatives kept per seat (default 8).
    pub max_alternatives: usize,
    /// Fallback mass for `Exact` resolutions (default 0.02).
    pub eps_exact: f32,
    /// Fallback mass for `Partial` resolutions (default 0.15).
    pub eps_partial: f32,
    /// Fallback mass for `Natural` resolutions (default 0.30).
    pub eps_natural: f32,
    /// No fallback branches at all (property tests).
    pub strict: bool,
    /// Weight multiplier per opponents'-call substitution in `resolve_lenient` (default 0.5).
    pub lenient_decay: f32,
}

impl Default for InterpretOptions {
    fn default() -> InterpretOptions {
        InterpretOptions {
            max_alternatives: 8,
            eps_exact: 0.02,
            eps_partial: 0.15,
            eps_natural: 0.30,
            strict: false,
            lenient_decay: 0.5,
        }
    }
}

/// A cheap, summary-only unsatisfiability check for Step B's pre-check (07-bidding.md §4.4.2):
/// empty shape set, inverted HCP range, or an HCP range that no shape in the set can reach. This
/// deliberately does *not* call `HandConstraint::is_satisfiable` (DNF expansion), which is too
/// slow for the per-combination check in the cross product.
pub(crate) fn summary_satisfiable(c: &HandConstraint) -> bool {
    let shapes = c.shapes();
    if shapes.is_empty() {
        return false;
    }
    let hcp = c.hcp_range();
    if hcp.is_empty() {
        return false;
    }
    // `ShapeSet::min_hcp`/`max_hcp` walk every member shape (up to ~560 for an unrestricted set),
    // which is exactly the case for most calls here (a bare HCP atom never narrows `shapes`). An
    // unrestricted shape set can never make the HCP range cross-check below fail (every HCP from
    // 0 to 37 is reachable by *some* shape), so skip the walk entirely when `shapes == ALL`; this
    // keeps the cross product's per-combination pre-check cheap enough for the 10 µs budget
    // (07-bidding.md §4.4.2) without changing the result.
    if shapes == bridge_core::ShapeSet::ALL {
        return true;
    }
    if *hcp.start() > shapes.max_hcp() || *hcp.end() < shapes.min_hcp() {
        return false;
    }
    true
}

/// Normalises a set of weighted alternatives to sum to 1 (no-op on an empty or already-zero-sum
/// slice).
fn normalize_alts(alts: &mut [(HandConstraint, f32, CallExplanation)]) {
    let total: f32 = alts.iter().map(|(_, w, _)| *w).sum();
    if total > 0.0 {
        for (_, w, _) in alts.iter_mut() {
            *w /= total;
        }
    }
}

/// Expands `node`'s constraint into one alternative per top-level `Or` branch (weighted by
/// `branch_weights`, or equally), each scaled by `weight_mult`; a non-`Or` constraint yields a
/// single alternative. Every alternative's [`CallExplanation`] carries `kind` (the call's overall
/// resolution kind, `Exact` or `Partial`) and `node`'s description as `text`.
fn expand_node_branches(
    sys: &SystemIR,
    node_id: NodeId,
    call_index: usize,
    call: Call,
    kind: ResolutionKind,
    weight_mult: f32,
    out: &mut Vec<(HandConstraint, f32, CallExplanation)>,
) {
    let node = sys.node(node_id);
    let text = node.description.clone();
    match &node.constraint {
        HandConstraint::Or(branches) if !branches.is_empty() => {
            let weights: Vec<f32> = match &node.branch_weights {
                Some(w) if w.len() == branches.len() => w.clone(),
                _ => vec![1.0 / branches.len() as f32; branches.len()],
            };
            let wsum: f32 = weights.iter().sum();
            for (branch, w) in branches.iter().zip(weights.iter()) {
                let w = if wsum > 0.0 { w / wsum } else { 0.0 };
                out.push((
                    branch.clone(),
                    weight_mult * w,
                    CallExplanation {
                        call_index,
                        call,
                        node: Some(node_id),
                        kind,
                        text: text.clone(),
                    },
                ));
            }
        }
        _ => {
            out.push((
                node.constraint.clone(),
                weight_mult,
                CallExplanation {
                    call_index,
                    call,
                    node: Some(node_id),
                    kind,
                    text,
                },
            ));
        }
    }
}

/// The complement of the union of `siblings`' constraints (used by both the implicit-pass
/// synthesis here and in `choose_bid`, so that the two stay bidirectionally consistent).
pub(crate) fn complement_of(sys: &SystemIR, siblings: &[(Call, NodeId)]) -> HandConstraint {
    let combined = siblings
        .iter()
        .map(|(_, id)| sys.node(*id).constraint.clone())
        .reduce(HandConstraint::or)
        .unwrap_or(HandConstraint::ANY);
    combined.not()
}

/// Applies the ε-mixture (07-bidding.md §4.1 step 7, §4.2): scales every alternative by `1 − ε`
/// and appends the defensive `(ANY, ε, Fallback)` branch, unless `opts.strict`.
fn apply_epsilon_mixture(
    alts: &mut Vec<(HandConstraint, f32, CallExplanation)>,
    kind: ResolutionKind,
    call_index: usize,
    call: Call,
    opts: &InterpretOptions,
) {
    if opts.strict {
        return;
    }
    let eps = match kind {
        ResolutionKind::Exact => opts.eps_exact,
        ResolutionKind::Partial { .. } => opts.eps_partial,
        ResolutionKind::Natural => opts.eps_natural,
        ResolutionKind::Fallback => 0.0,
    };
    if eps <= 0.0 {
        return;
    }
    for (_, w, _) in alts.iter_mut() {
        *w *= 1.0 - eps;
    }
    alts.push((
        HandConstraint::ANY,
        eps,
        CallExplanation {
            call_index,
            call,
            node: None,
            kind: ResolutionKind::Fallback,
            text: String::new(),
        },
    ));
}

/// The partner's constraint and forcing-situation flag from their most recent call, if any
/// (07-bidding.md §2.2): the maximum-weight alternative of partner's most recent call, and
/// whether that alternative's node is forcing.
///
/// Used by [`fill_partner_context`] (`interpret`'s own natural step) and, through
/// [`partner_context_for_prefix`], by `choose_bid`'s natural branch, so both compute
/// `CallContext::partner_constraint`/`forcing_situation` identically for the same auction prefix.
pub(crate) fn partner_context(
    table: &Table,
    per_call_so_far: &[CallInterpretation],
    s: Seat,
) -> (Option<HandConstraint>, bool) {
    let partner = s.partner();
    let Some(last) = per_call_so_far.iter().rev().find(|ci| ci.seat == partner) else {
        return (None, false);
    };
    // Skip the `Fallback` branch the ε-mixture appended (07-bidding.md §4.2): it is always
    // present after `apply_epsilon_mixture` and would otherwise win `max_by` whenever partner's
    // call is `Partial`/`Natural` with enough real alternatives that each falls under
    // `eps_partial`/`eps_natural` on its own, silently turning `partner_constraint` into `ANY`
    // and `forcing_situation` into `false` even when the node is actually forcing.
    let Some((c, _, ex)) = last
        .alternatives
        .iter()
        .filter(|(_, _, ex)| ex.kind != ResolutionKind::Fallback)
        .max_by(|a, b| a.1.total_cmp(&b.1))
    else {
        return (None, false);
    };
    let forcing = match ex.node {
        Some(node_id) => {
            let partner_sys = &table.systems[partner.index() as usize];
            let flags = &partner_sys.node(node_id).flags;
            matches!(flags.forcing, Forcing::OneRound | Forcing::ToGame)
        }
        None => false,
    };
    (Some(c.clone()), forcing)
}

/// [`partner_context`] for seat `s` about to call after `auction`, computed from Step A of
/// `auction` itself (no Step B). `choose_bid`'s natural branch (07-bidding.md §5.2 step 4) needs
/// the same `CallContext` that `interpret`'s natural step (§4.1 step 6) builds for the call it is
/// about to make, and Step A's `per_call[..n]` of `auction` equals that of `auction.with(call)`
/// for any `call`: each entry `j` reads only the calls up to `j` (the lookup key is truncated to
/// `n_k` calls, `classify` reads only history up to `j`, and the leading-pass count of the prefix
/// agrees with the extended auction's for every `j < n`). Strict options are used because
/// `partner_context` ignores the ε-mixture's `Fallback` branch anyway and the mixture scales every
/// other weight uniformly; `lenient_decay` stays at its default, so an `interpret` called with a
/// non-default `lenient_decay` can in rare ties pick a different partner alternative.
pub(crate) fn partner_context_for_prefix(
    table: &Table,
    auction: &Auction,
    s: Seat,
) -> (Option<HandConstraint>, bool) {
    let opts = InterpretOptions {
        strict: true,
        ..InterpretOptions::default()
    };
    let (per_call, _) = step_a(table, auction, &opts);
    partner_context(table, &per_call, s)
}

/// Fills `ctx.partner_constraint` / `ctx.forcing_situation` from the interpretation of partner's
/// calls so far (07-bidding.md §2.2); see [`partner_context`].
fn fill_partner_context(
    mut ctx: CallContext,
    table: &Table,
    per_call_so_far: &[CallInterpretation],
    s: Seat,
) -> CallContext {
    let (constraint, forcing) = partner_context(table, per_call_so_far, s);
    ctx.partner_constraint = constraint;
    ctx.forcing_situation = forcing;
    ctx
}

/// Natural inference for call `j` by `s` (07-bidding.md §4.1 step 6): a single alternative with
/// weight 1.
fn natural_alternative(
    table: &Table,
    auction: &Auction,
    s: Seat,
    j: usize,
    per_call_so_far: &[CallInterpretation],
) -> Vec<(HandConstraint, f32, CallExplanation)> {
    let ctx = classify(auction, j, s);
    let ctx = fill_partner_context(ctx, table, per_call_so_far, s);
    let inf = table.natural.infer(&ctx);
    let text = format!("{} ({})", inf.explanation, inf.rule);
    vec![(
        inf.constraint,
        1.0,
        CallExplanation {
            call_index: j,
            call: auction.calls()[j],
            node: None,
            kind: ResolutionKind::Natural,
            text,
        },
    )]
}

/// Step A for a leading pass (`j < lp`, 07-bidding.md §4.1 step 2): the complement of our own
/// system's opening candidates at this position, or natural inference if the opening table is
/// empty.
fn step_a_leading_pass(
    table: &Table,
    auction: &Auction,
    prefix: &Auction,
    s: Seat,
    vul: RelVul,
    j: usize,
    per_call_so_far: &[CallInterpretation],
) -> (ResolutionKind, Vec<(HandConstraint, f32, CallExplanation)>) {
    let sys = &table.systems[s.index() as usize];
    let opener_pos = auction.position_of(s);
    let children = sys.index.children(TrieId(0), opener_pos, vul);
    let legal: Vec<(Call, NodeId)> = children
        .into_iter()
        .filter(|(c, _)| prefix.is_legal(*c))
        .collect();
    if legal.is_empty() {
        return (
            ResolutionKind::Natural,
            natural_alternative(table, auction, s, j, per_call_so_far),
        );
    }
    let complement = complement_of(sys, &legal);
    (
        ResolutionKind::Exact,
        vec![(
            complement,
            1.0,
            CallExplanation {
                call_index: j,
                call: Call::Pass,
                node: None,
                kind: ResolutionKind::Exact,
                text: "no opening bid".to_string(),
            },
        )],
    )
}

/// Step A for a real call (`j >= lp`, 07-bidding.md §4.1 steps 3–6).
#[allow(clippy::too_many_arguments)]
fn step_a_call(
    table: &Table,
    auction: &Auction,
    prefix: &Auction,
    s: Seat,
    j: usize,
    lp: usize,
    opts: &InterpretOptions,
    divergence: &mut Option<usize>,
    per_call_so_far: &[CallInterpretation],
) -> (ResolutionKind, Vec<(HandConstraint, f32, CallExplanation)>) {
    let sys = &table.systems[s.index() as usize];
    let call = auction.calls()[j];
    let full_key =
        LookupKey::for_auction(auction, s).expect("j >= lp implies a non-passed-out auction");
    let n_k = j - lp + 1;
    let key = LookupKey {
        we_opened: full_key.we_opened,
        calls: &full_key.calls[..n_k],
        opener_pos: full_key.opener_pos,
        vul: full_key.vul,
    };
    let lookup = sys.index.resolve(&key);
    let mut d = lookup.matched_depth;

    if d == n_k {
        if let Some(node_id) = lookup.by_depth[n_k - 1] {
            let mut alts = Vec::new();
            expand_node_branches(sys, node_id, j, call, ResolutionKind::Exact, 1.0, &mut alts);
            normalize_alts(&mut alts);
            return (ResolutionKind::Exact, alts);
        }
        // The walk matched all `n_k` calls, but the last one landed on an implicit-pass trie
        // node (06-system.md §4.3): a node that exists only because some *deeper* row's path
        // runs through it (e.g. a competitive continuation like `1H-(P)-P-(1S)-X`), carrying no
        // row/entries of its own. This is fully on-system, not a fall-through — resolve the
        // §4.1.5.1 implicit-pass complement at the *parent* position (the siblings of this
        // call), not at `lookup.end` (which the walk already advanced past this call to: its
        // children are the calls *after* our pass, not our pass's siblings). Re-resolving the
        // one-call-shorter key is the only way to recover that parent `TrieId`, since `Lookup`
        // only ever exposes the trie id reached at its own `matched_depth`.
        if call == Call::Pass {
            let parent_key = LookupKey {
                we_opened: key.we_opened,
                calls: &key.calls[..n_k - 1],
                opener_pos: key.opener_pos,
                vul: key.vul,
            };
            let parent = sys.index.resolve(&parent_key);
            if parent.matched_depth == n_k - 1 {
                let siblings = sys.index.children(parent.end, key.opener_pos, key.vul);
                let legal_siblings: Vec<(Call, NodeId)> = siblings
                    .into_iter()
                    .filter(|(c, _)| prefix.is_legal(*c))
                    .collect();
                if !legal_siblings.is_empty() {
                    let complement = complement_of(sys, &legal_siblings);
                    return (
                        ResolutionKind::Exact,
                        vec![(
                            complement,
                            1.0,
                            CallExplanation {
                                call_index: j,
                                call,
                                node: None,
                                kind: ResolutionKind::Exact,
                                text: "implicit pass".to_string(),
                            },
                        )],
                    );
                }
            }
        }
        // Defensive: no row at this depth and no legal system siblings to complement either
        // (07-bidding.md does not spell out this case). Treat it like a one-short partial match
        // so the lenient/natural machinery below still applies instead of panicking.
        d = n_k - 1;
    }

    *divergence = Some(divergence.map_or(j, |m| m.min(j)));

    // 5.1: implicit pass. Only applies when the *current* call is the one that failed to match
    // (07-bidding.md §4.1 step 5, "d < n_k − 1 の場合...手順5.1を飛ばす").
    if d == n_k - 1 && call == Call::Pass {
        let siblings = sys.index.children(lookup.end, key.opener_pos, key.vul);
        let legal_siblings: Vec<(Call, NodeId)> = siblings
            .into_iter()
            .filter(|(c, _)| prefix.is_legal(*c))
            .collect();
        if !legal_siblings.is_empty() {
            let complement = complement_of(sys, &legal_siblings);
            return (
                ResolutionKind::Exact,
                vec![(
                    complement,
                    1.0,
                    CallExplanation {
                        call_index: j,
                        call,
                        node: None,
                        kind: ResolutionKind::Exact,
                        text: "implicit pass".to_string(),
                    },
                )],
            );
        }
    }

    // 5.2: lenient resolution.
    let attempts = sys.index.resolve_lenient(&key, LENIENT_MAX_SUBST);
    let mut weighted: Vec<(NodeId, f32)> = Vec::new();
    for (lk, subst) in attempts.iter() {
        if lk.matched_depth != n_k {
            continue;
        }
        let Some(node_id) = lk.by_depth[n_k - 1] else {
            continue;
        };
        let w = opts.lenient_decay.powi(i32::from(*subst));
        match weighted.iter_mut().find(|(id, _)| *id == node_id) {
            Some((_, existing)) => *existing += w,
            None => weighted.push((node_id, w)),
        }
    }
    if !weighted.is_empty() {
        let mut alts = Vec::new();
        for (node_id, w) in weighted {
            expand_node_branches(
                sys,
                node_id,
                j,
                call,
                ResolutionKind::Partial { matched_depth: d },
                w,
                &mut alts,
            );
        }
        normalize_alts(&mut alts);
        return (ResolutionKind::Partial { matched_depth: d }, alts);
    }

    // 6: natural inference.
    (
        ResolutionKind::Natural,
        natural_alternative(table, auction, s, j, per_call_so_far),
    )
}

/// Step A: builds `per_call` and the divergence index.
fn step_a(
    table: &Table,
    auction: &Auction,
    opts: &InterpretOptions,
) -> (Vec<CallInterpretation>, Option<usize>) {
    let calls = auction.calls();
    let n = calls.len();
    let lp = auction.leading_passes();
    let vulnerability = auction.vulnerability();
    let mut per_call: Vec<CallInterpretation> = Vec::with_capacity(n);
    let mut divergence: Option<usize> = None;
    let mut prefix = Auction::new(auction.dealer(), vulnerability);

    for (j, &call) in calls.iter().enumerate().take(n) {
        let s = auction.seat_at(j);
        let vul = RelVul {
            we: vulnerability.is_vulnerable(s),
            they: vulnerability.is_vulnerable(s.next()),
        };

        let (kind, mut alternatives) = if j < lp {
            step_a_leading_pass(table, auction, &prefix, s, vul, j, &per_call)
        } else {
            step_a_call(
                table,
                auction,
                &prefix,
                s,
                j,
                lp,
                opts,
                &mut divergence,
                &per_call,
            )
        };

        apply_epsilon_mixture(&mut alternatives, kind, j, call, opts);

        per_call.push(CallInterpretation {
            call_index: j,
            seat: s,
            call,
            kind,
            alternatives,
        });

        prefix
            .push(call)
            .expect("call from a valid Auction is legal at its own position");
    }

    (per_call, divergence)
}

/// `true` for the unconstrained atom (`HandConstraint::ANY`, i.e. `Atom::ANY`).
fn is_any(c: &HandConstraint) -> bool {
    matches!(c, HandConstraint::Atom(a) if *a == bridge_constraint::Atom::ANY)
}

/// `existing ∧ addition`, allocating the new `And`'s backing `Vec` once at its final size instead
/// of `HandConstraint::and`'s clone-then-push (which, starting from an already-owned `Vec` of
/// exactly `len` capacity, reallocates again on the `push`). Called once per surviving candidate
/// in Step B's cross product, so this compounds; see [`Combo`]'s doc comment for the same
/// reasoning applied to `key`.
///
/// `ANY` is treated as the identity of `∧` on both sides: without this, every combo that ever
/// passed through the initial `ANY` seed or a `Fallback` alternative carried it along forever as
/// a redundant `And` member (`And([ANY, c1, ANY, …])`), which was cloned and re-summarised at
/// every subsequent step of the cross product for no semantic benefit (07-bidding.md §4.4.2).
fn and_one_more(existing: &HandConstraint, addition: &HandConstraint) -> HandConstraint {
    if is_any(addition) {
        return existing.clone();
    }
    if is_any(existing) {
        return addition.clone();
    }
    match existing {
        HandConstraint::And(children) => {
            let mut v = Vec::with_capacity(children.len() + 1);
            v.extend_from_slice(children);
            v.push(addition.clone());
            HandConstraint::And(v)
        }
        other => HandConstraint::And(vec![other.clone(), addition.clone()]),
    }
}

/// `a ∩ b`: the tighter of two HCP ranges, or an empty range when they don't overlap.
fn clamp_hcp(
    a: &core::ops::RangeInclusive<u8>,
    b: &core::ops::RangeInclusive<u8>,
) -> core::ops::RangeInclusive<u8> {
    (*a.start().max(b.start()))..=(*a.end().min(b.end()))
}

/// A combo's running shape/HCP summary and the (expensive) `ShapeSet::min_hcp`/`max_hcp` bounds
/// derived from it, kept incrementally instead of being recomputed from the whole constraint tree
/// at every step of the cross product (see [`Combo`]'s doc comment for why this matters).
#[derive(Clone)]
struct Summary {
    shapes: bridge_core::ShapeSet,
    hcp: core::ops::RangeInclusive<u8>,
    /// `Some((min_hcp, max_hcp))` of `shapes`, or `None` when `shapes == ShapeSet::ALL` (no walk
    /// is needed then: every HCP in `0..=37` is reachable by *some* shape, so the cross-check
    /// below can never fail — see `summary_satisfiable`).
    bounds: Option<(u8, u8)>,
}

impl Summary {
    const ANY: Summary = Summary {
        shapes: bridge_core::ShapeSet::ALL,
        hcp: 0..=37,
        bounds: None,
    };

    /// The standalone summary of one alternative's own constraint (computed once per alternative
    /// in [`step_b`], not once per combo).
    fn of(c: &HandConstraint) -> Summary {
        let shapes = c.shapes();
        let bounds =
            (shapes != bridge_core::ShapeSet::ALL).then(|| (shapes.min_hcp(), shapes.max_hcp()));
        Summary {
            shapes,
            hcp: c.hcp_range(),
            bounds,
        }
    }

    /// `self ∧ addition`'s summary, or `None` when the cheap check finds it unsatisfiable (an
    /// empty shape set, an inverted HCP range, or an HCP range no shape in the set can reach —
    /// the same three checks as `summary_satisfiable`, just computed incrementally). The
    /// `min_hcp`/`max_hcp` walk (`ShapeSet::{min,max}_hcp`, up to ~560 member shapes) only runs
    /// when the shape set actually narrows from `self`'s, not on every combination; when it does
    /// not narrow, `self.bounds` is still valid for the new (possibly HCP-narrower) range and is
    /// reused as-is (07-bidding.md §4.4.2's `interpret < 10 µs` budget).
    fn and(&self, addition: &Summary) -> Option<Summary> {
        let shapes = self.shapes.intersect(addition.shapes);
        if shapes.is_empty() {
            return None;
        }
        let hcp = clamp_hcp(&self.hcp, &addition.hcp);
        if hcp.is_empty() {
            return None;
        }
        let bounds = if shapes == bridge_core::ShapeSet::ALL {
            None
        } else if shapes == self.shapes {
            self.bounds
        } else {
            Some((shapes.min_hcp(), shapes.max_hcp()))
        };
        if let Some((min_hcp, max_hcp)) = bounds {
            if *hcp.start() > max_hcp || *hcp.end() < min_hcp {
                return None;
            }
        }
        Some(Summary {
            shapes,
            hcp,
            bounds,
        })
    }
}

/// One partial cross-product combination for Step B, carried alongside a hidden dedup key
/// (`(node, kind, alternative-index)` per call, 07-bidding.md §4.4.3) that never leaves this
/// function: `CallExplanation` has no `branch_index` field, so the position of the chosen
/// alternative within its call's list stands in for it.
///
/// `key` doubles as a lightweight stand-in for `parts` during the fold: since calls are folded in
/// a fixed order (`seat_calls`, built once per seat below), `key[level].2` is enough to look the
/// level's actual `CallExplanation` back up afterwards (`materialize_parts`). This avoids cloning
/// a `Vec<CallExplanation>` (each element owning a `String`) at every intermediate combination —
/// only `Copy` tuples are cloned during the cross product, and the real `CallExplanation`s are
/// materialised once per surviving (post-dedup, post-truncation) combo instead of once per
/// intermediate one; on the 12-call bench auction this alone was the difference between roughly
/// 95 µs and single-digit µs (07-bidding.md §6.2's `interpret < 10 µs` target).
///
/// `summary` is the running `Summary` of `constraint` (see [`Summary::and`]): keeping it
/// incrementally, instead of recomputing `constraint.shapes()`/`hcp_range()` (a walk of the whole
/// `And` tree) and then `ShapeSet::min_hcp`/`max_hcp` (a walk of up to ~560 member shapes) from
/// scratch at every combination, is what keeps the cross-product's per-combination pre-check cheap
/// once a node's constraint carries a real suit-length or shape atom (`summary_satisfiable`'s
/// `shapes == ShapeSet::ALL` shortcut alone only covers bare-HCP atoms).
struct Combo {
    constraint: HandConstraint,
    summary: Summary,
    weight: f32,
    key: Vec<(Option<NodeId>, ResolutionKind, usize)>,
}

/// Rebuilds a surviving combo's `Vec<CallExplanation>` from its `key` and the seat's own calls in
/// fold order (see [`Combo`]'s doc comment).
fn materialize_parts(
    seat_calls: &[&CallInterpretation],
    key: &[(Option<NodeId>, ResolutionKind, usize)],
) -> Vec<CallExplanation> {
    key.iter()
        .enumerate()
        .map(|(level, &(_, _, alt_index))| seat_calls[level].alternatives[alt_index].2.clone())
        .collect()
}

/// Step B: combines `per_call` into the four seats' weighted disjunctions.
fn step_b(
    per_call: &[CallInterpretation],
    opts: &InterpretOptions,
) -> [Vec<(HandConstraint, f32, Explanation)>; 4] {
    let mut seats: [Vec<(HandConstraint, f32, Explanation)>; 4] = Default::default();

    for seat in Seat::ALL {
        let seat_calls: Vec<&CallInterpretation> =
            per_call.iter().filter(|c| c.seat == seat).collect();
        let mut combos: Vec<Combo> = vec![Combo {
            constraint: HandConstraint::ANY,
            summary: Summary::ANY,
            weight: 1.0,
            key: Vec::new(),
        }];
        let had_calls = !seat_calls.is_empty();

        for cj in &seat_calls {
            // Precomputed once per call, not once per (combo, alternative) pair: the same
            // `Summary::of` result is reused across every combo this call is folded into below.
            let alt_summaries: Vec<Summary> = cj
                .alternatives
                .iter()
                .map(|(c, _, _)| Summary::of(c))
                .collect();
            let mut next: Vec<Combo> = Vec::new();
            for combo in &combos {
                for (i, (ci, wi, ex)) in cj.alternatives.iter().enumerate() {
                    let Some(summary) = combo.summary.and(&alt_summaries[i]) else {
                        continue;
                    };
                    let c2 = and_one_more(&combo.constraint, ci);
                    // `with_capacity` + `extend_from_slice` (one allocation, sized exactly right)
                    // instead of `combo.key.clone()` then `push` (which can reallocate a second
                    // time): `key` is rebuilt on every surviving candidate in this loop, so the
                    // saving compounds across the cross product.
                    let mut key = Vec::with_capacity(combo.key.len() + 1);
                    key.extend_from_slice(&combo.key);
                    key.push((ex.node, ex.kind, i));
                    next.push(Combo {
                        constraint: c2,
                        summary,
                        weight: combo.weight * wi,
                        key,
                    });
                }
            }
            // Dedup by (node, kind, branch) parts, summing weights.
            let mut deduped: Vec<Combo> = Vec::new();
            for combo in next {
                match deduped.iter_mut().find(|d| d.key == combo.key) {
                    Some(existing) => existing.weight += combo.weight,
                    None => deduped.push(combo),
                }
            }
            deduped.sort_by(|a, b| b.weight.total_cmp(&a.weight));
            deduped.truncate(opts.max_alternatives);
            combos = deduped;
        }

        if had_calls && combos.is_empty() {
            tracing::warn!(?seat, "seat contradicts itself");
            combos = vec![Combo {
                constraint: HandConstraint::ANY,
                summary: Summary::ANY,
                weight: 1.0,
                key: Vec::new(),
            }];
        }

        let total: f32 = combos.iter().map(|c| c.weight).sum();
        let idx = seat.index() as usize;
        seats[idx] = combos
            .into_iter()
            .map(|c| {
                let w = if total > 0.0 {
                    c.weight / total
                } else {
                    c.weight
                };
                let parts = materialize_parts(&seat_calls, &c.key);
                (c.constraint, w, Explanation::from_parts(parts))
            })
            .collect();
        if seats[idx].is_empty() {
            seats[idx].push((HandConstraint::ANY, 1.0, Explanation::empty()));
        }
    }

    seats
}

/// Interprets `auction` under the four systems of `table`.
pub fn interpret(table: &Table, auction: &Auction, opts: &InterpretOptions) -> Interpretation {
    let (per_call, divergence) = step_a(table, auction, opts);
    let seats = step_b(&per_call, opts);
    Interpretation {
        seats,
        per_call,
        divergence,
    }
}

#[cfg(test)]
mod tests {
    use bridge_constraint::{Atom, HandConstraint, ShapeSet};
    use bridge_core::{Bid, Call, Hand, Holding, Rank, Strain, Suit};

    use super::*;

    fn balanced_15_17() -> HandConstraint {
        HandConstraint::Atom(Atom {
            shapes: ShapeSet::BALANCED,
            hcp: 15..=17,
            cards: Vec::new(),
            eval: Vec::new(),
        })
    }

    fn call_explanation(kind: ResolutionKind) -> CallExplanation {
        CallExplanation {
            call_index: 0,
            call: Call::Bid(Bid::new(1, Strain::NoTrump).unwrap()),
            node: None,
            kind,
            text: String::new(),
        }
    }

    /// North bid 1NT with a single non-fallback alternative (weight 0.98) plus the defensive
    /// fallback branch (weight 0.02, `ResolutionKind::Fallback`).
    fn one_call_interpretation() -> Interpretation {
        let alternatives = vec![
            (
                balanced_15_17(),
                0.98,
                call_explanation(ResolutionKind::Exact),
            ),
            (
                HandConstraint::ANY,
                0.02,
                call_explanation(ResolutionKind::Fallback),
            ),
        ];
        let per_call = vec![CallInterpretation {
            call_index: 0,
            seat: Seat::North,
            call: Call::Bid(Bid::new(1, Strain::NoTrump).unwrap()),
            kind: ResolutionKind::Exact,
            alternatives,
        }];
        Interpretation {
            seats: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            per_call,
            divergence: None,
        }
    }

    fn holding_of(ranks: &[Rank]) -> Holding {
        ranks.iter().fold(Holding::EMPTY, |h, &r| h.with(r))
    }

    /// 4=3=4=2 (clubs/diamonds/hearts/spades) shape, exactly 16 HCP: a balanced hand inside
    /// [`balanced_15_17`]'s range.
    fn balanced_16_hcp_hand() -> Hand {
        Hand::EMPTY
            .with_holding(
                Suit::Clubs,
                holding_of(&[Rank::Ace, Rank::King, Rank::Queen, Rank::Two]), // 9 HCP
            )
            .with_holding(
                Suit::Diamonds,
                holding_of(&[Rank::Ace, Rank::Two, Rank::Three]),
            ) // 4 HCP
            .with_holding(
                Suit::Hearts,
                holding_of(&[Rank::King, Rank::Two, Rank::Three, Rank::Four]), // 3 HCP
            )
            .with_holding(Suit::Spades, holding_of(&[Rank::Two, Rank::Three])) // 0 HCP
    }

    /// A flat, honour-free 4=3=3=3 hand: 0 HCP, well outside `balanced_15_17`'s range.
    fn zero_hcp_hand() -> Hand {
        Hand::EMPTY
            .with_holding(
                Suit::Clubs,
                holding_of(&[Rank::Two, Rank::Three, Rank::Four, Rank::Five]),
            )
            .with_holding(
                Suit::Diamonds,
                holding_of(&[Rank::Two, Rank::Three, Rank::Four]),
            )
            .with_holding(
                Suit::Hearts,
                holding_of(&[Rank::Two, Rank::Three, Rank::Four]),
            )
            .with_holding(
                Suit::Spades,
                holding_of(&[Rank::Two, Rank::Three, Rank::Four]),
            )
    }

    #[test]
    fn satisfied_by_is_vacuously_true_for_a_seat_with_no_calls() {
        let interpretation = one_call_interpretation();
        let hand = balanced_16_hcp_hand();
        assert!(interpretation.satisfied_by(Seat::East, hand));
        assert!(interpretation.satisfied_by(Seat::South, hand));
        assert!(interpretation.satisfied_by(Seat::West, hand));
    }

    #[test]
    fn satisfied_by_ignores_the_fallback_branch() {
        let interpretation = one_call_interpretation();
        // A 0-HCP hand satisfies only the `ANY`/`Fallback` alternative, never the strict
        // 15-17 balanced one, so `satisfied_by` (which excludes `Fallback`) must reject it.
        let weak_hand = zero_hcp_hand();
        assert_eq!(weak_hand.len(), 13);
        assert!(!interpretation.satisfied_by(Seat::North, weak_hand));
    }

    #[test]
    fn satisfied_by_accepts_a_hand_matching_the_strict_alternative() {
        let interpretation = one_call_interpretation();
        let hand = balanced_16_hcp_hand();
        assert!(interpretation.satisfied_by(Seat::North, hand));
    }

    #[test]
    fn likelihood_sums_only_the_alternatives_containing_the_hand() {
        let interpretation = one_call_interpretation();
        let hand = balanced_16_hcp_hand();
        // The hand is in the strict alternative (weight 0.98) and in `ANY` (weight 0.02): the
        // mixture mass is their sum.
        let likelihood = interpretation.likelihood(Seat::North, hand);
        assert!((likelihood - 1.0).abs() < 1e-6, "likelihood = {likelihood}");
    }

    #[test]
    fn likelihood_is_the_fallback_mass_alone_outside_the_strict_alternative() {
        let interpretation = one_call_interpretation();
        let weak_hand = zero_hcp_hand();
        let likelihood = interpretation.likelihood(Seat::North, weak_hand);
        assert!(
            (likelihood - 0.02).abs() < 1e-6,
            "likelihood = {likelihood}"
        );
    }

    #[test]
    fn likelihood_of_a_seat_with_no_calls_is_one() {
        let interpretation = one_call_interpretation();
        let hand = balanced_16_hcp_hand();
        assert_eq!(interpretation.likelihood(Seat::East, hand), 1.0);
    }
}
