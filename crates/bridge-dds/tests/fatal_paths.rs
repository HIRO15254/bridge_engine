//! DDS 2.9 has two configuration paths that terminate the host process with `exit(1)` from
//! inside C++ (docs/design/10-dds.md §7.2, R6), which neither the `noexcept` guard nor Rust can
//! intercept:
//!
//! - upstream `SetResources` always runs its hardware probe (`popen` of `sysctl` on macOS,
//!   `free | tail | awk` on Linux) and caps memory at what it reads; a failed probe reads 0, DDS
//!   sizes itself to zero threads and `Memory::GetPtr(0)` exits;
//! - a `max_memory_mb` below 24 gives zero threads the same way.
//!
//! The wrapper reimplements `SetResources` without the probe and clamps the memory, so both
//! must now run normally. Each case runs in a child process (this test binary re-executed with
//! a filter) because DDS is initialised once per process and a regression would kill the
//! process rather than fail an assertion.
#![cfg(dds_vendored)]

use std::process::Command;

use bridge_core::{Deal, Hand, Seat};
use bridge_dds::{DdsConfig, calc_dd_table, info, init};

const CHILD: &str = "BRIDGE_DDS_FATAL_PATHS_CHILD";
const CHILD_MEMORY: &str = "BRIDGE_DDS_FATAL_PATHS_MEMORY";

fn deal() -> Deal {
    let hand = |s: &str| -> Hand { s.parse().unwrap() };
    let mut hands = [Hand::EMPTY; 4];
    hands[Seat::North.index() as usize] = hand("KQ43.A72.KQ5.864");
    hands[Seat::East.index() as usize] = hand("-.T843.T986.JT975");
    hands[Seat::South.index() as usize] = hand("AJT982.K5.A73.A2");
    hands[Seat::West.index() as usize] = hand("765.QJ96.J42.KQ3");
    Deal::new(hands).unwrap()
}

/// The body run inside the child process; a no-op in the parent test run.
#[test]
fn child_entry() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let max_memory_mb = std::env::var(CHILD_MEMORY)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    init(DdsConfig {
        max_threads: 1,
        max_memory_mb,
    })
    .expect("init");
    let threads = info().expect("info").threads;
    assert!(threads >= 1, "DDS configured {threads} threads");
    calc_dd_table(&deal()).expect("calc_dd_table");
}

fn run_child(envs: &[(&str, &str)]) {
    let exe = std::env::current_exe().expect("test binary path");
    let mut cmd = Command::new(exe);
    cmd.args(["--exact", "child_entry", "--test-threads", "1"])
        .env(CHILD, "1");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn the test binary");
    assert!(
        out.status.success(),
        "child {envs:?} exited with {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // The child must really have run the body, not been filtered out.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("1 passed"), "child did not run: {stdout}");
}

/// Regression: with no `sysctl`/`free` reachable, upstream's probe read 0 MB and the first DDS
/// call printed `Memory::GetPtr: 0 vs. 0` and exited with status 1, whatever memory `init` passed.
#[test]
fn init_survives_a_failed_hardware_probe() {
    run_child(&[("PATH", "/nonexistent"), (CHILD_MEMORY, "950")]);
    run_child(&[("PATH", "/nonexistent"), (CHILD_MEMORY, "0")]);
}

/// Regression: `max_memory_mb` of 20 or 23 gave `floor(1.3 × M / 30) = 0` threads and the same
/// `exit(1)`; it is now raised to `MIN_MEMORY_MB`.
#[test]
fn init_with_tiny_memory_does_not_exit() {
    assert_eq!(bridge_dds::MIN_MEMORY_MB, 24);
    for mb in ["1", "20", "23", "24"] {
        run_child(&[(CHILD_MEMORY, mb)]);
    }
}
