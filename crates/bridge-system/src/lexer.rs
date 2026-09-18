//! Source loading, `#INCLUDE` resolution and paragraph splitting.

use std::sync::Arc;

use crate::{
    Lint,
    ast::{FileId, RawLine},
};

/// Resolves `#INCLUDE` paths to text. Abstracted so that includes work without `std::fs`
/// (for example in the browser).
pub trait SourceLoader {
    /// Loads the file at `path` relative to `from` (the including file's path).
    fn load(&self, from: &str, path: &str) -> Result<String, String>;
}

/// A loader backed by the file system.
#[derive(Clone, Copy, Debug, Default)]
pub struct FsLoader;

impl SourceLoader for FsLoader {
    fn load(&self, from: &str, path: &str) -> Result<String, String> {
        todo!("phase 3")
    }
}

/// A loader that serves in-memory sources (for tests and embedded systems).
#[derive(Clone, Debug, Default)]
pub struct MemLoader {
    /// `path → text`.
    pub files: Vec<(String, String)>,
}

impl SourceLoader for MemLoader {
    fn load(&self, from: &str, path: &str) -> Result<String, String> {
        todo!("phase 3")
    }
}

/// The loaded and flattened source.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// `(path, text)` per file, indexed by [`FileId`].
    pub files: Vec<(Arc<str>, Arc<str>)>,
    /// Every line in reading order, with column-0 `//` comment lines removed and includes
    /// spliced in place.
    pub lines: Vec<RawLine>,
    /// Missing or cyclic includes.
    pub lints: Vec<Lint>,
}

/// Loads `root` and resolves includes recursively (cycle guard, depth ≤ 16).
pub fn load(root_path: &str, root_text: &str, loader: &dyn SourceLoader) -> Loaded {
    todo!("phase 3")
}

/// Groups lines into paragraphs separated by one or more blank (whitespace-only) lines.
pub fn paragraphs(lines: &[RawLine]) -> Vec<Vec<RawLine>> {
    todo!("phase 3")
}

/// The root file id.
pub const ROOT: FileId = FileId(0);
