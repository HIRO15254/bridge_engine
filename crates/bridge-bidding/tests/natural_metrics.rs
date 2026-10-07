//! Accuracy measurements for `NaturalInference` (decision D8; `docs/design/06-system.md` §8.5,
//! `docs/design/11-testing.md` §6, roadmap task 3.11's measurement part).
//!
//! There is no labelled ground truth for "is this natural-inference constraint right", so three
//! independent proxies are measured instead, each writing its headline numbers into
//! `target/natural_metrics.json`:
//!
//! 1. **Hold-out.** Every non-artificial, non-fully-alertable node of the compiled real systems
//!    (`systems/sayc/sayc.bml`, plus whichever vendored jdh8/gjp files compile with zero
//!    `Error`-severity lints -- vendored data is git-ignored and skipped when absent) is hidden:
//!    `classify`/`infer` are asked for the same auction and the same call, system-blind, and the
//!    result is compared against the node's own constraint. 1,000 hands are sampled from each
//!    side (`bridge_constraint::Sampler`) to estimate `recall = P(h ⊨ C_nat | h ~ C_sys)`,
//!    `precision = P(h ⊨ C_sys | h ~ C_nat)` and the log2 volume ratio, aggregated by
//!    `Role x CallKind`. `sayc.bml` is self-authored, so its hold-out numbers are reported in a
//!    separate bucket from the third-party ("vendor") files to keep the circularity visible.
//! 2. **Reproduction.** For every non-opening decision point in the same node set,
//!    `NaturalInference::candidates` is asked for every legal call's own natural constraint;
//!    hands are sampled from each and replayed through `choose_bid` on a `Table` of empty
//!    `SystemIR`s (so `ctx.natural` is what answers) to see how often the same call comes back
//!    out, aggregated by the rule that produced the constraint. This is the one measurement that needs
//!    `choose_bid`, which is why the harness lives in `bridge-bidding` rather than
//!    `bridge-system` (see `docs/design/11-testing.md` §6).
//! 3. **Corpus.** For every call of every parsed corpus auction (PBN + LIN, `corpus/data`,
//!    skipped when absent), the real dealt hand is checked against `infer`'s constraint: the
//!    satisfaction rate and the constraint's volume form a Pareto (a looser rule should satisfy
//!    more of the time at the cost of a larger volume), aggregated by rule.
//!
//! This is a measurement harness, not a correctness gate: neither design document sets a
//! threshold here (unlike, say, the recognition-ratio tests) -- the numbers landing in
//! `target/natural_metrics.json` are phase 3's completion criterion. Phase 4.6 tuned the rule
//! confidences and the level floor against them (`natural_tuning`, docs/design/06-system.md
//! §8.6): measurement 2 also reports a contextual rate that samples the constraints `choose_bid`
//! itself ranks, and the corpus is split by game index (even = tune, odd = eval) for true-deal
//! agreement. Run once in release:
//! `cargo test -p bridge-bidding --release --test natural_metrics -- --ignored --nocapture`.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bridge_bidding::{
    BidChoice, BidContext, ImplicitPass, PolicyParams, Scoring, Table, choose_bid,
    natural_partner_context,
};
use bridge_constraint::{HandConstraint, SampleOptions, Sampler};
use bridge_core::{Auction, Call, Deal, Hand, Seat, Vulnerability};
use bridge_format::pbn;
use bridge_system::ast::{SeatCond, Tri};
use bridge_system::natural::classify;
use bridge_system::{
    Alertability, AuctionTrie, CallKind, CompileOptions, NaturalInference, Node, Side as SysSide,
    SystemIR, SystemMeta,
};
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::{Rng, SeedableRng};
use serde::Serialize;

// --------------------------------------------------------------------------------------------
// Shared plumbing: locating files, compiling systems, reconstructing auctions from nodes.
// --------------------------------------------------------------------------------------------

/// Hold-out nodes per bucket, capped so a very large vendored file cannot blow up the run time
/// (`docs/design/11-testing.md` §6 sets no threshold, only that numbers come out).
const HOLDOUT_CAP_PER_BUCKET: usize = 1500;
/// Hands sampled from each side of a hold-out comparison (D8, §8.5, measurement 1: "1,000").
const HOLDOUT_SAMPLES: u32 = 1000;
/// Non-opening decision points used for the reproduction measurement (capped independently of
/// the hold-out cap: every decision point fans out into several candidate calls, each sampled).
const REPRO_CAP_DECISION_POINTS: usize = 600;
/// Hands sampled per natural candidate for the reproduction measurement.
const REPRO_SAMPLES_PER_CANDIDATE: u32 = 100;

/// `<workspace root>`, two levels above `crates/bridge-bidding`.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/bridge-bidding is two levels under the workspace root")
        .to_path_buf()
}

