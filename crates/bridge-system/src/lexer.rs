//! Source loading, `#INCLUDE` resolution and paragraph splitting.

use std::sync::Arc;

use crate::{
    Lint, LintCode,
    ast::{FileId, RawLine, Span},
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
        let base = std::path::Path::new(from)
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""));
        let full = base.join(path);
        std::fs::read_to_string(&full).map_err(|e| format!("{}: {e}", full.display()))
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
        let key = join_path(from, path);
        self.files
            .iter()
            .find(|(p, _)| *p == key)
            .map(|(_, t)| t.clone())
            .ok_or_else(|| format!("no such file: {key}"))
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

/// The root file id.
pub const ROOT: FileId = FileId(0);

/// `#INCLUDE` recursion is capped at this many nested levels (a cycle guard as much as a depth
/// limit: any real inclusion chain is far shallower).
const MAX_INCLUDE_DEPTH: u32 = 16;

/// Joins `path` (as written after `#INCLUDE`) relative to the directory of `base`, normalising
/// `.`/`..` segments. Purely textual (`/`-separated): used for bookkeeping (file ids, the cycle
/// guard) independent of whatever a [`SourceLoader`] does internally to actually resolve bytes.
fn join_path(base: &str, path: &str) -> String {
    if path.starts_with('/') {
        return normalize_path(path);
    }
    let dir_end = base.rfind('/').map_or(0, |i| i + 1);
    let mut combined = String::with_capacity(dir_end + path.len());
    combined.push_str(&base[..dir_end]);
    combined.push_str(path);
    normalize_path(&combined)
}

fn normalize_path(path: &str) -> String {
    // An empty split segment appears both for a leading `/` (absolute path) and for a doubled
    // `//` in the middle; the `"" | "."` arm below drops both the same way, which used to also
    // silently drop the leading slash of an absolute `root_path` (any real filesystem path,
    // typically -- `#INCLUDE` targets computed from it then read back as a *relative*-looking
    // path missing its leading `/`, corrupting every file-table entry downstream of the first
    // `#INCLUDE`). Preserve it explicitly instead.
    let absolute = path.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            seg => out.push(seg),
        }
    }
    let joined = out.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// Loads `root` and resolves includes recursively (cycle guard, depth ≤ 16).
pub fn load(root_path: &str, root_text: &str, loader: &dyn SourceLoader) -> Loaded {
    let mut files = Vec::new();
    let mut lints = Vec::new();
    let mut lines = Vec::new();
    let mut stack = Vec::new();
    load_file(
        root_path, root_text, loader, &mut files, &mut lints, &mut lines, &mut stack, 0,
    );
    Loaded {
        files,
        lines,
        lints,
    }
}

#[allow(clippy::too_many_arguments)]
fn load_file(
    resolved_path: &str,
    text: &str,
    loader: &dyn SourceLoader,
    files: &mut Vec<(Arc<str>, Arc<str>)>,
    lints: &mut Vec<Lint>,
    out: &mut Vec<RawLine>,
    stack: &mut Vec<String>,
    depth: u32,
) {
    let file_id = FileId(files.len() as u16);
    files.push((Arc::from(resolved_path), Arc::from(text)));
    stack.push(resolved_path.to_string());

    for (idx, raw_line) in text.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        if raw_line.starts_with("//") {
            // Column-0 comment: dropped entirely.
            continue;
        }
        if let Some(rest) = raw_line.strip_prefix("#INCLUDE") {
            let arg = rest.trim();
            let starts_with_ws = rest.starts_with(' ') || rest.starts_with('\t');
            if !arg.is_empty() && starts_with_ws {
                let span = Span {
                    file: file_id,
                    line: line_no,
                    col: 0,
                    pasted_from: None,
                };
                let target = join_path(resolved_path, arg);
                if depth + 1 > MAX_INCLUDE_DEPTH || stack.contains(&target) {
                    lints.push(
                        Lint::error(
                            LintCode::IncludeCycle,
                            format!(
                                "#INCLUDE {arg}: cycle or depth > {MAX_INCLUDE_DEPTH} ({target})"
                            ),
                        )
                        .with_span(span),
                    );
                    continue;
                }
                match loader.load(resolved_path, arg) {
                    Ok(included_text) => {
                        load_file(
                            &target,
                            &included_text,
                            loader,
                            files,
                            lints,
                            out,
                            stack,
                            depth + 1,
                        );
                    }
                    Err(e) => {
                        lints.push(
                            Lint::warning(
                                LintCode::IncludeNotFound,
                                format!("#INCLUDE {arg}: {e}"),
                            )
                            .with_span(span),
                        );
                    }
                }
                continue;
            }
        }
        out.push(RawLine {
            span: Span {
                file: file_id,
                line: line_no,
                col: 0,
                pasted_from: None,
            },
            text: raw_line.to_string(),
        });
    }
    stack.pop();
}

