//! `cargo xtask coverage`: the phase-4 coverage report (docs/design/15-phase4-plan.md, lane D
//! step 1; docs/design/12-roadmap.md task 4.1).
//!
//! Writes `target/coverage_report.json` with four parts, all computed against the compiled
//! `systems/sayc/sayc.bml` through the public `bridge-system`/`bridge-bidding` API:
//!
//! - **generated**: `COVERAGE_REPLAYS` (default 1000) fixed-seed random deals replayed with
//!   natural completion (`choose_bid` with `ctx.natural` set and `ImplicitPass::Complement`).
//!   Each call is classified as a system call (the position is on-system: the exact resolve, or
//!   the first full lenient match, has a legal child; the system's implicit pass included), a
//!   natural completion (off-system position, answered by the natural engine), or a gap
//!   (`NoCandidate`, forced `Pass`). An auction is *all-system* when it has neither natural
//!   completions nor gaps. Reports the all-system rate, the gap (`NoCandidate`) top 50, the
//!   natural-completion tops (every natural call, and the first departure from the system per
//!   auction), the final-contract level histogram `[passout, 1..=7]`, and how many system calls
//!   were default passes (a `Pass` at priority <= -100: a `{stop}` row or the synthesised stop
//!   pass of a system stop, `systems/sayc/passes.bml`; `system_stop_passes` counts the latter).
//!   *Strict* accounting (`all_system_strict`, the phase-4 `[G]` criterion): a position whose
//!   exclusive group holds nothing but default passes (the system passes with any hand there)
//!   also counts as a departure when the natural choice `m_P(h)` is not `Pass`
//!   (`default_pass_overrides`, their tops, and `first_strict_departure_*`). Without it, the
//!   system stops make every such position look like a system decision.
//! - **positions**: `COVERAGE_POSITIONS` (default 200,000) positions drawn exactly like the
//!   forward-consistency harness (`crates/bridge-bidding/tests/common` +
//!   `tests/consistency.rs`: seed `0x5a1c0002`, 5% random-call substitution, random depth
//!   0..12, `NoCandidate` becoming `Pass` in the prefix), so the `NoCandidate` counts per 10^6
//!   positions are comparable with the phase-3 numbers of 12-roadmap.
//! - **corpus**: every PBN auction (recursive, sorted file order) and then every LIN board of
//!   `corpus/data`, enumerated in that order; even indices are the tune split, odd ones the eval
//!   split (D20). Reports the auction all-Exact rate and call-level kinds for all, eval, tune and
//!   the SAYC-compatible-opening subset (the true opener's hand lies in the exclusive region X of
//!   the recorded opening, from `SystemIR::exclusive`), seats with empty strict support, seats
//!   whose default-mode support is empty (what the sampler would report as `EmptySupport`), the
//!   `resolve_lenient` usage rate, the true-deal policy agreement (system / natural positions),
//!   and the maximum-likelihood `(ε, δ)` of `p(c|h) = (1−ε)[(1−δ)S + δM] + ε/n` on the tune
//!   split with its log-likelihood curves. For the subset it also lists where its calls leave
//!   the system (every natural call and the first one per auction, by trie position, with the
//!   reason: `call_not_a_row` when the position is on the system but the recorded call is not
//!   one of its rows, `call_not_a_row_default_pass_only` when the position's only rows are
//!   default passes, otherwise why the position itself is off the system).
//! - **lints / exclusive**: lint counts by severity and code, our own non-pass calls with no
//!   requirement at all (`unconstrained_own_calls`: a table header naming a call no row
//!   defines), and the members/branches the
//!   exclusive index shows as never chosen (shadowed), a fresh index build time (best of 3) and
//!   the postcard size of the compiled IR.
//!
//! Sizing: `COVERAGE_REPLAYS`, `COVERAGE_POSITIONS`, `COVERAGE_CORPUS_LIMIT` (default: all),
//! `COVERAGE_SEED` (replay seed, default `0xC0FE_4001`). `BRIDGE_CORPUS_DIR` and
//! `BRIDGE_SYSTEMS_DIR` override the data locations. `COVERAGE_OUT` overrides the output path.
//! `COVERAGE_TOP_N` (default 0) additionally lists that many first departures
//! (`generated.first_departure_top_n`), and `COVERAGE_PRINT_LINTS=<code substring>` prints the
//! matching lints to stderr; both are authoring aids. `COVERAGE_PRINT_LENIENT` prints each
//! corpus call resolved through `resolve_lenient` and each seat with empty default-mode support.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use bridge_bidding::{
    BidChoice, BidContext, ImplicitPass, InterpretOptions, PolicyParams, ResolutionKind, Scoring,
    Table, call_distribution, choose_bid, interpret,
};
use bridge_core::{Auction, Call, Card, Deal, Hand, Seat, Vulnerability};
use bridge_system::exclusive::branches_of;
use bridge_system::natural::classify;
use bridge_system::{LookupKey, RelVul, Severity, SystemIR};
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::{Rng, SeedableRng};
use serde_json::{Value, json};

use crate::Result;

/// Upper bound on opponents'-call substitutions `choose_bid` passes to `resolve_lenient`
/// (`bridge_bidding`'s private `LENIENT_MAX_SUBST`; mirrored here to classify positions the way
/// `choose_bid` does).
const LENIENT_MAX_SUBST: u8 = 2;

/// Base seed of the forward-consistency 10^6 harness (`tests/consistency.rs`).
const POSITIONS_SEED: u64 = 0x5A1C_0002;
/// The phase-3 `NoCandidate` tops named in 12-roadmap (trie path without leading passes, role;
/// the roadmap's "P-P-1D-(1H)" is the sample path of the `1D-(1H)` trie position), tracked by
/// name in the positions report.
const PHASE3_TOPS: &[(&str, &str)] = &[
    ("1D-(3C)", "responder"),
    ("1D-(1H)", "responder"),
    ("1C-(1H)", "responder"),
];

/// Priority at or below which a system `Pass` is a default "the partnership passes from here on"
/// pass: a `P = {prio:-100} {stop} any hand` row, or the stop pass the compiler synthesises after
/// a system stop (`{prio:-100}`, `Node::is_synthesised`; `systems/sayc/passes.bml`,
/// `docs/design/06-system.md` §4.5).
const DEFAULT_PASS_PRIORITY: i16 = -100;

/// The harness's off-system substitution rate.
const RANDOM_CALL_RATE: f64 = 0.05;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.replace('_', "").parse().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| {
            let v = v.replace('_', "");
            match v.strip_prefix("0x") {
                Some(hex) => u64::from_str_radix(hex, 16).ok(),
                None => v.parse().ok(),
            }
        })
        .unwrap_or(default)
}

