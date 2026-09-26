//! Reproduction harness (07-bidding.md §10's reverse direction; 11-testing.md §3).
//!
//! `consistency.rs` checks that `interpret` can explain whatever `choose_bid` picked (definitions
//! too *tight*). This harness checks the opposite direction: does a deal the interpretation of an
//! auction accepts actually *replay* into that same auction via `choose_bid`? A low rate means an
//! interpretation that is too *loose*, or an auction the system would not bid.
//!
//! Phase 4 reports four numbers (docs/design/15-phase4-plan.md, criteria (b)):
//!
//! - **(i) generated**: 100 SAYC auctions frozen in `tests/data/repro_generated.txt`, made by
//!   replaying fixed-seed random deals with [`PolicyParams::system_players`]. SAYC is the right
//!   model for them by construction, so this is the headline set. `write_generated_fixture`
//!   regenerates the file (`SAYC_REPRO_WRITE_FIXTURE=1`) and otherwise reports how far the file
//!   has drifted from what the current system generates.
//! - **(ii) corpus SAYC-reproducible subset**: corpus games of the eval split (odd enumeration
//!   index, [`is_eval`]) whose true deal replays to the recorded auction. Its size is reported
//!   next to the rate.
//! - **(iii) legacy**: the phase-3 definition, kept for continuity: the first 500 corpus auctions
//!   of both splits, read with [`InterpretOptions::legacy`] and sampled with the phase-3
//!   rejection sampler ([`LEGACY_SAMPLER`]), plus the likelihood-weighted uniform rate and its
//!   `any_reproduced` flag.
//! - **(iv) per-call true-deal agreement**: on the eval split, how often `choose_bid` with the
//!   owner's true hand makes the recorded call, split into system and natural positions.
//!
//! ## Samplers
//!
//! Deals come from a [`Sampler`]. On the phase-4 line `ConstraintProposal` is still `todo!()`, so
//! the headline sampler is [`Sampler::StrictRejection`]: uniform deals are kept when every seat's
//! hand satisfies the strict (non-`Fallback`) interpretation, up to a kept target and a draw cap;
//! every kept deal has weight 1, the rate is the raw fraction that replays, and an auction enters
//! the median when at least [`MIN_ESS`] deals were kept. The filter looks only at the kept count,
//! never at the replay outcome.
//!
//! [`Sampler::Weighted`] draws with `sample_deals` from a proposal and weights each deal by the
//! policy likelihood (`BiddingLikelihood`) over the proposal density; its rate is the weighted
//! fraction that replays and its filter is ESS >= [`MIN_ESS`]. With [`ProposalKind::Uniform`] it is
//! the phase-3 likelihood-weighted statistic, which is near 0/1 because every off-policy call gets
//! only the `epsilon / n` floor (phase-3 recheck 3), so it is not a headline.
//!
//! **Phase 5 swap.** Once `ConstraintProposal` and `AuctionPolicy` weighting land, the headline
//! becomes `Sampler::Weighted { proposal: ProposalKind::Constraint, n: 1000 }`: change
//! [`headline_sampler`]'s default (or run with `SAYC_REPRO_SAMPLER=constraint` first). Nothing else
//! changes: the mirror interpretation is already built with [`InterpretOptions::for_context`]
//! from the same `BidContext` the likelihood uses, [`evaluate`] already handles weights, and the
//! reports already carry ESS and attempts. `BiddingLikelihood` switches to `AuctionPolicy` inside
//! `bridge-sample` (lane P), not here. On this line `SAYC_REPRO_SAMPLER=constraint` panics in
//! `ConstraintProposal::prepare`.

mod common;

use std::path::{Path, PathBuf};

use bridge_bidding::{
    BidChoice, BidContext, ChoiceSource, ImplicitPass, InterpretOptions, Interpretation, NodeId,
    PolicyParams, Rejected, ResolutionKind, Scoring, Table, choose_bid, interpret, replay,
};
use bridge_constraint::{HandConstraint, KnownCards};
use bridge_core::{Auction, Call, Deal, Seat, Vulnerability};
use bridge_sample::{
    BiddingLikelihood, ConstraintProposal, Proposal, SampleContext, SampleOptions, Threads,
    UniformProposal, sample_deals,
};
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;
use serde_json::{Value, json};

/// The generated fixture, relative to this crate's manifest directory.
const GENERATED_FIXTURE: &str = "tests/data/repro_generated.txt";
/// Auctions in the generated fixture.
const GENERATED_COUNT: usize = 100;
/// Seed of the generated fixture's random deals (deal `i` uses `auction_seed(GEN_SEED, i)`).
const GEN_SEED: u64 = 0x5A1C_4001;
/// Highest final-contract level a generated auction may reach (the natural fallback's runaway
/// escalation to the 7 level is a replay artefact until the level floor lands; 09-sample.md
/// §10.2 applies the same rule to the ESS suite).
const MAX_GENERATED_LEVEL: u8 = 5;
/// Kept deals (or ESS) an auction needs for its rate to enter a headline median.
const MIN_ESS: f64 = 30.0;
/// Default kept-deal target of the rejection sampler (`SAYC_REPRO_TARGET`).
const TARGET_ACCEPTED: usize = 1000;
/// Default uniform-draw cap per auction of the rejection sampler (`SAYC_REPRO_MAX_DRAWS`). A
/// strict SAYC interpretation of a whole auction accepts about 1e-4 of uniform deals, so the
/// phase-3 cap of 200,000 left 44 of the 100 generated auctions under 30 kept deals; a draw with
/// its check costs about 50 ns, so 5e6 draws are a quarter of a second per auction and thread.
const MAX_DRAWS: usize = 5_000_000;
/// The legacy part's sampler: the phase-3 definition (1000 kept deals, at most 200,000 draws),
/// fixed so that its numbers stay comparable across phases whatever the headline sampler is.
const LEGACY_SAMPLER: Sampler = Sampler::StrictRejection {
    target: 1000,
    max_draws: 200_000,
};
/// Deals per auction of the weighted samplers.
const WEIGHTED_N: usize = 1000;
/// Default number of corpus auctions of the legacy part (`SAYC_REPRO_LIMIT`).
const LEGACY_LIMIT: usize = 500;

