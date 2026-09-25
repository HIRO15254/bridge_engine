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
//!    hands are sampled from each and replayed through `choose_bid` on an empty `SystemIR` (so
//!    `ctx.natural` is what answers) to see how often the same call comes back out, aggregated by
//!    the rule that produced the constraint. This is the one measurement that needs
//!    `choose_bid`, which is why the harness lives in `bridge-bidding` rather than
//!    `bridge-system` (see `docs/design/11-testing.md` §6).
//! 3. **Corpus.** For every call of every parsed corpus auction (PBN + LIN, `corpus/data`,
//!    skipped when absent), the real dealt hand is checked against `infer`'s constraint: the
//!    satisfaction rate and the constraint's volume form a Pareto (a looser rule should satisfy
//!    more of the time at the cost of a larger volume), aggregated by rule.
//!
//! This is a measurement harness, not a correctness gate: neither design document sets a
//! threshold here (unlike, say, the recognition-ratio tests) -- the numbers landing in
//! `target/natural_metrics.json` are phase 3's completion criterion; tuning `NaturalParams`
//! against them is phase 4's job. Run once in release:
//! `cargo test -p bridge-bidding --release --test natural_metrics -- --ignored --nocapture`.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bridge_bidding::{BidChoice, BidContext, ImplicitPass, PolicyParams, Scoring, choose_bid};
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
    /// The worst 40 by recall (excluding expected misses), for spot-checking.
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
    by_rule: Vec<RuleAgreement>,
}

fn run_reproduction(sources: &[CompiledSource]) -> ReproductionReport {
    let natural = NaturalInference::default();
    let opts = SampleOptions::default();
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x5265_7072_6f31_3233); // "Repro123"

    let system = empty_system();
    let bid_ctx = BidContext {
        scoring: Scoring::Mp,
        natural: Some(&natural),
        implicit_pass: ImplicitPass::Never,
        policy: PolicyParams::default(),
    };

    // Non-opening decision points only (`choose_bid`'s empty-prefix root always resolves
    // "exact, zero children" rather than falling through to `ctx.natural`, so the very first
    // call of the auction cannot exercise this path -- see the module doc's measurement-2 note
    // and `crates/bridge-bidding/src/choose.rs`'s `gather`).
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
                    choose_bid(&system, hand, &prefix, &bid_ctx),
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

    ReproductionReport {
        n_decision_points,
        n_candidates_tested,
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
    for g in &hold_out.groups {
        eprintln!(
            "  {}/{}/{}: n={} recall_mean={:.3} precision_mean={:.3} log_vol_ratio_mean={:.2}",
            g.bucket, g.role, g.kind, g.n, g.recall_mean, g.precision_mean, g.log_volume_ratio_mean
        );
    }

    let reproduction = run_reproduction(&sources);
    eprintln!(
        "reproduction: {} decision point(s), {} candidate(s) tested, overall agreement {:.3}",
        reproduction.n_decision_points,
        reproduction.n_candidates_tested,
        reproduction.overall_agreement_rate
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
