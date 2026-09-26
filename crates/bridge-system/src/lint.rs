//! Compile-time diagnostics.

use std::collections::HashMap;

use bridge_constraint::{Atom, Dnf, DnfOptions};

use crate::{CompileOptions, NodeId, RowId, SystemIR, ast::Span};

/// Severity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// Lint codes, grouped by stage.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum LintCode {
    // parse
    IncludeNotFound,
    IncludeCycle,
    UnknownDirective,
    PasteUnknownName,
    UnknownCallToken,
    SequenceNotFirst,
    IndentationMismatch,
    NonStandardToken,
    ColumnZeroContinuation,
    // expansion
    IllegalCall,
    UnboundOther,
    VariableNoCandidate,
    StepWithoutAnchor,
    WideWildcard,
    DuplicatePath,
    ShadowedByExact,
    ConditionTie,
    TooManyNodes,
    // constraints
    UnsatisfiableConstraint,
    ContradictsOwnHistory,
    LowRecognition,
    EmptyDescription,
    UnrecognizedFragment,
    SoftConstraint,
    AssumedContext,
    SiblingSubset,
    SiblingOverlap,
    DnfTruncated,
    // coverage
    MissingOpeningCoverage,
    MissingResponseCoverage,
}

/// One diagnostic.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Lint {
    /// Severity.
    pub severity: Severity,
    /// Code.
    pub code: LintCode,
    /// The row, if any.
    pub row: Option<RowId>,
    /// The node, if any.
    pub node: Option<NodeId>,
    /// Source location, if any.
    pub span: Option<Span>,
    /// Message.
    pub message: String,
}

impl core::fmt::Display for Severity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        })
    }
}

impl Lint {
    /// A new lint with no row, node or span attached yet.
    pub fn new(severity: Severity, code: LintCode, message: impl Into<String>) -> Lint {
        Lint {
            severity,
            code,
            row: None,
            node: None,
            span: None,
            message: message.into(),
        }
    }

    /// An [`Severity::Error`] lint.
    pub fn error(code: LintCode, message: impl Into<String>) -> Lint {
        Lint::new(Severity::Error, code, message)
    }

    /// A [`Severity::Warning`] lint.
    pub fn warning(code: LintCode, message: impl Into<String>) -> Lint {
        Lint::new(Severity::Warning, code, message)
    }

    /// An [`Severity::Info`] lint.
    pub fn info(code: LintCode, message: impl Into<String>) -> Lint {
        Lint::new(Severity::Info, code, message)
    }

    /// Attaches a source location.
    #[must_use]
    pub fn with_span(mut self, span: Span) -> Lint {
        self.span = Some(span);
        self
    }

    /// Attaches the row this lint concerns.
    #[must_use]
    pub fn with_row(mut self, row: RowId) -> Lint {
        self.row = Some(row);
        self
    }

    /// Attaches the node this lint concerns.
    #[must_use]
    pub fn with_node(mut self, node: NodeId) -> Lint {
        self.node = Some(node);
        self
    }
}

impl core::fmt::Display for Lint {
    /// `file:line: severity[Code]: message`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match &self.span {
            Some(span) => write!(
                f,
                "{}:{}: {}[{:?}]: {}",
                span.file.0, span.line, self.severity, self.code, self.message
            ),
            None => write!(
                f,
                "<no location>: {}[{:?}]: {}",
                self.severity, self.code, self.message
            ),
        }
    }
}

/// Summary counts for the compile-time `INFO` line.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LintSummary {
    /// Errors.
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
    /// Infos.
    pub infos: usize,
}

impl LintSummary {
    /// Counts by severity.
    pub fn of(lints: &[Lint]) -> LintSummary {
        let mut s = LintSummary::default();
        for l in lints {
            match l.severity {
                Severity::Error => s.errors += 1,
                Severity::Warning => s.warnings += 1,
                Severity::Info => s.infos += 1,
            }
        }
        s
    }
}

