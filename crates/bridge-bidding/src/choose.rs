//! `choose_bid`: hand + auction → call.
//!
//! 1. Candidates: the system's continuations for the prefix (falling back to `resolve_lenient`),
//!    or the natural engine's when the prefix is off-system and `ctx.natural` is set. The
//!    hand-independent part is `enumerate_position`, shared with `call_distribution` and
//!    `interpret`'s policy mirror.
//! 2. Illegal candidates (per `Auction::is_legal`) become `Tried { Illegal }` plus a
//!    `Diagnostic::IllegalSystemCall` (a system-definition lint, never a panic); candidates
//!    whose constraint the hand fails become `Tried { Unsatisfied }`.
//! 3. With `ImplicitPass::Complement`, a `Pass` with the complement of the siblings' constraints
//!    is synthesised when none is listed (the interpreter reads the pass as the same complement,
//!    the sibling group's `ExclusiveGroup::complement`); in the natural branch, a hand that fits
//!    no natural candidate passes (the natural implicit pass).
//! 4. Sort by `bridge_system::exclusive::rank_cmp_keys`: priority descending, then
//!    `SystemMeta::tie_break`, then call index ascending (the one rank order shared with the
//!    exclusive index and the natural ranking).
//! 5. Empty → `NoCandidate`; otherwise `Chosen` with every survivor in `alternatives`.

use std::collections::HashMap;

use bridge_core::{Auction, Call, Hand, Seat};
use bridge_system::exclusive::{RankKey, rank_cmp_keys};
use bridge_system::natural::classify;
use bridge_system::trie::TrieId;
use bridge_system::{
    ExclusiveGroup, LookupKey, NaturalCandidate, NaturalInference, PartnerContext, RelVul,
};

use crate::exclusion::partner_context;
use crate::interpret::{LENIENT_MAX_SUBST, summary_satisfiable};
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
}

/// The hand-independent candidates of one position: 07-bidding.md §5.2 steps 1–3 without the
/// hand filter. `choose_bid` (through [`gather`]), `call_distribution` and `interpret`'s mirror
/// all start from this one enumeration, so they can never disagree on the candidate set, on
/// whether the position is on-system, or on whether an implicit `Pass` is synthesised.
pub(crate) struct Position<'a> {
    /// The acting seat.
    pub(crate) seat: Seat,
    /// Its system.
    pub(crate) system: &'a SystemIR,
    /// The opener position of the lookup key.
    pub(crate) opener_pos: u8,
    /// The relative vulnerability of the lookup key.
    pub(crate) vul: RelVul,
    /// The exact resolve matched the whole prefix (otherwise the candidates, if any, come from
    /// the first full `resolve_lenient` match).
    pub(crate) exact_match: bool,
    /// Calls matched by the exact resolve.
    pub(crate) matched_depth: usize,
    /// The trie position whose children are the system candidates (`None`: neither the exact
    /// resolve nor any lenient attempt matched the whole prefix).
    pub(crate) end: Option<TrieId>,
    /// The system children `(call, node, legal here)` in `AuctionTrie::children` order.
    pub(crate) children: Vec<(Call, NodeId, bool)>,
    /// Some system child is illegal here (a lenient match or a wildcard subtree).
    pub(crate) any_illegal: bool,
    /// Whether the policy synthesises the system implicit `Pass` here: `ImplicitPass::Complement`,
    /// an exact match, no listed `Pass`, and at least one legal system candidate.
    pub(crate) implicit_pass: bool,
    /// Number of legal calls.
    pub(crate) n_legal: usize,
}

impl<'a> Position<'a> {
    /// The position is on-system: at least one legal system candidate.
    pub(crate) fn on_system(&self) -> bool {
        self.children.iter().any(|&(_, _, legal)| legal)
    }

    /// The legal system candidates `(call, node)` in children order.
    pub(crate) fn legal(&self) -> impl Iterator<Item = (Call, NodeId)> + '_ {
        self.children
            .iter()
            .filter(|&&(_, _, legal)| legal)
            .map(|&(c, n, _)| (c, n))
    }

    /// The resolution kind of a system reading at this position.
    pub(crate) fn system_kind(&self) -> crate::ResolutionKind {
        if self.exact_match {
            crate::ResolutionKind::Exact
        } else {
            crate::ResolutionKind::Partial {
                matched_depth: self.matched_depth,
            }
        }
    }

    /// The precomputed sibling group of this position (`None` off-system).
    pub(crate) fn group(&self) -> Option<&'a ExclusiveGroup> {
        let end = self.end?;
        self.system
            .exclusive()
            .group_for(end, self.opener_pos, self.vul)
    }
}

