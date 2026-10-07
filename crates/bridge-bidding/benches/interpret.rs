//! Criterion benches for `interpret` (target: under 10 µs for a 12-call auction) and
//! `sequence_log_likelihood`.
//!
//! Two hand-built systems: `bench_system` (bare HCP atoms only, so every node's `shapes()` is
//! exactly `ShapeSet::ALL`) and `bench_system_realistic` (each node also names its own suit's
//! length). `summary_satisfiable`'s `shapes == ShapeSet::ALL` shortcut means the bare-HCP system
//! alone cannot exercise the `ShapeSet::hcp_bounds` per-byte table lookups in Step B's
//! cross-product pre-check — nearly every real system node restricts at least one suit's length,
//! so both are benched.
//!
//! Two more benches (`bench_interpret_sayc_1nt`/`_competitive`) interpret real auctions against
//! the actual compiled SAYC system (`systems/sayc/sayc.bml`), rather than a hand-built stand-in:
//! `1NT-P-2C-P-2H-P-3NT-P-P-P` (a natural, non-competitive auction) and
//! `1S-(2H)-X-(P)-3S-(P)-P-P` (a competitive one, with a negative double and a raise under
//! interference).
//!
//! Phase 4 (docs/design/15-phase4-plan.md, lane B step 7) adds:
//!
//! - `interpret/sayc-12-call-on-policy`: a 12-call SAYC auction in which every call is one the
//!   policy makes (the acceptance bench `interpret/sayc-12-call-auction` ends in an off-policy,
//!   shadowed 3NT).
//! - `interpret/natural-heavy-auction`: a competitive corpus auction with 9 natural calls.
//! - `interpret-step-a/*`: Step A alone (`interpret_per_call`), for the Step A / Step B split
//!   (Step B = `interpret/*` minus `interpret-step-a/*`).
//! - `interpret-cold/*`: the first interpretation of an auction under a table whose positions are
//!   not memoised yet (a fresh natural-engine allocation per iteration, so every position misses
//!   the per-thread memo); `(cold - warm) / natural calls` is the cold cost per natural call.
//! - `interpret-human/sayc-12-call-auction`: the mirror under `PolicyParams::human()` (`δ > 0`
//!   adds the natural pieces at on-system positions).
//! - `auction-policy/*`: `AuctionPolicy::log_likelihood` per deal (and `AuctionPolicy::new`).

use std::sync::Arc;

use bridge_bidding::{
    AuctionPolicy, BidContext, ImplicitPass, InterpretOptions, PolicyParams, Scoring, Table,
    interpret, interpret_per_call, sequence_log_likelihood,
};
use bridge_constraint::{Atom, HandConstraint};
use bridge_core::{Auction, Bid, Call, Deal, Hand, Seat, Strain, Suit, Vulnerability};
use bridge_system::ast::{SeatCond, VulCond};
use bridge_system::pattern::{Binding, Side};
use bridge_system::{
    Alertability, Forcing, Node, NodeFlags, NodeId, Recognition, Row, RowId, SystemIR, SystemMeta,
};
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};

fn bid(level: u8, strain: Strain) -> Call {
    Call::Bid(Bid::new(level, strain).unwrap())
}

fn atom_hcp(lo: u8, hi: u8) -> HandConstraint {
    HandConstraint::Atom(Atom::ANY.with_hcp(lo..=hi))
}

/// `suit` has length `>=lo`, HCP in `hcp_lo..=hcp_hi`: a *realistic* node constraint (an opening
/// bid or raise always says something about its own suit), unlike `atom_hcp`'s bare-HCP atom
/// whose `shapes()` is exactly `ShapeSet::ALL` — the one case `summary_satisfiable`/`Summary::of`
/// can skip the `ShapeSet::hcp_bounds` per-byte table lookups on for free. See
/// `bench_system_realistic`.
fn atom_suit_hcp(suit: Suit, lo: u8, hcp_lo: u8, hcp_hi: u8) -> HandConstraint {
    HandConstraint::Atom(
        Atom::ANY
            .with_suit_len(suit, lo..=13)
            .with_hcp(hcp_lo..=hcp_hi),
    )
}

/// `Strain::Clubs..=Strain::Spades` map onto `Suit` at identical indices; `bench_system`/
/// `bench_system_realistic` never bid `Strain::NoTrump`.
fn strain_to_suit(strain: Strain) -> Suit {
    match strain {
        Strain::Clubs => Suit::Clubs,
        Strain::Diamonds => Suit::Diamonds,
        Strain::Hearts => Suit::Hearts,
        Strain::Spades => Suit::Spades,
        Strain::NoTrump => unreachable!("bench auctions never bid notrump"),
    }
}

