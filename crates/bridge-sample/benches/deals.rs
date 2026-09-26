//! Criterion benches for deal sampling.
//!
//! The design target (§6.4 / §9 of `09-sample.md`) is ≥ 10^4 constrained deals/s/core. This
//! bench measures the `UniformProposal` baseline and, now that phase 5 has landed,
//! `ConstraintProposal` on two constrained contexts, both single-threaded and (with the
//! `parallel` feature) on rayon's default pool, so the two proposals can be compared directly.
//!
//! Phase 5.4 additionally benches three real SAYC auctions (`systems/sayc/sayc.bml`, compiled and
//! interpreted, not the hand-built synthetic `Interpretation`s above), with bidding likelihood on
//! (`SampleContext::bidding`): a Stayman sequence to game, a competitive raise to game, and a
//! four-seat competitive auction where every seat has made at least one call. These are the
//! target's own "real auction" cases, closer to how the library is actually used than the
//! synthetic ones. They are interpreted as the mirror of `PolicyParams::system_players()`
//! (`InterpretOptions::for_context`), and weighted with that policy's `AuctionPolicy`
//! likelihood; each is also benched with residual rejection (`single_thread_residual`).

use std::sync::Arc;

use bridge_bidding::{
    BidContext, CallExplanation, CallInterpretation, Explanation, ImplicitPass, InterpretOptions,
    Interpretation, PolicyParams, ResolutionKind, Scoring, Table, interpret,
};
use bridge_constraint::{Atom, HandConstraint, KnownCards, ShapeSet};
use bridge_core::{Auction, Bid, Call, Seat, Strain, Suit, Vulnerability};
use bridge_sample::{
    BiddingLikelihood, ConstraintProposal, Proposal, SampleContext, SampleOptions, Threads,
    UniformProposal, sample_deals,
};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};

/// No calls for any seat: `Interpretation::likelihood` is vacuously 1 everywhere, so every
/// proposed deal gets a finite weight and the bench measures proposal throughput rather than
/// acceptance rate.
fn unconstrained_interpretation() -> Interpretation {
    Interpretation {
        seats: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        per_call: Vec::new(),
        divergence: None,
    }
}

/// One seat's ε-mixture alternatives (07-bidding.md §4.2, D15): `[(primary, 1 - eps), (ANY,
/// eps)]`, in both `seats` (`ConstraintProposal`) and `per_call` (`Interpretation::likelihood`).
fn add_seat_call(
    interpretation: &mut Interpretation,
    seat: Seat,
    primary: HandConstraint,
    eps: f32,
) {
    let weighted: Vec<(HandConstraint, f32)> =
        vec![(primary, 1.0 - eps), (HandConstraint::ANY, eps)];
    let idx = seat.index() as usize;
    interpretation.seats[idx] = weighted
        .iter()
        .map(|(c, w)| {
            (
                c.clone(),
                *w,
                Explanation {
                    text: String::new(),
                    node: None,
                    resolution: ResolutionKind::Exact,
                    parts: Vec::new(),
                },
            )
        })
        .collect();
    let call_index = interpretation.per_call.len();
    let alternatives = weighted
        .into_iter()
        .map(|(c, w)| {
            (
                c,
                w,
                CallExplanation {
                    call_index,
                    call: Call::Pass,
                    node: None,
                    kind: ResolutionKind::Exact,
                    text: String::new(),
                },
            )
        })
        .collect();
    interpretation.per_call.push(CallInterpretation {
        call_index,
        seat,
        call: Call::Pass,
        kind: ResolutionKind::Exact,
        alternatives,
        log_scale: 0.0,
        shadowed: false,
    });
}

fn balanced(hcp: core::ops::RangeInclusive<u8>) -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: ShapeSet::BALANCED,
        hcp,
        cards: Vec::new(),
        eval: Vec::new(),
    })
}

fn suit_len(suit: Suit, lo: u8, hi: u8, hcp: core::ops::RangeInclusive<u8>) -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: ShapeSet::from_suit_len(suit, lo, hi),
        hcp,
        cards: Vec::new(),
        eval: Vec::new(),
    })
}