/// Groups lines into paragraphs separated by one or more blank (whitespace-only) lines.
pub fn paragraphs(lines: &[RawLine]) -> Vec<Vec<RawLine>> {
    let mut result = Vec::new();
    let mut current: Vec<RawLine> = Vec::new();
    for line in lines {
        if line.text.trim().is_empty() {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
        } else {
            current.push(line.clone());
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_load_no_includes() {
        let loaded = load(
            "root.bml",
            "1C  Any hand\n// comment\n1D  Other\n",
            &FsLoader,
        );
        assert_eq!(loaded.lines.len(), 2);
        assert!(loaded.lints.is_empty());
        assert_eq!(loaded.files.len(), 1);
    }

    #[test]
    fn include_is_spliced_in_place() {
        let mem = MemLoader {
            files: vec![("inc.bml".to_string(), "1D  Included\n".to_string())],
        };
        let root = "1C  Any hand\n#INCLUDE inc.bml\n1H  After\n";
        let loaded = load("root.bml", root, &mem);
        assert!(loaded.lints.is_empty(), "{:?}", loaded.lints);
        let texts: Vec<_> = loaded.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["1C  Any hand", "1D  Included", "1H  After"]);
        assert_eq!(loaded.files.len(), 2);
    }

    #[test]
    fn missing_include_is_warning_and_dropped() {
        let mem = MemLoader::default();
        let loaded = load("root.bml", "#INCLUDE missing.bml\n1C X\n", &mem);
        assert_eq!(loaded.lints.len(), 1);
        assert_eq!(loaded.lints[0].code, LintCode::IncludeNotFound);
        assert_eq!(loaded.lints[0].severity, crate::Severity::Warning);
        assert_eq!(loaded.lines.len(), 1);
    }

    #[test]
    fn include_cycle_is_error() {
        let mem = MemLoader {
            files: vec![("a.bml".to_string(), "#INCLUDE root.bml\n".to_string())],
        };
        let loaded = load("root.bml", "#INCLUDE a.bml\n", &mem);
        assert_eq!(loaded.lints.len(), 1);
        assert_eq!(loaded.lints[0].code, LintCode::IncludeCycle);
        assert_eq!(loaded.lints[0].severity, crate::Severity::Error);
    }

    // Regression: an absolute root path (the normal case for `FsLoader`, e.g. `compile_real`'s
    // vendored-file walk) used to have its `#INCLUDE`d files' resolved path silently lose its
    // leading `/` (`normalize_path`'s `""` arm, meant only to drop `//` and `.` segments,
    // previously dropped the leading-slash split artifact of an absolute path the same way).
    #[test]
    fn included_files_keep_the_root_paths_leading_slash() {
        let mem = MemLoader {
            files: vec![(
                "/vendor/data/jdh8/blue/1C.bml".to_string(),
                "1C  Included\n".to_string(),
            )],
        };
        let loaded = load(
            "/vendor/data/jdh8/blue.bml",
            "1C  Any hand\n#INCLUDE blue/1C.bml\n",
            &mem,
        );
        assert!(loaded.lints.is_empty(), "{:?}", loaded.lints);
        assert_eq!(loaded.files.len(), 2);
        assert_eq!(loaded.files[0].0.as_ref(), "/vendor/data/jdh8/blue.bml");
        assert_eq!(loaded.files[1].0.as_ref(), "/vendor/data/jdh8/blue/1C.bml");
    }

    #[test]
    fn paragraphs_split_on_blank_lines() {
        let loaded = load("root.bml", "a\nb\n\n\nc\n", &FsLoader);
        let paras = paragraphs(&loaded.lines);
        assert_eq!(paras.len(), 2);
        assert_eq!(paras[0].len(), 2);
        assert_eq!(paras[1].len(), 1);
    }
}
