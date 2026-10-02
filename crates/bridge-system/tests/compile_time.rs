//! BML compile-time budget (`docs/design/11-testing.md` §1, §9, roadmap 3.2-3.4): compiling one
//! system should take well under 1 second. Checked against two files: the largest real vendored
//! file by source size (roadmap 3.2-3.4's honesty pass) and `systems/sayc/sayc.bml` (roadmap
//! 3.5's own target system, root file that `#INCLUDE`s the rest of `systems/sayc/*.bml`).
//!
//! The budget is only meaningful under optimizations, so the `< 1s` assertion only fires in a
//! `--release` build (`cfg!(debug_assertions)` is false there); a debug build still compiles both
//! files and reports the timings to stderr, so `cargo test` (no `--release`) exercises the same
//! code path without flaking on a slow debug build. The largest-real-file half is `#[ignore]`d
//! because it needs the vendored systems data; the SAYC half (`compiling_sayc_is_fast`) and the
//! exclusive-index bound (`sayc_exclusive_index_build_is_bounded`) are not. Every enforced budget
//! runs with
//! `cargo test -p bridge-system --release --test compile_time -- --include-ignored`
//! (`--ignored` alone would skip the two non-ignored checks).
//!
//! CI does not enforce the two non-ignored release-only checks: the per-PR jobs run the tests in
//! debug, where they assert nothing, and the nightly release job runs `-- --ignored`, which skips
//! them. They are local release checks, run by hand at the phase gates (`12-roadmap.md`); the
//! 20 ms bound of `sayc_exclusive_index_build_is_bounded` is calibrated on the development
//! machine only, not on a CI runner.
//!
//! **Partially blocked**: see `bss_oracle.rs`'s module doc for `compile_guarded`'s "blocked"
//! convention. If every vendored candidate is blocked, the real-file half is skipped (not
//! failed); `sayc.bml` does not depend on vendored data and is never blocked by this.

mod common;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bridge_system::CompileOptions;

/// Serialises the timed SAYC tests of this binary. The test harness runs them on parallel
/// threads by default, and a second SAYC compile on another core pushed the release
/// `compiling_sayc_is_fast` past its 1 s budget at loadavg ~5 (1.01-1.07 s, 2 runs in 3).
static SAYC_TIMING: Mutex<()> = Mutex::new(());

fn sayc_timing_lock() -> MutexGuard<'static, ()> {
    SAYC_TIMING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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
#[ignore = "needs the vendored systems data; cargo test -p bridge-system --release --test compile_time -- --include-ignored"]
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
    let _serial = sayc_timing_lock();
    let opts = CompileOptions::default();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../systems/sayc/sayc.bml");
    assert!(
        check_compile_time(&path, &opts).is_some(),
        "{} did not compile (see stderr)",
        path.display()
    );
}

/// A coarse release-only guard on SAYC's exclusive-index build (the criterion is <= 15 ms;
/// lane D2's thickened SAYC, about 9,500 index nodes, measured 19.8-20.0 ms best of 3 at
/// loadavg ~3.9 before the phase-4 performance lane, and 7.8-8.6 ms after it at loadavg 3,
/// still 8.9-9.8 ms at loadavg 22-24; the pasted-chain SAYC took 45 ms): best of 3 must stay
/// under 20 ms, about twice the current time: tight enough to flag a return towards the
/// pre-lane 18-20 ms, loose enough not to flake on a loaded machine. A debug build builds the
/// index once and asserts nothing. Holds the SAYC timing lock so that its own compile never overlaps
/// `compiling_sayc_is_fast`. A local release check that CI does not run (see the module doc).
#[test]
fn sayc_exclusive_index_build_is_bounded() {
    let _serial = sayc_timing_lock();
    let opts = CompileOptions::default();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../systems/sayc/sayc.bml");
    let ir = common::compile_guarded(&path, &opts).expect("sayc.bml compiles");
    let rounds = if cfg!(debug_assertions) { 1 } else { 3 };
    let mut best = Duration::MAX;
    for _ in 0..rounds {
        let started = Instant::now();
        let index = bridge_system::ExclusiveIndex::build(&ir);
        best = best.min(started.elapsed());
        assert_eq!(index.stats(&ir).groups, ir.exclusive().stats(&ir).groups);
    }
    eprintln!("sayc: exclusive index build {best:?} (best of {rounds})");
    if !cfg!(debug_assertions) {
        assert!(
            best < Duration::from_millis(20),
            "SAYC exclusive-index build {best:?} (best of 3) >= 20 ms"
        );
    }
}

/// Best-of-3 release timing of the whole SAYC compile and of rebuilding its exclusive index
/// alone (the index is built eagerly at the end of `compile()`, docs/design/06-system.md §5.4):
/// `cargo test -p bridge-system --release --test compile_time -- --ignored --nocapture
/// sayc_exclusive_index_share`.
#[test]
#[ignore = "timing; run in release with --ignored --nocapture"]
fn sayc_exclusive_index_share() {
    let _serial = sayc_timing_lock();
    let opts = CompileOptions::default();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../systems/sayc/sayc.bml");
    let mut compile_best = Duration::MAX;
    let mut index_best = Duration::MAX;
    for _ in 0..3 {
        let started = Instant::now();
        let ir = common::compile_guarded(&path, &opts).expect("sayc.bml compiles");
        compile_best = compile_best.min(started.elapsed());
        let started = Instant::now();
        let index = bridge_system::ExclusiveIndex::build(&ir);
        index_best = index_best.min(started.elapsed());
        assert_eq!(index.stats(&ir).groups, ir.exclusive().stats(&ir).groups);
    }
    eprintln!(
        "sayc: compile {compile_best:?}, exclusive index build {index_best:?} ({:.1}% of the \
         compile; best of 3)",
        100.0 * index_best.as_secs_f64() / compile_best.as_secs_f64()
    );
}
