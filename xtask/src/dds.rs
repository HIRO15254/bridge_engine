//! `dds vendor`: download the DDS v2.9.0 source archive, verify its SHA-256, and extract the
//! files `crates/bridge-dds/build.rs` compiles into `crates/bridge-dds/vendor/dds-2.9.0/`
//! (see `crates/bridge-dds/VENDOR.md`).
//!
//! Only `src/*.cpp`, `src/*.h`, `include/dll.h`, `LICENSE` and (if present) `NOTICE` are
//! extracted; the archive stays in `vendor/` so that `sha256sum -c SHA256SUMS` works there.

use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::process::ExitCode;

use flate2::read::GzDecoder;

use crate::Result;
use crate::fetch::{Fetcher, sha256_file};

const DDS_VERSION: &str = "2.9.0";
const DDS_URL: &str = "https://github.com/dds-bridge/dds/archive/refs/tags/v2.9.0.tar.gz";
/// SHA-256 of the GitHub tag archive, computed on 2026-09-18 (14,902,044 bytes). GitHub keeps
/// tag archives byte-stable; if this ever mismatches, inspect the new archive before updating.
const DDS_SHA256: &str = "9ef36d8c36bf697ba3b499fcb9dca51a4b423278ac72e947235ac86f0b5fc38a";
/// Number of `src/*.cpp` files `build.rs` expects.
const EXPECTED_CPP: usize = 27;

pub fn vendor() -> Result<ExitCode> {
    let vendor = crate::workspace_root().join("crates/bridge-dds/vendor");
    fs::create_dir_all(&vendor)?;
    let archive_name = format!("dds-{DDS_VERSION}.tar.gz");
    let archive = vendor.join(&archive_name);

    let cached = if archive.is_file() {
        let (sha256, bytes) = sha256_file(&archive)?;
        if sha256 == DDS_SHA256 {
            eprintln!("{}: verified (cached), {bytes} bytes", archive.display());
            Some(sha256)
        } else {
            eprintln!(
                "{}: cached archive has a different hash; downloading again",
                archive.display()
            );
            None
        }
    } else {
        None
    };
    let sha256 = match cached {
        Some(sha256) => sha256,
        None => download(&archive)?,
    };
    if sha256 != DDS_SHA256 {
        let _ = fs::remove_file(&archive);
        return Err(format!(
            "sha256 mismatch for {DDS_URL}: expected {DDS_SHA256}, got {sha256}; archive deleted. \
             If upstream re-generated the archive, verify it by hand and update DDS_SHA256 in xtask/src/dds.rs"
        )
        .into());
    }

    let out_dir = vendor.join(format!("dds-{DDS_VERSION}"));
    if out_dir.exists() {
        fs::remove_dir_all(&out_dir)?;
    }
    let (files, cpp) = extract(&archive, &out_dir)?;
    for required in ["src/dds.cpp", "include/dll.h", "LICENSE"] {
        if !out_dir.join(required).is_file() {
            return Err(format!(
                "{required} missing after extraction into {}",
                out_dir.display()
            )
            .into());
        }
    }
    if cpp != EXPECTED_CPP {
        eprintln!(
            "warning: expected {EXPECTED_CPP} src/*.cpp files, found {cpp}; check build.rs SOURCES"
        );
    }

    let sums = vendor.join("SHA256SUMS");
    fs::write(&sums, format!("{DDS_SHA256}  {archive_name}\n"))?;
    eprintln!(
        "extracted {files} files ({cpp} .cpp) into {}\nwrote {}",
        out_dir.display(),
        sums.display()
    );
    Ok(ExitCode::SUCCESS)
}

fn download(archive: &Path) -> Result<String> {
    let tmp = archive.with_extension("gz.part");
    let (sha256, bytes) = Fetcher::new().download(DDS_URL, &tmp)?;
    eprintln!("  {bytes} bytes, sha256 = {sha256}");
    fs::rename(&tmp, archive)?;
    Ok(sha256)
}

/// Extracts the wanted files of the `dds-<version>/` tree into `out_dir`.
/// Returns `(files extracted, of which src/*.cpp)`.
fn extract(archive: &Path, out_dir: &Path) -> Result<(usize, usize)> {
    let prefix = format!("dds-{DDS_VERSION}");
    let mut tar = tar::Archive::new(GzDecoder::new(File::open(archive)?));
    let mut files = 0;
    let mut cpp = 0;
    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?.into_owned();
        let Ok(relative) = path.strip_prefix(&prefix) else {
            continue; // pax headers and anything outside the tree
        };
        let Some(relative_str) = relative.to_str() else {
            continue;
        };
        if !wanted(relative_str) {
            continue;
        }
        let target = out_dir.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        io::copy(&mut entry, &mut File::create(&target)?)?;
        files += 1;
        if relative_str.ends_with(".cpp") {
            cpp += 1;
        }
    }
    Ok((files, cpp))
}

/// The layout `build.rs` expects: `src/*.cpp`, `src/*.h`, `include/dll.h`, `LICENSE`, `NOTICE`.
fn wanted(relative: &str) -> bool {
    match relative {
        "LICENSE" | "NOTICE" | "include/dll.h" => true,
        _ => relative
            .strip_prefix("src/")
            .is_some_and(|f| !f.contains('/') && (f.ends_with(".cpp") || f.ends_with(".h"))),
    }
}

#[cfg(test)]
mod tests {
    use super::wanted;

    #[test]
    fn wanted_matches_the_build_rs_layout() {
        for path in [
            "LICENSE",
            "NOTICE",
            "include/dll.h",
            "src/dds.cpp",
            "src/dds.h",
            "src/TransTableL.cpp",
        ] {
            assert!(wanted(path), "{path}");
        }
        for path in [
            "README.md",
            "include/other.h",
            "src/sub/x.cpp",
            "src/Makefile",
            "hands/list100.txt",
            "test/x.cpp",
        ] {
            assert!(!wanted(path), "{path}");
        }
    }
}
