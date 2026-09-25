//! Corpus evaluation harness (roadmap phase 6.2, `docs/design/14-lead.md` §4).
//!
//! `#[ignore]`: this needs `bridge_system::compile` and `bridge_sample::ConstraintProposal`,
//! both `todo!()` on this lane's base (owned by other lanes; see the crate root doc). **This
//! file is written to compile now and to be run to completion later** by whichever lane lands
//! last, with:
//!
//! ```text
//! BRIDGE_CORPUS_DIR=corpus/data cargo test -p bridge-lead --release --features dds -- --ignored corpus_eval
//! ```
//!
//! Environment variables: `LEAD_SAMPLES` (default 100, samples per board for the real harness),
//! `LEAD_UNIFORM=1` (use [`UniformProposal`] instead of [`ConstraintProposal`] for the main
//! evaluation, not just the baseline).
#![cfg(feature = "dds")]

mod common;

use std::path::{Path, PathBuf};
use std::time::Instant;

use bridge::system::lexer::FsLoader;
use bridge::system::{CompileOptions, NaturalInference};
use bridge_bidding::{Interpretation, Table};
use bridge_core::{Auction, Card, Contract, Deal};
use bridge_format::pbn;
use bridge_lead::{LeadOptions, LeadQuery, advise};
use bridge_sample::{
    ConstraintProposal, KnownCards, Proposal, SampleContext, SampleOptions, UniformProposal,
    sample_deals,
};

/// `BRIDGE_CORPUS_DIR`, or `<workspace>/corpus/data`; `None` when the directory is absent
/// (`crates/bridge-format/tests/common/mod.rs`'s convention).
fn corpus_dir() -> Option<PathBuf> {
    let dir = match std::env::var_os("BRIDGE_CORPUS_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/data"),
    };
    if dir.is_dir() { Some(dir) } else { None }
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

struct Board {
    label: String,
    auction: Auction,
    deal: Deal,
    contract: Contract,
}

/// The first `limit` boards, in deterministic file order, with a complete non-passed-out
/// auction, a full deal and a contract.
fn select_boards(corpus_dir: &Path, limit: usize) -> Vec<Board> {
    let mut boards = Vec::new();
    'files: for path in pbn_files(&corpus_dir.join("pbn")) {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let (file, _warnings) = pbn::parse_lenient(&bytes);
        let mut previous = None;
        for game in &file.games {
            let Ok(view) = game.view(previous.as_ref()) else {
                continue;
            };
            if let (Some(auction), Some(contract), Some(partial)) =
                (&view.auction, view.contract, view.deal)
            {
                if auction.is_complete() && !auction.is_passed_out() {
                    if let Some(deal) = partial.complete() {
                        boards.push(Board {
                            label: format!(
                                "{}#{}",
                                path.display(),
                                view.board.map(|b| b.to_string()).unwrap_or_default()
                            ),
                            auction: auction.clone(),
                            deal,
                            contract,
                        });
                        if boards.len() >= limit {
                            break 'files;
                        }
                    }
                }
            }
            previous = Some(view);
        }
    }
    boards
}

/// The real deal's DD lead scores, the maximum among them, and the (single, by construction)
/// distinct-score class of cards achieving it: the DD-optimal leads.
struct Truth {
    all_scores: Vec<(Card, u8)>,
    max: u8,
    cards: Vec<Card>,
}

fn dd_truth(dd: &dyn bridge::dd::DoubleDummy, board: &Board) -> Result<Truth, bridge::dd::DdError> {
    let all_scores = dd.lead_scores(
        &board.deal,
        board.contract.bid.strain(),
        board.contract.leader(),
    )?;
    let max = all_scores.iter().map(|&(_, s)| s).max().unwrap_or(0);
    let cards = all_scores
        .iter()
        .filter(|&&(_, s)| s == max)
        .map(|&(c, _)| c)
        .collect();
    Ok(Truth {
        all_scores,
        max,
        cards,
    })
}

/// `C(n, k)` for the small values this harness needs (`n <= 13`).
fn binomial(n: u64, k: u64) -> u64 {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut result = 1u64;
    for i in 0..k {
        result = result * (n - i) / (i + 1);
    }
    result
}

