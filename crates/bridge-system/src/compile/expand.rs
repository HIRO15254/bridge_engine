//! Depth-first expansion of the parsed AST into concrete [`Node`]s and the [`AuctionTrie`].
//!
//! A port of `bss.py::systemdata_bidtable` (`docs/design/06-system.md` §4.2): each [`BidTable`]
//! is walked once, left to right and top to bottom, threading a [`Frame`] (variable binding,
//! strains already bid, and a running [`Auction`] used for legality and depth bookkeeping) down
//! the tree. A pattern that binds a fresh variable, or that is written as several literal
//! strains, spawns one recursion per candidate; siblings are processed exact rows first (in row
//! order), then pattern rows (in row order), so that "first definition wins" and an exact row
//! shadows a pattern row that would generate the same call.
//!
//! A row whose concrete candidate collides with a call already present in the trie (whether from
//! an earlier sibling, or from another table's history re-tracing an already-expanded prefix —
//! the common case for a file with many tables sharing a prefix) reuses the existing [`Node`]
//! instead of creating a new one, so the tree stays a single shared structure no matter how many
//! tables touch the same position.
//!
//! The synthetic auction fixes the dealer at North (`docs/design/06-system.md` §4.2); since the
//! table's own `#SEAT`/`#VUL` conditions carry the *real* opener position and vulnerability
//! separately, seat `i` of this auction is simply "the `i`-th caller of the table", which lets
//! [`bridge_core::Seat::partner`] identify `own_prev`/`partner_last` (the two individuals of one
//! side alternate seats, e.g. opener/responder) without tracking player identity by hand.

use std::sync::Arc;

use bridge_constraint::{DnfOptions, HandConstraint};
use bridge_core::{Auction, Bid, Call, Seat, Side as TableSide, Strain, Suit, Vulnerability};

use crate::{
    Alertability, CompileOptions, Lint, LintCode, Node, NodeId, Recognition, Row, RowId,
    SystemMeta,
    ast::{BidTable, BmlNode, CallToken, Description, SeatCond, Span, VulCond},
    compile::desc::{compile_description, context::RowContext},
    natural::Role,
    pattern::{Binding, CallPattern, Level, OppClass, Side, SidedPattern, StrainSet, Var},
    trie::Edge,
};

/// Everything the expansion stage produces.
pub(crate) struct Expansion {
    pub rows: Vec<Row>,
    pub nodes: Vec<Node>,
    pub trie: crate::trie::AuctionTrie,
    pub lints: Vec<Lint>,
    /// Whether [`LintCode::TooManyNodes`] has already been reported once for this file: the
    /// `max_nodes` guard is hit repeatedly across many rows/candidates once the limit is
    /// reached, but should be reported only once.
    too_many_nodes_reported: bool,
    /// One physical BML row (keyed by its call token's source [`Span`], unique per occurrence
    /// even across `#INCLUDE`/`#PASTE`, see `expand_row`'s doc comment) can be reached by more
    /// than one candidate/history-retrace branch of the same table (e.g. a `Var` history token
    /// with several candidates re-enters every one of its children once per candidate). Those
    /// branches must accumulate into *one* [`Row`] (one entry in `rows`, with every branch's
    /// [`NodeId`]s in `Row::expansions`), not a fresh `Row` per branch -- otherwise `rows.len()`
    /// no longer counts physical BML rows, and per-row roll-ups (`LintCode::LowRecognition`, the
    /// recognition report's average) silently double- or quadruple-count the same source line.
    row_by_span: std::collections::HashMap<Span, RowId>,
}

/// Expands every [`BidTable`] block of a file, in order, into a shared [`Expansion`].
pub(crate) fn expand_file(
    tables: &[&BidTable],
    meta: &SystemMeta,
    opts: &CompileOptions,
) -> Expansion {
    let mut ex = Expansion {
        rows: Vec::new(),
        nodes: Vec::new(),
        trie: crate::trie::AuctionTrie::new(),
        lints: Vec::new(),
        too_many_nodes_reported: false,
        row_by_span: std::collections::HashMap::new(),
    };
    for table in tables {
        if ex.nodes.len() >= opts.max_nodes {
            // The limit can be reached exactly on the previous table's last candidate, in
            // which case `expand_row` never saw a candidate it had to refuse: report the
            // tables dropped here the same way.
            report_too_many_nodes(opts, &mut ex);
            break;
        }
        expand_table(table, meta, opts, &mut ex);
    }
    demote_illegal_call_for_bindings_that_succeeded(&mut ex);
    ex
}

/// An exact row nested under a variable-bound ancestor (a history token, or a `Var`/`Strains`
/// pattern higher in the tree) is expanded once per binding the ancestor can take, and the row's
/// own call can be illegal for some of those bindings while sufficient for others (`docs/design/
/// 06-system.md` §4.2 point 3's example: under `1C-(1X)-`, the child row `1H` is only illegal
/// when `X` is bound to `H` or `S`; for `X = C`/`D` the same row is a perfectly legal, intended
/// bid). `expand_row` cannot tell the two cases apart at the point it discovers one binding is
/// illegal -- it does not yet know whether a sibling binding of the same ancestor will later
/// succeed for this row -- so it always records the finding, and this pass demotes it after the
/// fact once every binding has been tried: a row that has at least one successful expansion
/// anywhere in the file had its `IllegalCall` correctly authored (the author meant the *reachable*
/// bindings), so the illegal ones are downgraded to `Info` the way `bss.py` silently drops them,
/// keeping `Error` only for a row that never expands under any binding.
fn demote_illegal_call_for_bindings_that_succeeded(ex: &mut Expansion) {
    let rows_with_a_success: std::collections::HashSet<RowId> = ex
        .rows
        .iter()
        .filter(|row| !row.expansions.is_empty())
        .map(|row| row.id)
        .collect();
    for lint in &mut ex.lints {
        if lint.code != LintCode::IllegalCall {
            continue;
        }
        let Some(row_id) = lint.row else { continue };
        if rows_with_a_success.contains(&row_id) {
            lint.severity = crate::Severity::Info;
            lint.message = format!(
                "{} (legal for at least one other binding of this row; dropped)",
                lint.message
            );
        }
    }
}

/// The state threaded down one path of the expansion tree.
#[derive(Clone)]
struct Frame {
    /// The authored pattern path (history plus rows), one entry per real call (no implicit
    /// passes): shared into every [`Row`]/[`Node`] created along this path.
    path: Vec<SidedPattern>,
    /// `path`'s 1:1 concrete-call counterpart (a `Call::Pass` filler at a wildcard step).
    resolved: Vec<Call>,
    /// The concrete auction, dealer fixed at North, implicit passes included -- but only up to
    /// the first wildcard step: a [`CallPattern::Class`] step has no concrete call, so nothing
    /// is pushed for it (`docs/design/06-system.md` §4.2 point 3, "`auction` を変えずに") and the
    /// auction is frozen from there on. Below a wildcard, legality and the last bid are read
    /// from `edges` instead ([`Frame::is_legal`], [`Frame::last_bid`]).
    auction: Auction,
    /// The trie path: one entry per call of the synthetic auction (implicit passes included), a
    /// concrete [`Edge::Call`] everywhere except a wildcard step, which is [`Edge::Class`].
    /// `edges.len()` is always the trie depth, and entry `i` was made by synthetic seat
    /// `North + i`.
    edges: Vec<Edge>,
    /// Variable bindings live for this path only.
    env: Binding,
    /// Strains bid so far by either side.
    used: StrainSet,
    /// The node made by each of the table's four synthetic seats so far (dealer = North =
    /// whoever calls first in the table), indexed by [`Seat::index`].
    last_by_seat: [Option<NodeId>; 4],
    /// `true` once an ancestor step was a [`CallPattern::Class`] wildcard: `Step` and a *fresh*
    /// variable are forbidden below one (`docs/design/06-system.md` §4.2 point 3), since neither
    /// has a real anchor once the opponents' actual call is unknown.
    under_wildcard: bool,
}

impl Frame {
    fn root() -> Frame {
        Frame {
            path: Vec::new(),
            resolved: Vec::new(),
            auction: Auction::new(Seat::North, Vulnerability::None),
            edges: Vec::new(),
            env: Binding::default(),
            used: StrainSet::EMPTY,
            last_by_seat: [None; 4],
            under_wildcard: false,
        }
    }

    /// The synthetic seat that will make the next call.
    fn next_seat(&self) -> Seat {
        Seat::North.offset((self.edges.len() % 4) as u8)
    }

    /// The last *known* bid of the path (a wildcard step's unknown call is not one).
    fn last_bid(&self) -> Option<Bid> {
        if !self.under_wildcard {
            return self.auction.last_bid().map(|(_, b)| b);
        }
        self.edges.iter().rev().find_map(|e| match e {
            Edge::Call(Call::Bid(b)) => Some(*b),
            _ => None,
        })
    }

    /// Whether `call` can be the next call. Exact on the concrete auction; below a wildcard,
    /// "legal for at least one call the wildcard(s) could stand for" ([`relaxed_is_legal`]).
    fn is_legal(&self, call: Call) -> bool {
        if self.under_wildcard {
            relaxed_is_legal(&self.edges, call)
        } else {
            self.auction.is_legal(call)
        }
    }

    /// Whether some call of `class` can be the next call.
    fn class_is_possible(&self, class: OppClass) -> bool {
        (0..=37u8)
            .filter_map(Call::from_index)
            .any(|call| class.matches(call) && self.is_legal(call))
    }

    /// Appends a concrete call, which the caller has checked with [`Frame::is_legal`].
    fn push_call(&mut self, call: Call) {
        if !self.under_wildcard {
            self.auction.push(call).expect("checked is_legal");
        }
        self.edges.push(Edge::Call(call));
        if let Call::Bid(b) = call {
            self.used = self.used.with(b.strain());
        }
    }

    /// Appends a wildcard step (checked with [`Frame::class_is_possible`]); the auction is
    /// frozen from here on.
    fn push_class(&mut self, class: OppClass) {
        self.edges.push(Edge::Class(class));
        self.under_wildcard = true;
    }

    /// The concrete calls of the path, with a `Pass` filler at every wildcard step (what
    /// [`Node::calls`] stores).
    fn calls(&self) -> Vec<Call> {
        self.edges
            .iter()
            .map(|e| match e {
                Edge::Call(call) => *call,
                Edge::Class(_) => Call::Pass,
            })
            .collect()
    }
}

/// The synthetic seat that made `edges[i]`.
fn seat_of_step(i: usize) -> Seat {
    Seat::North.offset((i % 4) as u8)
}

/// Whether a step is certainly a `Pass`.
fn is_certainly_pass(edge: &Edge) -> bool {
    matches!(edge, Edge::Call(Call::Pass) | Edge::Class(OppClass::Pass))
}