/// Runs the seven post-compile checks of `docs/design/06-system.md` §9.3 over a finished IR,
/// appending their findings to `ir.lints`.
///
/// Checks 3 (`IllegalCall`) and 4's "duplicate/tie" half are enforced as they happen during
/// expansion (`compile::expand`), not re-derived here. Check 5's per-fragment lints
/// (`EmptyDescription`, `UnrecognizedFragment`, `SoftConstraint`, `AssumedContext`) likewise come
/// from the description compiler itself; this pass only does the `Row`-level roll-up
/// (`LowRecognition`) that needs every expansion of a row to be finished first. Check 7
/// (coverage) needs `bridge_constraint::Sampler`, still `todo!()` on this branch (phase 2), so it
/// is skipped whenever `opts.coverage_samples == 0` *or* the sampler is unavailable — currently
/// always the latter.
pub fn run_post_compile_checks(ir: &mut SystemIR, opts: &CompileOptions) {
    check_satisfiability(ir);
    check_own_history(ir);
    check_recognition(ir);
    check_sibling_ambiguity(ir);
    check_coverage(ir, opts);
}

/// §9.3 check 1: every node's constraint must be satisfiable.
fn check_satisfiability(ir: &mut SystemIR) {
    let mut new_lints = Vec::new();
    for node in &ir.nodes {
        if !node.constraint.is_satisfiable() {
            new_lints.push(
                Lint::error(
                    LintCode::UnsatisfiableConstraint,
                    "no hand satisfies this constraint",
                )
                .with_row(node.row)
                .with_node(node.id)
                .with_span(ir.row(node.row).span.clone()),
            );
        }
    }
    ir.lints.extend(new_lints);
}

/// §9.3 check 2: a node's constraint, conjoined with the *same player's own* previous call's,
/// must stay satisfiable (an opener's rebid promising more than the opening did, say).
///
/// The nearest ancestor on the same `side` is usually *partner's* node, not this player's own
/// (within one partnership the two players' real calls alternate up the tree), so ANDing against
/// it would compare, say, the opener's hand against the responder's -- always meant to be
/// unsatisfiable and not what this check is for. Node doesn't need a new field to find the right
/// ancestor: `calls` is the concrete path from this synthetic table's own start (dealer fixed at
/// North, `docs/design/06-system.md` §4.2), so its length mod 4 is the synthetic seat that made
/// each call, and it agrees across tables for the same shared node (re-tracing always replays
/// the identical prefix).
fn check_own_history(ir: &mut SystemIR) {
    let parent_of = parent_map(ir);
    let mut new_lints = Vec::new();
    for node in &ir.nodes {
        let seat_index = node.calls.len() % 4;
        let mut cur = parent_of.get(&node.id).copied();
        while let Some(id) = cur {
            let ancestor = ir.node(id);
            if ancestor.side == node.side && ancestor.calls.len() % 4 == seat_index {
                let combined = node.constraint.clone().and(ancestor.constraint.clone());
                if !combined.is_satisfiable() {
                    new_lints.push(
                        Lint::warning(
                            LintCode::ContradictsOwnHistory,
                            "conjoined with this player's own previous call's constraint, no \
                             hand satisfies both",
                        )
                        .with_row(node.row)
                        .with_node(node.id)
                        .with_span(ir.row(node.row).span.clone()),
                    );
                }
                break;
            }
            cur = parent_of.get(&id).copied();
        }
    }
    ir.lints.extend(new_lints);
}

/// §9.3 check 5 (roll-up half): a row whose best (highest-ratio) expansion is still below
/// `meta.recognition_threshold`.
fn check_recognition(ir: &mut SystemIR) {
    let threshold = ir.meta.recognition_threshold;
    let mut new_lints = Vec::new();
    for row in &ir.rows {
        if row.recognition.total == 0 {
            continue; // an empty description: `EmptyDescription` is the description compiler's.
        }
        if row.recognition.ratio < threshold {
            let severity = if row.recognition.constraint_bearing {
                Severity::Warning
            } else {
                Severity::Info
            };
            new_lints.push(
                Lint::new(
                    severity,
                    LintCode::LowRecognition,
                    format!(
                        "recognition ratio {:.2} is below the threshold {:.2}",
                        row.recognition.ratio, threshold
                    ),
                )
                .with_row(row.id)
                .with_span(row.span.clone()),
            );
        }
    }
    ir.lints.extend(new_lints);
}

