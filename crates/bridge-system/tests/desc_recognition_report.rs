//! Provisional recognition-ratio report against the real BML corpus (`docs/design/06-system.md`
//! §11's completion criteria for task 3.3: jdh8 >= 0.65, gjp (gpaulissen) >= 0.5).
//!
//! The real BML front end (lexer/parser, `wip/bml-parser`) is still in progress in this phase,
//! so descriptions are pulled out with a crude, line-based heuristic instead of the real parser:
//! for each physical line, if it contains `=`, and the text before the first `=` looks like a
//! call token (non-empty, no spaces, at most 8 bytes), the text after that `=` is one
//! description. This under-counts real coverage (multi-line enumerations after a `one of:`
//! header are not stitched back together, and indentation-only nesting is not tracked), but is
//! enough to grow the v1 vocabulary against real files ahead of the real parser landing.
//!
//! Every description is compiled with a neutral [`RowContext`] (`Call::Pass`, `Role::Opener`, no
//! path information at all) — recognition of context-free tokens does not need it, and
//! context-dependent words simply fall back to their `assumed` defaults, which does not affect
//! `covered`/`total` (only `Recognition::assumed`, which this report does not use).
//!
//! Writes `target/recognition_report.json` (per-file and per-collection ratios) and asserts the
//! two thresholds above.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use bridge_core::{Call, Side as TableSide};
use bridge_system::compile::desc::{compile_description, context::RowContext};
use bridge_system::{Binding, Role, SystemMeta};
use serde::Serialize;

#[derive(Serialize)]
struct FileReport {
    path: String,
    covered: u32,
    total: u32,
    ratio: f64,
}

#[derive(Serialize)]
struct CollectionReport {
    name: String,
    covered: u32,
    total: u32,
    ratio: f64,
    files: Vec<FileReport>,
}

#[derive(Serialize)]
struct Report {
    collections: Vec<CollectionReport>,
    /// The most frequent unrecognised words across the whole corpus, most frequent first.
    top_unrecognized_words: Vec<(String, u32)>,
}

/// Text after the call token and an optional `=`, per line; see the module doc comment.
fn extract_descriptions(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let Some(eq) = trimmed.find('=') else {
            continue;
        };
        let (left, right) = trimmed.split_at(eq);
        let left = left.trim();
        if left.is_empty() || left.len() > 8 || left.contains(' ') || left.contains('*') {
            continue;
        }
        let desc = right[1..].trim();
        if !desc.is_empty() {
            out.push(desc.to_string());
        }
    }
    out
}

fn collect_bml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_bml_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "bml") {
            out.push(path);
        }
    }
}

fn neutral_ctx(binding: &Binding) -> RowContext<'_> {
    RowContext {
        call: Call::Pass,
        side: TableSide::NS,
        level: 0,
        is_jump: false,
        binding,
        hash_suit: None,
        own_prev: None,
        partner_last: None,
        their_last_bid: None,
        agreed_suit: None,
        role: Role::Opener,
        partner_hcp: None,
        own_hcp: None,
    }
}

fn report_collection(
    name: &str,
    dir: &Path,
    meta: &SystemMeta,
    word_freq: &mut BTreeMap<String, u32>,
) -> CollectionReport {
    let binding = Binding::default();
    let ctx = neutral_ctx(&binding);

    let mut paths = Vec::new();
    collect_bml_files(dir, &mut paths);
    paths.sort();

    let mut files = Vec::with_capacity(paths.len());
    let (mut covered_total, mut total_total) = (0u32, 0u32);

    for path in paths {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let (mut covered, mut total) = (0u32, 0u32);
        for desc in extract_descriptions(&text) {
            let compiled = compile_description(&desc, &ctx, meta);
            covered += u32::from(compiled.recognition.covered);
            total += u32::from(compiled.recognition.total);
            for &(start, end) in &compiled.recognition.unrecognized {
                let span = &compiled_text(&desc)[start as usize..end as usize];
                for word in span.split_whitespace() {
                    let word = word
                        .trim_matches(|c: char| {
                            matches!(c, ',' | ';' | '.' | '(' | ')' | ':' | '!' | '?')
                        })
                        .to_lowercase();
                    if !word.is_empty() {
                        *word_freq.entry(word).or_insert(0) += 1;
                    }
                }
            }
        }
        let ratio = if total == 0 {
            1.0
        } else {
            f64::from(covered) / f64::from(total)
        };
        let rel = path
            .strip_prefix(dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        files.push(FileReport {
            path: rel,
            covered,
            total,
            ratio,
        });
        covered_total += covered;
        total_total += total;
    }

    let ratio = if total_total == 0 {
        1.0
    } else {
        f64::from(covered_total) / f64::from(total_total)
    };
    CollectionReport {
        name: name.to_string(),
        covered: covered_total,
        total: total_total,
        ratio,
        files,
    }
}

/// Re-derives the normalised text a description compiled to, so an `unrecognized` byte span
/// (into that normalised text) can be sliced back out. Cheap enough for a one-off report.
fn compiled_text(desc: &str) -> String {
    bridge_system::compile::desc::normalize::normalize(desc).text
}

#[test]
fn recognition_report_against_the_bml_corpus() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("crates/bridge-system is two levels under the workspace root");
    let corpus_root = workspace_root.join("systems/vendor/data");
    let jdh8_dir = corpus_root.join("jdh8");
    let gjp_dir = corpus_root.join("gjp");

    if !jdh8_dir.is_dir() || !gjp_dir.is_dir() {
        eprintln!(
            "skipping recognition_report_against_the_bml_corpus: corpus not found at {}",
            corpus_root.display()
        );
        return;
    }

    let meta = SystemMeta::default();
    let mut word_freq: BTreeMap<String, u32> = BTreeMap::new();

    let jdh8 = report_collection("jdh8", &jdh8_dir, &meta, &mut word_freq);
    let gjp = report_collection("gjp", &gjp_dir, &meta, &mut word_freq);

    let mut top_words: Vec<(String, u32)> = word_freq.into_iter().collect();
    top_words.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    top_words.truncate(40);

    println!(
        "jdh8: ratio={:.3} covered={} total={}",
        jdh8.ratio, jdh8.covered, jdh8.total
    );
    println!(
        "gjp:  ratio={:.3} covered={} total={}",
        gjp.ratio, gjp.covered, gjp.total
    );
    println!("most frequent unrecognized words: {top_words:?}");

    let report = Report {
        collections: vec![jdh8, gjp],
        top_unrecognized_words: top_words,
    };

    let target_dir = workspace_root.join("target");
    fs::create_dir_all(&target_dir).expect("create target/ directory");
    let out_path = target_dir.join("recognition_report.json");
    let json = serde_json::to_string_pretty(&report).expect("Report serializes");
    fs::write(&out_path, json).expect("write target/recognition_report.json");

    let jdh8_report = &report.collections[0];
    let gjp_report = &report.collections[1];
    assert!(
        jdh8_report.ratio >= 0.65,
        "jdh8 recognition ratio {:.3} is below the 0.65 target (see {})",
        jdh8_report.ratio,
        out_path.display()
    );
    assert!(
        gjp_report.ratio >= 0.5,
        "gjp recognition ratio {:.3} is below the 0.5 target (see {})",
        gjp_report.ratio,
        out_path.display()
    );
}
