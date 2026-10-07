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

/// Reads `path` once and resolves every `#INCLUDE` it pulls in, returning `(root_path, root_text,
/// keyed_source)`: `root_text` is reused by the caller for [`crate::compile::compile`] itself (no
/// second read of `path`), and `keyed_source` is what [`SystemCache::key`] hashes -- `root_path`
/// followed by each resolved file's *own path* and length before its text, so that two files with
/// identical content at different paths (which would otherwise concatenate to the same bytes, and
/// so collide on one cache entry and report the wrong file's name) hash differently.
fn resolved_source(
    path: &Path,
    loader: &dyn SourceLoader,
) -> io::Result<(String, String, Vec<u8>)> {
    let root_text = std::fs::read_to_string(path)?;
    let root_path = path.to_string_lossy().into_owned();
    let loaded = lexer::load(&root_path, &root_text, loader);

    let mut keyed = Vec::new();
    keyed.extend_from_slice(root_path.as_bytes());
    keyed.push(0);
    for (file_path, text) in &loaded.files {
        keyed.extend_from_slice(file_path.as_bytes());
        keyed.push(0);
        keyed.extend_from_slice(&(text.len() as u64).to_le_bytes());
        keyed.extend_from_slice(text.as_bytes());
    }
    Ok((root_path, root_text, keyed))
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
        let (root_path, root_text, keyed_source) = resolved_source(path, loader)?;
        let key = Self::key(&keyed_source, opts);
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

        // `root_text` was already read above (for `resolved_source`'s own include resolution);
        // reusing it here means `path` is read from disk exactly once per call, so a concurrent
        // edit between the key computation and the compile can never store an IR under a key
        // computed from different bytes than the ones actually compiled.
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
    use crate::SystemMeta;
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

    fn temp_cache_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "bridge_system_cache_test_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn load_or_compile_round_trips_through_disk() {
        // A meta-only source (no bidding table): this exercises the whole cache path --
        // resolving includes, hashing, encoding, decoding and the `ir_format`/`compiler_version`
        // header check -- without depending on the description compiler at all.
        let dir = temp_cache_dir("roundtrip");
        let cache = SystemCache::new(&dir);
        let loader = MemLoader::default();
        let opts = CompileOptions::default();

        let src_path = dir.join("root.bml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&src_path, "#+TITLE: Test System\n").unwrap();

        let (ir1, lints1) = cache.load_or_compile(&src_path, &loader, &opts).unwrap();
        assert_eq!(ir1.meta.name, "Test System");
        assert!(lints1.is_empty());

        let (ir2, _) = cache.load_or_compile(&src_path, &loader, &opts).unwrap();
        assert_eq!(ir2.meta.name, "Test System");

        let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        // Exactly one root file plus one cache entry.
        assert_eq!(entries.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_or_compile_returns_the_cached_ir_on_a_hit_not_a_recompile() {
        // Plants a *sentinel* IR directly under the key `load_or_compile` would compute --
        // bypassing `compile()` entirely -- so the only way the next call can return it is a
        // genuine cache hit; a fresh compile of the source on disk would instead produce an IR
        // named "Test System", never the sentinel's name.
        let dir = temp_cache_dir("hit_proof");
        let cache = SystemCache::new(&dir);
        let loader = MemLoader::default();
        let opts = CompileOptions::default();

        let src_path = dir.join("root.bml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&src_path, "#+TITLE: Test System\n").unwrap();

        let (_, _, keyed_source) = resolved_source(&src_path, &loader).unwrap();
        let key = SystemCache::key(&keyed_source, &opts);

        let mut sentinel = SystemIR {
            meta: SystemMeta::default(),
            rows: Vec::new(),
            nodes: Vec::new(),
            index: crate::trie::AuctionTrie::new(),
            lints: Vec::new(),
        };
        sentinel.meta.name = "SENTINEL: planted directly, never compiled".to_string();
        sentinel.meta.ir_format = IR_FORMAT;
        sentinel.meta.compiler_version = crate::COMPILER_VERSION.to_string();
        cache.store(&key, &sentinel).unwrap();

        let (ir, _) = cache.load_or_compile(&src_path, &loader, &opts).unwrap();
        assert_eq!(ir.meta.name, sentinel.meta.name);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn key_differs_for_same_content_at_a_different_root_path() {
        // Two roots with byte-for-byte identical resolved content must still key differently:
        // otherwise they would share one cache entry and the second's IR would report the
        // first's file name.
        let dir = temp_cache_dir("path_sensitive");
        std::fs::create_dir_all(&dir).unwrap();
        let path_a = dir.join("a.bml");
        let path_b = dir.join("b.bml");
        std::fs::write(&path_a, "#+TITLE: Same Text\n").unwrap();
        std::fs::write(&path_b, "#+TITLE: Same Text\n").unwrap();
        let loader = MemLoader::default();
        let opts = CompileOptions::default();

        let (_, _, keyed_a) = resolved_source(&path_a, &loader).unwrap();
        let (_, _, keyed_b) = resolved_source(&path_b, &loader).unwrap();
        assert_ne!(
            SystemCache::key(&keyed_a, &opts),
            SystemCache::key(&keyed_b, &opts)
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