/// Whether a step is certainly *not* a `Pass`.
fn is_certainly_not_pass(edge: &Edge) -> bool {
    match edge {
        Edge::Call(call) => *call != Call::Pass,
        Edge::Class(class) => !class.matches(Call::Pass),
    }
}

/// Whether a wildcard class admits some bid.
fn class_admits_a_bid(class: OppClass) -> bool {
    !matches!(class, OppClass::Double | OppClass::Pass)
}

/// Legality of `call` after `edges`, some of which are wildcard steps standing for an unknown
/// call of their class (`docs/design/06-system.md` §4.2 point 3). The rule is "legal for at
/// least one assignment the wildcards could take", checked conservatively per call kind:
///
/// - the auction only counts as complete when it certainly is (three certain passes after a
///   certain non-pass, or four certain passes);
/// - a bid must be higher than the last *known* bid and, after an `AnyBidAtLevel(l)` wildcard,
///   at least at level `l`;
/// - a Double needs the last possibly-non-pass step to be an opponent's bid, or an opponent's
///   wildcard that admits a bid; a Redouble likewise an opponent's Double, or an opponent's
///   `(any)`/Double-class wildcard.
///
/// Without any wildcard this agrees exactly with [`Auction::is_legal`].
fn relaxed_is_legal(edges: &[Edge], call: Call) -> bool {
    let n = edges.len();
    let complete = n >= 4
        && edges[n - 3..].iter().all(is_certainly_pass)
        && (is_certainly_not_pass(&edges[n - 4]) || (n == 4 && is_certainly_pass(&edges[0])));
    if complete {
        return false;
    }
    let me = seat_of_step(n);
    let last_live = edges
        .iter()
        .enumerate()
        .rev()
        .find(|(_, e)| !is_certainly_pass(e));
    match call {
        Call::Pass => true,
        Call::Bid(b) => {
            let mut last_known: Option<Bid> = None;
            let mut min_level = 1u8;
            for edge in edges {
                match edge {
                    Edge::Call(Call::Bid(known)) => {
                        last_known = Some(*known);
                        min_level = 1;
                    }
                    Edge::Class(OppClass::AnyBidAtLevel(level)) => {
                        min_level = min_level.max(*level);
                    }
                    _ => {}
                }
            }
            last_known.is_none_or(|last| b > last) && b.level() >= min_level
        }
        Call::Double => last_live.is_some_and(|(i, e)| {
            seat_of_step(i).side() != me.side()
                && match e {
                    Edge::Call(c) => c.bid().is_some(),
                    Edge::Class(class) => class_admits_a_bid(*class),
                }
        }),
        Call::Redouble => last_live.is_some_and(|(i, e)| {
            seat_of_step(i).side() != me.side()
                && match e {
                    Edge::Call(c) => *c == Call::Double,
                    Edge::Class(class) => class.matches(Call::Double),
                }
        }),
    }
}

/// One candidate expansion of a pattern: the trie edge to take and, for a fresh variable, the
/// binding to carry into the subtree.
#[derive(Clone, Copy)]
struct Candidate {
    edge: Edge,
    binding: Binding,
}

/// Generates the candidate edges for `pattern` (`docs/design/06-system.md` §4.2 point 3), pure
/// and independent of any particular auction position beyond `last_bid` (used by
/// [`CallPattern::Step`]).
///
/// Sufficiency is *not* checked here (that is `Auction::is_legal`'s job on the caller's concrete
/// auction): a `Level::At` candidate is emitted even when it happens to be insufficient, so the
/// caller can report `IllegalCall`. A `Level::Any` wildcard, by contrast, is explicitly "whatever
/// level is needed" (`docs/design/06-system.md` §1.3), so only the minimum sufficient level per
/// strain is generated; this cannot itself be illegal.
fn generate_candidates(
    pattern: &CallPattern,
    env: &Binding,
    used: StrainSet,
    last_bid: Option<Bid>,
    under_wildcard: bool,
) -> Vec<Candidate> {
    match pattern {
        CallPattern::Exact(call) => vec![Candidate {
            edge: Edge::Call(*call),
            binding: *env,
        }],
        CallPattern::Strains { level, strains } => match level {
            Level::At(n) => strains
                .iter()
                .filter_map(|s| Bid::new(*n, s))
                .map(|b| Candidate {
                    edge: Edge::Call(Call::Bid(b)),
                    binding: *env,
                })
                .collect(),
            Level::Any => strains
                .iter()
                .flat_map(|s| bids_at_level(Level::Any, s, last_bid))
                .map(|b| Candidate {
                    edge: Edge::Call(Call::Bid(b)),
                    binding: *env,
                })
                .collect(),
        },
        CallPattern::Var { level, var } => {
            if let Some(strain) = env.get(*var) {
                // Already bound (including `oM`/`om`, which only ever read an existing binding).
                bids_at_level(*level, strain, last_bid)
                    .into_iter()
                    .map(|b| Candidate {
                        edge: Edge::Call(Call::Bid(b)),
                        binding: *env,
                    })
                    .collect()
            } else if matches!(var, Var::OtherMajor | Var::OtherMinor) || under_wildcard {
                // Either `oM`/`om` unbound (UnboundOther; the caller reports it) or a fresh
                // binding with no anchor below a wildcard step.
                Vec::new()
            } else {
                // A fresh variable only offers *sufficient* bids as candidates (bss.py's
                // `check_vars` silently drops `bid <= last_bid`); an insufficient one is simply
                // not a real choice here, not an authored call to flag as `IllegalCall`.
                env.candidates(*var, used)
                    .into_iter()
                    .flat_map(|strain| {
                        bids_at_level(*level, strain, last_bid)
                            .into_iter()
                            .filter(|b| last_bid.is_none_or(|last| *b > last))
                            .map(move |b| Candidate {
                                edge: Edge::Call(Call::Bid(b)),
                                binding: env.bind(*var, strain),
                            })
                    })
                    .collect()
            }
        }
        CallPattern::Step(n) => {
            if under_wildcard {
                return Vec::new();
            }
            match last_bid.and_then(|b| Bid::from_index(b.index().wrapping_add(*n))) {
                Some(bid) => vec![Candidate {
                    edge: Edge::Call(Call::Bid(bid)),
                    binding: *env,
                }],
                None => Vec::new(), // StepWithoutAnchor (no anchor, or past 7NT); caller reports.
            }
        }
        CallPattern::AnyOf(alts) => alts
            .iter()
            .flat_map(|p| generate_candidates(p, env, used, last_bid, under_wildcard))
            .collect(),
        CallPattern::Class(k) => vec![Candidate {
            edge: Edge::Class(*k),
            binding: *env,
        }],
    }
}

/// The bid(s) at `level` in `strain`: one for `Level::At`, or every sufficient level `1..=7` for
/// `Level::Any` (`docs/design/06-system.md` §4.2: `n` means "whatever level is needed", which is
/// every level above the last bid, not only the lowest one -- real files write `(nX)-3N` meaning
/// "over an opening at any level").
fn bids_at_level(level: Level, strain: Strain, last_bid: Option<Bid>) -> Vec<Bid> {
    match level {
        Level::At(n) => Bid::new(n, strain).into_iter().collect(),
        Level::Any => (1..=7)
            .filter_map(|n| Bid::new(n, strain))
            .filter(|b| last_bid.is_none_or(|last| *b > last))
            .collect(),
    }
}

/// The lowest-level bid in `strain` that is higher than `last_bid` (level 1 if there is none).
fn minimum_sufficient_bid(strain: Strain, last_bid: Option<Bid>) -> Option<Bid> {
    match last_bid {
        None => Bid::new(1, strain),
        Some(last) => (last.level()..=7).find_map(|level| {
            let b = Bid::new(level, strain)?;
            (b > last).then_some(b)
        }),
    }
}

/// Whether a call by `next` needs an opponents'-side implicit pass inserted first, because the
/// previous real call (`prev`, `None` at the very start of the table) was made by the same side.
fn needs_implicit_pass(prev: Option<Side>, next: Side) -> bool {
    prev == Some(next)
}

/// A call skipped at least one level below the minimum sufficient bid *in its own strain*: the
/// lowest legal level for `b`'s strain given `last_bid`, not simply the next call in bidding
/// order (`docs/design/06-system.md` §4.2's `is_jump`; a change of strain to a lower or equal
/// level, such as `1H-2C` or `1S-2H`, is not itself a jump).
fn is_jump(call: Call, last_bid: Option<Bid>) -> bool {
    match call {
        Call::Bid(b) => {
            minimum_sufficient_bid(b.strain(), last_bid).is_some_and(|min| b.level() > min.level())
        }
        _ => false,
    }
}

/// Estimate of `log2` of the constraint's volume (`docs/design/06-system.md` §5.3); `i16::MIN`
/// for an unsatisfiable (empty) constraint.
fn estimate_volume_log2(constraint: &HandConstraint) -> i16 {
    let Ok(dnf) = constraint.to_dnf(&DnfOptions::default()) else {
        return i16::MIN;
    };
    let total: f64 = dnf
        .terms
        .iter()
        .map(|t| {
            let hcp = t.atom.hcp_range();
            let hcp_span = (*hcp.end() as f64 - *hcp.start() as f64 + 1.0).max(0.0);
            hcp_span * (t.atom.shapes.len() as f64).max(1.0)
        })
        .sum();
    if total <= 0.0 {
        i16::MIN
    } else {
        (total.log2().round() as i64).clamp(i16::MIN as i64, i16::MAX as i64) as i16
    }
}

/// Substitutes bound variables in a description: `M`, `oM`, `m`, `om`, `X`/`Y`/`Z` become their
/// suit sentinel (`!h` etc.); an unresolved reference is left as-is. Matching is on word
/// boundaries only (`docs/design/06-system.md` risk R4).
///
/// `#` is deliberately NOT substituted here, unlike `RowContext.hash_suit`'s own resolution for
/// the *constraint* compiler (`SuitLen(Hash, …)`, `desc/context.rs`): the reference `bss.py` only
/// ever rewrites `M`/`m`/`oM`/`om`/`X`/`Y`/`Z` in description *text* (confirmed by reading
/// `src/bml/bss.py`'s substitution loop, which iterates exactly those seven variable names and
/// nothing else) and leaves a literal `#` untouched wherever it appears in prose -- e.g.
/// `bml-test`'s `example8.bml`, whose description deliberately contains the literal LaTeX-special
/// characters `& % $ # _ { } ~ ^ \` verbatim.
fn substitute_description(text: &str, env: &Binding) -> String {
    const VARS: [(&str, Var); 7] = [
        ("oM", Var::OtherMajor),
        ("om", Var::OtherMinor),
        ("M", Var::Major),
        ("m", Var::Minor),
        ("X", Var::X),
        ("Y", Var::Y),
        ("Z", Var::Z),
    ];
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    'outer: while i < text.len() {
        let rest = &text[i..];
        for (word, var) in VARS {
            if let Some(after) = rest.strip_prefix(word) {
                // A digit before the variable is allowed (`5M`, `3oM`): bss.py matches
                // `([0-9]+)VAR\b`, treating a leading run of digits as part of the same token
                // rather than a word character that would block the match.
                let before_ok =
                    i == 0 || !is_word_byte(bytes[i - 1]) || bytes[i - 1].is_ascii_digit();
                let after_ok = after.as_bytes().first().is_none_or(|&b| !is_word_byte(b));
                if before_ok && after_ok {
                    if let Some(strain) = env.get(var) {
                        match strain.suit() {
                            Some(suit) => out.push_str(suit_sentinel(suit)),
                            None => out.push_str("NT"),
                        }
                        i += word.len();
                        continue 'outer;
                    }
                }
            }
        }
        let ch = rest.chars().next().expect("i < text.len()");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn suit_sentinel(suit: Suit) -> &'static str {
    match suit {
        Suit::Clubs => "!c",
        Suit::Diamonds => "!d",
        Suit::Hearts => "!h",
        Suit::Spades => "!s",
    }
}

