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
    pub fn satisfied_by(&self, seat: Seat, hand: Hand) -> bool {
        todo!("phase 3")
    }

    /// `Σ w_i · [C_i ∋ hand]`: the set-membership mass of `hand` under the mixture. This is not
    /// the bidding-policy likelihood (see `sequence_log_likelihood`).
    pub fn likelihood(&self, seat: Seat, hand: Hand) -> f32 {
        todo!("phase 3")
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

/// Fills `ctx.partner_constraint` / `ctx.forcing_situation` from the interpretation of partner's
/// calls so far (07-bidding.md §2.2): the maximum-weight alternative of partner's most recent
/// call, and whether that alternative's node is forcing.
fn fill_partner_context(
    mut ctx: CallContext,
    table: &Table,
    per_call_so_far: &[CallInterpretation],
    s: Seat,
) -> CallContext {
    let partner = s.partner();
    if let Some(last) = per_call_so_far.iter().rev().find(|ci| ci.seat == partner) {
        if let Some((c, _, ex)) = last.alternatives.iter().max_by(|a, b| a.1.total_cmp(&b.1)) {
            ctx.partner_constraint = Some(c.clone());
            if let Some(node_id) = ex.node {
                let partner_sys = &table.systems[partner.index() as usize];
                let flags = &partner_sys.node(node_id).flags;
                ctx.forcing_situation =
                    matches!(flags.forcing, Forcing::OneRound | Forcing::ToGame);
            }
        }
    }
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
        // Defensive: the depth matched but our own call has no attached row (07-bidding.md does
        // not spell out this case). Treat it like a one-short partial match so the fallback
        // machinery below still applies instead of panicking.
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

/// `existing ∧ addition`, allocating the new `And`'s backing `Vec` once at its final size instead
/// of `HandConstraint::and`'s clone-then-push (which, starting from an already-owned `Vec` of
/// exactly `len` capacity, reallocates again on the `push`). Called once per surviving candidate
/// in Step B's cross product, so this compounds; see [`Combo`]'s doc comment for the same
/// reasoning applied to `key`.
fn and_one_more(existing: &HandConstraint, addition: &HandConstraint) -> HandConstraint {
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
struct Combo {
    constraint: HandConstraint,
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
            weight: 1.0,
            key: Vec::new(),
        }];
        let had_calls = !seat_calls.is_empty();

        for cj in &seat_calls {
            let mut next: Vec<Combo> = Vec::new();
            for combo in &combos {
                for (i, (ci, wi, ex)) in cj.alternatives.iter().enumerate() {
                    let c2 = and_one_more(&combo.constraint, ci);
                    if !summary_satisfiable(&c2) {
                        continue;
                    }
                    // `with_capacity` + `extend_from_slice` (one allocation, sized exactly right)
                    // instead of `combo.key.clone()` then `push` (which can reallocate a second
                    // time): `key` is rebuilt on every surviving candidate in this loop, so the
                    // saving compounds across the cross product.
                    let mut key = Vec::with_capacity(combo.key.len() + 1);
                    key.extend_from_slice(&combo.key);
                    key.push((ex.node, ex.kind, i));
                    next.push(Combo {
                        constraint: c2,
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
