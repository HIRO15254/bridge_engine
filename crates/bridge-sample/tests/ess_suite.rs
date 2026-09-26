//! Phase 5's ESS suite (`09-sample.md` §9, `11-testing.md` row `uniform_vs_constraint_ess`,
//! `12-roadmap.md` task 5.3, D20 of `15-phase4-plan.md`): 50 auctions, each interpreted with SAYC
//! as the mirror of the auction's own bidding policy (`InterpretOptions::for_context`) and sampled
//! with that policy's likelihood (`SampleContext::bidding = Some`), `n = 1000` deals per auction.
//! Criteria (phase 4, `11-testing.md` §13): with residual rejection, median ESS/n at least 0.5
//! overall and on the generated half (the corpus half is reported against a 0.4 target) with at
//! most 2 of the 50 cases exhausting the attempt budget; median ESS per attempt of
//! `ConstraintProposal::default()` at least 0.35. The time ratio of residual rejection to none
//! (target at most 2x) is printed, not asserted, since it depends on the machine's load.
//!
//! The 50 auctions:
//!
//! - 25 generated: random deals (fixed seeds, dealer rotating N/E/S/W, no one vulnerable) bid
//!   out with `bridge_bidding::replay` using SAYC at all four seats (with the same natural
//!   fallback the likelihood's policy uses), until the auction ends. Passed-out deals and
//!   slam-level runaway escalations are skipped (see `generated_cases`). Their likelihood uses
//!   `PolicyParams::system_players()` (the players bid exactly the system).
//! - 25 corpus: complete auctions with a complete deal from the eval split of the corpus (odd
//!   positions in the `corpus_auctions` enumeration of `crates/bridge-bidding/tests/
//!   reproduction.rs`: PBN files under `BRIDGE_CORPUS_DIR` or `<workspace>/corpus/data` in sorted
//!   order, every game whose view resolves an auction), 25 picked at evenly spaced positions.
//!   These were bid by humans with their own systems, so their likelihood uses
//!   `PolicyParams::human()`. The presets are fixed per source before any measurement.
//!
//! **Fixture.** In the default (eval) mode the cases are read from the fixture
//! `tests/data/ess_cases.txt` (auction and true deal per case), so changing the policy or the
//! system never changes the case set. `ESS_SUITE_WRITE_FIXTURE=1` regenerates the fixture from
//! the generator and the corpus eval split (and then runs on it); it is frozen once, at the
//! phase-4 integration. Without a fixture file the cases are generated on the fly.
//!
//! **Tuning mode.** `ESS_SUITE_MODE=tune` uses a disjoint seed set (other random deals, another
//! sampling seed) and the corpus tune split (even positions), never the fixture, and asserts
//! nothing: parameters may be looked at there, never on the eval cases.
//!
//! Every case runs three proposals on the same seed: `UniformProposal`, `ConstraintProposal`
//! without residual rejection and with it (`09-sample.md` §6.5). Per proposal the report has
//! ESS/n, ESS per attempt, the acceptance rate, whether the attempt budget ran out, and the wall
//! time; the summary adds the medians, the acceptance minimum, the wall time per effective sample
//! (`Σ elapsed / Σ ESS`), which decides whether residual rejection is on by default, and the
//! machine's load average.
//!
//! Known cards: the opening leader's hand (declarer's left-hand opponent), as in the lead
//! problem this sampler serves (phase 6, `14-lead.md`); the other three hands are sampled.
//! Setting `ESS_SUITE_KNOWN=none` samples all four hands instead (reported, not the criterion).
//!
//! Besides ESS, the report says *why* the weights spread (`ESS_SUITE_BREAKDOWN=0` skips it): for
//! every non-viewer call and every default-`ConstraintProposal` deal, whether the likelihood's
//! own policy (`call_distribution`) makes that call with the proposed hand (`p ≥ 0.9`), shares
//! it (`0.01 ≤ p < 0.9`), or rejects it (`p < 0.01`) although the hand is inside the call's own
//! interpretation (`off_inside_node`: the interpretation disagrees with the policy) or because the
//! hand is outside it (`off_outside_node`: the proposal drew it from a `Fallback` branch, a coarse
//! summary or the residual seat); and how many calls the deal's own hands make off-policy
//! (09-sample.md §10.2).
//!
//! `ESS_SUITE_CASES=a..b` runs only case indices `a..b` (0-24 generated, 25-49 corpus) so the
//! suite can be split into shorter runs; the criterion is only asserted when all cases ran.
//!
//! Writes `target/ess_report.json` (`target/ess_report_tune.json` in tuning mode). Run in
//! release:
//! `cargo test --release -p bridge-sample --all-features --test ess_suite -- --ignored --nocapture`

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use bridge_bidding::{
    BidContext, ImplicitPass, InterpretOptions, PolicyParams, Scoring, Table, interpret, replay,
};
use bridge_core::{Auction, Card, Deal, Hand, Seat, Vulnerability};
use bridge_sample::{
    BiddingLikelihood, ConstraintProposal, KnownCards, Proposal, SampleContext, SampleOptions,
    SampleReport, Threads, UniformProposal, rng_for, sample_deals,
};

