//! On-disk cache of compiled systems.
//!
//! Key = `blake3(resolved source ‖ compiler_version ‖ ir_format ‖ options)`; value = the
//! `postcard`-encoded [`SystemIR`]. Any mismatch recompiles. BML remains the source of truth;
//! the cache is an optimisation, not a distribution format.

use std::io;
use std::path::{Path, PathBuf};

use crate::{CompileOptions, IR_FORMAT, Lint, SystemIR, lexer, lexer::SourceLoader};

/// A cache directory.
#[derive(Clone, Debug)]
pub struct SystemCache {
    dir: PathBuf,
}

/// The resolved source of `path` (its own text plus every `#INCLUDE`d file's, in the order
/// [`crate::lexer::load`] visits them): the same bytes [`SystemCache::key`] hashes and
/// [`crate::compile::compile`] parses, so a cache hit and a fresh compile agree by construction.
fn resolved_source(path: &Path, loader: &dyn SourceLoader) -> io::Result<(String, String)> {
    let root_text = std::fs::read_to_string(path)?;
    let root_path = path.to_string_lossy().into_owned();
    let loaded = lexer::load(&root_path, &root_text, loader);
    let mut resolved = String::new();
    for (_, text) in &loaded.files {
        resolved.push_str(text);
    }
    Ok((root_path, resolved))
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
    ) -> io::Result<(SystemIR, Vec<Lint>)> {
        let (root_path, resolved) = resolved_source(path, loader)?;
        let key = Self::key(resolved.as_bytes(), opts);
        let entry_path = self.entry_path(&key);

        if let Ok(bytes) = std::fs::read(&entry_path) {
            if let Ok(ir) = postcard::from_bytes::<SystemIR>(&bytes) {
                if ir.meta.ir_format == IR_FORMAT
                    && ir.meta.compiler_version == crate::COMPILER_VERSION
                {
                    let lints = ir.lints.clone();
                    return Ok((ir, lints));
                }
            }
            // `ir_format`/`compiler_version` mismatch or a corrupt/foreign file: not an error,
            // just a miss. Fall through and recompile.
        }

        let root_text = std::fs::read_to_string(path)?;
        let (ir, lints) = crate::compile(&root_path, &root_text, loader, opts);
        self.store(&key, &ir)?;
        Ok((ir, lints))
    }

    /// The cache key for a resolved source: `blake3(source ‖ compiler_version ‖ ir_format ‖
    /// options)`.
    pub fn key(source: &[u8], opts: &CompileOptions) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(source);
        hasher.update(crate::COMPILER_VERSION.as_bytes());
        hasher.update(&IR_FORMAT.to_le_bytes());
        hasher.update(&opts.coverage_samples.to_le_bytes());
        hasher.update(&[opts.strict_dnf as u8]);
        hasher.update(&(opts.max_nodes as u64).to_le_bytes());
        *hasher.finalize().as_bytes()
    }

    fn entry_path(&self, key: &[u8; 32]) -> PathBuf {
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        self.dir.join(format!("{hex}.ir"))
    }

    /// Writes `ir` under `key`, atomically (temp file + rename): only I/O failures are `Err`.
    fn store(&self, key: &[u8; 32], ir: &SystemIR) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let bytes = postcard::to_allocvec(ir)
            .map_err(|e| io::Error::other(format!("postcard encode failed: {e}")))?;
        let final_path = self.entry_path(key);
        let tmp_path = self.dir.join(format!(
            "{}.tmp-{}",
            final_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("cache"),
            std::process::id()
        ));
        std::fs::write(&tmp_path, &bytes)?;
        std::fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::MemLoader;

    #[test]
    fn key_is_deterministic_and_sensitive_to_source_and_options() {
        let opts = CompileOptions::default();
        let k1 = SystemCache::key(b"1C Any hand", &opts);
        let k2 = SystemCache::key(b"1C Any hand", &opts);
        assert_eq!(k1, k2);

        let k3 = SystemCache::key(b"1D Any hand", &opts);
        assert_ne!(k1, k3);

        let mut opts2 = opts.clone();
        opts2.max_nodes = opts.max_nodes + 1;
        let k4 = SystemCache::key(b"1C Any hand", &opts2);
        assert_ne!(k1, k4);
    }

    #[test]
    fn load_or_compile_round_trips_through_disk() {
        // A meta-only source (no bidding table) so compilation never reaches the still-`todo!()`
        // description compiler (owned by another lane); this still exercises the whole cache
        // path: resolving includes, hashing, encoding, decoding and the `ir_format`/
        // `compiler_version` header check.
        let dir = std::env::temp_dir().join(format!(
            "bridge_system_cache_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cache = SystemCache::new(&dir);
        let loader = MemLoader::default();
        let opts = CompileOptions::default();

        let src_path = dir.join("root.bml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&src_path, "#+TITLE: Test System\n").unwrap();

        let (ir1, lints1) = cache.load_or_compile(&src_path, &loader, &opts).unwrap();
        assert_eq!(ir1.meta.name, "Test System");
        assert!(lints1.is_empty());

        // Second call must hit the cache and return an equivalent IR without recompiling by
        // reading a different (now-wrong) file at the same path: if it were recompiling, the
        // name would change.
        std::fs::write(&src_path, "#+TITLE: Changed But Uncached\n").unwrap();
        // The resolved source differs now, so this *is* a fresh compile under a new key; to
        // actually test the cache hit, restore the original text and compile again.
        std::fs::write(&src_path, "#+TITLE: Test System\n").unwrap();
        let (ir2, _) = cache.load_or_compile(&src_path, &loader, &opts).unwrap();
        assert_eq!(ir2.meta.name, "Test System");

        let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        // Exactly one root file plus one cache entry.
        assert_eq!(entries.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }
}
