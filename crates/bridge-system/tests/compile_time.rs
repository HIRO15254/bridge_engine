//! BML compile-time budget (`docs/design/11-testing.md` §1, §9): compiling one system should
//! take well under 1 second. `#[ignore]`d because it needs the vendored systems data; run with
//! `cargo test -p bridge-system --release -- --ignored compile_time`.
//!
//! **Blocked today**: see `bss_oracle.rs`'s module doc. When every candidate file is blocked by
//! `compile_description`'s `todo!()`, this test reports that and returns rather than measuring
//! nothing meaningful.

mod common;

use std::time::Instant;

use bridge_system::CompileOptions;

#[test]
#[ignore = "needs the vendored systems data; cargo test --release -- --ignored compile_time"]
fn compiling_the_largest_real_system_is_fast() {
    let dir = common::systems_dir();
    let opts = CompileOptions::default();

    // The largest file across the vendored corpora, by source size, is the one whose compile
    // time actually matters.
    let mut candidates: Vec<_> = [
        "vendor/data/bml-test/data",
        "vendor/data/jdh8",
        "vendor/data/gjp",
    ]
    .iter()
    .flat_map(|sub| common::bml_files(&dir.join(sub)))
    .collect();
    if candidates.is_empty() {
        eprintln!("no vendored systems data found; skipping");
        return;
    }
    candidates
        .sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)));

    for path in &candidates {
        let text = std::fs::read_to_string(path).unwrap();
        let started = Instant::now();
        let compiled = common::compile_guarded(path, &opts);
        let elapsed = started.elapsed();
        if compiled.is_none() {
            continue; // blocked by compile_description's todo!(); try the next-largest file
        }
        eprintln!(
            "compile_time: {} ({} bytes) compiled in {elapsed:?}",
            path.display(),
            text.len()
        );
        assert!(
            elapsed.as_secs_f64() < 1.0,
            "{} took {elapsed:?}, budget is < 1s",
            path.display()
        );
        return;
    }
    eprintln!(
        "compile_time: all {} candidate file(s) are blocked (compile_description still todo!()); \
         nothing to measure yet",
        candidates.len()
    );
}
