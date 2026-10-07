//! Executable checks for `docs/design/16-extended-bml.md`, the extended-BML reference.
//!
//! * Every fenced block tagged `bml` compiles with zero `Error` and zero `Warning` lints
//!   (a plain example must be clean; `Info` lints are allowed).
//! * A block tagged `bml,should-lint` names the lint it demonstrates on its first line
//!   (`// expect-lint: <Code>`, a column-0 comment the compiler drops); that lint must be
//!   produced, and no `Error` of another code may appear.
//! * A block tagged `bml,file=<path>` is also registered under `<path>` in the in-memory
//!   loader, so other blocks can `#INCLUDE` it.
//! * The reference stays complete: every `LintCode` variant, every `#+KEY:` meta key the
//!   compiler understands and every phrase of the description vocabulary tables appears in it.
//!
//! `BML_REFERENCE_VERBOSE=1 cargo test -p bridge-system --test bml_reference -- --nocapture`
//! prints every lint of every block.

use bridge_system::lexer::MemLoader;
use bridge_system::{CompileOptions, Severity, compile};

const DOC: &str = include_str!("../../../docs/design/16-extended-bml.md");
const LINT_RS: &str = include_str!("../src/lint.rs");
const META_RS: &str = include_str!("../src/compile/meta.rs");
const VOCAB_RS: [(&str, &str); 3] = [
    ("tokens.rs", include_str!("../src/compile/desc/tokens.rs")),
    ("clause.rs", include_str!("../src/compile/desc/clause.rs")),
    (
        "recognition.rs",
        include_str!("../src/compile/desc/recognition.rs"),
    ),
];

/// One fenced `bml` block of the reference.
struct Block {
    /// 1-based line of the opening fence.
    line: usize,
    should_lint: bool,
    file: Option<String>,
    text: String,
}

fn blocks() -> Vec<Block> {
    let mut out = Vec::new();
    let mut lines = DOC.lines().enumerate();
    while let Some((idx, line)) = lines.next() {
        let Some(info) = line.strip_prefix("```") else {
            continue;
        };
        let info = info.trim();
        let mut attrs = info.split(',').map(str::trim);
        let is_bml = attrs.next() == Some("bml");
        let mut should_lint = false;
        let mut file = None;
        for attr in attrs {
            if attr == "should-lint" {
                should_lint = true;
            } else if let Some(path) = attr.strip_prefix("file=") {
                file = Some(path.to_string());
            } else {
                panic!("line {}: unknown bml block attribute {attr:?}", idx + 1);
            }
        }
        let mut text = String::new();
        for (_, body) in lines.by_ref() {
            if body.starts_with("```") {
                break;
            }
            text.push_str(body);
            text.push('\n');
        }
        if is_bml {
            out.push(Block {
                line: idx + 1,
                should_lint,
                file,
                text,
            });
        }
    }
    out
}

fn options() -> CompileOptions {
    CompileOptions {
        coverage_samples: 0,
        ..CompileOptions::default()
    }
}