/// Builds a minimal node (most fields are irrelevant to `interpret`'s hot path).
///
/// `we_opened` selects which partnership's trie root this call is attached under (07-bidding.md
/// §3.2 / `AuctionTrie::root_id`): `LookupKey::for_auction` computes it as "does the calling
/// seat's side match the auction's opener's side", so with `Table::uniform` sharing one
/// `SystemIR` across all four seats, a 12-call auction where all four seats make real (non-Pass)
/// calls needs both roots populated at the positions where each side actually calls — never just
/// `true` for every node, or the opposing side's own calls have no trie coverage at all and fall
/// through to `NaturalInference` (still `todo!()` on this branch).
#[allow(clippy::too_many_arguments)]
fn make_node(
    ir: &mut SystemIR,
    we_opened: bool,
    calls: Vec<Call>,
    call: Call,
    constraint: HandConstraint,
    seat: SeatCond,
    vul: VulCond,
    description: &str,
    priority: i16,
) -> NodeId {
    let id = NodeId(ir.nodes.len() as u32);
    let row_id = RowId(ir.rows.len() as u32);
    ir.rows.push(Row {
        id: row_id,
        span: bridge_system::ast::Span {
            file: bridge_system::ast::FileId(0),
            line: 1,
            col: 0,
            pasted_from: None,
        },
        path: Arc::from(vec![]),
        description_raw: description.to_string(),
        recognition: Recognition::default(),
        expansions: vec![id],
    });
    ir.index.insert(we_opened, &calls, seat, vul, id).ok();
    ir.nodes.push(Node {
        id,
        row: row_id,
        side: Side::Us,
        path: Arc::from(vec![]),
        calls,
        call,
        binding: Binding::default(),
        seat,
        vul,
        constraint,
        branch_weights: None,
        priority,
        volume_log2: 0,
        alertable: Alertability::Unspecified,
        flags: NodeFlags {
            forcing: Forcing::Unknown,
            ..NodeFlags::default()
        },
        description: description.to_string(),
        children: Vec::new(),
    });
    id
}

/// A tiny hand-built SAYC-like system deep enough for a 12-call auction: 1C opening, overcalls
/// and rebids through 1C-1D-1H-1S-2C-2D-2H-2S-3C-3D-3H-3S (each level just re-describes "more of
/// the same" so the auction stays legal and in-system throughout).
fn bench_system() -> SystemIR {
    let mut ir = SystemIR {
        meta: SystemMeta::default(),
        rows: Vec::new(),
        nodes: Vec::new(),
        index: bridge_system::AuctionTrie::new(),
        lints: Vec::new(),
        exclusive_cell: Default::default(),
    };

    let strains = [
        Strain::Clubs,
        Strain::Diamonds,
        Strain::Hearts,
        Strain::Spades,
    ];
    let mut path = Vec::new();
    let mut j = 0usize; // Flat call index into the 12-call auction (North deals, so seat parity
    // alternates N,E,S,W,...; even j = North/South's own call, odd j = East/West's).
    for level in 1..=3u8 {
        for &strain in &strains {
            let call = bid(level, strain);
            let mut calls = path.clone();
            calls.push(call);
            make_node(
                &mut ir,
                j % 2 == 0,
                calls.clone(),
                call,
                atom_hcp(10, 24),
                SeatCond::Any,
                VulCond::default(),
                "12 calls of natural bidding",
                0,
            );
            path = calls;
            j += 1;
        }
    }
    ir
}