/// Baseline (b): the expected top-3 hit rate of picking 3 of the leader's cards uniformly at
/// random without replacement, given `classes` distinct DD-equivalence classes on the real deal
/// (grouping by identical score) of which `truth_classes` achieve the maximum. Hypergeometric:
/// `P(at least one truth class among 3 picks) = 1 - C(classes - truth_classes, 3) / C(classes, 3)`.
fn random_choice_baseline(classes: u64, truth_classes: u64) -> f64 {
    let total = binomial(classes, 3);
    if total == 0 {
        return 1.0; // fewer than 3 classes: any 3 picks cover every class.
    }
    let miss = binomial(classes - truth_classes, 3);
    1.0 - (miss as f64 / total as f64)
}

/// Number of distinct score values among `scores` (the DD-equivalence classes on the real deal).
fn distinct_classes(scores: &[(Card, u8)]) -> u64 {
    let mut values: Vec<u8> = scores.iter().map(|&(_, s)| s).collect();
    values.sort_unstable();
    values.dedup();
    values.len() as u64
}

/// Baseline (a): no bidding information at all. Samples uniformly (ignoring the auction beyond
/// deriving the contract/leader) and ranks the leader's cards by plain (unweighted, since the
/// vacuous interpretation's likelihood is 1 everywhere) mean defence tricks. This deliberately
/// does not call [`bridge_lead::advise`] (which always interprets the real auction): the point
/// of this baseline is to measure what bidding information is worth, so it must not use any.
fn baseline_a_top3(
    dd: &dyn bridge::dd::DoubleDummy,
    board: &Board,
    samples: usize,
    seed: u64,
) -> Result<Vec<Card>, String> {
    let leader = board.contract.leader();
    let known = KnownCards::from_viewer(leader, board.deal.hand(leader));
    let vacuous = Interpretation {
        seats: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        per_call: Vec::new(),
        divergence: None,
    };
    let play_constraints = [
        bridge_constraint::HandConstraint::ANY,
        bridge_constraint::HandConstraint::ANY,
        bridge_constraint::HandConstraint::ANY,
        bridge_constraint::HandConstraint::ANY,
    ];
    let ctx = SampleContext {
        known,
        interpretation: &vacuous,
        play_constraints: &play_constraints,
        play_soft: None,
        bidding: None,
    };
    let opts = SampleOptions {
        seed,
        ..SampleOptions::default()
    };
    let (deals, _report) =
        sample_deals(&ctx, &UniformProposal, samples, &opts).map_err(|e| e.to_string())?;

    let cards: Vec<Card> = board.deal.hand(leader).cards().collect();
    let mut totals: Vec<(Card, f64)> = cards.iter().map(|&c| (c, 0.0)).collect();
    let strain = board.contract.bid.strain();
    for weighted in &deals {
        let scores = dd
            .lead_scores(&weighted.deal, strain, leader)
            .map_err(|e| e.to_string())?;
        for (card, total) in totals.iter_mut() {
            let score = scores
                .iter()
                .find(|(c, _)| c == card)
                .map(|&(_, s)| s)
                .unwrap_or(0);
            *total += f64::from(score);
        }
    }
    totals.sort_by(|a, b| b.1.total_cmp(&a.1));
    Ok(totals.into_iter().take(3).map(|(c, _)| c).collect())
}

fn compile_table(system_path: &str) -> Result<Table, String> {
    let source =
        std::fs::read_to_string(system_path).map_err(|e| format!("reading {system_path}: {e}"))?;
    let (ir, _lints) =
        bridge::system::compile(system_path, &source, &FsLoader, &CompileOptions::default());
    Ok(Table::uniform(
        std::sync::Arc::new(ir),
        std::sync::Arc::new(NaturalInference::default()),
    ))
}