/// Every file with `extension` under `dir`, recursively, sorted; empty (not an error) if `dir`
/// does not exist.
fn files_with_ext(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files_with_ext(&path, extension));
        } else if path.extension().is_some_and(|e| e == extension) {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// One compiled real system, kept alongside where it came from for the report.
struct CompiledSource {
    /// `"sayc"`, `"jdh8"` or `"gjp"` (file provenance, kept for the node listing).
    origin: &'static str,
    /// Path relative to the workspace root.
    file: String,
    ir: SystemIR,
}

/// Compiles `path`; `None` when the file cannot be read, or compiles with any `Error`-severity
/// lint (the same "compiles" bar `compile_real.rs` uses in `bridge-system`) -- a system that
/// does not compile cleanly is not a trustworthy oracle for measurement 1.
fn compile_if_clean(path: &Path) -> Option<SystemIR> {
    let text = std::fs::read_to_string(path).ok()?;
    let opts = CompileOptions::default();
    let (ir, _lints) = bridge_system::compile(
        &path.to_string_lossy(),
        &text,
        &bridge_system::lexer::FsLoader,
        &opts,
    );
    let summary = bridge_system::lint::LintSummary::of(&ir.lints);
    if summary.errors > 0 {
        return None;
    }
    Some(ir)
}

/// `systems/sayc/sayc.bml`, plus every vendored jdh8/gjp file that compiles cleanly. Vendored
/// data is git-ignored (symlinked in from `systems/vendor/data`, see the module doc of
/// `crates/bridge-system/tests/compile_real.rs`) and simply contributes nothing when absent.
fn compiled_sources() -> Vec<CompiledSource> {
    let root = workspace_root();
    let mut out = Vec::new();

    let sayc_path = root.join("systems/sayc/sayc.bml");
    if let Some(ir) = compile_if_clean(&sayc_path) {
        out.push(CompiledSource {
            origin: "sayc",
            file: "systems/sayc/sayc.bml".to_string(),
            ir,
        });
    }

    let vendor_dir = root.join("systems/vendor/data");
    for rel in ["jdh8/blue.bml", "jdh8/wj.bml", "jdh8/defense.bml"] {
        let path = vendor_dir.join(rel);
        if let Some(ir) = compile_if_clean(&path) {
            out.push(CompiledSource {
                origin: "jdh8",
                file: format!("systems/vendor/data/{rel}"),
                ir,
            });
        }
    }
    for path in files_with_ext(&vendor_dir.join("gjp"), "bml") {
        if let Some(ir) = compile_if_clean(&path) {
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .display()
                .to_string();
            out.push(CompiledSource {
                origin: "gjp",
                file: rel,
                ir,
            });
        }
    }
    out
}

/// The opener position (`1..=4`) a [`SeatCond`] resolves to for auction reconstruction: the
/// least specific position it accepts (D17: `#SEAT` conditions never appear as literal leading
/// passes in `Node::calls`, so a concrete position has to be chosen here instead).
fn seat_position(cond: SeatCond) -> u8 {
    match cond {
        SeatCond::Any | SeatCond::First | SeatCond::FirstOrSecond => 1,
        SeatCond::Second => 2,
        SeatCond::Third | SeatCond::ThirdOrFourth => 3,
        SeatCond::Fourth => 4,
    }
}

/// Rebuilds a concrete [`Auction`] for `node`, plus the index and [`Seat`] of `node.call` within
/// it.
///
/// `Node::calls` is "from the opening bid", so a dealer of `Seat::North` is used throughout and
/// `seat_position(node.seat) - 1` leading passes are prepended (D17). Which physical partnership
/// is "us" (needed only to translate `node.vul`, which is relative to the system owner) follows
/// from `node.side` and the parity of `node.calls`'s own last index: the trie's `is_ours` rule
/// (`trie.rs`) is `is_ours(we_opened, i) = if i % 2 == 0 { we_opened } else { !we_opened }`,
/// solved for `we_opened` at `i = node.calls.len() - 1`. `classify`/`infer` themselves need
/// nothing about "us" vs "them" -- they read the reconstructed `Auction` and `owner` directly --
/// so this is the only place that distinction matters.
fn reconstruct(node: &Node) -> Option<(Auction, usize, Seat)> {
    if node.calls.is_empty() {
        return None;
    }
    let leading_passes = (seat_position(node.seat) - 1) as usize;
    let i_last = node.calls.len() - 1;
    let we_opened = if i_last % 2 == 0 {
        node.side == SysSide::Us
    } else {
        node.side == SysSide::Them
    };
    let opener_side_is_ns = leading_passes % 2 == 0;
    let we_side_is_ns = if we_opened {
        opener_side_is_ns
    } else {
        !opener_side_is_ns
    };
    let we_vul = node.vul.we == Tri::Yes;
    let they_vul = node.vul.they == Tri::Yes;
    let (ns_vul, ew_vul) = if we_side_is_ns {
        (we_vul, they_vul)
    } else {
        (they_vul, we_vul)
    };
    let vulnerability = match (ns_vul, ew_vul) {
        (false, false) => Vulnerability::None,
        (true, false) => Vulnerability::NS,
        (false, true) => Vulnerability::EW,
        (true, true) => Vulnerability::Both,
    };

    let mut calls = Vec::with_capacity(leading_passes + node.calls.len());
    calls.extend(std::iter::repeat_n(Call::Pass, leading_passes));
    calls.extend(node.calls.iter().copied());

    let auction = Auction::from_calls(Seat::North, vulnerability, calls).ok()?;
    let index = leading_passes + i_last;
    let owner = auction.seat_at(index);
    Some((auction, index, owner))
}

/// One eligible node, reconstructed into an auction, kept with its provenance.
struct DecisionPoint {
    bucket: &'static str, // "sayc" | "vendor" (jdh8 + gjp collapsed, D8's report split)
    origin: &'static str, // "sayc" | "jdh8" | "gjp"
    file: String,
    auction: Auction,
    index: usize,
    owner: Seat,
    node_constraint: HandConstraint,
}

/// Eligible nodes (§8.5 measurement 1's own filter: `!flags.artificial && alertable !=
/// Alertable`) from every compiled source, reconstructed into auctions. A node whose auction
/// fails to reconstruct (should not happen for a cleanly-compiled system; guarded rather than
/// asserted because a third-party BML file is not this lane's to fix) is silently skipped.
fn collect_decision_points(sources: &[CompiledSource]) -> Vec<DecisionPoint> {
    let mut out = Vec::new();
    for src in sources {
        let bucket = if src.origin == "sayc" {
            "sayc"
        } else {
            "vendor"
        };
        for node in &src.ir.nodes {
            if node.flags.artificial || node.alertable == Alertability::Alertable {
                continue;
            }
            let Some((auction, index, owner)) = reconstruct(node) else {
                continue;
            };
            out.push(DecisionPoint {
                bucket,
                origin: src.origin,
                file: src.file.clone(),
                auction,
                index,
                owner,
                node_constraint: node.constraint.clone(),
            });
        }
    }
    out
}

/// A uniformly random subset of `items` of size `cap` (partial Fisher-Yates), or `items`
/// unchanged if it is already that small. Deterministic given `rng`, so the report is
/// reproducible run to run.
fn cap_random<T>(mut items: Vec<T>, cap: usize, rng: &mut impl Rng) -> Vec<T> {
    let n = items.len();
    if n <= cap {
        return items;
    }
    for i in 0..cap {
        let j = i + (rng.next_u64() as usize) % (n - i);
        items.swap(i, j);
    }
    items.truncate(cap);
    items
}

/// A `SystemIR` with no nodes at all: every prefix is off-system, so `choose_bid` (with
/// `ctx.natural` set) answers purely from `NaturalInference::candidates`. Mirrors the private
/// `minimal_system` helper `bridge-system`'s own `ir.rs` unit tests use.
fn empty_system() -> SystemIR {
    SystemIR {
        meta: SystemMeta::default(),
        rows: Vec::new(),
        nodes: Vec::new(),
        index: AuctionTrie::new(),
        lints: Vec::new(),
        exclusive_cell: Default::default(),
    }
}

/// A short human-readable label for a [`CallKind`], coarser than its full `Debug` (the exact
/// booleans are noise for a report): the shape of the bid takes priority (`nt` > `cue` >
/// `reverse` > `raise` > `rebid_own` > `new_suit` > other), with `_Jump` appended when jumped.
fn kind_label(kind: CallKind) -> String {
    match kind {
        CallKind::Pass => "Pass".to_string(),
        CallKind::Redouble => "Redouble".to_string(),
        CallKind::Double(dk) => format!("Double({dk:?})"),
        CallKind::Bid {
            new_suit,
            raise,
            nt,
            jump,
            cue,
            reverse,
            rebid_own,
        } => {
            let base = if nt {
                "NT"
            } else if cue {
                "Cue"
            } else if reverse {
                "Reverse"
            } else if raise {
                "Raise"
            } else if rebid_own {
                "RebidOwn"
            } else if new_suit {
                "NewSuit"
            } else {
                "Other"
            };
            if jump > 0 {
                format!("Bid_{base}_Jump")
            } else {
                format!("Bid_{base}")
            }
        }
    }
}

