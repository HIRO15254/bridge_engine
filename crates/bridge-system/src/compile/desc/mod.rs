//! Description → constraint compiler.
//!
//! 1. [`normalize`]: strip the alert marker and `{prio:N}` / `{w:X}` annotations, map `!c` to a
//!    suit sentinel, `--` to `-`, collapse whitespace but keep line breaks.
//! 2. [`clause`]: parse into fragments with byte spans; precedence `and` > `or`/`/` > `,`/`;`;
//!    enumerations `a) b)` form an `Or` group; unrecognised spans are kept.
//! 3. [`tokens`]: context-free fragments (HCP, lengths, shapes, balance, cards, metrics) to
//!    partial atoms.
//! 4. [`context`]: context-dependent fragments (`GF INV MIN MAX weak PRE STR S/T QUANT NAT SPL
//!    fit TRF #`) resolved from the parent chain, the binding and `SystemMeta`, with provenance.
//! 5. Assemble into a [`HandConstraint`], compute
//!    [`Recognition`], emit `tracing` events.

pub mod clause;
pub mod context;
pub mod normalize;
pub mod recognition;
pub mod tokens;

use bridge_constraint::{Atom, DnfOptions, DnfTerm, HandConstraint};

use self::{
    clause::{Clause, Fragment, FragmentKind},
    context::{Provenance, RowContext, own_suit, suit_len_pins},
    tokens::{StrengthWord, Token},
};
use crate::{Lint, LintCode, NodeFlags, Recognition, Severity, SystemMeta};

/// The output of compiling one description.
#[derive(Clone, Debug)]
pub struct Compiled {
    /// The constraint (`ANY` for an empty or fully unrecognised description).
    pub constraint: HandConstraint,
    /// Weights of top-level `Or` branches from `{w:X}`.
    pub branch_weights: Option<Vec<f32>>,
    /// `{prio:N}`.
    pub priority: i16,
    /// Derived flags.
    pub flags: NodeFlags,
    /// Recognition statistics.
    pub recognition: Recognition,
    /// Diagnostics.
    pub lints: Vec<Lint>,
}