/// North opens 1NT (15-17 balanced); every other seat is unconstrained.
fn one_nt_opener_interpretation() -> Interpretation {
    let mut interpretation = unconstrained_interpretation();
    add_seat_call(&mut interpretation, Seat::North, balanced(15..=17), 0.02);
    interpretation
}

/// A four-call auction (1S - 2H (overcall) - 3S (limit raise) - Pass) with constraints on three
/// seats: North's opening shape and range, East's overcall, South's limit raise; West (the
/// passer) stays unconstrained.
fn four_call_three_seats_interpretation() -> Interpretation {
    let mut interpretation = unconstrained_interpretation();
    add_seat_call(
        &mut interpretation,
        Seat::North,
        suit_len(Suit::Spades, 5, 13, 11..=21),
        0.02,
    );
    add_seat_call(
        &mut interpretation,
        Seat::East,
        suit_len(Suit::Hearts, 5, 13, 8..=16),
        0.15,
    );
    add_seat_call(
        &mut interpretation,
        Seat::South,
        suit_len(Suit::Spades, 4, 13, 10..=12),
        0.15,
    );
    interpretation
}

/// Deals per second for `threads`, sampling `batch` deals per benchmarked iteration. `bidding`
/// is `None` for the synthetic cases above (`Interpretation::likelihood` is used instead) and
/// `Some` for the real-SAYC cases (§10.1), which weight by the auction's actual bidding
/// likelihood.
fn bench_threads<'a>(
    group: &mut criterion::BenchmarkGroup<'a, criterion::measurement::WallTime>,
    name: &str,
    threads: Threads,
    proposal: &dyn Proposal,
    interpretation: &Interpretation,
    bidding: Option<BiddingLikelihood<'_>>,
) {
    let play_constraints = [
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
    ];
    let ctx = SampleContext {
        known: KnownCards::EMPTY,
        interpretation,
        play_constraints: &play_constraints,
        play_soft: None,
        bidding,
    };
    let batch = 1_000usize;
    let opts = SampleOptions {
        seed: 42,
        max_attempts_per_sample: 16,
        max_attempt_factor: 200,
        threads,
    };

    group.throughput(Throughput::Elements(batch as u64));
    group.bench_function(name, |b| {
        b.iter(|| {
            let (deals, _) = sample_deals(&ctx, proposal, batch, &opts)
                .expect("every bench context has support");
            std::hint::black_box(deals)
        })
    });
}

fn bench_uniform_proposal(c: &mut Criterion) {
    let interpretation = unconstrained_interpretation();
    let mut group = c.benchmark_group("deals/uniform");
    bench_threads(
        &mut group,
        "single_thread",
        Threads::Single,
        &UniformProposal,
        &interpretation,
        None,
    );
    #[cfg(feature = "parallel")]
    bench_threads(
        &mut group,
        "auto",
        Threads::Auto,
        &UniformProposal,
        &interpretation,
        None,
    );
    group.finish();
}

fn bench_constraint_proposal(c: &mut Criterion) {
    let proposal = ConstraintProposal::default();

    let one_nt = one_nt_opener_interpretation();
    let mut group = c.benchmark_group("deals/constraint/1nt_opener");
    bench_threads(
        &mut group,
        "single_thread",
        Threads::Single,
        &proposal,
        &one_nt,
        None,
    );
    #[cfg(feature = "parallel")]
    bench_threads(&mut group, "auto", Threads::Auto, &proposal, &one_nt, None);
    group.finish();

    let four_call = four_call_three_seats_interpretation();
    let mut group = c.benchmark_group("deals/constraint/four_call_three_seats");
    bench_threads(
        &mut group,
        "single_thread",
        Threads::Single,
        &proposal,
        &four_call,
        None,
    );
    #[cfg(feature = "parallel")]
    bench_threads(
        &mut group,
        "auto",
        Threads::Auto,
        &proposal,
        &four_call,
        None,
    );
    group.finish();
}

/// `<crate>/../../systems`, matching `bridge-system`'s own test helper
/// (`crates/bridge-system/tests/common.rs`) and `crates/bridge/tests/dds_sample.rs`.
fn systems_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../systems")
}