/// Same shape as `bench_system`, but every node's constraint names its own suit (`>=4` cards,
/// `10-24` HCP) instead of a bare HCP atom — the realistic case `summary_satisfiable`'s
/// `shapes == ShapeSet::ALL` shortcut cannot take, and so the case `Summary`'s incremental
/// `ShapeSet::min_hcp`/`max_hcp` bounds (only recomputed when the shape set actually narrows) are
/// for. See the module doc for why the plain `bench_system` alone hid this cost.
fn bench_system_realistic() -> SystemIR {
    let mut ir = SystemIR {
        meta: SystemMeta::default(),
        rows: Vec::new(),
        nodes: Vec::new(),
        index: bridge_system::AuctionTrie::new(),
        lints: Vec::new(),
        exclusive_cell: Default::default(),
    };

    let strains = [
        Strain::Clubs,
        Strain::Diamonds,
        Strain::Hearts,
        Strain::Spades,
    ];
    let mut path = Vec::new();
    let mut j = 0usize;
    for level in 1..=3u8 {
        for &strain in &strains {
            let call = bid(level, strain);
            let mut calls = path.clone();
            calls.push(call);
            make_node(
                &mut ir,
                j % 2 == 0,
                calls.clone(),
                call,
                atom_suit_hcp(strain_to_suit(strain), 4, 10, 24),
                SeatCond::Any,
                VulCond::default(),
                "12 calls of natural bidding, own-suit length required",
                0,
            );
            path = calls;
            j += 1;
        }
    }
    ir
}

fn bench_auction() -> Auction {
    let strains = [
        Strain::Clubs,
        Strain::Diamonds,
        Strain::Hearts,
        Strain::Spades,
    ];
    let mut calls = Vec::new();
    for level in 1..=3u8 {
        for &strain in &strains {
            calls.push(bid(level, strain));
        }
    }
    Auction::from_calls(Seat::North, Vulnerability::None, calls).unwrap()
}

fn bench_interpret(c: &mut Criterion) {
    let system = Arc::new(bench_system());
    let natural = Arc::new(bridge_system::NaturalInference::default());
    let table = Table::uniform(system, natural);
    let auction = bench_auction();
    let opts = InterpretOptions::default();

    c.bench_function("interpret/12-call-auction", |b| {
        b.iter(|| std::hint::black_box(interpret(&table, &auction, &opts)))
    });
}

/// An arbitrary, fixed 4-way deal (a fresh full-deck deal in index order), used only to give
/// `sequence_log_likelihood`'s benches a `Deal` to score; the hands' actual content is irrelevant
/// to the timing.
fn bench_deal() -> Deal {
    let deck = Hand::FULL;
    let cards: Vec<_> = deck.cards().collect();
    let hands = [
        Hand::from_bits(
            cards[0..13]
                .iter()
                .fold(0u64, |acc, c| acc | (1u64 << c.index())),
        ),
        Hand::from_bits(
            cards[13..26]
                .iter()
                .fold(0u64, |acc, c| acc | (1u64 << c.index())),
        ),
        Hand::from_bits(
            cards[26..39]
                .iter()
                .fold(0u64, |acc, c| acc | (1u64 << c.index())),
        ),
        Hand::from_bits(
            cards[39..52]
                .iter()
                .fold(0u64, |acc, c| acc | (1u64 << c.index())),
        ),
    ]
    .map(|h| h.unwrap());
    Deal::new(hands).unwrap()
}

fn bench_sequence_log_likelihood(c: &mut Criterion) {
    let system = Arc::new(bench_system());
    let natural = Arc::new(bridge_system::NaturalInference::default());
    let table = Table::uniform(system, natural);
    let auction = bench_auction();
    let deal = bench_deal();
    let ctx = BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Never,
        policy: PolicyParams::default(),
    };

    c.bench_function("sequence_log_likelihood/12-call-auction", |b| {
        b.iter(|| std::hint::black_box(sequence_log_likelihood(&table, &deal, &auction, &ctx)))
    });
}

/// Same as `bench_interpret`, but with `bench_system_realistic` (07-bidding.md §4.4.2's cross
/// product now has real per-suit summaries to intersect, not the all-shapes case).
fn bench_interpret_realistic(c: &mut Criterion) {
    let system = Arc::new(bench_system_realistic());
    let natural = Arc::new(bridge_system::NaturalInference::default());
    let table = Table::uniform(system, natural);
    let auction = bench_auction();
    let opts = InterpretOptions::default();

    c.bench_function("interpret/12-call-auction-realistic", |b| {
        b.iter(|| std::hint::black_box(interpret(&table, &auction, &opts)))
    });
}

/// Same as `bench_sequence_log_likelihood`, but with `bench_system_realistic`.
fn bench_sequence_log_likelihood_realistic(c: &mut Criterion) {
    let system = Arc::new(bench_system_realistic());
    let natural = Arc::new(bridge_system::NaturalInference::default());
    let table = Table::uniform(system, natural);
    let auction = bench_auction();
    let deal = bench_deal();
    let ctx = BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Never,
        policy: PolicyParams::default(),
    };

    c.bench_function("sequence_log_likelihood/12-call-auction-realistic", |b| {
        b.iter(|| std::hint::black_box(sequence_log_likelihood(&table, &deal, &auction, &ctx)))
    });
}

