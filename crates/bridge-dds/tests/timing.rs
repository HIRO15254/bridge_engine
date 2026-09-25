//! Wall-clock timings quoted in docs/design/10-dds.md / 12-roadmap.md (phase 5): the mean
//! `calc_dd_table` time per deal, the mean `calc_dd_tables` time per deal (batched), and the
//! mean `solve_board` time per call (opening lead, all cards ranked), each over the same
//! pseudo-random deals. `#[ignore]`d: only meaningful in release, on an otherwise idle machine:
//!
//! ```text
//! cargo test --release -p bridge-dds --test timing -- --ignored --nocapture
//! ```

#![cfg(dds_vendored)]

use std::time::Instant;

use bridge_core::{Card, Deal, Hand, Seat, Strain};
use bridge_dds::{Mode, Position, Solutions, Target, calc_dd_table, calc_dd_tables, solve_board};

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
#[ignore = "timing only; run in release with --ignored --nocapture"]
fn timing() {
    let deals: Vec<Deal> = (1000..1100u64).map(shuffled).collect();
    // Warm-up: runtime initialisation and first-touch of DDS's per-thread memory.
    for d in &deals[..3] {
        calc_dd_table(d).expect("valid deal");
    }

    let start = Instant::now();
    for d in &deals {
        calc_dd_table(d).expect("valid deal");
    }
    let per_deal = start.elapsed() / deals.len() as u32;
    eprintln!(
        "calc_dd_table: {per_deal:?}/deal over {} deals",
        deals.len()
    );

    let start = Instant::now();
    calc_dd_tables(&deals).expect("valid deals");
    let per_deal = start.elapsed() / deals.len() as u32;
    eprintln!(
        "calc_dd_tables: {per_deal:?}/deal over {} deals",
        deals.len()
    );

    let start = Instant::now();
    for (i, d) in deals.iter().enumerate() {
        let pos = Position {
            deal: d,
            trump: Strain::from_index((i % 5) as u8),
            leader: Seat::from_index((i % 4) as u8),
            trick: &[],
        };
        solve_board(&pos, Target::Max, Solutions::AllRanked, Mode::Auto).expect("valid position");
    }
    let per_call = start.elapsed() / deals.len() as u32;
    eprintln!("solve_board: {per_call:?}/call over {} calls", deals.len());
}
