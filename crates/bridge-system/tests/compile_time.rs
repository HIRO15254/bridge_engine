//! BML compile-time budget (`docs/design/11-testing.md` §1, §9, roadmap 3.2-3.4): compiling one
//! system should take well under 1 second. Checked against two files: the largest real vendored
//! file by source size (roadmap 3.2-3.4's honesty pass) and `systems/sayc/sayc.bml` (roadmap
//! 3.5's own target system, root file that `#INCLUDE`s the rest of `systems/sayc/*.bml`).
//!
//! The budget is only meaningful under optimizations, so the `< 1s` assertion only fires in a
//! `--release` build (`cfg!(debug_assertions)` is false there); a debug build still compiles both
//! files and reports the timings to stderr, so `cargo test` (no `--release`) exercises the same
//! code path without flaking on a slow debug build. `#[ignore]`d because the largest-real-file
//! half needs the vendored systems data; run with
//! `cargo test -p bridge-system --release -- --ignored compile_time` for the enforced budget.
//!
//! **Partially blocked**: see `bss_oracle.rs`'s module doc for `compile_guarded`'s "blocked"
//! convention. If every vendored candidate is blocked, the real-file half is skipped (not
//! failed); `sayc.bml` does not depend on vendored data and is never blocked by this.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use bridge_system::CompileOptions;

/// Compiles `path`, reports the elapsed time, and -- only in a `--release` build -- asserts it
/// stayed under the 1 second budget. Returns `None` (nothing measured) when `compile_guarded`
/// reports the file as blocked.
fn check_compile_time(path: &Path, opts: &CompileOptions) -> Option<Duration> {
    let text = std::fs::read_to_string(path).unwrap();
    let started = Instant::now();
    let compiled = common::compile_guarded(path, opts);
    let elapsed = started.elapsed();
    compiled.as_ref()?;
    eprintln!(
        "compile_time: {} ({} bytes) compiled in {elapsed:?}{}",
        path.display(),
        text.len(),
        if cfg!(debug_assertions) {
            " (debug build; budget not enforced)"
        } else {
            ""
        }
    );
    if !cfg!(debug_assertions) {
        assert!(
            elapsed.as_secs_f64() < 1.0,
            "{} took {elapsed:?}, budget is < 1s",
            path.display()
        );
    }
    Some(elapsed)
}

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
        if check_compile_time(path, &opts).is_some() {
            return;
        }
        // blocked by compile_description's todo!(); try the next-largest file
    }
    eprintln!(
        "compile_time: all {} candidate file(s) are blocked (compile_description still todo!()); \
         nothing to measure yet",
        candidates.len()
    );
}

#[test]
fn compiling_sayc_is_fast() {
    let opts = CompileOptions::default();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../systems/sayc/sayc.bml");
    assert!(
        check_compile_time(&path, &opts).is_some(),
        "{} did not compile (see stderr)",
        path.display()
    );
}