// ------------------------------------------------------------------------------------------------
// Samplers
// ------------------------------------------------------------------------------------------------

/// The proposal a [`Sampler::Weighted`] draws from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ProposalKind {
    /// Uniformly random deals ([`UniformProposal`]).
    Uniform,
    /// The interpretation-driven proposal ([`ConstraintProposal`], phase 5).
    Constraint,
}

/// How deals are drawn for one auction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sampler {
    /// Uniform deals kept when every seat strictly satisfies the interpretation (weight 1), until
    /// `target` are kept or `max_draws` were drawn.
    StrictRejection { target: usize, max_draws: usize },
    /// `n` deals from `sample_deals` with `proposal`, weighted by the policy likelihood of the
    /// auction over the proposal density.
    Weighted { proposal: ProposalKind, n: usize },
}

impl Sampler {
    fn label(&self) -> String {
        match self {
            Sampler::StrictRejection { target, max_draws } => format!(
                "strict rejection: uniform deals kept when every seat satisfies the strict \
                 interpretation (target {target}, cap {max_draws} draws)"
            ),
            Sampler::Weighted { proposal, n } => format!(
                "{proposal:?} proposal, {n} deals weighted by the policy likelihood \
                 (BiddingLikelihood)"
            ),
        }
    }

    /// Draws deals for `auction`. `opts` is the interpretation the proposal reads (the rejection
    /// sampler makes it strict itself); `ctx` is the policy the weights use.
    fn draw(
        &self,
        table: &Table,
        ctx: &BidContext<'_>,
        auction: &Auction,
        opts: &InterpretOptions,
        seed: u64,
    ) -> Drawn {
        match *self {
            Sampler::StrictRejection { target, max_draws } => {
                let strict = InterpretOptions {
                    strict: true,
                    ..*opts
                };
                let interp = interpret(table, auction, &strict);
                let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
                let mut deals = Vec::new();
                let mut attempts = 0usize;
                while deals.len() < target && attempts < max_draws {
                    attempts += 1;
                    let deal = common::random_deal(&mut rng);
                    if Seat::ALL
                        .iter()
                        .all(|&seat| interp.satisfied_by(seat, deal.hand(seat)))
                    {
                        deals.push((deal, 1.0));
                    }
                }
                Drawn {
                    deals,
                    attempts,
                    error: None,
                }
            }
            Sampler::Weighted { proposal, n } => {
                let interp = interpret(table, auction, opts);
                let sample_ctx = SampleContext {
                    known: KnownCards::EMPTY,
                    interpretation: &interp,
                    play_constraints: &[HandConstraint::ANY; 4],
                    play_soft: None,
                    bidding: Some(BiddingLikelihood {
                        table,
                        auction,
                        ctx,
                    }),
                };
                let sample_opts = SampleOptions {
                    seed,
                    threads: Threads::Single,
                    ..SampleOptions::default()
                };
                let uniform = UniformProposal;
                let constraint = ConstraintProposal::default();
                let proposal: &dyn Proposal = match proposal {
                    ProposalKind::Uniform => &uniform,
                    ProposalKind::Constraint => &constraint,
                };
                match sample_deals(&sample_ctx, proposal, n, &sample_opts) {
                    Ok((weighted, report)) => {
                        let max = weighted
                            .iter()
                            .map(|d| d.log_weight)
                            .fold(f64::NEG_INFINITY, f64::max);
                        Drawn {
                            deals: weighted
                                .into_iter()
                                .map(|d| (d.deal, (d.log_weight - max).exp()))
                                .collect(),
                            attempts: report.attempts as usize,
                            error: None,
                        }
                    }
                    Err(e) => Drawn {
                        deals: Vec::new(),
                        attempts: 0,
                        error: Some(format!("{e:?}")),
                    },
                }
            }
        }
    }
}

/// The headline sampler: `SAYC_REPRO_SAMPLER` = `rejection` (default on the phase-4 line),
/// `constraint` (phase 5: `ConstraintProposal` + policy weights) or `uniform`. The kept target
/// and the draw cap of the rejection sampler come from `SAYC_REPRO_TARGET` and
/// `SAYC_REPRO_MAX_DRAWS`.
fn headline_sampler() -> Sampler {
    match std::env::var("SAYC_REPRO_SAMPLER").as_deref() {
        Ok("constraint") => Sampler::Weighted {
            proposal: ProposalKind::Constraint,
            n: WEIGHTED_N,
        },
        Ok("uniform") => Sampler::Weighted {
            proposal: ProposalKind::Uniform,
            n: WEIGHTED_N,
        },
        Ok("rejection") | Err(_) => Sampler::StrictRejection {
            target: env_usize("SAYC_REPRO_TARGET", TARGET_ACCEPTED),
            max_draws: env_usize("SAYC_REPRO_MAX_DRAWS", MAX_DRAWS),
        },
        Ok(other) => {
            panic!("SAYC_REPRO_SAMPLER={other}: expected rejection, constraint or uniform")
        }
    }
}

/// The deals drawn for one auction, with weights relative to the largest (1.0 for rejection).
struct Drawn {
    deals: Vec<(Deal, f64)>,
    /// Uniform draws (rejection) or proposal attempts (weighted).
    attempts: usize,
    /// `sample_deals` failed (for example `EmptySupport`).
    error: Option<String>,
}