fn loadavg() -> String {
    std::process::Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Runs the report and writes it. `args` is ignored beyond `--help`.
pub fn run(args: &[&str]) -> Result<std::process::ExitCode> {
    if args.contains(&"--help") {
        eprintln!(
            "cargo xtask coverage: writes target/coverage_report.json (see xtask/src/coverage.rs)"
        );
        return Ok(std::process::ExitCode::SUCCESS);
    }
    let started = Instant::now();
    let load_start = loadavg();
    let root = crate::workspace_root();
    let systems = match std::env::var_os("BRIDGE_SYSTEMS_DIR") {
        Some(d) => PathBuf::from(d),
        None => root.join("systems"),
    };
    let path = systems.join("sayc").join("sayc.bml");
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let t0 = Instant::now();
    let (ir, lints) = bridge_system::compile(
        &path.to_string_lossy(),
        &text,
        &bridge_system::lexer::FsLoader,
        &bridge_system::CompileOptions::default(),
    );
    let compile_ms = t0.elapsed().as_secs_f64() * 1e3;
    let ir = Arc::new(ir);
    let table = Table::uniform(
        ir.clone(),
        Arc::new(bridge_system::NaturalInference::default()),
    );
    let ctx = BidContext {
        scoring: Scoring::Imp,
        natural: Some(table.natural.as_ref()),
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::system_players(),
    };

    let lints_json = lint_report(&ir, &lints);
    if let Ok(show) = std::env::var("COVERAGE_SHOW") {
        for calls in show.split(';') {
            show_group(&ir, calls);
        }
    }
    eprintln!(
        "coverage: compiled sayc.bml in {compile_ms:.0} ms ({} rows, {} nodes); lints {lints_json}",
        ir.rows.len(),
        ir.nodes.len()
    );
    let exclusive_json = exclusive_report(&ir);

    let t = Instant::now();
    let generated = generated_report(&table, &ctx);
    let generated_s = t.elapsed().as_secs_f64();
    eprintln!(
        "coverage: generated in {generated_s:.1} s: all_system_rate {}",
        generated["all_system_rate"]
    );

    let t = Instant::now();
    let positions = positions_report(&table, &ctx);
    let positions_s = t.elapsed().as_secs_f64();
    eprintln!(
        "coverage: positions in {positions_s:.1} s: no_candidate {}",
        positions["no_candidate"]
    );

    let t = Instant::now();
    let corpus = match corpus_dir(&root) {
        Some(dir) => corpus_report(&table, &ctx, &dir),
        None => json!(null),
    };
    let corpus_s = t.elapsed().as_secs_f64();
    eprintln!("coverage: corpus in {corpus_s:.1} s");

    // `ir.meta.source_hash` is not filled in by the compiler (all zeros), so fingerprint the
    // sources here: FNV-1a 64 over every `.bml` file of the system directory, sorted by name.
    let system_hash = sources_fingerprint(&systems.join("sayc"));
    let report = json!({
        "system": "sayc.bml",
        "system_hash": format!("fnv1a64:{system_hash:016x}"),
        "compile_ms": compile_ms,
        "elapsed_s": started.elapsed().as_secs_f64(),
        "section_s": { "generated": generated_s, "positions": positions_s, "corpus": corpus_s },
        "loadavg_start": load_start,
        "loadavg_end": loadavg(),
        "release": !cfg!(debug_assertions),
        "lints": lints_json,
        "exclusive": exclusive_json,
        "generated": generated,
        "positions": positions,
        "corpus": corpus,
    });
    let out = match std::env::var_os("COVERAGE_OUT") {
        Some(p) => PathBuf::from(p),
        None => root.join("target").join("coverage_report.json"),
    };
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, serde_json::to_string_pretty(&report)?)?;
    eprintln!(
        "coverage: wrote {} in {:.1} s (loadavg {})",
        out.display(),
        started.elapsed().as_secs_f64(),
        loadavg()
    );
    Ok(std::process::ExitCode::SUCCESS)
}

/// FNV-1a 64 over the names and contents of the `.bml` files in `dir` (sorted by name): a stable
/// fingerprint of the SAYC sources a report was measured on.
fn sources_fingerprint(dir: &Path) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for file in files_with_ext(dir, "bml") {
        if let Some(name) = file.file_name() {
            feed(name.to_string_lossy().as_bytes());
        }
        if let Ok(bytes) = std::fs::read(&file) {
            feed(&bytes);
        }
    }
    hash
}

// ------------------------------------------------------------------------------------------
// Lints and the exclusive index.
// ------------------------------------------------------------------------------------------

fn lint_report(ir: &SystemIR, lints: &[bridge_system::Lint]) -> Value {
    let mut by_code: BTreeMap<String, u64> = BTreeMap::new();
    let (mut error, mut warning, mut info) = (0u64, 0u64, 0u64);
    // Authoring aid: `COVERAGE_PRINT_LINTS=<code substring>` prints the matching lints.
    let print = std::env::var("COVERAGE_PRINT_LINTS").ok();
    for l in lints {
        if let Some(filter) = &print {
            let code = format!("{:?}/{:?}", l.severity, l.code);
            if code.contains(filter.as_str()) {
                let path = l.node.map(|n| {
                    let node = ir.node(n);
                    format!("{:?} {:?}", node.side, node.calls)
                });
                eprintln!("lint {code} {:?} {path:?}: {}", l.span, l.message);
            }
        }
        match l.severity {
            Severity::Error => error += 1,
            Severity::Warning => warning += 1,
            _ => info += 1,
        }
        *by_code
            .entry(format!("{:?}/{:?}", l.severity, l.code))
            .or_default() += 1;
    }
    let shadowed: u64 = by_code
        .iter()
        .filter(|(k, _)| k.contains("ShadowedBranch"))
        .map(|(_, v)| *v)
        .sum();
    // Split by the side whose call the shadowed node is: our own calls are what `choose_bid`
    // picks; the opponents' calls are trie edges only (their nodes carry no requirement, so
    // every opponents' call ranked below another one at the same position reads as shadowed).
    let shadowed_us = lints
        .iter()
        .filter(|l| format!("{:?}", l.code).contains("ShadowedBranch"))
        .filter(|l| {
            l.node
                .is_some_and(|n| ir.node(n).side == bridge_system::Side::Us)
        })
        .count();
    // Our own calls (other than a pass) with no requirement at all: a table header such as
    // `(1X)-1Y-(D)-2X-(P)-` names a call of ours that no row defines, so the call is a trie edge
    // with an empty description and `choose_bid` makes it with any hand.
    let mut unconstrained: BTreeMap<String, u64> = BTreeMap::new();
    for node in &ir.nodes {
        if node.side == bridge_system::Side::Us
            && node.call != Call::Pass
            && node.description.trim().is_empty()
        {
            let span = &ir.row(node.row).span;
            *unconstrained
                .entry(format!("file {:?} line {}", span.file, span.line))
                .or_default() += 1;
        }
    }
    let unconstrained_nodes: u64 = unconstrained.values().sum();
    json!({
        "error": error,
        "warning": warning,
        "info": info,
        "unconstrained_own_calls": unconstrained_nodes,
        "unconstrained_own_call_rows": unconstrained,
        "shadowed_branch": shadowed,
        "shadowed_branch_us": shadowed_us,
        "shadowed_branch_them": shadowed - shadowed_us as u64,
        "by_code": by_code,
    })
}

