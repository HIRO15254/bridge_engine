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
//! 4. Sort by priority descending, ties by `SystemMeta::tie_break`.
//! 5. Empty → `NoCandidate`; otherwise `Chosen` with every survivor in `alternatives`.

use std::cmp::Ordering;
use std::collections::HashMap;

use bridge_core::{Auction, Call, Hand};
use bridge_system::natural::classify;
use bridge_system::{LookupKey, RelVul, TieBreak};

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
    /// For a `Natural` candidate, the `(explanation, rule)` of the `Inference` that produced it,
    /// kept so [`choose_bid`] can build the winner's explanation without re-running `infer` (and
    /// the partner-context computation it needs).
    pub(crate) natural_text: Option<(String, &'static str)>,
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
) -> (Vec<Kept>, Vec<Tried>, Vec<Diagnostic>) {
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
                natural_text: None,
            });
        }
    } else if let Some(natural) = ctx.natural {
        // `NaturalInference::candidates` builds each call's `CallContext` with a bare `classify`,
        // leaving `partner_constraint`/`forcing_situation` at `None`/`false`, whereas
        // `interpret`'s natural step fills both from the prefix's interpretation before `infer`.
        // Rules that read them (`rule_cue`'s `min_hcp`, `rule_pass_forcing`) would then give a
        // different constraint here than `interpret` gives for the same call, breaking the
        // bidirectional consistency of 07-bidding.md §2.3. So this branch builds the same context
        // as `interpret` (classify on the extended auction, partner context from the prefix's
        // Step A, then `infer`), keeping `candidates`' filtering and priority rule.
        let (partner_constraint, forcing_situation) =
            partner_context_for_prefix(table, auction, seat);
        for call in auction.legal_calls() {
            let Ok(next) = auction.with(call) else {
                continue;
            };
            let mut call_ctx = classify(&next, auction.len(), seat);
            call_ctx.partner_constraint = partner_constraint.clone();
            call_ctx.forcing_situation = forcing_situation;
            let inf = natural.infer(&call_ctx);
            if inf.rule == "fallback" {
                continue;
            }
            if call == Call::Pass {
                pass_offered = true;
            }
            if !inf.constraint.satisfies(hand) {
                // Natural candidates are not system nodes, so a rejection is not reported
                // (`Tried` and `IllegalSystemCall`/`UnsatisfiableNode` only ever reference a
                // system `NodeId`; see 07-bidding.md §5.2).
                continue;
            }
            let priority = (inf.confidence * 100.0).round() as i16;
            kept.push(Kept {
                call,
                node: None,
                priority,
                source: ChoiceSource::Natural,
                natural_text: Some((inf.explanation, inf.rule)),
            });
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
                natural_text: None,
            });
        }
    }

    (kept, tried, diagnostics)
}

/// `(call, priority)` for every kept candidate, dropping node identity and diagnostics; used by
/// `policy::call_distribution`, which only needs to score calls.
pub(crate) fn kept_priorities(
    table: &Table,
    hand: Hand,
    auction: &Auction,
    ctx: &BidContext<'_>,
) -> Vec<(Call, i16)> {
    let (kept, _, _) = gather(table, hand, auction, ctx);
    kept.into_iter().map(|k| (k.call, k.priority)).collect()
}

/// Tie-break comparator (ascending: the smaller side wins), per `SystemMeta::tie_break`.
fn tie_break_cmp(system: &SystemIR, tie_break: TieBreak, a: &Kept, b: &Kept) -> Ordering {
    match tie_break {
        TieBreak::RowOrder => {
            let ra = a.node.map(|n| system.node(n).row.0);
            let rb = b.node.map(|n| system.node(n).row.0);
            match (ra, rb) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        }
        TieBreak::Narrowest => {
            let va = a
                .node
                .map(|n| system.node(n).volume_log2)
                .unwrap_or(i16::MAX);
            let vb = b
                .node
                .map(|n| system.node(n).volume_log2)
                .unwrap_or(i16::MAX);
            va.cmp(&vb)
        }
        // `Call` has no `Ord`; its `index()` is already Pass < Double < Redouble < Bid(...)
        // ascending (bridge-core's own convention), so it stands in for call order directly.
        TieBreak::LowestCall => a.call.index().cmp(&b.call.index()),
        TieBreak::HighestCall => b.call.index().cmp(&a.call.index()),
    }
}

/// Chooses a call for `hand` after `auction` under `table` (its acting seat's system, or its
/// natural fallback; see `gather`'s doc comment (in this module) for why the whole `table` is
/// needed rather than just the acting seat's own [`SystemIR`]).
pub fn choose_bid(table: &Table, hand: Hand, auction: &Auction, ctx: &BidContext<'_>) -> BidChoice {
    let seat = auction.next_seat();
    let system = &table.systems[seat.index() as usize];
    let (mut kept, tried, diagnostics) = gather(table, hand, auction, ctx);
    let tie_break = system.meta.tie_break;
    kept.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| tie_break_cmp(system, tie_break, a, b))
    });

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
                let (text, rule) = kept[0]
                    .natural_text
                    .take()
                    .expect("a Natural-sourced kept candidate carries its inference text");
                format!("{text} ({rule})")
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
