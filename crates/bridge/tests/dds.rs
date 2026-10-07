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

/// DDS reports one representative per group of equivalent cards (`equals` holds the rest);
/// `lead_scores` must still list every card in the leader's hand, each with its group's score.
/// North here holds `AKQ` of spades and `T98` in three suits, so most leads are equivalent to
/// another one (the formula deal above has no touching cards, so it cannot catch this).
#[test]
fn lead_scores_lists_every_card_including_equivalent_ones() {
    let Some(backend) = dds() else {
        eprintln!("DDS not vendored in this build (cargo xtask dds vendor); skipping");
        return;
    };
    let deal: Deal = "N:AKQ2.T98.T98.T98 3.AKQJ.AKQJ.AKQJ JT98.765.765.765 7654.432.432.432"
        .parse()
        .expect("valid deal");
    let scores = backend
        .lead_scores(&deal, Strain::NoTrump, Seat::North)
        .expect("lead_scores should succeed on a valid deal");

    let mut cards: Vec<Card> = scores.iter().map(|&(card, _)| card).collect();
    cards.sort();
    cards.dedup();
    assert_eq!(
        cards.len(),
        13,
        "one entry per card, no duplicates: {scores:?}"
    );
    assert!(cards.iter().all(|&c| deal.hand(Seat::North).contains(c)));

    let score_of = |s: &str| {
        let card: Card = s.parse().expect("valid card");
        scores
            .iter()
            .find(|&&(c, _)| c == card)
            .map(|&(_, t)| t)
            .unwrap_or_else(|| panic!("{card} missing from {scores:?}"))
    };
    for group in [["SA", "SK", "SQ"], ["HT", "H9", "H8"], ["DT", "D9", "D8"]] {
        let first = score_of(group[0]);
        for card in &group[1..] {
            assert_eq!(score_of(card), first, "{group:?} are equivalent leads");
        }
    }
}