/// One auction under one sampler.
#[derive(Clone, Debug)]
struct Outcome {
    /// Deals drawn (kept, for rejection).
    kept: usize,
    attempts: usize,
    /// `(sum w)^2 / sum w^2`; equals `kept` for rejection.
    ess: f64,
    /// Weighted fraction of the deals whose replay reproduces the auction; `None` if none drawn.
    rate: Option<f64>,
    any_reproduced: bool,
    error: Option<String>,
}

/// Draws deals for `auction` with `sampler` and replays each with `choose_bid`.
fn evaluate(
    sampler: &Sampler,
    table: &Table,
    ctx: &BidContext<'_>,
    auction: &Auction,
    opts: &InterpretOptions,
    seed: u64,
) -> Outcome {
    let drawn = sampler.draw(table, ctx, auction, opts, seed);
    let (mut sum, mut sum_sq, mut reproduced) = (0.0f64, 0.0f64, 0.0f64);
    let mut any_reproduced = false;
    for (deal, w) in &drawn.deals {
        sum += w;
        sum_sq += w * w;
        let replayed = replay(table, deal, auction.dealer(), auction.vulnerability(), ctx);
        if replayed.auction == *auction {
            reproduced += w;
            any_reproduced = true;
        }
    }
    Outcome {
        kept: drawn.deals.len(),
        attempts: drawn.attempts,
        ess: if sum_sq > 0.0 {
            sum * sum / sum_sq
        } else {
            0.0
        },
        rate: (sum > 0.0).then(|| reproduced / sum),
        any_reproduced,
        error: drawn.error,
    }
}

// ------------------------------------------------------------------------------------------------
// Contexts
// ------------------------------------------------------------------------------------------------

/// The policy context for `preset`: IMPs, the table's natural fallback, the implicit pass.
fn bid_ctx(table: &Table, preset: PolicyParams) -> BidContext<'_> {
    BidContext {
        scoring: Scoring::Imp,
        natural: Some(table.natural.as_ref()),
        implicit_pass: ImplicitPass::Complement,
        policy: preset,
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    match std::env::var(name) {
        Ok(v) => v
            .parse()
            .unwrap_or_else(|_| panic!("{name}={v} is not a usize")),
        Err(_) => default,
    }
}

/// A distinct sub-seed per auction, so every auction's draws are an independent, reproducible
/// random stream.
fn auction_seed(base: u64, index: usize) -> u64 {
    base.wrapping_add((index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// Runs `f` over `items` on up to 8 threads; results come back in `items` order, so nothing
/// reported depends on the thread count.
fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(usize, &T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .clamp(1, 8);
    let mut slots: Vec<Option<R>> = (0..items.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        let f = &f;
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                scope.spawn(move || {
                    (t..items.len())
                        .step_by(threads)
                        .map(|i| (i, f(i, &items[i])))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for h in handles {
            for (i, r) in h.join().expect("reproduction worker panicked") {
                slots[i] = Some(r);
            }
        }
    });
    slots
        .into_iter()
        .map(|r| r.expect("every item processed"))
        .collect()
}

// ------------------------------------------------------------------------------------------------
// Corpus
// ------------------------------------------------------------------------------------------------

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

/// One corpus game with a validated auction.
struct CorpusGame {
    /// Enumeration index: the split key ([`is_eval`]).
    index: usize,
    label: String,
    auction: Auction,
    /// The true deal, when all 52 cards are recorded.
    deal: Option<Deal>,
}

/// Every PBN game under `<dir>/pbn` whose view resolves an auction, in sorted-path then file
/// order. This enumeration order defines the corpus split (D20): even index = tune, odd = eval.
/// The two `Optimum*Table.pbn` reference files (no `Auction` section) and any truncated/`-` game
/// contribute nothing. A view that fails to interpret resets `#` inheritance.
fn corpus_games(dir: &Path) -> Vec<CorpusGame> {
    let mut games = Vec::new();
    for path in pbn_files(&dir.join("pbn")) {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let (file, _warnings) = bridge_format::pbn::parse_lenient(&bytes);
        let name = path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let parent = path
            .parent()
            .and_then(Path::file_name)
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let mut previous: Option<bridge_format::GameView> = None;
        for (k, game) in file.games.iter().enumerate() {
            let view = game.view(previous.as_ref()).ok();
            if let Some(view) = &view {
                if let Some(auction) = &view.auction {
                    games.push(CorpusGame {
                        index: games.len(),
                        label: format!("{parent}/{name}#{k}"),
                        auction: auction.clone(),
                        deal: view.deal.as_ref().and_then(|d| d.complete()),
                    });
                }
            }
            previous = view;
        }
    }
    games
}

/// The corpus split (docs/design/15-phase4-plan.md D20): an odd enumeration index is the eval
/// split, an even one the tune split.
fn is_eval(index: usize) -> bool {
    index % 2 == 1
}

// ------------------------------------------------------------------------------------------------
// Generated fixture
// ------------------------------------------------------------------------------------------------

/// One generated auction and the deal that produced it.
#[derive(Clone, PartialEq, Debug)]
struct FixtureCase {
    id: String,
    deal: Deal,
    auction: Auction,
}

/// The fixture file: a header, then one tab-separated line per case:
/// `id  dealer  vulnerability  deal (PBN, N first)  calls (space-separated)`.
fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(GENERATED_FIXTURE)
}

fn format_fixture(cases: &[FixtureCase]) -> String {
    let mut out = String::new();
    out.push_str(
        "# SAYC-generated auctions for the reproduction harness (tests/reproduction.rs, part (i)).\n\
         # Random deals from seed 0x5A1C4001 (deal i: auction_seed(GEN_SEED, i)); board i + 1 gives\n\
         # the dealer and vulnerability; replayed with PolicyParams::system_players(), the SAYC\n\
         # natural fallback and ImplicitPass::Complement. Passed-out auctions and final contracts\n\
         # above the 5 level are skipped. Regenerate with SAYC_REPRO_WRITE_FIXTURE=1 (see\n\
         # write_generated_fixture); PROVISIONAL until re-frozen at phase-4 integration.\n\
         # id\tdealer\tvul\tdeal\tcalls\n",
    );
    for case in cases {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            case.id,
            case.auction.dealer(),
            case.auction.vulnerability(),
            case.deal,
            case.auction
        ));
    }
    out
}

fn parse_fixture(text: &str) -> Vec<FixtureCase> {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.len(), 5, "fixture line {line:?}: expected 5 fields");
            let dealer: Seat = fields[1].parse().expect("fixture dealer");
            let vul: Vulnerability = fields[2].parse().expect("fixture vulnerability");
            let deal: Deal = fields[3].parse().expect("fixture deal");
            let calls: Vec<Call> = fields[4]
                .split_whitespace()
                .map(|c| c.parse().expect("fixture call"))
                .collect();
            let auction = Auction::from_calls(dealer, vul, calls).expect("fixture auction");
            FixtureCase {
                id: fields[0].to_string(),
                deal,
                auction,
            }
        })
        .collect()
}