/// The suit `#` refers to (`docs/design/06-system.md` §1.3): walking `path` from this row's own
/// call backwards (own calls and the opponents' alike), the first position whose *authored*
/// pattern was not `Exact`/`Step`/`Class` — i.e. a `Var`, `Strains` or `AnyOf` — resolved to some
/// concrete suit. `resolved` is `path`'s 1:1 concrete-call counterpart.
fn hash_suit(path: &[SidedPattern], resolved: &[Call]) -> Option<Suit> {
    for (pat, call) in path.iter().zip(resolved.iter()).rev() {
        let is_variable_position = !matches!(
            pat.pat,
            CallPattern::Exact(_) | CallPattern::Step(_) | CallPattern::Class(_)
        );
        if is_variable_position {
            if let Call::Bid(b) = call {
                return b.strain().suit();
            }
        }
    }
    None
}

/// Whether `seat_now` is (or is about to become) this side's *founding* player for role
/// purposes -- the one whose calls are Opener/Overcaller/Balancer, as opposed to
/// Responder/Advancer (`docs/design/06-system.md` §7.2). A player who has already made a real
/// (non-`Pass`) call keeps that role on every later call of theirs (a rebid); otherwise, once
/// the *other* player of the side has made a real call, this one is the responder/advancer. A
/// side's own `Pass` does not by itself establish a role, so it is not "a real call" here: it
/// lets a later, genuine first bid by either player still count as founding (the balancing-seat
/// case, `1H-P-P-?`).
fn is_founding_call(self_bid_before: bool, mate_bid_before: bool) -> bool {
    self_bid_before || !mate_bid_before
}

/// Determines the auction role from the path shape (`docs/design/06-system.md` §7.2): the
/// simplest faithful reading available without `NaturalInference::classify` (owned by another
/// lane): the first call on a side is its opener/overcaller (depending on who opened first), a
/// later call on the same side is the responder/advancer, and a first call made after both sides
/// have already bid is the balancer.
fn infer_role(
    side: Side,
    we_opened: bool,
    is_first_of_side: bool,
    competitive_before: bool,
) -> Role {
    match (
        side == Side::Us,
        we_opened,
        is_first_of_side,
        competitive_before,
    ) {
        (true, true, true, _) => Role::Opener,
        (true, true, false, _) => Role::Responder,
        (true, false, true, false) => Role::Overcaller,
        (true, false, true, true) => Role::Balancer,
        (true, false, false, _) => Role::Advancer,
        (false, true, true, false) => Role::Overcaller,
        (false, true, true, true) => Role::Balancer,
        (false, true, false, _) => Role::Advancer,
        (false, false, true, _) => Role::Opener,
        (false, false, false, _) => Role::Responder,
    }
}

/// Expands one `BidTable`, appending to `ex`.
fn expand_table(table: &BidTable, meta: &SystemMeta, opts: &CompileOptions, ex: &mut Expansion) {
    let first_side = table.history.first().map(|t| t.side).or_else(|| {
        table
            .rows
            .first()
            .and_then(|r| r.calls.first())
            .map(|t| t.side)
    });
    let Some(first_side) = first_side else {
        return; // an empty table (parse recovery already dropped everything): nothing to expand.
    };
    let we_opened = first_side == Side::Us;
    let root = Frame::root();

    expand_history(
        &table.history,
        &table.history_desc,
        we_opened,
        table.seat,
        table.vul,
        meta,
        opts,
        &root,
        None,
        ex,
        &mut |frame, parent, ex| {
            expand_children(
                &table.rows,
                we_opened,
                table.seat,
                table.vul,
                meta,
                opts,
                frame,
                parent,
                ex,
            );
        },
    );
}

/// Expands the history row left to right; `on_done` runs once per resulting branch (normally one,
/// but a bound variable in the history can itself have several candidates, e.g. `1M-` covering
/// both majors, in which case the rest of the table is expanded once per branch), with `parent`
/// set to the node for the last history token on that branch (`None` for an empty history), so
/// the table's own rows can be linked in as its children.
#[allow(clippy::too_many_arguments)]
fn expand_history(
    tokens: &[CallToken],
    history_desc: &Option<Description>,
    we_opened: bool,
    seat: SeatCond,
    vul: VulCond,
    meta: &SystemMeta,
    opts: &CompileOptions,
    frame: &Frame,
    parent: Option<NodeId>,
    ex: &mut Expansion,
    on_done: &mut dyn FnMut(&Frame, Option<NodeId>, &mut Expansion),
) {
    let Some((tok, rest)) = tokens.split_first() else {
        on_done(frame, parent, ex);
        return;
    };
    let is_last = rest.is_empty();
    let synthetic = BmlNode {
        calls: vec![tok.clone()],
        description: if is_last {
            history_desc.clone().unwrap_or_default()
        } else {
            Description::default()
        },
        children: Vec::new(),
        indent: 0,
        span: tok.span.clone(),
    };
    let mut claimed = Claimed::default();
    let outcomes = expand_row(
        &synthetic,
        true,
        &mut claimed,
        we_opened,
        seat,
        vul,
        meta,
        opts,
        frame,
        ex,
    );
    for (node_id, next) in outcomes {
        if let Some(p) = parent {
            if !ex.nodes[p.0 as usize].children.contains(&node_id) {
                ex.nodes[p.0 as usize].children.push(node_id);
            }
        }
        expand_history(
            rest,
            history_desc,
            we_opened,
            seat,
            vul,
            meta,
            opts,
            &next,
            Some(node_id),
            ex,
            on_done,
        );
    }
}

/// Expands one sibling list: exact rows first (row order), then pattern rows (row order), each
/// recursing into its own children and, when `parent` is given, linking each row's node as one of
/// `parent`'s [`Node::children`].
///
/// `bids_processed` (bss.py's own name for this) tracks every trie edge already produced by an
/// *earlier row of this same list*, exact or pattern alike: a later row whose candidate
/// collides with one is a genuine redefinition of the same position (`first definition wins`,
/// already reported by [`build_or_reuse_node`]/[`handle_duplicate`]), so its subtree is not
/// merged into the earlier row's node -- unlike a history token's collision with a *different*
/// table's prefix, which is exactly what re-tracing means to merge. Keyed by [`Edge`], not by
/// the concrete call, so a wildcard step never collides with a real `Pass` (or with another
/// wildcard class).
///
/// A *pattern* row never even builds a node for a candidate an earlier sibling already
/// produced ([`Claimed`]): an exact row shadows it with [`LintCode::ShadowedByExact`], and an
/// earlier pattern row (the `1M …` then catch-all `1X …` idiom) silently, as bss.py's
/// `bid not in bids_processed` does.
#[allow(clippy::too_many_arguments)]
fn expand_children(
    rows: &[BmlNode],
    we_opened: bool,
    seat: SeatCond,
    vul: VulCond,
    meta: &SystemMeta,
    opts: &CompileOptions,
    frame: &Frame,
    parent: Option<NodeId>,
    ex: &mut Expansion,
) {
    let mut claimed = Claimed::default();
    let mut bids_processed: std::collections::HashSet<Edge> = std::collections::HashSet::new();

    for row in rows.iter().filter(|r| is_exact_row(r)) {
        for (node_id, next) in expand_row(
            row,
            true,
            &mut claimed,
            we_opened,
            seat,
            vul,
            meta,
            opts,
            frame,
            ex,
        ) {
            let edge = *next
                .edges
                .last()
                .expect("expand_row pushed this row's edge");
            if !bids_processed.insert(edge) {
                continue; // this sibling list already produced `call`; skip the subtree.
            }
            if let Some(p) = parent {
                if !ex.nodes[p.0 as usize].children.contains(&node_id) {
                    ex.nodes[p.0 as usize].children.push(node_id);
                }
            }
            expand_children(
                &row.children,
                we_opened,
                seat,
                vul,
                meta,
                opts,
                &next,
                Some(node_id),
                ex,
            );
        }
    }
    for row in rows.iter().filter(|r| !is_exact_row(r)) {
        for (node_id, next) in expand_row(
            row,
            false,
            &mut claimed,
            we_opened,
            seat,
            vul,
            meta,
            opts,
            frame,
            ex,
        ) {
            let edge = *next
                .edges
                .last()
                .expect("expand_row pushed this row's edge");
            if !bids_processed.insert(edge) {
                continue; // this sibling list already produced `call`; skip the subtree.
            }
            if let Some(p) = parent {
                if !ex.nodes[p.0 as usize].children.contains(&node_id) {
                    ex.nodes[p.0 as usize].children.push(node_id);
                }
            }
            expand_children(
                &row.children,
                we_opened,
                seat,
                vul,
                meta,
                opts,
                &next,
                Some(node_id),
                ex,
            );
        }
    }
}

/// The trie edges a sibling list has already produced, split by the kind of row that produced
/// them (see [`expand_children`]).
#[derive(Default)]
struct Claimed {
    /// By exact rows (processed first).
    exact: Vec<Edge>,
    /// By earlier pattern rows.
    pattern: Vec<Edge>,
}

fn is_exact_row(row: &BmlNode) -> bool {
    matches!(row.calls[0].pattern, CallPattern::Exact(_))
}

