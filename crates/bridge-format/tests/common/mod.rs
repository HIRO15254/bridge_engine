//! Shared test helpers: corpus location and proptest strategies.
#![allow(dead_code)]

use std::path::PathBuf;

use bridge_core::{Auction, Call, Card, Deal, Hand, PlayHistory, Seat, Strain, Vulnerability};
use proptest::prelude::*;

/// `BRIDGE_CORPUS_DIR`, or `<workspace>/corpus/data`; `None` when the directory is absent.
pub fn corpus_dir() -> Option<PathBuf> {
    let dir = match std::env::var_os("BRIDGE_CORPUS_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/data"),
    };
    if dir.is_dir() {
        Some(dir)
    } else {
        eprintln!("corpus directory {} not found; skipping", dir.display());
        None
    }
}

/// Every file with `extension` under `dir`, recursively, sorted.
pub fn files_with_extension(dir: &std::path::Path, extension: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files_with_extension(&path, extension));
        } else if path.extension().is_some_and(|e| e == extension) {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// A random deal: 52 cards shuffled and dealt 13 to each seat.
pub fn arb_deal() -> impl Strategy<Value = Deal> {
    Just((0..52u8).collect::<Vec<u8>>())
        .prop_shuffle()
        .prop_map(|order| {
            let mut hands = [Hand::EMPTY; 4];
            for (k, index) in order.into_iter().enumerate() {
                let card = Card::from_index(index).expect("index < 52");
                hands[k / 13] = hands[k / 13].with(card);
            }
            Deal::new(hands).expect("13 cards each, all distinct")
        })
}

pub fn arb_seat() -> impl Strategy<Value = Seat> {
    (0..4u8).prop_map(Seat::from_index)
}

pub fn arb_vulnerability() -> impl Strategy<Value = Vulnerability> {
    (0..4u8).prop_map(Vulnerability::from_index)
}

/// A random legal auction, complete unless the random choices run out (bounded length).
pub fn arb_auction() -> impl Strategy<Value = Auction> {
    (
        arb_seat(),
        arb_vulnerability(),
        proptest::collection::vec(any::<prop::sample::Index>(), 4..40),
        // Bias towards passes so auctions terminate at realistic lengths.
        proptest::collection::vec(0..3u8, 4..40),
    )
        .prop_map(|(dealer, vul, picks, pass_bias)| {
            let mut auction = Auction::new(dealer, vul);
            for (pick, bias) in picks.into_iter().zip(pass_bias) {
                if auction.is_complete() {
                    break;
                }
                let call = if bias > 0 && auction.is_legal(Call::Pass) {
                    Call::Pass
                } else {
                    let legal: Vec<Call> = auction.legal_calls().collect();
                    *pick.get(&legal)
                };
                auction.push(call).expect("chosen from legal calls");
            }
            while !auction.is_complete() {
                auction.push(Call::Pass).expect("pass is always legal");
            }
            auction
        })
}

/// A random legal play of `n_cards` (bounded by 52) for `deal` in `trump` led by `leader`.
pub fn play_cards(
    deal: &Deal,
    trump: Strain,
    leader: Seat,
    picks: &[prop::sample::Index],
) -> PlayHistory {
    let mut history = PlayHistory::new(trump, leader);
    for pick in picks {
        if history.cards().len() >= 52 {
            break;
        }
        let seat = history.next_to_play();
        let remaining = deal.hand(seat).difference(history.played());
        let legal: Vec<Card> = remaining
            .cards()
            .filter(|c| history.is_legal(*c, remaining))
            .collect();
        let card = *pick.get(&legal);
        history
            .play(card, remaining)
            .expect("chosen from legal cards");
    }
    history
}
