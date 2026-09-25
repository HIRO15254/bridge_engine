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
//! evaluation, not just baseline (a)), `BRIDGE_CORPUS_DIR` and `BRIDGE_SYSTEMS_DIR` (both follow
//! `crates/bridge-format/tests/common/mod.rs` / `systems/README.md`'s convention: an explicit
//! directory, or else relative to `CARGO_MANIFEST_DIR`).
#![cfg(feature = "dds")]

mod common;

use std::path::{Path, PathBuf};
use std::time::Instant;

use bridge::system::lexer::FsLoader;
use bridge::system::{CompileOptions, NaturalInference};
use bridge_bidding::{Interpretation, Table};
use bridge_constraint::{HandConstraint, KnownCards};
use bridge_core::{Auction, Card, Contract, Deal};
use bridge_format::pbn;
use bridge_lead::{LeadAdvice, LeadOptions, LeadQuery, LeadScore, advise, advise_with_context};
use bridge_sample::{ConstraintProposal, Proposal, SampleContext, UniformProposal};

/// Cargo runs an integration test's binary with the *package* root (`crates/bridge-lead`) as its
/// working directory, not the workspace root — so every workspace-relative path in this file
/// (the corpus, the system, `target/lead_report.json`) is resolved explicitly against the
/// workspace root rather than a bare relative path (which would silently resolve under
/// `crates/bridge-lead/` instead; `crates/bridge-system/tests/desc_recognition_report.rs` follows
/// the same convention).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/bridge-lead is two levels under the workspace root")
        .to_path_buf()
}

/// `BRIDGE_CORPUS_DIR`, or `<workspace>/corpus/data`; `None` when the directory is absent
/// (`crates/bridge-format/tests/common/mod.rs`'s convention).
fn corpus_dir() -> Option<PathBuf> {
    let dir = match std::env::var_os("BRIDGE_CORPUS_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => workspace_root().join("corpus/data"),
    };
    if dir.is_dir() { Some(dir) } else { None }
}

