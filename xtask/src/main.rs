//! `cargo xtask <command>` — developer tasks that are not part of the library.
//!
//! | Command | Phase | Purpose |
//! | --- | --- | --- |
//! | `corpus fetch [--pin] [--force] [--only <name>]...` | 1 | Download the PBN/LIN corpora listed in `corpus/manifest.toml`, verify SHA-256, unpack into `corpus/data/` (or `$BRIDGE_CORPUS_DIR`) |
//! | `systems fetch [--pin] [--force] [--only <name>]...` | 3 | Download external BML files and expected `.bss` outputs listed in `systems/vendor/manifest.toml` into `systems/vendor/data/` |
//! | `dds vendor` | 5 | Download DDS v2.9.0 sources into `crates/bridge-dds/vendor/dds-2.9.0/` and verify the hash |
//! | `dds regen-bindings` | 5 | Regenerate `crates/bridge-dds/src/sys.rs` with bindgen (offline check against the hand-written file) |
//! | `coverage` | 3 | Run the bidirectional-consistency harness and write `target/coverage_report.json` |
//!
//! `fetch` options: `--pin` writes the SHA-256 of a not-yet-pinned entry back into the manifest,
//! `--force` re-downloads entries that are already present and verified, `--only <name>` limits
//! the run to the named entries (repeatable). Exit code 0 = all verified, 2 = some entries are
//! still unpinned (hash printed, manifest untouched), 1 = an entry failed.
//!
//! `.cargo/config.toml` has `[alias] xtask = "run --package xtask --"`, so invoke as `cargo xtask`.

mod dds;
mod fetch;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Error type of every command: a message for the user, nothing to match on.
type Error = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, Error>;

const USAGE: &str = "usage: cargo xtask <corpus fetch | systems fetch | dds vendor | dds regen-bindings | coverage> [options]
  corpus fetch  [--pin] [--force] [--only <name>]...
  systems fetch [--pin] [--force] [--only <name>]...";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match words.as_slice() {
        ["corpus", "fetch", rest @ ..] => fetch::run(fetch::Target::Corpus, rest),
        ["systems", "fetch", rest @ ..] => fetch::run(fetch::Target::Systems, rest),
        ["dds", "vendor"] => dds::vendor(),
        ["dds", "regen-bindings"] => Ok(not_implemented("dds regen-bindings", 5)),
        ["coverage", ..] => Ok(not_implemented("coverage", 3)),
        _ => {
            eprintln!("{USAGE}");
            Ok(ExitCode::from(2))
        }
    };
    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("xtask: error: {err}");
            ExitCode::from(1)
        }
    }
}

fn not_implemented(command: &str, phase: u8) -> ExitCode {
    eprintln!("xtask {command}: not implemented yet (scheduled for phase {phase})");
    ExitCode::from(1)
}

/// The workspace root (the parent of `xtask/`), independent of the current directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask/ lives directly under the workspace root")
        .to_path_buf()
}