fn load_fixture() -> Vec<FixtureCase> {
    let path = fixture_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    parse_fixture(&text)
}

/// Replays fixed-seed random deals with the system-players policy until `count` auctions pass the
/// filter (not passed out, final level <= [`MAX_GENERATED_LEVEL`]). Also returns how many deals
/// were skipped as `[passed out, above the level]`.
fn generate_fixture(table: &Table, count: usize) -> (Vec<FixtureCase>, [usize; 2]) {
    let ctx = bid_ctx(table, PolicyParams::system_players());
    let mut out = Vec::new();
    let mut skipped = [0usize; 2];
    let mut i = 0usize;
    while out.len() < count {
        assert!(i < 100_000, "could not generate {count} auctions");
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(auction_seed(GEN_SEED, i));
        let deal = common::random_deal(&mut rng);
        let board = (i % 16) as u16 + 1;
        let dealer = Seat::ALL[i % 4];
        let vul = Vulnerability::from_board_number(board);
        let replayed = replay(table, &deal, dealer, vul, &ctx);
        match replayed.auction.contract() {
            None => skipped[0] += 1,
            Some(c) if c.bid.level() > MAX_GENERATED_LEVEL => skipped[1] += 1,
            Some(_) => out.push(FixtureCase {
                id: format!("gen-{i}"),
                deal,
                auction: replayed.auction,
            }),
        }
        i += 1;
    }
    (out, skipped)
}

// ------------------------------------------------------------------------------------------------
// Records and summaries
// ------------------------------------------------------------------------------------------------

/// One auction's record in a part of the report.
struct Record {
    id: String,
    auction: String,
    /// The last call resolved `Exact`'s node (`"node:<id>"`), or `"off_system"`.
    node_key: String,
    /// The last call's `ResolutionKind`.
    kind_key: String,
    outcome: Outcome,
}

/// The last call in `interp.per_call` resolved `Exact`, if any, and its node.
fn last_exact_node(interp: &Interpretation) -> Option<NodeId> {
    interp
        .per_call
        .iter()
        .rev()
        .find(|pc| pc.kind == ResolutionKind::Exact)
        .and_then(|pc| pc.alternatives.first())
        .and_then(|(_, _, ex)| ex.node)
}

fn record(
    id: String,
    table: &Table,
    auction: &Auction,
    opts: &InterpretOptions,
    outcome: Outcome,
) -> Record {
    let interp = interpret(table, auction, opts);
    Record {
        id,
        auction: auction.to_string(),
        node_key: last_exact_node(&interp)
            .map_or_else(|| "off_system".to_string(), |id| format!("node:{}", id.0)),
        kind_key: interp.per_call.last().map_or_else(
            || "empty_auction".to_string(),
            |pc| format!("{:?}", pc.kind),
        ),
        outcome,
    }
}

fn sorted(values: &[f64]) -> Vec<f64> {
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v
}

fn median(values: &[f64]) -> f64 {
    let v = sorted(values);
    let n = v.len();
    match n {
        0 => 0.0,
        _ if n % 2 == 1 => v[n / 2],
        _ => (v[n / 2 - 1] + v[n / 2]) / 2.0,
    }
}

/// The `p`-quantile (nearest rank on the sorted values); `0.0` if empty.
fn quantile(values: &[f64], p: f64) -> f64 {
    let v = sorted(values);
    if v.is_empty() {
        return 0.0;
    }
    let rank = (p * (v.len() - 1) as f64).round() as usize;
    v[rank.min(v.len() - 1)]
}

/// The headline of one part: the median rate over the auctions with ESS (kept count, for
/// rejection) >= [`MIN_ESS`], and how many there are.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Headline {
    auctions: usize,
    counted: usize,
    median_rate: f64,
    median_kept: f64,
}

fn headline(records: &[Record]) -> Headline {
    let rates: Vec<f64> = records
        .iter()
        .filter(|r| r.outcome.ess >= MIN_ESS)
        .filter_map(|r| r.outcome.rate)
        .collect();
    let kept: Vec<f64> = records.iter().map(|r| r.outcome.kept as f64).collect();
    Headline {
        auctions: records.len(),
        counted: rates.len(),
        median_rate: median(&rates),
        median_kept: median(&kept),
    }
}

/// Groups the counted records by `key` (largest group first) with each group's median rate.
fn grouped_medians(records: &[Record], key: impl Fn(&Record) -> &str) -> Vec<Value> {
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for r in records.iter().filter(|r| r.outcome.ess >= MIN_ESS) {
        if let Some(rate) = r.outcome.rate {
            groups.entry(key(r)).or_default().push(rate);
        }
    }
    let mut rows: Vec<(&str, Vec<f64>)> = groups.into_iter().collect();
    rows.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(b.0)));
    rows.into_iter()
        .map(|(key, rates)| json!({ "key": key, "count": rates.len(), "median": median(&rates) }))
        .collect()
}

