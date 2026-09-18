//! Criterion benches for the constraint sampler (target: ≥ 10^5 hands/s/core from an `Atom`).

use criterion::{Criterion, criterion_group, criterion_main};

fn bench_placeholder(c: &mut Criterion) {
    // Phase 2 replaces this with `Sampler::prepare(15-17 balanced).sample(rng)`.
    c.bench_function("sampler/placeholder", |b| {
        b.iter(|| std::hint::black_box(0u64))
    });
}

criterion_group!(benches, bench_placeholder);
criterion_main!(benches);