const N: usize = 1000;
const GENERATED: usize = 25;
const CORPUS: usize = 25;
/// Master seed of the random deals the generated eval auctions are bid from.
const DEAL_SEED: u64 = 0x5A7C_0005_0003;
/// Master seed of every eval `sample_deals` call.
const SAMPLE_SEED: u64 = 0xE55;
/// Tuning mode's seeds (disjoint from the eval ones).
const TUNE_DEAL_SEED: u64 = 0x7E57_0005_0004;
const TUNE_SAMPLE_SEED: u64 = 0x7E55;

/// Which case set and seeds a run uses (`ESS_SUITE_MODE`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// The fixture (or its generator with the eval seeds) and the corpus eval split.
    Eval,
    /// Another seed set and the corpus tune split; asserts nothing.
    Tune,
}

impl Mode {
    fn from_env() -> Mode {
        match std::env::var("ESS_SUITE_MODE").as_deref() {
            Ok("tune") => Mode::Tune,
            Ok("eval") | Err(_) => Mode::Eval,
            Ok(other) => panic!("ESS_SUITE_MODE must be eval or tune, got {other:?}"),
        }
    }

    fn deal_seed(self) -> u64 {
        match self {
            Mode::Eval => DEAL_SEED,
            Mode::Tune => TUNE_DEAL_SEED,
        }
    }

    fn sample_seed(self) -> u64 {
        match self {
            Mode::Eval => SAMPLE_SEED,
            Mode::Tune => TUNE_SAMPLE_SEED,
        }
    }

    /// The corpus split's parity in the `corpus_auctions` enumeration (D20: even = tune, odd =
    /// eval).
    fn corpus_parity(self) -> usize {
        match self {
            Mode::Eval => 1,
            Mode::Tune => 0,
        }
    }
}

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

/// The likelihood's policy context for a case from `source`: `system_players()` for generated
/// auctions, `human()` for corpus ones (pre-registered, D18/D20).
fn bid_ctx(source: &str) -> BidContext<'static> {
    BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Complement,
        policy: match source {
            "generated" => PolicyParams::system_players(),
            _ => PolicyParams::human(),
        },
    }
}