/// Compiles one substituted description in its context (`docs/design/06-system.md` §7.1).
///
/// `Lint`s constructed here carry no `row`/`node`/`span` (this function is not given a
/// [`crate::RowId`]/[`crate::NodeId`], and reconstructing an [`crate::ast::Span`] from a byte
/// offset needs the row's own `Span`, which only `compile/mod.rs`'s expansion loop has); the
/// byte-level detail instead goes into each lint's `message`. The caller (`compile/mod.rs`,
/// out of this task's scope) is expected to fill in `row`/`node`/`span` when it collects these
/// lints into `SystemIR::lints`.
pub fn compile_description(text: &str, ctx: &RowContext<'_>, meta: &SystemMeta) -> Compiled {
    let normalized = normalize::normalize(text);
    let (fragments, top_clause) = clause::parse(&normalized.text);

    // Pass 1's tokens already sit inside `fragments`; `context::resolve` only wants the ones
    // that need resolving, in order, so its (Atom, Provenance) pairs can be zipped back to the
    // right fragment by index.
    let token_indices: Vec<usize> = fragments
        .iter()
        .enumerate()
        .filter_map(|(i, f)| matches!(f.kind, FragmentKind::Token(_)).then_some(i))
        .collect();
    let pass1_tokens: Vec<Token> = token_indices
        .iter()
        .map(|&i| match &fragments[i].kind {
            FragmentKind::Token(t) => t.clone(),
            FragmentKind::Unrecognized(_) => {
                unreachable!("token_indices only keeps FragmentKind::Token entries")
            }
        })
        .collect();
    // Whether a fragment matching `pred` is *stated alongside* the token at `k` (an index into
    // `pass1_tokens`): see [`stated_alongside`].
    let stated_with: &context::StatedWith<'_> =
        &|k, pred| stated_alongside(&top_clause, &fragments, token_indices[k], pred);
    let (atoms, provs) = context::resolve(&pass1_tokens, ctx, meta, stated_with);
    let any = || HandConstraint::Atom(Atom::ANY);
    // Pass 2 can resolve a context-dependent strength word (`INV`, `MIN`, `S/T`, …) to an HCP
    // range that contradicts a number the author wrote out explicitly in the same description
    // (`docs/design/06-system.md` §7.5's "衝突は明示が勝つ" rule, generalized from `NAT` to every
    // `Strength` word): e.g. jdh8's `3C = INV, 7+!c, 4--7 HCP` states its own 4--7 HCP range, so
    // `INV`'s context-derived range must not be ANDed against it (that would make the row
    // unsatisfiable whenever the two disagree). A `Strength` word's HCP atom is dropped (treated
    // as `Atom::ANY`, which `build` already elides) only when an explicit `Hcp`/`Points` fragment
    // is stated alongside *that* word ([`stated_alongside`]: conjoined with it, neither negated
    // nor a mere possibility): `GF, not 20+ HCP` keeps `GF` (the negation narrows it to 13..=19),
    // `GF, may have 11 HCP` keeps `GF` (a possibility states nothing), and `weak or 16+ HCP` keeps
    // `weak` (the number describes the other branch). This does not touch a `Strength` word's
    // other effects (`NodeFlags` from `derive_flags`, e.g. `GF` still implies `Forcing::ToGame`),
    // only the HCP atom this pass would otherwise have produced from it.
    let is_number = |t: &Token| matches!(t, Token::Hcp(_) | Token::Points(_));
    let atoms: Vec<HandConstraint> = pass1_tokens
        .iter()
        .zip(atoms)
        .enumerate()
        .map(|(k, (tok, atom))| {
            if matches!(tok, Token::Strength(_)) && stated_with(k, &is_number) {
                any()
            } else {
                atom
            }
        })
        .collect();

    // `QUANT INV to 6NT`: the `INV` word only says that `QUANT` is an invitation (to a slam, not
    // to game), so its game-invitational HCP range must not be ANDed against `QUANT`'s slam-invite
    // range (the two are disjoint and the row would be unsatisfiable).
    let has_quant = pass1_tokens
        .iter()
        .any(|t| matches!(t, Token::Strength(StrengthWord::Quantitative)));
    let atoms: Vec<HandConstraint> = if has_quant {
        pass1_tokens
            .iter()
            .zip(atoms)
            .map(|(tok, atom)| match tok {
                Token::Strength(
                    StrengthWord::Invitational
                    | StrengthWord::InvitationalPlus
                    | StrengthWord::InvitationalMild
                    | StrengthWord::InvitationalStrong,
                ) => any(),
                _ => atom,
            })
            .collect()
    } else {
        atoms
    };

    // `MIN`/`MAX` halve the hand's own range so far (`own_hcp`). When the description itself
    // names a category for that range (`weak-two, MAX`: the maximum of a weak two), and the half
    // of the tracked range is disjoint from it (the tracked range is the hull of an opening's
    // `Or` branches, e.g. gjp's `2C = 1) weak-two in !d 2) 25+ NT 3) FG`, whose hull 5..=37 has
    // its upper half nowhere near a weak two), the word is re-based on the stated category, so
    // `MAX` means the top of the weak-two range rather than contradicting it.
    let atoms = rebase_relative_strength(
        &pass1_tokens,
        atoms,
        &|k| conjoined_leaves(&top_clause, &fragments, token_indices[k]),
        &token_indices,
    );

    // Known beats assumed: when the strength words of one description resolve to disjoint HCP
    // ranges and some of them rest on an assumed context (partner's or this player's range was
    // unknown, §7.5's defaults), the assumed ones are dropped rather than making the row
    // unsatisfiable (gjp `MAX, FG, 5-5`: `MAX` of the responder's own stated 5-9 against a `FG`
    // derived from an assumed partner opening range).
    let atoms = if strength_under_or(&top_clause, &fragments, false) {
        // Strength words in different `Or` branches (`weak or GF`) are meant to be disjoint.
        atoms
    } else {
        drop_conflicting_assumed_strength(&pass1_tokens, atoms, &provs)
    };

    // `NAT` itself is the origin of the "衝突は明示が勝つ" rule generalized above, but never
    // implemented it for its own suit-length atom: `resolve_natural` always ANDs
    // `suit_len[call's suit] >= natural_suit_length` (5 by default), even when the row states its
    // own length for that suit explicitly (e.g. gjp's `NAT, normally 4!s`, or an `at least 4!c`
    // read as an exact 4). An explicit `SuitLen`/`Shape` fragment that pins the length of NAT's
    // own suit and is stated alongside the `NAT` word ([`stated_alongside`]) means the author
    // already said what that length is; `NAT`'s assumed minimum must not additionally AND against
    // it (unsatisfiable whenever the two disagree, e.g. a 5+ default against a stated 4). A
    // negated length (`NAT, not 4!c`), a possibility (`NAT, may be 3!c`) or a length in only one
    // `Or` branch (`NAT, 6+!d or 4!s` for a 2D bid) does not say what the length is, so `NAT`
    // keeps its atom there.
    let atoms: Vec<HandConstraint> = match own_suit(ctx) {
        None => atoms,
        Some(suit) => {
            let pins = |t: &Token| suit_len_pins(t, suit, ctx);
            pass1_tokens
                .iter()
                .zip(atoms)
                .enumerate()
                .map(|(k, (tok, atom))| {
                    if matches!(tok, Token::Natural) && stated_with(k, &pins) {
                        any()
                    } else {
                        atom
                    }
                })
                .collect()
        }
    };

    let mut resolved: Vec<Option<(HandConstraint, Provenance)>> = vec![None; fragments.len()];
    for ((&idx, atom), mut prov) in token_indices.iter().zip(atoms).zip(provs) {
        prov.span = fragments[idx].span;
        resolved[idx] = Some((atom, prov));
    }

    let built = build(&top_clause, &fragments, &resolved).unwrap_or(HandConstraint::ANY);
    let (constraint, dnf_truncated) = simplify(built);

    let mut recognition = recognition::compute(&normalized.text, &fragments);
    recognition.assumed = u8::try_from(
        resolved
            .iter()
            .filter(|r| r.as_ref().is_some_and(|(_, p)| p.assumed))
            .count(),
    )
    .unwrap_or(u8::MAX);

    let flags = derive_flags(&fragments, &top_clause, &normalized, ctx);

    let mut lints: Vec<Lint> = Vec::new();
    let info = |code: LintCode, message: String| Lint {
        severity: Severity::Info,
        code,
        row: None,
        node: None,
        span: None,
        message,
    };

    if recognition.total == 0 {
        lints.push(info(
            LintCode::EmptyDescription,
            "empty description".to_string(),
        ));
    }

    for &(start, end) in &recognition.unrecognized {
        lints.push(info(
            LintCode::UnrecognizedFragment,
            format!(
                "unrecognized text {:?} at {start}..{end}",
                &normalized.text[start as usize..end as usize]
            ),
        ));
    }

    if recognition.ratio < meta.recognition_threshold {
        let severity = if recognition.constraint_bearing {
            Severity::Warning
        } else {
            Severity::Info
        };
        let message = format!(
            "recognition ratio {:.2} is below the {:.2} threshold",
            recognition.ratio, meta.recognition_threshold
        );
        if severity == Severity::Warning {
            tracing::warn!(
                target: "bridge_system::lint",
                code = ?LintCode::LowRecognition,
                ratio = recognition.ratio,
                "{message}"
            );
        }
        lints.push(Lint {
            severity,
            code: LintCode::LowRecognition,
            row: None,
            node: None,
            span: None,
            message,
        });
    }

    if recognition.assumed > 0 {
        lints.push(info(
            LintCode::AssumedContext,
            format!(
                "{} fragment(s) resolved with an assumed (not stated) context value",
                recognition.assumed
            ),
        ));
    }

    if recognition.soft > 0 {
        lints.push(info(
            LintCode::SoftConstraint,
            format!("{} hedged fragment(s) (usually/may/…)", recognition.soft),
        ));
    }

    if dnf_truncated {
        let message = "DNF term cap exceeded while simplifying; kept the tree form".to_string();
        tracing::warn!(target: "bridge_system::lint", code = ?LintCode::DnfTruncated, "{message}");
        lints.push(Lint {
            severity: Severity::Warning,
            code: LintCode::DnfTruncated,
            row: None,
            node: None,
            span: None,
            message,
        });
    }

    tracing::debug!(
        target: "bridge_system::compile::desc",
        desc = %normalized.text,
        ratio = recognition.ratio,
        fragments = %fragment_summary(&fragments),
        unrecognized = %unrecognized_summary(&normalized.text, &recognition.unrecognized),
        assumed = recognition.assumed,
    );

    Compiled {
        constraint,
        branch_weights: (!normalized.weights.is_empty()).then_some(normalized.weights),
        priority: normalized.priority.unwrap_or(0),
        flags,
        recognition,
        lints,
    }
}

