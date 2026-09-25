//! Parses the reference (`bml-test`) and real-world (`jdh8`, `gjp`) corpora and requires zero
//! `Error`-severity lints (`cargo test -p bridge-system --release -- --ignored --nocapture`).
//!
//! Vendored data is git-ignored; the test is skipped (not failed) when it is absent.

mod common;

use bridge_system::{Severity, lexer, parser};
use common::{bml_files, systems_dir};

struct FileReport {
    path: std::path::PathBuf,
    errors: usize,
    warnings: usize,
    infos: usize,
}

fn parse_one(path: &std::path::Path) -> FileReport {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let loaded = lexer::load(&path.to_string_lossy(), &text, &lexer::FsLoader);
    let file = parser::parse(loaded);
    let summary = bridge_system::lint::LintSummary::of(&file.lints);
    if summary.errors > 0 || std::env::var_os("BML_DEBUG_LINTS").is_some() {
        for lint in &file.lints {
            if summary.errors > 0 && lint.severity != Severity::Error {
                continue;
            }
            eprintln!("  {}: {lint}", path.display());
        }
    }
    FileReport {
        path: path.to_path_buf(),
        errors: summary.errors,
        warnings: summary.warnings,
        infos: summary.infos,
    }
}

fn run_over(dir: std::path::PathBuf, label: &str) -> (usize, usize, usize, usize) {
    let files = bml_files(&dir);
    if files.is_empty() {
        eprintln!("{label}: {} not found or empty; skipping", dir.display());
        return (0, 0, 0, 0);
    }
    let mut total_errors = 0;
    let mut total_warnings = 0;
    let mut total_infos = 0;
    for path in &files {
        let report = parse_one(path);
        println!(
            "{label:<10} {:<70} errors {:>3} warnings {:>3} infos {:>3}",
            report
                .path
                .strip_prefix(&dir)
                .unwrap_or(&report.path)
                .display(),
            report.errors,
            report.warnings,
            report.infos
        );
        total_errors += report.errors;
        total_warnings += report.warnings;
        total_infos += report.infos;
    }
    (files.len(), total_errors, total_warnings, total_infos)
}

#[test]
#[ignore = "needs the vendored systems data (cargo xtask systems fetch)"]
fn corpus_parses_with_zero_errors() {
    let dir = systems_dir();
    let mut grand_files = 0;
    let mut grand_errors = 0;
    let mut grand_warnings = 0;
    let mut grand_infos = 0;

    for (sub, label) in [
        ("vendor/data/bml-test/data", "bml-test"),
        ("vendor/data/jdh8", "jdh8"),
        ("vendor/data/gjp", "gjp"),
    ] {
        let (files, errors, warnings, infos) = run_over(dir.join(sub), label);
        grand_files += files;
        grand_errors += errors;
        grand_warnings += warnings;
        grand_infos += infos;
    }

    println!(
        "TOTAL: {grand_files} files, {grand_errors} errors, {grand_warnings} warnings, {grand_infos} infos"
    );
    if grand_files == 0 {
        return; // no vendored data at all; nothing to check
    }
    assert_eq!(
        grand_errors, 0,
        "{grand_errors} Error-severity lints (see stderr)"
    );
}
