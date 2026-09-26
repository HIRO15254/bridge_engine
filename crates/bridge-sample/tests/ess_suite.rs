//! Phase 5.3's ESS suite (`09-sample.md` §9, `11-testing.md` row `uniform_vs_constraint_ess`,
//! `12-roadmap.md` task 5.3): 50 real auctions, each interpreted with SAYC and sampled with the
//! auction's own bidding likelihood (`SampleContext::bidding = Some`), `n = 1000` deals per
//! auction for both `UniformProposal` and `ConstraintProposal`. Criterion: the median
//! `ConstraintProposal` ESS is at least `0.5 n`; `UniformProposal` is reported for comparison.
//!
//! The 50 auctions:
//!
//! - 25 generated: random deals (fixed seeds, dealer rotating N/E/S/W, no one vulnerable) bid
//!   out with `bridge_bidding::replay` using SAYC at all four seats, until the auction ends.
//!   Passed-out deals are skipped (there is no auction to learn from).
//! - 25 corpus: complete auctions from the PBN tournament records under `BRIDGE_CORPUS_DIR`
//!   (default `<workspace>/corpus/data`), parsed with `bridge-format`, 25 picked at evenly spaced
//!   positions in file order. These were bid by humans with their own systems, so they are
//!   interpreted with SAYC as an off-system approximation. If the corpus is absent this half is
//!   skipped (said so on stderr and in the report) and the criterion is checked on the generated
//!   half alone.
//!
//! Known cards: the opening leader's hand (declarer's left-hand opponent), as in the lead
//! problem this sampler serves (phase 6, `14-lead.md`); the other three hands are sampled.
//! Setting `ESS_SUITE_KNOWN=none` samples all four hands instead (reported, not the criterion).
//!
//! `ESS_SUITE_CASES=a..b` runs only case indices `a..b` (0-24 generated, 25-49 corpus) so the
//! suite can be split into shorter runs; the criterion is only asserted when all cases ran.
//!
//! Writes `target/ess_report.json`. Run in release:
//! `cargo test --release -p bridge-sample --all-features --test ess_suite -- --ignored --nocapture`

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bridge_bidding::{
    BidContext, ImplicitPass, InterpretOptions, PolicyParams, Scoring, Table, interpret, replay,
};
use bridge_core::{Auction, Card, Deal, Hand, Seat, Vulnerability};
use bridge_sample::{
    BiddingLikelihood, ConstraintProposal, KnownCards, Proposal, SampleContext, SampleOptions,
    Threads, UniformProposal, rng_for, sample_deals,
};

const N: usize = 1000;
const GENERATED: usize = 25;
const CORPUS: usize = 25;
/// Master seed of the random deals the generated auctions are bid from.
const DEAL_SEED: u64 = 0x5A7C_0005_0003;
/// Master seed of every `sample_deals` call.
const SAMPLE_SEED: u64 = 0xE55;

fn workspace_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn compile_sayc() -> bridge_system::SystemIR {
    let path = workspace_dir().join("systems/sayc/sayc.bml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let (ir, lints) = bridge_system::compile(
        &path.to_string_lossy(),
        &text,
        &bridge_system::lexer::FsLoader,
        &bridge_system::CompileOptions::default(),
    );
    let errors = lints
        .iter()
        .filter(|l| l.severity == bridge_system::Severity::Error)
        .count();
    assert_eq!(errors, 0, "sayc.bml compiled with {errors} error lint(s)");
    ir
}

fn bid_ctx() -> BidContext<'static> {
    BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::default(),
    }
}

/// A uniformly random deal from `rng_for(DEAL_SEED, i)` (Fisher-Yates over the 52 cards).
fn random_deal(i: u64) -> Deal {
    use rand_core::Rng;
    let mut rng = rng_for(DEAL_SEED, i);
    let mut cards: Vec<Card> = Hand::FULL.cards().collect();
    for k in (1..cards.len()).rev() {
        let j = (rng.next_u64() % (k as u64 + 1)) as usize;
        cards.swap(k, j);
    }
    let mut hands = [Hand::EMPTY; 4];
    for (hand, chunk) in hands.iter_mut().zip(cards.chunks(13)) {
        for &card in chunk {
            *hand = hand.with(card);
        }
    }
    Deal::new(hands).expect("52 cards in four 13-card hands")
}

struct Case {
    label: String,
    source: &'static str,
    deal: Deal,
    auction: Auction,
}