/// §9.3 check 6: among nodes with the same parent, side, seat and vulnerability condition, a
/// later sibling's DNF contained in an earlier one's is unreachable (`SiblingSubset`); any
/// overlap short of that is merely worth knowing about (`SiblingOverlap`). The table's *openings*
/// (the trie roots -- nodes with no parent at all) are one such sibling group too, grouped the
/// same way, since an opening bid can shadow another opening bid exactly as a later child can
/// shadow an earlier one.
fn check_sibling_ambiguity(ir: &mut SystemIR) {
    let opts = DnfOptions::default();
    let mut new_lints = Vec::new();

    let mut has_parent: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
    for node in &ir.nodes {
        has_parent.extend(node.children.iter().copied());
    }

    for parent in &ir.nodes {
        check_sibling_group(ir, &parent.children, &opts, &mut new_lints);
    }

    let roots: Vec<NodeId> = ir
        .nodes
        .iter()
        .map(|n| n.id)
        .filter(|id| !has_parent.contains(id))
        .collect();
    check_sibling_group(ir, &roots, &opts, &mut new_lints);

    ir.lints.extend(new_lints);
}

/// One group of nodes that share a parent (or, for the table's openings, share none): grouped
/// further by `(side, seat, vul)`, then checked pairwise for [`LintCode::SiblingSubset`] /
/// [`LintCode::SiblingOverlap`] in row order.
fn check_sibling_group(
    ir: &SystemIR,
    children: &[NodeId],
    opts: &DnfOptions,
    new_lints: &mut Vec<Lint>,
) {
    let mut groups: HashMap<
        (
            crate::pattern::Side,
            crate::ast::SeatCond,
            crate::ast::VulCond,
        ),
        Vec<NodeId>,
    > = HashMap::new();
    for &child in children {
        let n = ir.node(child);
        groups
            .entry((n.side, n.seat, n.vul))
            .or_default()
            .push(child);
    }

    for siblings in groups.into_values() {
        if siblings.len() < 2 {
            continue;
        }
        let dnfs: Vec<Option<Dnf>> = siblings
            .iter()
            .map(|&id| ir.node(id).constraint.to_dnf(opts).ok())
            .collect();
        for i in 0..siblings.len() {
            for j in (i + 1)..siblings.len() {
                let (Some(earlier), Some(later)) = (&dnfs[i], &dnfs[j]) else {
                    continue; // truncated beyond `max_terms`; skip, per §9.3 point 6.
                };
                if dnf_subset(later, earlier) {
                    let earlier_node = ir.node(siblings[i]);
                    let later_node = ir.node(siblings[j]);
                    let severity = if earlier_node.priority == later_node.priority {
                        Severity::Warning
                    } else {
                        Severity::Info
                    };
                    new_lints.push(
                        Lint::new(
                            severity,
                            LintCode::SiblingSubset,
                            "this sibling's constraint is a subset of an earlier sibling's; \
                             it can never be reached",
                        )
                        .with_row(later_node.row)
                        .with_node(later_node.id)
                        .with_span(ir.row(later_node.row).span.clone()),
                    );
                } else if dnf_overlap(earlier, later) {
                    let later_node = ir.node(siblings[j]);
                    new_lints.push(
                        Lint::info(
                            LintCode::SiblingOverlap,
                            "this sibling's constraint overlaps an earlier sibling's",
                        )
                        .with_row(later_node.row)
                        .with_node(later_node.id)
                        .with_span(ir.row(later_node.row).span.clone()),
                    );
                }
            }
        }
    }
}

