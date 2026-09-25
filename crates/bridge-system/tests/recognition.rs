//! Description compiler recognition rate (`docs/design/11-testing.md` §1, §5.5): the mean
//! `Recognition.ratio` over every row of a corpus, reported per corpus in
//! `target/recognition_report.json`, with roadmap targets jdh8 >= 0.65, gpaulissen (`gjp`) >=
//! 0.50. `#[ignore]`d because it needs the vendored systems data; run with
//! `cargo test -p bridge-system --release -- --ignored recognition`.
//!
//! **Blocked today**: see `bss_oracle.rs`'s module doc -- `compile_description` is still
//! `todo!()`, so no row anywhere produces a real `Recognition` yet. The report is still written,
//! with every corpus at `rows: 0`, so the JSON shape and the threshold check are exercised now
//! and need no changes once the lane lands.

mod common;

use bridge_system::CompileOptions;

struct CorpusReport {
    name: &'static str,
    files: usize,
    blocked: usize,
    rows: usize,
    mean_ratio: f64,
    threshold: f64,
}

impl CorpusReport {
    fn to_json(&self) -> String {
        format!(
            concat!(
                "    {{\"corpus\": \"{}\", \"files\": {}, \"blocked\": {}, \"rows\": {}, ",
                "\"mean_recognition\": {:.4}, \"threshold\": {}}}"
            ),
            self.name, self.files, self.blocked, self.rows, self.mean_ratio, self.threshold
        )
    }
}

fn measure(dir: &std::path::Path, sub: &str, name: &'static str, threshold: f64) -> CorpusReport {
    let opts = CompileOptions::default();
    let files = common::bml_files(&dir.join(sub));
    let mut blocked = 0usize;
    let mut rows = 0usize;
    let mut ratio_sum = 0.0f64;

    for path in &files {
        let Some(ir) = common::compile_guarded(path, &opts) else {
            blocked += 1;
            continue;
        };
        for row in &ir.rows {
            rows += 1;
            ratio_sum += row.recognition.ratio as f64;
        }
    }

    CorpusReport {
        name,
        files: files.len(),
        blocked,
        rows,
        mean_ratio: if rows > 0 {
            ratio_sum / rows as f64
        } else {
            0.0
        },
        threshold,
    }
}

#[test]
#[ignore = "needs the vendored systems data; cargo test --release -- --ignored recognition"]
fn recognition_report() {
    let dir = common::systems_dir();
    if !dir.is_dir() {
        eprintln!("{} not found; skipping", dir.display());
        return;
    }

    let reports = [
        measure(&dir, "vendor/data/jdh8", "jdh8", 0.65),
        measure(&dir, "vendor/data/gjp", "gjp", 0.50),
    ];

    let json = format!(
        "{{\n  \"reports\": [\n{}\n  ]\n}}\n",
        reports
            .iter()
            .map(CorpusReport::to_json)
            .collect::<Vec<_>>()
            .join(",\n")
    );
    std::fs::create_dir_all("target").ok();
    std::fs::write("target/recognition_report.json", &json).expect("write recognition_report.json");
    eprintln!("{json}");

    let any_rows = reports.iter().any(|r| r.rows > 0);
    if !any_rows {
        eprintln!(
            "recognition: every corpus is fully blocked (compile_description still todo!()); \
             nothing to threshold-check yet"
        );
        return;
    }
    for r in &reports {
        if r.rows > 0 {
            assert!(
                r.mean_ratio >= r.threshold,
                "{}: mean recognition {:.4} < threshold {}",
                r.name,
                r.mean_ratio,
                r.threshold
            );
        }
    }
}