/// The `systems` directory (`<crate>/../../systems`, or `BRIDGE_SYSTEMS_DIR` if set), matching
/// `bridge-system`'s and `bridge-bidding`'s own integration tests (`tests/common/mod.rs`'s
/// `systems_dir`).
fn systems_dir() -> std::path::PathBuf {
    match std::env::var_os("BRIDGE_SYSTEMS_DIR") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../systems"),
    }
}

/// Compiles the real SAYC system (`systems/sayc/sayc.bml`, root file, pulling in every other
/// `systems/sayc/*.bml` via `#INCLUDE`) once, for the SAYC benches below: a compiled system with
/// real per-node suit-length and shape constraints (unlike `bench_system_realistic`'s
/// hand-written "own suit, `>=4`" nodes), the actual case 07-bidding.md §4.4.2's budget is written
/// for. `coverage_samples: 0` skips the coverage lints (irrelevant to timing `interpret`, and
/// otherwise the slowest part of compiling); errors would still show up as `Lint`s of
/// `Severity::Error`, asserted empty so a broken bench never silently benches a half-compiled
/// system.
fn compile_sayc() -> SystemIR {
    let path = systems_dir().join("sayc").join("sayc.bml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let opts = bridge_system::CompileOptions {
        coverage_samples: 0,
        ..bridge_system::CompileOptions::default()
    };
    let (ir, lints) = bridge_system::compile(
        &path.to_string_lossy(),
        &text,
        &bridge_system::lexer::FsLoader,
        &opts,
    );
    let errors: Vec<_> = lints
        .iter()
        .filter(|l| l.severity == bridge_system::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "systems/sayc/sayc.bml: {errors:?}");
    ir
}

/// `1NT-P-2C-P-2H-P-3NT-P-P-P`: opening 1NT, Stayman, a heart fit found and raised, natural game
/// close-out (module doc's non-competitive SAYC example).
fn bench_auction_sayc_1nt() -> Auction {
    Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        vec![
            bid(1, Strain::NoTrump),
            Call::Pass,
            bid(2, Strain::Clubs),
            Call::Pass,
            bid(2, Strain::Hearts),
            Call::Pass,
            bid(3, Strain::NoTrump),
            Call::Pass,
            Call::Pass,
            Call::Pass,
        ],
    )
    .unwrap()
}

/// `1S-(2H)-X-(P)-3S-(P)-P-P`: opening 1 spade, a heart overcall, a negative double, and a
/// competitive raise to game (module doc's competitive SAYC example) -- exercises Step A's
/// `Natural` fallback (the double and everything after it, per the probe behind this bench, are
/// resolved by `bridge_system::natural::classify`/`infer`, not the compiled trie) alongside Step
/// B's cross product over a real system.
fn bench_auction_sayc_competitive() -> Auction {
    Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        vec![
            bid(1, Strain::Spades),
            bid(2, Strain::Hearts),
            Call::Double,
            Call::Pass,
            bid(3, Strain::Spades),
            Call::Pass,
            Call::Pass,
            Call::Pass,
        ],
    )
    .unwrap()
}

fn bench_interpret_sayc_1nt(c: &mut Criterion) {
    let system = Arc::new(compile_sayc());
    let natural = Arc::new(bridge_system::NaturalInference::default());
    let table = Table::uniform(system, natural);
    let auction = bench_auction_sayc_1nt();
    let opts = InterpretOptions::default();

    c.bench_function("interpret/sayc-1nt-auction", |b| {
        b.iter(|| std::hint::black_box(interpret(&table, &auction, &opts)))
    });
}

fn bench_interpret_sayc_competitive(c: &mut Criterion) {
    let system = Arc::new(compile_sayc());
    let natural = Arc::new(bridge_system::NaturalInference::default());
    let table = Table::uniform(system, natural);
    let auction = bench_auction_sayc_competitive();
    let opts = InterpretOptions::default();

    c.bench_function("interpret/sayc-competitive-auction", |b| {
        b.iter(|| std::hint::black_box(interpret(&table, &auction, &opts)))
    });
}