/// The `p`-th quantile (`0.0..=1.0`) of `xs`, which must be non-empty. Nearest-rank, no
/// interpolation -- these are diagnostic numbers, not a statistical claim.
fn quantile(xs: &mut [f64], p: f64) -> f64 {
    xs.sort_by(|a, b| a.total_cmp(b));
    let i = ((xs.len() - 1) as f64 * p).round() as usize;
    xs[i]
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

// --------------------------------------------------------------------------------------------
// Measurement 1: hold-out.
// --------------------------------------------------------------------------------------------

#[derive(Serialize, Clone)]
struct NodeMetric {
    bucket: String,
    origin: String,
    file: String,
    auction: String,
    call: String,
    role: String,
    kind: String,
    rule: String,
    expected_miss: bool,
    recall: f64,
    precision: Option<f64>,
    log_volume_ratio: Option<f64>,
}

#[derive(Serialize)]
struct GroupStat {
    bucket: String,
    role: String,
    kind: String,
    n: usize,
    recall_mean: f64,
    recall_p10: f64,
    precision_n: usize,
    precision_mean: f64,
    precision_p10: f64,
    log_volume_ratio_mean: f64,
}

#[derive(Serialize)]
struct BucketSummary {
    bucket: String,
    files: Vec<String>,
    n_eligible_nodes: usize,
    n_evaluated: usize,
    n_expected_miss: usize,
    n_sys_unsatisfiable: usize,
}

#[derive(Serialize)]
struct HoldOutReport {
    buckets: Vec<BucketSummary>,
    groups: Vec<GroupStat>,
    /// Every evaluated node (§8.5: "ノード別一覧"), for diffing a run against a later one
    /// (e.g. before/after a `NaturalParams` sweep in phase 4) node by node. A few hundred rows,
    /// small enough to keep in full rather than only the worst ones below.
    nodes: Vec<NodeMetric>,
    /// The worst 40 by recall (excluding expected misses), for a quick spot-check without
    /// scanning all of `nodes`.
    worst_recall_nodes: Vec<NodeMetric>,
}

fn run_holdout(sources: &[CompiledSource]) -> HoldOutReport {
    let natural = NaturalInference::default();
    let opts = SampleOptions::default();
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x486f_6c64_4f75_7431); // "HoldOut1"

    let all_points = collect_decision_points(sources);

    let mut by_bucket: BTreeMap<&'static str, Vec<DecisionPoint>> = BTreeMap::new();
    for dp in all_points {
        by_bucket.entry(dp.bucket).or_default().push(dp);
    }

    let mut buckets = Vec::new();
    let mut metrics: Vec<NodeMetric> = Vec::new();

    for (bucket, points) in by_bucket {
        let n_eligible = points.len();
        let files: Vec<String> = {
            let mut fs: Vec<String> = points.iter().map(|p| p.file.clone()).collect();
            fs.sort();
            fs.dedup();
            fs
        };
        let points = cap_random(points, HOLDOUT_CAP_PER_BUCKET, &mut rng);

        let mut n_expected_miss = 0usize;
        let mut n_sys_unsatisfiable = 0usize;
        for dp in &points {
            let ctx = classify(&dp.auction, dp.index, dp.owner);
            let inf = natural.infer(&ctx);
            let call = dp.auction.calls()[dp.index];

            let sys_sampler = Sampler::prepare(&dp.node_constraint, Hand::FULL, Hand::EMPTY, &opts)
                .expect("Hand::FULL/Hand::EMPTY never overlap");
            if sys_sampler.count() == 0 {
                n_sys_unsatisfiable += 1;
                continue;
            }
            let mut recall_hits = 0u32;
            for _ in 0..HOLDOUT_SAMPLES {
                let hand = sys_sampler.sample(&mut rng).expect("count() > 0").hand;
                if inf.constraint.satisfies(hand) {
                    recall_hits += 1;
                }
            }
            let recall = recall_hits as f64 / HOLDOUT_SAMPLES as f64;

            let nat_sampler = Sampler::prepare(&inf.constraint, Hand::FULL, Hand::EMPTY, &opts)
                .expect("Hand::FULL/Hand::EMPTY never overlap");
            let (precision, log_volume_ratio) = if nat_sampler.count() == 0 {
                (None, None)
            } else {
                let mut precision_hits = 0u32;
                for _ in 0..HOLDOUT_SAMPLES {
                    let hand = nat_sampler.sample(&mut rng).expect("count() > 0").hand;
                    if dp.node_constraint.satisfies(hand) {
                        precision_hits += 1;
                    }
                }
                let precision = precision_hits as f64 / HOLDOUT_SAMPLES as f64;
                let ratio =
                    (nat_sampler.count() as f64).log2() - (sys_sampler.count() as f64).log2();
                (Some(precision), Some(ratio))
            };

            let expected_miss = inf.rule == "fallback";
            if expected_miss {
                n_expected_miss += 1;
            }

            metrics.push(NodeMetric {
                bucket: bucket.to_string(),
                origin: dp.origin.to_string(),
                file: dp.file.clone(),
                auction: dp
                    .auction
                    .calls()
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
                call: call.to_string(),
                role: format!("{:?}", ctx.role),
                kind: kind_label(ctx.kind),
                rule: inf.rule.to_string(),
                expected_miss,
                recall,
                precision,
                log_volume_ratio,
            });
        }

        buckets.push(BucketSummary {
            bucket: bucket.to_string(),
            files,
            n_eligible_nodes: n_eligible,
            n_evaluated: points.len() - n_sys_unsatisfiable,
            n_expected_miss,
            n_sys_unsatisfiable,
        });
    }

    // Role x CallKind aggregation, per bucket, excluding expected misses (§8.5: "reported
    // separately" -- a fallback constraint is `ANY`, whose recall/precision would just measure
    // the deck, not the rule table).
    type GroupKey = (String, String, String);
    type GroupSamples = (Vec<f64>, Vec<f64>, Vec<f64>);
    let mut groups_acc: BTreeMap<GroupKey, GroupSamples> = BTreeMap::new();
    for m in &metrics {
        if m.expected_miss {
            continue;
        }
        let key = (m.bucket.clone(), m.role.clone(), m.kind.clone());
        let entry = groups_acc.entry(key).or_default();
        entry.0.push(m.recall);
        if let Some(p) = m.precision {
            entry.1.push(p);
        }
        if let Some(v) = m.log_volume_ratio {
            entry.2.push(v);
        }
    }
    let mut groups: Vec<GroupStat> = groups_acc
        .into_iter()
        .map(
            |((bucket, role, kind), (mut recalls, mut precisions, log_vols))| GroupStat {
                bucket,
                role,
                kind,
                n: recalls.len(),
                recall_mean: mean(&recalls),
                recall_p10: quantile(&mut recalls, 0.10),
                precision_n: precisions.len(),
                precision_mean: mean(&precisions),
                precision_p10: if precisions.is_empty() {
                    0.0
                } else {
                    quantile(&mut precisions, 0.10)
                },
                log_volume_ratio_mean: mean(&log_vols),
            },
        )
        .collect();
    groups.sort_by(|a, b| (&a.bucket, &a.role, &a.kind).cmp(&(&b.bucket, &b.role, &b.kind)));

    let mut worst_recall_nodes: Vec<NodeMetric> = metrics
        .iter()
        .filter(|m| !m.expected_miss)
        .cloned()
        .collect();
    worst_recall_nodes.sort_by(|a, b| a.recall.total_cmp(&b.recall));
    worst_recall_nodes.truncate(40);

    HoldOutReport {
        buckets,
        groups,
        nodes: metrics,
        worst_recall_nodes,
    }
}

// --------------------------------------------------------------------------------------------
// Measurement 2: reproduction (needs `choose_bid`, hence this crate).
// --------------------------------------------------------------------------------------------

#[derive(Serialize)]
struct RuleAgreement {
    rule: String,
    n: usize,
    agreement_rate: f64,
}