/// Whether some fragment matching `pred` is *stated alongside* fragment `target`: it sits in a
/// conjunction with `target` (an `And` ancestor of `target` has a sibling subtree that
/// [`states`] it), so it holds in every branch in which `target` holds. A fragment in a sibling
/// `Or` branch of `target` does not count (`weak or 16+ HCP`), nor does a negated or
/// possibility-hedged one (see [`states`]). `docs/design/06-system.md` §7.5's "衝突は明示が勝つ"
/// rule uses this to decide when an explicit fragment overrides a context word's own atom.
fn stated_alongside(
    top: &Clause,
    fragments: &[Fragment],
    target: usize,
    pred: &context::TokenPred<'_>,
) -> bool {
    /// `None` when `target` is not below `clause`; otherwise whether a matching fragment is
    /// stated alongside it within `clause`.
    fn walk(
        clause: &Clause,
        fragments: &[Fragment],
        target: usize,
        pred: &context::TokenPred<'_>,
    ) -> Option<bool> {
        match clause {
            Clause::Leaf(idx) => (*idx == target).then_some(false),
            Clause::And(items) => items.iter().enumerate().find_map(|(k, item)| {
                walk(item, fragments, target, pred).map(|found| {
                    found
                        || items
                            .iter()
                            .enumerate()
                            .any(|(j, other)| j != k && states(other, fragments, pred))
                })
            }),
            Clause::Or(items) => items
                .iter()
                .find_map(|item| walk(item, fragments, target, pred)),
        }
    }
    walk(top, fragments, target, pred).unwrap_or(false)
}

/// The fragments that hold whenever fragment `target` does: every leaf reachable from a sibling
/// of an `And` ancestor of `target` through `And` nodes only (a leaf under a sibling `Or` is one
/// possibility among several, so it is left out), skipping negated and possibility-hedged ones.
fn conjoined_leaves(top: &Clause, fragments: &[Fragment], target: usize) -> Vec<usize> {
    fn and_leaves(clause: &Clause, fragments: &[Fragment], out: &mut Vec<usize>) {
        match clause {
            Clause::Leaf(idx) => {
                let f = &fragments[*idx];
                if !f.negated && !f.possibility {
                    out.push(*idx);
                }
            }
            Clause::And(items) => items.iter().for_each(|c| and_leaves(c, fragments, out)),
            Clause::Or(_) => {}
        }
    }
    fn contains(clause: &Clause, target: usize) -> bool {
        match clause {
            Clause::Leaf(idx) => *idx == target,
            Clause::And(items) | Clause::Or(items) => items.iter().any(|c| contains(c, target)),
        }
    }
    let mut out = Vec::new();
    let mut node = top;
    loop {
        match node {
            Clause::Leaf(_) => break,
            Clause::And(items) => {
                let Some(k) = items.iter().position(|c| contains(c, target)) else {
                    break;
                };
                for (j, item) in items.iter().enumerate() {
                    if j != k {
                        and_leaves(item, fragments, &mut out);
                    }
                }
                node = &items[k];
            }
            Clause::Or(items) => match items.iter().find(|c| contains(c, target)) {
                Some(next) => node = next,
                None => break,
            },
        }
    }
    out
}

/// Re-bases a `MIN`/`MAX` word on the range a conjoined category word (`weak`, `PRE`, `NEG`,
/// `strong`) states, when halving the tracked own range put it entirely outside that category
/// (see the call site). `leaves_of(k)` lists the fragment indices conjoined with token `k`;
/// `token_indices` maps token indices to fragment indices.
fn rebase_relative_strength(
    tokens: &[Token],
    mut atoms: Vec<HandConstraint>,
    leaves_of: &dyn Fn(usize) -> Vec<usize>,
    token_indices: &[usize],
) -> Vec<HandConstraint> {
    let hcp_of = |atom: &HandConstraint| match atom {
        HandConstraint::Atom(a) if *a != Atom::ANY => Some(a.hcp.clone()),
        _ => None,
    };
    for k in 0..tokens.len() {
        let is_min = match tokens[k] {
            Token::Strength(StrengthWord::Min) => true,
            Token::Strength(StrengthWord::Max) => false,
            _ => continue,
        };
        let Some(own) = hcp_of(&atoms[k]) else {
            continue;
        };
        let leaves = leaves_of(k);
        let mut base: Option<core::ops::RangeInclusive<u8>> = None;
        for (j, tok) in tokens.iter().enumerate() {
            let category = matches!(
                tok,
                Token::Strength(
                    StrengthWord::Weak
                        | StrengthWord::Preemptive
                        | StrengthWord::Negative
                        | StrengthWord::Strong
                )
            );
            if !category || !leaves.contains(&token_indices[j]) {
                continue;
            }
            if let Some(r) = hcp_of(&atoms[j]) {
                base = Some(match base {
                    None => r,
                    Some(b) => *b.start().max(r.start())..=*b.end().min(r.end()),
                });
            }
        }
        let Some(base) = base else {
            continue;
        };
        let disjoint = own.start() > base.end() || base.start() > own.end();
        if base.start() > base.end() || !disjoint {
            continue;
        }
        let (lo, hi) = (*base.start(), *base.end());
        let mid = lo + (hi - lo) / 2;
        let half = if is_min {
            lo..=mid
        } else {
            (mid + 1).min(hi)..=hi
        };
        atoms[k] = HandConstraint::Atom(Atom {
            hcp: half,
            ..Atom::ANY
        });
    }
    atoms
}