/// Members and branches that the exclusive index shows as never chosen by `choose_bid`: a member
/// whose call has no piece at all (`ExclusiveGroup::is_shadowed`), and a branch of a member's
/// top-level `Or` for which no piece of that node survives. Counted over distinct nodes /
/// `(node, branch)` pairs, "in some group" (some seat/vulnerability class) and "in every group
/// the node appears in".
fn exclusive_report(ir: &SystemIR) -> Value {
    let t = Instant::now();
    let index = ir.exclusive();
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    // `compile()` builds the index eagerly, so `build_ms` is normally ~0; time a fresh build
    // (best of 3) to see what the index costs on this system.
    let fresh_build_ms = (0..3)
        .map(|_| {
            let t = Instant::now();
            let fresh = bridge_system::ExclusiveIndex::build(ir);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            std::hint::black_box(fresh.group_count());
            ms
        })
        .fold(f64::INFINITY, f64::min);
    let ir_postcard_bytes = postcard::to_allocvec(ir).map(|b| b.len()).unwrap_or(0);
    // (node, branch) -> (groups where shadowed, groups where present)
    let mut branch_stats: HashMap<(u32, u16), (u32, u32)> = HashMap::new();
    let mut call_shadowed: HashMap<u32, (u32, u32)> = HashMap::new();
    for group in index.groups() {
        for &(call, node) in &group.members {
            let pieces = group.pieces(call).unwrap_or(&[]);
            let e = call_shadowed.entry(node.0).or_default();
            e.1 += 1;
            if pieces.is_empty() {
                e.0 += 1;
            }
            let n_branches = branches_of(&ir.node(node).constraint).len().max(1) as u16;
            for b in 0..n_branches {
                let present = pieces.iter().any(|p| p.node == node && p.branch == b);
                let e = branch_stats.entry((node.0, b)).or_default();
                e.1 += 1;
                if !present {
                    e.0 += 1;
                }
            }
        }
    }
    fn any<K>(m: &HashMap<K, (u32, u32)>) -> usize {
        m.values().filter(|(s, _)| *s > 0).count()
    }
    fn all<K>(m: &HashMap<K, (u32, u32)>) -> usize {
        m.values().filter(|(s, n)| *s == *n).count()
    }
    json!({
        "build_ms": build_ms,
        "fresh_build_ms_best_of_3": fresh_build_ms,
        "ir_postcard_bytes": ir_postcard_bytes,
        "groups": index.group_count(),
        "keys": index.key_count(),
        "nodes": call_shadowed.len(),
        "shadowed_calls_in_some_group": any(&call_shadowed),
        "shadowed_calls_in_every_group": all(&call_shadowed),
        "branches": branch_stats.len(),
        "shadowed_branches_in_some_group": any(&branch_stats),
        "shadowed_branches_in_every_group": all(&branch_stats),
    })
}

// ------------------------------------------------------------------------------------------
// Positions: on-system classification and labels.
// ------------------------------------------------------------------------------------------

/// How `choose_bid` sees the position after `auction` for its next seat.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OnSystem {
    /// The exact resolve has a legal child.
    Exact,
    /// The first full `resolve_lenient` match has a legal child.
    Lenient,
    /// No legal system candidate: the natural engine answers.
    Off,
}

fn rel_vul(auction: &Auction, seat: Seat) -> RelVul {
    let v = auction.vulnerability();
    RelVul {
        we: v.is_vulnerable(seat),
        they: v.is_vulnerable(seat.next()),
    }
}

/// Authoring aid (`COVERAGE_SHOW="1D 1H P P"`, calls from the dealer, North dealing, none
/// vulnerable): prints the sibling group the next seat chooses from, best rank first, with
/// each member's priority, source line and whether the exclusive index shadows it. Several
/// auctions may be given, separated by `;`.
fn show_group(ir: &SystemIR, calls: &str) {
    let mut auction = Auction::new(Seat::North, Vulnerability::None);
    for token in calls.split_whitespace() {
        match token.parse::<Call>() {
            Ok(call) => {
                if auction.push(call).is_err() {
                    eprintln!("show: illegal call {token}");
                    return;
                }
            }
            Err(_) => {
                eprintln!("show: cannot parse {token}");
                return;
            }
        }
    }
    let seat = auction.next_seat();
    let key = key_for(&auction, seat);
    let lookup = ir.index.resolve(&key);
    eprintln!(
        "show {auction}: matched {}/{} calls",
        lookup.matched_depth,
        key.calls.len()
    );
    if lookup.matched_depth != key.calls.len() {
        return;
    }
    let Some(group) = ir
        .exclusive()
        .group_for(lookup.end, key.opener_pos, key.vul)
    else {
        eprintln!("show: no group");
        return;
    };
    for &(call, node_id) in &group.members {
        let node = ir.node(node_id);
        let span = ir.row(node.row).span.clone();
        eprintln!(
            "  {call} prio {} shadowed {} row {:?} {:?}",
            node.priority,
            group.is_shadowed(call),
            node.row,
            span
        );
    }
}

/// The lookup key `choose_bid` builds for the next seat (the root key for an empty or
/// all-pass prefix).
fn key_for(auction: &Auction, seat: Seat) -> LookupKey<'_> {
    match LookupKey::for_auction(auction, seat) {
        Some(key) => key,
        None => LookupKey {
            we_opened: true,
            calls: &[],
            opener_pos: auction.position_of(seat),
            vul: rel_vul(auction, seat),
        },
    }
}

/// Mirrors `bridge_bidding::choose::gather`'s on-system test.
fn on_system(system: &SystemIR, auction: &Auction) -> (OnSystem, usize) {
    let seat = auction.next_seat();
    let key = key_for(auction, seat);
    let lookup = system.index.resolve(&key);
    let matched = lookup.matched_depth;
    let legal_child = |end| {
        system
            .index
            .children(end, key.opener_pos, key.vul)
            .iter()
            .any(|&(c, _)| auction.is_legal(c))
    };
    if matched == key.calls.len() {
        let state = if legal_child(lookup.end) {
            OnSystem::Exact
        } else {
            OnSystem::Off
        };
        return (state, matched);
    }
    let lenient = system
        .index
        .resolve_lenient(&key, LENIENT_MAX_SUBST)
        .into_iter()
        .find(|(lk, _)| lk.matched_depth == key.calls.len());
    match lenient {
        Some((lk, _)) if legal_child(lk.end) => (OnSystem::Lenient, matched),
        _ => (OnSystem::Off, matched),
    }
}

/// Whether the node the next seat's resolve ends at (the exact resolve, or the first full lenient
/// match, as in [`on_system`]) offers nothing but default passes: every member of its exclusive
/// group is a `Pass` at priority <= [`DEFAULT_PASS_PRIORITY`] (a `{stop}` pass row or the
/// synthesised stop pass of a system stop, with no real row next to it). There the
/// system passes with any hand, so the position says nothing the author decided about the hand.
fn default_pass_only(system: &SystemIR, auction: &Auction) -> bool {
    let seat = auction.next_seat();
    let key = key_for(auction, seat);
    let lookup = system.index.resolve(&key);
    let end = if lookup.matched_depth == key.calls.len() {
        lookup.end
    } else {
        match system
            .index
            .resolve_lenient(&key, LENIENT_MAX_SUBST)
            .into_iter()
            .find(|(lk, _)| lk.matched_depth == key.calls.len())
        {
            Some((lk, _)) => lk.end,
            None => return false,
        }
    };
    let Some(group) = system.exclusive().group_for(end, key.opener_pos, key.vul) else {
        return false;
    };
    !group.members.is_empty()
        && group.members.iter().all(|&(call, node)| {
            call == Call::Pass && system.node(node).priority <= DEFAULT_PASS_PRIORITY
        })
}

/// The natural engine's deterministic choice `m_P(h)` (`call_distribution` at `ε = 0, δ = 1`),
/// or `None` when it is uniform (`⊥`).
fn natural_choice(
    table: &Table,
    ctx: &BidContext<'_>,
    hand: Hand,
    auction: &Auction,
) -> Option<Call> {
    let natural = BidContext {
        policy: PolicyParams {
            epsilon: 0.0,
            deviation: 1.0,
            legacy_temperature: None,
        },
        ..*ctx
    };
    deterministic_choice(&call_distribution(table, hand, auction, &natural))
}

/// One call as a path token: leading passes as `P`, our side's calls plain, the opponents' in
/// parentheses (from `seat`'s point of view).
fn token(auction: &Auction, index: usize, seat: Seat) -> String {
    let call = auction.calls()[index];
    let s = call_str(call);
    if index < auction.leading_passes() || auction.seat_at(index).side() == seat.side() {
        s
    } else {
        format!("({s})")
    }
}

