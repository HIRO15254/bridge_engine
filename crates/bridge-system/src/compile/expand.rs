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
    ast::{BidTable, BmlNode, CallToken, Description, SeatCond, VulCond},
    compile::desc::{compile_description, context::RowContext},
    natural::Role,
    pattern::{Binding, CallPattern, Level, Side, SidedPattern, StrainSet, Var},
    trie::Edge,
};

/// Everything the expansion stage produces.
pub(crate) struct Expansion {
    pub rows: Vec<Row>,
    pub nodes: Vec<Node>,
    pub trie: crate::trie::AuctionTrie,
    pub lints: Vec<Lint>,
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
    };
    for table in tables {
        if ex.nodes.len() >= opts.max_nodes {
            break;
        }
        expand_table(table, meta, opts, &mut ex);
    }
    ex
}

/// The state threaded down one path of the expansion tree.
#[derive(Clone)]
struct Frame {
    /// The authored pattern path (history plus rows), one entry per real call (no implicit
    /// passes): shared into every [`Row`]/[`Node`] created along this path.
    path: Vec<SidedPattern>,
    /// `path`'s 1:1 concrete-call counterpart (a `Call::Pass` filler at a wildcard step).
    resolved: Vec<Call>,
    /// The concrete auction, dealer fixed at North; implicit passes and the wildcard filler are
    /// both real entries here, so `auction.calls().len()` always matches the trie depth.
    auction: Auction,
    /// The trie path in lock-step with `auction`: a concrete [`Edge::Call`] everywhere except a
    /// wildcard step, which is [`Edge::Class`] (its `auction`/`resolved` entry is a `Pass`
    /// filler, never meant to be read as "they passed").
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
        Seat::North.offset((self.auction.calls().len() % 4) as u8)
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
                .filter_map(|s| minimum_sufficient_bid(s, last_bid))
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
                env.candidates(*var, used)
                    .into_iter()
                    .flat_map(|strain| {
                        bids_at_level(*level, strain, last_bid)
                            .into_iter()
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

/// The bid(s) at `level` in `strain`: one for `Level::At`, or the single minimum sufficient one
/// for `Level::Any`.
fn bids_at_level(level: Level, strain: Strain, last_bid: Option<Bid>) -> Vec<Bid> {
    match level {
        Level::At(n) => Bid::new(n, strain).into_iter().collect(),
        Level::Any => minimum_sufficient_bid(strain, last_bid)
            .into_iter()
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

/// A call skipped at least one level below the minimum sufficient bid.
fn is_jump(call: Call, last_bid: Option<Bid>) -> bool {
    match call {
        Call::Bid(b) => {
            let min_sufficient = match last_bid {
                None => Bid::new(1, b.strain()),
                Some(last) => Bid::from_index(last.index() + 1),
            };
            min_sufficient.is_some_and(|min| b.level() > min.level())
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

/// Substitutes bound variables in a description: `M`, `oM`, `m`, `om`, `X`/`Y`/`Z` and `#` become
/// their suit sentinel (`!h` etc.); an unresolved reference is left as-is. Matching is on word
/// boundaries only (`docs/design/06-system.md` risk R4).
fn substitute_description(text: &str, env: &Binding, hash_suit: Option<Suit>) -> String {
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
        if let Some(suit) = hash_suit {
            if rest.starts_with('#') {
                out.push_str(suit_sentinel(suit));
                i += 1;
                continue;
            }
        }
        for (word, var) in VARS {
            if let Some(after) = rest.strip_prefix(word) {
                let before_ok = i == 0 || !is_word_byte(bytes[i - 1]);
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
    let mut claimed = Vec::new();
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
    let mut claimed: Vec<Call> = Vec::new();

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

fn is_exact_row(row: &BmlNode) -> bool {
    matches!(row.calls[0].pattern, CallPattern::Exact(_))
}

/// Expands one row's call token into its concrete candidates, creating (or reusing) a [`Node`]
/// per candidate.
#[allow(clippy::too_many_arguments)]
fn expand_row(
    row: &BmlNode,
    is_exact_row: bool,
    claimed_by_exact: &mut Vec<Call>,
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

    let row_id = RowId(ex.rows.len() as u32);
    ex.rows.push(Row {
        id: row_id,
        span: tok.span.clone(),
        path: Arc::clone(&row_path),
        description_raw: row.description.text.clone(),
        recognition: Recognition::default(),
        expansions: Vec::new(),
    });

    let last_bid = frame.auction.last_bid().map(|(_, b)| b);
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

    let mut out = Vec::new();
    for cand in candidates {
        if ex.nodes.len() >= opts.max_nodes {
            ex.lints.push(Lint::error(
                LintCode::TooManyNodes,
                format!("expansion aborted: reached max_nodes = {}", opts.max_nodes),
            ));
            break;
        }

        let mut next = frame.clone();

        if needs_implicit_pass(next.path.last().map(|p| p.side), side) {
            let opp_pass = Call::Pass;
            if next.auction.push(opp_pass).is_err() {
                continue; // the auction is already complete: nothing legal follows.
            }
            next.edges.push(Edge::Call(opp_pass));
        }

        let seat_now = next.next_seat();
        let concrete_call = match cand.edge {
            Edge::Call(call) => {
                if !next.auction.is_legal(call) {
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
                next.auction.push(call).expect("checked is_legal");
                next.edges.push(Edge::Call(call));
                if let Call::Bid(b) = call {
                    next.used = next.used.with(b.strain());
                }
                call
            }
            Edge::Class(class) => {
                // No concrete call; auction/used stay put (a `Pass` filler keeps `auction`'s
                // depth matching the trie's), per §4.2 point 3.
                let filler = Call::Pass;
                if next.auction.push(filler).is_err() {
                    continue;
                }
                next.edges.push(Edge::Class(class));
                next.under_wildcard = true;
                filler
            }
        };

        if is_exact_row {
            claimed_by_exact.push(concrete_call);
        } else if claimed_by_exact.contains(&concrete_call) {
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

        let is_first_of_side = match side {
            Side::Us => frame.last_by_seat[seat_now.index() as usize].is_none(),
            Side::Them => frame.last_by_seat[seat_now.index() as usize].is_none(),
        };
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
    out
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
    let calls = next_frame.auction.calls().to_vec();
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
    let their_last_bid = if side == Side::Us {
        prior_last_bid
    } else {
        None
    };
    let agreed_suit = own_prev
        .and_then(|n| n.flags.agreed_suit)
        .or_else(|| partner_last.and_then(|n| n.flags.agreed_suit));

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

    let substituted = substitute_description(&row.description.text, &next_frame.env, hash);
    let compiled = compile_description(&substituted, &ctx, meta);
    debug_assert!(
        compiled.constraint.is_samplable(),
        "the description compiler must never produce HandConstraint::Custom"
    );
    ex.lints.extend(compiled.lints);
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

    let has_wildcard = next_frame.edges.iter().any(|e| matches!(e, Edge::Class(_)));
    let insert_result = if has_wildcard {
        ex.trie.insert_path(
            we_opened_of(next_frame),
            &next_frame.edges,
            seat,
            vul,
            node_id,
        )
    } else {
        ex.trie.insert(
            we_opened_of(next_frame),
            next_frame.auction.calls(),
            seat,
            vul,
            node_id,
        )
    };

    match insert_result {
        Ok(()) => Some(node_id),
        Err(existing) => {
            let new_description = ex.nodes.pop().expect("just pushed").description;
            handle_duplicate(row_id, existing, &new_description, ex);
            Some(existing)
        }
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

fn handle_duplicate(row_id: RowId, existing: NodeId, new_description: &str, ex: &mut Expansion) {
    let existing_description = ex.nodes[existing.0 as usize].description.clone();
    if new_description.is_empty() || new_description == existing_description {
        return; // the ordinary "another table retraces this prefix" case: nothing to report.
    }
    if existing_description.is_empty() {
        ex.nodes[existing.0 as usize].description = new_description.to_string();
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
                    "redefinition with a different, non-empty description ({new_description:?} vs {existing_description:?})"
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
        let out = substitute_description("4+M shows a Major and Moon", &env, None);
        // Only the standalone `M` is substituted; `Major`/`Moon` are untouched.
        assert_eq!(out, "4+!h shows a Major and Moon");
    }

    #[test]
    fn substitute_description_leaves_unbound_variables_as_is() {
        let out = substitute_description("5+M", &Binding::default(), None);
        assert_eq!(out, "5+M");
    }

    #[test]
    fn substitute_description_maps_hash_to_the_given_suit() {
        let out = substitute_description("good 4+#", &Binding::default(), Some(Suit::Diamonds));
        assert_eq!(out, "good 4+!d");
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
}
