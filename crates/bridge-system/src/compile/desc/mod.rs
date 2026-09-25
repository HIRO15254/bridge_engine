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
    let (atoms, provs) = context::resolve(&pass1_tokens, ctx, meta);
    // Pass 2 can resolve a context-dependent strength word (`INV`, `MIN`, `S/T`, …) to an HCP
    // range that contradicts a number the author wrote out explicitly in the same description
    // (`docs/design/06-system.md` §7.5's "衝突は明示が勝つ" rule, generalized from `NAT` to every
    // `Strength` word): e.g. jdh8's `3C = INV, 7+!c, 4--7 HCP` states its own 4--7 HCP range, so
    // `INV`'s context-derived range must not be ANDed against it (that would make the row
    // unsatisfiable whenever the two disagree). An explicit `Hcp`/`Points` fragment anywhere in
    // the description means every `Strength` word's HCP contribution is dropped (treated as
    // `Atom::ANY`, which `build` already elides): the explicit number is what the author meant to
    // constrain the hand by, and the strength word's own atom would only ever narrow or
    // contradict it. This does not touch a `Strength` word's other effects (`NodeFlags` from
    // `derive_flags`, e.g. `GF` still implies `Forcing::ToGame`), only the HCP atom this pass
    // would otherwise have produced from it.
    let has_explicit_number = pass1_tokens
        .iter()
        .any(|t| matches!(t, Token::Hcp(_) | Token::Points(_)));
    let atoms: Vec<Atom> = if has_explicit_number {
        pass1_tokens
            .iter()
            .zip(atoms)
            .map(|(tok, atom)| {
                if matches!(tok, Token::Strength(_)) {
                    Atom::ANY
                } else {
                    atom
                }
            })
            .collect()
    } else {
        atoms
    };

    // `NAT` itself is the origin of the "衝突は明示が勝つ" rule generalized above, but never
    // implemented it for its own suit-length atom: `resolve_natural` always ANDs
    // `suit_len[call's suit] >= natural_suit_length` (5 by default), even when the row states its
    // own length for that suit explicitly (e.g. gjp's `NAT, normally 4!s`, or an `at least 4!c`
    // read as an exact 4). An explicit `SuitLen`/`Shape` fragment that pins the length of NAT's
    // own suit means the author already said what that length is; `NAT`'s assumed minimum must
    // not additionally AND against it (unsatisfiable whenever the two disagree, e.g. a 5+ default
    // against a stated 4).
    let nat_suit_pinned_explicitly =
        own_suit(ctx).is_some_and(|suit| pass1_tokens.iter().any(|t| suit_len_pins(t, suit, ctx)));
    let atoms: Vec<Atom> = if nat_suit_pinned_explicitly {
        pass1_tokens
            .iter()
            .zip(atoms)
            .map(|(tok, atom)| {
                if matches!(tok, Token::Natural) {
                    Atom::ANY
                } else {
                    atom
                }
            })
            .collect()
    } else {
        atoms
    };

    let mut resolved: Vec<Option<(Atom, Provenance)>> = vec![None; fragments.len()];
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

    let flags = derive_flags(&fragments, &normalized, ctx);

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

/// Folds a [`Clause`] tree into a [`HandConstraint`], skipping every fragment that contributes no
/// literal (a `Convention`/`Forcing`/`NoBound` token, an `Unrecognized` fragment, or one Pass 2
/// left at `Atom::ANY`): `None` means "this subtree is exactly `Atom::ANY`". `And` drops such
/// children (`ANY` is `And`'s identity); `Or` cannot: `Or(ANY, x, …)` is `ANY` itself (`x` would
/// never need to hold), so one uninformative branch collapses the whole group to `None` too.
fn build(
    clause: &Clause,
    fragments: &[Fragment],
    resolved: &[Option<(Atom, Provenance)>],
) -> Option<HandConstraint> {
    match clause {
        Clause::Leaf(idx) => {
            let frag = &fragments[*idx];
            let (atom, _) = resolved[*idx].as_ref()?;
            if *atom == Atom::ANY {
                return None;
            }
            let hc = HandConstraint::Atom(atom.clone());
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
    normalized: &normalize::Normalized,
    ctx: &RowContext<'_>,
) -> NodeFlags {
    let mut artificial = normalized.alert;
    let mut sign_off = false;
    let mut has_support = false;
    let mut game_forcing = false;
    let mut forcing: Option<crate::Forcing> = None;

    for f in fragments {
        let FragmentKind::Token(token) = &f.kind else {
            continue;
        };
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
            Token::Forcing(v) => {
                forcing = Some(forcing.map_or(*v, |existing| stronger_forcing(existing, *v)));
            }
            Token::Strength(StrengthWord::GameForcing) => game_forcing = true,
            Token::Support(_) => has_support = true,
            _ => {}
        }
    }

    // "`GF`, `FG`, … | `Strength(GameForcing)` + `Forcing(ToGame)`" (§7.4): being game-forcing
    // implies forcing to game even without a separate `F`/`FG` word.
    if game_forcing {
        forcing = Some(forcing.map_or(crate::Forcing::ToGame, |existing| {
            stronger_forcing(existing, crate::Forcing::ToGame)
        }));
    }

    NodeFlags {
        artificial,
        forcing: forcing.unwrap_or_default(),
        soft: fragments.iter().any(|f| f.hedged),
        transfer_to: None,
        agreed_suit: if has_support { ctx.agreed_suit } else { None },
        sign_off,
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
    if f.hedged {
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