/// Whether `clause` definitely states a fragment matching `pred`: a leaf that matches and is
/// neither negated (`not 20+ HCP` states the opposite) nor possibility-hedged (`may have 11 HCP`
/// states nothing, §7.6); an `And` with any such child; an `Or` all of whose branches state one
/// (`4!s or 5!s` still pins the spade length, `6+!d or 4!s` does not pin diamonds).
fn states(clause: &Clause, fragments: &[Fragment], pred: &context::TokenPred<'_>) -> bool {
    match clause {
        Clause::Leaf(idx) => {
            let f = &fragments[*idx];
            !f.negated && !f.possibility && matches!(&f.kind, FragmentKind::Token(t) if pred(t))
        }
        Clause::And(items) => items.iter().any(|c| states(c, fragments, pred)),
        Clause::Or(items) => !items.is_empty() && items.iter().all(|c| states(c, fragments, pred)),
    }
}

/// `true` when some `Strength` fragment sits below an `Or` node of `clause` (`inside_or` says
/// whether an ancestor already was one).
fn strength_under_or(clause: &Clause, fragments: &[Fragment], inside_or: bool) -> bool {
    match clause {
        Clause::Leaf(idx) => {
            inside_or
                && matches!(
                    fragments[*idx].kind,
                    FragmentKind::Token(Token::Strength(_))
                )
        }
        Clause::And(items) => items
            .iter()
            .any(|c| strength_under_or(c, fragments, inside_or)),
        Clause::Or(items) => items.iter().any(|c| strength_under_or(c, fragments, true)),
    }
}

/// Drops the assumed-context `Strength` literals of a description whose `Strength` literals
/// (all conjoined) resolve to disjoint HCP ranges, as long as at least one known-context one
/// remains; otherwise returns `atoms` unchanged. `tokens`, `atoms` and `provs` are parallel.
fn drop_conflicting_assumed_strength(
    tokens: &[Token],
    atoms: Vec<HandConstraint>,
    provs: &[Provenance],
) -> Vec<HandConstraint> {
    let strength_hcp = |i: usize| match (&tokens[i], &atoms[i]) {
        (Token::Strength(_), HandConstraint::Atom(a)) if *a != Atom::ANY => Some(a.hcp.clone()),
        _ => None,
    };
    let ranges: Vec<(usize, core::ops::RangeInclusive<u8>)> = (0..tokens.len())
        .filter_map(|i| strength_hcp(i).map(|r| (i, r)))
        .collect();
    let lo = ranges.iter().map(|(_, r)| *r.start()).max();
    let hi = ranges.iter().map(|(_, r)| *r.end()).min();
    let disjoint = matches!((lo, hi), (Some(lo), Some(hi)) if lo > hi);
    let any_known = ranges.iter().any(|(i, _)| !provs[*i].assumed);
    if !disjoint || !any_known {
        return atoms;
    }
    let dropped: Vec<usize> = ranges
        .iter()
        .filter(|(i, _)| provs[*i].assumed)
        .map(|(i, _)| *i)
        .collect();
    atoms
        .into_iter()
        .enumerate()
        .map(|(i, a)| {
            if dropped.contains(&i) {
                HandConstraint::Atom(Atom::ANY)
            } else {
                a
            }
        })
        .collect()
}

/// Folds a [`Clause`] tree into a [`HandConstraint`], skipping every fragment that contributes no
/// literal (a `Convention`/`Forcing`/`NoBound` token, an `Unrecognized` fragment, a fragment
/// hedged with a possibility word, or one Pass 2 left at `Atom::ANY`): `None` means "this
/// subtree is exactly `Atom::ANY`". `And` drops such children (`ANY` is `And`'s identity); `Or`
/// cannot: `Or(ANY, x, …)` is `ANY` itself (`x` would never need to hold), so one uninformative
/// branch collapses the whole group to `None` too.
fn build(
    clause: &Clause,
    fragments: &[Fragment],
    resolved: &[Option<(HandConstraint, Provenance)>],
) -> Option<HandConstraint> {
    match clause {
        Clause::Leaf(idx) => {
            let frag = &fragments[*idx];
            if frag.possibility {
                return None;
            }
            let (literal, _) = resolved[*idx].as_ref()?;
            if matches!(literal, HandConstraint::Atom(a) if *a == Atom::ANY) {
                return None;
            }
            let hc = literal.clone();
            Some(if frag.negated { hc.not() } else { hc })
        }
        Clause::And(items) => {
            let mut parts = Vec::with_capacity(items.len());
            for item in items {
                if let Some(hc) = build(item, fragments, resolved) {
                    parts.push(hc);
                }
            }
            fold(parts, HandConstraint::And)
        }
        Clause::Or(items) => {
            let mut parts = Vec::with_capacity(items.len());
            for item in items {
                match build(item, fragments, resolved) {
                    None => return None,
                    Some(hc) => parts.push(hc),
                }
            }
            fold(parts, HandConstraint::Or)
        }
    }
}

fn fold(
    mut parts: Vec<HandConstraint>,
    many: impl FnOnce(Vec<HandConstraint>) -> HandConstraint,
) -> Option<HandConstraint> {
    match parts.len() {
        0 => None,
        1 => Some(parts.pop().expect("len == 1")),
        _ => Some(many(parts)),
    }
}