/// A part's summary: headline, kept/ESS distribution, rate distribution, groupings, detail.
fn part_json(sampler: &Sampler, preset: &str, interpret_mode: &str, records: &[Record]) -> Value {
    let h = headline(records);
    let kept: Vec<f64> = records.iter().map(|r| r.outcome.kept as f64).collect();
    let ess: Vec<f64> = records.iter().map(|r| r.outcome.ess).collect();
    let counted: Vec<f64> = records
        .iter()
        .filter(|r| r.outcome.ess >= MIN_ESS)
        .filter_map(|r| r.outcome.rate)
        .collect();
    let target = match sampler {
        Sampler::StrictRejection { target, .. } => *target,
        Sampler::Weighted { n, .. } => *n,
    };
    let count = |f: &dyn Fn(&Record) -> bool| records.iter().filter(|r| f(r)).count();
    json!({
        "sampler": sampler.label(),
        "preset": preset,
        "interpretation": interpret_mode,
        "auctions": h.auctions,
        "min_ess": MIN_ESS,
        "auctions_reaching_min": h.counted,
        "median_rate": h.median_rate,
        "mean_rate": if counted.is_empty() { 0.0 } else { counted.iter().sum::<f64>() / counted.len() as f64 },
        "rate_p10": quantile(&counted, 0.1),
        "rate_p90": quantile(&counted, 0.9),
        "rate_nonzero": counted.iter().filter(|&&r| r > 0.0).count(),
        "rate_at_least_0_6": counted.iter().filter(|&&r| r >= 0.6).count(),
        "any_reproduced": count(&|r| r.outcome.any_reproduced),
        "errors": count(&|r| r.outcome.error.is_some()),
        "kept": {
            "zero": count(&|r| r.outcome.kept == 0),
            "1-29": count(&|r| r.outcome.kept > 0 && (r.outcome.kept as f64) < MIN_ESS),
            ">=30": count(&|r| r.outcome.kept as f64 >= MIN_ESS),
            "reached_target": count(&|r| r.outcome.kept >= target),
            "median": h.median_kept,
            "p10": quantile(&kept, 0.1),
            "p90": quantile(&kept, 0.9),
        },
        "ess": { "median": median(&ess), "p10": quantile(&ess, 0.1), "p90": quantile(&ess, 0.9) },
        "by_resolution_kind": grouped_medians(records, |r| r.kind_key.as_str()),
        "by_node": grouped_medians(records, |r| r.node_key.as_str()),
        "detail": records.iter().map(|r| json!({
            "id": r.id,
            "auction": r.auction,
            "node": r.node_key,
            "resolution_kind": r.kind_key,
            "kept": r.outcome.kept,
            "attempts": r.outcome.attempts,
            "ess": r.outcome.ess,
            "rate": r.outcome.rate,
            "any_reproduced": r.outcome.any_reproduced,
            "error": r.outcome.error,
        })).collect::<Vec<_>>(),
    })
}

fn headline_line(name: &str, records: &[Record]) -> String {
    let h = headline(records);
    format!(
        "{name}: {} auction(s), {} with >= {MIN_ESS} kept/ESS, median rate {:.4}, median kept {:.0}",
        h.auctions, h.counted, h.median_rate, h.median_kept
    )
}

// ------------------------------------------------------------------------------------------------
// Per-call true-deal agreement
// ------------------------------------------------------------------------------------------------

/// Whether the position after `prefix` is on-system for its acting seat: at least one legal system
/// candidate, as `choose_bid` decides it. Probed with the natural engine and the implicit pass
/// switched off, where `choose_bid` either chooses a system call or reports the legal system
/// candidates it rejected as `Rejected::Unsatisfied`.
fn is_on_system(table: &Table, hand: bridge_core::Hand, prefix: &Auction) -> bool {
    let probe = BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Never,
        policy: PolicyParams::system_players(),
    };
    match choose_bid(table, hand, prefix, &probe) {
        BidChoice::Chosen(c) => c.source == ChoiceSource::System,
        BidChoice::NoCandidate(nc) => nc.tried.iter().any(|t| t.reason == Rejected::Unsatisfied),
    }
}

/// Agreement counts at one kind of position.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Agree {
    calls: usize,
    agree: usize,
    /// `choose_bid` found no candidate for the true hand.
    gaps: usize,
}

impl Agree {
    fn json(&self) -> Value {
        json!({
            "calls": self.calls,
            "agree": self.agree,
            "rate": if self.calls == 0 { 0.0 } else { self.agree as f64 / self.calls as f64 },
            "gaps": self.gaps,
        })
    }
}

/// `[system, natural]` agreement of one auction's calls with `choose_bid` on the true deal.
fn agreement(table: &Table, ctx: &BidContext<'_>, deal: &Deal, auction: &Auction) -> [Agree; 2] {
    let mut out = [Agree::default(); 2];
    let mut prefix = Auction::new(auction.dealer(), auction.vulnerability());
    for &call in auction.calls() {
        let hand = deal.hand(prefix.next_seat());
        let slot = if is_on_system(table, hand, &prefix) {
            0
        } else {
            1
        };
        out[slot].calls += 1;
        match choose_bid(table, hand, &prefix, ctx) {
            BidChoice::Chosen(c) => {
                if c.call == call {
                    out[slot].agree += 1;
                }
            }
            BidChoice::NoCandidate(_) => out[slot].gaps += 1,
        }
        prefix
            .push(call)
            .expect("a validated auction's call is legal");
    }
    out
}

// ------------------------------------------------------------------------------------------------
// The harness
// ------------------------------------------------------------------------------------------------

fn loadavg() -> String {
    std::process::Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::fs::read_to_string("/proc/loadavg").ok())
        .unwrap_or_default()
}

