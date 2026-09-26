//! Corpus evaluation harness (roadmap phase 6.2, `docs/design/14-lead.md` §4).
//!
//! `#[ignore]`: it compiles `systems/sayc/sayc.bml`, samples deals for 100 corpus boards and
//! solves every sample double-dummy, which takes minutes even in release. Run it with:
//!
//! ```text
//! cargo test -p bridge-lead --release --features dds,parallel --test corpus_eval -- --ignored --nocapture
//! ```
//!
//! Each board's result is written to `target/lead_eval/<config>/board_NNN.json`, where
//! `<config>` names the configuration (e.g. `n100-constraint-seed0-boards100`), and every run
//! rewrites that configuration's report from all records present in its directory:
//! `target/lead_report.json` for the default configuration (100 samples, `ConstraintProposal`,
//! 100 boards), `target/lead_report_<suffix>.json` otherwise (e.g. `lead_report_n500.json`). Runs
//! under different configurations therefore never overwrite each other's records or reports. The
//! evaluation can be split into shorter runs with `LEAD_BOARDS=a..b` (half-open range of selected
//! board indices); the report is complete once `boards_with_records == boards_selected`. A board whose real deal cannot be solved or for
//! which `advise` returns an error (e.g. `NoSamples`) is recorded as skipped with the reason.
//!
//! Environment variables: `LEAD_SAMPLES` (default 100, samples per board for the real harness),
//! `LEAD_BOARD_COUNT` (default 100, boards selected), `LEAD_BOARDS` (see above),
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
        for (game_index, game) in file.games.iter().enumerate() {
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
                                "{}#{} (game {game_index})",
                                path.strip_prefix(corpus_dir).unwrap_or(&path).display(),
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
/// advisor's: the same equivalence groups and the same hit checks ([`hits_truth`] on the led card,
/// [`hits_truth_group`] as the secondary group-credited rate). Re-implementing the ranking without
/// grouping would let a touching-honour sequence like AKQ fill all 3 slots with one DD class.
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

/// The primary hit: whether the card the advisor actually leads (the group's representative) is
/// DD-optimal on the real deal. Shared by the main advisor and baseline (a), and like-for-like
/// with baseline (b), which also picks `k` single cards.
fn hits_truth(lead: &LeadScore, truth: &Truth) -> bool {
    truth.cards.contains(&lead.card)
}

/// The secondary, group-credited hit: whether *any* card of `lead`'s group (representative or
/// equivalent) is DD-optimal. A top-`k` list of groups covers more than `k` cards, so this rate
/// is only comparable with baseline (b) evaluated at the covered card count
/// (`baseline_random_covered_top{1,3}`), not with (b) at `k` (review finding: on this corpus the
/// top-3 groups cover 5.4 cards on average).
fn hits_truth_group(lead: &LeadScore, truth: &Truth) -> bool {
    hits_truth(lead, truth) || lead.equivalents.iter().any(|c| truth.cards.contains(c))
}

/// Number of the leader's cards the given groups cover (representatives plus equivalents).
fn cards_covered(leads: &[LeadScore]) -> u64 {
    leads.iter().map(|l| 1 + l.equivalents.len() as u64).sum()
}

/// `LEAD_BOARDS=a..b`: the half-open range of selected board indices this run evaluates
/// (default `0..LEAD_BOARD_COUNT`). Lets the 100-board evaluation be split into several shorter
/// runs; every run rewrites its configuration's report from all per-board records present.
fn board_range(total: usize) -> std::ops::Range<usize> {
    let Ok(spec) = std::env::var("LEAD_BOARDS") else {
        return 0..total;
    };
    let (a, b) = spec
        .split_once("..")
        .expect("LEAD_BOARDS must look like a..b");
    let a: usize = a.parse().expect("LEAD_BOARDS start");
    let b: usize = b.parse().expect("LEAD_BOARDS end");
    a.min(total)..b.min(total)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

fn median(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    if v.len() % 2 == 1 {
        v[mid]
    } else {
        (v[mid - 1] + v[mid]) / 2.0
    }
}

/// Evaluates one board and returns its record (`status` is `"ok"` or `"skipped"`).
fn evaluate_board(
    index: usize,
    board: &Board,
    table: &Table,
    proposal: &dyn Proposal,
    dd: &dyn bridge::dd::DoubleDummy,
    config: &serde_json::Value,
    samples: usize,
) -> serde_json::Value {
    let start = Instant::now();
    let mut record = serde_json::json!({
        "index": index,
        "label": board.label,
        "contract": board.contract.to_string(),
        "config": config,
    });
    let skip = |mut record: serde_json::Value, reason: String| {
        record["status"] = "skipped".into();
        record["reason"] = reason.into();
        record
    };

    let truth = match dd_truth(dd, board) {
        Ok(truth) => truth,
        Err(e) => return skip(record, format!("DD solve of the real deal failed: {e}")),
    };
    let m = truth.cards.len() as u64;
    record["optimal_cards"] = m.into();
    record["nontrivial"] = (distinct_classes(&truth.all_scores) >= 2).into();
    record["baseline_random_top1"] = random_choice_baseline(m, 1).into();
    record["baseline_random_top3"] = random_choice_baseline(m, 3).into();

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
    let advice = match advise(table, &query, proposal, dd, &opts) {
        Ok(advice) => advice,
        Err(e) => return skip(record, format!("advise failed: {e}")),
    };
    let Some(top1) = advice.leads.first() else {
        return skip(record, "advise returned no leads".to_string());
    };
    record["top1_hit"] = hits_truth(top1, &truth).into();
    record["top3_hit"] = advice.leads.iter().any(|l| hits_truth(l, &truth)).into();
    record["top1_group_hit"] = hits_truth_group(top1, &truth).into();
    record["top3_group_hit"] = advice
        .leads
        .iter()
        .any(|l| hits_truth_group(l, &truth))
        .into();
    let covered1 = cards_covered(&advice.leads[..1]);
    let covered3 = cards_covered(&advice.leads);
    record["top1_cards_covered"] = covered1.into();
    record["top3_cards_covered"] = covered3.into();
    // Baseline (b) at the group-credited metric's own coverage: `covered` random cards.
    record["baseline_random_covered_top1"] = random_choice_baseline(m, covered1).into();
    record["baseline_random_covered_top3"] = random_choice_baseline(m, covered3).into();
    // The loss from the *choice actually made*: the chosen card's score on the real deal (not
    // the advisor's own sample-estimated mean, which measures estimation bias, not the
    // consequence of the choice). `all_scores` covers every one of the leader's 13 cards (the
    // `DoubleDummy` contract `aggregate.rs::scores_by_card` also relies on).
    let real = truth
        .all_scores
        .iter()
        .find(|(c, _)| *c == top1.card)
        .map(|&(_, s)| s)
        .expect("all_scores covers every one of the leader's 13 cards");
    record["top1_card"] = top1.card.to_string().into();
    record["tricks_lost_top1"] = f64::from(truth.max - real).into();
    record["estimation_error_top1"] = (top1.mean_defence_tricks - f64::from(real)).abs().into();
    record["ess"] = advice.sample_report.ess.into();
    record["ess_ratio"] = advice.sample_report.ess_ratio.into();
    record["produced"] = advice.sample_report.produced.into();
    record["attempts"] = advice.sample_report.attempts.into();
    record["advise_seconds"] = start.elapsed().as_secs_f64().into();

    match baseline_a(dd, board, samples, 0) {
        Ok(base) => {
            record["baseline_no_bidding_top1"] = base
                .leads
                .first()
                .is_some_and(|l| hits_truth(l, &truth))
                .into();
            record["baseline_no_bidding_top3"] =
                base.leads.iter().any(|l| hits_truth(l, &truth)).into();
            record["baseline_no_bidding_top1_group"] = base
                .leads
                .first()
                .is_some_and(|l| hits_truth_group(l, &truth))
                .into();
            record["baseline_no_bidding_top3_group"] = base
                .leads
                .iter()
                .any(|l| hits_truth_group(l, &truth))
                .into();
        }
        Err(e) => record["baseline_no_bidding_error"] = e.into(),
    }
    record["seconds"] = start.elapsed().as_secs_f64().into();
    record["status"] = "ok".into();
    record
}

/// Builds the summary over every per-board record in `records_dir` whose `config` equals
/// `config` and whose index is below `total`. `records_dir` is already per-configuration; the
/// `config` check additionally drops records a different harness version wrote there.
fn summarise(records_dir: &Path, config: &serde_json::Value, total: usize) -> serde_json::Value {
    let mut records: Vec<serde_json::Value> = Vec::new();
    for i in 0..total {
        let path = records_dir.join(format!("board_{i:03}.json"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if value["config"] == *config {
            records.push(value);
        }
    }
    let ok: Vec<&serde_json::Value> = records.iter().filter(|r| r["status"] == "ok").collect();
    let skipped: Vec<serde_json::Value> = records
        .iter()
        .filter(|r| r["status"] == "skipped")
        .map(|r| serde_json::json!({ "index": r["index"], "label": r["label"], "reason": r["reason"] }))
        .collect();
    let flag = |r: &&serde_json::Value, key: &str| r[key].as_bool() == Some(true);
    let num = |r: &&serde_json::Value, key: &str| r[key].as_f64().unwrap_or(0.0);
    let rate = |set: &[&serde_json::Value], key: &str| {
        if set.is_empty() {
            0.0
        } else {
            set.iter().filter(|r| flag(r, key)).count() as f64 / set.len() as f64
        }
    };
    let avg = |set: &[&serde_json::Value], key: &str| {
        mean(&set.iter().map(|r| num(r, key)).collect::<Vec<_>>())
    };
    let nontrivial: Vec<&serde_json::Value> = ok
        .iter()
        .copied()
        .filter(|r| flag(r, "nontrivial"))
        .collect();
    let with_baseline: Vec<&serde_json::Value> = ok
        .iter()
        .copied()
        .filter(|r| r.get("baseline_no_bidding_top1").is_some())
        .collect();
    let ess_ratios: Vec<f64> = ok.iter().map(|r| num(r, "ess_ratio")).collect();
    let ess: Vec<f64> = ok.iter().map(|r| num(r, "ess")).collect();
    let seconds: Vec<f64> = ok.iter().map(|r| num(r, "seconds")).collect();
    serde_json::json!({
        "boards_selected": total,
        "boards_with_records": records.len(),
        "boards": ok.len(),
        "boards_skipped": skipped.len(),
        "skipped": skipped,
        "nontrivial_boards": nontrivial.len(),
        "samples_per_board": config["samples"],
        "proposal": config["proposal"],
        "hit_rate_top1": rate(&ok, "top1_hit"),
        "hit_rate_top3": rate(&ok, "top3_hit"),
        "hit_rate_top1_nontrivial": rate(&nontrivial, "top1_hit"),
        "hit_rate_top3_nontrivial": rate(&nontrivial, "top3_hit"),
        "group_hit_rate_top1": rate(&ok, "top1_group_hit"),
        "group_hit_rate_top3": rate(&ok, "top3_group_hit"),
        "group_hit_rate_top1_nontrivial": rate(&nontrivial, "top1_group_hit"),
        "group_hit_rate_top3_nontrivial": rate(&nontrivial, "top3_group_hit"),
        "mean_top1_cards_covered": avg(&ok, "top1_cards_covered"),
        "mean_top3_cards_covered": avg(&ok, "top3_cards_covered"),
        "mean_tricks_lost_top1": avg(&ok, "tricks_lost_top1"),
        "mean_estimation_error_top1": avg(&ok, "estimation_error_top1"),
        "ess_mean": mean(&ess),
        "ess_median": median(&ess),
        "ess_min": ess.iter().copied().fold(f64::INFINITY, f64::min),
        "ess_max": ess.iter().copied().fold(0.0, f64::max),
        "mean_ess_ratio": mean(&ess_ratios),
        "median_ess_ratio": median(&ess_ratios),
        "boards_ess_ratio_ge_0_5": ess_ratios.iter().filter(|&&r| r >= 0.5).count(),
        "boards_ess_lt_5": ess.iter().filter(|&&e| e < 5.0).count(),
        "mean_seconds_per_board": mean(&seconds),
        "total_seconds": seconds.iter().sum::<f64>(),
        "baseline_boards": with_baseline.len(),
        "baseline_no_bidding_hit_rate_top1": rate(&with_baseline, "baseline_no_bidding_top1"),
        "baseline_no_bidding_hit_rate_top3": rate(&with_baseline, "baseline_no_bidding_top3"),
        "baseline_no_bidding_group_hit_rate_top1":
            rate(&with_baseline, "baseline_no_bidding_top1_group"),
        "baseline_no_bidding_group_hit_rate_top3":
            rate(&with_baseline, "baseline_no_bidding_top3_group"),
        "baseline_random_hit_rate_top1": avg(&ok, "baseline_random_top1"),
        "baseline_random_hit_rate_top3": avg(&ok, "baseline_random_top3"),
        "baseline_random_covered_hit_rate_top1": avg(&ok, "baseline_random_covered_top1"),
        "baseline_random_covered_hit_rate_top3": avg(&ok, "baseline_random_covered_top3"),
    })
}

/// Bumped whenever a record's fields or their meaning change, so `summarise` never mixes records
/// written by an older harness (2: the primary hit became representative-card-only, group hits
/// and coverage were added).
const RECORD_VERSION: u64 = 2;

const DEFAULT_SAMPLES: usize = 100;
const DEFAULT_BOARD_COUNT: usize = 100;

/// `lead_report.json` for the default configuration, `lead_report_<suffix>.json` otherwise, where
/// the suffix lists only the settings that differ from the default (`n500`, `uniform`,
/// `seed7`, `boards20`, joined by `-`).
fn report_file_name(samples: usize, uniform: bool, seed: u64, boards: usize) -> String {
    let mut parts = Vec::new();
    if samples != DEFAULT_SAMPLES {
        parts.push(format!("n{samples}"));
    }
    if uniform {
        parts.push("uniform".to_string());
    }
    if seed != 0 {
        parts.push(format!("seed{seed}"));
    }
    if boards != DEFAULT_BOARD_COUNT {
        parts.push(format!("boards{boards}"));
    }
    if parts.is_empty() {
        "lead_report.json".to_string()
    } else {
        format!("lead_report_{}.json", parts.join("-"))
    }
}

#[test]
fn report_file_names() {
    assert_eq!(report_file_name(100, false, 0, 100), "lead_report.json");
    assert_eq!(
        report_file_name(500, false, 0, 100),
        "lead_report_n500.json"
    );
    assert_eq!(
        report_file_name(100, true, 3, 20),
        "lead_report_uniform-seed3-boards20.json"
    );
}

#[test]
#[ignore = "release-only corpus evaluation: needs BRIDGE_CORPUS_DIR and several minutes of DDS"]
fn corpus_eval() {
    let Some(dir) = corpus_dir() else {
        eprintln!("no corpus directory; skipping");
        return;
    };
    let Some(dd) = bridge::dd::dds() else {
        eprintln!("DDS not vendored; skipping");
        return;
    };

    let samples = env_usize("LEAD_SAMPLES", DEFAULT_SAMPLES);
    let board_count = env_usize("LEAD_BOARD_COUNT", DEFAULT_BOARD_COUNT);
    let use_uniform = std::env::var("LEAD_UNIFORM").as_deref() == Ok("1");

    let boards = select_boards(&dir, board_count);
    assert!(!boards.is_empty(), "no eligible boards found under {dir:?}");

    let system_path = systems_dir().join("sayc").join("sayc.bml");
    let table = compile_table(&system_path).expect("system compiles");

    let uniform = UniformProposal;
    let constraint = ConstraintProposal::default();
    let proposal: &dyn Proposal = if use_uniform { &uniform } else { &constraint };
    // Records are only merged into the summary when they were produced under this exact
    // configuration, so a stale record from a run with different settings is never mixed in.
    let proposal_name = if use_uniform { "uniform" } else { "constraint" };
    let seed = 0u64;
    let config = serde_json::json!({
        "samples": samples,
        "proposal": proposal_name,
        "seed": seed,
        "boards_selected": boards.len(),
        "record_version": RECORD_VERSION,
    });

    // Each configuration gets its own records directory and report, so e.g. the n = 500
    // reference run cannot overwrite the default n = 100 run's evidence (review finding).
    let target_dir = workspace_root().join("target");
    let config_name = format!(
        "n{samples}-{proposal_name}-seed{seed}-boards{}",
        boards.len()
    );
    let records_dir = target_dir.join("lead_eval").join(&config_name);
    std::fs::create_dir_all(&records_dir).expect("creating the per-configuration records dir");
    let report_path = target_dir.join(report_file_name(samples, use_uniform, seed, boards.len()));

    for index in board_range(boards.len()) {
        let record = evaluate_board(
            index,
            &boards[index],
            &table,
            proposal,
            dd.as_ref(),
            &config,
            samples,
        );
        println!("{record}");
        std::fs::write(
            records_dir.join(format!("board_{index:03}.json")),
            serde_json::to_string_pretty(&record).unwrap(),
        )
        .expect("writing a per-board record");
    }

    let mut report = summarise(&records_dir, &config, boards.len());
    report["records_dir"] = format!("target/lead_eval/{config_name}").into();
    std::fs::write(&report_path, serde_json::to_string_pretty(&report).unwrap())
        .expect("writing the lead report");
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