#[derive(Serialize)]
struct ReproductionReport {
    n_decision_points: usize,
    n_candidates_tested: usize,
    overall_agreement_rate: f64,
    /// The same agreement with each candidate's hands drawn from the constraint `choose_bid`
    /// itself ranks (under its own partner context, `natural_partner_context`, so the natural
    /// level floor included; phase 4.6), over the same decision points with their own seeds. With
    /// `LevelFloor::NONE` the two definitions sample the same constraints.
    contextual_agreement_rate: f64,
    by_rule: Vec<RuleAgreement>,
}

fn run_reproduction(sources: &[CompiledSource]) -> ReproductionReport {
    let natural = std::sync::Arc::new(NaturalInference::default());
    let opts = SampleOptions::default();
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x5265_7072_6f31_3233); // "Repro123"

    let table = Table::uniform(std::sync::Arc::new(empty_system()), natural.clone());
    let bid_ctx = BidContext {
        scoring: Scoring::Mp,
        natural: Some(&natural),
        implicit_pass: ImplicitPass::Never,
        policy: PolicyParams::default(),
    };

    // Non-opening decision points only, by choice: the phase-3 definition of measurement 2
    // leaves the opening out. (On the empty system the root has no children, so `choose_bid`
    // would answer an opening from `ctx.natural` too; the corpus rates of `natural_tuning`
    // include openings.)
    let points: Vec<DecisionPoint> = collect_decision_points(sources)
        .into_iter()
        .filter(|dp| dp.index > 0)
        .collect();
    let points = cap_random(points, REPRO_CAP_DECISION_POINTS, &mut rng);
    let n_decision_points = points.len();

    let mut n_candidates_tested = 0usize;
    let mut overall_hits = 0usize;
    let mut overall_total = 0usize;
    let mut by_rule_acc: BTreeMap<String, (usize, usize)> = BTreeMap::new(); // rule -> (hits, total)

    for dp in &points {
        let prefix_calls = &dp.auction.calls()[..dp.index];
        let Ok(prefix) = Auction::from_calls(
            dp.auction.dealer(),
            dp.auction.vulnerability(),
            prefix_calls.iter().copied(),
        ) else {
            continue;
        };
        debug_assert_eq!(prefix.next_seat(), dp.owner);

        for (call, constraint, _priority) in natural.candidates(&prefix, dp.owner) {
            let next = prefix
                .with(call)
                .expect("candidates() only returns legal calls");
            let ctx = classify(&next, prefix.len(), dp.owner);
            let inf = natural.infer(&ctx);
            debug_assert_ne!(inf.rule, "fallback", "candidates() excludes fallback calls");

            let Ok(sampler) = Sampler::prepare(&constraint, Hand::FULL, Hand::EMPTY, &opts) else {
                continue;
            };
            if sampler.count() == 0 {
                continue;
            }
            n_candidates_tested += 1;

            let mut hits = 0usize;
            for _ in 0..REPRO_SAMPLES_PER_CANDIDATE {
                let hand = sampler.sample(&mut rng).expect("count() > 0").hand;
                let agrees = matches!(
                    choose_bid(&table, hand, &prefix, &bid_ctx),
                    BidChoice::Chosen(chosen) if chosen.call == call
                );
                if agrees {
                    hits += 1;
                }
            }
            overall_hits += hits;
            overall_total += REPRO_SAMPLES_PER_CANDIDATE as usize;
            let entry = by_rule_acc.entry(inf.rule.to_string()).or_default();
            entry.0 += hits;
            entry.1 += REPRO_SAMPLES_PER_CANDIDATE as usize;
        }
    }

    let mut by_rule: Vec<RuleAgreement> = by_rule_acc
        .into_iter()
        .map(|(rule, (hits, total))| RuleAgreement {
            rule,
            n: total,
            agreement_rate: if total == 0 {
                0.0
            } else {
                hits as f64 / total as f64
            },
        })
        .collect();
    by_rule.sort_by(|a, b| a.rule.cmp(&b.rule));

    let mut keys = RuleKeys::default();
    let (_, contextual) = reproduction_positions(&points, natural.params(), &mut keys, false);
    let contextual_agreement_rate = rate(tune_eval(&contextual, &keys.defaults(), false));

    ReproductionReport {
        n_decision_points,
        n_candidates_tested,
        contextual_agreement_rate,
        overall_agreement_rate: if overall_total == 0 {
            0.0
        } else {
            overall_hits as f64 / overall_total as f64
        },
        by_rule,
    }
}

// --------------------------------------------------------------------------------------------
// Measurement 3: corpus satisfaction (independent of any compiled system).
// --------------------------------------------------------------------------------------------

#[derive(Serialize)]
struct CorpusRulePoint {
    rule: String,
    n: usize,
    satisfaction_rate: f64,
    mean_log2_volume: f64,
}

#[derive(Serialize)]
struct CorpusReport {
    n_files: usize,
    n_games: usize,
    n_calls: usize,
    by_rule: Vec<CorpusRulePoint>,
}

/// `BRIDGE_CORPUS_DIR`, or `<workspace root>/corpus/data`; `None` when absent.
fn corpus_dir(root: &Path) -> Option<PathBuf> {
    let dir = match std::env::var_os("BRIDGE_CORPUS_DIR") {
        Some(d) => PathBuf::from(d),
        None => root.join("corpus/data"),
    };
    dir.is_dir().then_some(dir)
}

/// Every `(Deal, Auction)` pair the PBN and LIN corpora yield, dealer and vulnerability already
/// folded into `Auction` (so no separate handling is needed downstream).
fn corpus_games(dir: &Path) -> Vec<(Deal, Auction)> {
    let mut out = Vec::new();

    for path in files_with_ext(&dir.join("pbn"), "pbn") {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let (file, _warnings) = pbn::parse_lenient(&bytes);
        let mut previous: Option<pbn::GameView> = None;
        for game in &file.games {
            let view = game.view(previous.as_ref());
            if let Ok(v) = &view {
                if let (Some(partial), Some(auction)) = (&v.deal, &v.auction) {
                    if let Some(deal) = partial.complete() {
                        out.push((deal, auction.clone()));
                    }
                }
            }
            if let Ok(v) = view {
                previous = Some(v);
            }
        }
    }

    for path in files_with_ext(&dir.join("lin"), "lin") {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let (boards, _warnings) = bridge_format::lin::parse_lenient(&bytes);
        for board in &boards {
            let game = board.to_game();
            let Ok(view) = game.view(None) else { continue };
            if let (Some(partial), Some(auction)) = (&view.deal, &view.auction) {
                if let Some(deal) = partial.complete() {
                    out.push((deal, auction.clone()));
                }
            }
        }
    }

    out
}