fn part_enabled(name: &str) -> bool {
    match std::env::var("SAYC_REPRO_PARTS") {
        Ok(parts) => parts.split(',').any(|p| p.trim() == name),
        Err(_) => true,
    }
}

fn workspace_target() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/bridge-bidding is two levels under the workspace root")
        .join("target")
}

/// The reproduction harness (11-testing.md §3): parts (i)-(iv) of the module doc, written to
/// `target/reproduction_report.json`. `#[ignore]`d: it samples up to `SAYC_REPRO_TARGET` deals per
/// auction. Parts (ii)-(iv) need the corpus (`BRIDGE_CORPUS_DIR` or `corpus/data`) and are skipped
/// without it. Sizing: `SAYC_REPRO_PARTS` (comma list of generated, corpus, legacy, agreement),
/// `SAYC_REPRO_TARGET`, `SAYC_REPRO_MAX_DRAWS`, `SAYC_REPRO_GENERATED` (fixture auctions used),
/// `SAYC_REPRO_CORPUS_LIMIT` (subset auctions sampled), `SAYC_REPRO_LIMIT` (legacy auctions),
/// `SAYC_REPRO_SAMPLER`. No threshold is asserted: the numbers are reported (phase 4 target:
/// median >= 0.6 on (i) and on (ii)).
#[test]
#[ignore = "samples up to 1000 kept deals per auction; run with `cargo test --release -- --ignored`"]
fn sayc_reproduction_rate() {
    let table = common::compile_sayc("sayc.bml");
    let system_ctx = bid_ctx(&table, PolicyParams::system_players());
    let human_ctx = bid_ctx(&table, PolicyParams::human());
    let sampler = headline_sampler();
    let mut report = serde_json::Map::new();
    report.insert("loadavg_start".into(), json!(loadavg()));
    report.insert("headline_sampler".into(), json!(sampler.label()));
    let mut lines = Vec::new();
    let started = std::time::Instant::now();

    // (i) the generated fixture, system-players preset.
    if part_enabled("generated") {
        let t = std::time::Instant::now();
        let mut cases = load_fixture();
        cases.truncate(env_usize("SAYC_REPRO_GENERATED", cases.len()));
        let opts = InterpretOptions::for_context(&system_ctx);
        let drift = par_map(&cases, |_, c| {
            replay(
                &table,
                &c.deal,
                c.auction.dealer(),
                c.auction.vulnerability(),
                &system_ctx,
            )
            .auction
                != c.auction
        })
        .into_iter()
        .filter(|&d| d)
        .count();
        let records = par_map(&cases, |i, c| {
            let outcome = evaluate(
                &sampler,
                &table,
                &system_ctx,
                &c.auction,
                &opts,
                auction_seed(0x5A1C_4101, i),
            );
            record(c.id.clone(), &table, &c.auction, &opts, outcome)
        });
        let mut part = part_json(&sampler, "system_players", "mirror (for_context)", &records);
        part["fixture"] = json!(GENERATED_FIXTURE);
        part["fixture_drift"] = json!(drift);
        part["seconds"] = json!(t.elapsed().as_secs_f64());
        lines.push(format!(
            "{} ({drift} fixture auction(s) no longer replay from their deal)",
            headline_line("(i) generated", &records)
        ));
        report.insert("generated".into(), part);
    }

    let corpus = common::corpus_dir().map(|dir| (dir.clone(), corpus_games(&dir)));
    match &corpus {
        None => lines.push("corpus directory not found: parts (ii)-(iv) skipped".into()),
        Some((dir, games)) => {
            report.insert("corpus_dir".into(), json!(dir.display().to_string()));
            report.insert("corpus_games".into(), json!(games.len()));
            let eval: Vec<&CorpusGame> = games.iter().filter(|g| is_eval(g.index)).collect();
            let eval_with_deal: Vec<(&CorpusGame, &Deal)> = eval
                .iter()
                .filter_map(|g| g.deal.as_ref().map(|d| (*g, d)))
                .collect();

            // (ii) the corpus SAYC-reproducible subset of the eval split, human preset.
            if part_enabled("corpus") {
                let t = std::time::Instant::now();
                let reproducible: Vec<&CorpusGame> = par_map(&eval_with_deal, |_, (g, deal)| {
                    replay(
                        &table,
                        deal,
                        g.auction.dealer(),
                        g.auction.vulnerability(),
                        &human_ctx,
                    )
                    .auction
                        == g.auction
                })
                .into_iter()
                .zip(&eval_with_deal)
                .filter(|(r, _)| *r)
                .map(|(_, (g, _))| *g)
                .collect();
                let limit = env_usize("SAYC_REPRO_CORPUS_LIMIT", reproducible.len());
                let sampled = &reproducible[..limit.min(reproducible.len())];
                // The headline reads the corpus with the human preset (D18). Its strict support
                // also holds the natural deviation pieces Y_c (weight delta) at on-system
                // positions, which an unweighted sampler overweights, so the same subset is also
                // reported under the system-players preset.
                let run = |ctx: &BidContext<'_>| {
                    let opts = InterpretOptions::for_context(ctx);
                    par_map(sampled, |i, g| {
                        let outcome = evaluate(
                            &sampler,
                            &table,
                            ctx,
                            &g.auction,
                            &opts,
                            auction_seed(0x5A1C_4201, i),
                        );
                        record(g.label.clone(), &table, &g.auction, &opts, outcome)
                    })
                };
                let records = run(&human_ctx);
                let records_system = run(&system_ctx);
                let mut part = part_json(&sampler, "human", "mirror (for_context)", &records);
                part["system_players"] = part_json(
                    &sampler,
                    "system_players",
                    "mirror (for_context)",
                    &records_system,
                );
                part["eval_games"] = json!(eval.len());
                part["eval_games_with_deal"] = json!(eval_with_deal.len());
                part["reproducible"] = json!(reproducible.len());
                part["reproducible_share"] =
                    json!(reproducible.len() as f64 / eval_with_deal.len().max(1) as f64);
                part["seconds"] = json!(t.elapsed().as_secs_f64());
                lines.push(format!(
                    "{} (subset {} of {} eval games with a deal)",
                    headline_line("(ii) corpus SAYC-reproducible subset", &records),
                    reproducible.len(),
                    eval_with_deal.len()
                ));
                lines.push(headline_line(
                    "(ii') the same subset under system_players",
                    &records_system,
                ));
                report.insert("corpus_subset".into(), part);
            }

            // (iii) the legacy definition: first 500 corpus auctions, both splits.
            if part_enabled("legacy") {
                let t = std::time::Instant::now();
                let limit = env_usize("SAYC_REPRO_LIMIT", LEGACY_LIMIT);
                let legacy: Vec<&CorpusGame> = games.iter().take(limit).collect();
                let opts = InterpretOptions::legacy();
                let weighted = Sampler::Weighted {
                    proposal: ProposalKind::Uniform,
                    n: WEIGHTED_N,
                };
                let pairs = par_map(&legacy, |i, g| {
                    let seed = auction_seed(0x5A1C_3001, i);
                    let raw = evaluate(
                        &LEGACY_SAMPLER,
                        &table,
                        &system_ctx,
                        &g.auction,
                        &opts,
                        seed,
                    );
                    let w = evaluate(
                        &weighted,
                        &table,
                        &system_ctx,
                        &g.auction,
                        &opts,
                        seed ^ 0xA5A5_5A5A_0F0F_F0F0,
                    );
                    (
                        record(g.label.clone(), &table, &g.auction, &opts, raw),
                        record(g.label.clone(), &table, &g.auction, &opts, w),
                    )
                });
                let (raw, w): (Vec<Record>, Vec<Record>) = pairs.into_iter().unzip();
                let weighted_rates: Vec<f64> = w.iter().filter_map(|r| r.outcome.rate).collect();
                let any = w.iter().filter(|r| r.outcome.any_reproduced).count();
                let mut part = part_json(&LEGACY_SAMPLER, "system_players", "legacy()", &raw);
                part["weighted_uniform"] = json!({
                    "sampler": weighted.label(),
                    "rate_median": median(&weighted_rates),
                    "any_reproduced": any,
                    "ess_median": median(&w.iter().map(|r| r.outcome.ess).collect::<Vec<_>>()),
                });
                part["seconds"] = json!(t.elapsed().as_secs_f64());
                lines.push(format!(
                    "{}; likelihood-weighted uniform: any reproduced {any}/{}",
                    headline_line(
                        "(iii) legacy (first corpus auctions, legacy interpret)",
                        &raw
                    ),
                    w.len()
                ));
                report.insert("legacy".into(), part);
            }

            // (iv) per-call true-deal agreement on the eval split.
            if part_enabled("agreement") {
                let t = std::time::Instant::now();
                let per_game = par_map(&eval_with_deal, |_, (g, deal)| {
                    agreement(&table, &human_ctx, deal, &g.auction)
                });
                let mut total = [Agree::default(); 2];
                for [s, n] in &per_game {
                    for (acc, x) in total.iter_mut().zip([s, n]) {
                        acc.calls += x.calls;
                        acc.agree += x.agree;
                        acc.gaps += x.gaps;
                    }
                }
                let all = Agree {
                    calls: total[0].calls + total[1].calls,
                    agree: total[0].agree + total[1].agree,
                    gaps: total[0].gaps + total[1].gaps,
                };
                lines.push(format!(
                    "(iv) true-deal agreement (eval split, {} games): system {}/{} = {:.3}, \
                     natural {}/{} = {:.3}, all {:.3}",
                    per_game.len(),
                    total[0].agree,
                    total[0].calls,
                    total[0].agree as f64 / total[0].calls.max(1) as f64,
                    total[1].agree,
                    total[1].calls,
                    total[1].agree as f64 / total[1].calls.max(1) as f64,
                    all.agree as f64 / all.calls.max(1) as f64,
                ));
                report.insert(
                    "agreement".into(),
                    json!({
                        "split": "eval (odd enumeration index)",
                        "games": per_game.len(),
                        "system_positions": total[0].json(),
                        "natural_positions": total[1].json(),
                        "all": all.json(),
                        "seconds": t.elapsed().as_secs_f64(),
                    }),
                );
            }
        }
    }

    report.insert("seconds".into(), json!(started.elapsed().as_secs_f64()));
    report.insert("loadavg_end".into(), json!(loadavg()));
    let target = workspace_target();
    std::fs::create_dir_all(&target).expect("create target/");
    std::fs::write(
        target.join("reproduction_report.json"),
        serde_json::to_string_pretty(&Value::Object(report)).expect("report serializes"),
    )
    .expect("write target/reproduction_report.json");
    for line in &lines {
        eprintln!("sayc_reproduction_rate: {line}");
    }
    eprintln!(
        "sayc_reproduction_rate: {:.1} s, loadavg {}",
        started.elapsed().as_secs_f64(),
        loadavg()
    );
}

