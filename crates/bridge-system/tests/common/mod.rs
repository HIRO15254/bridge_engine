//! Shared test helpers: where the vendored/fixture BML files live.
#![allow(dead_code)]

use std::path::PathBuf;

/// `BRIDGE_SYSTEMS_DIR`, or `<crate>/../../systems`.
pub fn systems_dir() -> PathBuf {
    match std::env::var_os("BRIDGE_SYSTEMS_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../systems"),
    }
}

/// Every `.bml` file under `dir`, recursively, sorted.
pub fn bml_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(bml_files(&path));
        } else if path.extension().is_some_and(|e| e == "bml") {
            out.push(path);
        }
    }
    out.sort();
    out
}
