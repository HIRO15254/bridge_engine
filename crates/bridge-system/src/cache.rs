//! On-disk cache of compiled systems.
//!
//! Key = `blake3(resolved source ‖ compiler_version ‖ ir_format ‖ options)`; value = the
//! `postcard`-encoded [`SystemIR`]. Any mismatch recompiles. BML remains the source of truth;
//! the cache is an optimisation, not a distribution format.

use std::path::{Path, PathBuf};

use crate::{CompileOptions, Lint, SystemIR, lexer::SourceLoader};

/// A cache directory.
#[derive(Clone, Debug)]
pub struct SystemCache {
    dir: PathBuf,
}

impl SystemCache {
    /// Uses `dir` (created on first write).
    pub fn new(dir: impl Into<PathBuf>) -> SystemCache {
        SystemCache { dir: dir.into() }
    }

    /// Loads the cached IR for `path` if its key matches, otherwise compiles and stores it.
    /// Lints are stored with the IR.
    pub fn load_or_compile(
        &self,
        path: &Path,
        loader: &dyn SourceLoader,
        opts: &CompileOptions,
    ) -> std::io::Result<(SystemIR, Vec<Lint>)> {
        todo!("phase 3")
    }

    /// The cache key for a resolved source.
    pub fn key(source: &[u8], opts: &CompileOptions) -> [u8; 32] {
        todo!("phase 3")
    }
}
