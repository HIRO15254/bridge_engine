//! Criterion benches for deal sampling.
//!
//! The design target (§6.4 / §9 of `09-sample.md`) is ≥ 10^4 constrained deals/s/core. This
//! bench measures the `UniformProposal` baseline and, now that phase 5 has landed,
//! `ConstraintProposal` on two constrained contexts, both single-threaded and (with the
//! `parallel` feature) on rayon's default pool, so the two proposals can be compared directly.

use bridge_bidding::{
    CallExplanation, CallInterpretation, Explanation, Interpretation, ResolutionKind,
};
use bridge_constraint::{Atom, HandConstraint, KnownCards, ShapeSet};
use bridge_core::{Call, Seat, Suit};
use bridge_sample::{
    ConstraintProposal, Proposal, SampleContext, SampleOptions, Threads, UniformProposal,
    sample_deals,
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

/// Deals per second for `threads`, sampling `batch` deals per benchmarked iteration.
fn bench_threads<'a>(
    group: &mut criterion::BenchmarkGroup<'a, criterion::measurement::WallTime>,
    name: &str,
    threads: Threads,
    proposal: &dyn Proposal,
    interpretation: &Interpretation,
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
        bidding: None,
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
    );
    #[cfg(feature = "parallel")]
    bench_threads(
        &mut group,
        "auto",
        Threads::Auto,
        &UniformProposal,
        &interpretation,
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
    );
    #[cfg(feature = "parallel")]
    bench_threads(&mut group, "auto", Threads::Auto, &proposal, &one_nt);
    group.finish();

    let four_call = four_call_three_seats_interpretation();
    let mut group = c.benchmark_group("deals/constraint/four_call_three_seats");
    bench_threads(
        &mut group,
        "single_thread",
        Threads::Single,
        &proposal,
        &four_call,
    );
    #[cfg(feature = "parallel")]
    bench_threads(&mut group, "auto", Threads::Auto, &proposal, &four_call);
    group.finish();
}

criterion_group!(benches, bench_uniform_proposal, bench_constraint_proposal);
criterion_main!(benches);
