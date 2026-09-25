//! Reproduction-rate property (07-bidding.md §10's reverse direction; 11-testing.md §3):
//! `consistency.rs` checks that `interpret` can always explain whatever `choose_bid` picked
//! (definitions too *tight*); this harness checks the opposite direction over real tournament
//! auctions -- does a deal `interpret` accepts for an auction actually *replay* into that same
//! auction via `choose_bid`? A node whose reproduction rate is low is a node whose constraint is
//! too *loose* (accepts hands that would never actually have produced that call).
//!
//! 11-testing.md §3's own pseudocode samples with `bridge_constraint::ConstraintProposal` and
//! counts a raw (unweighted) fraction, because that proposal already draws from something close
//! to the target distribution. `ConstraintProposal` is still `todo!()` on this branch (phase 5,
//! a parallel lane's scope -- see `bridge-sample/src/lib.rs`'s crate-level doc comment and its
//! `#![allow(dead_code, unused_variables)]`), so this harness instead follows the task brief's own
//! escape hatch: sample with `bridge_sample::UniformProposal` (fully random deals, agnostic to the
//! auction) and fold the correction into the importance weight itself, by setting
//! `SampleContext::bidding = Some(BiddingLikelihood { .. })`. That makes `sample_deals` score each
//! uniformly-drawn deal by `sequence_log_likelihood` (the actual `choose_bid` policy's likelihood
//! of the real auction under that deal) rather than by `Interpretation::likelihood`. Each auction's
//! reproduction rate is therefore the *weighted* fraction of its 1000 uniform samples that replay
//! correctly -- weighted by `exp(log_weight - max log_weight)`, the same numerically-stable
//! renormalisation `bridge_sample::effective_sample_size` uses -- not a raw count as in the design
//! doc's `ConstraintProposal`-based pseudocode.

mod common;

use std::path::{Path, PathBuf};

use bridge_bidding::{
    BidContext, ImplicitPass, InterpretOptions, Interpretation, NodeId, PolicyParams,
    ResolutionKind, Scoring, interpret, replay,
};
use bridge_constraint::{HandConstraint, KnownCards};
use bridge_core::Auction;
use bridge_sample::{
    BiddingLikelihood, SampleContext, SampleOptions, UniformProposal, sample_deals,
};
use serde_json::json;

/// Every `.pbn` file under `dir`, recursively, in a stable (sorted) order.
fn pbn_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("pbn") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// Extracts up to `limit` validated [`Auction`]s from every `.pbn` game under `<dir>/pbn`, in file
/// order. A game contributes an auction only when its view resolves one (`GameView::auction`);
/// the two `Optimum*Table.pbn` reference files (no `Auction` section) and any truncated/`-` game
/// contribute none. The vendored tournament corpus (four 2019 world-championship finals, ~600
/// boards across both rooms) comfortably exceeds `limit` on its own, so LIN is not also needed
/// here.
fn corpus_auctions(dir: &Path, limit: usize) -> Vec<Auction> {
    let mut auctions = Vec::new();
    'files: for path in pbn_files(&dir.join("pbn")) {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let (file, _warnings) = bridge_format::pbn::parse_lenient(&bytes);
        let mut previous: Option<bridge_format::GameView> = None;
        for game in &file.games {
            let view = game.view(previous.as_ref()).ok();
            if let Some(view) = &view {
                if let Some(auction) = &view.auction {
                    auctions.push(auction.clone());
                    if auctions.len() >= limit {
                        break 'files;
                    }
                }
            }
            previous = view;
        }
    }
    auctions
}