fn call_str(call: Call) -> String {
    match call {
        Call::Pass => "P".to_string(),
        Call::Double => "X".to_string(),
        Call::Redouble => "XX".to_string(),
        Call::Bid(b) => format!("{b}"),
    }
}

fn role_of(auction: &Auction, seat: Seat) -> String {
    let probe = auction
        .with(Call::Pass)
        .expect("Pass is legal while incomplete");
    format!("{:?}", classify(&probe, auction.len(), seat).role).to_lowercase()
}

/// Aggregation key of a position: the matched part of the path (leading passes stripped, so
/// the key is the trie position whatever the opener's seat; the sample path shows one seat),
/// the first unmatched call (empty when the whole prefix matched; later calls are not part of
/// the key, see the sample path), and the acting seat's role.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct PosKey {
    matched: String,
    unmatched: String,
    role: String,
}

fn pos_key(auction: &Auction, matched_depth: usize) -> PosKey {
    let seat = auction.next_seat();
    let lead = if LookupKey::for_auction(auction, seat).is_some() {
        auction.leading_passes()
    } else {
        auction.len()
    };
    let matched_abs = lead + matched_depth;
    let unmatched = if matched_abs < auction.len() {
        token(auction, matched_abs, seat)
    } else {
        String::new()
    };
    PosKey {
        matched: trie_path(auction, matched_depth),
        unmatched,
        role: role_of(auction, seat),
    }
}

/// Why a position left the system: the whole prefix matched but no legal child is left
/// (`exhausted`), or the walk stopped at a call that has no trie edge, split by whose call that
/// is and whether it is a `Pass` (our own passes are implicit unless a row makes them explicit).
fn departure_category(auction: &Auction, matched_depth: usize) -> &'static str {
    let seat = auction.next_seat();
    let lead = if LookupKey::for_auction(auction, seat).is_some() {
        auction.leading_passes()
    } else {
        auction.len()
    };
    let at = lead + matched_depth;
    if at >= auction.len() {
        return "exhausted";
    }
    let ours = auction.seat_at(at).side() == seat.side();
    match (ours, auction.calls()[at] == Call::Pass, matched_depth == 0) {
        (false, _, true) => "no_rows_for_their_opening",
        (true, true, _) => "our_pass_not_in_trie",
        (true, false, _) => "our_call_not_in_trie",
        (false, true, _) => "their_pass_not_in_trie",
        (false, false, _) => "their_call_not_in_trie",
    }
}

/// The matched calls of the exact resolve with the leading passes stripped: the label of the
/// trie position the resolve ends at (seat-independent).
fn trie_path(auction: &Auction, matched_depth: usize) -> String {
    let seat = auction.next_seat();
    if LookupKey::for_auction(auction, seat).is_none() {
        return "-".to_string();
    }
    let lead = auction.leading_passes();
    let parts: Vec<String> = (lead..lead + matched_depth)
        .map(|i| token(auction, i, seat))
        .collect();
    if parts.is_empty() {
        "-".to_string()
    } else {
        parts.join("-")
    }
}

#[derive(Default)]
struct Agg {
    count: u64,
    by_kind: BTreeMap<&'static str, u64>,
    sample_path: String,
    sample_hand: String,
    sample_hcp: u8,
}

fn record(map: &mut HashMap<PosKey, Agg>, key: PosKey, kind: &'static str, a: &Auction, h: Hand) {
    let e = map.entry(key).or_default();
    e.count += 1;
    *e.by_kind.entry(kind).or_default() += 1;
    if e.sample_path.is_empty() {
        e.sample_path = format!("{a}");
        e.sample_hand = format!("{h:?}");
        e.sample_hcp = bridge_eval::hcp(h);
    }
}

/// Total count per kind over every key of `map`.
fn by_kind_totals(map: &HashMap<PosKey, Agg>) -> BTreeMap<&'static str, u64> {
    let mut totals = BTreeMap::new();
    for agg in map.values() {
        for (&kind, &n) in &agg.by_kind {
            *totals.entry(kind).or_default() += n;
        }
    }
    totals
}

fn top(map: &HashMap<PosKey, Agg>, n: usize, scale: f64) -> Vec<Value> {
    let mut rows: Vec<(&PosKey, &Agg)> = map.iter().collect();
    rows.sort_by(|a, b| b.1.count.cmp(&a.1.count).then_with(|| a.0.cmp(b.0)));
    rows.truncate(n);
    rows.into_iter()
        .map(|(k, a)| {
            json!({
                "matched": k.matched,
                "unmatched": k.unmatched,
                "role": k.role,
                "count": a.count,
                "scaled": a.count as f64 * scale,
                "by_kind": a.by_kind,
                "sample_path": a.sample_path,
                "sample_hand": a.sample_hand,
                "sample_hcp": a.sample_hcp,
            })
        })
        .collect()
}

// ------------------------------------------------------------------------------------------
// Generated replays.
// ------------------------------------------------------------------------------------------

/// A random deal, dealt exactly like `crates/bridge-bidding/tests/common::random_deal`.
fn random_deal(rng: &mut impl Rng) -> Deal {
    let mut deck: Vec<u8> = (0..52).collect();
    for i in 0..52 {
        let j = i + (rng.next_u32() as usize) % (52 - i);
        deck.swap(i, j);
    }
    let hands = std::array::from_fn(|seat| {
        let mut hand = Hand::EMPTY;
        for &c in &deck[seat * 13..seat * 13 + 13] {
            hand = hand.with(Card::from_index(c).expect("index < 52"));
        }
        hand
    });
    Deal::new(hands).expect("a full-deck shuffle is a valid deal")
}

/// What happened at one generated position.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    System,
    SystemImplicitPass,
    Natural,
    Gap,
}