/// §9.3 check 7 (coverage): disabled until `bridge_constraint::Sampler` (phase 2) lands on this
/// branch, and whenever `opts.coverage_samples == 0`.
fn check_coverage(ir: &mut SystemIR, opts: &CompileOptions) {
    if opts.coverage_samples == 0 {
        return;
    }
    tracing::debug!(
        target: "bridge_system::lint",
        "coverage checks (MissingOpeningCoverage/MissingResponseCoverage) skipped: \
         bridge_constraint::Sampler is not implemented on this branch yet"
    );
    let _ = ir;
}

/// `Atom` `a ⊆ b`: `hcp` and `shapes` are range/bitset containment; `cards`/`eval` are compared as
/// sets (`b`'s literals must all also hold in `a`, since a subset hand set can only be *more*
/// constrained).
fn atom_subset(a: &Atom, b: &Atom) -> bool {
    a.hcp.start() >= b.hcp.start()
        && a.hcp.end() <= b.hcp.end()
        && a.shapes.intersect(b.shapes) == a.shapes
        && b.cards.iter().all(|r| a.cards.contains(r))
        && b.eval.iter().all(|r| a.eval.contains(r))
}

/// `a`'s DNF is a subset of `b`'s: every term of `a` is contained in some term of `b`.
fn dnf_subset(a: &Dnf, b: &Dnf) -> bool {
    a.terms
        .iter()
        .all(|ta| b.terms.iter().any(|tb| atom_subset(&ta.atom, &tb.atom)))
}

/// Some term of `a` and some term of `b` are simultaneously satisfiable.
fn dnf_overlap(a: &Dnf, b: &Dnf) -> bool {
    a.terms.iter().any(|ta| {
        b.terms
            .iter()
            .any(|tb| !ta.atom.intersect(&tb.atom).is_trivially_unsat())
    })
}

/// `NodeId -> parent NodeId`, derived from every node's `children`.
fn parent_map(ir: &SystemIR) -> HashMap<NodeId, NodeId> {
    let mut map = HashMap::new();
    for node in &ir.nodes {
        for &child in &node.children {
            map.insert(child, node.id);
        }
    }
    map
}

#[cfg(test)]
mod post_compile_tests {
    use std::sync::Arc;

    use bridge_constraint::HandConstraint;
    use bridge_core::{Bid, Call, Strain};

    use super::*;
    use crate::{
        Alertability, BalancedDef, ConventionDefaults, Node, NodeFlags, Recognition, Row,
        StrengthVocab, SystemMeta, TieBreak,
        ast::{FileId, SeatCond, Span, VulCond},
        natural::{AdvanceParams, NaturalParams, RebidParams, ResponseParams},
        pattern::{CallPattern, Side, SidedPattern},
        trie::AuctionTrie,
    };

    /// `NaturalParams::default()` is a phase-3 stub owned by another lane; these tests only need
    /// *some* valid value.
    fn dummy_natural_params() -> NaturalParams {
        NaturalParams {
            opening_hcp: 12..=21,
            open_1m_len: 3,
            open_1major_len: 5,
            nt: vec![(1, 15..=17)],
            weak_two: (6, 5..=10),
            preempt: vec![(3, 7, 5..=10)],
            strong_two_c: 22,
            overcall: [(5, 8..=16), (5, 8..=16), (5, 8..=16)],
            nt_overcall: 15..=18,
            takeout_double: (12, 15, 3),
            response: ResponseParams {
                new_suit_1: (4, 6),
                new_suit_2: (5, 10),
                raise: (3, 6..=9),
                jump_raise: (4, 10..=12),
                nt: vec![(1, 6..=9)],
                jump_shift: 17,
            },
            rebid: RebidParams {
                reverse: 17,
                jump_rebid: 16..=18,
                nt_1: 12..=14,
                nt_2: 18..=19,
                raise: 16..=18,
                jump_raise: 19..=20,
            },
            advance: AdvanceParams {
                raise: (3, 6..=9),
                new_suit: (5, 8),
                cue: 10,
            },
            balancing_shift: -3,
            implicit_raise_support: true,
            level_floor: Default::default(),
        }
    }

