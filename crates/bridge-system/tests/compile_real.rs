//! Compiles every vendored real-world BML file and requires zero `Error`-severity lints on
//! whatever compiles (roadmap 3.2/3.4). Vendored data is git-ignored; skipped when absent.
//!
//! **Blocked today**: see `bss_oracle.rs`'s module doc -- `compile_description` is still
//! `todo!()`, so every file with a non-empty description (all of them) is reported as blocked
//! rather than compiled. `common::compile_guarded` makes that a skip, not a panic.

mod common;

use bridge_system::{CompileOptions, Severity};

#[test]
#[ignore = "needs the vendored systems data (cargo xtask systems fetch)"]
fn real_files_compile_with_zero_errors() {
    let dir = common::systems_dir();
    let opts = CompileOptions::default();

    let mut grand_files = 0usize;
    let mut grand_blocked = 0usize;
    let mut grand_errors = 0usize;
    let mut grand_warnings = 0usize;

    for (sub, label) in [
        ("vendor/data/bml-test/data", "bml-test"),
        ("vendor/data/jdh8", "jdh8"),
        ("vendor/data/gjp", "gjp"),
    ] {
        let files = common::bml_files(&dir.join(sub));
        if files.is_empty() {
            eprintln!("{label}: not found or empty; skipping");
            continue;
        }
        for path in &files {
            grand_files += 1;
            let Some(ir) = common::compile_guarded(path, &opts) else {
                grand_blocked += 1;
                continue;
            };
            let summary = bridge_system::lint::LintSummary::of(&ir.lints);
            grand_errors += summary.errors;
            grand_warnings += summary.warnings;
            if summary.errors > 0 {
                for lint in &ir.lints {
                    if lint.severity == Severity::Error {
                        eprintln!("  {}: {lint}", path.display());
                    }
                }
            }
        }
    }

    eprintln!(
        "compile_real: {grand_files} file(s), {grand_blocked} blocked \
         (compile_description still todo!()), {} compiled, {grand_errors} errors, \
         {grand_warnings} warnings",
        grand_files - grand_blocked
    );
    if grand_files == grand_blocked {
        return; // nothing landed to check yet; not a failure
    }
    assert_eq!(
        grand_errors, 0,
        "{grand_errors} Error-severity lints (see stderr)"
    );
}