fn run_corpus(root: &Path) -> Option<CorpusReport> {
    let dir = corpus_dir(root)?;
    let natural = NaturalInference::default();
    let opts = SampleOptions::default();

    let games = corpus_games(&dir);
    let n_files = files_with_ext(&dir.join("pbn"), "pbn").len()
        + files_with_ext(&dir.join("lin"), "lin").len();
    let n_games = games.len();
    let mut n_calls = 0usize;

    // rule -> (satisfied count, n, sum of log2 volume)
    let mut by_rule_acc: BTreeMap<String, (usize, usize, f64)> = BTreeMap::new();

    for (deal, auction) in &games {
        for index in 0..auction.calls().len() {
            let owner = auction.seat_at(index);
            let ctx = classify(auction, index, owner);
            let inf = natural.infer(&ctx);
            n_calls += 1;

            let hand = deal.hand(owner);
            let satisfied = inf.constraint.satisfies(hand);

            let log2_volume = Sampler::prepare(&inf.constraint, Hand::FULL, Hand::EMPTY, &opts)
                .ok()
                .map(|s| (s.count().max(1) as f64).log2())
                .unwrap_or(0.0);

            let entry = by_rule_acc.entry(inf.rule.to_string()).or_default();
            entry.1 += 1;
            if satisfied {
                entry.0 += 1;
            }
            entry.2 += log2_volume;
        }
    }

    let mut by_rule: Vec<CorpusRulePoint> = by_rule_acc
        .into_iter()
        .map(|(rule, (satisfied, n, sum_log2))| CorpusRulePoint {
            rule,
            n,
            satisfaction_rate: satisfied as f64 / n as f64,
            mean_log2_volume: sum_log2 / n as f64,
        })
        .collect();
    by_rule.sort_by(|a, b| a.rule.cmp(&b.rule));

    Some(CorpusReport {
        n_files,
        n_games,
        n_calls,
        by_rule,
    })
}

// --------------------------------------------------------------------------------------------
// The test.
// --------------------------------------------------------------------------------------------

#[derive(Serialize)]
struct NaturalMetricsReport {
    hold_out: HoldOutReport,
    reproduction: ReproductionReport,
    /// `None` when `corpus/data` is absent (06-system.md §8.5: measurement 3 depends on the
    /// phase-1 corpus and is skipped, not failed, until it is fetched).
    corpus: Option<CorpusReport>,
}