    fn meta() -> SystemMeta {
        SystemMeta {
            name: String::new(),
            description: String::new(),
            authors: Vec::new(),
            version: "0".to_string(),
            date: None,
            source_hash: [0; 32],
            compiler_version: String::new(),
            ir_format: crate::IR_FORMAT,
            dist_method: bridge_eval::DistMethod::GOREN_321,
            tie_break: TieBreak::default(),
            strength: StrengthVocab::default(),
            balanced: BalancedDef::default(),
            natural: dummy_natural_params(),
            conventions: ConventionDefaults::default(),
            recognition_threshold: 0.5,
            extra: Default::default(),
        }
    }

    fn span() -> Span {
        Span {
            file: FileId(0),
            line: 1,
            col: 0,
            pasted_from: None,
        }
    }

    fn bid(level: u8, strain: Strain) -> Call {
        Call::Bid(Bid::new(level, strain).unwrap())
    }

    /// A minimal row + node pair: `row_id == node_id` (both simply the insertion index), no
    /// children yet (the caller wires those up).
    struct Builder {
        rows: Vec<Row>,
        nodes: Vec<Node>,
    }

    impl Builder {
        fn new() -> Builder {
            Builder {
                rows: Vec::new(),
                nodes: Vec::new(),
            }
        }

        #[allow(clippy::too_many_arguments)]
        fn push(
            &mut self,
            side: Side,
            call: Call,
            constraint: HandConstraint,
            priority: i16,
            recognition_ratio: f32,
        ) -> NodeId {
            let id = NodeId(self.nodes.len() as u32);
            let row_id = RowId(self.rows.len() as u32);
            let path: Arc<[SidedPattern]> = vec![SidedPattern {
                side,
                pat: CallPattern::Exact(call),
            }]
            .into();
            self.rows.push(Row {
                id: row_id,
                span: span(),
                path: Arc::clone(&path),
                description_raw: String::new(),
                recognition: Recognition {
                    covered: (recognition_ratio * 10.0) as u16,
                    total: 10,
                    ratio: recognition_ratio,
                    unrecognized: Vec::new(),
                    constraint_bearing: true,
                    assumed: 0,
                    soft: 0,
                },
                expansions: vec![id],
            });
            self.nodes.push(Node {
                id,
                row: row_id,
                side,
                path,
                calls: vec![call],
                call,
                binding: Binding::default(),
                seat: SeatCond::Any,
                vul: VulCond::default(),
                constraint,
                branch_weights: None,
                priority,
                volume_log2: 0,
                alertable: Alertability::Unspecified,
                flags: NodeFlags::default(),
                description: String::new(),
                children: Vec::new(),
            });
            id
        }

        fn link(&mut self, parent: NodeId, child: NodeId) {
            self.nodes[parent.0 as usize].children.push(child);
        }

        fn finish(self) -> SystemIR {
            SystemIR {
                meta: meta(),
                rows: self.rows,
                nodes: self.nodes,
                index: AuctionTrie::new(),
                lints: Vec::new(),
                exclusive_cell: Default::default(),
            }
        }
    }

    fn opts() -> CompileOptions {
        CompileOptions {
            coverage_samples: 0,
            strict_dnf: false,
            max_nodes: 1000,
        }
    }

    use crate::pattern::Binding;

    #[test]
    fn unsatisfiable_constraint_is_reported() {
        let mut b = Builder::new();
        // hcp 20..=10 is an empty (inverted) range: unsatisfiable.
        let atom = Atom::ANY.with_hcp(38..=40);
        b.push(
            Side::Us,
            bid(1, Strain::Clubs),
            HandConstraint::Atom(atom),
            0,
            1.0,
        );
        let mut ir = b.finish();

        check_satisfiability(&mut ir);
        assert!(
            ir.lints
                .iter()
                .any(|l| l.code == LintCode::UnsatisfiableConstraint
                    && l.severity == Severity::Error)
        );
    }

    #[test]
    fn satisfiable_constraint_is_not_reported() {
        let mut b = Builder::new();
        b.push(Side::Us, bid(1, Strain::Clubs), HandConstraint::ANY, 0, 1.0);
        let mut ir = b.finish();
        check_satisfiability(&mut ir);
        assert!(ir.lints.is_empty());
    }