/// Regenerates the generated fixture when `SAYC_REPRO_WRITE_FIXTURE=1`; otherwise compares the
/// file with what the current system generates and reports how many cases differ (the fixture is
/// frozen on purpose, so a difference is information, not a failure).
#[test]
#[ignore = "regenerates tests/data/repro_generated.txt with SAYC_REPRO_WRITE_FIXTURE=1"]
fn write_generated_fixture() {
    let table = common::compile_sayc("sayc.bml");
    let (cases, [passed_out, too_high]) = generate_fixture(&table, GENERATED_COUNT);
    eprintln!(
        "generator: {} deals replayed, {passed_out} passed out and {too_high} above the {} level \
         skipped",
        cases.len() + passed_out + too_high,
        MAX_GENERATED_LEVEL
    );
    if std::env::var("SAYC_REPRO_WRITE_FIXTURE").as_deref() == Ok("1") {
        let path = fixture_path();
        std::fs::create_dir_all(path.parent().expect("fixture has a parent"))
            .expect("create tests/data");
        std::fs::write(&path, format_fixture(&cases)).expect("write fixture");
        eprintln!("wrote {} cases to {}", cases.len(), path.display());
        return;
    }
    let frozen = load_fixture();
    let differ = frozen.iter().zip(&cases).filter(|(a, b)| a != b).count()
        + frozen.len().abs_diff(cases.len());
    eprintln!(
        "generated fixture: {differ} of {} case(s) differ from what the current system generates",
        frozen.len()
    );
}