#[test]
#[ignore = "needs BRIDGE_CORPUS_DIR, bridge_system::compile and ConstraintProposal (both todo!() on this lane's base)"]
fn corpus_eval() {
    let Some(dir) = corpus_dir() else {
        eprintln!("no corpus directory; skipping");
        return;
    };
    let Some(dd) = bridge::dd::dds() else {
        eprintln!("DDS not vendored; skipping");
        return;
    };

    let samples: usize = std::env::var("LEAD_SAMPLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let use_uniform = std::env::var("LEAD_UNIFORM").as_deref() == Ok("1");

    let boards = select_boards(&dir, 100);
    assert!(!boards.is_empty(), "no eligible boards found under {dir:?}");

    // `systems/sayc/sayc.bml` does not exist as a compiled root yet on this lane's base and
    // `bridge_system::compile` is `todo!()`; both are expected to be true until the `system` and
    // `sample` lanes land (crate root doc). This call is reached only when the corpus and DDS
    // preconditions above are both satisfied, and is expected to panic until then.
    let table = compile_table("systems/sayc/sayc.bml").expect("system compiles");

    let uniform = UniformProposal;
    let constraint = ConstraintProposal::default();
    let proposal: &dyn Proposal = if use_uniform { &uniform } else { &constraint };

    let mut top1_hits = 0usize;
    let mut top3_hits = 0usize;
    let mut tricks_lost: Vec<f64> = Vec::new();
    let mut ess_ratios: Vec<f64> = Vec::new();
    let mut per_board_seconds: Vec<f64> = Vec::new();
    let mut baseline_a_top3_hits = 0usize;
    let mut baseline_b_top3_rates: Vec<f64> = Vec::new();

    for board in &boards {
        let start = Instant::now();

        let truth = dd_truth(dd.as_ref(), board).expect("DD solve on the real deal");
        let classes = distinct_classes(&truth.all_scores);
        // The set of cards scoring the maximum is one distinct-score class by construction.
        baseline_b_top3_rates.push(random_choice_baseline(classes, 1));

        let query = LeadQuery {
            auction: &board.auction,
            leader_hand: board.deal.hand(board.contract.leader()),
        };
        let opts = LeadOptions {
            samples,
            seed: 0,
            top_k: 3,
            ..LeadOptions::default()
        };
        let advice = match advise(&table, &query, proposal, dd.as_ref(), &opts) {
            Ok(advice) => advice,
            Err(e) => panic!("advise should succeed on {}: {e}", board.label),
        };

        let hits_truth = |lead: &bridge_lead::LeadScore| {
            truth.cards.contains(&lead.card)
                || lead.equivalents.iter().any(|c| truth.cards.contains(c))
        };
        if advice.leads.first().is_some_and(hits_truth) {
            top1_hits += 1;
        }
        if advice.leads.iter().any(hits_truth) {
            top3_hits += 1;
        }
        if let Some(top1) = advice.leads.first() {
            tricks_lost.push(f64::from(truth.max) - top1.mean_defence_tricks);
        }
        ess_ratios.push(advice.sample_report.ess_ratio);
        per_board_seconds.push(start.elapsed().as_secs_f64());

        match baseline_a_top3(dd.as_ref(), board, samples, 0) {
            Ok(top3) => {
                if top3.iter().any(|c| truth.cards.contains(c)) {
                    baseline_a_top3_hits += 1;
                }
            }
            Err(e) => eprintln!("{}: baseline (a) failed: {e}", board.label),
        }
    }

    let n = boards.len() as f64;
    let report = serde_json::json!({
        "boards": boards.len(),
        "samples_per_board": samples,
        "proposal": if use_uniform { "uniform" } else { "constraint" },
        "hit_rate_top1": top1_hits as f64 / n,
        "hit_rate_top3": top3_hits as f64 / n,
        "mean_tricks_lost_top1": tricks_lost.iter().sum::<f64>() / tricks_lost.len().max(1) as f64,
        "mean_ess_ratio": ess_ratios.iter().sum::<f64>() / ess_ratios.len().max(1) as f64,
        "mean_seconds_per_board": per_board_seconds.iter().sum::<f64>() / n,
        "baseline_no_bidding_hit_rate_top3": baseline_a_top3_hits as f64 / n,
        "baseline_random_hit_rate_top3": baseline_b_top3_rates.iter().sum::<f64>() / n,
    });

    std::fs::create_dir_all("target").ok();
    std::fs::write(
        "target/lead_report.json",
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .expect("writing target/lead_report.json");
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