    #[test]
    fn contradicts_own_history_when_rebid_narrows_below_its_opening() {
        let mut b = Builder::new();
        let opening = b.push(
            Side::Us,
            bid(1, Strain::Clubs),
            HandConstraint::Atom(Atom::ANY.with_hcp(12..=14)),
            0,
            1.0,
        );
        let rebid = b.push(
            Side::Us,
            bid(2, Strain::Diamonds),
            HandConstraint::Atom(Atom::ANY.with_hcp(18..=20)),
            0,
            1.0,
        );
        b.link(opening, rebid);
        let mut ir = b.finish();

        check_own_history(&mut ir);
        assert!(
            ir.lints
                .iter()
                .any(|l| l.code == LintCode::ContradictsOwnHistory && l.node == Some(rebid))
        );
    }

    #[test]
    fn own_history_does_not_flag_a_consistent_rebid() {
        let mut b = Builder::new();
        let opening = b.push(
            Side::Us,
            bid(1, Strain::Clubs),
            HandConstraint::Atom(Atom::ANY.with_hcp(12..=21)),
            0,
            1.0,
        );
        let rebid = b.push(
            Side::Us,
            bid(2, Strain::Diamonds),
            HandConstraint::Atom(Atom::ANY.with_hcp(18..=21)),
            0,
            1.0,
        );
        b.link(opening, rebid);
        let mut ir = b.finish();
        check_own_history(&mut ir);
        assert!(ir.lints.is_empty());
    }

    #[test]
    fn opponents_history_is_not_checked_against_our_own() {
        let mut b = Builder::new();
        let opening = b.push(
            Side::Us,
            bid(1, Strain::Clubs),
            HandConstraint::Atom(Atom::ANY.with_hcp(12..=14)),
            0,
            1.0,
        );
        // A `Them` node nested under our node: unrelated side, never combined.
        let overcall = b.push(
            Side::Them,
            bid(1, Strain::Diamonds),
            HandConstraint::Atom(Atom::ANY.with_hcp(18..=21)),
            0,
            1.0,
        );
        b.link(opening, overcall);
        let mut ir = b.finish();
        check_own_history(&mut ir);
        assert!(ir.lints.is_empty());
    }

    #[test]
    fn own_history_compares_the_same_players_calls_not_partners() {
        // opener 1C (12-14) - (P) - responder 1H (6-9, contradicts nothing of opener's own
        // range, but WOULD contradict it if wrongly compared) - (P) - opener rebid 2C (18-20):
        // the rebid must be checked against the opener's own *opening* (1C), not against the
        // responder's 1H in between.
        let mut b = Builder::new();
        let opener_open = b.push(
            Side::Us,
            bid(1, Strain::Clubs),
            HandConstraint::Atom(Atom::ANY.with_hcp(12..=14)),
            0,
            1.0,
        );
        b.nodes[opener_open.0 as usize].calls = vec![bid(1, Strain::Clubs)]; // depth 1 (seat N)

        let responder_resp = b.push(
            Side::Us,
            bid(1, Strain::Hearts),
            HandConstraint::Atom(Atom::ANY.with_hcp(6..=9)),
            0,
            1.0,
        );
        b.nodes[responder_resp.0 as usize].calls =
            vec![bid(1, Strain::Clubs), Call::Pass, bid(1, Strain::Hearts)]; // depth 3 (seat S) -- a different physical player from the opener.
        b.link(opener_open, responder_resp);

        let opener_rebid = b.push(
            Side::Us,
            bid(2, Strain::Clubs),
            HandConstraint::Atom(Atom::ANY.with_hcp(18..=20)),
            0,
            1.0,
        );
        b.nodes[opener_rebid.0 as usize].calls = vec![
            bid(1, Strain::Clubs),
            Call::Pass,
            bid(1, Strain::Hearts),
            Call::Pass,
            bid(2, Strain::Clubs),
        ]; // depth 5 (seat N again): the same physical player as the opening.
        b.link(responder_resp, opener_rebid);

        let mut ir = b.finish();
        check_own_history(&mut ir);

        // The rebid (18-20) contradicts the *opening* (12-14), not the responder's 1H (6-9) it
        // is nested under in the tree.
        assert!(
            ir.lints
                .iter()
                .any(|l| l.code == LintCode::ContradictsOwnHistory && l.node == Some(opener_rebid))
        );
        // And responder's own 1H, which contradicts nothing of the opener's, is not flagged.
        assert!(!ir.lints.iter().any(|l| l.node == Some(responder_resp)));
    }