/// A uniformly random deal from `rng_for(seed, i)` (Fisher-Yates over the 52 cards).
fn random_deal(seed: u64, i: u64) -> Deal {
    use rand_core::Rng;
    let mut rng = rng_for(seed, i);
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

fn generated_cases(table: &Table, seed: u64) -> Vec<Case> {
    // With the same natural fallback `sequence_log_likelihood` scores with, so every generated
    // call is the argmax of the very policy the importance weights use. Without it (`natural:
    // None`), `replay` turns every `NoCandidate` gap into a pass that the likelihood's own policy
    // (which does consult the natural fallback) scores at its ε floor for almost every hand,
    // including the deal's own: the auction is then off-policy by construction (09-sample.md
    // §10.2).
    let ctx = BidContext {
        natural: Some(table.natural.as_ref()),
        ..bid_ctx("generated")
    };
    let mut out = Vec::new();
    let mut i = 0u64;
    while out.len() < GENERATED {
        let deal = random_deal(seed, i);
        let dealer = Seat::ALL[(i % 4) as usize];
        let replayed = replay(table, &deal, dealer, Vulnerability::None, &ctx);
        // The natural fallback sometimes walks into a runaway escalation (both sides, or one
        // partnership, bidding on round after round up to the 7 level, e.g. `1H 1S P 2H X 2NT P
        // 3H X 3NT ... 7NT`): a `bridge-bidding` artefact, not an auction worth measuring, so
        // slam-level contracts are skipped (09-sample.md §10.2).
        let sensible = replayed
            .auction
            .contract()
            .is_some_and(|c| c.bid.level() <= 5);
        if !replayed.auction.is_passed_out() && sensible {
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

/// The corpus cases of one split: every game of the corpus's PBN files (sorted), enumerated as
/// `crates/bridge-bidding/tests/reproduction.rs`'s `corpus_auctions` does (each game whose view
/// resolves an auction takes the next index), keeping the indices of `parity` (D20: even = tune,
/// odd = eval) whose auction is complete with a contract and whose deal is complete; then
/// `CORPUS` of them at evenly spaced positions.
fn corpus_cases(dir: &Path, parity: usize) -> Vec<Case> {
    let mut all = Vec::new();
    let mut index = 0usize;
    for path in pbn_files(&dir.join("pbn")) {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let (file, _warnings) = bridge_format::pbn::parse_lenient(&bytes);
        let mut previous: Option<bridge_format::GameView> = None;
        for (k, game) in file.games.iter().enumerate() {
            let view = game.view(previous.as_ref()).ok();
            if let Some(view) = &view {
                if let Some(auction) = &view.auction {
                    let this = index;
                    index += 1;
                    let deal = view.deal.as_ref().and_then(|d| d.complete());
                    if let (true, Some(deal)) = (this % 2 == parity, deal) {
                        if auction.is_complete() && auction.contract().is_some() {
                            let name = path
                                .file_stem()
                                .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                            all.push(Case {
                                label: format!("{name}#{k}"),
                                source: "corpus",
                                deal,
                                auction: auction.clone(),
                            });
                        }
                    }
                }
            }
            previous = view;
        }
    }
    if all.len() <= CORPUS {
        return all;
    }
    let step = all.len() / CORPUS;
    all.into_iter().step_by(step).take(CORPUS).collect()
}

/// `tests/data/ess_cases.txt`.
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/ess_cases.txt")
}

/// One fixture line per case: `label<TAB>source<TAB>dealer<TAB>vulnerability<TAB>deal<TAB>calls`
/// (the deal in PBN form, the calls space-separated). `#` lines are comments.
fn write_fixture(path: &Path, cases: &[Case], note: &str) {
    let mut text = String::new();
    text.push_str(
        "# ESS suite cases (crates/bridge-sample/tests/ess_suite.rs, 09-sample.md §9).\n\
         # label\tsource\tdealer\tvulnerability\tdeal\tcalls\n",
    );
    let _ = writeln!(text, "# {note}");
    for case in cases {
        let _ = writeln!(
            text,
            "{}\t{}\t{}\t{}\t{}\t{}",
            case.label,
            case.source,
            case.auction.dealer(),
            case.auction.vulnerability(),
            case.deal,
            auction_text(&case.auction)
        );
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(path, text).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
}

/// Parses [`write_fixture`]'s format; `None` when the file does not exist.
fn read_fixture(path: &Path) -> Option<Vec<Case>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut cases = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            6,
            "{}:{}: expected 6 fields",
            path.display(),
            n + 1
        );
        let source = match fields[1] {
            "generated" => "generated",
            "corpus" => "corpus",
            other => panic!("{}:{}: unknown source {other:?}", path.display(), n + 1),
        };
        let dealer: Seat = fields[2].parse().expect("dealer");
        let vulnerability: Vulnerability = fields[3].parse().expect("vulnerability");
        let deal: Deal = fields[4].parse().expect("deal");
        let calls: Vec<bridge_core::Call> = fields[5]
            .split_whitespace()
            .map(|c| c.parse().expect("call"))
            .collect();
        let auction = Auction::from_calls(dealer, vulnerability, calls).expect("legal auction");
        cases.push(Case {
            label: fields[0].to_string(),
            source,
            deal,
            auction,
        });
    }
    Some(cases)
}

/// How the policy (`call_distribution`, the likelihood's own per-call factor) scores one
/// non-viewer call on one proposed hand.
#[derive(Clone, Copy)]
enum Verdict {
    /// `p ≥ 0.9`: the policy makes this call.
    On,
    /// `0.01 ≤ p < 0.9`: the call shares the policy's mass with others (equal-priority ties,
    /// several natural candidates).
    Shared,
    /// `p < 0.01` although the hand satisfies one of the call's own non-`Fallback`
    /// interpretation alternatives: the interpretation admits hands the policy bids differently.
    OffInsideNode,
    /// `p < 0.01` and the hand satisfies none of them: it came from a `Fallback` branch, a
    /// coarse (§6.4 (c)) summary, or the residual last seat.
    OffOutsideNode,
}

/// `[On, Shared, OffInsideNode, OffOutsideNode]` counts, per call resolution kind
/// `[Exact, Partial, Natural, Fallback]`.
type Breakdown = [[u64; 4]; 4];

fn kind_index(kind: bridge_bidding::ResolutionKind) -> usize {
    match kind {
        bridge_bidding::ResolutionKind::Exact => 0,
        bridge_bidding::ResolutionKind::Partial { .. } => 1,
        bridge_bidding::ResolutionKind::Natural => 2,
        bridge_bidding::ResolutionKind::Fallback => 3,
    }
}

const KIND_NAMES: [&str; 4] = ["exact", "partial", "natural", "fallback"];
const VERDICT_NAMES: [&str; 4] = ["on", "shared", "off_inside_node", "off_outside_node"];
/// One proposal's run on one case.
struct Run {
    ess_ratio: f64,
    ess_per_attempt: f64,
    produced: usize,
    attempts: u64,
    acceptance: f64,
    budget_exhausted: bool,
    seconds: f64,
    ess: f64,
}

impl Run {
    fn of(report: &SampleReport) -> Run {
        Run {
            ess_ratio: report.ess_ratio,
            ess_per_attempt: report.ess_per_attempt,
            produced: report.produced,
            attempts: report.attempts,
            acceptance: report.acceptance_rate,
            budget_exhausted: report.budget_exhausted,
            seconds: report.elapsed.as_secs_f64(),
            ess: report.ess,
        }
    }

    fn json(&self) -> String {
        format!(
            "{{\"ess_ratio\": {}, \"ess_per_attempt\": {}, \"produced\": {}, \"attempts\": {}, \
             \"acceptance_rate\": {}, \"budget_exhausted\": {}, \"seconds\": {}}}",
            json_f64(self.ess_ratio),
            json_f64(self.ess_per_attempt),
            self.produced,
            self.attempts,
            json_f64(self.acceptance),
            self.budget_exhausted,
            json_f64(self.seconds),
        )
    }
}

/// The proposals every case runs, in report order.
const PROPOSALS: [&str; 3] = ["uniform", "constraint", "constraint_residual"];

struct Row {
    label: String,
    source: &'static str,
    auction: String,
    viewer: Option<Seat>,
    /// One per [`PROPOSALS`] entry.
    runs: [Run; 3],
    /// Share of the default `ConstraintProposal`'s deals on which every non-viewer call has
    /// `p ≥ 0.01`.
    constraint_all_on_policy: f64,
    breakdown: Breakdown,
    /// Calls (of any seat) the deal's own hands make with `p < 0.01`: an auction the policy
    /// itself would (almost) never bid with the actual cards.
    true_deal_off_policy_calls: usize,
    calls: usize,
}

impl Row {
    /// The run of `ConstraintProposal::default()` (the criterion's proposal).
    fn default_constraint(&self) -> &Run {
        if ConstraintProposal::default().residual_rejection {
            &self.runs[2]
        } else {
            &self.runs[1]
        }
    }
}

fn run_case(table: &Table, case: &Case, known_none: bool, seed: u64, breakdown_on: bool) -> Row {
    let bctx = bid_ctx(case.source);
    // The mirror of the very policy the likelihood uses (D19), built from the same context.
    let interp = interpret(table, &case.auction, &InterpretOptions::for_context(&bctx));
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
        seed,
        threads: Threads::Auto,
        ..SampleOptions::default()
    };
    let run = |proposal: &dyn Proposal| {
        sample_deals(&ctx, proposal, N, &opts)
            .unwrap_or_else(|e| panic!("{}: sampling failed: {e}", case.label))
    };
    let (_, uniform) = run(&UniformProposal);
    let (plain_deals, plain) = run(&ConstraintProposal {
        residual_rejection: false,
        ..ConstraintProposal::default()
    });
    // `ESS_SUITE_RESIDUAL_MIN_ACCEPTANCE` overrides the residual variant's acceptance floor
    // (tuning mode only; the eval report records the value used).
    let (residual_deals, residual) = run(&ConstraintProposal {
        residual_rejection: true,
        residual_min_acceptance: residual_min_acceptance(),
        ..ConstraintProposal::default()
    });
    let deals = if ConstraintProposal::default().residual_rejection {
        residual_deals
    } else {
        plain_deals
    };

    // The likelihood's own policy, per call, exactly as `sequence_log_likelihood` evaluates it.
    let policy_ctx = BidContext {
        natural: Some(table.natural.as_ref()),
        ..bctx
    };
    let mut prefixes = Vec::with_capacity(case.auction.calls().len());
    let mut prefix = Auction::new(case.auction.dealer(), case.auction.vulnerability());
    for &call in case.auction.calls() {
        prefixes.push(prefix.clone());
        prefix.push(call).expect("legal auction");
    }
    let p_call = |j: usize, hand: Hand| -> f32 {
        let call = case.auction.calls()[j];
        bridge_bidding::call_distribution(table, hand, &prefixes[j], &policy_ctx)
            .iter()
            .find(|(c, _)| *c == call)
            .map_or(0.0, |(_, p)| *p)
    };

    let true_deal_off_policy_calls = (0..case.auction.calls().len())
        .filter(|&j| p_call(j, case.deal.hand(case.auction.seat_at(j))) < 0.01)
        .count();

    let mut breakdown: Breakdown = [[0; 4]; 4];
    let mut all_on = 0usize;
    for weighted in deals.iter().filter(|_| breakdown_on) {
        let mut on = true;
        for per_call in &interp.per_call {
            if Some(per_call.seat) == viewer {
                continue;
            }
            let hand = weighted.deal.hand(per_call.seat);
            let p = p_call(per_call.call_index, hand);
            let verdict = if p >= 0.9 {
                Verdict::On
            } else if p >= 0.01 {
                Verdict::Shared
            } else if per_call.alternatives.iter().any(|(c, w, e)| {
                *w > 0.0 && e.kind != bridge_bidding::ResolutionKind::Fallback && c.satisfies(hand)
            }) {
                Verdict::OffInsideNode
            } else {
                Verdict::OffOutsideNode
            };
            if matches!(verdict, Verdict::OffInsideNode | Verdict::OffOutsideNode) {
                on = false;
            }
            breakdown[kind_index(per_call.kind)][verdict as usize] += 1;
        }
        if on {
            all_on += 1;
        }
    }

    Row {
        label: case.label.clone(),
        source: case.source,
        auction: auction_text(&case.auction),
        viewer,
        runs: [Run::of(&uniform), Run::of(&plain), Run::of(&residual)],
        constraint_all_on_policy: if deals.is_empty() || !breakdown_on {
            f64::NAN
        } else {
            all_on as f64 / deals.len() as f64
        },
        breakdown,
        true_deal_off_policy_calls,
        calls: case.auction.calls().len(),
    }
}

