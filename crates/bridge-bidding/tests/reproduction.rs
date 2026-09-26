//! Reproduction-rate property (07-bidding.md §10's reverse direction; 11-testing.md §3):
//! `consistency.rs` checks that `interpret` can always explain whatever `choose_bid` picked
//! (definitions too *tight*); this harness checks the opposite direction over real tournament
//! auctions -- does a deal `interpret` accepts for an auction actually *replay* into that same
//! auction via `choose_bid`? A node whose reproduction rate is low is a node whose constraint is
//! too *loose* (accepts hands that would never actually have produced that call).
//!
//! 11-testing.md §3's pseudocode samples deals from the auction's own interpretation and counts
//! the raw fraction that replays. `ConstraintProposal` is still `todo!()` (phase 5), so the
//! headline statistic here gets the same distribution by rejection: uniformly random deals are
//! kept only when every seat's hand strictly satisfies the auction's interpretation
//! (`Interpretation::satisfied_by` under `InterpretOptions { strict: true, .. }`), up to
//! [`TARGET_ACCEPTED`] kept deals out of at most [`MAX_DRAWS`] draws, and the rate is the raw
//! fraction of kept deals whose `replay` reproduces the auction. Which auctions enter the headline
//! median depends only on how many deals were kept (at least [`MIN_ACCEPTED`]), never on whether
//! they replay, so the statistic is not tied to the rate by construction.
//!
//! The earlier headline (phase 3 recheck finding) weighted uniform deals by
//! `sequence_log_likelihood` and took the median over auctions with ESS >= 30. A call `choose_bid`
//! would not make gets only `epsilon / n_legal` of the policy's mass, so one reproducing deal
//! carries almost all the weight (ESS about 1): a high ESS meant that *no* deal reproduced, and the
//! ESS filter selected exactly the auctions whose rate was 0. That likelihood-weighted rate is
//! still reported per auction, next to an `any_reproduced` flag, as a near-0/1 statistic -- it is
//! no longer used as a headline.

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
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;
use serde_json::json;

/// Deals kept per auction by the rejection sampler.
const TARGET_ACCEPTED: usize = 1000;
/// Uniform draws per auction before the rejection sampler gives up.
const MAX_DRAWS: usize = 200_000;
/// Kept deals an auction needs for its raw rate to enter the headline median.
const MIN_ACCEPTED: usize = 30;

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
    /// Raw fraction of the rejection-sampled deals (every seat strictly satisfies the
    /// interpretation) whose `choose_bid` replay reproduces this auction exactly; `None` when no
    /// deal was kept.
    raw_rate: Option<f64>,
    /// Deals the rejection sampler kept, and how many uniform draws it took.
    accepted: usize,
    draws: usize,
    /// The likelihood-weighted fraction of `requested` uniform samples whose replay reproduces
    /// this auction (a near-0/1 statistic, see the module doc).
    weighted_rate: f64,
    /// Whether any of the `requested` likelihood-weighted uniform samples replayed.
    any_reproduced: bool,
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

    let (raw_rate, accepted, draws) = rejection_rate(
        table,
        bid_ctx,
        auction,
        seed ^ 0xA5A5_5A5A_0F0F_F0F0,
        TARGET_ACCEPTED,
        MAX_DRAWS,
    );

    let log_weight_max = deals
        .iter()
        .map(|d| d.log_weight)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut weight_sum = 0.0f64;
    let mut reproduced_weight = 0.0f64;
    let mut any_reproduced = false;
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
            any_reproduced = true;
        }
    }
    let weighted_rate = if weight_sum > 0.0 {
        reproduced_weight / weight_sum
    } else {
        0.0
    };

    AuctionRecord {
        path: format!("{auction}"),
        node_key: node_key(&interp),
        kind_key: kind_key(&interp),
        raw_rate,
        accepted,
        draws,
        weighted_rate,
        any_reproduced,
        requested,
        produced: deals.len(),
        ess: sample_report.ess,
    }
}