/// Expands one row's call token into its concrete candidates, creating (or reusing) a [`Node`]
/// per candidate.
///
/// One physical row (`tok.span`) can be reached by more than one caller within the same table:
/// an ancestor `Var`/`Strains`/`AnyOf` pattern with several candidates re-enters every child once
/// per candidate, and a multi-candidate history token re-enters the whole rest of the table once
/// per candidate the same way. Every such branch reaching this row shares *one* [`Row`] (looked
/// up by `tok.span` in `ex.row_by_span`, see [`Expansion::row_by_span`]'s doc comment), so a
/// row's `expansions` accumulate across every branch instead of the row being recreated per
/// branch.
#[allow(clippy::too_many_arguments)]
fn expand_row(
    row: &BmlNode,
    is_exact_row: bool,
    claimed: &mut Claimed,
    we_opened: bool,
    seat: SeatCond,
    vul: VulCond,
    meta: &SystemMeta,
    opts: &CompileOptions,
    frame: &Frame,
    ex: &mut Expansion,
) -> Vec<(NodeId, Frame)> {
    let tok = &row.calls[0];
    let side = tok.side;

    let mut row_path = frame.path.clone();
    row_path.push(SidedPattern {
        side,
        pat: tok.pattern.clone(),
    });
    let row_path: Arc<[SidedPattern]> = row_path.into();

    let row_id = match ex.row_by_span.get(&tok.span) {
        Some(&id) => id,
        None => {
            let id = RowId(ex.rows.len() as u32);
            ex.rows.push(Row {
                id,
                span: tok.span.clone(),
                path: Arc::clone(&row_path),
                description_raw: row.description.text.clone(),
                recognition: Recognition::default(),
                expansions: Vec::new(),
            });
            ex.row_by_span.insert(tok.span.clone(), id);
            id
        }
    };

    let last_bid = frame.last_bid();
    let candidates = generate_candidates(
        &tok.pattern,
        &frame.env,
        frame.used,
        last_bid,
        frame.under_wildcard,
    );

    if candidates.is_empty() {
        report_empty_candidates(&tok.pattern, frame, last_bid, tok, ex);
        return Vec::new();
    }

    // `Level::Any` ("`n`") widened to every sufficient level can produce many candidates; flag
    // it rather than silently branching into a wide fan-out (`docs/design/06-system.md` §9.2).
    const WIDE_WILDCARD_THRESHOLD: usize = 8;
    if candidates.len() > WIDE_WILDCARD_THRESHOLD
        && matches!(
            tok.pattern,
            CallPattern::Var {
                level: Level::Any,
                ..
            } | CallPattern::Strains {
                level: Level::Any,
                ..
            }
        )
    {
        ex.lints.push(
            Lint::info(
                LintCode::WideWildcard,
                format!(
                    "{}: {} candidate calls for level `n`",
                    tok.raw,
                    candidates.len()
                ),
            )
            .with_span(tok.span.clone())
            .with_row(row_id),
        );
    }

    let mut out = Vec::new();
    for cand in candidates {
        if ex.nodes.len() >= opts.max_nodes {
            report_too_many_nodes(opts, ex);
            break;
        }

        let mut next = frame.clone();

        if needs_implicit_pass(next.path.last().map(|p| p.side), side) {
            if !next.is_legal(Call::Pass) {
                continue; // the auction is already complete: nothing legal follows.
            }
            next.push_call(Call::Pass);
        }

        let seat_now = next.next_seat();
        let concrete_call = match cand.edge {
            Edge::Call(call) => {
                if !next.is_legal(call) {
                    ex.lints.push(
                        Lint::error(
                            LintCode::IllegalCall,
                            format!("{}: {call:?} is not a legal call here", tok.raw),
                        )
                        .with_span(tok.span.clone())
                        .with_row(row_id),
                    );
                    continue;
                }
                next.push_call(call);
                call
            }
            Edge::Class(class) => {
                // No concrete call: `auction`/`used` stay put (§4.2 point 3) and only the trie
                // path records the wildcard. `resolved`/`Node::calls` carry a `Pass` filler.
                if !next.class_is_possible(class) {
                    continue;
                }
                next.push_class(class);
                Call::Pass
            }
        };

        if is_exact_row {
            claimed.exact.push(cand.edge);
        } else if claimed.pattern.contains(&cand.edge) {
            // An earlier *pattern* sibling already produced this call (`1M …` then a catch-all
            // `1X …`): skipped silently, like bss.py's `bid not in bids_processed`, before any
            // node is built, so no spurious DuplicatePath is reported.
            continue;
        } else if claimed.exact.contains(&cand.edge) {
            ex.lints.push(
                Lint::info(
                    LintCode::ShadowedByExact,
                    format!(
                        "{}: shadowed by an earlier exact row for the same call",
                        tok.raw
                    ),
                )
                .with_span(tok.span.clone())
                .with_row(row_id),
            );
            continue;
        }

        next.env = cand.binding;
        next.path.push(SidedPattern {
            side,
            pat: tok.pattern.clone(),
        });
        next.resolved.push(concrete_call);

        let mate = seat_now.partner();
        let bid_before = |s: Seat| {
            frame.last_by_seat[s.index() as usize]
                .is_some_and(|id| ex.nodes[id.0 as usize].call != Call::Pass)
        };
        let is_first_of_side = is_founding_call(bid_before(seat_now), bid_before(mate));
        let competitive_before = frame.last_by_seat.iter().flatten().count() >= 1
            && frame.path.iter().any(|p| p.side == Side::Us)
            && frame.path.iter().any(|p| p.side == Side::Them);
        let role = infer_role(side, we_opened, is_first_of_side, competitive_before);

        let Some(node_id) = build_or_reuse_node(
            row,
            side,
            row_id,
            Arc::clone(&row_path),
            concrete_call,
            seat_now,
            last_bid,
            role,
            seat,
            vul,
            meta,
            &next,
            ex,
        ) else {
            continue;
        };

        next.last_by_seat[seat_now.index() as usize] = Some(node_id);
        ex.rows[row_id.0 as usize].expansions.push(node_id);
        out.push((node_id, next));
    }
    if !is_exact_row {
        claimed.pattern.extend(
            out.iter()
                .map(|(_, next)| *next.edges.last().expect("pushed")),
        );
    }
    out
}

/// Reports [`LintCode::TooManyNodes`] once per file.
fn report_too_many_nodes(opts: &CompileOptions, ex: &mut Expansion) {
    if !ex.too_many_nodes_reported {
        ex.lints.push(Lint::error(
            LintCode::TooManyNodes,
            format!("expansion aborted: reached max_nodes = {}", opts.max_nodes),
        ));
        ex.too_many_nodes_reported = true;
    }
}

fn report_empty_candidates(
    pattern: &CallPattern,
    frame: &Frame,
    last_bid: Option<Bid>,
    tok: &CallToken,
    ex: &mut Expansion,
) {
    match pattern {
        CallPattern::Var { var, .. }
            if matches!(var, Var::OtherMajor | Var::OtherMinor)
                && frame.env.get(*var).is_none() =>
        {
            ex.lints.push(
                Lint::error(
                    LintCode::UnboundOther,
                    format!("{}: the paired major/minor is not yet bound", tok.raw),
                )
                .with_span(tok.span.clone()),
            );
        }
        CallPattern::Step(_) if last_bid.is_none() || frame.under_wildcard => {
            ex.lints.push(
                Lint::error(
                    LintCode::StepWithoutAnchor,
                    format!("{}: no prior bid to step from", tok.raw),
                )
                .with_span(tok.span.clone()),
            );
        }
        CallPattern::Var { .. } => {
            ex.lints.push(
                Lint::info(
                    LintCode::VariableNoCandidate,
                    format!("{}: no candidate strain is available", tok.raw),
                )
                .with_span(tok.span.clone()),
            );
        }
        _ => {}
    }
}

/// The last bid made by a seat of the partnership other than `seat_now`'s, scanning `auction`
/// backwards. Unlike "the last bid by anyone" (which can be our own or our partner's bid, e.g.
/// `1N-(P)-2C` has no bid from *them* yet even though `1N` is the auction's last bid), this is
/// always genuinely the opponents' bid, for a row on either [`Side`].
///
/// Read from the trie path (`edges[i]` made by synthetic seat `North + i`): an opponents'
/// wildcard step that could be a bid makes their last bid unknown (`None`).
fn opponents_last_bid(edges: &[Edge], seat_now: Seat) -> Option<Bid> {
    let their_side = seat_now.side().other();
    for (i, edge) in edges.iter().enumerate().rev() {
        if !their_side.contains(seat_of_step(i)) {
            continue;
        }
        match edge {
            Edge::Call(Call::Bid(b)) => return Some(*b),
            Edge::Call(_) => {}
            Edge::Class(class) if class_admits_a_bid(*class) => return None,
            Edge::Class(_) => {}
        }
    }
    None
}

