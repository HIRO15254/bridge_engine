//! Compiles every vendored real-world BML file and requires its `Error`-severity lints to equal
//! `tests/data/real_expected_errors.txt` exactly (roadmap 3.2/3.4's "Error lint 0 (or an exact
//! expected set)"). Vendored data is git-ignored; skipped when absent.
//!
//! `tests/data/real_lint_triage.md` classifies every one of these Errors (file:line, row, lint,
//! cause, class): each entry here is either a genuine contradiction/illegal sequence in the
//! source file itself (class b) or a recorded, not-yet-fixed compiler/vocabulary gap (class c).
//! Every fixable compiler bug found in this triage (class a) was fixed directly, with its own
//! regression test, instead of being recorded here -- see `compile/desc/{clause,context,tokens}.rs`
//! and `compile/expand.rs` for those fixes and `sayc.rs`/inline unit tests for their regressions.
//!
//! This test fails on any *new* Error the compiler produces that is not in the expected file, and
//! on any *stale* expected entry that no longer reproduces (fixed for free, or the source file
//! changed) -- both signal the file needs a triage update, not a blind re-sync.

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use bridge_system::{CompileOptions, Severity};

/// One expected Error, keyed the same way as an actual one: path relative to
/// `systems/vendor/data`, 1-based line, lint code (`{:?}`). The reason is kept only for the
/// failure message, not for equality -- two runs can phrase the same finding differently without
/// the test caring, as long as the location and code still match.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct ExpectedKey {
    path: String,
    line: u32,
    code: String,
}

fn parse_expected(text: &str) -> Vec<(ExpectedKey, String)> {
    let mut out = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.splitn(4, '\t').collect();
        assert!(
            fields.len() == 4,
            "real_expected_errors.txt:{}: expected 4 tab-separated fields, got {:?}",
            lineno + 1,
            line
        );
        let line_no: u32 = fields[1]
            .parse()
            .unwrap_or_else(|_| panic!("real_expected_errors.txt:{}: bad line number", lineno + 1));
        out.push((
            ExpectedKey {
                path: fields[0].to_string(),
                line: line_no,
                code: fields[2].to_string(),
            },
            fields[3].to_string(),
        ));
    }
    out
}

#[test]
#[ignore = "needs the vendored systems data (cargo xtask systems fetch)"]
fn real_files_error_set_matches_expected() {
    let dir = common::systems_dir();
    let opts = CompileOptions::default();

    let expected_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/real_expected_errors.txt");
    let expected_text = std::fs::read_to_string(&expected_path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", expected_path.display()));
    let expected_entries = parse_expected(&expected_text);
    let expected_set: BTreeSet<ExpectedKey> =
        expected_entries.iter().map(|(k, _)| k.clone()).collect();
    let reason_of = |k: &ExpectedKey| -> &str {
        expected_entries
            .iter()
            .find(|(ek, _)| ek == k)
            .map(|(_, reason)| reason.as_str())
            .unwrap_or("<no reason on file>")
    };

    let mut grand_files = 0usize;
    let mut grand_blocked = 0usize;
    let mut actual_set: BTreeSet<ExpectedKey> = BTreeSet::new();

    for sub in [
        "vendor/data/bml-test/data",
        "vendor/data/jdh8",
        "vendor/data/gjp",
    ] {
        let root = dir.join(sub);
        let files = common::bml_files(&root);
        if files.is_empty() {
            eprintln!("{sub}: not found or empty; skipping");
            continue;
        }
        for path in &files {
            grand_files += 1;
            let Some(ir) = common::compile_guarded(path, &opts) else {
                grand_blocked += 1;
                continue;
            };
            // Path relative to `systems/vendor/data`, the same anchor
            // `real_expected_errors.txt` uses, so entries survive the vendored checkout living
            // at whatever absolute path this machine happened to clone it to.
            let rel = path
                .strip_prefix(dir.join("vendor/data"))
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            for lint in &ir.lints {
                if lint.severity != Severity::Error {
                    continue;
                }
                let line = lint.span.as_ref().map(|s| s.line).unwrap_or(0);
                actual_set.insert(ExpectedKey {
                    path: rel.clone(),
                    line,
                    code: format!("{:?}", lint.code),
                });
            }
        }
    }

    eprintln!(
        "compile_real: {grand_files} file(s), {grand_blocked} blocked \
         (compile_description still todo!()), {} compiled, {} distinct Error location(s)",
        grand_files - grand_blocked,
        actual_set.len()
    );
    if grand_files == grand_blocked {
        return; // nothing landed to check yet; not a failure
    }

    let new_errors: Vec<&ExpectedKey> = actual_set.difference(&expected_set).collect();
    let stale_entries: Vec<&ExpectedKey> = expected_set.difference(&actual_set).collect();

    if !new_errors.is_empty() {
        eprintln!("--- new Error(s) not in real_expected_errors.txt ---");
        for k in &new_errors {
            eprintln!("  {}:{} {}", k.path, k.line, k.code);
        }
    }
    if !stale_entries.is_empty() {
        eprintln!("--- stale real_expected_errors.txt entries that no longer reproduce ---");
        for k in &stale_entries {
            eprintln!("  {}:{} {} ({})", k.path, k.line, k.code, reason_of(k));
        }
    }
    assert!(
        new_errors.is_empty() && stale_entries.is_empty(),
        "{} new Error(s), {} stale expected entry/entries (see stderr); triage new ones into \
         real_expected_errors.txt / real_lint_triage.md or fix the compiler, and delete stale \
         entries",
        new_errors.len(),
        stale_entries.len()
    );
}
