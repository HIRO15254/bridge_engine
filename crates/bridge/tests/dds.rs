//! `bridge::dd::dds()` fallback and basic solving (docs/design/12-roadmap.md 5.7).
//!
//! Gracefully does nothing when the DDS sources were not vendored (`cargo xtask dds vendor`),
//! so `cargo test -p bridge --features dds` still passes without them, matching `bridge_dds`'s
//! own `#[cfg(dds_vendored)]`-gated tests.
#![cfg(all(feature = "dds", not(target_arch = "wasm32")))]

use bridge::dd::dds;
use bridge::{Card, Deal, Hand, Seat, Strain};

fn sample_deal() -> Deal {
    let mut hands = [Hand::EMPTY; 4];
    for i in 0..52u8 {
        let card = Card::from_index(i).expect("i < 52");
        hands[(i % 4) as usize] = hands[(i % 4) as usize].with(card);
    }
    Deal::new(hands).expect("four 13-card hands covering the deck")
}

#[test]
fn dds_backend_solves_a_deal_when_vendored() {
    let Some(backend) = dds() else {
        eprintln!("DDS not vendored in this build (cargo xtask dds vendor); skipping");
        return;
    };

    let deal = sample_deal();
    let table = backend
        .dd_table(&deal)
        .expect("dd_table should succeed on a valid deal");
    // Every cell is a trick count in 0..=13.
    for strain in bridge::Strain::ALL {
        for seat in Seat::ALL {
            assert!(table.tricks(strain, seat) <= 13);
        }
    }

    let scores = backend
        .lead_scores(&deal, Strain::NoTrump, Seat::North)
        .expect("lead_scores should succeed on a valid deal");
    assert_eq!(scores.len(), 13, "one score per card in North's hand");
    for (card, tricks) in &scores {
        assert!(deal.hand(Seat::North).contains(*card));
        assert!(*tricks <= 13);
    }
}

#[test]
fn dds_none_iff_unavailable() {
    assert_eq!(dds().is_some(), bridge_dds::is_available());
}
