//! `choose_bid`: hand + auction → call.
//!
//! 1. Candidates: the system's continuations for the prefix (falling back to `resolve_lenient`),
//!    or the natural engine's when the prefix is off-system and `ctx.natural` is set.
//! 2. Illegal candidates (per `Auction::is_legal`) become `Tried { Illegal }` plus a
//!    `Diagnostic::IllegalSystemCall` (a system-definition lint, never a panic); candidates
//!    whose constraint the hand fails become `Tried { Unsatisfied }`.
//! 3. With `ImplicitPass::Complement`, a `Pass` with the complement of the siblings' constraints
//!    is synthesised when none is listed (the interpreter uses the same complement, see
//!    `interpret::complement_of`).
//! 4. Sort by `bridge_system::exclusive::rank_cmp_keys`: priority descending, then
//!    `SystemMeta::tie_break`, then call index ascending (the one rank order shared with the
//!    exclusive index and the natural ranking).
//! 5. Empty → `NoCandidate`; otherwise `Chosen` with every survivor in `alternatives`.

use std::collections::HashMap;

use bridge_core::{Auction, Call, Hand};
use bridge_system::exclusive::{RankKey, rank_cmp_keys};
use bridge_system::natural::classify;
use bridge_system::{LookupKey, NaturalCandidate, NaturalInference, PartnerContext, RelVul};

use crate::interpret::{
    LENIENT_MAX_SUBST, complement_of, partner_context_for_prefix, summary_satisfiable,
};
use crate::{BidContext, ImplicitPass, NodeId, SystemIR, Table};

/// The outcome of a bidding decision. `NoCandidate` is information about the system's coverage,
/// not an error.
#[derive(Clone, Debug)]
pub enum BidChoice {
    /// A call was chosen.
    Chosen(Chosen),
    /// No listed call applies.
    NoCandidate(NoCandidate),
}

impl BidChoice {
    /// The chosen call, if any.
    pub fn call(&self) -> Option<Call> {
        match self {
            BidChoice::Chosen(c) => Some(c.call),
            BidChoice::NoCandidate(_) => None,
        }
    }

    /// `true` for [`BidChoice::Chosen`].
    pub fn is_chosen(&self) -> bool {
        matches!(self, BidChoice::Chosen(_))
    }
}

/// Where a chosen call came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(missing_docs)]
pub enum ChoiceSource {
    System,
    Natural,
    ImplicitPass,
}

/// A chosen call.
#[derive(Clone, Debug)]
pub struct Chosen {
    /// The call.
    pub call: Call,
    /// Its node (`None` for natural / implicit pass).
    pub node: Option<NodeId>,
    /// Source.
    pub source: ChoiceSource,
    /// Explanation text.
    pub explanation: String,
    /// Every satisfying legal candidate, sorted; the chosen one first.
    pub alternatives: Vec<Alternative>,
    /// System-definition problems noticed on the way.
    pub diagnostics: Vec<Diagnostic>,
}

/// A candidate that survived filtering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Alternative {
    /// The call.
    pub call: Call,
    /// Its node.
    pub node: Option<NodeId>,
    /// Its priority.
    pub priority: i16,
}

/// No candidate applied.
#[derive(Clone, Debug)]
pub struct NoCandidate {
    /// Every candidate and why it was rejected.
    pub tried: Vec<Tried>,
    /// System-definition problems noticed on the way.
    pub diagnostics: Vec<Diagnostic>,
}

/// A rejected candidate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tried {
    /// The node.
    pub node: NodeId,
    /// Its call.
    pub call: Call,
    /// Why it was rejected.
    pub reason: Rejected,
}

/// Rejection reasons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(missing_docs)]
pub enum Rejected {
    Unsatisfied,
    Illegal,
    NotApplicable,
}

/// A problem in the system definition found while bidding.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Diagnostic {
    /// The system lists a call that is illegal at this point.
    IllegalSystemCall {
        /// The node.
        node: NodeId,
        /// The call.
        call: Call,
    },
    /// Two nodes offer the same call at the same point.
    DuplicateCandidate {
        /// First node.
        node_a: NodeId,
        /// Second node.
        node_b: NodeId,
        /// The call.
        call: Call,
    },
    /// A node's constraint is unsatisfiable.
    UnsatisfiableNode {
        /// The node.
        node: NodeId,
    },
}