/// `BRIDGE_SYSTEMS_DIR`, or `<workspace>/systems` (`systems/README.md`'s convention).
fn systems_dir() -> PathBuf {
    match std::env::var_os("BRIDGE_SYSTEMS_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => workspace_root().join("systems"),
    }
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

/// The real deal's DD lead scores, the maximum among them, and the (possibly several) cards
/// achieving it: the DD-optimal leads.
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

/// Number of distinct score values among `scores` (the DD-equivalence classes on the real deal).
/// Used only to separate "trivial" boards (every lead scores the same, so every baseline and
/// every advisor trivially hits) from boards where the choice of lead actually matters.
fn distinct_classes(scores: &[(Card, u8)]) -> u64 {
    let mut values: Vec<u8> = scores.iter().map(|&(_, s)| s).collect();
    values.sort_unstable();
    values.dedup();
    values.len() as u64
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

/// Baseline (b): the expected top-`k` hit rate of picking `k` of the leader's 13 cards uniformly
/// at random without replacement, given that `m` of those 13 cards are DD-optimal on the real
/// deal. Hypergeometric: `P(at least one optimal card among k picks) = 1 - C(13 - m, k) / C(13,
/// k)`. This is over the leader's *cards* (`13 choose k`), not over distinct DD-score *values* —
/// with touching honours routinely sharing a score, the number of cards achieving the maximum is
/// usually well above 1, so this baseline is informative rather than trivially 1.0 (review
/// finding: the previous version picked among score-value classes, which this corpus never has
/// more than 3 of, making the baseline exactly 1.0 on every board).
fn random_choice_baseline(m: u64, k: u64) -> f64 {
    let total = binomial(13, k);
    if total == 0 {
        return 1.0;
    }
    let miss = binomial(13u64.saturating_sub(m), k);
    1.0 - (miss as f64 / total as f64)
}

/// The vacuous interpretation: no seat has any resolved alternative, so
/// [`bridge_sample::sequence_log_likelihood`] would find nothing to weight by, and — paired with
/// `bidding: None` in [`SampleContext`] — sampling ignores the auction and calls entirely, other
/// than to have already derived the contract and leader from it. Equivalent to "no bidding
/// information", i.e. baseline (a).
fn vacuous_interpretation() -> Interpretation {
    Interpretation {
        seats: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        per_call: Vec::new(),
        divergence: None,
    }
}

/// Baseline (a): no bidding information. Samples uniformly (ignoring the auction beyond deriving
/// the contract/leader) and ranks with the exact same pipeline `advise` uses
/// (`bridge_lead::advise_with_context`, `#[doc(hidden)]`) so its top-3 is comparable to the
/// advisor's: equivalence groups, not bare cards, and a hit credits the whole group
/// (review finding: re-implementing the ranking without grouping inflated the measured value of
/// bidding information, since a touching-honour sequence like AKQ is one DD class but would count
/// as 3 separate "hits" worth of baseline coverage).
fn baseline_a(
    dd: &dyn bridge::dd::DoubleDummy,
    board: &Board,
    samples: usize,
    seed: u64,
) -> Result<LeadAdvice, String> {
    let leader = board.contract.leader();
    let known = KnownCards::from_viewer(leader, board.deal.hand(leader));
    let vacuous = vacuous_interpretation();
    let play_constraints = [
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
    ];
    let ctx = SampleContext {
        known,
        interpretation: &vacuous,
        play_constraints: &play_constraints,
        play_soft: None,
        bidding: None,
    };
    let vulnerable = board
        .auction
        .vulnerability()
        .is_vulnerable(board.contract.declarer);
    let opts = LeadOptions {
        samples,
        seed,
        top_k: 3,
        ..LeadOptions::default()
    };
    advise_with_context(
        &ctx,
        board.contract,
        vulnerable,
        &UniformProposal,
        dd,
        &opts,
    )
    .map_err(|e| e.to_string())
}

fn compile_table(system_path: &Path) -> Result<Table, String> {
    let source = std::fs::read_to_string(system_path)
        .map_err(|e| format!("reading {}: {e}", system_path.display()))?;
    let path_str = system_path.to_string_lossy();
    let (ir, _lints) =
        bridge::system::compile(&path_str, &source, &FsLoader, &CompileOptions::default());
    Ok(Table::uniform(
        std::sync::Arc::new(ir),
        std::sync::Arc::new(NaturalInference::default()),
    ))
}

/// Whether `lead` (its representative card, or one of its equivalents) contains one of `truth`'s
/// DD-optimal cards. Shared by the main advisor's hit check and baseline (a)'s.
fn hits_truth(lead: &LeadScore, truth: &Truth) -> bool {
    truth.cards.contains(&lead.card) || lead.equivalents.iter().any(|c| truth.cards.contains(c))
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

    let system_path = systems_dir().join("sayc.bml");
    // `bridge_system::compile` is `todo!()` on this lane's base and `systems/sayc.bml` may not
    // exist as a compiled root yet (crate root doc); both are expected to be true until the
    // `system` and `sample` lanes land. This call is reached only when the corpus and DDS
    // preconditions above are both satisfied, and is expected to panic until then.
    let table = compile_table(&system_path).expect("system compiles");

    let uniform = UniformProposal;
    let constraint = ConstraintProposal::default();
    let proposal: &dyn Proposal = if use_uniform { &uniform } else { &constraint };

    let mut top1_hits = 0usize;
    let mut top3_hits = 0usize;
    let mut top1_hits_nontrivial = 0usize;
    let mut top3_hits_nontrivial = 0usize;
    let mut nontrivial_boards = 0usize;
    let mut tricks_lost: Vec<f64> = Vec::new();
    let mut estimation_error: Vec<f64> = Vec::new();
    let mut ess_ratios: Vec<f64> = Vec::new();
    let mut per_board_seconds: Vec<f64> = Vec::new();
    let mut baseline_a_top1_hits = 0usize;
    let mut baseline_a_top3_hits = 0usize;
    let mut baseline_b_top1_rates: Vec<f64> = Vec::new();
    let mut baseline_b_top3_rates: Vec<f64> = Vec::new();

    for board in &boards {
        let start = Instant::now();

        let truth = dd_truth(dd.as_ref(), board).expect("DD solve on the real deal");
        let m = truth.cards.len() as u64;
        baseline_b_top1_rates.push(random_choice_baseline(m, 1));
        baseline_b_top3_rates.push(random_choice_baseline(m, 3));
        let nontrivial = distinct_classes(&truth.all_scores) >= 2;
        if nontrivial {
            nontrivial_boards += 1;
        }

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

        let top1_hit = advice.leads.first().is_some_and(|l| hits_truth(l, &truth));
        let top3_hit = advice.leads.iter().any(|l| hits_truth(l, &truth));
        if top1_hit {
            top1_hits += 1;
        }
        if top3_hit {
            top3_hits += 1;
        }
        if nontrivial {
            if top1_hit {
                top1_hits_nontrivial += 1;
            }
            if top3_hit {
                top3_hits_nontrivial += 1;
            }
        }
        // The loss from the *choice actually made*: the chosen card's score on the real deal
        // (not the advisor's own sample-estimated mean, which measures estimation bias, not the
        // consequence of the choice — review finding). `all_scores` covers every one of the
        // leader's 13 cards (the `DoubleDummy` contract `aggregate.rs::scores_by_card` also
        // relies on), so the lookup cannot miss.
        if let Some(top1) = advice.leads.first() {
            let real = truth
                .all_scores
                .iter()
                .find(|(c, _)| *c == top1.card)
                .map(|&(_, s)| s)
                .expect("all_scores covers every one of the leader's 13 cards");
            tricks_lost.push(f64::from(truth.max - real));
            estimation_error.push((top1.mean_defence_tricks - f64::from(real)).abs());
        }
        ess_ratios.push(advice.sample_report.ess_ratio);
        per_board_seconds.push(start.elapsed().as_secs_f64());

        match baseline_a(dd.as_ref(), board, samples, 0) {
            Ok(advice) => {
                if advice.leads.first().is_some_and(|l| hits_truth(l, &truth)) {
                    baseline_a_top1_hits += 1;
                }
                if advice.leads.iter().any(|l| hits_truth(l, &truth)) {
                    baseline_a_top3_hits += 1;
                }
            }
            Err(e) => eprintln!("{}: baseline (a) failed: {e}", board.label),
        }
    }

    let n = boards.len() as f64;
    let nontrivial_n = (nontrivial_boards as f64).max(1.0);
    let report = serde_json::json!({
        "boards": boards.len(),
        "nontrivial_boards": nontrivial_boards,
        "samples_per_board": samples,
        "proposal": if use_uniform { "uniform" } else { "constraint" },
        "hit_rate_top1": top1_hits as f64 / n,
        "hit_rate_top3": top3_hits as f64 / n,
        "hit_rate_top1_nontrivial": top1_hits_nontrivial as f64 / nontrivial_n,
        "hit_rate_top3_nontrivial": top3_hits_nontrivial as f64 / nontrivial_n,
        "mean_tricks_lost_top1": tricks_lost.iter().fold(0.0, |a, b| a + b) / tricks_lost.len().max(1) as f64,
        "mean_estimation_error_top1": estimation_error.iter().fold(0.0, |a, b| a + b) / estimation_error.len().max(1) as f64,
        "mean_ess_ratio": ess_ratios.iter().fold(0.0, |a, b| a + b) / n,
        "mean_seconds_per_board": per_board_seconds.iter().fold(0.0, |a, b| a + b) / n,
        "baseline_no_bidding_hit_rate_top1": baseline_a_top1_hits as f64 / n,
        "baseline_no_bidding_hit_rate_top3": baseline_a_top3_hits as f64 / n,
        "baseline_random_hit_rate_top1": baseline_b_top1_rates.iter().fold(0.0, |a, b| a + b) / n,
        "baseline_random_hit_rate_top3": baseline_b_top3_rates.iter().fold(0.0, |a, b| a + b) / n,
    });

    let target_dir = workspace_root().join("target");
    std::fs::create_dir_all(&target_dir).ok();
    std::fs::write(
        target_dir.join("lead_report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .expect("writing target/lead_report.json");
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
