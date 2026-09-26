//! `Mode::ReuseTable` must never reuse a transposition table built for a different deal or
//! trump. DDS's own `mode == 2` skips the transposition-table reset unconditionally, which is
//! only correct when the previous call *on the same thread index* was the same deal and trump;
//! the wrapper hands out thread-index slots arbitrarily, so the caller cannot guarantee that.
//!
//! Its own test binary: it pins DDS to a single thread (so every call lands on slot 0 and the
//! stale table is guaranteed to be there), and `init` is process-global.

#![cfg(dds_vendored)]

use bridge_core::{Card, Deal, Hand, Seat, Strain};
use bridge_dds::{DdsConfig, Mode, Position, Solutions, Target, init, solve_board};

/// A deterministic pseudo-random deal (Fisher-Yates driven by a 64-bit LCG).
fn shuffled(seed: u64) -> Deal {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut order: Vec<u8> = (0..52).collect();
    for i in (1..order.len()).rev() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let j = ((state >> 33) % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }
    let mut hands = [Hand::EMPTY; 4];
    for (pos, &idx) in order.iter().enumerate() {
        let card = Card::from_index(idx).expect("idx < 52");
        hands[pos / 13] = hands[pos / 13].with(card);
    }
    Deal::new(hands).expect("four 13-card hands")
}

#[test]
fn reuse_table_after_an_unrelated_deal_matches_a_fresh_search() {
    init(DdsConfig {
        max_threads: 1,
        max_memory_mb: 0,
    })
    .expect("DDS vendored");

    let deals: Vec<Deal> = (0..12u64).map(shuffled).collect();
    let strains = [Strain::NoTrump, Strain::Spades, Strain::Hearts];
    let mut mismatches = Vec::new();
    for (i, d) in deals.iter().enumerate() {
        for (k, &trump) in strains.iter().enumerate() {
            let leader = Seat::from_index(((i + k) % 4) as u8);
            let pos = Position {
                deal: d,
                trump,
                leader,
                trick: &[],
            };
            let reused = solve_board(&pos, Target::Max, Solutions::AllRanked, Mode::ReuseTable)
                .expect("valid position");
            let fresh = solve_board(&pos, Target::Max, Solutions::AllRanked, Mode::Search)
                .expect("valid position");
            if reused.cards != fresh.cards {
                mismatches.push((i, trump, reused.cards, fresh.cards));
            }
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}