/// The lookup key `choose_bid` uses for `seat` about to call after `auction` (the root with
/// `we_opened` when nobody has bid yet).
pub(crate) fn policy_key(auction: &Auction, seat: Seat) -> LookupKey<'_> {
    match LookupKey::for_auction(auction, seat) {
        Some(key) => key,
        None => {
            let vulnerability = auction.vulnerability();
            LookupKey {
                we_opened: true,
                calls: &[],
                opener_pos: auction.position_of(seat),
                vul: RelVul {
                    we: vulnerability.is_vulnerable(seat),
                    they: vulnerability.is_vulnerable(seat.next()),
                },
            }
        }
    }
}

/// Enumerates the candidates of the position after `auction` (see [`Position`]).
pub(crate) fn enumerate_position<'a>(
    table: &'a Table,
    auction: &Auction,
    implicit_pass: ImplicitPass,
) -> Position<'a> {
    let seat = auction.next_seat();
    let system: &'a SystemIR = &table.systems[seat.index() as usize];
    let key = policy_key(auction, seat);
    let lookup = system.index.resolve(&key);
    // Whether the candidates come from the exact resolve or from `resolve_lenient` (the
    // opponents' real call substituted by `Pass`, "system on"). This gates both the
    // `IllegalSystemCall` lint and the implicit-pass synthesis, so that the policy only ever
    // treats a position as fully on-system when the whole prefix resolves; at a lenient position
    // it reads the first full lenient match (the fewest substitutions) and nothing else.
    let exact_match = lookup.matched_depth == key.calls.len();
    let end = if exact_match {
        Some(lookup.end)
    } else {
        system
            .index
            .resolve_lenient(&key, LENIENT_MAX_SUBST)
            .into_iter()
            .find(|(lk, _)| lk.matched_depth == key.calls.len())
            .map(|(lk, _)| lk.end)
    };
    let mut children = Vec::new();
    let mut any_illegal = false;
    let mut pass_offered = false;
    let mut any_legal = false;
    if let Some(end) = end {
        for (call, node) in system.index.children(end, key.opener_pos, key.vul) {
            let legal = auction.is_legal(call);
            pass_offered |= call == Call::Pass;
            any_illegal |= !legal;
            any_legal |= legal;
            children.push((call, node, legal));
        }
    }
    let implicit_pass = implicit_pass == ImplicitPass::Complement
        && exact_match
        && !pass_offered
        && any_legal
        && auction.is_legal(Call::Pass);
    Position {
        seat,
        system,
        opener_pos: key.opener_pos,
        vul: key.vul,
        exact_match,
        matched_depth: lookup.matched_depth,
        end,
        children,
        any_illegal,
        implicit_pass,
        n_legal: auction.legal_calls().count(),
    }
}

/// The legal system members of `pos` in rank order ([`bridge_system::exclusive::rank_cmp`]):
/// the index group's members filtered by legality.
pub(crate) fn ranked_legal(pos: &Position<'_>) -> Vec<(Call, NodeId)> {
    match pos.group() {
        Some(group) => group
            .members
            .iter()
            .copied()
            .filter(|&(call, node)| {
                pos.children
                    .iter()
                    .any(|&(c, n, legal)| legal && c == call && n == node)
            })
            .collect(),
        None => Vec::new(),
    }
}

/// The system policy's choice `s_P(h)` at an on-system position: the first legal member (in rank
/// order) `hand` satisfies, else the implicit `Pass` when the position synthesises one, else
/// `None` (`⊥`).
pub(crate) fn system_choice(pos: &Position<'_>, hand: Hand) -> Option<Call> {
    ranked_legal(pos)
        .into_iter()
        .find(|&(_, node)| pos.system.node(node).constraint.satisfies(hand))
        .map(|(call, _)| call)
        .or_else(|| pos.implicit_pass.then_some(Call::Pass))
}