#[test]
#[ignore = "slow (samples real compiled systems + the corpus); run once in release, see the module doc"]
fn natural_inference_metrics() {
    let root = workspace_root();
    let sources = compiled_sources();
    eprintln!(
        "compiled sources: {}",
        sources
            .iter()
            .map(|s| s.file.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    // `systems/sayc/sayc.bml` is checked into the repo, not optional vendored data: unlike a
    // missing jdh8/gjp file, it compiling with an `Error`-severity lint (so `compile_if_clean`
    // drops it) is this lane's own regression, not an absent fixture. Catch it here rather than
    // silently reporting an empty, all-zero JSON (a real failure this measurement harness must
    // not hide -- see the reviewer finding on this test's missing assertions).
    assert!(
        sources.iter().any(|s| s.origin == "sayc"),
        "systems/sayc/sayc.bml must compile with zero Error-severity lints"
    );

    let hold_out = run_holdout(&sources);
    for b in &hold_out.buckets {
        eprintln!(
            "hold-out[{}]: {} eligible, {} evaluated ({} expected-miss, {} sys-unsatisfiable) over {} file(s)",
            b.bucket,
            b.n_eligible_nodes,
            b.n_evaluated,
            b.n_expected_miss,
            b.n_sys_unsatisfiable,
            b.files.len()
        );
    }
    let sayc_bucket =
        hold_out.buckets.iter().find(|b| b.bucket == "sayc").expect(
            "compiled_sources() checked above to include sayc, so run_holdout must bucket it",
        );
    assert!(
        sayc_bucket.n_evaluated > 0,
        "hold-out measurement 1 evaluated 0 sayc nodes: sayc.bml has no eligible (non-artificial, \
         non-alertable) nodes at all, or every one turned out system-unsatisfiable"
    );
    for g in &hold_out.groups {
        eprintln!(
            "  {}/{}/{}: n={} recall_mean={:.3} precision_mean={:.3} log_vol_ratio_mean={:.2}",
            g.bucket, g.role, g.kind, g.n, g.recall_mean, g.precision_mean, g.log_volume_ratio_mean
        );
    }

    let reproduction = run_reproduction(&sources);
    eprintln!(
        "reproduction: {} decision point(s), {} candidate(s) tested, overall agreement {:.3} \
         (contextual {:.3})",
        reproduction.n_decision_points,
        reproduction.n_candidates_tested,
        reproduction.overall_agreement_rate,
        reproduction.contextual_agreement_rate
    );
    assert!(
        reproduction.n_candidates_tested > 0,
        "reproduction measurement 2 tested 0 candidates: sayc.bml's non-opening decision points \
         must exercise NaturalInference::candidates"
    );

    let corpus = run_corpus(&root);
    match &corpus {
        Some(c) => eprintln!(
            "corpus: {} file(s), {} game(s), {} call(s)",
            c.n_files, c.n_games, c.n_calls
        ),
        None => eprintln!("corpus: corpus/data not found; skipping measurement 3"),
    }

    let report = NaturalMetricsReport {
        hold_out,
        reproduction,
        corpus,
    };
    let json = serde_json::to_string_pretty(&report).expect("NaturalMetricsReport serializes");
    let target_dir = root.join("target");
    std::fs::create_dir_all(&target_dir).expect("create target/ directory");
    std::fs::write(target_dir.join("natural_metrics.json"), json)
        .expect("write target/natural_metrics.json");
}

// --------------------------------------------------------------------------------------------
// Level floor (docs/design/06-system.md §8, lane-S acceptance "S, natural"): replay escalation.
// --------------------------------------------------------------------------------------------

/// SAYC compiled once, with the natural engine replaced by `params`.
fn sayc_table(params: bridge_system::NaturalParams) -> Table {
    let base = common::compile_sayc("sayc.bml");
    Table {
        natural: std::sync::Arc::new(NaturalInference::new(params)),
        ..base
    }
}

/// Final-contract levels `[passed out, 1..=7]` of `n` fixed-seed random deals replayed with
/// SAYC plus natural completion (dealer rotating, vulnerability rotating), and the number of
/// replays with a forced-pass gap.
fn level_histogram(table: &Table, n: usize, seed: u64) -> ([usize; 8], usize) {
    let ctx = BidContext {
        scoring: Scoring::Imp,
        natural: Some(table.natural.as_ref()),
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::system_players(),
    };
    let vuls = [
        Vulnerability::None,
        Vulnerability::NS,
        Vulnerability::EW,
        Vulnerability::Both,
    ];
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
    let mut by_level = [0usize; 8];
    let mut with_gaps = 0;
    for i in 0..n {
        let deal = common::random_deal(&mut rng);
        let r = bridge_bidding::replay(table, &deal, Seat::ALL[i % 4], vuls[(i / 4) % 4], &ctx);
        let level = r.auction.contract().map_or(0, |c| c.bid.level() as usize);
        by_level[level] += 1;
        with_gaps += usize::from(!r.gaps.is_empty());
    }
    (by_level, with_gaps)
}

/// The level-floor seed of the acceptance run.
const LEVEL_FLOOR_SEED: u64 = 0x1e7e_1f10;

fn assert_level_floor(n: usize) {
    let started = std::time::Instant::now();
    let floored = sayc_table(bridge_system::NaturalParams::default());
    let (hist, gaps) = level_histogram(&floored, n, LEVEL_FLOOR_SEED);
    let seven = hist[7];
    let six_plus = hist[6] + hist[7];
    eprintln!(
        "level floor (default table): final levels [passout, 1..7] = {hist:?}, {gaps} replay(s) \
         with gaps, {n} deals in {:?}",
        started.elapsed()
    );
    assert!(seven * 100 <= n, "7-level contracts {seven}/{n} > 1%");
    assert!(
        six_plus * 100 <= 5 * n,
        "6+-level contracts {six_plus}/{n} > 5%"
    );
}

/// Default-suite version (200 deals).
#[test]
fn level_floor_limits_replay_escalation() {
    assert_level_floor(200);
}

/// The acceptance size (2000 fixed-seed deals), with the no-floor baseline for comparison:
/// `cargo test -p bridge-bidding --release --test natural_metrics -- --ignored level_floor`.
#[test]
#[ignore = "2000 replays twice; run in release with --ignored --nocapture"]
fn level_floor_limits_replay_escalation_2000() {
    let none = bridge_system::NaturalParams {
        level_floor: bridge_system::LevelFloor::NONE,
        ..Default::default()
    };
    let (hist, gaps) = level_histogram(&sayc_table(none), 2000, LEVEL_FLOOR_SEED);
    eprintln!(
        "level floor NONE: final levels [passout, 1..7] = {hist:?}, {gaps} replay(s) with gaps"
    );
    assert_level_floor(2000);
}

// --------------------------------------------------------------------------------------------
// infer_batch == per-call infer at every position of generated and corpus auctions.
// --------------------------------------------------------------------------------------------

/// Checks `infer_batch` over every legal call at every position of `auction`, under the default
/// partner context and a known partner range (which exercises the level floor).
fn check_batch_positions(engine: &NaturalInference, auction: &Auction) -> usize {
    use bridge_constraint::Atom;
    use bridge_system::PartnerContext;

    let partners = [
        PartnerContext::default(),
        PartnerContext {
            partner_constraint: Some(HandConstraint::Atom(Atom::ANY.with_hcp(6..=10))),
            forcing_situation: false,
        },
    ];
    let mut checked = 0;
    for j in 0..auction.len() {
        let Ok(prefix) = Auction::from_calls(
            auction.dealer(),
            auction.vulnerability(),
            auction.calls()[..j].iter().copied(),
        ) else {
            break;
        };
        let owner = prefix.next_seat();
        let calls: Vec<Call> = prefix.legal_calls().collect();
        for partner in &partners {
            let batch = engine.infer_batch(&prefix, owner, partner, &calls);
            for (got, &call) in batch.iter().zip(&calls) {
                let next = prefix.with(call).expect("legal");
                let mut ctx = classify(&next, prefix.len(), owner);
                ctx.partner_constraint = partner.partner_constraint.clone();
                ctx.forcing_situation = partner.forcing_situation;
                let want = engine.infer(&ctx);
                assert_eq!(got.rule, want.rule, "{prefix:?} {call}");
                assert_eq!(got.confidence, want.confidence, "{prefix:?} {call}");
                assert_eq!(
                    format!("{:?}", got.constraint),
                    format!("{:?}", want.constraint),
                    "{prefix:?} {call}"
                );
                checked += 1;
            }
        }
    }
    checked
}

fn check_batch(n_generated: usize, n_corpus: usize) {
    let table = sayc_table(bridge_system::NaturalParams::default());
    let engine = table.natural.as_ref();
    let ctx = BidContext {
        scoring: Scoring::Imp,
        natural: Some(engine),
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::system_players(),
    };
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0xba7c_4001);
    let mut checked = 0;
    for i in 0..n_generated {
        let deal = common::random_deal(&mut rng);
        let r = bridge_bidding::replay(&table, &deal, Seat::ALL[i % 4], Vulnerability::None, &ctx);
        checked += check_batch_positions(engine, &r.auction);
    }
    let mut corpus = 0;
    if let Some(dir) = corpus_dir(&workspace_root()) {
        for (_, auction) in corpus_games(&dir).iter().take(n_corpus) {
            checked += check_batch_positions(engine, auction);
            corpus += 1;
        }
    }
    eprintln!(
        "infer_batch == infer: {n_generated} generated + {corpus} corpus auction(s), {checked} \
         (position, partner, call) checks"
    );
}

/// Default-suite version: 100 generated and 50 corpus auctions (corpus skipped when absent).
#[test]
fn infer_batch_matches_infer_on_generated_and_corpus_auctions() {
    check_batch(100, 50);
}

/// The acceptance size: 1000 generated and 500 corpus auctions.
#[test]
#[ignore = "1000 + 500 auctions; run in release with --ignored --nocapture"]
fn infer_batch_matches_infer_on_generated_and_corpus_auctions_full() {
    check_batch(1000, 500);
}

// --------------------------------------------------------------------------------------------
// Phase 4.6: tuning the natural rank (rule confidences) and the level floor on the corpus tune
// split (docs/design/15-phase4-plan.md step 5, D20).
// --------------------------------------------------------------------------------------------

/// `(rule, default priority)` pairs seen so far; a pair's position is its key in a priority
/// vector. A rule with two default confidences (`pass_default`, limited or not) gets two keys.
#[derive(Default)]
struct RuleKeys {
    keys: Vec<(&'static str, i16)>,
}

impl RuleKeys {
    fn id(&mut self, rule: &'static str, priority: i16) -> u16 {
        match self.keys.iter().position(|&k| k == (rule, priority)) {
            Some(i) => i as u16,
            None => {
                self.keys.push((rule, priority));
                (self.keys.len() - 1) as u16
            }
        }
    }

    fn defaults(&self) -> Vec<i16> {
        self.keys.iter().map(|k| k.1).collect()
    }
}

/// One decision position prepared for re-ranking under any rule-priority vector: the natural
/// candidates (call index, rule key) and the tested hands as `(mask of satisfied candidates,
/// target call index, multiplicity)`.
struct TunePos {
    cands: Vec<(u8, u16)>,
    pass_listed: bool,
    hands: Vec<(u64, u8, u32)>,
}

impl TunePos {
    fn new(ranked: &[bridge_system::NaturalCandidate], keys: &mut RuleKeys) -> TunePos {
        assert!(ranked.len() <= 64);
        TunePos {
            cands: ranked
                .iter()
                .map(|c| (c.call.index(), keys.id(c.rule, c.priority())))
                .collect(),
            pass_listed: ranked.iter().any(|c| c.call == Call::Pass),
            hands: Vec::new(),
        }
    }

    fn mask(ranked: &[bridge_system::NaturalCandidate], hand: Hand) -> u64 {
        ranked
            .iter()
            .enumerate()
            .filter(|(_, c)| c.constraint.satisfies(hand))
            .fold(0, |m, (i, _)| m | 1 << i)
    }

    fn add(&mut self, mask: u64, target: u8) {
        match self.hands.iter_mut().find(|h| h.0 == mask && h.1 == target) {
            Some(h) => h.2 += 1,
            None => self.hands.push((mask, target, 1)),
        }
    }
}

/// The natural policy's call over `positions` under rule priorities `pr` (priority descending,
/// then call index ascending: `TieBreak::RowOrder`'s natural order), as `(agreeing hands, all
/// hands)`. With `pass_when_none`, a hand satisfying no candidate passes (the natural implicit
/// pass under `ImplicitPass::Complement`, and `NoCandidate` read as a pass); otherwise it is a
/// miss (`ImplicitPass::Never`).
fn tune_eval(positions: &[TunePos], pr: &[i16], pass_when_none: bool) -> (u64, u64) {
    let pass = Call::Pass.index();
    let mut order: Vec<usize> = Vec::with_capacity(38);
    let (mut hits, mut total) = (0u64, 0u64);
    for p in positions {
        order.clear();
        order.extend(0..p.cands.len());
        order.sort_by(|&a, &b| {
            pr[p.cands[b].1 as usize]
                .cmp(&pr[p.cands[a].1 as usize])
                .then(p.cands[a].0.cmp(&p.cands[b].0))
        });
        for &(mask, target, n) in &p.hands {
            let chosen = order
                .iter()
                .find(|&&i| mask >> i & 1 == 1)
                .map(|&i| p.cands[i].0)
                .or_else(|| pass_when_none.then_some(pass));
            total += u64::from(n);
            if chosen == Some(target) {
                hits += u64::from(n);
            }
        }
    }
    (hits, total)
}

fn rate((hits, total): (u64, u64)) -> f64 {
    if total == 0 {
        0.0
    } else {
        hits as f64 / total as f64
    }
}

/// Measurement 2 prepared for re-ranking, per decision point: `bare` samples each call's
/// constraint from `NaturalInference::candidates` (the phase-3 definition, no partner context),
/// `contextual` samples the constraint of the ranked candidate `choose_bid` itself uses (under
/// its own partner context, `natural_partner_context`, so the level floor included). Hands are drawn with
/// a per-decision-point seed, so every parameter set sees the same hands for the same
/// constraint. With `check`, every bare hand's prediction under the default priorities is
/// asserted equal to `choose_bid` on the natural-only table.
fn reproduction_positions(
    points: &[DecisionPoint],
    params: &bridge_system::NaturalParams,
    keys: &mut RuleKeys,
    check: bool,
) -> (Vec<TunePos>, Vec<TunePos>) {
    let natural = std::sync::Arc::new(NaturalInference::new(params.clone()));
    let table = Table::uniform(std::sync::Arc::new(empty_system()), natural.clone());
    let bid_ctx = BidContext {
        scoring: Scoring::Mp,
        natural: Some(&natural),
        implicit_pass: ImplicitPass::Never,
        policy: PolicyParams::default(),
    };
    let opts = SampleOptions::default();
    let tie_break = bridge_system::TieBreak::default();
    let (mut bare, mut contextual) = (Vec::new(), Vec::new());
    for (k, dp) in points.iter().enumerate() {
        let Ok(prefix) = Auction::from_calls(
            dp.auction.dealer(),
            dp.auction.vulnerability(),
            dp.auction.calls()[..dp.index].iter().copied(),
        ) else {
            continue;
        };
        let partner = natural_partner_context(&table, &natural, &prefix, bid_ctx.implicit_pass);
        let ranked = natural.ranked_candidates(&prefix, dp.owner, &partner, tie_break);
        let mut pos_bare = TunePos::new(&ranked, keys);
        let mut pos_ctx = TunePos::new(&ranked, keys);
        let defaults = keys.defaults();
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x5475_6e65_0000_0000 ^ k as u64);
        let bare_constraints: Vec<(Call, HandConstraint)> = natural
            .candidates(&prefix, dp.owner)
            .into_iter()
            .map(|(call, c, _)| (call, c))
            .collect();
        let ctx_constraints: Vec<(Call, HandConstraint)> = ranked
            .iter()
            .map(|c| (c.call, c.constraint.clone()))
            .collect();
        for (pos, constraints, is_bare) in [
            (&mut pos_bare, &bare_constraints, true),
            (&mut pos_ctx, &ctx_constraints, false),
        ] {
            for (call, constraint) in constraints {
                let Ok(sampler) = Sampler::prepare(constraint, Hand::FULL, Hand::EMPTY, &opts)
                else {
                    continue;
                };
                if sampler.count() == 0 {
                    continue;
                }
                for _ in 0..REPRO_SAMPLES_PER_CANDIDATE {
                    let hand = sampler.sample(&mut rng).expect("count() > 0").hand;
                    let mask = TunePos::mask(&ranked, hand);
                    pos.add(mask, call.index());
                    if check && is_bare {
                        let one = TunePos {
                            cands: pos.cands.clone(),
                            pass_listed: pos.pass_listed,
                            hands: vec![(mask, call.index(), 1)],
                        };
                        let predicted = tune_eval(std::slice::from_ref(&one), &defaults, false);
                        let got = matches!(
                            choose_bid(&table, hand, &prefix, &bid_ctx),
                            BidChoice::Chosen(chosen) if chosen.call == *call
                        );
                        assert_eq!(predicted.0 == 1, got, "{prefix:?} {call} {hand:?}");
                    }
                }
            }
        }
        bare.push(pos_bare);
        contextual.push(pos_ctx);
    }
    (bare, contextual)
}

/// Every call of every corpus game prepared for re-ranking against the real call with the real
/// hand (true-deal agreement of the natural policy), split by game enumeration index: even
/// games tune, odd games evaluate. Candidates are ranked under `choose_bid`'s partner context
/// with `ImplicitPass::Complement`, the reading of a hand that fits no candidate that the
/// corpus rates (`tune_eval` with `pass_when_none`) assume.
fn corpus_positions(
    games: &[(Deal, Auction)],
    params: &bridge_system::NaturalParams,
    keys: &mut RuleKeys,
) -> (Vec<TunePos>, Vec<TunePos>) {
    let natural = std::sync::Arc::new(NaturalInference::new(params.clone()));
    let table = Table::uniform(std::sync::Arc::new(empty_system()), natural.clone());
    let tie_break = bridge_system::TieBreak::default();
    let (mut tune, mut eval) = (Vec::new(), Vec::new());
    for (g, (deal, auction)) in games.iter().enumerate() {
        for (j, &call) in auction.calls().iter().enumerate() {
            let Ok(prefix) = Auction::from_calls(
                auction.dealer(),
                auction.vulnerability(),
                auction.calls()[..j].iter().copied(),
            ) else {
                break;
            };
            let owner = prefix.next_seat();
            let partner =
                natural_partner_context(&table, &natural, &prefix, ImplicitPass::Complement);
            let ranked = natural.ranked_candidates(&prefix, owner, &partner, tie_break);
            let mut pos = TunePos::new(&ranked, keys);
            pos.add(TunePos::mask(&ranked, deal.hand(owner)), call.index());
            if g % 2 == 0 {
                tune.push(pos);
            } else {
                eval.push(pos);
            }
        }
    }
    (tune, eval)
}

/// Every position set of one parameter set, and its measured rates.
struct TuneSets {
    repro_bare: Vec<TunePos>,
    repro_ctx: Vec<TunePos>,
    corpus_tune: Vec<TunePos>,
    corpus_eval: Vec<TunePos>,
}

#[derive(Clone, Copy)]
struct TuneRates {
    /// Measurement 2, phase-3 definition, all decision points / tune half / eval half.
    repro_bare: [f64; 3],
    /// Measurement 2, contextual constraints, all / tune half / eval half.
    repro_ctx: [f64; 3],
    /// True-deal agreement on the corpus tune / eval split.
    corpus: [f64; 2],
}

impl std::fmt::Display for TuneRates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [ba, bt, be] = self.repro_bare;
        let [ca, ct, ce] = self.repro_ctx;
        let [pt, pe] = self.corpus;
        write!(
            f,
            "repro(phase-3) all/tune/eval {ba:.4}/{bt:.4}/{be:.4}, repro(contextual) \
             {ca:.4}/{ct:.4}/{ce:.4}, corpus true-deal tune/eval {pt:.4}/{pe:.4}"
        )
    }
}

