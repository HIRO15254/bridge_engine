//! Criterion benches for deal sampling (target: ≥ 10^4 constrained deals/s/core).

use criterion::{Criterion, criterion_group, criterion_main};

fn bench_placeholder(c: &mut Criterion) {
    // Phase 5 replaces this with `sample_deals` on a corpus auction.
    c.bench_function("deals/placeholder", |b| {
        b.iter(|| std::hint::black_box(0u64))
    });
}

criterion_group!(benches, bench_placeholder);
criterion_main!(benches);
