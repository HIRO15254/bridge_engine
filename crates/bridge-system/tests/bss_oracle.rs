//! `.bss` oracle test (`docs/design/06-system.md` §4.2, `11-testing.md` §5): compiles each
//! vendored `bml-test` file, renders the expansion back into `.bss` form (`bss.py`'s own
//! `systemdata_to_bss` encoding: `\n`-escaped descriptions, a trailing lone-`.` line dropped) and
//! compares it against the reference Python `bss.py`'s expected `.bss` output *in both
//! directions*: every oracle entry must be produced, and we must not produce an entry the oracle
//! does not have.
//!
//! Vendored data is git-ignored; the test is skipped (not failed) when it is absent.
//!
//! Intended differences from the oracle (the `#` substitution the design deliberately changed,
//! see `docs/design/06-system.md` §1.4, or a structural difference in how many nodes a table
//! produces) are recorded as overrides next to this test, under `tests/bss_overrides/<stem>
//! .bss.override` -- never under the git-ignored vendor data directory -- with a comment
//! explaining each one (see `common::bss::OverrideAction`).

mod common;

use std::collections::{HashMap, HashSet};

use bridge_system::{CompileOptions, Side};
use common::bss::{
    BssKey, apply_overrides, bss_seat_char, bss_sequence, bss_vul_char, parse_bss, parse_overrides,
    to_bss_desc,
};

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
    let overrides_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/bss_overrides");

    let opts = CompileOptions::default();
    let mut compiled = 0usize;
    let mut blocked = 0usize;
    let mut matching = 0usize;
    let mut overridden = 0usize;
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

        let override_path = overrides_dir.join(format!("{stem}.bss.override"));
        let deleted: HashSet<BssKey> = match std::fs::read_to_string(&override_path) {
            Ok(text) => {
                let actions = parse_overrides(&text);
                overridden += actions.len();
                apply_overrides(&mut oracle, &actions)
            }
            Err(_) => HashSet::new(),
        };

        // Render our own expansion into the same (we_open, seat, vul, sequence) -> bss-encoded
        // description map bss.py's file would hold, one entry per node.
        let mut ours: HashMap<BssKey, String> = HashMap::new();
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
            ours.insert(key, to_bss_desc(&node.description));
        }

        // Direction 1: every oracle entry must be produced, with a matching description.
        for entry in &oracle.entries {
            let key: BssKey = (entry.we_open, entry.seat, entry.vul, entry.sequence.clone());
            let star = if entry.we_open { "" } else { "*" };
            match ours.get(&key) {
                None => mismatches.push(format!(
                    "{stem}: no node expanded for {star}{}{}{}",
                    entry.seat, entry.vul, entry.sequence
                )),
                Some(desc) => {
                    if *desc == entry.desc {
                        matching += 1;
                    } else {
                        mismatches.push(format!(
                            "{stem}: {star}{}{}{} description mismatch:\n    ours:   {:?}\n    oracle: {:?}",
                            entry.seat, entry.vul, entry.sequence, desc, entry.desc
                        ));
                    }
                }
            }
        }

        // Direction 2: we must not produce a node the oracle doesn't have and that no override
        // explicitly deleted.
        let oracle_keys: HashSet<BssKey> = oracle
            .entries
            .iter()
            .map(|e| (e.we_open, e.seat, e.vul, e.sequence.clone()))
            .collect();
        for key in ours.keys() {
            if !oracle_keys.contains(key) && !deleted.contains(key) {
                let (we_open, seat, vul, sequence) = key;
                let star = if *we_open { "" } else { "*" };
                mismatches.push(format!(
                    "{stem}: extra node {star}{seat}{vul}{sequence} not in the oracle (add a \
                     DELETE override under tests/bss_overrides/ if this is intentional)"
                ));
            }
        }
    }

    eprintln!(
        "bss_oracle: {compiled} file(s) compiled and compared, {blocked} blocked \
         (compile_description still todo!()), {matching} entries matching, {overridden} \
         override action(s) applied"
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