fn generated_report(table: &Table, ctx: &BidContext<'_>) -> Value {
    let n = env_usize("COVERAGE_REPLAYS", 1000);
    let seed = env_u64("COVERAGE_SEED", 0xC0FE_4001);
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
    let mut all_system = 0u64;
    let mut with_natural = 0u64;
    let mut with_gap = 0u64;
    let mut counts: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut levels = [0u64; 8];
    let mut gaps: HashMap<PosKey, Agg> = HashMap::new();
    let mut naturals: HashMap<PosKey, Agg> = HashMap::new();
    let mut departures: HashMap<PosKey, Agg> = HashMap::new();
    let mut total_calls = 0u64;
    let mut lenient_positions = 0u64;
    let mut natural_by_category: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut departure_by_category: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut natural_passes = 0u64;
    let mut only_passes_after_departure = 0u64;
    let mut default_passes = 0u64;
    let mut stop_passes = 0u64;
    // Strict accounting: a position whose only rows are default passes counts as a departure
    // when the natural choice there is not `Pass` (the pass is the stop's, not a decision).
    let mut all_system_strict = 0u64;
    let mut default_pass_only_positions = 0u64;
    let mut overrides = 0u64;
    let mut with_override = 0u64;
    let mut all_system_with_override = 0u64;
    let mut override_tops: HashMap<PosKey, Agg> = HashMap::new();
    let mut strict_departures: HashMap<PosKey, Agg> = HashMap::new();
    let mut strict_departure_by_category: BTreeMap<&'static str, u64> = BTreeMap::new();

    for _ in 0..n {
        let deal = random_deal(&mut rng);
        let dealer = Seat::from_index((rng.next_u32() % 4) as u8);
        let vul = Vulnerability::from_index((rng.next_u32() % 4) as u8);
        let mut auction = Auction::new(dealer, vul);
        let mut has_natural = false;
        let mut has_gap = false;
        let mut departed = false;
        let mut non_pass_after_departure = false;
        let mut has_override = false;
        let mut strict_departed = false;
        while !auction.is_complete() && auction.len() < 320 {
            let seat = auction.next_seat();
            let hand = deal.hand(seat);
            let system = &table.systems[seat.index() as usize];
            let (state, matched) = on_system(system, &auction);
            if state == OnSystem::Lenient {
                lenient_positions += 1;
            }
            let (call, outcome) = match choose_bid(table, hand, &auction, ctx) {
                BidChoice::Chosen(c) => {
                    if state != OnSystem::Off
                        && c.call == Call::Pass
                        && c.node
                            .is_some_and(|id| system.node(id).priority <= DEFAULT_PASS_PRIORITY)
                    {
                        default_passes += 1;
                        stop_passes +=
                            u64::from(c.node.is_some_and(|id| system.node(id).is_synthesised()));
                    }
                    let outcome = match (state, c.source) {
                        (OnSystem::Off, _) => Outcome::Natural,
                        (_, bridge_bidding::ChoiceSource::ImplicitPass) => {
                            Outcome::SystemImplicitPass
                        }
                        _ => Outcome::System,
                    };
                    (c.call, outcome)
                }
                BidChoice::NoCandidate(_) => (Call::Pass, Outcome::Gap),
            };
            if state != OnSystem::Off && default_pass_only(system, &auction) {
                default_pass_only_positions += 1;
                if let Some(m) = natural_choice(table, ctx, hand, &auction) {
                    if m != Call::Pass {
                        overrides += 1;
                        has_override = true;
                        let key = pos_key(&auction, matched);
                        record(
                            &mut override_tops,
                            key.clone(),
                            call_label(m),
                            &auction,
                            hand,
                        );
                        if !strict_departed {
                            strict_departed = true;
                            *strict_departure_by_category
                                .entry("default_pass_override")
                                .or_default() += 1;
                            record(
                                &mut strict_departures,
                                key,
                                "default_pass_override",
                                &auction,
                                hand,
                            );
                        }
                    }
                }
            }
            total_calls += 1;
            let label = match outcome {
                Outcome::System => "system",
                Outcome::SystemImplicitPass => "system_implicit_pass",
                Outcome::Natural => "natural",
                Outcome::Gap => "gap",
            };
            *counts.entry(label).or_default() += 1;
            if departed && call != Call::Pass {
                non_pass_after_departure = true;
            }
            if matches!(outcome, Outcome::Natural | Outcome::Gap) {
                let category = departure_category(&auction, matched);
                if outcome == Outcome::Natural {
                    *natural_by_category.entry(category).or_default() += 1;
                    if call == Call::Pass {
                        natural_passes += 1;
                    }
                }
                if !departed {
                    *departure_by_category.entry(category).or_default() += 1;
                    if call != Call::Pass {
                        non_pass_after_departure = true;
                    }
                }
                let key = pos_key(&auction, matched);
                if outcome == Outcome::Natural {
                    has_natural = true;
                    record(&mut naturals, key.clone(), call_label(call), &auction, hand);
                } else {
                    has_gap = true;
                    let kind = match state {
                        OnSystem::Exact => "exact",
                        OnSystem::Lenient => "lenient",
                        OnSystem::Off => "off_system",
                    };
                    record(&mut gaps, key.clone(), kind, &auction, hand);
                }
                if !strict_departed {
                    strict_departed = true;
                    *strict_departure_by_category.entry(category).or_default() += 1;
                    record(
                        &mut strict_departures,
                        key.clone(),
                        category,
                        &auction,
                        hand,
                    );
                }
                if !departed {
                    departed = true;
                    record(&mut departures, key, category, &auction, hand);
                }
            }
            auction.push(call).expect("choose_bid returns a legal call");
        }
        if has_natural {
            with_natural += 1;
        }
        if has_gap {
            with_gap += 1;
        }
        if has_override {
            with_override += 1;
        }
        if !has_natural && !has_gap {
            all_system += 1;
            if has_override {
                all_system_with_override += 1;
            } else {
                all_system_strict += 1;
            }
        } else if !non_pass_after_departure {
            only_passes_after_departure += 1;
        }
        let level = auction.contract().map_or(0, |c| c.bid.level() as usize);
        levels[level] += 1;
    }

    json!({
        "replays": n,
        "seed": seed,
        "all_system": all_system,
        "all_system_rate": all_system as f64 / n.max(1) as f64,
        "system_default_passes": default_passes,
        "system_stop_passes": stop_passes,
        "all_system_strict": all_system_strict,
        "all_system_strict_rate": all_system_strict as f64 / n.max(1) as f64,
        "all_system_with_default_pass_override": all_system_with_override,
        "default_pass_only_positions": default_pass_only_positions,
        "default_pass_overrides": overrides,
        "auctions_with_default_pass_override": with_override,
        "default_pass_override_top50": top(&override_tops, 50, 1.0),
        "first_strict_departure_by_category": strict_departure_by_category,
        "first_strict_departure_top50": top(&strict_departures, 50, 1.0),
        "auctions_with_natural": with_natural,
        "auctions_with_gap": with_gap,
        "calls": total_calls,
        "call_outcomes": counts,
        "lenient_positions": lenient_positions,
        "natural_passes": natural_passes,
        "natural_by_category": natural_by_category,
        "first_departure_by_category": departure_by_category,
        "departed_with_only_passes_after": only_passes_after_departure,
        "final_level_histogram": levels,
        "final_level_labels": ["passout", "1", "2", "3", "4", "5", "6", "7"],
        "no_candidate_top50": top(&gaps, 50, 1.0),
        "first_departure_top50": top(&departures, 50, 1.0),
        "natural_completion_top50": top(&naturals, 50, 1.0),
        // Authoring aid: `COVERAGE_TOP_N=<n>` also lists the first `n` departures.
        "first_departure_top_n": top(&departures, env_usize("COVERAGE_TOP_N", 0), 1.0),
    })
}

/// A short, static label for a natural call's kind (for `by_kind` of the natural tops).
fn call_label(call: Call) -> &'static str {
    match call {
        Call::Pass => "pass",
        Call::Double => "double",
        Call::Redouble => "redouble",
        Call::Bid(b) if b.strain() == bridge_core::Strain::NoTrump => "nt",
        Call::Bid(_) => "suit",
    }
}

// ------------------------------------------------------------------------------------------
// Forward-consistency-style positions (NoCandidate per 10^6).
// ------------------------------------------------------------------------------------------