    #[test]
    fn own_history_does_not_compare_against_partners_contradictory_range() {
        // opener 1C (16+, an artificially strong-club-like range) - (P) - responder 2H (weak,
        // 0-10): with the old "nearest same-side ancestor" logic this compares responder's hand
        // against opener's and (being genuinely disjoint) would always warn; it must not, since
        // it is comparing two different players' hands.
        let mut b = Builder::new();
        let opener_open = b.push(
            Side::Us,
            bid(1, Strain::Clubs),
            HandConstraint::Atom(Atom::ANY.with_hcp(16..=37)),
            0,
            1.0,
        );
        b.nodes[opener_open.0 as usize].calls = vec![bid(1, Strain::Clubs)];

        let responder_resp = b.push(
            Side::Us,
            bid(2, Strain::Hearts),
            HandConstraint::Atom(Atom::ANY.with_hcp(0..=10)),
            0,
            1.0,
        );
        b.nodes[responder_resp.0 as usize].calls =
            vec![bid(1, Strain::Clubs), Call::Pass, bid(2, Strain::Hearts)];
        b.link(opener_open, responder_resp);

        let mut ir = b.finish();
        check_own_history(&mut ir);
        assert!(ir.lints.is_empty());
    }

    #[test]
    fn low_recognition_below_threshold_is_reported() {
        let mut b = Builder::new();
        b.push(Side::Us, bid(1, Strain::Clubs), HandConstraint::ANY, 0, 0.2);
        let mut ir = b.finish();
        check_recognition(&mut ir);
        assert!(ir.lints.iter().any(|l| l.code == LintCode::LowRecognition));
    }

    #[test]
    fn recognition_at_or_above_threshold_is_not_reported() {
        let mut b = Builder::new();
        b.push(Side::Us, bid(1, Strain::Clubs), HandConstraint::ANY, 0, 0.9);
        let mut ir = b.finish();
        check_recognition(&mut ir);
        assert!(ir.lints.is_empty());
    }

    #[test]
    fn sibling_subset_flags_an_unreachable_later_sibling() {
        let mut b = Builder::new();
        let parent = b.push(Side::Us, bid(1, Strain::Clubs), HandConstraint::ANY, 0, 1.0);
        let earlier = b.push(
            Side::Us,
            bid(1, Strain::Diamonds),
            HandConstraint::Atom(Atom::ANY.with_hcp(0..=21)),
            0,
            1.0,
        );
        let later = b.push(
            Side::Us,
            bid(1, Strain::Hearts),
            HandConstraint::Atom(Atom::ANY.with_hcp(12..=14)),
            0,
            1.0,
        );
        b.link(parent, earlier);
        b.link(parent, later);
        let mut ir = b.finish();

        check_sibling_ambiguity(&mut ir);
        assert!(
            ir.lints
                .iter()
                .any(|l| l.code == LintCode::SiblingSubset && l.node == Some(later))
        );
    }

    #[test]
    fn sibling_subset_is_info_when_priorities_differ() {
        let mut b = Builder::new();
        let parent = b.push(Side::Us, bid(1, Strain::Clubs), HandConstraint::ANY, 0, 1.0);
        let earlier = b.push(
            Side::Us,
            bid(1, Strain::Diamonds),
            HandConstraint::Atom(Atom::ANY.with_hcp(0..=21)),
            0,
            1.0,
        );
        let later = b.push(
            Side::Us,
            bid(1, Strain::Hearts),
            HandConstraint::Atom(Atom::ANY.with_hcp(12..=14)),
            5,
            1.0,
        );
        b.link(parent, earlier);
        b.link(parent, later);
        let mut ir = b.finish();

        check_sibling_ambiguity(&mut ir);
        let found = ir
            .lints
            .iter()
            .find(|l| l.code == LintCode::SiblingSubset && l.node == Some(later))
            .expect("subset should still be flagged");
        assert_eq!(found.severity, Severity::Info);
    }