/// `1C-P-1H-P-1S-P-2NT-P-3NT-P-P-P`: a full 12-call, non-competitive auction against the real
/// compiled SAYC system (opening 1C, two one-over-one responses, an opener's rebid, and a close
/// to game) -- unlike `bench_interpret_sayc_1nt`/`_competitive` (10 and 8 calls respectively),
/// this is the actual auction length 11-testing.md §9 and 07-bidding.md §4.6 budget
/// (`interpret < 10 µs`) is written against.
fn bench_auction_sayc_12_call() -> Auction {
    Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        vec![
            bid(1, Strain::Clubs),
            Call::Pass,
            bid(1, Strain::Hearts),
            Call::Pass,
            bid(1, Strain::Spades),
            Call::Pass,
            bid(2, Strain::NoTrump),
            Call::Pass,
            bid(3, Strain::NoTrump),
            Call::Pass,
            Call::Pass,
            Call::Pass,
        ],
    )
    .unwrap()
}

fn bench_interpret_sayc_12_call(c: &mut Criterion) {
    let system = Arc::new(compile_sayc());
    let natural = Arc::new(bridge_system::NaturalInference::default());
    let table = Table::uniform(system, natural);
    let auction = bench_auction_sayc_12_call();
    let opts = InterpretOptions::default();

    c.bench_function("interpret/sayc-12-call-auction", |b| {
        b.iter(|| std::hint::black_box(interpret(&table, &auction, &opts)))
    });
}

/// A natural-heavy competitive auction from the corpus (D20 corpus enumeration of
/// `tests/common::corpus_auctions_with_deals`, index 615; dealer West, NS vulnerable):
/// `1H-(1S)-2C-(2D)-X-(P)-2H-(P)-3H-(P)-P-(P)`, 12 calls of which 9 are read naturally under
/// SAYC (2 of them shadowed; the count is recomputed and printed when the bench starts).
fn bench_auction_natural_heavy() -> Auction {
    Auction::from_calls(
        Seat::West,
        Vulnerability::NS,
        vec![
            bid(1, Strain::Hearts),
            bid(1, Strain::Spades),
            bid(2, Strain::Clubs),
            bid(2, Strain::Diamonds),
            Call::Double,
            Call::Pass,
            bid(2, Strain::Hearts),
            Call::Pass,
            bid(3, Strain::Hearts),
            Call::Pass,
            Call::Pass,
            Call::Pass,
        ],
    )
    .unwrap()
}

/// `P-P-1NT-P-2C-P-2S-P-4S-P-P-P` (dealer North, none vulnerable): a 12-call SAYC auction that
/// `replay` bids with the system-players policy (Stayman, a 2S answer, a raise to game), in which
/// every call is one the policy makes (none shadowed; the four closing passes are read
/// naturally). `interpret/sayc-12-call-auction` is the acceptance bench, but its 3NT is
/// off-policy: SAYC has no continuation after `1C-1H-1S-2NT` and the natural rules have no 3NT
/// candidate there, so it is shadowed and read by its `Fallback` pieces only.
fn bench_auction_sayc_12_on_policy() -> Auction {
    Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        vec![
            Call::Pass,
            Call::Pass,
            bid(1, Strain::NoTrump),
            Call::Pass,
            bid(2, Strain::Clubs),
            Call::Pass,
            bid(2, Strain::Spades),
            Call::Pass,
            bid(4, Strain::Spades),
            Call::Pass,
            Call::Pass,
            Call::Pass,
        ],
    )
    .unwrap()
}

fn sayc_table() -> Table {
    let system = Arc::new(compile_sayc());
    let natural = Arc::new(bridge_system::NaturalInference::default());
    Table::uniform(system, natural)
}

fn bench_interpret_step_a(c: &mut Criterion) {
    let table = sayc_table();
    let opts = InterpretOptions::default();
    for (name, auction) in [
        ("sayc-12-call-auction", bench_auction_sayc_12_call()),
        ("sayc-1nt-auction", bench_auction_sayc_1nt()),
        ("sayc-competitive-auction", bench_auction_sayc_competitive()),
    ] {
        c.bench_function(&format!("interpret-step-a/{name}"), |b| {
            b.iter(|| std::hint::black_box(interpret_per_call(&table, &auction, &opts)))
        });
    }
    let synthetic = Table::uniform(
        Arc::new(bench_system()),
        Arc::new(bridge_system::NaturalInference::default()),
    );
    let auction = bench_auction();
    c.bench_function("interpret-step-a/12-call-auction", |b| {
        b.iter(|| std::hint::black_box(interpret_per_call(&synthetic, &auction, &opts)))
    });
}