/// Rejection-samples deals from `auction`'s own strict interpretation: uniform deals are kept when
/// every seat's hand satisfies it, until `target` are kept or `max_draws` were drawn. Returns the
/// raw fraction of kept deals whose `replay` reproduces `auction` (`None` if none was kept), the
/// kept count and the draw count.
fn rejection_rate(
    table: &bridge_bidding::Table,
    bid_ctx: &BidContext<'_>,
    auction: &Auction,
    seed: u64,
    target: usize,
    max_draws: usize,
) -> (Option<f64>, usize, usize) {
    let strict = InterpretOptions {
        strict: true,
        ..InterpretOptions::default()
    };
    let interp = interpret(table, auction, &strict);
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
    let (mut accepted, mut reproduced, mut draws) = (0usize, 0usize, 0usize);
    while accepted < target && draws < max_draws {
        draws += 1;
        let deal = common::random_deal(&mut rng);
        if !bridge_core::Seat::ALL
            .iter()
            .all(|&seat| interp.satisfied_by(seat, deal.hand(seat)))
        {
            continue;
        }
        accepted += 1;
        let replayed = replay(
            table,
            &deal,
            auction.dealer(),
            auction.vulnerability(),
            bid_ctx,
        );
        if replayed.auction == *auction {
            reproduced += 1;
        }
    }
    let rate = (accepted > 0).then(|| reproduced as f64 / accepted as f64);
    (rate, accepted, draws)
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

/// The `p`-quantile (`0.0..=1.0`, nearest-rank on the sorted values) of `values`; `0.0` if empty.
fn quantile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = (p * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

/// The ESS distribution of `records`: quantiles plus counts per bucket (`[1,2)`, `[2,5)`,
/// `[5,10)`, `[10,30)`, `[30,100)`, `[100,inf)`; an ESS below 1 falls into the first bucket).
fn ess_distribution(records: &[AuctionRecord]) -> serde_json::Value {
    let ess: Vec<f64> = records.iter().map(|r| r.ess).collect();
    let edges = [1.0, 2.0, 5.0, 10.0, 30.0, 100.0, f64::INFINITY];
    let labels = ["<2", "2-5", "5-10", "10-30", "30-100", ">=100"];
    let mut counts = [0usize; 6];
    for &e in &ess {
        let bucket = edges[1..].iter().position(|&hi| e < hi).unwrap_or(5);
        counts[bucket] += 1;
    }
    let buckets: serde_json::Map<String, serde_json::Value> = labels
        .iter()
        .zip(counts)
        .map(|(l, c)| ((*l).to_string(), json!(c)))
        .collect();
    json!({
        "min": quantile(&ess, 0.0),
        "p10": quantile(&ess, 0.1),
        "p25": quantile(&ess, 0.25),
        "median": quantile(&ess, 0.5),
        "p75": quantile(&ess, 0.75),
        "p90": quantile(&ess, 0.9),
        "max": quantile(&ess, 1.0),
        "buckets": buckets,
    })
}

/// The headline numbers: the median raw (rejection-sampled) rate over auctions with at least
/// [`MIN_ACCEPTED`] kept deals, how many such auctions there are, and the share of all auctions
/// for which any likelihood-weighted uniform sample replayed. The filter looks only at the kept
/// count, never at the replay outcome.
fn headline(records: &[AuctionRecord]) -> (f64, usize, f64) {
    let rates: Vec<f64> = records
        .iter()
        .filter(|r| r.accepted >= MIN_ACCEPTED)
        .filter_map(|r| r.raw_rate)
        .collect();
    let any = records.iter().filter(|r| r.any_reproduced).count();
    let any_share = if records.is_empty() {
        0.0
    } else {
        any as f64 / records.len() as f64
    };
    (median(&rates), rates.len(), any_share)
}

/// Groups `records` by `key`, sorted by descending group size, and computes each group's median
/// `rate` (11-testing.md §3: "中央値・分位点").
fn grouped_medians(
    records: &[AuctionRecord],
    key: impl Fn(&AuctionRecord) -> String,
) -> Vec<serde_json::Value> {
    use std::collections::HashMap;
    let mut groups: HashMap<String, Vec<f64>> = HashMap::new();
    for r in records.iter().filter(|r| r.accepted >= MIN_ACCEPTED) {
        if let Some(rate) = r.raw_rate {
            groups.entry(key(r)).or_default().push(rate);
        }
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
    let (median_raw, counted, any_share) = headline(records);
    let weighted: Vec<f64> = records.iter().map(|r| r.weighted_rate).collect();
    let report = json!({
        "auctions": records.len(),
        "corpus_dir": corpus_dir.display().to_string(),
        "method": "rejection sampling on the strict interpretation of every seat (uniform draws)",
        "target_accepted": TARGET_ACCEPTED,
        "max_draws": MAX_DRAWS,
        "min_accepted": MIN_ACCEPTED,
        "auctions_with_min_accepted": counted,
        "median_raw_rate": median_raw,
        "any_weighted_sample_reproduced_share": any_share,
        "weighted_rate_median_all": median(&weighted),
        "ess_distribution": ess_distribution(records),
        "by_node": grouped_medians(records, |r| r.node_key.clone()),
        "by_resolution_kind": grouped_medians(records, |r| r.kind_key.clone()),
        "auctions_detail": records.iter().map(|r| json!({
            "path": r.path,
            "node": r.node_key,
            "resolution_kind": r.kind_key,
            "raw_rate": r.raw_rate,
            "accepted": r.accepted,
            "draws": r.draws,
            "weighted_rate": r.weighted_rate,
            "any_reproduced": r.any_reproduced,
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
    // `SAYC_REPRO_LIMIT` caps the auction count (default 500) for a quick partial run.
    let limit: usize = match std::env::var("SAYC_REPRO_LIMIT") {
        Ok(v) => v.parse().expect("SAYC_REPRO_LIMIT is a valid usize"),
        Err(_) => 500,
    };
    let auctions = corpus_auctions(&dir, limit);
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
    // Auctions are independent (each has its own seed), so they are spread over a few threads;
    // results are put back in corpus order, so the report does not depend on the thread count.
    let threads = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(8);
    let mut slots: Vec<Option<AuctionRecord>> = (0..auctions.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let (table, bid_ctx, auctions) = (&table, &bid_ctx, &auctions);
                scope.spawn(move || {
                    (t..auctions.len())
                        .step_by(threads)
                        .map(|i| {
                            let seed = auction_seed(0x5A1C_3001, i);
                            (i, process_auction(table, bid_ctx, &auctions[i], seed))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for h in handles {
            for (i, record) in h.join().expect("reproduction worker panicked") {
                slots[i] = Some(record);
            }
        }
    });
    let records: Vec<AuctionRecord> = slots
        .into_iter()
        .map(|r| r.expect("every auction processed"))
        .collect();
    let elapsed = started.elapsed();

    let (median_raw, counted, any_share) = headline(&records);
    write_json(&records, &dir);
    eprintln!(
        "sayc_reproduction_rate: {} auction(s); median raw reproduction rate (rejection-sampled \
         from each auction's strict interpretation) over the {counted} auction(s) with >= \
         {MIN_ACCEPTED} kept deals = {median_raw:.4}; share of auctions any likelihood-weighted \
         uniform sample reproduced = {any_share:.3}; in {elapsed:?}",
        records.len(),
    );
}

/// The headline statistic is not tied to the rate: an auction many deals reproduce (SAYC passed
/// out, where every kept deal has four hands that open nothing) counts in it with a high rate.
/// Under the old ESS >= 30 filter such an auction could only enter with rate 0.
#[test]
fn headline_counts_an_auction_that_many_deals_reproduce() {
    let table = common::compile_sayc("sayc.bml");
    let bid_ctx = BidContext {
        scoring: Scoring::Imp,
        natural: Some(table.natural.as_ref()),
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::default(),
    };
    let pass = bridge_core::Call::Pass;
    let auction = Auction::from_calls(
        bridge_core::Seat::North,
        bridge_core::Vulnerability::None,
        vec![pass, pass, pass, pass],
    )
    .expect("passed-out auction is legal");
    let (rate, accepted, _) = rejection_rate(&table, &bid_ctx, &auction, 11, 60, 20_000);
    assert!(accepted >= MIN_ACCEPTED, "only {accepted} deals kept");
    let rate = rate.expect("some deal kept");
    assert!(rate > 0.5, "passed-out rate {rate}");

    let record = |raw_rate: Option<f64>, accepted: usize| AuctionRecord {
        path: String::new(),
        node_key: String::new(),
        kind_key: String::new(),
        raw_rate,
        accepted,
        draws: 0,
        weighted_rate: 0.0,
        any_reproduced: false,
        requested: 0,
        produced: 0,
        ess: 1.0,
    };
    let (median, counted, _) = headline(&[record(Some(rate), accepted), record(Some(0.0), 5)]);
    assert_eq!(counted, 1);
    assert_eq!(median, rate);
}