/// A surviving candidate, before sorting (07-bidding.md §5.2).
pub(crate) struct Kept {
    pub(crate) call: Call,
    pub(crate) node: Option<NodeId>,
    pub(crate) priority: i16,
    pub(crate) source: ChoiceSource,
}

/// The natural candidates of a position in rank order, with the partner context they were
/// inferred under (so the winner's explanation can be rebuilt without recomputing it).
pub(crate) struct NaturalRanked {
    pub(crate) ranked: Vec<NaturalCandidate>,
    pub(crate) partner: PartnerContext,
    /// Whether `Pass` is itself one of the natural candidates (then there is no natural implicit
    /// pass).
    pub(crate) pass_listed: bool,
}

/// The output of [`gather`].
pub(crate) struct Gathered {
    /// Surviving candidates, unsorted.
    pub(crate) kept: Vec<Kept>,
    /// Rejected system candidates.
    pub(crate) tried: Vec<Tried>,
    /// System-definition problems noticed on the way.
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// The position has at least one legal system candidate (exact resolve, or the first full
    /// lenient match): `kept` then holds system candidates and the implicit pass only.
    pub(crate) on_system: bool,
    /// Off-system with a natural engine: the ranked natural candidates `kept` was filtered from.
    pub(crate) natural: Option<NaturalRanked>,
}

/// The ranked natural candidates for the next call after `auction` (07-bidding.md §5.2 step
/// 1.4), with `CallContext::partner_constraint`/`forcing_situation` filled from the prefix's own
/// Step A exactly as `interpret`'s natural step fills them, so the two agree on what a natural
/// call shows (bidirectional consistency, §2.3).
pub(crate) fn natural_ranked(
    table: &Table,
    auction: &Auction,
    natural: &NaturalInference,
) -> NaturalRanked {
    let seat = auction.next_seat();
    let system = &table.systems[seat.index() as usize];
    let (partner_constraint, forcing_situation) = partner_context_for_prefix(table, auction, seat);
    let partner = PartnerContext {
        partner_constraint,
        forcing_situation,
    };
    let ranked = natural.ranked_candidates(auction, seat, &partner, system.meta.tie_break);
    let pass_listed = ranked.iter().any(|c| c.call == Call::Pass);
    NaturalRanked {
        ranked,
        partner,
        pass_listed,
    }
}

/// The natural policy's choice `m_P(h)` (docs/design/15-phase4-plan.md D18): the first ranked
/// candidate `hand` satisfies; else the natural implicit `Pass` under
/// `ImplicitPass::Complement` when `Pass` is not itself a candidate; else `None`.
pub(crate) fn natural_choice(
    ranked: &NaturalRanked,
    hand: Hand,
    ctx: &BidContext<'_>,
) -> Option<Call> {
    ranked
        .ranked
        .iter()
        .find(|c| c.constraint.satisfies(hand))
        .map(|c| c.call)
        .or_else(|| {
            (ctx.implicit_pass == ImplicitPass::Complement && !ranked.pass_listed)
                .then_some(Call::Pass)
        })
}