/// `docs/design/06-system.md` §7.1 step 5: "簡約 (Atom が 8 個以下なら `to_dnf`)". Skipped for a
/// bigger tree (not worth the `to_dnf` cost here; the constraint is still correct, just less
/// flattened) or when the term cap truncates the expansion (the second return value flags that,
/// for `DnfTruncated`) — in both cases the original tree is kept as-is.
fn simplify(constraint: HandConstraint) -> (HandConstraint, bool) {
    if atom_count(&constraint) > 8 {
        return (constraint, false);
    }
    let Ok(dnf) = constraint.to_dnf(&DnfOptions::default()) else {
        return (constraint, false);
    };
    if dnf.truncated || !dnf.terms.iter().all(DnfTerm::is_exact) {
        return (constraint, dnf.truncated);
    }
    let simplified = match dnf.terms.len() {
        0 => HandConstraint::Or(Vec::new()),
        1 => HandConstraint::Atom(dnf.terms.into_iter().next().expect("len == 1").atom),
        _ => HandConstraint::Or(
            dnf.terms
                .into_iter()
                .map(|t| HandConstraint::Atom(t.atom))
                .collect(),
        ),
    };
    (simplified, false)
}

fn atom_count(constraint: &HandConstraint) -> usize {
    match constraint {
        HandConstraint::Atom(_) | HandConstraint::Custom(_) => 1,
        HandConstraint::Or(children) | HandConstraint::And(children) => {
            children.iter().map(atom_count).sum()
        }
        HandConstraint::Not(inner) => atom_count(inner),
    }
}

/// Derives [`NodeFlags`] from the same fragment list, independently of whether each token
/// produced a constraint literal. Not implemented here (left at their default/`None`): `TRF`'s
/// target-suit sequencing and `UNT`'s unbid-suit computation, which need aggregate our/their
/// suit sets `RowContext` does not carry; see `open_issues` in the compiler's final report.
fn derive_flags(
    fragments: &[Fragment],
    top: &Clause,
    normalized: &normalize::Normalized,
    ctx: &RowContext<'_>,
) -> NodeFlags {
    let mut artificial = normalized.alert;
    let mut sign_off = false;
    let mut has_support = false;

    for f in fragments {
        let FragmentKind::Token(token) = &f.kind else {
            continue;
        };
        // A negated or merely possible convention/support (`no STAY`, `may be a transfer`) is
        // not something this call is.
        if f.negated || f.possibility {
            continue;
        }
        match token {
            Token::Convention(name) => {
                artificial = true;
                if matches!(
                    name.as_str(),
                    "S/O" | "SIGN OFF" | "SIGN-OFF" | "T/P" | "TO PLAY"
                ) {
                    sign_off = true;
                }
            }
            Token::Splinter(..) => artificial = true,
            Token::Support(_) => has_support = true,
            _ => {}
        }
    }

    NodeFlags {
        artificial,
        forcing: clause_forcing(top, fragments).unwrap_or_default(),
        soft: fragments.iter().any(|f| f.hedged),
        transfer_to: None,
        agreed_suit: if has_support { ctx.agreed_suit } else { None },
        sign_off,
    }
}

/// The forcing status one fragment states: `F`/`F1`/`NF`/… directly, `GF`/`FG` as
/// `Forcing::ToGame` (§7.4), a negated forcing word (`not forcing`, `non-forcing`) as
/// `NonForcing`. A negated `GF` (`not GF`) says nothing about one-round forcing and a possibility
/// (`may be forcing`) nothing definite, so both state nothing (`None`).
fn fragment_forcing(f: &Fragment) -> Option<crate::Forcing> {
    let FragmentKind::Token(token) = &f.kind else {
        return None;
    };
    if f.possibility {
        return None;
    }
    let stated = match token {
        Token::Forcing(v) => *v,
        Token::Strength(StrengthWord::GameForcing) => crate::Forcing::ToGame,
        _ => return None,
    };
    if !f.negated {
        return Some(stated);
    }
    match (token, stated) {
        (Token::Forcing(_), crate::Forcing::OneRound | crate::Forcing::ToGame) => {
            Some(crate::Forcing::NonForcing)
        }
        _ => None,
    }
}

/// Forcing status over the clause tree: within an `And` the strongest stated value wins; across
/// the branches of an `Or` only what every branch agrees on holds (the same value, or
/// `OneRound` when every branch is forcing but to different degrees); otherwise nothing is
/// known (`None`, i.e. `Forcing::Unknown`). So `weak or GF` and `PRE … or FG …` are not
/// game-forcing.
fn clause_forcing(clause: &Clause, fragments: &[Fragment]) -> Option<crate::Forcing> {
    match clause {
        Clause::Leaf(idx) => fragment_forcing(&fragments[*idx]),
        Clause::And(items) => items
            .iter()
            .filter_map(|c| clause_forcing(c, fragments))
            .reduce(stronger_forcing),
        Clause::Or(items) => {
            let values: Vec<Option<crate::Forcing>> =
                items.iter().map(|c| clause_forcing(c, fragments)).collect();
            let first = (*values.first()?)?;
            if values.iter().all(|v| *v == Some(first)) {
                return Some(first);
            }
            let all_forcing = values
                .iter()
                .all(|v| matches!(v, Some(crate::Forcing::OneRound | crate::Forcing::ToGame)));
            all_forcing.then_some(crate::Forcing::OneRound)
        }
    }
}

fn stronger_forcing(a: crate::Forcing, b: crate::Forcing) -> crate::Forcing {
    fn rank(f: crate::Forcing) -> u8 {
        match f {
            crate::Forcing::ToGame => 3,
            crate::Forcing::OneRound => 2,
            crate::Forcing::Unknown => 1,
            crate::Forcing::NonForcing => 0,
        }
    }
    if rank(b) > rank(a) { b } else { a }
}

/// A compact, grep-able summary of every fragment, for the `DEBUG` tracing event (§7.8).
fn fragment_summary(fragments: &[Fragment]) -> String {
    fragments
        .iter()
        .map(fragment_tag)
        .collect::<Vec<_>>()
        .join(",")
}

