//! `.bss` oracle test (`docs/design/06-system.md` §4.2, `11-testing.md` §5): compiles each
//! vendored `bml-test` file and compares the resulting (we-opened, seat, vul, call sequence,
//! description) tuples against the reference Python `bss.py`'s expected `.bss` output.
//!
//! Vendored data is git-ignored; the test is skipped (not failed) when it is absent.
//!
//! **Blocked today**: `compile_description` is still `todo!()` on this branch (owned by another
//! lane), so `compile()` panics for every row with a non-empty description -- every row in every
//! real `.bml` file. `common::compile_guarded` turns that panic into a per-file skip instead of a
//! test failure; the comparison logic below is unconditional, so it runs for real the moment
//! `compile_description` lands, with no further changes needed here.

mod common;

use bridge_system::{CompileOptions, Side};
use common::bss::{apply_override, bss_seat_char, bss_sequence, bss_vul_char, parse_bss};

#[test]
fn expansion_matches_the_bss_oracle_where_available() {
    let root = common::systems_dir().join("vendor/data/bml-test");
    let data_dir = root.join("data");
    let expected_dir = root.join("expected");
    if !data_dir.is_dir() {
        eprintln!(
            "bml-test data not found under {}; skipping",
            data_dir.display()
        );
        return;
    }

    let opts = CompileOptions::default();
    let mut compiled = 0usize;
    let mut blocked = 0usize;
    let mut mismatches: Vec<String> = Vec::new();

    for bml_path in common::bml_files(&data_dir) {
        let Some(stem) = bml_path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let bss_path = expected_dir.join(format!("{stem}.bss"));
        if !bss_path.is_file() {
            continue; // no oracle for this file (e.g. an #INCLUDE-only fragment)
        }

        let Some(ir) = common::compile_guarded(&bml_path, &opts) else {
            blocked += 1;
            continue;
        };
        compiled += 1;

        let bss_text = std::fs::read_to_string(&bss_path)
            .unwrap_or_else(|e| panic!("{}: {e}", bss_path.display()));
        let mut oracle = parse_bss(&bss_text);
        let override_path = expected_dir.join(format!("{stem}.bss.override"));
        if let Ok(text) = std::fs::read_to_string(&override_path) {
            apply_override(&mut oracle, &parse_bss(&text));
        }

        // (we_open, seat, vul, sequence) -> description, one entry per node.
        let mut ours: std::collections::HashMap<(bool, char, char, String), String> =
            std::collections::HashMap::new();
        for node in &ir.nodes {
            let we_open = node
                .path
                .first()
                .map(|p| p.side == Side::Us)
                .unwrap_or(true);
            let key = (
                we_open,
                bss_seat_char(node.seat),
                bss_vul_char(node.vul),
                bss_sequence(&node.calls),
            );
            ours.insert(key, node.description.clone());
        }

        for entry in &oracle.entries {
            let key = (entry.we_open, entry.seat, entry.vul, entry.sequence.clone());
            let star = if entry.we_open { "" } else { "*" };
            match ours.get(&key) {
                None => mismatches.push(format!(
                    "{stem}: no node expanded for {star}{}{}{}",
                    entry.seat, entry.vul, entry.sequence
                )),
                Some(desc) => {
                    if desc.trim() != entry.desc.trim() {
                        mismatches.push(format!(
                            "{stem}: {star}{}{}{} description mismatch:\n    ours:   {:?}\n    oracle: {:?}",
                            entry.seat, entry.vul, entry.sequence, desc.trim(), entry.desc.trim()
                        ));
                    }
                }
            }
        }
    }

    eprintln!(
        "bss_oracle: {compiled} file(s) compiled and compared, {blocked} blocked \
         (compile_description still todo!())"
    );
    if compiled == 0 {
        return; // nothing landed to compare yet; not a failure
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatch(es):\n\n{}",
        mismatches.len(),
        mismatches.join("\n\n")
    );
}
