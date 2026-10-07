//! `interpret`: auction → constraints.
//!
//! **Step A (per call).** The default, [`InterpretMode::Mirror`], is the calibrated mirror of the
//! bidding policy `p(c|h) = (1 − ε)·[(1 − δ)·S + δ·M] + ε/n` (docs/design/15-phase4-plan.md
//! D18/D19; 07-bidding.md §4). For call `c` at its position the pieces are:
//!
//! - `X_c^(b)`: per branch, the system's exclusive region (the hands whose first satisfied
//!   candidate in `rank_cmp` order has call `c`), taken from the `ExclusiveIndex` and recomputed
//!   at run time when a higher-ranked sibling is illegal after the prefix; raw weight
//!   `(1 − ε)(1 − δ)`;
//! - `N_sys`: the hands with no system candidate, raw weight `(1 − ε)(1 − δ)/n`;
//! - `Y_c`: the natural exclusive region (first satisfied ranked natural candidate, or the
//!   natural implicit `Pass`), raw weight `(1 − ε)·δ` on-system, `1 − ε` off-system;
//! - `N_nat`: the hands with no natural choice, raw weight `(1 − ε)·δ/n` (or `(1 − ε)/n`);
//! - `ANY`, raw weight `ε/n`.
//!
//! The weights are normalised and `log_scale = ln Σ raw` is recorded, so that
//! `p(c|h) = exp(log_scale)·Σ_i w_i·1[h ∈ C_i]` (exact for literal-free pieces; pieces with
//! literals may only over-cover). A call the policy never makes at its position (both `X_c` and
//! `Y_c` empty) is flagged `shadowed` and read by its `Fallback` pieces only. The mirror reads
//! with `table.natural` and the policy of [`InterpretOptions`]; build the options with
//! [`InterpretOptions::for_context`] (and see [`crate::BidContext::natural`]) so that the mirror
//! and the likelihood describe the same policy.
//!
//! [`InterpretMode::Legacy`] keeps the phase-3 Step A until reproduction (iii) (the phase-3
//! continuity metric) is retired: the call's node (one alternative per top-level `Or` branch, or
//! its lenient / natural reading) scaled by `1 − ε` plus a defensive `(ANY, ε, Fallback)` branch,
//! with `ε` set by the resolution kind.
//!
//! **Step B (per seat).** The alternatives of a seat's calls are combined by cross product
//! (`and`; unsatisfiable combinations dropped by a check on precomputed summaries), truncated at
//! each step to `K` combinations by estimated mass `w · cells(summary)` rather than by weight,
//! always keeping the all-`ANY` catch-all combination (so the proposal's support covers the
//! target's), then renormalised. Each call contributes only its own pieces; calls before the
//! divergence point keep their `Exact` reading, which is the operational meaning of "weaken
//! later constraints, not earlier ones".

use bridge_constraint::HandConstraint;
use bridge_core::{Auction, Call, Hand, Seat};
use bridge_system::natural::classify;
use bridge_system::trie::{LookupKey, RelVul, TrieId};
use bridge_system::{CallContext, Forcing, SystemIR};
use smallvec::SmallVec;

use bridge_system::exclusive::PieceSummary;

use crate::exclusion::{MirrorSpec, PieceRole, Reader, mirror_call};
use crate::{BidContext, ImplicitPass, NodeId, PolicyParams, Table};

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
        let len: usize = parts.iter().map(|p| p.text.len() + 3).sum();
        let mut text = String::with_capacity(len);
        for t in parts
            .iter()
            .map(|p| p.text.as_str())
            .filter(|t| !t.is_empty())
        {
            if !text.is_empty() {
                text.push_str(" / ");
            }
            text.push_str(t);
        }
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
    /// Alternatives (the "pieces" of docs/design/15-phase4-plan.md D19); weights sum to 1.
    pub alternatives: Vec<(HandConstraint, f32, CallExplanation)>,
    /// `ln Σ raw` of the pieces' raw weights before normalisation, so that under the policy the
    /// interpretation mirrors, `p(call | h) = exp(log_scale) · Σ_i w_i · 1[h ∈ C_i]` for every
    /// hand `h` of the calling seat (exactly for literal-free pieces; pieces with `cards`/`eval`
    /// literals may only over-cover, never under-cover).
    ///
    /// [`InterpretMode::Legacy`] does not calibrate its weights and records `0.0`.
    pub log_scale: f64,
    /// `true` when the policy never makes this call at its position for any hand (its exclusive
    /// system region and natural region are both empty); the call is then read by its
    /// `Fallback` pieces only.
    ///
    /// [`InterpretMode::Legacy`] never detects this and records `false`.
    pub shadowed: bool,
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