/// Builds the [`Node`] for one candidate and inserts it into the trie; on a collision with an
/// existing entry (first definition wins), discards it and returns the existing node instead.
#[allow(clippy::too_many_arguments)]
fn build_or_reuse_node(
    row: &BmlNode,
    side: Side,
    row_id: RowId,
    path: Arc<[SidedPattern]>,
    call: Call,
    seat_now: Seat,
    prior_last_bid: Option<Bid>,
    role: Role,
    seat: SeatCond,
    vul: VulCond,
    meta: &SystemMeta,
    next_frame: &Frame,
    ex: &mut Expansion,
) -> Option<NodeId> {
    // An empty-description entry (typically a history token re-traced under this table's own
    // `#SEAT`/`#VUL`) never outranks an already-defined, non-empty entry that covers it: reuse
    // that node instead of inserting a new, more specific entry whose `HandConstraint::ANY` and
    // blank description would otherwise win at lookup for the narrower condition and shadow the
    // real definition (`docs/design/06-system.md` §4.2/§9.3).
    if row.description.text.trim().is_empty() {
        if let Some(existing) =
            ex.trie
                .covering_entry(we_opened_of(next_frame), &next_frame.edges, seat, vul)
        {
            // This row is never given its own node (the trie entry is reused, see above), so it
            // would otherwise keep the `Recognition::default()` it was created with (ratio 0.0)
            // forever, dragging down every `mean recognition` roll-up over `SystemIR::rows` with
            // a phantom failure for a row that has no description to recognise in the first
            // place. An empty description is defined as fully recognised (`recognition::compute`'s
            // own early return, ratio 1.0); give this row that same, correct value instead of
            // leaving the pre-compile placeholder in place.
            let row_recognition = &mut ex.rows[row_id.0 as usize].recognition;
            if row_recognition.total == 0 && row_recognition.ratio == 0.0 {
                row_recognition.ratio = 1.0;
            }
            return Some(existing);
        }
    }

    let calls = next_frame.calls();
    let level = call.bid().map_or(0, |b| b.level());
    let hash = hash_suit(&next_frame.path, &next_frame.resolved);

    let table_side = if seat_now.side() == bridge_core::Side::NS {
        TableSide::NS
    } else {
        TableSide::EW
    };
    let own_prev =
        next_frame.last_by_seat[seat_now.index() as usize].map(|id| &ex.nodes[id.0 as usize]);
    let partner_last = next_frame.last_by_seat[seat_now.partner().index() as usize]
        .map(|id| &ex.nodes[id.0 as usize]);
    let their_last_bid = opponents_last_bid(&next_frame.edges, seat_now);
    let bid_suit = call.bid().and_then(|b| b.strain().suit());
    // Partner's call only agrees `bid_suit` when it actually shows length there: a bare
    // same-strain call is not enough on its own (`docs/design/06-system.md` §7.5's
    // `partner_len_min = partner_last.constraint.suit_len(agreed).start`). Two guards, both
    // needed: `!artificial` rules out relays/asks that name a suit only as a code (a puppet or a
    // step response), and the length check rules out a *non*-artificial call that mentions the
    // strain while showing shortness or the other hand's suits there (e.g. opener's `2S` over
    // `1C-1D-2S = 16+, 5+!c and 4+!h`, which is clubs-and-hearts, not spades). Without the length
    // guard a later splinter in that same strain would wrongly agree it and AND its own
    // shortness claim against a phantom length requirement, contradicting itself.
    let partner_bid_this_suit = bid_suit.is_some_and(|suit| {
        partner_last.is_some_and(|n| {
            !n.flags.artificial
                && n.call.bid().and_then(|b| b.strain().suit()) == Some(suit)
                && *n.constraint.suit_len(suit).start() >= 3
        })
    });
    let agreed_suit = if partner_bid_this_suit {
        bid_suit
    } else {
        own_prev
            .and_then(|n| n.flags.agreed_suit)
            .or_else(|| partner_last.and_then(|n| n.flags.agreed_suit))
    };

    let ctx = RowContext {
        call,
        side: table_side,
        level,
        is_jump: is_jump(call, prior_last_bid),
        binding: &next_frame.env,
        hash_suit: hash,
        own_prev,
        partner_last,
        their_last_bid,
        agreed_suit,
        role,
    };

    let substituted = substitute_description(&row.description.text, &next_frame.env);
    let compiled = compile_description(&substituted, &ctx, meta);
    debug_assert!(
        compiled.constraint.is_samplable(),
        "the description compiler must never produce HandConstraint::Custom"
    );
    // `LintCode::LowRecognition` is dropped here: it is a per-*row* roll-up over that row's best
    // expansion (`check_recognition`, run once the whole file is expanded), and the description
    // compiler's own per-node copy would otherwise report the same threshold breach twice for a
    // row with a single expansion (the common case).
    ex.lints.extend(
        compiled
            .lints
            .into_iter()
            .filter(|l| l.code != LintCode::LowRecognition),
    );
    let recognition = compiled.recognition.clone();

    let node_id = NodeId(ex.nodes.len() as u32);
    let node = Node {
        id: node_id,
        row: row_id,
        side,
        path,
        calls,
        call,
        binding: next_frame.env,
        seat,
        vul,
        volume_log2: estimate_volume_log2(&compiled.constraint),
        constraint: compiled.constraint,
        branch_weights: compiled.branch_weights,
        priority: compiled.priority,
        alertable: if row.description.alert {
            Alertability::Alertable
        } else {
            Alertability::Unspecified
        },
        flags: compiled.flags,
        description: substituted,
        children: Vec::new(),
    };
    ex.nodes.push(node);

    let row_recognition = &mut ex.rows[row_id.0 as usize].recognition;
    if row_recognition.total == 0 || recognition.ratio > row_recognition.ratio {
        *row_recognition = recognition;
    }

    // A genuine specificity tie (two *different*, overlapping conditions that would both match
    // the same real auction with equal specificity) is reported once, before insertion, since
    // `insert_path`'s own `Err` path is reserved for an *identical* condition (`DuplicatePath`,
    // first wins outright, no ambiguity to report) -- see `AuctionTrie::tied_entry`.
    if let Some(existing) =
        ex.trie
            .tied_entry(we_opened_of(next_frame), &next_frame.edges, seat, vul)
    {
        ex.lints.push(
            Lint::info(
                LintCode::ConditionTie,
                format!(
                    "{}: this seat/vul condition has the same specificity as node {} and \
                     overlaps it; insertion order decides which one lookup prefers",
                    row.calls[0].raw, existing.0
                ),
            )
            .with_row(row_id),
        );
    }

    let insert_result = ex.trie.insert_path(
        we_opened_of(next_frame),
        &next_frame.edges,
        seat,
        vul,
        node_id,
    );

    match insert_result {
        Ok(()) => {
            if !row.description.text.trim().is_empty() {
                fill_covered_placeholders(node_id, we_opened_of(next_frame), next_frame, ex);
            }
            Some(node_id)
        }
        Err(existing) => {
            let new_node = ex.nodes.pop().expect("just pushed");
            handle_duplicate(row_id, existing, new_node, ex);
            Some(existing)
        }
    }
}

/// The mirror image of `build_or_reuse_node`'s `covering_entry` guard, for the opposite file
/// order: a more specific, empty-description entry for the same call (typically a history token
/// re-traced under a `#SEAT`/`#VUL` table that came *before* the general definition, e.g. through
/// `#INCLUDE` order) was inserted first, so it would outrank the general `node_id` at lookup for
/// its narrower condition with a blank description and `HandConstraint::ANY`. Every such
/// placeholder that `node_id`'s condition covers is filled with `node_id`'s compiled content
/// (keeping its own id, children and `#SEAT`/`#VUL` condition), exactly like
/// [`handle_duplicate`]'s fill for an identical condition, so lookup gives the same meaning
/// whichever order the two definitions appear in.
///
/// Known limit: rows below the placeholder that were already expanded were compiled with the
/// placeholder's `ANY` constraint as their `own_prev`/`partner_last` context; only the node
/// itself is repaired here.
fn fill_covered_placeholders(
    node_id: NodeId,
    we_opened: bool,
    next_frame: &Frame,
    ex: &mut Expansion,
) {
    let (seat, vul) = {
        let n = &ex.nodes[node_id.0 as usize];
        (n.seat, n.vul)
    };
    for placeholder in ex
        .trie
        .covered_entries(we_opened, &next_frame.edges, seat, vul)
    {
        if placeholder == node_id || !ex.nodes[placeholder.0 as usize].description.is_empty() {
            continue;
        }
        let template = ex.nodes[node_id.0 as usize].clone();
        let target = &mut ex.nodes[placeholder.0 as usize];
        let children = std::mem::take(&mut target.children);
        *target = Node {
            id: placeholder,
            children,
            seat: target.seat,
            vul: target.vul,
            ..template
        };
        ex.lints.push(
            Lint::info(
                LintCode::DuplicatePath,
                "a later, less specific definition filled this more specific placeholder's \
                 empty description",
            )
            .with_row(ex.nodes[node_id.0 as usize].row)
            .with_node(placeholder),
        );
    }
}

/// `we_opened` (for the trie's two-root split) is fixed for the whole table and equals the side
/// of the very first path entry.
fn we_opened_of(frame: &Frame) -> bool {
    frame
        .path
        .first()
        .map(|p| p.side == Side::Us)
        .unwrap_or(true)
}