/// One auction's reproduction-rate record.
struct AuctionRecord {
    path: String,
    /// The last call resolved `Exact`'s node, formatted `"node:<id>"`, or `"off_system"` when no
    /// call in the auction resolved `Exact` at all (11-testing.md §3: "ノード別 (最後に Exact
    /// 解決したノード)").
    node_key: String,
    /// The auction's very last call's `ResolutionKind` (11-testing.md §3's other axis), or
    /// `"empty_auction"` for the vacuous zero-call auction (never produced by real corpus data,
    /// kept only so the match is total).
    kind_key: String,
    /// The weighted fraction of `requested` uniform samples whose `choose_bid` replay reproduces
    /// this auction exactly.
    rate: f64,
    requested: usize,
    produced: usize,
    ess: f64,
}

/// The last call in `interp.per_call` resolved `Exact`, if any, and its node -- taken from its
/// first alternative's [`CallExplanation`] (an `Exact` call's alternatives all come from the same
/// trie position, so every alternative names the same node).
fn last_exact_node(interp: &Interpretation) -> Option<NodeId> {
    interp
        .per_call
        .iter()
        .rev()
        .find(|pc| pc.kind == ResolutionKind::Exact)
        .and_then(|pc| pc.alternatives.first())
        .and_then(|(_, _, ex)| ex.node)
}

fn node_key(interp: &Interpretation) -> String {
    match last_exact_node(interp) {
        Some(id) => format!("node:{}", id.0),
        None => "off_system".to_string(),
    }
}

fn kind_key(interp: &Interpretation) -> String {
    match interp.per_call.last() {
        Some(pc) => format!("{:?}", pc.kind),
        None => "empty_auction".to_string(),
    }
}

