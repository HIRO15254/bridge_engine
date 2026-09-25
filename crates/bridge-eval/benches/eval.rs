//! Criterion benches for the evaluation functions (target: `hcp` under 10 ns).

use bridge_core::{Hand, Holding};
use bridge_eval::DistMethod;
use criterion::{Criterion, criterion_group, criterion_main};

/// A representative 13-card hand: 5=4=3=1 with a mix of honours, spot cards and voids-adjacent
/// suits, so every metric under bench has non-trivial work to do.
fn sample_hand() -> Hand {
    // Spades: A K Q x x (5), Hearts: K Q x x (4), Diamonds: A x x (3), Clubs: x (1).
    let spades = Holding::top_ranks(3)
        .with(bridge_core::Rank::Four)
        .with(bridge_core::Rank::Three);
    let hearts = Holding::top_ranks(0)
        .with(bridge_core::Rank::King)
        .with(bridge_core::Rank::Queen)
        .with(bridge_core::Rank::Six)
        .with(bridge_core::Rank::Five);
    let diamonds = Holding::top_ranks(0)
        .with(bridge_core::Rank::Ace)
        .with(bridge_core::Rank::Seven)
        .with(bridge_core::Rank::Six);
    let clubs = Holding::top_ranks(0).with(bridge_core::Rank::Two);
    Hand::from_holdings(clubs, diamonds, hearts, spades)
}

fn bench_hcp(c: &mut Criterion) {
    let hand = sample_hand();
    c.bench_function("hcp", |b| {
        b.iter(|| bridge_eval::hcp(std::hint::black_box(hand)))
    });
}

fn bench_losers(c: &mut Criterion) {
    let hand = sample_hand();
    c.bench_function("losers", |b| {
        b.iter(|| bridge_eval::losers(std::hint::black_box(hand)))
    });
}

fn bench_quick_tricks(c: &mut Criterion) {
    let hand = sample_hand();
    c.bench_function("quick_tricks", |b| {
        b.iter(|| bridge_eval::quick_tricks(std::hint::black_box(hand)))
    });
}

fn bench_distribution_points(c: &mut Criterion) {
    let hand = sample_hand();
    c.bench_function("distribution_points", |b| {
        b.iter(|| {
            bridge_eval::distribution_points(std::hint::black_box(hand), DistMethod::GOREN_321)
        })
    });
}

criterion_group!(
    benches,
    bench_hcp,
    bench_losers,
    bench_quick_tricks,
    bench_distribution_points
);
criterion_main!(benches);