fn bench_interpret_sayc_12_on_policy(c: &mut Criterion) {
    let table = sayc_table();
    let opts = InterpretOptions::default();
    let auction = bench_auction_sayc_12_on_policy();
    let shadowed = interpret(&table, &auction, &opts)
        .per_call
        .iter()
        .filter(|pc| pc.shadowed)
        .count();
    eprintln!("sayc-12-call-on-policy: {shadowed} shadowed calls");
    c.bench_function("interpret/sayc-12-call-on-policy", |b| {
        b.iter(|| std::hint::black_box(interpret(&table, &auction, &opts)))
    });
}

fn bench_interpret_natural_heavy(c: &mut Criterion) {
    let table = sayc_table();
    let opts = InterpretOptions::default();
    let auction = bench_auction_natural_heavy();
    let natural_calls = interpret(&table, &auction, &opts)
        .per_call
        .iter()
        .filter(|pc| pc.kind == bridge_bidding::ResolutionKind::Natural)
        .count();
    c.bench_function("interpret/natural-heavy-auction", |b| {
        b.iter(|| std::hint::black_box(interpret(&table, &auction, &opts)))
    });
    // A table with the same systems and a fresh natural-engine allocation: nothing of it is
    // memoised yet (the memo is keyed by the table's allocations).
    let fresh = || Table {
        systems: table.systems.clone(),
        natural: Arc::new((*table.natural).clone()),
    };
    eprintln!("natural-heavy-auction: {natural_calls} natural calls");
    c.bench_function("interpret-cold/natural-heavy-auction", |b| {
        b.iter_batched_ref(
            fresh,
            |t| std::hint::black_box(interpret(t, &auction, &opts)),
            BatchSize::SmallInput,
        )
    });
    let sayc_12 = bench_auction_sayc_12_call();
    c.bench_function("interpret-cold/sayc-12-call-auction", |b| {
        b.iter_batched_ref(
            fresh,
            |t| std::hint::black_box(interpret(t, &sayc_12, &opts)),
            BatchSize::SmallInput,
        )
    });
}

fn human_ctx(table: &Table) -> BidContext<'_> {
    BidContext {
        scoring: Scoring::Imp,
        natural: Some(table.natural.as_ref()),
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::human(),
    }
}

fn bench_interpret_human(c: &mut Criterion) {
    let table = sayc_table();
    let ctx = human_ctx(&table);
    let opts = InterpretOptions::for_context(&ctx);
    let auction = bench_auction_sayc_12_call();
    c.bench_function("interpret-human/sayc-12-call-auction", |b| {
        b.iter(|| std::hint::black_box(interpret(&table, &auction, &opts)))
    });
}

fn bench_auction_policy(c: &mut Criterion) {
    let table = sayc_table();
    let auction = bench_auction_sayc_12_call();
    let deal = bench_deal();
    for (name, policy) in [
        ("system-players", PolicyParams::system_players()),
        ("human", PolicyParams::human()),
    ] {
        let ctx = BidContext {
            policy,
            ..human_ctx(&table)
        };
        let ap = AuctionPolicy::new(&table, &auction, &ctx);
        c.bench_function(
            &format!("auction-policy/log-likelihood/sayc-12-call-auction/{name}"),
            |b| b.iter(|| std::hint::black_box(ap.log_likelihood(&deal))),
        );
        c.bench_function(
            &format!("auction-policy/new/sayc-12-call-auction/{name}"),
            |b| b.iter(|| std::hint::black_box(AuctionPolicy::new(&table, &auction, &ctx))),
        );
        c.bench_function(
            &format!("sequence_log_likelihood/sayc-12-call-auction/{name}"),
            |b| {
                b.iter(|| {
                    std::hint::black_box(sequence_log_likelihood(&table, &deal, &auction, &ctx))
                })
            },
        );
    }
}

criterion_group!(
    benches,
    bench_interpret,
    bench_sequence_log_likelihood,
    bench_interpret_realistic,
    bench_sequence_log_likelihood_realistic,
    bench_interpret_sayc_1nt,
    bench_interpret_sayc_competitive,
    bench_interpret_sayc_12_call,
    bench_interpret_sayc_12_on_policy,
    bench_interpret_step_a,
    bench_interpret_natural_heavy,
    bench_interpret_human,
    bench_auction_policy
);
criterion_main!(benches);
