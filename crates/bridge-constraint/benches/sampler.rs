//! Criterion benches for the constraint sampler (target: >= 10^5 hands/s/core from an `Atom`).
//!
//! Two scenarios (2.4-2.6):
//! - `full_deck_15_17_balanced`: the whole 52-card pool, no fixed cards, a shape + HCP window
//!   (exact path with a shared `FULL_SUIT` table for every suit).
//! - `mid_play_26_pool_6_fixed`: a mid-play position with 6 already-known cards and 26 still
//!   unknown, plus an HCP window (exact path, per-suit tables rebuilt for the smaller pool).
//!
//! Each scenario benches `Sampler::prepare` and, separately, `Sampler::sample` throughput from an
//! already-prepared sampler (so the reported hands/s excludes the one-time `prepare` cost, matching
//! how a caller would reuse a prepared sampler across many draws).

use bridge_constraint::{Atom, HandConstraint, SampleOptions, Sampler, ShapeSet};
use bridge_core::{Card, Hand};
use criterion::{Criterion, criterion_group, criterion_main};
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;

/// 15-17 HCP, balanced, on the full 52-card pool.
fn full_deck_15_17_balanced() -> HandConstraint {
    let atom = Atom {
        shapes: ShapeSet::BALANCED,
        ..Atom::ANY.with_hcp(15..=17)
    };
    HandConstraint::Atom(atom)
}

/// A mid-play position: 6 cards already known (fixed), 26 cards still unknown (the pool split
/// between two unseen hands), an HCP window on the completed 13-card hand.
fn mid_play_pool_and_fixed() -> (Hand, Hand) {
    let mut fixed = Hand::EMPTY;
    for i in 0..6u8 {
        fixed = fixed.with(Card::from_index(i).expect("index < 52"));
    }
    let mut pool = Hand::EMPTY;
    for i in 6..32u8 {
        pool = pool.with(Card::from_index(i).expect("index < 52"));
    }
    (pool, fixed)
}

fn mid_play_10_16_hcp() -> HandConstraint {
    HandConstraint::Atom(Atom::ANY.with_hcp(10..=16))
}

fn bench_prepare(c: &mut Criterion) {
    let mut group = c.benchmark_group("sampler/prepare");

    let full = full_deck_15_17_balanced();
    group.bench_function("full_deck_15_17_balanced", |b| {
        b.iter(|| {
            let sampler = Sampler::prepare(
                std::hint::black_box(&full),
                Hand::FULL,
                Hand::EMPTY,
                &SampleOptions::default(),
            )
            .unwrap();
            std::hint::black_box(sampler.count())
        });
    });

    let (pool, fixed) = mid_play_pool_and_fixed();
    let mid = mid_play_10_16_hcp();
    group.bench_function("mid_play_26_pool_6_fixed", |b| {
        b.iter(|| {
            let sampler = Sampler::prepare(
                std::hint::black_box(&mid),
                pool,
                fixed,
                &SampleOptions::default(),
            )
            .unwrap();
            std::hint::black_box(sampler.count())
        });
    });

    group.finish();
}

fn bench_sample(c: &mut Criterion) {
    let mut group = c.benchmark_group("sampler/sample");

    let full = full_deck_15_17_balanced();
    let full_sampler = Sampler::prepare(&full, Hand::FULL, Hand::EMPTY, &SampleOptions::default())
        .expect("Hand::FULL/Hand::EMPTY never overlap");
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x5A5A_5A5A);
    group.bench_function("full_deck_15_17_balanced", |b| {
        b.iter(|| std::hint::black_box(full_sampler.sample(&mut rng)));
    });

    let (pool, fixed) = mid_play_pool_and_fixed();
    let mid = mid_play_10_16_hcp();
    let mid_sampler = Sampler::prepare(&mid, pool, fixed, &SampleOptions::default())
        .expect("pool/fixed are disjoint by construction");
    let mut rng2 = Xoshiro256PlusPlus::seed_from_u64(0x5A5A_5A5B);
    group.bench_function("mid_play_26_pool_6_fixed", |b| {
        b.iter(|| std::hint::black_box(mid_sampler.sample(&mut rng2)));
    });

    group.finish();
}

criterion_group!(benches, bench_prepare, bench_sample);
criterion_main!(benches);