fn breakdown_json(b: &Breakdown) -> String {
    let mut out = String::from("{");
    for (k, kind) in KIND_NAMES.iter().enumerate() {
        let _ = write!(out, "{}\"{kind}\": {{", if k > 0 { ", " } else { "" });
        for (v, verdict) in VERDICT_NAMES.iter().enumerate() {
            let _ = write!(
                out,
                "{}\"{verdict}\": {}",
                if v > 0 { ", " } else { "" },
                b[k][v]
            );
        }
        out.push('}');
    }
    out.push('}');
    out
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

/// The residual variant's `residual_min_acceptance`: `ESS_SUITE_RESIDUAL_MIN_ACCEPTANCE`, else
/// the default.
fn residual_min_acceptance() -> f64 {
    std::env::var("ESS_SUITE_RESIDUAL_MIN_ACCEPTANCE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(ConstraintProposal::default().residual_min_acceptance)
}

/// `sysctl -n vm.loadavg` (macOS) or `/proc/loadavg` (Linux), trimmed; `None` elsewhere.
fn loadavg() -> Option<String> {
    if let Ok(text) = std::fs::read_to_string("/proc/loadavg") {
        return Some(
            text.split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    let out = std::process::Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    Some(
        text.trim()
            .trim_start_matches('{')
            .trim_end_matches('}')
            .trim()
            .to_string(),
    )
}

/// Per-proposal summary over `rows`.
struct Summary {
    median_ess_ratio: f64,
    median_ess_ratio_generated: f64,
    median_ess_ratio_corpus: f64,
    median_ess_per_attempt: f64,
    median_ess_per_attempt_generated: f64,
    median_ess_per_attempt_corpus: f64,
    median_acceptance: f64,
    min_acceptance: f64,
    budget_exhausted: usize,
    seconds: f64,
    /// `Σ seconds / Σ ESS`: wall time per effective sample.
    seconds_per_effective_sample: f64,
    cases_ge_half: usize,
}

impl Summary {
    fn of(rows: &[Row], k: usize) -> Summary {
        let pick = |source: Option<&str>, f: &dyn Fn(&Run) -> f64| -> Vec<f64> {
            rows.iter()
                .filter(|r| source.is_none_or(|s| r.source == s))
                .map(|r| f(&r.runs[k]))
                .collect()
        };
        let ratio = |r: &Run| r.ess_ratio;
        let per_attempt = |r: &Run| r.ess_per_attempt;
        let acceptance = pick(None, &|r| r.acceptance);
        let seconds: f64 = rows.iter().map(|r| r.runs[k].seconds).sum();
        let ess: f64 = rows.iter().map(|r| r.runs[k].ess).sum();
        Summary {
            median_ess_ratio: median(&pick(None, &ratio)),
            median_ess_ratio_generated: median(&pick(Some("generated"), &ratio)),
            median_ess_ratio_corpus: median(&pick(Some("corpus"), &ratio)),
            median_ess_per_attempt: median(&pick(None, &per_attempt)),
            median_ess_per_attempt_generated: median(&pick(Some("generated"), &per_attempt)),
            median_ess_per_attempt_corpus: median(&pick(Some("corpus"), &per_attempt)),
            median_acceptance: median(&acceptance),
            min_acceptance: acceptance.iter().copied().fold(f64::INFINITY, f64::min),
            budget_exhausted: rows.iter().filter(|r| r.runs[k].budget_exhausted).count(),
            seconds,
            seconds_per_effective_sample: if ess > 0.0 { seconds / ess } else { f64::NAN },
            cases_ge_half: rows.iter().filter(|r| r.runs[k].ess_ratio >= 0.5).count(),
        }
    }

    fn json(&self) -> String {
        format!(
            "{{\"median_ess_ratio\": {}, \"median_ess_ratio_generated\": {}, \
             \"median_ess_ratio_corpus\": {}, \"median_ess_per_attempt\": {}, \
             \"median_ess_per_attempt_generated\": {}, \"median_ess_per_attempt_corpus\": {}, \
             \"median_acceptance_rate\": {}, \"min_acceptance_rate\": {}, \
             \"budget_exhausted_cases\": {}, \"seconds\": {}, \
             \"seconds_per_effective_sample\": {}, \"cases_ess_ratio_ge_0_5\": {}}}",
            json_f64(self.median_ess_ratio),
            json_f64(self.median_ess_ratio_generated),
            json_f64(self.median_ess_ratio_corpus),
            json_f64(self.median_ess_per_attempt),
            json_f64(self.median_ess_per_attempt_generated),
            json_f64(self.median_ess_per_attempt_corpus),
            json_f64(self.median_acceptance),
            json_f64(self.min_acceptance),
            self.budget_exhausted,
            json_f64(self.seconds),
            json_f64(self.seconds_per_effective_sample),
            self.cases_ge_half,
        )
    }
}

/// The cases of `mode`: in eval mode the fixture when present (regenerated first under
/// `ESS_SUITE_WRITE_FIXTURE=1`), else the generator and the corpus eval split; in tuning mode
/// always the generator with the tuning seed and the corpus tune split. Returns the cases and a
/// note on where they came from.
fn load_cases(table: &Table, mode: Mode) -> (Vec<Case>, String) {
    let path = fixture_path();
    let write = std::env::var("ESS_SUITE_WRITE_FIXTURE").is_ok_and(|v| v == "1");
    if mode == Mode::Eval && !write {
        if let Some(cases) = read_fixture(&path) {
            return (cases, format!("fixture {}", path.display()));
        }
    }
    let mut cases = generated_cases(table, mode.deal_seed());
    let note = match corpus_dir() {
        Some(dir) => {
            let found = corpus_cases(&dir, mode.corpus_parity());
            let note = format!(
                "{} generated (deal seed {:#x}) + {} corpus auctions ({} split) from {}",
                cases.len(),
                mode.deal_seed(),
                found.len(),
                if mode == Mode::Eval { "eval" } else { "tune" },
                dir.strip_prefix(workspace_dir()).unwrap_or(&dir).display()
            );
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
    if mode == Mode::Eval && write {
        write_fixture(&path, &cases, &note);
        println!("wrote {}", path.display());
    }
    (cases, note)
}

#[test]
#[ignore = "statistical benchmark over 50 auctions; run in release on demand"]
fn uniform_vs_constraint_ess_suite() {
    let suite_start = Instant::now();
    let load_before = loadavg();
    let table = Table::uniform(
        Arc::new(compile_sayc()),
        Arc::new(bridge_bidding::NaturalInference::default()),
    );
    let mode = Mode::from_env();
    let known_none = std::env::var("ESS_SUITE_KNOWN").is_ok_and(|v| v == "none");
    let breakdown_on = std::env::var("ESS_SUITE_BREAKDOWN").map_or(true, |v| v != "0");
    let range = case_range();

    let (cases, corpus_note) = load_cases(&table, mode);
    let total_cases = cases.len();

    let mut rows = Vec::new();
    for (idx, case) in cases.iter().enumerate() {
        if !range.contains(&idx) {
            continue;
        }
        let row = run_case(&table, case, known_none, mode.sample_seed(), breakdown_on);
        let [u, c, r] = &row.runs;
        println!(
            "{idx:>2} {:<9} {:<20} uniform {:>6.4} | constraint {:>6.4} (per attempt {:.4}, \
             acc {:.3}, {:.2}s) | residual {:>6.4} (per attempt {:.4}, acc {:.3}{}, {:.2}s) | \
             all-on-policy {:.3}, true-deal off-policy calls {}/{}  {}",
            row.source,
            row.label,
            u.ess_ratio,
            c.ess_ratio,
            c.ess_per_attempt,
            c.acceptance,
            c.seconds,
            r.ess_ratio,
            r.ess_per_attempt,
            r.acceptance,
            if r.budget_exhausted {
                ", EXHAUSTED"
            } else {
                ""
            },
            r.seconds,
            row.constraint_all_on_policy,
            row.true_deal_off_policy_calls,
            row.calls,
            row.auction
        );
        rows.push(row);
    }
    let load_after = loadavg();
    let suite_seconds = suite_start.elapsed().as_secs_f64();

    let summaries: Vec<Summary> = (0..PROPOSALS.len())
        .map(|k| Summary::of(&rows, k))
        .collect();
    let default_k = if ConstraintProposal::default().residual_rejection {
        2
    } else {
        1
    };
    let criterion = &summaries[default_k];
    let plain = &summaries[1];
    let residual = &summaries[2];
    for (name, s) in PROPOSALS.iter().zip(&summaries) {
        println!(
            "{name:<20} median ESS/n {:.4} (generated {:.4}, corpus {:.4}); ESS per attempt \
             {:.4} (generated {:.4}, corpus {:.4}); acceptance median {:.3} min {:.4}; \
             exhausted {}; {:.2}s, {:.3e} s per effective sample; {} cases >= 0.5",
            s.median_ess_ratio,
            s.median_ess_ratio_generated,
            s.median_ess_ratio_corpus,
            s.median_ess_per_attempt,
            s.median_ess_per_attempt_generated,
            s.median_ess_per_attempt_corpus,
            s.median_acceptance,
            s.min_acceptance,
            s.budget_exhausted,
            s.seconds,
            s.seconds_per_effective_sample,
            s.cases_ge_half,
        );
    }
    println!(
        "residual rejection / plain: sampling time {:.2}x, time per effective sample {:.2}x",
        residual.seconds / plain.seconds,
        residual.seconds_per_effective_sample / plain.seconds_per_effective_sample,
    );
    println!(
        "{} auctions ({corpus_note}); mode {mode:?}; suite {suite_seconds:.1}s; loadavg {} -> {}",
        rows.len(),
        load_before.as_deref().unwrap_or("?"),
        load_after.as_deref().unwrap_or("?"),
    );

    let mut json = String::new();
    json.push_str("{\n");
    let _ = writeln!(json, "  \"n\": {N},");
    let _ = writeln!(
        json,
        "  \"mode\": {},",
        json_str(if mode == Mode::Eval { "eval" } else { "tune" })
    );
    let _ = writeln!(json, "  \"sample_seed\": {},", mode.sample_seed());
    let _ = writeln!(
        json,
        "  \"residual_min_acceptance\": {},",
        json_f64(residual_min_acceptance())
    );
    let _ = writeln!(
        json,
        "  \"known\": {},",
        json_str(if known_none { "none" } else { "opening_leader" })
    );
    let _ = writeln!(
        json,
        "  \"policy\": {{\"generated\": \"system_players\", \"corpus\": \"human\"}},"
    );
    let _ = writeln!(json, "  \"cases\": {},", json_str(&corpus_note));
    let _ = writeln!(json, "  \"cases_run\": {},", rows.len());
    let _ = writeln!(json, "  \"cases_total\": {total_cases},");
    let _ = writeln!(json, "  \"suite_seconds\": {},", json_f64(suite_seconds));
    let _ = writeln!(
        json,
        "  \"loadavg_before\": {},",
        load_before.as_deref().map_or("null".to_string(), json_str)
    );
    let _ = writeln!(
        json,
        "  \"loadavg_after\": {},",
        load_after.as_deref().map_or("null".to_string(), json_str)
    );
    let _ = writeln!(
        json,
        "  \"default_proposal\": {},",
        json_str(PROPOSALS[default_k])
    );
    // The headline numbers, for the default `ConstraintProposal` (the criterion's proposal).
    let _ = writeln!(
        json,
        "  \"median_constraint_ess_ratio\": {},",
        json_f64(criterion.median_ess_ratio)
    );
    let _ = writeln!(
        json,
        "  \"median_constraint_ess_ratio_generated\": {},",
        json_f64(criterion.median_ess_ratio_generated)
    );
    let _ = writeln!(
        json,
        "  \"median_constraint_ess_ratio_corpus\": {},",
        json_f64(criterion.median_ess_ratio_corpus)
    );
    let _ = writeln!(
        json,
        "  \"median_constraint_ess_per_attempt\": {},",
        json_f64(criterion.median_ess_per_attempt)
    );
    json.push_str("  \"proposals\": {");
    for (k, (name, s)) in PROPOSALS.iter().zip(&summaries).enumerate() {
        let _ = write!(
            json,
            "{}\"{name}\": {}",
            if k > 0 { ", " } else { "" },
            s.json()
        );
    }
    json.push_str("},\n");
    for source in ["generated", "corpus"] {
        let mut total: Breakdown = [[0; 4]; 4];
        for r in rows.iter().filter(|r| r.source == source) {
            for (total_row, row) in total.iter_mut().zip(&r.breakdown) {
                for (t, x) in total_row.iter_mut().zip(row) {
                    *t += x;
                }
            }
        }
        let _ = writeln!(
            json,
            "  \"policy_breakdown_{source}\": {},",
            breakdown_json(&total)
        );
        println!("policy breakdown ({source}): {}", breakdown_json(&total));
    }
    json.push_str("  \"auctions\": [\n");
    for (i, r) in rows.iter().enumerate() {
        let viewer = r
            .viewer
            .map_or_else(|| "null".to_string(), |s| json_str(&format!("{s:?}")));
        let _ = write!(
            json,
            "    {{\"label\": {}, \"source\": {}, \"auction\": {}, \"viewer\": {viewer}, ",
            json_str(&r.label),
            json_str(r.source),
            json_str(&r.auction),
        );
        for (name, run) in PROPOSALS.iter().zip(&r.runs) {
            let _ = write!(json, "\"{name}\": {}, ", run.json());
        }
        let _ = write!(
            json,
            "\"uniform_ess_ratio\": {}, \"constraint_ess_ratio\": {}, \
             \"constraint_all_on_policy\": {}, \"true_deal_off_policy_calls\": {}, \
             \"calls\": {}, \"policy_breakdown\": {}}}",
            json_f64(r.runs[0].ess_ratio),
            json_f64(r.default_constraint().ess_ratio),
            json_f64(r.constraint_all_on_policy),
            r.true_deal_off_policy_calls,
            r.calls,
            breakdown_json(&r.breakdown),
        );
        json.push_str(if i + 1 < rows.len() { ",\n" } else { "\n" });
    }
    json.push_str("  ]\n}\n");
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace_dir().join("target"), PathBuf::from);
    let _ = std::fs::create_dir_all(&target);
    let path = target.join(match mode {
        Mode::Eval => "ess_report.json",
        Mode::Tune => "ess_report_tune.json",
    });
    std::fs::write(&path, json).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
    println!("wrote {}", path.display());

    // The phase-4 criteria (11-testing.md §9): ESS/n >= 0.5 overall and on the generated cases
    // with residual rejection, which exhausts the attempt budget on at most 2 cases; ESS per
    // attempt >= 0.35 for the default proposal. The wall-time ratio is load-sensitive and only
    // printed above.
    if mode == Mode::Eval && rows.len() == total_cases && !known_none {
        assert!(
            residual.median_ess_ratio >= 0.5,
            "median ESS/n with residual rejection = {:.4} < 0.5",
            residual.median_ess_ratio
        );
        assert!(
            residual.median_ess_ratio_generated >= 0.5
                || residual.median_ess_ratio_generated.is_nan(),
            "median ESS/n with residual rejection on the generated cases = {:.4} < 0.5",
            residual.median_ess_ratio_generated
        );
        assert!(
            residual.budget_exhausted <= 2,
            "{} cases exhausted the attempt budget with residual rejection",
            residual.budget_exhausted
        );
        assert!(
            criterion.median_ess_per_attempt >= 0.35,
            "median ESS per attempt of the default ConstraintProposal = {:.4} < 0.35",
            criterion.median_ess_per_attempt
        );
    }
}

/// The fixture parses into the suite's 25 + 25 cases, each a complete auction with a contract
/// whose calls are legal (checked by `Auction::from_calls`) and whose deal is a full deal.
#[test]
fn ess_fixture_parses() {
    let path = fixture_path();
    let cases = read_fixture(&path).unwrap_or_else(|| panic!("{} is missing", path.display()));
    let count = |source: &str| cases.iter().filter(|c| c.source == source).count();
    assert_eq!(count("generated"), GENERATED);
    assert_eq!(count("corpus"), CORPUS);
    for case in &cases {
        assert!(
            case.auction.is_complete(),
            "{}: incomplete auction",
            case.label
        );
        assert!(
            case.auction.contract().is_some(),
            "{}: passed out",
            case.label
        );
    }
}