// ------------------------------------------------------------------------------------------------
// Default-suite checks
// ------------------------------------------------------------------------------------------------

/// The fixture parses into 100 distinct, complete, not-passed-out auctions at most at the 5 level,
/// and writing it back gives the same cases.
#[test]
fn generated_fixture_is_well_formed() {
    let cases = load_fixture();
    assert_eq!(cases.len(), GENERATED_COUNT);
    let ids: std::collections::HashSet<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids.len(), cases.len(), "fixture ids are unique");
    for c in &cases {
        assert!(c.auction.is_complete(), "{}: incomplete", c.id);
        assert!(!c.auction.is_passed_out(), "{}: passed out", c.id);
        let level = c.auction.contract().expect("a contract").bid.level();
        assert!(level <= MAX_GENERATED_LEVEL, "{}: level {level}", c.id);
    }
    assert_eq!(parse_fixture(&format_fixture(&cases)), cases);
}

/// The fixture generator is deterministic and keeps only auctions that pass its filter.
#[test]
fn fixture_generator_is_deterministic() {
    let table = common::compile_sayc("sayc.bml");
    let (a, _) = generate_fixture(&table, 3);
    let (b, _) = generate_fixture(&table, 3);
    assert_eq!(a, b);
    assert!(a.iter().all(|c| !c.auction.is_passed_out()));
}

/// The corpus split rule: odd enumeration index = eval.
#[test]
fn corpus_split_is_odd_index() {
    assert!(!is_eval(0));
    assert!(is_eval(1));
    assert_eq!((0..10).filter(|&i| is_eval(i)).count(), 5);
}

fn passed_out() -> Auction {
    Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        vec![Call::Pass, Call::Pass, Call::Pass, Call::Pass],
    )
    .expect("passed-out auction is legal")
}

/// The headline is not tied to the rate: an auction many deals reproduce (SAYC passed out, where
/// every kept deal has four hands that open nothing) counts in it with a high rate.
#[test]
fn headline_counts_an_auction_that_many_deals_reproduce() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = bid_ctx(&table, PolicyParams::system_players());
    let auction = passed_out();
    let sampler = Sampler::StrictRejection {
        target: 60,
        max_draws: 20_000,
    };
    let outcome = evaluate(
        &sampler,
        &table,
        &ctx,
        &auction,
        &InterpretOptions::for_context(&ctx),
        11,
    );
    assert!(
        outcome.kept as f64 >= MIN_ESS,
        "only {} deals kept",
        outcome.kept
    );
    assert_eq!(outcome.ess, outcome.kept as f64);
    let rate = outcome.rate.expect("some deal kept");
    assert!(rate > 0.5, "passed-out rate {rate}");

    let rec = |rate: Option<f64>, kept: usize| Record {
        id: String::new(),
        auction: String::new(),
        node_key: String::new(),
        kind_key: String::new(),
        outcome: Outcome {
            kept,
            attempts: 0,
            ess: kept as f64,
            rate,
            any_reproduced: false,
            error: None,
        },
    };
    let h = headline(&[rec(Some(rate), outcome.kept), rec(Some(0.0), 5)]);
    assert_eq!(h.counted, 1);
    assert_eq!(h.median_rate, rate);
}

/// The weighted path (the phase-5 headline with `ConstraintProposal`) runs end to end with the
/// uniform proposal: a passed-out auction is reproduced by most of the weight.
#[test]
fn weighted_sampler_reproduces_a_passed_out_auction() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = bid_ctx(&table, PolicyParams::system_players());
    let auction = passed_out();
    let sampler = Sampler::Weighted {
        proposal: ProposalKind::Uniform,
        n: 200,
    };
    let outcome = evaluate(
        &sampler,
        &table,
        &ctx,
        &auction,
        &InterpretOptions::for_context(&ctx),
        12,
    );
    assert_eq!(outcome.kept, 200);
    assert!(outcome.error.is_none());
    assert!(outcome.any_reproduced);
    let rate = outcome.rate.expect("deals drawn");
    assert!(rate > 0.9, "weighted passed-out rate {rate}");
}

/// The opening position is on-system, and agreement on a generated case is complete (its calls
/// are `choose_bid`'s own on its deal, as long as the fixture has not drifted).
#[test]
fn agreement_counts_system_positions() {
    let table = common::compile_sayc("sayc.bml");
    let ctx = bid_ctx(&table, PolicyParams::system_players());
    let empty = Auction::new(Seat::North, Vulnerability::None);
    let hand: bridge_core::Hand = "AK32.KQ2.QJ3.K32".parse().expect("hand");
    assert!(is_on_system(&table, hand, &empty));

    let case = &generate_fixture(&table, 1).0[0];
    let [system, natural] = agreement(&table, &ctx, &case.deal, &case.auction);
    assert_eq!(system.calls + natural.calls, case.auction.len());
    assert_eq!(system.agree + natural.agree, case.auction.len());
    assert!(system.calls >= 1);
}