/// `crates/bridge-bidding/tests/common::random_sayc_position_with_substitution`, verbatim in its
/// use of the RNG, so the same seed draws the same positions as the consistency harness.
fn random_position(
    rng: &mut impl Rng,
    table: &Table,
    ctx: &BidContext<'_>,
    random_call_rate: f64,
) -> (Deal, Auction) {
    let deal = random_deal(rng);
    let dealer = Seat::from_index((rng.next_u32() % 4) as u8);
    let vul = Vulnerability::from_index((rng.next_u32() % 4) as u8);
    let mut auction = Auction::new(dealer, vul);
    let depth = rng.next_u32() % 12;
    let checked = auction.seat_at(depth as usize);
    for _ in 0..depth {
        if auction.is_complete() {
            break;
        }
        let seat = auction.next_seat();
        let substitute = random_call_rate > 0.0
            && seat != checked
            && ((rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64) < random_call_rate;
        let call = if substitute {
            let legal: Vec<Call> = auction.legal_calls().collect();
            legal[(rng.next_u64() % legal.len() as u64) as usize]
        } else {
            match choose_bid(table, deal.hand(seat), &auction, ctx) {
                BidChoice::Chosen(c) => c.call,
                BidChoice::NoCandidate(_) => Call::Pass,
            }
        };
        auction = auction.with(call).expect("legal");
    }
    (deal, auction)
}

fn positions_report(table: &Table, ctx: &BidContext<'_>) -> Value {
    let n = env_usize("COVERAGE_POSITIONS", 200_000);
    let seed = env_u64("COVERAGE_POSITIONS_SEED", POSITIONS_SEED);
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
    let mut gaps: HashMap<PosKey, Agg> = HashMap::new();
    let mut on_system_gaps: HashMap<PosKey, Agg> = HashMap::new();
    let mut by_node: HashMap<(String, String), (u64, Vec<String>)> = HashMap::new();
    let mut by_trie: HashMap<(u32, String), (u64, String)> = HashMap::new();
    let (mut chosen, mut no_candidate, mut implicit_pass, mut natural) = (0u64, 0u64, 0u64, 0u64);
    for _ in 0..n {
        let (deal, auction) =
            std::iter::repeat_with(|| random_position(&mut rng, table, ctx, RANDOM_CALL_RATE))
                .find(|(_, a)| !a.is_complete())
                .expect("eventually incomplete");
        let seat = auction.next_seat();
        let hand = deal.hand(seat);
        let system = &table.systems[seat.index() as usize];
        match choose_bid(table, hand, &auction, ctx) {
            BidChoice::Chosen(c) => match c.source {
                bridge_bidding::ChoiceSource::ImplicitPass => implicit_pass += 1,
                bridge_bidding::ChoiceSource::Natural => natural += 1,
                bridge_bidding::ChoiceSource::System => chosen += 1,
            },
            BidChoice::NoCandidate(_) => {
                no_candidate += 1;
                let (state, matched) = on_system(system, &auction);
                let kind = match state {
                    OnSystem::Exact => "exact",
                    OnSystem::Lenient => "lenient",
                    OnSystem::Off => "off_system",
                };
                let key = pos_key(&auction, matched);
                let e = by_node
                    .entry((trie_path(&auction, matched), key.role.clone()))
                    .or_default();
                e.0 += 1;
                if e.1.len() < 8 {
                    e.1.push(format!("{auction} | {hand:?}"));
                }
                if state != OnSystem::Off {
                    record(&mut on_system_gaps, key.clone(), kind, &auction, hand);
                }
                record(&mut gaps, key, kind, &auction, hand);
                let trie = system.index.resolve(&key_for(&auction, seat)).end.0;
                let e = by_trie
                    .entry((trie, role_of(&auction, seat)))
                    .or_insert_with(|| (0, format!("{auction}")));
                e.0 += 1;
            }
        }
    }
    let scale = 1e6 / n.max(1) as f64;
    // The phase-3 NoCandidate tops (12-roadmap, phase 3 row), keyed like that report: by the
    // trie position the exact resolve ends at (the matched calls, leading passes stripped) and
    // the role, whatever call stopped the walk.
    let watch: Vec<Value> = PHASE3_TOPS
        .iter()
        .map(|&(matched, role)| {
            let (count, samples) = by_node
                .get(&(matched.to_string(), role.to_string()))
                .cloned()
                .unwrap_or_default();
            json!({
                "trie_path": matched, "role": role, "count": count,
                "per_1e6": count as f64 * scale, "samples": samples,
            })
        })
        .collect();
    let mut trie_rows: Vec<_> = by_trie.into_iter().collect();
    trie_rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
    trie_rows.truncate(50);
    json!({
        "positions": n,
        "seed": seed,
        "random_call_rate": RANDOM_CALL_RATE,
        "chosen_system": chosen,
        "chosen_natural": natural,
        "implicit_pass": implicit_pass,
        "no_candidate": no_candidate,
        "no_candidate_per_1e6": no_candidate as f64 * scale,
        "no_candidate_on_system": on_system_gaps.values().map(|a| a.count).sum::<u64>(),
        "phase3_tops": watch,
        "no_candidate_top50": top(&gaps, 50, scale),
        "no_candidate_on_system_top50": top(&on_system_gaps, 50, scale),
        "no_candidate_top50_by_trie": trie_rows.into_iter().map(|((trie, role), (count, path))| json!({
            "trie": trie, "role": role, "count": count, "per_1e6": count as f64 * scale, "sample_path": path,
        })).collect::<Vec<_>>(),
    })
}

// ------------------------------------------------------------------------------------------
// Corpus.
// ------------------------------------------------------------------------------------------

fn corpus_dir(root: &Path) -> Option<PathBuf> {
    let dir = match std::env::var_os("BRIDGE_CORPUS_DIR") {
        Some(d) => PathBuf::from(d),
        None => root.join("corpus/data"),
    };
    dir.is_dir().then_some(dir)
}

fn files_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, ext, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, ext, &mut out);
    out.sort();
    out
}

/// The corpus auctions in enumeration order (PBN files recursively in sorted order, games in
/// file order, then LIN files likewise), with the deal when it is complete. The PBN part is the
/// same enumeration as `crates/bridge-bidding/tests/reproduction.rs`'s `corpus_auctions`, so
/// indices (and the D20 split) agree with it.
fn corpus_games(dir: &Path) -> (usize, Vec<(Auction, Option<Deal>)>) {
    let mut out = Vec::new();
    let pbn_files = files_with_ext(&dir.join("pbn"), "pbn");
    let lin_files = files_with_ext(&dir.join("lin"), "lin");
    let n_files = pbn_files.len() + lin_files.len();
    for path in pbn_files {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let (file, _warnings) = bridge_format::pbn::parse_lenient(&bytes);
        let mut previous: Option<bridge_format::GameView> = None;
        for game in &file.games {
            let view = game.view(previous.as_ref()).ok();
            if let Some(view) = &view {
                if let Some(auction) = &view.auction {
                    let deal = view.deal.as_ref().and_then(|p| p.complete());
                    out.push((auction.clone(), deal));
                }
            }
            previous = view;
        }
    }
    for path in lin_files {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let (boards, _warnings) = bridge_format::lin::parse_lenient(&bytes);
        for board in &boards {
            let game = board.to_game();
            let Ok(view) = game.view(None) else { continue };
            if let Some(auction) = &view.auction {
                let deal = view.deal.as_ref().and_then(|p| p.complete());
                out.push((auction.clone(), deal));
            }
        }
    }
    (n_files, out)
}

#[derive(Default)]
struct KindStats {
    auctions: u64,
    all_exact: u64,
    calls: u64,
    exact: u64,
    partial: u64,
    natural: u64,
    fallback: u64,
    shadowed: u64,
    empty_strict_seats: u64,
    empty_default_seats: u64,
    lenient_calls: u64,
}

impl KindStats {
    fn to_json(&self) -> Value {
        let c = self.calls.max(1) as f64;
        json!({
            "auctions": self.auctions,
            "all_exact": self.all_exact,
            "all_exact_rate": self.all_exact as f64 / self.auctions.max(1) as f64,
            "calls": self.calls,
            "kinds": { "exact": self.exact, "partial": self.partial, "natural": self.natural, "fallback": self.fallback },
            "shadowed": self.shadowed,
            "system_resolution_rate": (self.exact + self.partial) as f64 / c,
            "exact_rate": self.exact as f64 / c,
            "partial_rate": self.partial as f64 / c,
            "natural_rate": self.natural as f64 / c,
            "empty_strict_support_seats": self.empty_strict_seats,
            "sampler_empty_support_seats": self.empty_default_seats,
            "resolve_lenient_calls": self.lenient_calls,
            "resolve_lenient_rate": self.lenient_calls as f64 / c,
        })
    }
}