fn auction_text(auction: &Auction) -> String {
    auction
        .calls()
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn generated_cases(table: &Table) -> Vec<Case> {
    let ctx = bid_ctx();
    let mut out = Vec::new();
    let mut i = 0u64;
    while out.len() < GENERATED {
        let deal = random_deal(i);
        let dealer = Seat::ALL[(i % 4) as usize];
        let replayed = replay(table, &deal, dealer, Vulnerability::None, &ctx);
        if !replayed.auction.is_passed_out() && replayed.auction.contract().is_some() {
            out.push(Case {
                label: format!("gen-{i}"),
                source: "generated",
                deal,
                auction: replayed.auction,
            });
        }
        i += 1;
        assert!(
            i < 10_000,
            "could not find {GENERATED} non-passed-out deals"
        );
    }
    out
}

fn corpus_dir() -> Option<PathBuf> {
    let dir = match std::env::var_os("BRIDGE_CORPUS_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => workspace_dir().join("corpus/data"),
    };
    dir.is_dir().then_some(dir)
}

fn pbn_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(pbn_files(&path));
        } else if path.extension().is_some_and(|e| e == "pbn") {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// Every complete, not-passed-out auction with a complete deal in the corpus's PBN files, in
/// file order; then `CORPUS` of them at evenly spaced positions.
fn corpus_cases(dir: &Path) -> Vec<Case> {
    let mut all = Vec::new();
    for path in pbn_files(&dir.join("pbn")) {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let (file, _warnings) = bridge_format::pbn::parse_lenient(&bytes);
        let mut previous: Option<bridge_format::GameView> = None;
        for (k, game) in file.games.iter().enumerate() {
            let Ok(view) = game.view(previous.as_ref()) else {
                continue;
            };
            let deal = view.deal.as_ref().and_then(|d| d.complete());
            if let (Some(deal), Some(auction)) = (deal, view.auction.clone()) {
                if auction.is_complete() && auction.contract().is_some() {
                    let name = path
                        .file_stem()
                        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                    all.push(Case {
                        label: format!("{name}#{k}"),
                        source: "corpus",
                        deal,
                        auction,
                    });
                }
            }
            previous = Some(view);
        }
    }
    if all.len() <= CORPUS {
        return all;
    }
    let step = all.len() / CORPUS;
    all.into_iter().step_by(step).take(CORPUS).collect()
}

struct Row {
    label: String,
    source: &'static str,
    auction: String,
    viewer: Option<Seat>,
    uniform_ess_ratio: f64,
    constraint_ess_ratio: f64,
    constraint_produced: usize,
    constraint_acceptance: f64,
}

fn run_case(table: &Table, case: &Case, known_none: bool) -> Row {
    let interp = interpret(table, &case.auction, &InterpretOptions::default());
    let bctx = bid_ctx();
    let contract = case.auction.contract().expect("cases have a contract");
    let viewer = (!known_none).then(|| contract.leader());
    let known = match viewer {
        Some(seat) => KnownCards::from_viewer(seat, case.deal.hand(seat)),
        None => KnownCards::EMPTY,
    };
    let play_constraints = [
        bridge_constraint::HandConstraint::ANY,
        bridge_constraint::HandConstraint::ANY,
        bridge_constraint::HandConstraint::ANY,
        bridge_constraint::HandConstraint::ANY,
    ];
    let ctx = SampleContext {
        known,
        interpretation: &interp,
        play_constraints: &play_constraints,
        play_soft: None,
        bidding: Some(BiddingLikelihood {
            table,
            auction: &case.auction,
            ctx: &bctx,
        }),
    };
    let opts = SampleOptions {
        seed: SAMPLE_SEED,
        threads: Threads::Auto,
        ..SampleOptions::default()
    };
    let run = |proposal: &dyn Proposal| {
        sample_deals(&ctx, proposal, N, &opts)
            .unwrap_or_else(|e| panic!("{}: sampling failed: {e}", case.label))
            .1
    };
    let uniform = run(&UniformProposal);
    let constraint = run(&ConstraintProposal::default());
    Row {
        label: case.label.clone(),
        source: case.source,
        auction: auction_text(&case.auction),
        viewer,
        uniform_ess_ratio: uniform.ess_ratio,
        constraint_ess_ratio: constraint.ess_ratio,
        constraint_produced: constraint.produced,
        constraint_acceptance: constraint.acceptance_rate,
    }
}

fn median(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).expect("finite ESS ratios"));
    let m = v.len() / 2;
    if v.len() % 2 == 1 {
        v[m]
    } else {
        0.5 * (v[m - 1] + v[m])
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_f64(x: f64) -> String {
    if x.is_finite() {
        format!("{x}")
    } else {
        "null".to_string()
    }
}

fn case_range() -> core::ops::Range<usize> {
    let total = GENERATED + CORPUS;
    let Ok(spec) = std::env::var("ESS_SUITE_CASES") else {
        return 0..total;
    };
    let (a, b) = spec
        .split_once("..")
        .unwrap_or_else(|| panic!("ESS_SUITE_CASES must be a..b, got {spec:?}"));
    let a: usize = a.trim().parse().expect("ESS_SUITE_CASES start");
    let b: usize = b.trim().parse().expect("ESS_SUITE_CASES end");
    a..b.min(total)
}

#[test]
#[ignore = "statistical benchmark over 50 auctions; run in release on demand"]
fn uniform_vs_constraint_ess_suite() {
    let table = Table::uniform(
        Arc::new(compile_sayc()),
        Arc::new(bridge_bidding::NaturalInference::default()),
    );
    let known_none = std::env::var("ESS_SUITE_KNOWN").is_ok_and(|v| v == "none");
    let range = case_range();

    let mut cases = generated_cases(&table);
    let corpus_note = match corpus_dir() {
        Some(dir) => {
            let found = corpus_cases(&dir);
            let note = format!("{} corpus auctions from {}", found.len(), dir.display());
            cases.extend(found);
            note
        }
        None => {
            let note = "corpus directory not found (BRIDGE_CORPUS_DIR or corpus/data): corpus \
                        half skipped, criterion checked on the generated auctions only"
                .to_string();
            eprintln!("{note}");
            note
        }
    };
    let total_cases = cases.len();

    let mut rows = Vec::new();
    for (idx, case) in cases.iter().enumerate() {
        if !range.contains(&idx) {
            continue;
        }
        let row = run_case(&table, case, known_none);
        println!(
            "{idx:>2} {:<9} {:<22} uniform {:>7.4} constraint {:>7.4} (produced {}, acc {:.3})  {}",
            row.source,
            row.label,
            row.uniform_ess_ratio,
            row.constraint_ess_ratio,
            row.constraint_produced,
            row.constraint_acceptance,
            row.auction
        );
        rows.push(row);
    }

    let uniform: Vec<f64> = rows.iter().map(|r| r.uniform_ess_ratio).collect();
    let constraint: Vec<f64> = rows.iter().map(|r| r.constraint_ess_ratio).collect();
    let median_uniform = median(&uniform);
    let median_constraint = median(&constraint);
    let by_source = |source: &str| -> f64 {
        let xs: Vec<f64> = rows
            .iter()
            .filter(|r| r.source == source)
            .map(|r| r.constraint_ess_ratio)
            .collect();
        median(&xs)
    };
    let median_generated = by_source("generated");
    let median_corpus = by_source("corpus");
    println!(
        "{} auctions ({corpus_note}); median ESS/n: uniform {median_uniform:.4}, constraint \
         {median_constraint:.4} (generated {median_generated:.4}, corpus {median_corpus:.4})",
        rows.len()
    );

    let mut json = String::new();
    json.push_str("{\n");
    let _ = writeln!(json, "  \"n\": {N},");
    let _ = writeln!(
        json,
        "  \"known\": {},",
        json_str(if known_none { "none" } else { "opening_leader" })
    );
    let _ = writeln!(json, "  \"corpus\": {},", json_str(&corpus_note));
    let _ = writeln!(json, "  \"cases_run\": {},", rows.len());
    let _ = writeln!(json, "  \"cases_total\": {total_cases},");
    let _ = writeln!(
        json,
        "  \"median_uniform_ess_ratio\": {},",
        json_f64(median_uniform)
    );
    let _ = writeln!(
        json,
        "  \"median_constraint_ess_ratio\": {},",
        json_f64(median_constraint)
    );
    let _ = writeln!(
        json,
        "  \"median_constraint_ess_ratio_generated\": {},",
        json_f64(median_generated)
    );
    let _ = writeln!(
        json,
        "  \"median_constraint_ess_ratio_corpus\": {},",
        json_f64(median_corpus)
    );
    json.push_str("  \"auctions\": [\n");
    for (i, r) in rows.iter().enumerate() {
        let viewer = r
            .viewer
            .map_or_else(|| "null".to_string(), |s| json_str(&format!("{s:?}")));
        let _ = write!(
            json,
            "    {{\"label\": {}, \"source\": {}, \"auction\": {}, \"viewer\": {viewer}, \
             \"uniform_ess_ratio\": {}, \"constraint_ess_ratio\": {}, \
             \"constraint_produced\": {}, \"constraint_acceptance_rate\": {}}}",
            json_str(&r.label),
            json_str(r.source),
            json_str(&r.auction),
            json_f64(r.uniform_ess_ratio),
            json_f64(r.constraint_ess_ratio),
            r.constraint_produced,
            json_f64(r.constraint_acceptance),
        );
        json.push_str(if i + 1 < rows.len() { ",\n" } else { "\n" });
    }
    json.push_str("  ]\n}\n");
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace_dir().join("target"), PathBuf::from);
    let _ = std::fs::create_dir_all(&target);
    let path = target.join("ess_report.json");
    std::fs::write(&path, json).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
    println!("wrote {}", path.display());

    if rows.len() == total_cases && !known_none {
        assert!(
            median_constraint >= 0.5,
            "median ConstraintProposal ESS/n = {median_constraint:.4} < 0.5"
        );
    }
}