#[test]
fn every_bml_block_compiles_without_errors() {
    let blocks = blocks();
    assert!(blocks.len() >= 60, "only {} bml blocks found", blocks.len());
    let loader = MemLoader {
        files: blocks
            .iter()
            .filter_map(|b| b.file.clone().map(|f| (f, b.text.clone())))
            .collect(),
    };
    // `BML_REFERENCE_VERBOSE=1 cargo test --test bml_reference -- --nocapture` prints every
    // lint of every block, to check the non-error lints the prose describes.
    let verbose = std::env::var_os("BML_REFERENCE_VERBOSE").is_some();
    let mut failures = Vec::new();
    for block in &blocks {
        let root = block.file.as_deref().unwrap_or("root.bml");
        let (_, lints) = compile(root, &block.text, &loader, &options());
        if verbose {
            println!("--- block at line {} ({root})", block.line);
            for lint in &lints {
                println!("    {lint}");
            }
        }
        let expected = if block.should_lint {
            let first = block.text.lines().next().unwrap_or_default();
            match first.strip_prefix("// expect-lint:") {
                Some(code) => Some(code.trim().to_string()),
                None => {
                    failures.push(format!(
                        "line {}: a should-lint block must start with `// expect-lint: <Code>`",
                        block.line
                    ));
                    continue;
                }
            }
        } else {
            None
        };
        let errors: Vec<String> = lints
            .iter()
            .filter(|l| l.severity == Severity::Error)
            .filter(|l| expected.as_deref() != Some(format!("{:?}", l.code).as_str()))
            .map(ToString::to_string)
            .collect();
        if !errors.is_empty() {
            failures.push(format!(
                "line {}: unexpected errors:\n    {}",
                block.line,
                errors.join("\n    ")
            ));
        }
        if expected.is_none() {
            let warnings: Vec<String> = lints
                .iter()
                .filter(|l| l.severity == Severity::Warning)
                .map(ToString::to_string)
                .collect();
            if !warnings.is_empty() {
                failures.push(format!(
                    "line {}: a plain bml block must not produce warnings (tag it \
                     `bml,should-lint` if it demonstrates one):\n    {}",
                    block.line,
                    warnings.join("\n    ")
                ));
            }
        }
        if let Some(code) = expected {
            if !lints.iter().any(|l| format!("{:?}", l.code) == code) {
                let got: Vec<String> = lints.iter().map(ToString::to_string).collect();
                failures.push(format!(
                    "line {}: expected lint {code} not produced; got:\n    {}",
                    block.line,
                    got.join("\n    ")
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every `LintCode` variant (parsed from the enum in `lint.rs`) is documented.
#[test]
fn every_lint_code_is_documented() {
    let start = LINT_RS.find("pub enum LintCode {").expect("LintCode enum");
    let body = &LINT_RS[start..];
    let body = &body[..body.find("\n}").expect("end of LintCode")];
    let mut missing = Vec::new();
    let mut count = 0;
    for line in body.lines().skip(1) {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('#') || line.is_empty() {
            continue;
        }
        let name = line.trim_end_matches(',');
        if name.chars().all(|c| c.is_ascii_alphanumeric()) {
            count += 1;
            if !DOC.contains(&format!("`{name}`")) {
                missing.push(name.to_string());
            }
        }
    }
    assert!(count >= 30, "parsed only {count} LintCode variants");
    assert!(missing.is_empty(), "undocumented lint codes: {missing:?}");
}

/// Every meta key matched in `compile/meta.rs` is documented as `#+KEY:`.
#[test]
fn every_meta_key_is_documented() {
    let mut missing = Vec::new();
    let mut count = 0;
    for line in META_RS.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('"') else {
            continue;
        };
        let Some((key, tail)) = rest.split_once('"') else {
            continue;
        };
        if !tail.trim_start().starts_with("=>")
            || key.is_empty()
            || !key.chars().all(|c| c.is_ascii_uppercase())
        {
            continue;
        }
        count += 1;
        if !DOC.contains(&format!("#+{key}:")) {
            missing.push(key.to_string());
        }
    }
    assert!(count >= 10, "parsed only {count} meta keys");
    assert!(missing.is_empty(), "undocumented meta keys: {missing:?}");
}

/// The string literals of every `const NAME: &[...] = &[...];` table in `src`, per table.
fn phrase_tables(src: &str) -> Vec<(String, Vec<String>)> {
    let src = src.split("#[cfg(test)]").next().unwrap_or(src);
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(pos) = rest.find("const ") {
        rest = &rest[pos + "const ".len()..];
        let Some(colon) = rest.find(':') else { break };
        let name = rest[..colon].trim().to_string();
        let decl = &rest[colon..];
        let Some(eq) = decl.find('=') else { break };
        if !decl[..eq].trim_start_matches(':').trim().starts_with("&[") {
            continue;
        }
        let body = &decl[eq..];
        let Some(end) = body.find("];") else { break };
        let body = &body[..end];
        let mut phrases = Vec::new();
        let mut parts = body.split('"');
        parts.next();
        while let Some(lit) = parts.next() {
            phrases.push(lit.to_string());
            parts.next();
        }
        if !phrases.is_empty() {
            out.push((name, phrases));
        }
    }
    out
}

/// Every phrase of the description vocabulary tables appears, in backticks, in the reference.
#[test]
fn every_vocabulary_phrase_is_documented() {
    let mut missing = Vec::new();
    let mut count = 0;
    for (file, src) in VOCAB_RS {
        for (table, phrases) in phrase_tables(src) {
            for phrase in phrases {
                count += 1;
                if !DOC.contains(&format!("`{phrase}`")) {
                    missing.push(format!("{file}/{table}: {phrase:?}"));
                }
            }
        }
    }
    assert!(count >= 150, "parsed only {count} vocabulary phrases");
    assert!(
        missing.is_empty(),
        "undocumented vocabulary:\n  {}",
        missing.join("\n  ")
    );
}