    #[test]
    fn sibling_overlap_short_of_subset_is_info() {
        let mut b = Builder::new();
        let parent = b.push(Side::Us, bid(1, Strain::Clubs), HandConstraint::ANY, 0, 1.0);
        let earlier = b.push(
            Side::Us,
            bid(1, Strain::Diamonds),
            HandConstraint::Atom(Atom::ANY.with_hcp(0..=15)),
            0,
            1.0,
        );
        let later = b.push(
            Side::Us,
            bid(1, Strain::Hearts),
            HandConstraint::Atom(Atom::ANY.with_hcp(10..=21)),
            0,
            1.0,
        );
        b.link(parent, earlier);
        b.link(parent, later);
        let mut ir = b.finish();

        check_sibling_ambiguity(&mut ir);
        assert!(
            ir.lints
                .iter()
                .any(|l| l.code == LintCode::SiblingOverlap && l.node == Some(later))
        );
        assert!(!ir.lints.iter().any(|l| l.code == LintCode::SiblingSubset));
    }

    #[test]
    fn sibling_subset_is_flagged_among_root_level_openings_too() {
        // Two top-level openings (no parent at all -- the trie roots) where the second's
        // constraint is a subset of the first's.
        let mut b = Builder::new();
        let earlier = b.push(
            Side::Us,
            bid(1, Strain::Diamonds),
            HandConstraint::Atom(Atom::ANY.with_hcp(0..=21)),
            0,
            1.0,
        );
        let later = b.push(
            Side::Us,
            bid(1, Strain::Hearts),
            HandConstraint::Atom(Atom::ANY.with_hcp(12..=14)),
            0,
            1.0,
        );
        // No `b.link(...)`: both are roots, exactly like two opening bids.
        let mut ir = b.finish();

        check_sibling_ambiguity(&mut ir);
        assert!(
            ir.lints
                .iter()
                .any(|l| l.code == LintCode::SiblingSubset && l.node == Some(later))
        );
        let _ = earlier;
    }

    #[test]
    fn disjoint_siblings_are_not_reported() {
        let mut b = Builder::new();
        let parent = b.push(Side::Us, bid(1, Strain::Clubs), HandConstraint::ANY, 0, 1.0);
        let earlier = b.push(
            Side::Us,
            bid(1, Strain::Diamonds),
            HandConstraint::Atom(Atom::ANY.with_hcp(0..=11)),
            0,
            1.0,
        );
        let later = b.push(
            Side::Us,
            bid(1, Strain::Hearts),
            HandConstraint::Atom(Atom::ANY.with_hcp(12..=21)),
            0,
            1.0,
        );
        b.link(parent, earlier);
        b.link(parent, later);
        let mut ir = b.finish();
        check_sibling_ambiguity(&mut ir);
        assert!(ir.lints.is_empty());
    }

    #[test]
    fn coverage_is_skipped_when_samples_is_zero() {
        let mut ir = Builder::new().finish();
        check_coverage(&mut ir, &opts());
        assert!(ir.lints.is_empty());
    }

    #[test]
    fn run_post_compile_checks_runs_every_check() {
        let mut b = Builder::new();
        let atom = Atom::ANY.with_hcp(38..=40); // unsatisfiable
        b.push(
            Side::Us,
            bid(1, Strain::Clubs),
            HandConstraint::Atom(atom),
            0,
            0.1,
        );
        let mut ir = b.finish();
        run_post_compile_checks(&mut ir, &opts());
        assert!(
            ir.lints
                .iter()
                .any(|l| l.code == LintCode::UnsatisfiableConstraint)
        );
        assert!(ir.lints.iter().any(|l| l.code == LintCode::LowRecognition));
    }
}
