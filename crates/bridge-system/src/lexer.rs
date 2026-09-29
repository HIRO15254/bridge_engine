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
    let path = path.replace('\\', "/");
    if path.starts_with('/') || drive_prefix(&path).is_some() {
        return normalize_path(&path);
    }
    let base = base.replace('\\', "/");
    let dir_end = base.rfind('/').map_or(0, |i| i + 1);
    let mut combined = String::with_capacity(dir_end + path.len());
    combined.push_str(&base[..dir_end]);
    combined.push_str(&path);
    normalize_path(&combined)
}

/// The `C:` drive prefix of a Windows path, if any.
fn drive_prefix(path: &str) -> Option<&str> {
    let bytes = path.as_bytes();
    (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':').then(|| &path[..2])
}

fn normalize_path(path: &str) -> String {
    // An empty split segment appears both for a leading `/` (absolute path) and for a doubled
    // `//` in the middle; the `"" | "."` arm below drops both the same way, which used to also
    // silently drop the leading slash of an absolute `root_path` (any real filesystem path,
    // typically -- `#INCLUDE` targets computed from it then read back as a *relative*-looking
    // path missing its leading `/`, corrupting every file-table entry downstream of the first
    // `#INCLUDE`). Preserve it explicitly instead.
    //
    // Windows paths (`Path::join` output such as `D:\repo\crates\x/../../systems/a.bml`)
    // mix `\` and `/`. Every separator is read as `/`, and a `C:` drive prefix is kept as the
    // root, so that `..` segments pop real directories instead of the whole backslashed prefix.
    let path = path.replace('\\', "/");
    let drive = drive_prefix(&path).unwrap_or("");
    let body = &path[drive.len()..];
    let absolute = body.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for seg in body.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                // Pop only a real segment: a leading `..` (nothing left to pop, or only other
                // `..`s so far) must survive, or `../x.bml` would silently become `x.bml`. Above
                // the root of an absolute path there is nothing to go to, so it is dropped there.
                match out.last() {
                    Some(&last) if last != ".." => {
                        out.pop();
                    }
                    _ if absolute => {}
                    _ => out.push(".."),
                }
            }
            seg => out.push(seg),
        }
    }
    let joined = out.join("/");
    if absolute {
        format!("{drive}/{joined}")
    } else {
        format!("{drive}{joined}")
    }
}

/// Loads `root` and resolves includes recursively (cycle guard, depth ≤ 16).
pub fn load(root_path: &str, root_text: &str, loader: &dyn SourceLoader) -> Loaded {
    let mut files = Vec::new();
    let mut lints = Vec::new();
    let mut lines = Vec::new();
    let mut stack = Vec::new();
    // Normalised like every `#INCLUDE` target, so a cycle back to the root is recognised
    // whatever spelling (`./root.bml`, `a/../root.bml`) the caller used for it.
    let root_path = normalize_path(root_path);
    load_file(
        &root_path, root_text, loader, &mut files, &mut lints, &mut lines, &mut stack, 0,
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
        if let Some(arg) = include_target(raw_line) {
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
                        format!("#INCLUDE {arg}: cycle or depth > {MAX_INCLUDE_DEPTH} ({target})"),
                    )
                    .with_span(span),
                );
                continue;
            }
            match loader.load(resolved_path, arg) {
                Ok(included_text) => {
                    // `bml.py` substitutes `'\n' + text + '\n'` for the directive, so an
                    // included file always starts and ends its own paragraph: without these
                    // blank lines two back-to-back `#INCLUDE`s would merge the last table of
                    // one file with the first paragraph of the next.
                    out.push(blank_line(span.clone()));
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
                    out.push(blank_line(span));
                }
                Err(e) => {
                    lints.push(
                        Lint::warning(LintCode::IncludeNotFound, format!("#INCLUDE {arg}: {e}"))
                            .with_span(span),
                    );
                }
            }
            continue;
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

/// The path argument of an `#INCLUDE` line, following `bml.py`'s `^\s*#\s*INCLUDE\s*(\S+)`:
/// leading whitespace and whitespace after `#` are allowed, and only the first word after the
/// keyword is the path (anything after it is ignored).
fn include_target(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('#')?;
    let rest = rest.trim_start().strip_prefix("INCLUDE")?;
    rest.split_whitespace().next()
}