/// One corpus call's policy summary, for agreement and the `(ε, δ)` fit.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct PolicyObs {
    on_system: bool,
    /// 0: the system choice `s_P(h)` is the human call; 1: another call; 2: `⊥` (uniform).
    s: u8,
    /// Same for the natural choice `m_P(h)`.
    m: u8,
    /// Number of legal calls.
    n: u8,
}

/// The first call with probability ~1 in a distribution computed with `ε = 0`, or `None` when
/// the distribution is uniform (`⊥`).
fn deterministic_choice(dist: &[(Call, f32)]) -> Option<Call> {
    dist.iter().find(|(_, p)| *p > 0.5).map(|(c, _)| *c)
}

fn obs_state(choice: Option<Call>, call: Call) -> u8 {
    match choice {
        Some(c) if c == call => 0,
        Some(_) => 1,
        None => 2,
    }
}

fn policy_obs(
    table: &Table,
    ctx: &BidContext<'_>,
    prefix: &Auction,
    hand: Hand,
    call: Call,
) -> PolicyObs {
    let seat = prefix.next_seat();
    let system = &table.systems[seat.index() as usize];
    let on = on_system(system, prefix).0 != OnSystem::Off;
    let with = |epsilon: f32, deviation: f32| BidContext {
        policy: PolicyParams {
            epsilon,
            deviation,
            legacy_temperature: None,
        },
        ..*ctx
    };
    let n = prefix.legal_calls().count() as u8;
    let m = deterministic_choice(&call_distribution(table, hand, prefix, &with(0.0, 1.0)));
    let s = if on {
        deterministic_choice(&call_distribution(table, hand, prefix, &with(0.0, 0.0)))
    } else {
        None
    };
    PolicyObs {
        on_system: on,
        s: obs_state(s, call),
        m: obs_state(m, call),
        n,
    }
}

/// `ln p(c|h)` of one observation class under `(ε, δ)`.
fn obs_log_p(o: &PolicyObs, eps: f64, delta: f64) -> f64 {
    let n = f64::from(o.n);
    let ind = |state: u8| match state {
        0 => 1.0,
        1 => 0.0,
        _ => 1.0 / n,
    };
    let pi = if o.on_system {
        (1.0 - delta) * ind(o.s) + delta * ind(o.m)
    } else {
        ind(o.m)
    };
    ((1.0 - eps) * pi + eps / n).ln()
}

fn log_lik(obs: &BTreeMap<PolicyObs, u64>, eps: f64, delta: f64) -> f64 {
    obs.iter()
        .map(|(o, &k)| k as f64 * obs_log_p(o, eps, delta))
        .sum()
}

/// Grid MLE of `(ε, δ)`: a coarse grid (ε log-spaced 1e-5..0.9, δ in 0..=0.99), then a fine
/// grid around the coarse optimum.
fn fit(obs: &BTreeMap<PolicyObs, u64>) -> (f64, f64, f64) {
    let mut best = (f64::NEG_INFINITY, 0.0, 0.0);
    for i in 0..100 {
        let eps = 10f64.powf(-5.0 + 0.05 * f64::from(i));
        for j in 0..100 {
            let delta = f64::from(j) * 0.01;
            let ll = log_lik(obs, eps, delta);
            if ll > best.0 {
                best = (ll, eps, delta);
            }
        }
    }
    let (_, e0, d0) = best;
    for i in -50..=50 {
        let eps = e0 * 10f64.powf(0.002 * f64::from(i));
        if eps >= 1.0 {
            continue;
        }
        for j in -100..=100 {
            let delta = d0 + 0.0001 * f64::from(j);
            if !(0.0..1.0).contains(&delta) {
                continue;
            }
            let ll = log_lik(obs, eps, delta);
            if ll > best.0 {
                best = (ll, eps, delta);
            }
        }
    }
    best
}

#[derive(Default)]
struct Agreement {
    system_positions: u64,
    system_agree: u64,
    system_no_choice: u64,
    natural_positions: u64,
    natural_agree: u64,
    natural_no_choice: u64,
}

impl Agreement {
    fn add(&mut self, o: &PolicyObs) {
        if o.on_system {
            self.system_positions += 1;
            match o.s {
                0 => self.system_agree += 1,
                2 => self.system_no_choice += 1,
                _ => {}
            }
        } else {
            self.natural_positions += 1;
            match o.m {
                0 => self.natural_agree += 1,
                2 => self.natural_no_choice += 1,
                _ => {}
            }
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "system_positions": self.system_positions,
            "system_agree": self.system_agree,
            "system_no_choice": self.system_no_choice,
            "system_agreement": self.system_agree as f64 / self.system_positions.max(1) as f64,
            "natural_positions": self.natural_positions,
            "natural_agree": self.natural_agree,
            "natural_no_choice": self.natural_no_choice,
            "natural_agreement": self.natural_agree as f64 / self.natural_positions.max(1) as f64,
            "overall_agreement": (self.system_agree + self.natural_agree) as f64
                / (self.system_positions + self.natural_positions).max(1) as f64,
        })
    }
}

/// Whether the true opener's hand lies in the exclusive region X of the recorded opening
/// (`None` for a passed-out auction).
fn sayc_compatible_opening(table: &Table, auction: &Auction, deal: &Deal) -> Option<bool> {
    let k = auction.leading_passes();
    if k >= auction.len() {
        return None;
    }
    let opener = auction.seat_at(k);
    let system = &table.systems[opener.index() as usize];
    let opening = auction.calls()[k];
    let hand = deal.hand(opener);
    let root = bridge_system::trie::TrieId(0);
    let pieces = system.exclusive().pieces(
        root,
        auction.position_of(opener),
        rel_vul(auction, opener),
        opening,
    );
    Some(pieces.is_some_and(|ps| ps.iter().any(|p| p.constraint.satisfies(hand))))
}

/// Whether call `index` of `auction` needed `resolve_lenient` in its caller's system: the
/// exact resolve of the prefix including the call stops short, and a full lenient match exists.
fn used_lenient(table: &Table, auction: &Auction, index: usize) -> bool {
    let seat = auction.seat_at(index);
    let Ok(prefix) = Auction::from_calls(
        auction.dealer(),
        auction.vulnerability(),
        auction.calls()[..=index].iter().copied(),
    ) else {
        return false;
    };
    let Some(key) = LookupKey::for_auction(&prefix, seat) else {
        return false;
    };
    let system = &table.systems[seat.index() as usize];
    if system.index.resolve(&key).matched_depth == key.calls.len() {
        return false;
    }
    system
        .index
        .resolve_lenient(&key, LENIENT_MAX_SUBST)
        .iter()
        .any(|(lk, _)| lk.matched_depth == key.calls.len())
}