fn fragment_tag(f: &Fragment) -> String {
    let mut tag = match &f.kind {
        FragmentKind::Unrecognized(s) => format!("unrec({s})"),
        FragmentKind::Token(t) => token_tag(t),
    };
    if f.negated {
        tag = format!("not({tag})");
    }
    if f.possibility {
        tag = format!("maybe({tag})");
    } else if f.hedged {
        tag = format!("hedge({tag})");
    }
    tag
}

fn token_tag(t: &Token) -> String {
    match t {
        Token::Hcp(r) => format!("hcp({r:?})"),
        Token::Points(r) => format!("pts({r:?})"),
        Token::SuitLen(s, r) => format!("len({s:?},{r:?})"),
        Token::Shape(s) => format!("shape({s})"),
        Token::Balanced => "bal".to_string(),
        Token::SemiBalanced => "semi-bal".to_string(),
        Token::Unbalanced => "unbal".to_string(),
        Token::Strength(w) => format!("str({w:?})"),
        Token::Forcing(v) => format!("forcing({v:?})"),
        Token::Convention(name) => format!("conv({name})"),
        Token::Quality(s, q) => format!("qual({s:?},{q:?})"),
        Token::HonourRun(s, h, n) => format!("honours({s:?},{h},{n})"),
        Token::Splinter(s, mini) => format!("spl({s:?},{mini})"),
        Token::Stopper(s) => format!("stop({s:?})"),
        Token::Shortness(s, n) => format!("short({s:?},{n})"),
        Token::Support(n) => format!("supp({n})"),
        Token::Controls(r) => format!("ctrl({r:?})"),
        Token::Losers(r) => format!("losers({r:?})"),
        Token::Natural => "nat".to_string(),
        Token::NoBound => "nobound".to_string(),
    }
}

/// The first three unrecognised spans' text, `|`-joined, for the `DEBUG` tracing event (§7.8).
fn unrecognized_summary(text: &str, spans: &[(u16, u16)]) -> String {
    spans
        .iter()
        .take(3)
        .map(|&(start, end)| &text[start as usize..end as usize])
        .collect::<Vec<_>>()
        .join("|")
}

#[cfg(test)]
mod tests {
    use bridge_core::{Bid, Call, Hand, Holding, Rank, Side as TableSide, Strain};

    use super::*;
    use crate::natural::Role;
    use crate::pattern::Binding;

    fn holding_of(cards: &str) -> Holding {
        let mut h = Holding::EMPTY;
        for c in cards.chars() {
            let rank = match c.to_ascii_uppercase() {
                'A' => Rank::Ace,
                'K' => Rank::King,
                'Q' => Rank::Queen,
                'J' => Rank::Jack,
                'T' => Rank::Ten,
                '9' => Rank::Nine,
                '8' => Rank::Eight,
                '7' => Rank::Seven,
                '6' => Rank::Six,
                '5' => Rank::Five,
                '4' => Rank::Four,
                '3' => Rank::Three,
                '2' => Rank::Two,
                other => panic!("bad rank char {other}"),
            };
            h = h.with(rank);
        }
        h
    }

    fn hand(clubs: &str, diamonds: &str, hearts: &str, spades: &str) -> Hand {
        let h = Hand::from_holdings(
            holding_of(clubs),
            holding_of(diamonds),
            holding_of(hearts),
            holding_of(spades),
        );
        assert_eq!(h.len(), 13, "test hand must hold 13 cards");
        h
    }

