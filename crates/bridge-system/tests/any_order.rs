//! `#ANYORDER` (`docs/design/06-system.md` §4.7): a table whose fresh `X`/`Y`/`Z` bindings
//! ignore BML's `X < Y < Z` strain order.

use std::collections::BTreeSet;

use bridge_system::lexer::MemLoader;
use bridge_system::{CompileOptions, LintCode, Severity, SystemIR};

fn compile(source: &str) -> SystemIR {
    let opts = CompileOptions {
        coverage_samples: 0,
        ..CompileOptions::default()
    };
    let (ir, _) = bridge_system::compile("inline.bml", source, &MemLoader::default(), &opts);
    ir
}

/// Every node's call sequence (opening first, implicit passes as `Pass`), with its description.
fn positions(ir: &SystemIR) -> BTreeSet<String> {
    ir.nodes
        .iter()
        .filter(|n| !n.is_synthesised())
        .map(|n| {
            let calls: Vec<String> = n.calls.iter().map(|c| format!("{c}")).collect();
            format!("{} = {}", calls.join(" "), n.description)
        })
        .collect()
}

fn has_position(ir: &SystemIR, calls: &str) -> bool {
    positions(ir)
        .iter()
        .any(|p| p.split(" = ").next() == Some(calls))
}

fn count(ir: &SystemIR, code: LintCode) -> usize {
    ir.lints.iter().filter(|l| l.code == code).count()
}

fn no_errors(ir: &SystemIR) {
    assert!(
        !ir.lints.iter().any(|l| l.severity == Severity::Error),
        "{:?}",
        ir.lints
    );
}

#[test]
fn without_the_directive_y_stays_above_x() {
    let ir = compile(
        "1X-(2Y)-
D = 10+ hcp, 0--2Y
",
    );
    no_errors(&ir);
    // Only (2S) over 1H: Y must be above X.
    assert!(has_position(&ir, "1H 2S X"));
    assert!(!has_position(&ir, "1H 2C X"));
    assert!(!has_position(&ir, "1H 2D X"));
}

#[test]
fn any_order_lets_y_take_a_strain_below_x() {
    let ir = compile(
        "#ANYORDER
1X-(2Y)-
D = 10+ hcp, 0--2Y
",
    );
    no_errors(&ir);
    for pos in ["1H 2C X", "1H 2D X", "1H 2S X", "1S 2H X", "1D 2C X", "1C 2D X"] {
        assert!(has_position(&ir, pos), "{pos} missing");
    }
    // Distinct and unused: Y never repeats X's strain.
    assert!(!has_position(&ir, "1H 2H X"));
    // The description is substituted per binding, whatever the order.
    let p = positions(&ir);
    assert!(p.contains("1H 2C X = 10+ hcp, 0--2!c"), "{p:?}");
    assert!(p.contains("1S 2H X = 10+ hcp, 0--2!h"), "{p:?}");
    assert_eq!(count(&ir, LintCode::AnyOrderWithoutVariables), 0);
}

#[test]
fn three_variables_stay_distinct_in_any_order() {
    let ir = compile(
        "#ANYORDER
1X-(1Y)-2Z-
3Z = 6+Z
",
    );
    no_errors(&ir);
    // Z below both X and Y (ordered BML would need Z > Y > X).
    assert!(has_position(&ir, "1D 1S 2C Pass 3C"));
    assert!(has_position(&ir, "1H 1S 2D Pass 3D"));
    // Z never takes X's or Y's strain (both were bid, so they are used).
    for p in positions(&ir) {
        let calls: Vec<&str> = p.split(" = ").next().unwrap().split(' ').collect();
        if calls.len() < 3 {
            continue;
        }
        let strain = |c: &str| c.chars().nth(1);
        assert_ne!(strain(calls[2]), strain(calls[0]), "{p}");
        assert_ne!(strain(calls[2]), strain(calls[1]), "{p}");
    }
}

#[test]
fn the_directive_is_table_scoped() {
    // The first table has #ANYORDER (written under a row: it still covers the whole table); the
    // second, in the same file, keeps the order.
    let ir = compile(
        "1X-(2Y)-
D = 10+ hcp
  #ANYORDER

1X-(3Y)-
D = 12+ hcp
",
    );
    no_errors(&ir);
    assert!(has_position(&ir, "1H 2C X"));
    assert!(has_position(&ir, "1H 3S X"));
    assert!(!has_position(&ir, "1H 3C X"));
}

#[test]
fn a_pasted_directive_applies_to_the_target_table() {
    let ir = compile(
        "#CUT free
#ANYORDER
D = 10+ hcp
#ENDCUT

1X-(2Y)-
#PASTE free
",
    );
    no_errors(&ir);
    assert!(has_position(&ir, "1S 2C X"));
}

#[test]
fn majors_and_minors_are_unaffected() {
    // M and m never had an order; #ANYORDER changes nothing for them (and warns, since the table
    // has fewer than two of X, Y, Z).
    let plain = compile(
        "1m-(1M)-
D = 8+ hcp, 4+oM
",
    );
    let free = compile(
        "#ANYORDER
1m-(1M)-
D = 8+ hcp, 4+oM
",
    );
    assert_eq!(positions(&plain), positions(&free));
    assert_eq!(count(&free, LintCode::AnyOrderWithoutVariables), 1);
    assert_eq!(count(&plain, LintCode::AnyOrderWithoutVariables), 0);
}

#[test]
fn a_single_variable_gets_the_no_effect_lint() {
    let ir = compile(
        "#ANYORDER
1X-(P)-
2X = 6--9 hcp, 3+X
",
    );
    let lint = ir
        .lints
        .iter()
        .find(|l| l.code == LintCode::AnyOrderWithoutVariables)
        .expect("lint");
    assert_eq!(lint.severity, Severity::Info);
    // The table still compiles as usual.
    assert!(has_position(&ir, "1H Pass 2H"));
}

#[test]
fn ordered_expansions_are_a_subset_of_any_order_ones() {
    let src = "1X-(2Y)-P-(3Y)-
D = 13+ hcp, 0--1Y
4X = 15+ hcp, 6+X
";
    let ordered = positions(&compile(src));
    let free = positions(&compile(&format!("#ANYORDER\n{src}")));
    assert!(ordered.is_subset(&free));
    assert!(free.len() > ordered.len());
}
