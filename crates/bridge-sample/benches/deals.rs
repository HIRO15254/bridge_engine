//! Criterion benches for deal sampling.
//!
//! The design target (§6.4 / §9 of `09-sample.md`) is ≥ 10^4 constrained deals/s/core with
//! `ConstraintProposal`, which is phase 5 and not implemented yet (`todo!()` in
//! `constraint_proposal.rs`). This bench instead measures the `UniformProposal` baseline that
//! *is* implemented in this phase, both single-threaded and (with the `parallel` feature) on
//! rayon's default pool, so the two can be compared once `ConstraintProposal` lands.

use bridge_bidding::Interpretation;
use bridge_constraint::{HandConstraint, KnownCards};
use bridge_sample::{SampleContext, SampleOptions, Threads, UniformProposal, sample_deals};
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

/// Deals per second for `threads`, sampling `batch` deals per benchmarked iteration.
fn bench_threads(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &str,
    threads: Threads,
) {
    let interpretation = unconstrained_interpretation();
    let play_constraints = [
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
    ];
    let ctx = SampleContext {
        known: KnownCards::EMPTY,
        interpretation: &interpretation,
        play_constraints: &play_constraints,
        play_soft: None,
        bidding: None,
    };
    let batch = 1_000usize;
    let opts = SampleOptions {
        seed: 42,
        max_attempts_per_sample: 16,
        max_attempt_factor: 50,
        threads,
    };

    group.throughput(Throughput::Elements(batch as u64));
    group.bench_function(name, |b| {
        b.iter(|| {
            let (deals, _) = sample_deals(&ctx, &UniformProposal, batch, &opts)
                .expect("full-deck uniform sampling never fails");
            std::hint::black_box(deals)
        })
    });
}

fn bench_uniform_proposal(c: &mut Criterion) {
    let mut group = c.benchmark_group("deals/uniform");
    bench_threads(&mut group, "single_thread", Threads::Single);
    #[cfg(feature = "parallel")]
    bench_threads(&mut group, "auto", Threads::Auto);
    group.finish();
}

criterion_group!(benches, bench_uniform_proposal);
criterion_main!(benches);