/// A row's candidate collided with an already-inserted entry (`docs/design/06-system.md` §4.2:
/// "duplicate paths -> first wins"). `new_node` is the freshly built, not-yet-shared node that
/// lost; it is discarded except when it *fills* the existing node's empty description, in which
/// case every one of its compiled fields -- not just the description -- replaces the existing
/// node's, since that existing node was only ever a placeholder (constraint `ANY`, default
/// flags/priority/weights) created by an earlier, incomplete re-trace of the same call.
fn handle_duplicate(row_id: RowId, existing: NodeId, new_node: Node, ex: &mut Expansion) {
    let existing_description = ex.nodes[existing.0 as usize].description.clone();
    if new_node.description.is_empty() || new_node.description == existing_description {
        return; // the ordinary "another table retraces this prefix" case: nothing to report.
    }
    if existing_description.is_empty() {
        let existing_node = &mut ex.nodes[existing.0 as usize];
        let id = existing_node.id;
        let children = std::mem::take(&mut existing_node.children);
        *existing_node = Node {
            id,
            children,
            ..new_node
        };
        ex.lints.push(
            Lint::info(
                LintCode::DuplicatePath,
                "a later definition filled this node's empty description",
            )
            .with_row(row_id)
            .with_node(existing),
        );
    } else {
        ex.lints.push(
            Lint::warning(
                LintCode::DuplicatePath,
                format!(
                    "redefinition with a different, non-empty description ({:?} vs {existing_description:?})",
                    new_node.description
                ),
            )
            .with_row(row_id)
            .with_node(existing),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::{CallPattern, OppClass, StrainSet};
    use bridge_core::Strain;

    #[test]
    fn implicit_pass_only_between_same_side_calls() {
        assert!(!needs_implicit_pass(None, Side::Us));
        assert!(needs_implicit_pass(Some(Side::Us), Side::Us));
        assert!(!needs_implicit_pass(Some(Side::Us), Side::Them));
        assert!(needs_implicit_pass(Some(Side::Them), Side::Them));
    }

    #[test]
    fn exact_pattern_yields_one_candidate() {
        let cands = generate_candidates(
            &CallPattern::Exact(Call::Pass),
            &Binding::default(),
            StrainSet::EMPTY,
            None,
            false,
        );
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].edge, Edge::Call(Call::Pass));
    }

    #[test]
    fn strains_literal_generates_every_strain_at_the_fixed_level() {
        let pat = CallPattern::Strains {
            level: Level::At(2),
            strains: StrainSet::EMPTY.with(Strain::Hearts).with(Strain::Spades),
        };
        let cands = generate_candidates(&pat, &Binding::default(), StrainSet::EMPTY, None, false);
        let calls: Vec<Call> = cands
            .iter()
            .map(|c| match c.edge {
                Edge::Call(call) => call,
                Edge::Class(_) => unreachable!(),
            })
            .collect();
        assert_eq!(
            calls,
            vec![
                Call::Bid(Bid::new(2, Strain::Hearts).unwrap()),
                Call::Bid(Bid::new(2, Strain::Spades).unwrap()),
            ]
        );
    }

    #[test]
    fn var_major_binds_and_generates_both_majors_when_unused() {
        let pat = CallPattern::Var {
            level: Level::At(1),
            var: Var::Major,
        };
        let cands = generate_candidates(&pat, &Binding::default(), StrainSet::EMPTY, None, false);
        assert_eq!(cands.len(), 2);
        assert_eq!(cands[0].binding.get(Var::Major), Some(Strain::Hearts));
        assert_eq!(cands[1].binding.get(Var::Major), Some(Strain::Spades));
    }

    #[test]
    fn var_reuses_existing_binding_without_branching() {
        let env = Binding::default().bind(Var::Major, Strain::Spades);
        let pat = CallPattern::Var {
            level: Level::At(2),
            var: Var::Major,
        };
        let cands = generate_candidates(&pat, &env, StrainSet::EMPTY, None, false);
        assert_eq!(cands.len(), 1);
        assert_eq!(
            cands[0].edge,
            Edge::Call(Call::Bid(Bid::new(2, Strain::Spades).unwrap()))
        );
    }

    #[test]
    fn other_major_without_major_bound_yields_no_candidates() {
        let pat = CallPattern::Var {
            level: Level::At(2),
            var: Var::OtherMajor,
        };
        let cands = generate_candidates(&pat, &Binding::default(), StrainSet::EMPTY, None, false);
        assert!(cands.is_empty());
    }

    #[test]
    fn step_counts_above_the_last_bid() {
        let last = Bid::new(1, Strain::Diamonds).unwrap(); // index 1
        let cands = generate_candidates(
            &CallPattern::Step(2),
            &Binding::default(),
            StrainSet::EMPTY,
            Some(last),
            false,
        );
        assert_eq!(
            cands[0].edge,
            Edge::Call(Call::Bid(Bid::new(1, Strain::Spades).unwrap())) // index 1 + 2 = 3 = 1S
        );
    }

    #[test]
    fn step_without_anchor_yields_no_candidates() {
        let cands = generate_candidates(
            &CallPattern::Step(1),
            &Binding::default(),
            StrainSet::EMPTY,
            None,
            false,
        );
        assert!(cands.is_empty());
    }

    #[test]
    fn step_and_fresh_variable_forbidden_under_wildcard() {
        assert!(
            generate_candidates(
                &CallPattern::Step(1),
                &Binding::default(),
                StrainSet::EMPTY,
                Some(Bid::new(1, Strain::Clubs).unwrap()),
                true,
            )
            .is_empty()
        );
        assert!(
            generate_candidates(
                &CallPattern::Var {
                    level: Level::At(2),
                    var: Var::X
                },
                &Binding::default(),
                StrainSet::EMPTY,
                None,
                true,
            )
            .is_empty()
        );
    }

    #[test]
    fn class_pattern_is_a_single_wildcard_edge() {
        let cands = generate_candidates(
            &CallPattern::Class(OppClass::AnyBid),
            &Binding::default(),
            StrainSet::EMPTY,
            None,
            false,
        );
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].edge, Edge::Class(OppClass::AnyBid));
    }

    #[test]
    fn any_of_concatenates_each_alternative() {
        let pat = CallPattern::AnyOf(vec![
            CallPattern::Exact(Call::Bid(Bid::new(2, Strain::Spades).unwrap())),
            CallPattern::Exact(Call::Bid(Bid::new(3, Strain::Hearts).unwrap())),
        ]);
        let cands = generate_candidates(&pat, &Binding::default(), StrainSet::EMPTY, None, false);
        assert_eq!(cands.len(), 2);
    }

    #[test]
    fn substitute_description_replaces_word_bound_variables_only() {
        let env = Binding::default().bind(Var::Major, Strain::Hearts);
        let out = substitute_description("4+M shows a Major and Moon", &env);
        // Only the standalone `M` is substituted; `Major`/`Moon` are untouched.
        assert_eq!(out, "4+!h shows a Major and Moon");
    }

    #[test]
    fn substitute_description_leaves_unbound_variables_as_is() {
        let out = substitute_description("5+M", &Binding::default());
        assert_eq!(out, "5+M");
    }

    #[test]
    fn substitute_description_replaces_a_digit_prefixed_variable() {
        let env = Binding::default().bind(Var::Major, Strain::Hearts);
        assert_eq!(substitute_description("5M or 4M", &env), "5!h or 4!h");

        let env = env.bind(Var::Major, Strain::Hearts);
        assert_eq!(substitute_description("3oM", &env), "3!s");

        let env = Binding::default().bind(Var::Minor, Strain::Clubs);
        assert_eq!(substitute_description("2m", &env), "2!c");
    }

    #[test]
    fn substitute_description_leaves_a_literal_hash_untouched() {
        // Unlike `M`/`m`/`oM`/`om`/`X`/`Y`/`Z`, `#` is never substituted in description *text*
        // (only `RowContext.hash_suit` resolves it, for the constraint compiler) -- confirmed
        // against the reference `bss.py`, and against `bml-test`'s `example8.bml`, whose
        // description text uses a literal `#` as a LaTeX-special-character example, not a
        // variable.
        let out = substitute_description("good 4+#", &Binding::default());
        assert_eq!(out, "good 4+#");
    }

    #[test]
    fn hash_suit_finds_the_nearest_variable_position() {
        let path = vec![
            SidedPattern {
                side: Side::Us,
                pat: CallPattern::Exact(Call::Bid(Bid::new(1, Strain::Clubs).unwrap())),
            },
            SidedPattern {
                side: Side::Them,
                pat: CallPattern::Strains {
                    level: Level::At(2),
                    strains: StrainSet::EMPTY.with(Strain::Hearts).with(Strain::Spades),
                },
            },
        ];
        let resolved = vec![
            Call::Bid(Bid::new(1, Strain::Clubs).unwrap()),
            Call::Bid(Bid::new(2, Strain::Spades).unwrap()),
        ];
        assert_eq!(hash_suit(&path, &resolved), Some(Suit::Spades));
    }

    #[test]
    fn hash_suit_is_none_when_no_variable_position_precedes() {
        let path = vec![SidedPattern {
            side: Side::Us,
            pat: CallPattern::Exact(Call::Bid(Bid::new(1, Strain::Clubs).unwrap())),
        }];
        let resolved = vec![Call::Bid(Bid::new(1, Strain::Clubs).unwrap())];
        assert_eq!(hash_suit(&path, &resolved), None);
    }

    #[test]
    fn is_jump_detects_a_skipped_level() {
        let last = Bid::new(1, Strain::Clubs).unwrap();
        assert!(!is_jump(
            Call::Bid(Bid::new(1, Strain::Diamonds).unwrap()),
            Some(last)
        ));
        assert!(is_jump(
            Call::Bid(Bid::new(2, Strain::Diamonds).unwrap()),
            Some(last)
        ));
        assert!(!is_jump(
            Call::Bid(Bid::new(1, Strain::Clubs).unwrap()),
            None
        ));
        assert!(is_jump(
            Call::Bid(Bid::new(2, Strain::Clubs).unwrap()),
            None
        ));
        // A change of strain to a lower level is not a jump, even though 2H comes after 1S in
        // bidding order: 2H is the minimum sufficient bid in hearts over 1S.
        let last = Bid::new(1, Strain::Spades).unwrap();
        assert!(!is_jump(
            Call::Bid(Bid::new(2, Strain::Hearts).unwrap()),
            Some(last)
        ));
    }

    #[test]
    fn estimate_volume_log2_matches_a_hand_written_count() {
        use bridge_constraint::{Atom, HandConstraint};
        use bridge_core::ShapeSet;
        // A single HCP value (span 1) over every 13-card shape: volume == the shape count.
        let atom = Atom::ANY.with_hcp(20..=20);
        let c = HandConstraint::Atom(atom);
        let expected = (ShapeSet::ALL.len() as f64).log2().round() as i16;
        assert_eq!(estimate_volume_log2(&c), expected);
    }

    #[test]
    fn estimate_volume_log2_of_unsatisfiable_is_min() {
        use bridge_constraint::{Atom, HandConstraint};
        let atom = Atom::ANY.with_hcp(38..=40); // above the achievable max (37): unsatisfiable
        let c = HandConstraint::Atom(atom);
        assert_eq!(estimate_volume_log2(&c), i16::MIN);
    }

    #[test]
    fn opponents_last_bid_ignores_our_own_and_partners_bids() {
        // `1N-(P)-2C`: dealer North bids 1N, East (Them) passes, South (Us) bids 2C. `Them`'s
        // only call is a pass, so there is no bid from them yet, not the auction's last bid
        // (1N, which is ours).
        let edges = [
            Edge::Call(Call::Bid(Bid::new(1, Strain::NoTrump).unwrap())),
            Edge::Call(Call::Pass),
        ];
        assert_eq!(opponents_last_bid(&edges, Seat::South), None);
    }

    #[test]
    fn opponents_last_bid_finds_the_opponents_own_bid() {
        // 1C-(1H)-1S: South's row context should see East's 1H as their_last_bid.
        let edges = [
            Edge::Call(Call::Bid(Bid::new(1, Strain::Clubs).unwrap())),
            Edge::Call(Call::Bid(Bid::new(1, Strain::Hearts).unwrap())),
        ];
        assert_eq!(
            opponents_last_bid(&edges, Seat::South),
            Some(Bid::new(1, Strain::Hearts).unwrap())
        );
    }

    #[test]
    fn opponents_last_bid_is_unknown_behind_a_bidding_wildcard() {
        // 1C-(1H)-P-(suit): the opponents' wildcard hides whatever they bid after 1H.
        let edges = [
            Edge::Call(Call::Bid(Bid::new(1, Strain::Clubs).unwrap())),
            Edge::Call(Call::Bid(Bid::new(1, Strain::Hearts).unwrap())),
            Edge::Call(Call::Pass),
            Edge::Class(OppClass::AnySuitBid),
        ];
        assert_eq!(opponents_last_bid(&edges, Seat::North), None);
    }

    #[test]
    fn relaxed_legality_matches_auction_without_wildcards() {
        let bid = |l, s| Call::Bid(Bid::new(l, s).unwrap());
        let sequences: [&[Call]; 5] = [
            &[],
            &[bid(1, Strain::Clubs)],
            &[bid(1, Strain::Clubs), Call::Double],
            &[bid(1, Strain::Clubs), Call::Pass, Call::Pass],
            &[Call::Pass, Call::Pass, Call::Pass],
        ];
        for calls in sequences {
            let auction =
                Auction::from_calls(Seat::North, Vulnerability::None, calls.iter().copied())
                    .unwrap();
            let edges: Vec<Edge> = calls.iter().copied().map(Edge::Call).collect();
            for call in (0..=37u8).filter_map(Call::from_index) {
                assert_eq!(
                    relaxed_is_legal(&edges, call),
                    auction.is_legal(call),
                    "{calls:?} then {call:?}"
                );
            }
        }
    }

    #[test]
    fn relaxed_legality_below_a_wildcard() {
        let c1 = Edge::Call(Call::Bid(Bid::new(1, Strain::Clubs).unwrap()));
        let suit = Edge::Class(OppClass::AnySuitBid);
        let pass = Edge::Call(Call::Pass);
        // 1C (suit) X: doubling their unknown suit bid.
        assert!(relaxed_is_legal(&[c1, suit], Call::Double));
        // 1C (suit) P (P) X: reopening double of the same unknown bid.
        assert!(relaxed_is_legal(&[c1, suit, pass, pass], Call::Double));
        // 1C (suit) P (P) P: three certain passes after a certainly non-pass wildcard end it.
        assert!(!relaxed_is_legal(&[c1, suit, pass, pass, pass], Call::Pass));
        // 1C (P-class) X: doubling partner's bid is still illegal.
        assert!(!relaxed_is_legal(
            &[c1, Edge::Class(OppClass::Pass)],
            Call::Double
        ));
        // 1C (any) XX: the wildcard may be a Double.
        assert!(relaxed_is_legal(
            &[c1, Edge::Class(OppClass::AnyCall)],
            Call::Redouble
        ));
        // A bid must still beat the last known bid.
        assert!(!relaxed_is_legal(
            &[
                Edge::Call(Call::Bid(Bid::new(1, Strain::Diamonds).unwrap())),
                suit
            ],
            Call::Bid(Bid::new(1, Strain::Clubs).unwrap())
        ));
    }

    // -- Full-table expansion scenarios (built as AST literals, bypassing the parser, so each
    // scenario is exact) --------------------------------------------------------------------

    /// A fresh, never-repeated span: real parsed rows always have distinct spans (each `#INCLUDE`
    /// occurrence gets its own `FileId`, see `lexer::load_file`; two rows on the same file always
    /// differ in `line`), and `expand_row` now keys its row-deduplication cache on exactly this
    /// span (`Expansion::row_by_span`), so a fixed dummy span across every test-built token would
    /// wrongly collapse genuinely distinct rows into one.
    fn test_span() -> crate::ast::Span {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT_LINE: AtomicU32 = AtomicU32::new(1);
        crate::ast::Span {
            file: crate::ast::FileId(0),
            line: NEXT_LINE.fetch_add(1, Ordering::Relaxed),
            col: 0,
            pasted_from: None,
        }
    }

    fn tok(side: Side, pattern: CallPattern, raw: &str) -> CallToken {
        CallToken {
            side,
            pattern,
            raw: raw.to_string(),
            span: test_span(),
        }
    }

    fn exact_tok(side: Side, call: Call, raw: &str) -> CallToken {
        tok(side, CallPattern::Exact(call), raw)
    }

    fn some_desc(text: &str) -> Description {
        Description {
            text: text.to_string(),
            alert: false,
            col: 0,
        }
    }

    fn test_row(calls: Vec<CallToken>, text: &str, children: Vec<BmlNode>) -> BmlNode {
        BmlNode {
            calls,
            description: some_desc(text),
            children,
            indent: 0,
            span: test_span(),
        }
    }

    fn test_table(seat: SeatCond, history: Vec<CallToken>, rows: Vec<BmlNode>) -> BidTable {
        BidTable {
            hidden: false,
            seat,
            vul: VulCond::default(),
            history,
            history_desc: None,
            rows,
            span: test_span(),
        }
    }

    fn expand(tables: &[&BidTable]) -> Expansion {
        expand_file(tables, &SystemMeta::default(), &CompileOptions::default())
    }

    #[test]
    fn seat_retrace_reuses_the_general_definition_instead_of_shadowing_it() {
        // Table 1: the general opening definition, no #SEAT restriction.
        let opening = test_table(
            SeatCond::Any,
            Vec::new(),
            vec![test_row(
                vec![exact_tok(
                    Side::Us,
                    Call::Bid(Bid::new(1, Strain::Hearts).unwrap()),
                    "1H",
                )],
                "11-15 hcp, 5+!h",
                Vec::new(),
            )],
        );
        // Table 2: `#SEAT 34` retraces `1H-` (empty description: a pure history token) and adds
        // its own response.
        let seat34 = test_table(
            SeatCond::ThirdOrFourth,
            vec![exact_tok(
                Side::Us,
                Call::Bid(Bid::new(1, Strain::Hearts).unwrap()),
                "1H",
            )],
            vec![test_row(
                vec![exact_tok(
                    Side::Us,
                    Call::Bid(Bid::new(2, Strain::Clubs).unwrap()),
                    "2C",
                )],
                "MAX, 5+!c",
                Vec::new(),
            )],
        );

        let ex = expand(&[&opening, &seat34]);

        // Only two real nodes: the opening and its 2C child. No shadowing placeholder for the
        // seat-34 retrace of 1H.
        assert_eq!(ex.nodes.len(), 2);
        assert_eq!(ex.nodes[0].description, "11-15 hcp, 5+!h");
        assert_eq!(ex.nodes[0].children, vec![NodeId(1)]);

        // Resolving 1H for a 3rd/4th-seat opener returns the *general* node, not a shadowing
        // empty one.
        let key = crate::trie::LookupKey {
            we_opened: true,
            calls: &[Call::Bid(Bid::new(1, Strain::Hearts).unwrap())],
            opener_pos: 3,
            vul: crate::trie::RelVul {
                we: false,
                they: false,
            },
        };
        let lookup = ex.trie.resolve(&key);
        assert_eq!(lookup.by_depth[0], Some(NodeId(0)));

        // Regression: the seat-34 retrace still gets its own `Row` (RowId(1), row 0 is the
        // opening's own "1H" and row 2 is its "2C" child) even though it reuses the opening's
        // node -- and that row's `description_raw` is genuinely empty (a pure history retrace,
        // not real content), so its recognition must be the same "empty description, ratio 1.0"
        // answer `compile_description` gives everywhere else, not the pre-compile
        // `Recognition::default()` placeholder (ratio 0.0) it was created with. Left unfixed,
        // this phantom row drags down every mean-recognition roll-up over `SystemIR::rows` by one
        // 0.0 entry per history retrace in the file, for content that was never actually
        // unrecognised.
        assert_eq!(ex.rows.len(), 3);
        assert_eq!(ex.rows[1].description_raw, "");
        assert_eq!(ex.rows[1].recognition.ratio, 1.0);
        assert_eq!(ex.rows[1].recognition.total, 0);
    }

    #[test]
    fn fill_replaces_every_field_of_the_placeholder_not_just_the_description() {
        // Table 1: a history retrace of `1N` (empty description) with its own child, written
        // before `1N` is ever really defined.
        let retrace_first = test_table(
            SeatCond::Any,
            vec![exact_tok(
                Side::Us,
                Call::Bid(Bid::new(1, Strain::NoTrump).unwrap()),
                "1N",
            )],
            vec![test_row(
                vec![exact_tok(
                    Side::Us,
                    Call::Bid(Bid::new(2, Strain::Clubs).unwrap()),
                    "2C",
                )],
                "no major",
                Vec::new(),
            )],
        );
        // Table 2: the real definition of the opening `1N`, coming later in the file.
        let real_definition = test_table(
            SeatCond::Any,
            Vec::new(),
            vec![test_row(
                vec![exact_tok(
                    Side::Us,
                    Call::Bid(Bid::new(1, Strain::NoTrump).unwrap()),
                    "1N",
                )],
                "15-17 hcp, bal",
                Vec::new(),
            )],
        );

        let ex = expand(&[&retrace_first, &real_definition]);

        assert_eq!(ex.nodes.len(), 2);
        let n1 = &ex.nodes[0];
        assert_eq!(n1.description, "15-17 hcp, bal");
        // The child linked while the node was still a placeholder is kept.
        assert_eq!(n1.children, vec![NodeId(1)]);
        // The row now points at the real definition's row, not the history retrace's.
        assert_eq!(ex.rows[n1.row.0 as usize].description_raw, "15-17 hcp, bal");
        // The constraint is no longer the wide-open placeholder: narrower than a bare `Atom::ANY`.
        use bridge_constraint::Atom;
        let any_volume = estimate_volume_log2(&HandConstraint::Atom(Atom::ANY));
        assert!(n1.volume_log2 < any_volume);
    }

    #[test]
    fn a_multi_candidate_ancestor_makes_its_child_row_accumulate_not_duplicate() {
        // `1M` (unbound `Var::Major`) has two candidates (1H, 1S); each candidate's own
        // recursion into the child row `2C` must land in the *same* `Row` (one physical BML
        // line), with both nodes in that one row's `expansions` -- not a fresh `Row` created per
        // candidate (which would silently double `rows.len()` and any per-row roll-up over it).
        let child = test_row(
            vec![exact_tok(
                Side::Us,
                Call::Bid(Bid::new(2, Strain::Clubs).unwrap()),
                "2C",
            )],
            "some desc",
            Vec::new(),
        );
        let table = test_table(
            SeatCond::Any,
            Vec::new(),
            vec![test_row(
                vec![tok(
                    Side::Us,
                    CallPattern::Var {
                        level: Level::At(1),
                        var: Var::Major,
                    },
                    "1M",
                )],
                "a major",
                vec![child],
            )],
        );

        let ex = expand(&[&table]);

        // 1H + 1S, each with its own 2C child: 4 nodes.
        assert_eq!(ex.nodes.len(), 4);
        // But only 2 physical rows: "1M" and "2C" (not 1 + 2).
        assert_eq!(ex.rows.len(), 2);
        let child_row = ex
            .rows
            .iter()
            .find(|r| r.description_raw == "some desc")
            .expect("the 2C row");
        assert_eq!(
            child_row.expansions.len(),
            2,
            "one 2C expansion per 1M candidate, same row"
        );
    }

    // Regression for the real-file triage (roadmap 3.2-3.4, class a/c):
    // `docs/design/06-system.md` §4.2 point 3's own example, built directly as AST literals:
    // `1C-(1X)-1H` (jdh8/blue/1C.bml). `X` (`Var::X`) excludes clubs (already used by our own
    // `1C`), so it has three candidates D/H/S, all sufficient bids over `1C` themselves. The
    // child row `1H` is then only a *legal* continuation when `X = D` (`H` outranks `D` at the
    // same level); over `X = H` it repeats the same call (insufficient) and over `X = S` it is a
    // lower strain at the same level (also insufficient) -- both illegal.
    #[test]
    fn illegal_call_is_demoted_to_info_when_a_sibling_binding_of_the_same_row_succeeds() {
        let their_x = test_row(
            vec![tok(
                Side::Them,
                CallPattern::Var {
                    level: Level::At(1),
                    var: Var::X,
                },
                "1X",
            )],
            "",
            vec![test_row(
                vec![exact_tok(
                    Side::Us,
                    Call::Bid(Bid::new(1, Strain::Hearts).unwrap()),
                    "1H",
                )],
                "F, 4=!h",
                Vec::new(),
            )],
        );
        let table = test_table(
            SeatCond::Any,
            vec![exact_tok(
                Side::Us,
                Call::Bid(Bid::new(1, Strain::Clubs).unwrap()),
                "1C",
            )],
            vec![their_x],
        );

        let ex = expand(&[&table]);

        let child_row = ex
            .rows
            .iter()
            .find(|r| r.description_raw == "F, 4=!h")
            .expect("the 1H row");
        assert_eq!(
            child_row.expansions.len(),
            1,
            "only X = D makes 1H a sufficient (legal) bid"
        );

        let illegal_lints: Vec<&Lint> = ex
            .lints
            .iter()
            .filter(|l| l.code == LintCode::IllegalCall && l.row == Some(child_row.id))
            .collect();
        assert_eq!(
            illegal_lints.len(),
            2,
            "X = H and X = S both make 1H illegal"
        );
        for lint in illegal_lints {
            assert_eq!(
                lint.severity,
                crate::Severity::Info,
                "demoted: a sibling binding (X = D) of this same row did succeed"
            );
        }
    }

    // Same family, opposite outcome: when a row is illegal under *every* binding of its
    // variable-bound ancestor, `IllegalCall` must stay `Error` (nothing to demote it against).
    // The child here re-bids `1C`, our own opening's own strain: `X` never binds to clubs (already
    // used), so every one of its three candidates (D/H/S) outranks clubs at the same level, making
    // a bare `1C` repeat insufficient regardless of which one X took.
    #[test]
    fn illegal_call_stays_an_error_when_no_binding_of_the_row_ever_succeeds() {
        let their_x = test_row(
            vec![tok(
                Side::Them,
                CallPattern::Var {
                    level: Level::At(1),
                    var: Var::X,
                },
                "1X",
            )],
            "",
            vec![test_row(
                vec![exact_tok(
                    Side::Us,
                    Call::Bid(Bid::new(1, Strain::Clubs).unwrap()),
                    "1C",
                )],
                "never legal",
                Vec::new(),
            )],
        );
        let table = test_table(
            SeatCond::Any,
            vec![exact_tok(
                Side::Us,
                Call::Bid(Bid::new(1, Strain::Clubs).unwrap()),
                "1C",
            )],
            vec![their_x],
        );

        let ex = expand(&[&table]);

        let child_row = ex
            .rows
            .iter()
            .find(|r| r.description_raw == "never legal")
            .expect("the re-bid 1C row");
        assert_eq!(child_row.expansions.len(), 0);

        let illegal_lints: Vec<&Lint> = ex
            .lints
            .iter()
            .filter(|l| l.code == LintCode::IllegalCall && l.row == Some(child_row.id))
            .collect();
        assert_eq!(illegal_lints.len(), 3, "all of X = D/H/S make 1C illegal");
        for lint in illegal_lints {
            assert_eq!(lint.severity, crate::Severity::Error);
        }
    }

    #[test]
    fn overlapping_equal_specificity_vul_conditions_across_tables_report_condition_tie() {
        // Two tables both define the opening `1C`, under different but overlapping, equally
        // specific `#VUL` conditions (`we=Yes,they=Any` vs `we=Any,they=Yes`: each has
        // specificity 1). A real "both vulnerable" auction satisfies both, so which entry
        // `best_entry` returns depends only on insertion order -- a genuine tie, not a duplicate
        // (the two conditions are not identical, so `insert_path` does not `Err`).
        let mut table_a = test_table(
            SeatCond::Any,
            Vec::new(),
            vec![test_row(
                vec![exact_tok(
                    Side::Us,
                    Call::Bid(Bid::new(1, Strain::Clubs).unwrap()),
                    "1C",
                )],
                "we vul",
                Vec::new(),
            )],
        );
        table_a.vul = crate::ast::VulCond {
            we: crate::ast::Tri::Yes,
            they: crate::ast::Tri::Any,
        };

        let mut table_b = test_table(
            SeatCond::Any,
            Vec::new(),
            vec![test_row(
                vec![exact_tok(
                    Side::Us,
                    Call::Bid(Bid::new(1, Strain::Clubs).unwrap()),
                    "1C",
                )],
                "they vul",
                Vec::new(),
            )],
        );
        table_b.vul = crate::ast::VulCond {
            we: crate::ast::Tri::Any,
            they: crate::ast::Tri::Yes,
        };

        let ex = expand(&[&table_a, &table_b]);
        assert!(
            ex.lints.iter().any(|l| l.code == LintCode::ConditionTie),
            "expected a ConditionTie lint, got: {:?}",
            ex.lints.iter().map(|l| l.code).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_later_pattern_row_duplicating_an_earlier_ones_call_does_not_merge_its_subtree() {
        // `1C-(1D)` history, then two sibling *pattern* rows that both produce 2H/2S:
        // `2M first` (a `Var::Major`) and `2HS second` (a literal `Strains` of the same two
        // suits). bss.py's `bids_processed` makes the second row's subtree not attach.
        let history = vec![
            exact_tok(
                Side::Us,
                Call::Bid(Bid::new(1, Strain::Clubs).unwrap()),
                "1C",
            ),
            exact_tok(
                Side::Them,
                Call::Bid(Bid::new(1, Strain::Diamonds).unwrap()),
                "(1D)",
            ),
        ];
        let row_2m = test_row(
            vec![tok(
                Side::Us,
                CallPattern::Var {
                    level: Level::At(2),
                    var: Var::Major,
                },
                "2M",
            )],
            "first",
            vec![test_row(
                vec![exact_tok(
                    Side::Us,
                    Call::Bid(Bid::new(2, Strain::NoTrump).unwrap()),
                    "2N",
                )],
                "a",
                Vec::new(),
            )],
        );
        let row_2hs = test_row(
            vec![tok(
                Side::Us,
                CallPattern::Strains {
                    level: Level::At(2),
                    strains: StrainSet::EMPTY.with(Strain::Hearts).with(Strain::Spades),
                },
                "2HS",
            )],
            "second",
            vec![test_row(
                vec![exact_tok(
                    Side::Us,
                    Call::Bid(Bid::new(3, Strain::Clubs).unwrap()),
                    "3C",
                )],
                "b",
                Vec::new(),
            )],
        );
        let table = test_table(SeatCond::Any, history, vec![row_2m, row_2hs]);

        let ex = expand(&[&table]);

        // The 2H node (row "first") has exactly one child (2N), never 3C from "second".
        let two_hearts = Call::Bid(Bid::new(2, Strain::Hearts).unwrap());
        let node_2h = ex
            .nodes
            .iter()
            .find(|n| n.call == two_hearts)
            .expect("2H node exists");
        assert_eq!(node_2h.children.len(), 1);
        let child = &ex.nodes[node_2h.children[0].0 as usize];
        assert_eq!(child.description, "a");

        // The later pattern row is skipped before any node is built, silently (bss.py's
        // `bid not in bids_processed`): no DuplicatePath, and "second" has no expansion.
        assert!(
            ex.lints.iter().all(|l| l.code != LintCode::DuplicatePath),
            "{:?}",
            ex.lints
        );
        let second = ex
            .rows
            .iter()
            .find(|r| r.description_raw == "second")
            .expect("row exists");
        assert!(second.expansions.is_empty());
    }

    #[test]
    fn agreed_suit_is_set_when_a_call_supports_partners_suit() {
        let opening = exact_tok(
            Side::Us,
            Call::Bid(Bid::new(1, Strain::Hearts).unwrap()),
            "1H",
        );
        let support = exact_tok(
            Side::Us,
            Call::Bid(Bid::new(2, Strain::Hearts).unwrap()),
            "2H",
        );
        let table = test_table(
            SeatCond::Any,
            Vec::new(),
            vec![test_row(
                vec![opening],
                "12+ hcp, 5+!h",
                vec![test_row(vec![support], "6-9 hcp, 3+ support", Vec::new())],
            )],
        );

        let ex = expand(&[&table]);

        let two_hearts = Call::Bid(Bid::new(2, Strain::Hearts).unwrap());
        let node = ex
            .nodes
            .iter()
            .find(|n| n.call == two_hearts)
            .expect("2H node exists");
        assert_eq!(node.flags.agreed_suit, Some(Suit::Hearts));
    }

    /// Regression for `systems/vendor/data/bml-test/data/example3.bml:27`'s `1C-1D-2S / 3S
    /// Splinter`: `2S` is a two-suiter ("5+!c and 4+!h") that happens to share its strain with
    /// the row's own later `3S` call, but never shows *spade* length. `partner_bid_this_suit`
    /// must not agree spades from that same-strain coincidence alone (it previously did,
    /// producing `suit_len[S] >= 4` ANDed against the splinter's own `suit_len[S] <= 1`, an
    /// unsatisfiable contradiction that was wrongly attributed to the source file rather than to
    /// this compiler bug).
    #[test]
    fn splinter_does_not_agree_partners_same_strain_call_with_no_shown_length_there() {
        let opening = exact_tok(
            Side::Us,
            Call::Bid(Bid::new(1, Strain::Clubs).unwrap()),
            "1C",
        );
        let overcall = exact_tok(
            Side::Them,
            Call::Bid(Bid::new(1, Strain::Diamonds).unwrap()),
            "1D",
        );
        let rebid = exact_tok(
            Side::Us,
            Call::Bid(Bid::new(2, Strain::Spades).unwrap()),
            "2S",
        );
        let splinter = exact_tok(
            Side::Us,
            Call::Bid(Bid::new(3, Strain::Spades).unwrap()),
            "3S",
        );
        let table = test_table(
            SeatCond::Any,
            vec![opening, overcall],
            vec![test_row(
                vec![rebid],
                "16+ hcp, 5+!c and 4+!h",
                vec![test_row(vec![splinter], "Splinter", Vec::new())],
            )],
        );

        let ex = expand(&[&table]);

        let three_spades = Call::Bid(Bid::new(3, Strain::Spades).unwrap());
        let node = ex
            .nodes
            .iter()
            .find(|n| n.call == three_spades)
            .expect("3S node exists");
        assert!(
            node.constraint.is_satisfiable(),
            "3S's splinter must not agree spades from 2S's same-strain call alone: {:?}",
            node.constraint
        );
        assert!(
            !ex.lints
                .iter()
                .any(|l| l.code == LintCode::UnsatisfiableConstraint),
            "unexpected UnsatisfiableConstraint: {:?}",
            ex.lints
        );
    }

    #[test]
    fn is_founding_call_and_infer_role_cover_every_role() {
        // Opener: side just opened, neither seat of the side has acted.
        assert!(is_founding_call(false, false));
        assert_eq!(infer_role(Side::Us, true, true, false), Role::Opener);

        // Responder: partner (mate) already made a real bid.
        assert!(!is_founding_call(false, true));
        assert_eq!(infer_role(Side::Us, true, false, false), Role::Responder);

        // Opener rebid: the same seat bid before, regardless of the mate.
        assert!(is_founding_call(true, false));
        assert!(is_founding_call(true, true));
        assert_eq!(infer_role(Side::Us, true, true, true), Role::Opener);

        // Overcaller: the non-opening side's founding call, no prior competitive action.
        assert_eq!(infer_role(Side::Them, true, true, false), Role::Overcaller);

        // Advancer: partner (the overcaller) already bid.
        assert_eq!(infer_role(Side::Them, true, false, false), Role::Advancer);

        // Balancer: the non-opening side's founding call, but only after both sides have
        // already acted (a pass does not, by itself, establish a role -- see
        // `is_founding_call`).
        assert_eq!(infer_role(Side::Them, true, true, true), Role::Balancer);
    }
}