/// A synthetic blank line (a paragraph break) attributed to the `#INCLUDE` line at `span`.
fn blank_line(span: Span) -> RawLine {
    RawLine {
        span,
        text: String::new(),
    }
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
        // The include is framed by two synthetic blank lines, as `bml.py` frames it with `\n`.
        assert_eq!(texts, ["1C  Any hand", "", "1D  Included", "", "1H  After"]);
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

    // Regression (integration review #0): `bml.py` splices every include as
    // `'\n' + text + '\n'`, so an included file always starts and ends a paragraph. Two
    // back-to-back `#INCLUDE`s must therefore yield two separate tables, never one merged one.
    #[test]
    fn back_to_back_includes_are_separate_paragraphs() {
        let mem = MemLoader {
            files: vec![
                (
                    "a.bml".to_string(),
                    "1C  clubs
1D  diamonds
"
                    .to_string(),
                ),
                (
                    "b.bml".to_string(),
                    "1H  hearts
1S  spades
"
                    .to_string(),
                ),
            ],
        };
        let loaded = load("root.bml", "#INCLUDE a.bml\n#INCLUDE b.bml\n", &mem);
        assert!(loaded.lints.is_empty(), "{:?}", loaded.lints);
        let paras = paragraphs(&loaded.lines);
        let texts: Vec<Vec<&str>> = paras
            .iter()
            .map(|p| p.iter().map(|l| l.text.as_str()).collect())
            .collect();
        assert_eq!(
            texts,
            vec![
                vec!["1C  clubs", "1D  diamonds"],
                vec!["1H  hearts", "1S  spades"]
            ]
        );
    }

    // Regression (integration review #0): `bml.py`'s `^\s*#\s*INCLUDE\s*(\S+)` also accepts an
    // indented `#INCLUDE` and `# INCLUDE`, and only the first word after it is the path.
    #[test]
    fn include_directive_accepts_bml_py_spellings() {
        let mem = MemLoader {
            files: vec![(
                "a.bml".to_string(),
                "1C  clubs
"
                .to_string(),
            )],
        };
        for root in [
            "  #INCLUDE a.bml
",
            "# INCLUDE a.bml
",
            "#\tINCLUDE\ta.bml   trailing words
",
        ] {
            let loaded = load("root.bml", root, &mem);
            assert!(loaded.lints.is_empty(), "{root:?}: {:?}", loaded.lints);
            let texts: Vec<_> = loaded
                .lines
                .iter()
                .map(|l| l.text.as_str())
                .filter(|t| !t.is_empty())
                .collect();
            assert_eq!(texts, ["1C  clubs"], "{root:?}");
        }
    }

    // Regression (integration review #1): a `..` that has nothing to pop must be kept, and the
    // root path is normalised before it is used as the cycle-guard/`from` key.
    #[test]
    fn normalize_keeps_unpoppable_parent_segments() {
        assert_eq!(normalize_path("../x.bml"), "../x.bml");
        assert_eq!(normalize_path("../../a/../x.bml"), "../../x.bml");
        assert_eq!(normalize_path("./a/./b/../x.bml"), "a/x.bml");
        assert_eq!(normalize_path("/../x.bml"), "/x.bml");
    }

    #[test]
    fn windows_paths_normalise_with_their_drive_as_root() {
        // What `Path::new(env!("CARGO_MANIFEST_DIR")).join("../../systems/sayc/sayc.bml")`
        // produces on Windows: the backslashed prefix used to be one segment, so both `..`
        // popped it away and the root became the relative `../systems/sayc/sayc.bml`.
        assert_eq!(
            normalize_path(r"D:\a\repo\crates\bridge-bidding/../../systems/sayc/sayc.bml"),
            "D:/a/repo/systems/sayc/sayc.bml"
        );
        assert_eq!(normalize_path(r"C:\..\x.bml"), "C:/x.bml");
        assert_eq!(normalize_path("C:a/../x.bml"), "C:x.bml");
        assert_eq!(
            join_path("D:/a/repo/systems/sayc/sayc.bml", "openings.bml"),
            "D:/a/repo/systems/sayc/openings.bml"
        );
        assert_eq!(join_path(r"D:\a\sayc.bml", r"sub\x.bml"), "D:/a/sub/x.bml");
        assert_eq!(join_path("a/b.bml", r"E:\x.bml"), "E:/x.bml");
        assert_eq!(join_path("../root.bml", "sub/a.bml"), "../sub/a.bml");
        assert_eq!(join_path("../sub/a.bml", "b.bml"), "../sub/b.bml");
    }

    #[test]
    fn cycle_back_to_an_unnormalised_root_is_detected() {
        let mem = MemLoader {
            files: vec![("a.bml".to_string(), "#INCLUDE root.bml\n".to_string())],
        };
        let loaded = load("./root.bml", "#INCLUDE a.bml\n", &mem);
        assert_eq!(loaded.lints.len(), 1, "{:?}", loaded.lints);
        assert_eq!(loaded.lints[0].code, LintCode::IncludeCycle);
    }

    // Regression (integration review #1): a two-level include below an absolute root path, and
    // below a root given as `../root.bml`, must resolve through the real file system.
    #[test]
    fn fs_loader_resolves_two_level_includes() {
        let dir = std::env::temp_dir().join(format!(
            "bridge-system-lexer-{}-{}",
            std::process::id(),
            line!()
        ));
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join("root.bml"), "#INCLUDE sub/a.bml\n").unwrap();
        std::fs::write(sub.join("a.bml"), "#INCLUDE b.bml\n").unwrap();
        std::fs::write(sub.join("b.bml"), "1C  clubs\n").unwrap();
        let root_text = "#INCLUDE sub/a.bml\n";

        let abs_root = dir.join("root.bml");
        let loaded = load(abs_root.to_str().unwrap(), root_text, &FsLoader);
        assert!(loaded.lints.is_empty(), "{:?}", loaded.lints);
        assert!(loaded.lines.iter().any(|l| l.text == "1C  clubs"));
        // `/tmp/...` on Unix, `C:/Users/...` on Windows.
        assert!(
            std::path::Path::new(&*loaded.files[2].0).is_absolute(),
            "{:?}",
            loaded.files[2].0
        );

        // `../<dir>/root.bml`, seen from `<dir>/sub`: expressed relative to `sub` by hand so
        // the test does not depend on (or change) the process's working directory.
        let rel_root = format!("{}/../root.bml", sub.to_str().unwrap());
        let loaded = load(&rel_root, root_text, &FsLoader);
        assert!(loaded.lints.is_empty(), "{:?}", loaded.lints);
        assert!(loaded.lines.iter().any(|l| l.text == "1C  clubs"));
        std::fs::remove_dir_all(&dir).unwrap();
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