/// The ranked natural candidates for the next call after `auction` (07-bidding.md §5.2 step
/// 1.4), with `CallContext::partner_constraint`/`forcing_situation` from [`partner_context`], the
/// same function `interpret`'s natural step uses, so the two agree on what a natural call shows
/// (bidirectional consistency, §2.3).
pub(crate) fn natural_ranked(
    table: &Table,
    auction: &Auction,
    natural: &NaturalInference,
    implicit_pass: ImplicitPass,
) -> NaturalRanked {
    let seat = auction.next_seat();
    let system = &table.systems[seat.index() as usize];
    let partner = partner_context(table, natural, auction, implicit_pass);
    let ranked = natural.ranked_candidates(auction, seat, &partner, system.meta.tie_break);
    NaturalRanked { ranked, partner }
}

/// The natural policy's choice `m_P(h)` (docs/design/15-phase4-plan.md D18): the first ranked
/// candidate `hand` satisfies; else the natural implicit `Pass` under
/// `ImplicitPass::Complement` (a hand that fits no natural candidate passes); else `None`.
pub(crate) fn natural_choice(
    ranked: &NaturalRanked,
    hand: Hand,
    implicit_pass: ImplicitPass,
) -> Option<Call> {
    ranked
        .ranked
        .iter()
        .find(|c| c.constraint.satisfies(hand))
        .map(|c| c.call)
        .or_else(|| (implicit_pass == ImplicitPass::Complement).then_some(Call::Pass))
}

/// The output of [`gather`].
pub(crate) struct Gathered {
    /// Surviving candidates, unsorted.
    pub(crate) kept: Vec<Kept>,
    /// Rejected system candidates.
    pub(crate) tried: Vec<Tried>,
    /// System-definition problems noticed on the way.
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// Off-system with a natural engine: the ranked natural candidates `kept` was filtered from.
    pub(crate) natural: Option<NaturalRanked>,
}

/// Steps 1–3 of §5.2: [`enumerate_position`] plus the hand filter (and the natural branch at an
/// off-system position).
///
/// Takes the whole `table`, not just the acting seat's own `SystemIR`: the natural branch needs
/// the reading of the auction so far (partner's system included) to fill
/// `CallContext::partner_constraint`/`forcing_situation` exactly as `interpret` does.
pub(crate) fn gather(
    table: &Table,
    hand: Hand,
    auction: &Auction,
    ctx: &BidContext<'_>,
) -> Gathered {
    let pos = enumerate_position(table, auction, ctx.implicit_pass);
    let system = pos.system;
    let mut diagnostics = Vec::new();
    let mut tried = Vec::new();
    let mut kept: Vec<Kept> = Vec::new();

    for &(call, node_id, legal) in &pos.children {
        if !legal {
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
            if pos.exact_match {
                diagnostics.push(Diagnostic::IllegalSystemCall {
                    node: node_id,
                    call,
                });
            }
            continue;
        }
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

    // A position with no legal continuation in the system (a leaf reached through an opponents'
    // edge that only a deeper row put in the trie, or a lenient match whose children are all
    // illegal here) is off-system for every call we could make: the natural candidates answer
    // here instead of `NoCandidate`. Illegal system children are still reported above.
    let mut natural_out = None;
    if !pos.on_system() {
        if let Some(natural) = ctx.natural {
            let ranked = natural_ranked(table, auction, natural, ctx.implicit_pass);
            for cand in &ranked.ranked {
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
            // The natural implicit pass (07-bidding.md §5.2 step 3): a hand that fits no natural
            // candidate passes, instead of leaving a forced-`Pass` gap.
            if kept.is_empty()
                && ctx.implicit_pass == ImplicitPass::Complement
                && auction.is_legal(Call::Pass)
            {
                kept.push(Kept {
                    call: Call::Pass,
                    node: None,
                    priority: i16::MIN + 1,
                    source: ChoiceSource::ImplicitPass,
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

    // The system implicit pass (07-bidding.md §5.2 step 3): the hand satisfies no legal system
    // candidate (their complement, which `interpret` reads the pass as).
    if pos.implicit_pass && !kept.iter().any(|k| k.source == ChoiceSource::System) {
        kept.push(Kept {
            call: Call::Pass,
            node: None,
            priority: i16::MIN + 1,
            source: ChoiceSource::ImplicitPass,
        });
    }

    Gathered {
        kept,
        tried,
        diagnostics,
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
