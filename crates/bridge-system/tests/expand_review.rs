//! Regression tests for the expansion-stage findings of the phase-3 integration review
//! (class wildcard steps, pattern-vs-pattern sibling overlap, seat-specific history placeholders,
//! and the `max_nodes` guard at a table boundary), driven through the public `compile` API.

mod common;

use bridge_core::{Seat, Vulnerability};
use bridge_system::{
    CompileOptions, Lint, LintCode, Severity, SystemIR, compile, lexer::MemLoader,
};
use common::auction;

fn compile_str(text: &str, opts: &CompileOptions) -> (SystemIR, Vec<Lint>) {
    compile("t.bml", text, &MemLoader::default(), opts)
}

fn compile_default(text: &str) -> (SystemIR, Vec<Lint>) {
    compile_str(text, &CompileOptions::default())
}

/// `(matched_depth, description of the node for the last matched call)` for `calls`, with
/// North the dealer and the owner (N/S).
fn lookup(ir: &SystemIR, calls: &str) -> (usize, Option<String>) {
    let a = auction(Seat::North, Vulnerability::None, calls);
    let l = ir.resolve(&a, Seat::North).expect("non-empty auction");
    let desc = l
        .matched_depth
        .checked_sub(1)
        .and_then(|i| l.by_depth[i].map(|id| ir.node(id).description.clone()));
    (l.matched_depth, desc)
}

fn codes(lints: &[Lint], code: LintCode) -> Vec<&Lint> {
    lints.iter().filter(|l| l.code == code).collect()
}

// Review #6 (b)/(c): a class wildcard never collides with an exact `(P)` sibling or with a
// second class sibling, and each class edge keeps its own subtree.
#[test]
fn class_siblings_do_not_collide_with_pass_or_each_other() {
    let (ir, lints) = compile_default(
        "1C-\n\
         (P)     no interference\n\
         \x20 1H  4+!h\n\
         (suit)  suit overcall\n\
         \x20 2N  natural\n\
         (any)   anything else\n\
         \x20 2C  clubs again\n",
    );
    assert!(
        codes(&lints, LintCode::ShadowedByExact).is_empty(),
        "{lints:?}"
    );
    assert!(
        lints.iter().all(|l| l.severity != Severity::Error),
        "{lints:?}"
    );
    assert_eq!(lookup(&ir, "1C P 1H"), (3, Some("4+!h".to_string())));
    assert_eq!(lookup(&ir, "1C 1S 2NT"), (3, Some("natural".to_string())));
    // `(X)` is no suit bid: `(suit)` does not match it, `(any)` does, and its subtree exists.
    assert_eq!(lookup(&ir, "1C X 2C"), (3, Some("clubs again".to_string())));
}

// Review #6 (a): a Double below a wildcard doubles the opponents' unknown call, not partner's
// bid, so it is legal; so is a reopening Double after `(suit) P (P)`.
#[test]
fn double_below_a_wildcard_is_not_illegal() {
    let (ir, lints) = compile_default(
        "1C-\n\
         (suit)  suit overcall\n\
         \x20 D   negative\n\
         \x20 P   nothing to say\n\
         \x20   (P)\n\
         \x20     D  reopening double\n",
    );
    assert!(codes(&lints, LintCode::IllegalCall).is_empty(), "{lints:?}");
    assert_eq!(lookup(&ir, "1C 1S X"), (3, Some("negative".to_string())));
    assert_eq!(
        lookup(&ir, "1C 1S P P X"),
        (5, Some("reopening double".to_string()))
    );
}

// Review #6 (a): a Redouble below `(any)` (which may be a Double) is not rejected either, while
// a Double of partner's own bid right after a `(P)`-class-free exact pass still is.
#[test]
fn legality_below_a_wildcard_is_relaxed_but_not_disabled() {
    let (_, lints) = compile_default(
        "1C-\n\
         (any)   anything\n\
         \x20 R   business\n",
    );
    assert!(codes(&lints, LintCode::IllegalCall).is_empty(), "{lints:?}");
    // Below `(suit)`, a bid that is not higher than the last *known* bid (1C) is still illegal.
    let (_, lints) = compile_default(
        "1D-\n\
         (suit)  suit overcall\n\
         \x20 1C  impossible\n",
    );
    assert_eq!(codes(&lints, LintCode::IllegalCall).len(), 1, "{lints:?}");
}

// Review #9: a later pattern sibling whose candidate an earlier pattern sibling already produced
// is skipped silently (bss.py's `bids_processed`), never reported as a DuplicatePath Warning.
#[test]
fn pattern_catch_all_after_a_pattern_row_is_silently_skipped() {
    let (ir, lints) = compile_default(
        "1C-\n\
         1M  4+M, 6+ hcp\n\
         1X  5+X, 6+ hcp\n",
    );
    assert!(
        codes(&lints, LintCode::DuplicatePath).is_empty(),
        "{lints:?}"
    );
    assert_eq!(lookup(&ir, "1C P 1H").1.as_deref(), Some("4+!h, 6+ hcp"));
    assert_eq!(lookup(&ir, "1C P 1S").1.as_deref(), Some("4+!s, 6+ hcp"));
    assert_eq!(lookup(&ir, "1C P 1D").1.as_deref(), Some("5+!d, 6+ hcp"));
}

// Review #10: a seat-specific table whose history retraces `1H` with no description, placed
// before the general `1H` definition, must not shadow it in 3rd/4th seat.
#[test]
#[ignore = "review #10: fixed in the next commit"]
fn seat_specific_placeholder_before_the_general_row_does_not_shadow_it() {
    let (ir, lints) =
        compile_default("#SEAT 34\n\n1H-\n2C  drury\n\n#SEAT 0\n\n1H  5+!h, 12+ hcp\n");
    assert!(
        lints.iter().all(|l| l.severity != Severity::Error),
        "{lints:?}"
    );
    assert_eq!(
        lookup(&ir, "P P 1H"),
        (1, Some("5+!h, 12+ hcp".to_string()))
    );
    assert_eq!(lookup(&ir, "1H"), (1, Some("5+!h, 12+ hcp".to_string())));
    assert_eq!(lookup(&ir, "P P 1H P 2C"), (3, Some("drury".to_string())));
}

// Review #11: hitting `max_nodes` exactly on a table's last candidate still reports
// TooManyNodes when later tables are dropped.
#[test]
fn max_nodes_at_a_table_boundary_reports_too_many_nodes() {
    let opts = CompileOptions {
        max_nodes: 2,
        ..CompileOptions::default()
    };
    let (ir, lints) = compile_str("1C 3+!c\n  1H 4+!h\n\n1D 3+!d\n", &opts);
    assert_eq!(ir.nodes.len(), 2);
    assert_eq!(codes(&lints, LintCode::TooManyNodes).len(), 1, "{lints:?}");
}
