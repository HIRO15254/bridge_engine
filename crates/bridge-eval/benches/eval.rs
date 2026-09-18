//! Criterion benches for the evaluation functions (target: `hcp` under 10 ns).

use criterion::{Criterion, criterion_group, criterion_main};

fn bench_hcp(c: &mut Criterion) {
    let hand = bridge_core::Hand::FULL; // placeholder input until phase 2 adds real hands
    c.bench_function("hcp", |b| {
        b.iter(|| bridge_eval::hcp(std::hint::black_box(hand)))
    });
}

criterion_group!(benches, bench_hcp);
criterion_main!(benches);