/// A distinct sub-seed per auction, so every auction's 1000 uniform draws are an independent,
/// reproducible random stream (same construction as `rng_for`'s own `splitmix64` step, spelled
/// out here rather than imported since only a `u64 -> u64` seed derivation is needed, not a full
/// RNG).
fn auction_seed(base: u64, index: usize) -> u64 {
    base.wrapping_add((index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// Runs one auction's reproduction-rate check: 1000 uniformly-random deals, importance-weighted by
/// `sequence_log_likelihood` of `auction` under `bid_ctx`, each replayed with `choose_bid` and
/// compared back against `auction`.
fn process_auction(
    table: &bridge_bidding::Table,
    bid_ctx: &BidContext<'_>,
    auction: &Auction,
    seed: u64,
) -> AuctionRecord {
    let interp = interpret(table, auction, &InterpretOptions::default());
    let sample_ctx = SampleContext {
        known: KnownCards::EMPTY,
        interpretation: &interp,
        play_constraints: &[HandConstraint::ANY; 4],
        play_soft: None,
        bidding: Some(BiddingLikelihood {
            table,
            auction,
            ctx: bid_ctx,
        }),
    };
    let opts = SampleOptions {
        seed,
        ..SampleOptions::default()
    };
    let requested = 1000;
    let (deals, sample_report) = sample_deals(&sample_ctx, &UniformProposal, requested, &opts)
        .expect("UniformProposal + BiddingLikelihood always prepares and never hits EmptySupport");

    let log_weight_max = deals
        .iter()
        .map(|d| d.log_weight)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut weight_sum = 0.0f64;
    let mut reproduced_weight = 0.0f64;
    for weighted in &deals {
        let w = (weighted.log_weight - log_weight_max).exp();
        weight_sum += w;
        let replayed = replay(
            table,
            &weighted.deal,
            auction.dealer(),
            auction.vulnerability(),
            bid_ctx,
        );
        if replayed.auction == *auction {
            reproduced_weight += w;
        }
    }
    let rate = if weight_sum > 0.0 {
        reproduced_weight / weight_sum
    } else {
        0.0
    };

    AuctionRecord {
        path: format!("{auction}"),
        node_key: node_key(&interp),
        kind_key: kind_key(&interp),
        rate,
        requested,
        produced: deals.len(),
        ess: sample_report.ess,
    }
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

/// Groups `records` by `key`, sorted by descending group size, and computes each group's median
/// `rate` (11-testing.md §3: "中央値・分位点").
fn grouped_medians(
    records: &[AuctionRecord],
    key: impl Fn(&AuctionRecord) -> String,
) -> Vec<serde_json::Value> {
    use std::collections::HashMap;
    let mut groups: HashMap<String, Vec<f64>> = HashMap::new();
    for r in records {
        groups.entry(key(r)).or_default().push(r.rate);
    }
    let mut rows: Vec<(String, Vec<f64>)> = groups.into_iter().collect();
    rows.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
    rows.into_iter()
        .map(|(key, rates)| {
            json!({
                "key": key,
                "count": rates.len(),
                "median": median(&rates),
            })
        })
        .collect()
}

/// Writes `<workspace>/target/reproduction_report.json` (11-testing.md §3's shape).
fn write_json(records: &[AuctionRecord], corpus_dir: &Path) {
    let rates: Vec<f64> = records.iter().map(|r| r.rate).collect();
    let report = json!({
        "auctions": records.len(),
        "corpus_dir": corpus_dir.display().to_string(),
        "overall_median": median(&rates),
        "by_node": grouped_medians(records, |r| r.node_key.clone()),
        "by_resolution_kind": grouped_medians(records, |r| r.kind_key.clone()),
        "auctions_detail": records.iter().map(|r| json!({
            "path": r.path,
            "node": r.node_key,
            "resolution_kind": r.kind_key,
            "rate": r.rate,
            "requested": r.requested,
            "produced": r.produced,
            "ess": r.ess,
        })).collect::<Vec<_>>(),
    });

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/bridge-bidding is two levels under the workspace root");
    let target_dir = workspace_root.join("target");
    std::fs::create_dir_all(&target_dir).expect("create target/ directory");
    std::fs::write(
        target_dir.join("reproduction_report.json"),
        serde_json::to_string_pretty(&report).expect("report serializes"),
    )
    .expect("write target/reproduction_report.json");
}

/// The reproduction-rate harness (task brief / 11-testing.md §3): up to 500 real corpus auctions,
/// 1000 importance-weighted uniform samples each. `#[ignore]`d because it needs the vendored
/// corpus (`BRIDGE_CORPUS_DIR`, or the `corpus/data` symlink checked out alongside this worktree)
/// and takes noticeably longer than the default test suite. No pass/fail threshold is asserted
/// here (the task brief: "report the number; no threshold yet" -- phase 4's own completion
/// condition is a reported median >= 0.6, see `docs/design/11-testing.md` §3 and the roadmap).
#[test]
#[ignore = "needs BRIDGE_CORPUS_DIR (or the vendored corpus/data symlink); samples 1000 deals per auction, run with `cargo test --release -- --ignored`"]
fn sayc_reproduction_rate() {
    let Some(dir) = common::corpus_dir() else {
        eprintln!("sayc_reproduction_rate: no corpus directory found; skipping");
        return;
    };
    let auctions = corpus_auctions(&dir, 500);
    assert!(
        !auctions.is_empty(),
        "corpus directory {} yielded no auctions",
        dir.display()
    );

    let table = common::compile_sayc("sayc.bml");
    let bid_ctx = BidContext {
        scoring: Scoring::Imp,
        natural: Some(table.natural.as_ref()),
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::default(),
    };

    let started = std::time::Instant::now();
    let records: Vec<AuctionRecord> = auctions
        .iter()
        .enumerate()
        .map(|(i, auction)| {
            process_auction(&table, &bid_ctx, auction, auction_seed(0x5A1C_3001, i))
        })
        .collect();
    let elapsed = started.elapsed();

    let rates: Vec<f64> = records.iter().map(|r| r.rate).collect();
    let overall_median = median(&rates);
    write_json(&records, &dir);
    eprintln!(
        "sayc_reproduction_rate: {} auction(s), overall median reproduction rate = {:.4}, in \
         {elapsed:?}",
        records.len(),
        overall_median,
    );
}