impl TuneSets {
    fn build(
        points: &[DecisionPoint],
        games: &[(Deal, Auction)],
        params: &bridge_system::NaturalParams,
        keys: &mut RuleKeys,
        check: bool,
    ) -> TuneSets {
        let (repro_bare, repro_ctx) = reproduction_positions(points, params, keys, check);
        let (corpus_tune, corpus_eval) = corpus_positions(games, params, keys);
        TuneSets {
            repro_bare,
            repro_ctx,
            corpus_tune,
            corpus_eval,
        }
    }

    fn rates(&self, pr: &[i16]) -> TuneRates {
        let halves = |set: &[TunePos]| {
            let (tune, eval): (Vec<_>, Vec<_>) =
                set.iter().enumerate().partition(|(i, _)| i % 2 == 0);
            let sum = |v: Vec<(usize, &TunePos)>| {
                v.into_iter().fold((0, 0), |acc, (_, p)| {
                    let r = tune_eval(std::slice::from_ref(p), pr, false);
                    (acc.0 + r.0, acc.1 + r.1)
                })
            };
            [
                rate(tune_eval(set, pr, false)),
                rate(sum(tune)),
                rate(sum(eval)),
            ]
        };
        TuneRates {
            repro_bare: halves(&self.repro_bare),
            repro_ctx: halves(&self.repro_ctx),
            corpus: [
                rate(tune_eval(&self.corpus_tune, pr, true)),
                rate(tune_eval(&self.corpus_eval, pr, true)),
            ],
        }
    }

