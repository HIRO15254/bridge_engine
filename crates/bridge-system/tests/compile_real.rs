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
    // `dir.join("vendor/data")` is not canonicalized (`systems_dir()` builds it through a
    // `../../systems` component that is never resolved), but an `#INCLUDE`d file's path *is*
    // normalized away down to its shortest form by `lexer::join_path` (it collapses any `..` it
    // finds while joining, including ones from this crate-relative root having gone through a
    // table's own directory and back). A literal `strip_prefix` between the two would then fail
    // for every included file while still succeeding for the root file's own (never-normalized)
    // path. Canonicalizing both sides once here keeps the comparison honest regardless of which
    // shape a given file's path happens to be in.
    let vendor_root = dir.join("vendor/data").canonicalize().ok();

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
    // Count of *raw* Errors mapping to each key: several roots can `#INCLUDE` the same file (or
    // retrace the same position), reproducing the same source Error more than once. A count > 1
    // is reported (not just silently deduplicated) so a genuinely new second Error that happens
    // to land on an already-listed (file, line, code) is not hidden by the set-based comparison
    // below.
    let mut actual_counts: std::collections::BTreeMap<ExpectedKey, u32> =
        std::collections::BTreeMap::new();

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
            let Some((ir, file_table)) = common::compile_guarded_with_files(path, &opts) else {
                grand_blocked += 1;
                continue;
            };
            // A lint's `span.file` names the file it actually came from -- the root file being
            // compiled here only when there was no `#INCLUDE` involved. An `Error` raised inside
            // an `#INCLUDE`d file must be attributed to *that* file's path and line, not to the
            // root's: otherwise every included file's Errors are misattributed to whichever root
            // happened to pull it in, at a line number that belongs to a different file entirely
            // (`real_lint_triage.md`'s review), and the same source Error is double-counted once
            // per root that includes it.
            for lint in &ir.lints {
                if lint.severity != Severity::Error {
                    continue;
                }
                let span = lint.span.as_ref();
                let file_id = span.map_or(0, |s| s.file.0 as usize);
                let true_path = file_table.get(file_id).map(String::as_str).unwrap_or("");
                // Path relative to `systems/vendor/data`, the same anchor
                // `real_expected_errors.txt` uses, so entries survive the vendored checkout
                // living at whatever absolute path this machine happened to clone it to.
                // Canonicalized on both sides first (see the comment on `vendor_root` above).
                let canonical = std::fs::canonicalize(true_path).ok();
                let rel = match (&canonical, &vendor_root) {
                    (Some(c), Some(root)) => c
                        .strip_prefix(root)
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_else(|_| true_path.to_string()),
                    _ => true_path.to_string(),
                };
                let line = span.map_or(0, |s| s.line);
                let key = ExpectedKey {
                    path: rel,
                    line,
                    code: format!("{:?}", lint.code),
                };
                actual_set.insert(key.clone());
                *actual_counts.entry(key).or_insert(0) += 1;
            }
        }
    }

    let duplicate_locations: Vec<(&ExpectedKey, &u32)> =
        actual_counts.iter().filter(|&(_, &n)| n > 1).collect();
    if !duplicate_locations.is_empty() {
        eprintln!("--- (file, line, code) reached more than once (multiple roots/retraces) ---");
        for (k, n) in &duplicate_locations {
            eprintln!("  {}:{} {} x{}", k.path, k.line, k.code, n);
        }
    }

    eprintln!(
        "compile_real: {grand_files} file(s), {grand_blocked} blocked \
         (compile_description still todo!()), {} compiled, {} distinct Error location(s), \
         {} raw Error(s)",
        grand_files - grand_blocked,
        actual_set.len(),
        actual_counts.values().sum::<u32>()
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
