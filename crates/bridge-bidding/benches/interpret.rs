//! Criterion benches for `interpret` (target: under 10 µs for a 12-call auction).

use criterion::{Criterion, criterion_group, criterion_main};

fn bench_placeholder(c: &mut Criterion) {
    // Phase 3 replaces this with `interpret(&table, &auction, &opts)` on a compiled SAYC.
    c.bench_function("interpret/placeholder", |b| {
        b.iter(|| std::hint::black_box(0u64))
    });
}

criterion_group!(benches, bench_placeholder);
criterion_main!(benches);