    /// The tuning objective: `w_corpus` times the true-deal agreement on the corpus tune split
    /// plus `1 - w_corpus` times the contextual measurement-2 agreement on the tune half of the
    /// decision points.
    fn objective(&self, pr: &[i16], w_corpus: f64) -> f64 {
        let tune_half: Vec<&TunePos> = self.repro_ctx.iter().step_by(2).collect();
        let (mut hits, mut total) = (0, 0);
        for p in tune_half {
            let r = tune_eval(std::slice::from_ref(p), pr, false);
            hits += r.0;
            total += r.1;
        }
        (1.0 - w_corpus) * rate((hits, total))
            + w_corpus * rate(tune_eval(&self.corpus_tune, pr, true))
    }
}

/// Coordinate descent over rule priorities `0, 5, …, 100` on [`TuneSets::objective`]; a value
/// replaces the current one only when it strictly improves the objective, and values are tried
/// nearest first, so the result moves each priority as little as the objective allows.
fn tune_priorities(sets: &TuneSets, start: &[i16], w_corpus: f64) -> (Vec<i16>, f64) {
    let mut pr = start.to_vec();
    let mut best = sets.objective(&pr, w_corpus);
    for _sweep in 0..6 {
        let mut improved = false;
        for k in 0..pr.len() {
            let keep = pr[k];
            let mut best_v = keep;
            // Nearest values first, so a plateau keeps the value closest to the current one.
            let mut values: Vec<i16> = (0..=20).map(|v| v * 5).collect();
            values.sort_by_key(|&v| ((v - keep).abs(), v));
            for v in values {
                pr[k] = v;
                let j = sets.objective(&pr, w_corpus);
                if j > best + 1e-9 {
                    best = j;
                    best_v = v;
                    improved = true;
                }
            }
            pr[k] = best_v;
        }
        if !improved {
            break;
        }
    }
    (pr, best)
}

/// Phase 4.6: for each candidate level-floor table, the measurement-2 agreement (phase-3 and
/// contextual definitions) and the true-deal agreement on the corpus tune/eval split, under the
/// default rule confidences and under confidences tuned on the tune split only
/// (`cargo test -p bridge-bidding --release --test natural_metrics -- --ignored --nocapture
/// natural_tuning`).
#[test]
#[ignore = "slow (samples every decision point and replays the corpus per floor table)"]
fn natural_tuning() {
    use bridge_system::{LevelFloor, NaturalParams};
    let root = workspace_root();
    let Some(dir) = corpus_dir(&root) else {
        eprintln!("corpus/data not found; skipping");
        return;
    };
    let games = corpus_games(&dir);
    let sources = compiled_sources();
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x5265_7072_6f31_3233);
    let points: Vec<DecisionPoint> = collect_decision_points(&sources)
        .into_iter()
        .filter(|dp| dp.index > 0)
        .collect();
    let points = cap_random(points, REPRO_CAP_DECISION_POINTS, &mut rng);
    let shift = |d: i16| {
        let f = |t: [u8; 7]| t.map(|v| if v == 0 { 0 } else { (v as i16 + d) as u8 });
        LevelFloor {
            suit: f(LevelFloor::STANDARD.suit),
            nt: f(LevelFloor::STANDARD.nt),
        }
    };
    let floors = [
        ("none", LevelFloor::NONE),
        ("standard", LevelFloor::STANDARD),
        ("standard-2", shift(-2)),
        ("standard+2", shift(2)),
        (
            "from-level-4",
            LevelFloor {
                suit: [0, 0, 0, 22, 26, 31, 35],
                nt: [0, 0, 0, 28, 30, 32, 36],
            },
        ),
    ];
    let w_list: Vec<f64> = std::env::var("TUNE_W")
        .ok()
        .map(|v| v.split(',').map(|x| x.parse().expect("TUNE_W")).collect())
        .unwrap_or_else(|| vec![0.5]);
    let only: Option<String> = std::env::var("TUNE_FLOOR").ok();
    let mut keys = RuleKeys::default();
    for (i, (name, floor)) in floors.into_iter().enumerate() {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let started = std::time::Instant::now();
        let params = NaturalParams {
            level_floor: floor,
            ..NaturalParams::default()
        };
        let sets = TuneSets::build(&points, &games, &params, &mut keys, i < 2);
        let defaults = keys.defaults();
        let base = sets.rates(&defaults);
        eprintln!("floor {name} ({:?})\n  default: {base}", started.elapsed());
        for &w in &w_list {
            let (tuned, j) = tune_priorities(&sets, &defaults, w);
            let after = sets.rates(&tuned);
            if let Ok(fixed) = std::env::var("TUNE_FIX") {
                // "rule:default=value,..." applied on top of the tuned vector and of the
                // defaults.
                for (label, start) in [("tuned", &tuned), ("defaults", &defaults)] {
                    let mut v = start.clone();
                    for item in fixed.split(',') {
                        let (key, value) = item.split_once('=').expect("rule:default=value");
                        let (rule, d) = key.split_once(':').expect("rule:default");
                        let d: i16 = d.parse().expect("default");
                        let k = keys
                            .keys
                            .iter()
                            .position(|&(r, p)| r == rule && p == d)
                            .expect("key");
                        v[k] = value.parse().expect("value");
                    }
                    eprintln!(
                        "  fixed {fixed} on the {label}: objective {:.4}, {}",
                        sets.objective(&v, w),
                        sets.rates(&v)
                    );
                }
            }
            eprintln!(
                "  w_corpus {w}: objective {:.4} -> {j:.4}\n  tuned:   {after}",
                sets.objective(&defaults, w),
            );
            let changed: Vec<String> = keys
                .keys
                .iter()
                .zip(&tuned)
                .filter(|((_, d), t)| d != *t)
                .map(|((r, d), t)| format!("{r}:{d}->{t}"))
                .collect();
            eprintln!("  changed priorities: {}", changed.join(", "));
        }
    }
    eprintln!(
        "decision points {}, corpus games {} (keys {:?})",
        points.len(),
        games.len(),
        keys.keys
    );
}