/// Compiles the checked-in `systems/sayc/sayc.bml` (always present; not vendored corpus data).
fn compile_sayc() -> bridge_system::SystemIR {
    let path = systems_dir().join("sayc").join("sayc.bml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let opts = bridge_system::CompileOptions::default();
    let (ir, lints) = bridge_system::compile(
        &path.to_string_lossy(),
        &text,
        &bridge_system::lexer::FsLoader,
        &opts,
    );
    let errors = lints
        .iter()
        .filter(|l| l.severity == bridge_system::Severity::Error)
        .count();
    assert_eq!(
        errors, 0,
        "sayc.bml compiled with {errors} Error-severity lint(s)"
    );
    ir
}

fn sayc_table() -> Table {
    Table::uniform(
        Arc::new(compile_sayc()),
        Arc::new(bridge_bidding::NaturalInference::default()),
    )
}

fn bid_ctx() -> BidContext<'static> {
    BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::system_players(),
    }
}

fn bid(level: u8, strain: Strain) -> Call {
    Call::Bid(Bid::new(level, strain).expect("1..=7"))
}

/// 1NT - P - 2C (Stayman) - P - 2H (a 4-card major found) - P - 3NT (game) - P - P - P.
fn stayman_to_3nt() -> Auction {
    Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        [
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
    .expect("legal auction")
}

/// 1S - 2H (overcall) - 2S (raise) - P - 4S (competitive raise to game) - P - P - P.
fn competitive_raise_to_4s() -> Auction {
    Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        [
            bid(1, Strain::Spades),
            bid(2, Strain::Hearts),
            bid(2, Strain::Spades),
            Call::Pass,
            bid(4, Strain::Spades),
            Call::Pass,
            Call::Pass,
            Call::Pass,
        ],
    )
    .expect("legal auction")
}

/// A four-seat competitive auction: every seat calls at least once (North opens, East overcalls,
/// South raises, West raises again, North competes to game, then three passes end it).
fn four_seat_competitive() -> Auction {
    Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        [
            bid(1, Strain::Spades),
            bid(2, Strain::Hearts),
            bid(2, Strain::Spades),
            bid(3, Strain::Hearts),
            bid(4, Strain::Spades),
            Call::Pass,
            Call::Pass,
            Call::Pass,
        ],
    )
    .expect("legal auction")
}

/// Interprets `auction` against SAYC and benches `sample_deals` with the auction's real bidding
/// likelihood (`SampleContext::bidding`), single-threaded and (with the `parallel` feature) on
/// rayon's default pool.
fn bench_sayc_auction(c: &mut Criterion, group_name: &str, table: &Table, auction: &Auction) {
    let bctx = bid_ctx();
    // The mirror of the likelihood's own policy (07-bidding.md §4.2).
    let interp = interpret(table, auction, &InterpretOptions::for_context(&bctx));
    let bidding = BiddingLikelihood {
        table,
        auction,
        ctx: &bctx,
    };
    let proposal = ConstraintProposal::default();

    let mut group = c.benchmark_group(group_name);
    bench_threads(
        &mut group,
        "single_thread",
        Threads::Single,
        &proposal,
        &interp,
        Some(bidding),
    );
    // Residual rejection (09-sample.md §6.5): fewer produced deals per attempt, flatter weights.
    let residual = ConstraintProposal {
        residual_rejection: true,
        ..ConstraintProposal::default()
    };
    bench_threads(
        &mut group,
        "single_thread_residual",
        Threads::Single,
        &residual,
        &interp,
        Some(bidding),
    );
    #[cfg(feature = "parallel")]
    bench_threads(
        &mut group,
        "auto",
        Threads::Auto,
        &proposal,
        &interp,
        Some(bidding),
    );
    group.finish();
}

fn bench_sayc_proposal(c: &mut Criterion) {
    let table = sayc_table();
    bench_sayc_auction(c, "deals/sayc/stayman_to_3nt", &table, &stayman_to_3nt());
    bench_sayc_auction(
        c,
        "deals/sayc/competitive_raise_to_4s",
        &table,
        &competitive_raise_to_4s(),
    );
    bench_sayc_auction(
        c,
        "deals/sayc/four_seat_competitive",
        &table,
        &four_seat_competitive(),
    );
}

criterion_group!(
    benches,
    bench_uniform_proposal,
    bench_constraint_proposal,
    bench_sayc_proposal
);
criterion_main!(benches);