/// Which Step A [`interpret`] runs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum InterpretMode {
    /// The calibrated policy mirror (docs/design/15-phase4-plan.md D19): per call, the pieces
    /// `X_c` (system exclusive region per branch), `N_sys` (no system candidate), `Y_c` (natural
    /// exclusive region), `N_nat` (no natural candidate) and `ANY`, weighted from
    /// [`InterpretOptions::policy`] so that the density of a call's pieces is its policy
    /// probability up to the recorded [`CallInterpretation::log_scale`].
    #[default]
    Mirror,
    /// The phase-3 interpretation: each call's node (or lenient/natural reading) with the
    /// `eps_exact`/`eps_partial`/`eps_natural` fallback mixture and `lenient_decay`. Kept until
    /// reproduction (iii) is retired (docs/design/07-bidding.md §9 item 8)
    /// ([`InterpretOptions::legacy`]).
    Legacy,
}

/// Options for [`interpret`]. Build them with [`InterpretOptions::for_context`] from the same
/// [`BidContext`] the likelihood uses, so the interpretation and the policy cannot drift apart.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct InterpretOptions {
    /// Maximum alternatives kept per seat (default 8).
    pub max_alternatives: usize,
    /// No fallback branches at all (property tests): only the non-`Fallback` pieces are kept.
    pub strict: bool,
    /// Which Step A runs (default [`InterpretMode::Mirror`]).
    pub mode: InterpretMode,
    /// The policy the mirror is calibrated to (ignored by [`InterpretMode::Legacy`]).
    pub policy: PolicyParams,
    /// The implicit-pass rule of the mirrored policy (ignored by [`InterpretMode::Legacy`]).
    pub implicit_pass: ImplicitPass,
    /// Legacy mode only: fallback mass for `Exact` resolutions (default 0.02).
    pub eps_exact: f32,
    /// Legacy mode only: fallback mass for `Partial` resolutions (default 0.15).
    pub eps_partial: f32,
    /// Legacy mode only: fallback mass for `Natural` resolutions (default 0.30).
    pub eps_natural: f32,
    /// Legacy mode only: weight multiplier per opponents'-call substitution in
    /// `resolve_lenient` (default 0.5).
    pub lenient_decay: f32,
}

impl Default for InterpretOptions {
    /// The mirror of [`PolicyParams::system_players`] with `ImplicitPass::Complement`, `K = 8`,
    /// not strict (the legacy knobs at their phase-3 defaults). `Complement` because an unlisted
    /// `Pass` is then read as the complement of its siblings, as the phase-3 interpretation always
    /// read it; under `Never` the policy has no implicit pass and such a pass carries only the
    /// `Fallback` pieces. Prefer [`InterpretOptions::for_context`], which takes both from the
    /// likelihood's `BidContext`.
    fn default() -> InterpretOptions {
        InterpretOptions {
            max_alternatives: 8,
            strict: false,
            mode: InterpretMode::Mirror,
            policy: PolicyParams::system_players(),
            implicit_pass: ImplicitPass::Complement,
            eps_exact: 0.02,
            eps_partial: 0.15,
            eps_natural: 0.30,
            lenient_decay: 0.5,
        }
    }
}

impl InterpretOptions {
    /// The mirror of the policy `ctx` describes: `policy` and `implicit_pass` are taken from
    /// `ctx`, everything else is the default.
    ///
    /// The mirror's natural engine is the table's (`table.natural`), which is the policy's
    /// engine when `ctx.natural` is `None` or `Some(&*table.natural)` (see
    /// [`BidContext::natural`]); to mirror a different engine, interpret with a [`Table`] that
    /// holds it.
    pub fn for_context(ctx: &BidContext<'_>) -> InterpretOptions {
        InterpretOptions {
            policy: ctx.policy,
            implicit_pass: ctx.implicit_pass,
            ..InterpretOptions::default()
        }
    }