/// Steps 1–3 of §5.2, shared by [`choose_bid`] and `policy::call_distribution` (which needs the
/// same surviving-candidate set to score calls, but not the final sort/pick).
///
/// Takes the whole `table`, not just the acting seat's own `SystemIR`: the natural branch needs
/// the interpretation of the auction so far (partner's system included) to fill
/// `CallContext::partner_constraint`/`forcing_situation` exactly as `interpret`'s natural step
/// does, so the two agree on what a natural call shows.
pub(crate) fn gather(
    table: &Table,
    hand: Hand,
    auction: &Auction,
    ctx: &BidContext<'_>,
) -> Gathered {
    let seat = auction.next_seat();
    let system = &table.systems[seat.index() as usize];
    let vulnerability = auction.vulnerability();
    let vul = RelVul {
        we: vulnerability.is_vulnerable(seat),
        they: vulnerability.is_vulnerable(seat.next()),
    };
    let key = match LookupKey::for_auction(auction, seat) {
        Some(key) => key,
        None => LookupKey {
            we_opened: true,
            calls: &[],
            opener_pos: auction.position_of(seat),
            vul,
        },
    };

    let mut diagnostics = Vec::new();
    let mut tried = Vec::new();
    let mut kept: Vec<Kept> = Vec::new();
    let mut legal_system_candidates: Vec<(Call, NodeId)> = Vec::new();
    let mut pass_offered = false;

    let lookup = system.index.resolve(&key);
    // Whether `system_children` came from the exact resolve or from `resolve_lenient` (the
    // opponents' real call substituted by `Pass`, "system on"). This gates both the
    // `IllegalSystemCall` lint below and the implicit-pass synthesis further down, so that
    // `choose_bid` only ever treats a position as fully on-system under the same condition
    // `interpret`'s Step A does (07-bidding.md §4.1.5.1/§5.2 step 3) — otherwise the two disagree
    // whenever an opponents' off-system call is more than one substitution away from a match
    // (`interpret` falls through to Natural there, `choose_bid` must too, not synthesise a Pass
    // from a lenient sibling that Natural would never reproduce).
    let exact_match = lookup.matched_depth == key.calls.len();
    let system_children: Option<Vec<(Call, NodeId)>> = if exact_match {
        Some(system.index.children(lookup.end, key.opener_pos, key.vul))
    } else {
        system
            .index
            .resolve_lenient(&key, LENIENT_MAX_SUBST)
            .into_iter()
            .find(|(lk, _)| lk.matched_depth == key.calls.len())
            .map(|(lk, _)| system.index.children(lk.end, key.opener_pos, key.vul))
    };

    if let Some(children) = &system_children {
        for &(call, node_id) in children {
            if call == Call::Pass {
                pass_offered = true;
            }
            if !auction.is_legal(call) {
                tried.push(Tried {
                    node: node_id,
                    call,
                    reason: Rejected::Illegal,
                });
                // A compiled system never lists an exact-resolve child that is illegal at its own
                // position (the compiler drops those via `Lint::IllegalCall`). A lenient-derived
                // child, though, stands for a *different*, substituted sequence and is routinely
                // illegal against the real auction (e.g. a raise that is fine "as if they had
                // passed" but not after their real overcall) — that is not a system-definition
                // problem, so only an exact-resolve candidate is worth the lint.
                if exact_match {
                    diagnostics.push(Diagnostic::IllegalSystemCall {
                        node: node_id,
                        call,
                    });
                }
                continue;
            }
            legal_system_candidates.push((call, node_id));
            let node = system.node(node_id);
            if !summary_satisfiable(&node.constraint) {
                diagnostics.push(Diagnostic::UnsatisfiableNode { node: node_id });
            }
            if !node.constraint.satisfies(hand) {
                tried.push(Tried {
                    node: node_id,
                    call,
                    reason: Rejected::Unsatisfied,
                });
                continue;
            }
            kept.push(Kept {
                call,
                node: Some(node_id),
                priority: node.priority,
                source: ChoiceSource::System,
            });
        }
    }

    // A position with no legal continuation in the system (a leaf reached through an opponents'
    // edge that only a deeper row put in the trie, or a lenient match whose children are all
    // illegal here) is off-system for every call we could make: `interpret` resolves each of them
    // as `Natural` (§4.1 step 5.1 finds no sibling, `resolve_lenient` no node), so the natural
    // candidates answer here too instead of `NoCandidate`. Illegal system children are still
    // reported above.
    let off_system = legal_system_candidates.is_empty();
    let mut natural_out = None;
    if off_system {
        if let Some(natural) = ctx.natural {
            // `NaturalInference::candidates` builds each call's `CallContext` with a bare
            // `classify`, leaving `partner_constraint`/`forcing_situation` at `None`/`false`,
            // whereas `interpret`'s natural step fills both from the prefix's interpretation
            // before `infer`. Rules that read them (`rule_cue`'s `min_hcp`, `rule_pass_forcing`)
            // would then give a different constraint here than `interpret` gives for the same
            // call, breaking the bidirectional consistency of 07-bidding.md §2.3. So this branch
            // uses `natural_ranked`, which fills them the same way `interpret` does.
            let ranked = natural_ranked(table, auction, natural);
            for cand in &ranked.ranked {
                if cand.call == Call::Pass {
                    pass_offered = true;
                }
                if !cand.constraint.satisfies(hand) {
                    // Natural candidates are not system nodes, so a rejection is not reported
                    // (`Tried` and `IllegalSystemCall`/`UnsatisfiableNode` only ever reference a
                    // system `NodeId`; see 07-bidding.md §5.2).
                    continue;
                }
                kept.push(Kept {
                    call: cand.call,
                    node: None,
                    priority: cand.priority(),
                    source: ChoiceSource::Natural,
                });
            }
            natural_out = Some(ranked);
        }
    }

    // Duplicate-candidate diagnostic: the same call offered by two different system nodes.
    let mut seen: HashMap<Call, NodeId> = HashMap::new();
    for k in &kept {
        if let Some(node_id) = k.node {
            match seen.get(&k.call) {
                Some(&first) if first != node_id => {
                    diagnostics.push(Diagnostic::DuplicateCandidate {
                        node_a: first,
                        node_b: node_id,
                        call: k.call,
                    });
                }
                Some(_) => {}
                None => {
                    seen.insert(k.call, node_id);
                }
            }
        }
    }

    // Implicit pass (07-bidding.md §5.2 step 3). Only synthesised against *system* siblings from
    // the *exact* resolve (`exact_match`), so that the complement matches the one `interpret`'s
    // Step A §4.1.5.1 would compute at the same point (bidirectional consistency, 07-bidding.md
    // §2.3): `interpret` only ever takes its own implicit-pass branch when the direct (non-lenient)
    // resolve stalls exactly one call short, never when it only succeeds after substituting an
    // opponents' call via `resolve_lenient`.
    if ctx.implicit_pass == ImplicitPass::Complement
        && exact_match
        && !pass_offered
        && auction.is_legal(Call::Pass)
        && !legal_system_candidates.is_empty()
    {
        let complement = complement_of(system, &legal_system_candidates);
        if complement.satisfies(hand) {
            kept.push(Kept {
                call: Call::Pass,
                node: None,
                priority: i16::MIN + 1,
                source: ChoiceSource::ImplicitPass,
            });
        }
    }

    Gathered {
        kept,
        tried,
        diagnostics,
        on_system: !off_system,
        natural: natural_out,
    }
}