fn corpus_report(table: &Table, ctx: &BidContext<'_>, dir: &Path) -> Value {
    let (n_files, mut games) = corpus_games(dir);
    let limit = env_usize("COVERAGE_CORPUS_LIMIT", usize::MAX);
    games.truncate(limit);
    let opts = InterpretOptions::for_context(ctx);
    let strict_opts = InterpretOptions {
        strict: true,
        ..opts
    };
    let mut all = KindStats::default();
    let mut tune = KindStats::default();
    let mut eval = KindStats::default();
    let mut subset = KindStats::default();
    let mut subset_eval = KindStats::default();
    let mut with_deal = 0u64;
    let mut passed_out = 0u64;
    let mut opening_checked = 0u64;
    let mut agree_all = Agreement::default();
    let mut agree_tune = Agreement::default();
    let mut agree_eval = Agreement::default();
    let mut obs_tune: BTreeMap<PolicyObs, u64> = BTreeMap::new();
    let mut obs_eval: BTreeMap<PolicyObs, u64> = BTreeMap::new();
    let print_lenient = std::env::var_os("COVERAGE_PRINT_LENIENT").is_some();
    let mut subset_natural: HashMap<PosKey, Agg> = HashMap::new();
    let mut subset_first_natural: HashMap<PosKey, Agg> = HashMap::new();

    for (i, (auction, deal)) in games.iter().enumerate() {
        let interp = interpret(table, auction, &opts);
        let strict = interpret(table, auction, &strict_opts);
        let mut st = KindStats {
            auctions: 1,
            ..KindStats::default()
        };
        let mut every_exact = true;
        for pc in &interp.per_call {
            st.calls += 1;
            match pc.kind {
                ResolutionKind::Exact => st.exact += 1,
                ResolutionKind::Partial { .. } => st.partial += 1,
                ResolutionKind::Natural => st.natural += 1,
                ResolutionKind::Fallback => st.fallback += 1,
            }
            if pc.kind != ResolutionKind::Exact {
                every_exact = false;
            }
            if pc.shadowed {
                st.shadowed += 1;
            }
            if used_lenient(table, auction, pc.call_index) {
                st.lenient_calls += 1;
                if print_lenient {
                    eprintln!("lenient: {auction} call {}", pc.call_index);
                }
            }
        }
        if every_exact {
            st.all_exact = 1;
        }
        for seat in Seat::ALL {
            if auction.calls_by(seat).next().is_none() {
                continue;
            }
            let s = seat.index() as usize;
            if strict.seats[s].iter().all(|(c, _, _)| !c.is_satisfiable()) {
                st.empty_strict_seats += 1;
            }
            if interp.seats[s].iter().all(|(c, _, _)| !c.is_satisfiable()) {
                st.empty_default_seats += 1;
                if print_lenient {
                    eprintln!("empty default support: auction {i} {auction} seat {seat:?}");
                }
            }
        }
        let is_tune = i % 2 == 0;
        add_stats(&mut all, &st);
        add_stats(if is_tune { &mut tune } else { &mut eval }, &st);
        if auction.leading_passes() >= auction.len() {
            passed_out += 1;
        }

        let Some(deal) = deal else { continue };
        with_deal += 1;
        if let Some(ok) = sayc_compatible_opening(table, auction, deal) {
            opening_checked += 1;
            if ok {
                add_stats(&mut subset, &st);
                if !is_tune {
                    add_stats(&mut subset_eval, &st);
                }
                // Where the subset's calls leave the system (authoring aid for 4.2-4.4).
                let mut first = true;
                for pc in &interp.per_call {
                    if pc.kind != ResolutionKind::Natural {
                        continue;
                    }
                    let Ok(prefix) = Auction::from_calls(
                        auction.dealer(),
                        auction.vulnerability(),
                        auction.calls()[..pc.call_index].iter().copied(),
                    ) else {
                        continue;
                    };
                    let seat = prefix.next_seat();
                    let system = &table.systems[seat.index() as usize];
                    let (state, depth) = on_system(system, &prefix);
                    let key = pos_key(&prefix, depth);
                    // On-system with the human call not among the rows, or why the position
                    // itself is off the system.
                    let kind = if state == OnSystem::Off {
                        departure_category(&prefix, depth)
                    } else if default_pass_only(system, &prefix) {
                        "call_not_a_row_default_pass_only"
                    } else {
                        "call_not_a_row"
                    };
                    record(
                        &mut subset_natural,
                        key.clone(),
                        kind,
                        &prefix,
                        deal.hand(seat),
                    );
                    if first {
                        record(
                            &mut subset_first_natural,
                            key,
                            kind,
                            &prefix,
                            deal.hand(seat),
                        );
                        first = false;
                    }
                }
            }
        }
        let mut prefix = Auction::new(auction.dealer(), auction.vulnerability());
        for &call in auction.calls() {
            if prefix.is_complete() {
                break;
            }
            let hand = deal.hand(prefix.next_seat());
            let o = policy_obs(table, ctx, &prefix, hand, call);
            agree_all.add(&o);
            if is_tune {
                agree_tune.add(&o);
                *obs_tune.entry(o).or_default() += 1;
            } else {
                agree_eval.add(&o);
                *obs_eval.entry(o).or_default() += 1;
            }
            if prefix.push(call).is_err() {
                break;
            }
        }
    }

    let (ll, eps_hat, delta_hat) = fit(&obs_tune);
    let n_tune: u64 = obs_tune.values().sum();
    let n_eval: u64 = obs_eval.values().sum();
    let delta_curve: Vec<Value> = (0..100)
        .map(|j| {
            let d = f64::from(j) * 0.01;
            json!([d, log_lik(&obs_tune, eps_hat, d)])
        })
        .collect();
    let eps_curve: Vec<Value> = (0..100)
        .map(|i| {
            let e = 10f64.powf(-5.0 + 0.05 * f64::from(i));
            json!([e, log_lik(&obs_tune, e, delta_hat)])
        })
        .collect();
    let class_counts: Vec<Value> = obs_tune
        .iter()
        .map(|(o, k)| json!({"on_system": o.on_system, "s": o.s, "m": o.m, "n": o.n, "count": k}))
        .collect();
    let human = PolicyParams::human();
    let sp = PolicyParams::system_players();

    json!({
        "files": n_files,
        "auctions": games.len(),
        "with_deal": with_deal,
        "passed_out": passed_out,
        "opening_checked": opening_checked,
        "split_rule": "enumeration index: even = tune, odd = eval",
        "all": all.to_json(),
        "tune": tune.to_json(),
        "eval": eval.to_json(),
        "sayc_compatible_opening": subset.to_json(),
        "sayc_compatible_opening_eval": subset_eval.to_json(),
        "sayc_compatible_opening_natural_top50": top(&subset_natural, 50, 1.0),
        "sayc_compatible_opening_first_natural_top50": top(&subset_first_natural, 50, 1.0),
        "sayc_compatible_opening_first_natural_by_kind": by_kind_totals(&subset_first_natural),
        "sayc_compatible_opening_natural_by_kind": by_kind_totals(&subset_natural),
        "agreement": {
            "all": agree_all.to_json(),
            "tune": agree_tune.to_json(),
            "eval": agree_eval.to_json(),
        },
        "mle": {
            "split": "tune",
            "calls": n_tune,
            "epsilon": eps_hat,
            "deviation": delta_hat,
            "log_likelihood": ll,
            "log_likelihood_per_call": ll / n_tune.max(1) as f64,
            "eval_calls": n_eval,
            "eval_log_likelihood_at_mle": log_lik(&obs_eval, eps_hat, delta_hat),
            "tune_log_likelihood_at_placeholder_human": log_lik(&obs_tune, f64::from(human.epsilon), f64::from(human.deviation)),
            "tune_log_likelihood_at_system_players": log_lik(&obs_tune, f64::from(sp.epsilon), f64::from(sp.deviation)),
            "curve_delta_at_eps_hat": delta_curve,
            "curve_eps_at_delta_hat": eps_curve,
            "class_counts": class_counts,
            "class_legend": "s/m: 0 = the system/natural choice is the human call, 1 = another call, 2 = no choice (uniform)",
        },
    })
}

fn add_stats(into: &mut KindStats, st: &KindStats) {
    into.auctions += st.auctions;
    into.all_exact += st.all_exact;
    into.calls += st.calls;
    into.exact += st.exact;
    into.partial += st.partial;
    into.natural += st.natural;
    into.fallback += st.fallback;
    into.shadowed += st.shadowed;
    into.empty_strict_seats += st.empty_strict_seats;
    into.empty_default_seats += st.empty_default_seats;
    into.lenient_calls += st.lenient_calls;
}