    /// The phase-3 interpretation ([`InterpretMode::Legacy`]) with its default ε values.
    pub fn legacy() -> InterpretOptions {
        InterpretOptions {
            mode: InterpretMode::Legacy,
            ..InterpretOptions::default()
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
    // An unrestricted shape set can never make the HCP range cross-check below fail (every HCP
    // from 0 to 37 is reachable by *some* shape), so skip `ShapeSet::hcp_bounds` entirely when
    // `shapes == ALL`; this keeps the cross product's per-combination pre-check cheap enough for
    // the 10 µs budget (07-bidding.md §4.4.2) without changing the result.
    if shapes == bridge_core::ShapeSet::ALL {
        return true;
    }
    let (shapes_min_hcp, shapes_max_hcp) = shapes.hcp_bounds();
    if *hcp.start() > shapes_max_hcp || *hcp.end() < shapes_min_hcp {
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
fn complement_of(sys: &SystemIR, siblings: &[(Call, NodeId)]) -> HandConstraint {
    let combined = siblings
        .iter()
        .map(|(_, id)| sys.node(*id).constraint.clone())
        .reduce(HandConstraint::or)
        .unwrap_or(HandConstraint::ANY);
    combined.not()
}

/// Applies the ε-mixture (07-bidding.md §4.1 step 7, §4.2): scales every alternative by `1 − ε`
/// and appends the defensive `(ANY, ε, Fallback)` branch, unless `opts.strict`.
///
/// Step A's alternatives normally sum to 1. Only a lenient (`Partial`) resolution may sum to
/// less: the `ρ^subst` decay of §4.1 step 5.2 is kept, not normalised away, and the missing mass
/// `1 − Σw` also goes to the `Fallback` branch, so the weights still sum to 1 and a resolution
/// that needed more substitutions carries less confidence. With `opts.strict` there is no
/// `Fallback` branch to receive it, so the alternatives are renormalised instead.
fn apply_epsilon_mixture(
    alts: &mut Vec<(HandConstraint, f32, CallExplanation)>,
    kind: ResolutionKind,
    call_index: usize,
    call: Call,
    opts: &InterpretOptions,
) {
    let total: f32 = alts.iter().map(|(_, w, _)| *w).sum();
    let deficit = if total > 0.0 && total < 1.0 - 1e-6 {
        1.0 - total
    } else {
        0.0
    };
    if opts.strict {
        if deficit > 0.0 {
            normalize_alts(alts);
        }
        return;
    }
    let eps = match kind {
        ResolutionKind::Exact => opts.eps_exact,
        ResolutionKind::Partial { .. } => opts.eps_partial,
        ResolutionKind::Natural => opts.eps_natural,
        ResolutionKind::Fallback => 0.0,
    };
    let eps = eps.max(0.0);
    let fallback = eps + (1.0 - eps) * deficit;
    if fallback <= 0.0 {
        return;
    }
    for (_, w, _) in alts.iter_mut() {
        *w *= 1.0 - eps;
    }
    alts.push((
        HandConstraint::ANY,
        fallback,
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
/// Legacy mode only ([`InterpretMode::Legacy`]); the mirror and `choose_bid` use
/// `exclusion::Reader::partner_context`.
fn partner_context(
    table: &Table,
    per_call_so_far: &[CallInterpretation],
    s: Seat,
) -> (Option<HandConstraint>, bool) {
    let partner = s.partner();
    let Some(last) = per_call_so_far.iter().rev().find(|ci| ci.seat == partner) else {
        return (None, false);
    };
    // An opponent's bid, double or redouble after partner's forcing call releases the obligation
    // to bid: a pass there is a normal action, not the contradictory `pass_forcing`.
    let intervened = per_call_so_far
        .iter()
        .filter(|ci| ci.call_index > last.call_index)
        .any(|ci| ci.seat.side() != s.side() && ci.call != Call::Pass);
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
        Some(_) if intervened => false,
        Some(node_id) => {
            let partner_sys = &table.systems[partner.index() as usize];
            let flags = &partner_sys.node(node_id).flags;
            matches!(flags.forcing, Forcing::OneRound | Forcing::ToGame)
        }
        None => false,
    };
    (Some(c.clone()), forcing)
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
    // An explicit opening `Pass` row states what the pass shows: use it, exactly as for any other
    // listed call (and as `choose_bid` offers it), instead of complementing it together with the
    // opening bids.
    if let Some(&(_, pass_node)) = legal.iter().find(|(c, _)| *c == Call::Pass) {
        let mut alts = Vec::new();
        expand_node_branches(
            sys,
            pass_node,
            j,
            Call::Pass,
            ResolutionKind::Exact,
            1.0,
            &mut alts,
        );
        normalize_alts(&mut alts);
        return (ResolutionKind::Exact, alts);
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
    // Set when our call landed on an implicit-pass trie node: its implicit-pass complement was
    // already computed (or found empty) at the parent position below, and §4.1 step 5.1 must not
    // run again at `lookup.end`, which is the position *after* our pass.
    let mut on_implicit_node = false;

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
        // No row at this depth and no legal system siblings to complement either (07-bidding.md
        // §4.1 step 4). Treat it like a one-short partial match so the lenient/natural machinery
        // below still applies, but skip step 5.1: its complement would be taken at `lookup.end`,
        // whose children are the calls *after* our pass (e.g. the opponents' next call).
        d = n_k - 1;
        on_implicit_node = true;
    }

    *divergence = Some(divergence.map_or(j, |m| m.min(j)));

    // 5.1: implicit pass. Only applies when the *current* call is the one that failed to match
    // (07-bidding.md §4.1 step 5, "d < n_k − 1 の場合...手順5.1を飛ばす").
    if d == n_k - 1 && call == Call::Pass && !on_implicit_node {
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
        // Keep `ρ^subst` (07-bidding.md §4.1 step 5.2): normalise only when several attempts
        // together exceed 1; otherwise `apply_epsilon_mixture` hands the missing mass to the
        // `Fallback` branch.
        let total: f32 = weighted.iter().map(|(_, w)| *w).sum();
        let scale = if total > 1.0 { 1.0 / total } else { 1.0 };
        let mut alts = Vec::new();
        for (node_id, w) in weighted {
            expand_node_branches(
                sys,
                node_id,
                j,
                call,
                ResolutionKind::Partial { matched_depth: d },
                w * scale,
                &mut alts,
            );
        }
        return (ResolutionKind::Partial { matched_depth: d }, alts);
    }

    // 6: natural inference.
    (
        ResolutionKind::Natural,
        natural_alternative(table, auction, s, j, per_call_so_far),
    )
}

/// Legacy Step A ([`InterpretMode::Legacy`]): builds `per_call` and the divergence index.
fn step_a_legacy(
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
            log_scale: 0.0,
            shadowed: false,
        });

        prefix
            .push(call)
            .expect("call from a valid Auction is legal at its own position");
    }

    (per_call, divergence)
}

/// The output of Step A: `per_call`, the divergence index, and per call the summaries of its
/// alternatives and the index of its `ANY` piece (Step B's pre-check, mass ordering and
/// catch-all).
struct StepA {
    per_call: Vec<CallInterpretation>,
    divergence: Option<usize>,
    /// The summaries of every call's alternatives, concatenated: call `j`'s are
    /// `summaries[offsets[j]..offsets[j + 1]]`.
    summaries: Vec<Summary>,
    offsets: Vec<usize>,
    any_index: Vec<Option<usize>>,
}

impl StepA {
    /// The summaries of call `j`'s alternatives.
    fn summaries(&self, j: usize) -> &[Summary] {
        &self.summaries[self.offsets[j]..self.offsets[j + 1]]
    }
}

/// Mirror Step A ([`InterpretMode::Mirror`], docs/design/15-phase4-plan.md D19): per call, the
/// calibrated pieces of `exclusion::mirror_call`, weights normalised with
/// `log_scale = ln Σ raw` recorded.
fn step_a_mirror(table: &Table, auction: &Auction, opts: &InterpretOptions) -> StepA {
    let spec = MirrorSpec {
        table,
        natural: table.natural.as_ref(),
        policy: opts.policy,
        implicit_pass: opts.implicit_pass,
        strict: opts.strict,
        want_text: true,
        membership: false,
    };
    let mut reader = Reader::new(table, table.natural.as_ref(), auction, opts.implicit_pass);
    let calls = auction.calls();
    let n = calls.len();
    let mut per_call = Vec::with_capacity(n);
    let mut summaries = Vec::with_capacity(3 * n);
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(0);
    let mut any_index = Vec::with_capacity(n);
    let mut divergence = None;
    let mut prefix = Auction::new(auction.dealer(), auction.vulnerability());
    for (j, &call) in calls.iter().enumerate() {
        let seat = auction.seat_at(j);
        let mut m = mirror_call(&spec, &mut reader, &prefix, call);
        if m.kind != ResolutionKind::Exact && divergence.is_none() {
            divergence = Some(j);
        }
        let total: f64 = m.pieces.iter().map(|p| p.raw).sum();
        let log_scale = if total > 0.0 {
            total.ln()
        } else {
            f64::NEG_INFINITY
        };
        let sys = &table.systems[seat.index() as usize];
        let mut alternatives = Vec::with_capacity(m.pieces.len());
        let mut any = None;
        // A piece reads its own node's description when that is not the call's node, else the
        // call's text (the non-`Fallback` pieces, and every piece of a shadowed call); the last
        // piece that reads the call's text takes it instead of a clone.
        let own_text = |p: &crate::exclusion::MirrorPiece<'_>| matches!(p.node, Some(id) if Some(id) != m.node);
        let mut text_uses = m
            .pieces
            .iter()
            .filter(|p| !own_text(p) && (!p.role.is_fallback() || m.shadowed))
            .count();
        // Drained in place: the pieces are moved out one by one, the inline buffer is not
        // moved as a whole first.
        for (i, piece) in m.pieces.drain(..).enumerate() {
            let fallback = piece.role.is_fallback();
            if piece.role == PieceRole::Any {
                any = Some(i);
            }
            let kind = match piece.role {
                PieceRole::System => m.kind,
                PieceRole::Natural => ResolutionKind::Natural,
                _ => ResolutionKind::Fallback,
            };
            let text = match piece.node {
                Some(id) if Some(id) != m.node => sys.node(id).description.clone(),
                _ if !fallback || m.shadowed => {
                    text_uses -= 1;
                    if text_uses == 0 {
                        std::mem::take(&mut m.text)
                    } else {
                        m.text.clone()
                    }
                }
                _ => String::new(),
            };
            summaries.push(Summary::of_piece(&piece.summary));
            alternatives.push((
                piece.flat.into_owned(),
                (piece.raw / total) as f32,
                CallExplanation {
                    call_index: j,
                    call,
                    node: if piece.role == PieceRole::System {
                        piece.node
                    } else {
                        None
                    },
                    kind,
                    text,
                },
            ));
        }
        per_call.push(CallInterpretation {
            call_index: j,
            seat,
            call,
            kind: m.kind,
            alternatives,
            log_scale,
            shadowed: m.shadowed,
        });
        offsets.push(summaries.len());
        any_index.push(any);
        prefix
            .push(call)
            .expect("call from a valid Auction is legal at its own position");
    }
    StepA {
        per_call,
        divergence,
        summaries,
        offsets,
        any_index,
    }
}

/// Legacy Step A wrapped as [`StepA`] (summaries computed from the constraints).
fn step_a_legacy_full(table: &Table, auction: &Auction, opts: &InterpretOptions) -> StepA {
    let (per_call, divergence) = step_a_legacy(table, auction, opts);
    let mut summaries = Vec::new();
    let mut offsets = vec![0];
    for ci in &per_call {
        summaries.extend(ci.alternatives.iter().map(|(c, _, _)| Summary::of(c)));
        offsets.push(summaries.len());
    }
    let any_index = per_call
        .iter()
        .map(|ci| {
            ci.alternatives
                .iter()
                .position(|(_, _, ex)| ex.kind == ResolutionKind::Fallback)
        })
        .collect();
    StepA {
        per_call,
        divergence,
        summaries,
        offsets,
        any_index,
    }
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
///
/// [`materialize_constraint`] builds the same tree as a fold of this function in one pass; the
/// fold is kept as the test oracle.
#[cfg(test)]
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

/// A combo's running shape/HCP summary and the `ShapeSet::min_hcp`/`max_hcp` bounds derived from
/// it, kept incrementally instead of being recomputed from the whole constraint tree at every
/// step of the cross product (see [`Combo`]'s doc comment for why this matters).
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
        let bounds = (shapes != bridge_core::ShapeSet::ALL).then(|| shapes.hcp_bounds());
        Summary {
            shapes,
            hcp: c.hcp_range(),
            bounds,
        }
    }

    /// The summary of a piece precomputed by the exclusive index (or the mirror).
    fn of_piece(p: &PieceSummary) -> Summary {
        Summary {
            shapes: p.shapes,
            hcp: p.hcp.clone(),
            bounds: p.shape_hcp_bounds,
        }
    }

    /// The number of (shape, HCP) cells of the summary box: the volume of the mass-ordered
    /// truncation (07-bidding.md §4.4).
    fn cells(&self) -> f32 {
        let span = if self.hcp.is_empty() {
            0
        } else {
            u32::from(*self.hcp.end() - *self.hcp.start()) + 1
        };
        (u32::from(self.shapes.len()) * span) as f32
    }

    /// `self ∧ addition`'s summary, or `None` when the cheap check finds it unsatisfiable (an
    /// empty shape set, an inverted HCP range, or an HCP range no shape in the set can reach —
    /// the same three checks as `summary_satisfiable`, just computed incrementally). The
    /// `min_hcp`/`max_hcp` bounds (`ShapeSet::hcp_bounds`, at most 72 per-byte table lookups)
    /// only run when the shape set actually narrows from `self`'s, not on every combination; when it does
    /// not narrow, `self.bounds` is still valid for the new (possibly HCP-narrower) range and is
    /// reused as-is (07-bidding.md §4.4.2's `interpret < 10 µs` budget).
    fn and(&self, addition: &Summary) -> Option<Summary> {
        let hcp = clamp_hcp(&self.hcp, &addition.hcp);
        if hcp.is_empty() {
            return None;
        }
        // `bounds` is `None` exactly when `shapes == ShapeSet::ALL`, so an unrestricted side
        // leaves the other side's shapes and bounds as they are (no intersection, no walk).
        let (shapes, bounds) = match (self.bounds, addition.bounds) {
            (_, None) => (self.shapes, self.bounds),
            (None, _) => (addition.shapes, addition.bounds),
            _ => {
                let shapes = self.shapes.intersect(addition.shapes);
                if shapes.is_empty() {
                    return None;
                }
                let bounds = if shapes == self.shapes {
                    self.bounds
                } else if shapes == addition.shapes {
                    addition.bounds
                } else {
                    Some(shapes.hcp_bounds())
                };
                (shapes, bounds)
            }
        };
        if shapes.is_empty() {
            return None;
        }
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

/// One partial cross-product combination for Step B.
///
/// `key[level]` is the index of the alternative the combination takes at the seat's `level`-th
/// call: since calls are folded in a fixed order (`seat_calls`, built once per seat below), that
/// is enough to look the level's actual `CallExplanation` *and* `HandConstraint` back up
/// afterwards (`materialize_parts`/`materialize_constraint`). This avoids cloning a
/// `Vec<CallExplanation>` (each element owning a `String`) or rebuilding the `HandConstraint::And`
/// tree at every intermediate combination: only the small index key is copied during the cross
/// product, and the real `CallExplanation`s/`HandConstraint` are materialised once per surviving
/// (post-truncation) combo instead of once per intermediate one (07-bidding.md §6.2's
/// `interpret < 10 µs` target). The key stores up to 8 levels inline, spilling to the heap only
/// for a seat with more calls than that.
///
/// `summary` is the running `Summary` of the combo's (not-yet-materialised) constraint (see
/// [`Summary::and`]): keeping it incrementally, instead of recomputing
/// `constraint.shapes()`/`hcp_range()` (a walk of the whole `And` tree) and then
/// `ShapeSet::hcp_bounds` (at most 72 per-byte table lookups) from scratch at every combination,
/// is what keeps the cross-product's per-combination pre-check cheap once a node's constraint
/// carries a real suit-length or shape atom (`summary_satisfiable`'s `shapes == ShapeSet::ALL`
/// shortcut alone only covers bare-HCP atoms). `mass` (`weight · cells(summary)`, the truncation
/// order) is computed once per combination rather than once per sort comparison.
type ComboKey = SmallVec<[u16; 8]>;

struct Combo {
    summary: Summary,
    weight: f32,
    mass: f32,
    key: ComboKey,
    /// Every level so far took its call's `ANY` piece (the catch-all combination).
    catch_all: bool,
}

/// Rebuilds a surviving combo's `Vec<CallExplanation>` from its `key` and the seat's own calls in
/// fold order (see [`Combo`]'s doc comment).
fn materialize_parts(seat_calls: &[&CallInterpretation], key: &[u16]) -> Vec<CallExplanation> {
    key.iter()
        .enumerate()
        .map(|(level, &alt)| seat_calls[level].alternatives[usize::from(alt)].2.clone())
        .collect()
}

/// Rebuilds a surviving combo's `HandConstraint` (the `And` of one alternative per call) from its
/// `key` and the seat's own calls in fold order (see [`Combo`]'s doc comment): the same tree
/// `and_one_more` would have built incrementally, but assembled once instead of once per
/// intermediate cross-product candidate. An empty `key` (a seat with no calls) is `ANY`, matching
/// [`Summary::ANY`]/the initial `Combo`.
fn materialize_constraint(seat_calls: &[&CallInterpretation], key: &[u16]) -> HandConstraint {
    // The fold `acc = and_one_more(&acc, alt)` over the key, built without its intermediate
    // clones: `ANY` alternatives are skipped, the first remaining one is the seed (its children
    // when it is an `And`), and every later one is appended as one child.
    let mut parts = key
        .iter()
        .enumerate()
        .map(|(level, &alt)| &seat_calls[level].alternatives[usize::from(alt)].0)
        .filter(|c| !is_any(c));
    let Some(first) = parts.next() else {
        return HandConstraint::ANY;
    };
    let rest: SmallVec<[&HandConstraint; 8]> = parts.collect();
    if rest.is_empty() {
        return first.clone();
    }
    let mut v = match first {
        HandConstraint::And(children) => {
            let mut v = Vec::with_capacity(children.len() + rest.len());
            v.extend_from_slice(children);
            v
        }
        other => {
            let mut v = Vec::with_capacity(1 + rest.len());
            v.push(other.clone());
            v
        }
    };
    v.extend(rest.into_iter().cloned());
    HandConstraint::And(v)
}

/// Step B: combines `per_call` into the four seats' weighted disjunctions (07-bidding.md §4.4):
/// the cross product of each seat's calls' pieces, pruned by the precomputed summaries,
/// truncated at each step to `K` combinations by estimated mass `w · cells(summary)` (not by
/// `w`), always keeping the all-`ANY` catch-all combination, then renormalised.
fn step_b(a: &StepA, opts: &InterpretOptions) -> [Vec<(HandConstraint, f32, Explanation)>; 4] {
    let per_call = &a.per_call;
    let mut seats: [Vec<(HandConstraint, f32, Explanation)>; 4] = Default::default();
    let k = opts.max_alternatives.max(1);

    // Two buffers reused across seats and levels (the cross product allocates nothing else).
    let mut combos: Vec<Combo> = Vec::with_capacity(4 * k);
    let mut next: Vec<Combo> = Vec::with_capacity(4 * k);
    for seat in Seat::ALL {
        let seat_idx: SmallVec<[usize; 8]> = (0..per_call.len())
            .filter(|&j| per_call[j].seat == seat)
            .collect();
        let seat_calls: SmallVec<[&CallInterpretation; 8]> =
            seat_idx.iter().map(|&j| &per_call[j]).collect();
        combos.clear();
        combos.push(Combo {
            summary: Summary::ANY,
            weight: 1.0,
            mass: 0.0,
            key: ComboKey::new(),
            catch_all: true,
        });
        let had_calls = !seat_calls.is_empty();

        for (level, cj) in seat_calls.iter().enumerate() {
            let j = seat_idx[level];
            let alt_summaries = a.summaries(j);
            let any = a.any_index[j];
            next.clear();
            for combo in &combos {
                for (i, (_, wi, _)) in cj.alternatives.iter().enumerate() {
                    let Some(summary) = combo.summary.and(&alt_summaries[i]) else {
                        continue;
                    };
                    let mut key = combo.key.clone();
                    key.push(u16::try_from(i).expect("fewer than 65536 alternatives per call"));
                    let weight = combo.weight * wi;
                    next.push(Combo {
                        mass: weight * summary.cells(),
                        summary,
                        weight,
                        key,
                        catch_all: combo.catch_all && any == Some(i),
                    });
                }
            }
            if next.len() > k {
                next.sort_by(|x, y| y.mass.total_cmp(&x.mass));
                match next.iter().position(|c| c.catch_all) {
                    Some(ca) if ca >= k => {
                        let catch_all = next.swap_remove(ca);
                        next.truncate(k - 1);
                        next.push(catch_all);
                    }
                    _ => next.truncate(k),
                }
            }
            std::mem::swap(&mut combos, &mut next);
        }

        if had_calls && combos.is_empty() {
            tracing::warn!(?seat, "seat contradicts itself");
            combos.push(Combo {
                summary: Summary::ANY,
                weight: 1.0,
                mass: 0.0,
                key: ComboKey::new(),
                catch_all: true,
            });
        }

        // Highest weight first (the order callers and explanations expect).
        combos.sort_by(|x, y| y.weight.total_cmp(&x.weight));
        let total: f32 = combos.iter().map(|c| c.weight).sum();
        let idx = seat.index() as usize;
        seats[idx] = combos
            .drain(..)
            .map(|c| {
                let w = if total > 0.0 {
                    c.weight / total
                } else {
                    c.weight
                };
                let parts = materialize_parts(&seat_calls, &c.key);
                let constraint = materialize_constraint(&seat_calls, &c.key);
                (constraint, w, Explanation::from_parts(parts))
            })
            .collect();
        if seats[idx].is_empty() {
            seats[idx].push((HandConstraint::ANY, 1.0, Explanation::empty()));
        }
    }

    seats
}

/// Step A of [`interpret`] alone: the per-call interpretations (`Interpretation::per_call`),
/// without Step B's per-seat combination. For benches that report the Step A / Step B split.
#[doc(hidden)]
pub fn interpret_per_call(
    table: &Table,
    auction: &Auction,
    opts: &InterpretOptions,
) -> Vec<CallInterpretation> {
    match opts.mode {
        InterpretMode::Mirror => step_a_mirror(table, auction, opts),
        InterpretMode::Legacy => step_a_legacy_full(table, auction, opts),
    }
    .per_call
}

/// Interprets `auction` under the four systems of `table`.
pub fn interpret(table: &Table, auction: &Auction, opts: &InterpretOptions) -> Interpretation {
    let a = match opts.mode {
        InterpretMode::Mirror => step_a_mirror(table, auction, opts),
        InterpretMode::Legacy => step_a_legacy_full(table, auction, opts),
    };
    let seats = step_b(&a, opts);
    Interpretation {
        seats,
        per_call: a.per_call,
        divergence: a.divergence,
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
            log_scale: 0.0,
            shadowed: false,
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

    #[test]
    fn materialize_constraint_equals_the_and_one_more_fold() {
        let a = balanced_15_17();
        let b = HandConstraint::And(vec![balanced_15_17(), HandConstraint::ANY.not()]);
        let any = HandConstraint::ANY;
        let alternatives = [a.clone(), b.clone(), any.clone()];
        let interp = one_call_interpretation();
        let template = &interp.per_call[0];
        let mut calls: Vec<CallInterpretation> = Vec::new();
        for _ in 0..3 {
            let mut c = template.clone();
            c.alternatives = alternatives
                .iter()
                .map(|x| (x.clone(), 1.0, template.alternatives[0].2.clone()))
                .collect();
            calls.push(c);
        }
        let refs: Vec<&CallInterpretation> = calls.iter().collect();
        for i in 0..3u16 {
            for j in 0..3u16 {
                for k in 0..3u16 {
                    let key = [i, j, k];
                    let fold =
                        key.iter()
                            .enumerate()
                            .fold(HandConstraint::ANY, |acc, (level, &alt)| {
                                and_one_more(&acc, &refs[level].alternatives[usize::from(alt)].0)
                            });
                    assert_eq!(
                        format!("{:?}", materialize_constraint(&refs, &key)),
                        format!("{fold:?}"),
                        "{i} {j} {k}"
                    );
                }
            }
        }
    }
}
