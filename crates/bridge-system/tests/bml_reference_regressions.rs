//! Regressions for the code/doc mismatches found while reviewing the extended-BML reference
//! (`docs/design/16-extended-bml.md`). Each test pins the behaviour the reference now
//! documents.

use bridge_constraint::HandConstraint;
use bridge_eval::LtcMethod;
use bridge_system::lexer::MemLoader;
use bridge_system::{Alertability, CompileOptions, LintCode, Node, Severity, SystemIR};

fn compile(source: &str) -> SystemIR {
    let opts = CompileOptions {
        coverage_samples: 0,
        ..CompileOptions::default()
    };
    let (ir, _) = bridge_system::compile("inline.bml", source, &MemLoader::default(), &opts);
    ir
}

/// The node whose full auction (passes included) renders as `auction` (`"1C-Pass-1D"`).
fn node_at<'a>(ir: &'a SystemIR, auction: &str) -> Option<&'a Node> {
    ir.nodes.iter().find(|n| {
        n.calls
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join("-")
            == auction
    })
}

fn lints_of(ir: &SystemIR, code: LintCode) -> Vec<&bridge_system::Lint> {
    ir.lints.iter().filter(|l| l.code == code).collect()
}

fn no_errors(ir: &SystemIR) {
    let errors: Vec<_> = ir
        .lints
        .iter()
        .filter(|l| l.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

/// The loser range of the node's single evaluation requirement, in half-losers.
fn loser_halves(node: &Node) -> core::ops::RangeInclusive<u8> {
    let HandConstraint::Atom(atom) = &node.constraint else {
        panic!("not an atom: {:?}", node.constraint);
    };
    let req = atom
        .eval
        .iter()
        .find(|r| r.metric == bridge_constraint::Metric::Losers(LtcMethod::Classic))
        .unwrap_or_else(|| panic!("no loser requirement: {atom:?}"));
    req.range.clone()
}

#[test]
fn loser_counts_are_whole_losers() {
    let ir = compile("1C = LTC 7\n\n1D = 6-7 losers\n\n1H = 5+ losers\n\n1S = LTC\n");
    no_errors(&ir);
    // `Metric::Losers` counts half-losers: 7 losers is 14 halves.
    assert_eq!(loser_halves(node_at(&ir, "1C").unwrap()), 14..=14);
    assert_eq!(loser_halves(node_at(&ir, "1D").unwrap()), 12..=14);
    assert_eq!(loser_halves(node_at(&ir, "1H").unwrap()), 10..=24);
    // A bare `LTC` is recognised but constrains nothing.
    let HandConstraint::Atom(atom) = &node_at(&ir, "1S").unwrap().constraint else {
        panic!("not an atom");
    };
    assert!(atom.eval.is_empty(), "{atom:?}");
}

#[test]
fn the_alert_marker_makes_the_call_artificial() {
    let ir = compile("1C = 12+ hcp\n\n1C-\n1D = !Foo, 6+ hcp\n1H = {prio:5} !Foo, 6+ hcp\n");
    for auction in ["1C-Pass-1D", "1C-Pass-1H"] {
        let node = node_at(&ir, auction).unwrap();
        assert_eq!(node.alertable, Alertability::Alertable, "{auction}");
        assert!(node.flags.artificial, "{auction}");
        assert!(!node.description.contains('!'), "{:?}", node.description);
    }
    assert_eq!(node_at(&ir, "1C-Pass-1H").unwrap().priority, 5);
    // A suit symbol at the start is not an alert.
    let ir = compile("1C = !c 5+ cards\n");
    let node = node_at(&ir, "1C").unwrap();
    assert_eq!(node.alertable, Alertability::Unspecified);
    assert!(!node.flags.artificial);
}

#[test]
fn a_step_past_seven_notrump_is_reported() {
    let ir = compile("7S = 30+ hcp\n\n7S-(P)-\n2steps = 5+ hcp\n");
    let lints = lints_of(&ir, LintCode::StepWithoutAnchor);
    assert_eq!(lints.len(), 1, "{:?}", ir.lints);
    assert_eq!(lints[0].severity, Severity::Error);
    assert!(lints[0].message.contains("7NT"), "{}", lints[0].message);
}

#[test]
fn an_n_level_with_no_sufficient_level_is_reported() {
    let ir = compile("7N = 30+ hcp\n\n7N-(P)-\nnS = 5+!s\n");
    assert_eq!(
        lints_of(&ir, LintCode::NoSufficientLevel).len(),
        1,
        "{:?}",
        ir.lints
    );
}

#[test]
fn a_repeated_row_in_one_sibling_list_reports_its_dropped_subtree() {
    let ir = compile("1C = 12+ hcp\n  1D = 0--5 hcp\n1C = 12+ hcp\n  1H = 6+ hcp\n");
    assert!(node_at(&ir, "1C-Pass-1D").is_some());
    assert!(node_at(&ir, "1C-Pass-1H").is_none());
    let lints = lints_of(&ir, LintCode::DuplicatePath);
    assert_eq!(lints.len(), 1, "{:?}", ir.lints);
    assert_eq!(lints[0].severity, Severity::Warning);
    // Across tables the subtrees merge, silently.
    let ir = compile("1C = 12+ hcp\n  1D = 0--5 hcp\n\n1C = 12+ hcp\n  1H = 6+ hcp\n");
    assert!(node_at(&ir, "1C-Pass-1D").is_some());
    assert!(node_at(&ir, "1C-Pass-1H").is_some());
    assert!(
        lints_of(&ir, LintCode::DuplicatePath).is_empty(),
        "{:?}",
        ir.lints
    );
}

#[test]
fn two_cut_blocks_in_one_paragraph_define_two_clipboards() {
    let ir = compile(
        "#CUT a\n1C = 12+ hcp, 5+!c\n#ENDCUT\n#CUT b\n1D = 12+ hcp, 5+!d\n#ENDCUT\n\n#PASTE a\n\n#PASTE b\n",
    );
    assert!(
        ir.lints.iter().all(|l| !matches!(
            l.code,
            LintCode::PasteUnknownName | LintCode::UnknownDirective
        )),
        "{:?}",
        ir.lints
    );
    assert!(node_at(&ir, "1C").is_some());
    assert!(node_at(&ir, "1D").is_some());
}

#[test]
fn malformed_paste_arguments_are_reported_and_ignored() {
    let ir = compile("#CUT a\n1C = 12+ hcp\n#ENDCUT\n\n#PASTE a =x oops\n");
    assert_eq!(
        lints_of(&ir, LintCode::UnknownDirective).len(),
        2,
        "{:?}",
        ir.lints
    );
    // The empty target did not insert `x` between every character.
    let node = node_at(&ir, "1C").unwrap();
    assert_eq!(node.description, "12+ hcp");
}

#[test]
fn a_hide_or_bidtable_in_a_paragraph_of_its_own_is_reported() {
    for directive in ["#HIDE", "#BIDTABLE"] {
        let ir = compile(&format!("{directive}\n\n1C = 12+ hcp\n"));
        let lints = lints_of(&ir, LintCode::UnknownDirective);
        assert_eq!(lints.len(), 1, "{directive}: {:?}", ir.lints);
        assert!(lints[0].message.contains(directive), "{}", lints[0].message);
    }
}

#[test]
fn an_unparsable_history_row_drops_the_whole_table() {
    let ir = compile("1N = x\n\n#HIDE\n1N--2C-\n2H = w\n");
    assert_eq!(
        lints_of(&ir, LintCode::UnknownCallToken).len(),
        1,
        "{:?}",
        ir.lints
    );
    // The rows under it are not re-rooted as openings.
    assert!(node_at(&ir, "2H").is_none());
}

#[test]
fn a_typo_in_a_tables_first_row_is_reported() {
    for source in [
        "1N-2Q- = y\n2D = z\n",
        "1N--2C- = y\n2D = z\n",
        "1Nx = q\n1D = r\n",
    ] {
        let ir = compile(source);
        assert_eq!(
            lints_of(&ir, LintCode::UnknownCallToken).len(),
            1,
            "{source:?}: {:?}",
            ir.lints
        );
    }
    // Prose written with suit symbols, or that merely starts with a number, is left alone.
    for source in [
        "1!d-(2!c)-3!d is preemptive.\n",
        "2-suited hands are shown by 2N.\n",
    ] {
        let ir = compile(source);
        assert!(ir.lints.is_empty(), "{source:?}: {:?}", ir.lints);
    }
}

#[test]
fn fs_loader_reads_a_backslash_include_path() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("bml_reference_fs_include");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/a.bml"), "1C = 12+ hcp\n").unwrap();
    let root = dir.join("root.bml");
    let text = "#INCLUDE sub\\a.bml\n";
    std::fs::write(&root, text).unwrap();
    let (ir, _) = bridge_system::compile(
        &root.to_string_lossy(),
        text,
        &bridge_system::lexer::FsLoader,
        &CompileOptions::default(),
    );
    assert!(
        lints_of(&ir, LintCode::IncludeNotFound).is_empty(),
        "{:?}",
        ir.lints
    );
    assert!(node_at(&ir, "1C").is_some());
}