    fn ctx(binding: &Binding, call: Call, role: Role) -> RowContext<'_> {
        RowContext {
            call,
            side: TableSide::NS,
            level: match call {
                Call::Bid(bid) => bid.level(),
                _ => 0,
            },
            is_jump: false,
            binding,
            hash_suit: None,
            own_prev: None,
            partner_last: None,
            their_last_bid: None,
            agreed_suit: None,
            role,
            partner_hcp: None,
            own_hcp: None,
        }
    }

    #[test]
    fn empty_description_is_any_with_lint() {
        let binding = Binding::default();
        let c = ctx(&binding, Call::Pass, Role::Opener);
        let meta = SystemMeta::default();
        let compiled = compile_description("", &c, &meta);
        assert!(matches!(compiled.constraint, HandConstraint::Atom(a) if a == Atom::ANY));
        assert_eq!(compiled.recognition.ratio, 1.0);
        assert!(
            compiled
                .lints
                .iter()
                .any(|l| l.code == LintCode::EmptyDescription)
        );
    }

    #[test]
    fn hcp_and_balanced_conjunction() {
        let binding = Binding::default();
        let c = ctx(
            &binding,
            Call::Bid(Bid::new(1, Strain::NoTrump).unwrap()),
            Role::Opener,
        );
        let meta = SystemMeta::default();
        let compiled = compile_description("15-17 hcp, balanced", &c, &meta);

        // 4-3-3-3 shape, 15 hcp (7+2+1+5).
        let good = hand("AK92", "Q43", "J43", "KQ4");
        assert!(compiled.constraint.satisfies(good));

        // Same suit-quality mix but a 5-4-3-1 (unbalanced) shape.
        let unbalanced = hand("AK932", "Q432", "J43", "K");
        assert!(!compiled.constraint.satisfies(unbalanced));

        assert!(compiled.recognition.constraint_bearing);
        assert_eq!(compiled.priority, 0);
        assert!(compiled.branch_weights.is_none());
    }

    #[test]
    fn negation_excludes_the_shape() {
        let binding = Binding::default();
        let c = ctx(&binding, Call::Pass, Role::Opener);
        let meta = SystemMeta::default();
        let compiled = compile_description("not 4333", &c, &meta);

        // A literal digit shape reads S H D C, so "4333" is exactly spades=4, hearts=3,
        // diamonds=3, clubs=3 (one specific rotation of the 4333 class, not every rotation).
        let flat = hand("K92", "J43", "Q43", "AK92");
        assert!(!compiled.constraint.satisfies(flat));

        // 4-4-3-2 (spades=2), a different rotation: does not match "4333" at all.
        let other = hand("AK92", "Q876", "J54", "63");
        assert!(compiled.constraint.satisfies(other));
    }

    #[test]
    fn hedge_sets_soft_flag_without_loosening() {
        let binding = Binding::default();
        let c = ctx(&binding, Call::Pass, Role::Opener);
        let meta = SystemMeta::default();
        let compiled = compile_description("usually 15-17 hcp", &c, &meta);
        assert!(compiled.flags.soft);
        assert!(
            compiled
                .lints
                .iter()
                .any(|l| l.code == LintCode::SoftConstraint)
        );
        // Still exactly 15..=17, not widened.
        assert_eq!(compiled.constraint.hcp_range(), 15..=17);
    }

    #[test]
    fn enumeration_forms_or_group() {
        let binding = Binding::default();
        let c = ctx(&binding, Call::Pass, Role::Opener);
        let meta = SystemMeta::default();
        let compiled = compile_description("a) 5+!s\nb) 5+!h", &c, &meta);

        let spades = hand("432", "987", "62", "AKQ32");
        assert!(compiled.constraint.satisfies(spades));
        let hearts = hand("432", "987", "AKQ32", "62");
        assert!(compiled.constraint.satisfies(hearts));
        let neither = hand("AK92", "Q876", "J54", "63");
        assert!(!compiled.constraint.satisfies(neither));
    }

    #[test]
    fn bare_convention_carries_no_constraint_but_sets_flags() {
        let binding = Binding::default();
        let c = ctx(&binding, Call::Pass, Role::Opener);
        let meta = SystemMeta::default();
        let compiled = compile_description("STAY", &c, &meta);
        assert!(matches!(compiled.constraint, HandConstraint::Atom(a) if a == Atom::ANY));
        assert!(compiled.flags.artificial);
        assert!(!compiled.recognition.constraint_bearing);
    }

    #[test]
    fn splinter_convention_builds_a_compound_constraint() {
        // Unlike a bare named convention (above), `SPL` is a special case (§7.4/§7.5): it
        // compiles to shortness in the row's own suit AND support in the agreed suit AND a
        // game-forcing HCP range, not `Atom::ANY`. See `crates/bridge-system/tests/desc_
        // vocabulary.rs::splinter_row_builds_shortness_support_and_strength` for the full
        // satisfies()-based coverage; this unit test only checks that a constraint is produced
        // at all, from this module's own minimal fixtures.
        let binding = Binding::default();
        let call = Call::Bid(Bid::new(4, Strain::Clubs).unwrap());
        let mut c = ctx(&binding, call, Role::Opener);
        c.agreed_suit = Some(bridge_core::Suit::Hearts);
        let meta = SystemMeta::default();
        let compiled = compile_description("SPL", &c, &meta);
        assert!(
            !matches!(compiled.constraint, HandConstraint::Atom(a) if a == Atom::ANY),
            "SPL must not compile to the unconstrained atom"
        );
        assert!(compiled.flags.artificial);
    }

    #[test]
    fn priority_and_weights_extracted() {
        let binding = Binding::default();
        let c = ctx(&binding, Call::Pass, Role::Opener);
        let meta = SystemMeta::default();
        let compiled = compile_description("5+!c {w:0.6} or 4+!h {w:0.4} {prio:2}", &c, &meta);
        assert_eq!(compiled.priority, 2);
        assert_eq!(compiled.branch_weights, Some(vec![0.6, 0.4]));
    }

    // Regression for the real-file triage (roadmap 3.2-3.4, class a): jdh8's `3C = INV, 7+!c,
    // 4--7 HCP` states its own HCP range, but with an assumed opening partner (12..=21, since
    // there is no `partner_last` here) `INV`'s context-derived range is 22-12=10 .. 24-12=12 --
    // disjoint from the author's own 4-7, so ANDing both (as Pass 2 used to) made the whole row
    // `Unsatisfiable`. The explicit number must win: `INV`'s HCP atom is dropped, not intersected.
    #[test]
    fn explicit_hcp_wins_over_a_conflicting_context_strength_word() {
        let binding = Binding::default();
        let c = ctx(&binding, Call::Pass, Role::Responder);
        let meta = SystemMeta::default();
        let compiled = compile_description("INV, 7+!c, 4-7 hcp", &c, &meta);
        assert_eq!(compiled.constraint.hcp_range(), 4..=7);
        assert!(
            compiled.constraint.is_satisfiable(),
            "explicit range must win, not be ANDed with INV's contradicting context range"
        );

        // Sanity: without the explicit HCP fragment, INV's context range is exactly the
        // conflicting 10..=12 this test relies on, so the fix is actually exercised above.
        let inv_only = compile_description("INV", &c, &meta);
        assert_eq!(inv_only.constraint.hcp_range(), 10..=12);
    }

    // Regression for gjp/common/1m-2m.bml:9 and its siblings (real_lint_triage.md): `NAT` never
    // implemented the "衝突は明示が勝つ" rule for its own suit length, so `1H-1S / 2S = NAT, 4=!s`
    // ANDed NAT's assumed 4+ minimum (`Role::Opener` at level 2 defaults to 5, well past the
    // stated 4) against the row's own exact `4=!s`, making the row unsatisfiable no matter which
    // number the author actually wrote.
    #[test]
    fn explicit_suit_length_wins_over_nats_own_assumed_minimum() {
        let binding = Binding::default();
        let c = ctx(
            &binding,
            Call::Bid(Bid::new(2, Strain::Spades).unwrap()),
            Role::Opener,
        );
        let meta = SystemMeta::default();
        let compiled = compile_description("NAT, 4=!s", &c, &meta);
        assert_eq!(
            compiled.constraint.suit_len(bridge_core::Suit::Spades),
            4..=4
        );
        assert!(
            compiled.constraint.is_satisfiable(),
            "the row's own exact spade length must win, not be ANDed with NAT's assumed minimum"
        );

        // Sanity: bare `NAT` here really does default to a longer minimum than 4, so the fix is
        // actually exercised above.
        let nat_only = compile_description("NAT", &c, &meta);
        assert!(
            *nat_only
                .constraint
                .suit_len(bridge_core::Suit::Spades)
                .start()
                > 4
        );
    }

    // Recheck regression: the explicit-wins rules used to scan the flat token list, so a negated,
    // possibility-hedged or other-branch number switched a strength word's HCP atom off and left
    // the row far wider than written (`GF, not 20+ HCP` came out 0..=19, `GF, may have 11 HCP`
    // and `weak or 16+ HCP` unconstrained).
    #[test]
    fn strength_word_keeps_its_range_unless_a_number_is_stated_alongside_it() {
        let binding = Binding::default();
        let c = ctx(&binding, Call::Pass, Role::Responder);
        let meta = SystemMeta::default();
        let negated = compile_description("GF, not 20+ hcp", &c, &meta);
        assert_eq!(negated.constraint.hcp_range(), 13..=19);
        let possibility = compile_description("GF, may have 11 hcp", &c, &meta);
        assert_eq!(possibility.constraint.hcp_range(), 13..=37);
        let other_branch = compile_description("weak or 16+ hcp", &c, &meta);
        assert!(
            !matches!(&other_branch.constraint, HandConstraint::Atom(a) if *a == Atom::ANY),
            "`weak` must keep its range in its own Or branch"
        );
        // 12 hcp is neither weak nor 16+.
        let middle = hand("A32", "K32", "Q32", "KJ32");
        assert!(!other_branch.constraint.satisfies(middle));
        // Sanity: a number stated alongside the word still wins.
        let stated = compile_description("GF, 10+ hcp", &c, &meta);
        assert_eq!(stated.constraint.hcp_range(), 10..=37);
    }

    // Recheck regression: `NAT`'s own-suit length used to be dropped by any length token for the
    // suit anywhere in the description, including a negated one, a possibility and one in a
    // single `Or` branch, so a natural 2C could hold 0-3 clubs.
    #[test]
    fn nat_keeps_its_length_unless_a_length_is_stated_alongside_it() {
        let binding = Binding::default();
        let meta = SystemMeta::default();
        let two_c = ctx(
            &binding,
            Call::Bid(Bid::new(2, Strain::Clubs).unwrap()),
            Role::Responder,
        );
        let clubs = bridge_core::Suit::Clubs;
        let bare = compile_description("NAT", &two_c, &meta)
            .constraint
            .suit_len(clubs);
        assert!(*bare.start() >= 4);
        for text in ["NAT, may be 3!c", "NAT, maybe 4!c"] {
            let compiled = compile_description(text, &two_c, &meta);
            assert_eq!(compiled.constraint.suit_len(clubs), bare, "{text}");
        }
        let negated = compile_description("NAT, not 4!c", &two_c, &meta);
        assert!(*negated.constraint.suit_len(clubs).start() >= 5);

        let two_d = ctx(
            &binding,
            Call::Bid(Bid::new(2, Strain::Diamonds).unwrap()),
            Role::Responder,
        );
        let or_branch = compile_description("NAT, 6+!d or 4!s", &two_d, &meta);
        let void_in_diamonds = hand("AK32", "", "QJ432", "K432");
        assert!(!or_branch.constraint.satisfies(void_in_diamonds));
        // Every branch pinning the suit still counts as stating its length.
        let all_branches = compile_description("NAT, 3!d or 4!d", &two_d, &meta);
        assert_eq!(
            all_branches
                .constraint
                .suit_len(bridge_core::Suit::Diamonds),
            3..=4
        );
    }

    // Recheck regression for gjp/common/1C.bml:54 `2M = FG, NAT (maybe 3 cards only)`: the
    // possibility used to drop NAT's length, leaving the row with no length in the bid major.
    #[test]
    fn gjp_nat_with_a_possible_short_suit_keeps_a_length() {
        let binding = Binding::default();
        let meta = SystemMeta::default();
        let two_s = ctx(
            &binding,
            Call::Bid(Bid::new(2, Strain::Spades).unwrap()),
            Role::Responder,
        );
        let compiled = compile_description("FG, NAT (maybe 3 cards only)", &two_s, &meta);
        assert!(
            *compiled
                .constraint
                .suit_len(bridge_core::Suit::Spades)
                .start()
                >= 4
        );
    }

    // gjp/common/2C.bml:59/66-69: opener's tracked range is the hull (5..=37) of the `2C` row's
    // `weak-two / 25+ NT / FG` branches, so `MAX` of it lies far above a weak two. Once the
    // explicit-wins rules stopped wiping that row's branches out, `weak-two, MAX` became
    // unsatisfiable; `MAX` must mean the top of the weak two it is conjoined with.
    #[test]
    fn max_is_rebased_on_a_conjoined_category_word() {
        let binding = Binding::default();
        let meta = SystemMeta::default();
        let mut c = ctx(
            &binding,
            Call::Bid(Bid::new(2, Strain::Spades).unwrap()),
            Role::Opener,
        );
        c.own_hcp = Some(5..=37);
        let weak = compile_description("weak", &c, &meta)
            .constraint
            .hcp_range();
        let max = compile_description("weak, MAX", &c, &meta);
        assert!(max.constraint.is_satisfiable());
        let r = max.constraint.hcp_range();
        assert_eq!(*r.end(), *weak.end());
        assert!(*r.start() > *weak.start());
        // Without a category word, `MAX` still halves the tracked range.
        let bare = compile_description("MAX", &c, &meta).constraint.hcp_range();
        assert_eq!(bare, 22..=37);
    }

    #[test]
    fn game_forcing_resolves_through_context() {
        let binding = Binding::default();
        let c = ctx(&binding, Call::Pass, Role::Responder);
        let meta = SystemMeta::default();
        let compiled = compile_description("GF", &c, &meta);
        // gf_total (25) - assumed opening partner (12) = 13.
        assert_eq!(compiled.constraint.hcp_range(), 13..=37);
        assert!(
            compiled
                .lints
                .iter()
                .any(|l| l.code == LintCode::AssumedContext)
        );
    }
}