/// Sorts `kept` into the single rank order ([`rank_cmp_keys`]): priority descending, then
/// `system.meta.tie_break`, then call index ascending.
pub(crate) fn sort_kept(system: &SystemIR, kept: &mut [Kept]) {
    kept.sort_by(|a, b| {
        rank_cmp_keys(
            system,
            &RankKey {
                call: a.call,
                priority: a.priority,
                node: a.node,
            },
            &RankKey {
                call: b.call,
                priority: b.priority,
                node: b.node,
            },
        )
    });
}

/// Chooses a call for `hand` after `auction` under `table` (its acting seat's system, or its
/// natural fallback; see `gather`'s doc comment (in this module) for why the whole `table` is
/// needed rather than just the acting seat's own [`SystemIR`]).
pub fn choose_bid(table: &Table, hand: Hand, auction: &Auction, ctx: &BidContext<'_>) -> BidChoice {
    let seat = auction.next_seat();
    let system = &table.systems[seat.index() as usize];
    let Gathered {
        mut kept,
        tried,
        diagnostics,
        natural: natural_ranked_out,
        ..
    } = gather(table, hand, auction, ctx);
    sort_kept(system, &mut kept);

    if kept.is_empty() {
        return BidChoice::NoCandidate(NoCandidate { tried, diagnostics });
    }

    let winner_call = kept[0].call;
    let winner_node = kept[0].node;
    let winner_source = kept[0].source;
    let alternatives: Vec<Alternative> = kept
        .iter()
        .map(|k| Alternative {
            call: k.call,
            node: k.node,
            priority: k.priority,
        })
        .collect();

    let explanation = match winner_node {
        Some(node_id) => system.node(node_id).description.clone(),
        None => match winner_source {
            ChoiceSource::Natural => {
                let natural = ctx
                    .natural
                    .expect("a Natural-sourced candidate implies a natural engine");
                let ranked = natural_ranked_out
                    .as_ref()
                    .expect("a Natural-sourced candidate implies ranked natural candidates");
                let next = auction
                    .with(winner_call)
                    .expect("a kept candidate is legal");
                let mut call_ctx = classify(&next, auction.len(), seat);
                call_ctx.partner_constraint = ranked.partner.partner_constraint.clone();
                call_ctx.forcing_situation = ranked.partner.forcing_situation;
                let inf = natural.infer(&call_ctx);
                format!("{} ({})", inf.explanation, inf.rule)
            }
            ChoiceSource::ImplicitPass => "implicit pass".to_string(),
            ChoiceSource::System => String::new(),
        },
    };

    BidChoice::Chosen(Chosen {
        call: winner_call,
        node: winner_node,
        source: winner_source,
        explanation,
        alternatives,
        diagnostics,
    })
}
