//! `cargo xtask <command>` — developer tasks that are not part of the library.
//!
//! | Command | Phase | Purpose |
//! | --- | --- | --- |
//! | `corpus fetch` | 1 | Download the PBN/LIN corpora listed in `corpus/manifest.toml`, verify SHA-256, unpack into `corpus/data/` |
//! | `systems fetch` | 3 | Download external BML files and expected `.bss` outputs listed in `systems/vendor/manifest.toml` |
//! | `dds vendor` | 5 | Download DDS v2.9.0 sources into `crates/bridge-dds/vendor/dds-2.9.0/` and verify the hash |
//! | `dds regen-bindings` | 5 | Regenerate `crates/bridge-dds/src/sys.rs` with bindgen (offline check against the hand-written file) |
//! | `coverage` | 3 | Run the bidirectional-consistency harness and write `target/coverage_report.json` |
//!
//! Add `[alias] xtask = "run --package xtask --"` to `.cargo/config.toml` to invoke as `cargo xtask`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["corpus", "fetch"] => not_implemented("corpus fetch", 1),
        ["systems", "fetch"] => not_implemented("systems fetch", 3),
        ["dds", "vendor"] => not_implemented("dds vendor", 5),
        ["dds", "regen-bindings"] => not_implemented("dds regen-bindings", 5),
        ["coverage"] => not_implemented("coverage", 3),
        _ => {
            eprintln!(
                "usage: cargo xtask <corpus fetch | systems fetch | dds vendor | dds regen-bindings | coverage>"
            );
            ExitCode::from(2)
        }
    }
}

fn not_implemented(command: &str, phase: u8) -> ExitCode {
    eprintln!("xtask {command}: not implemented yet (scheduled for phase {phase})");
    ExitCode::from(1)
}
