//! Shared `proptest` strategies for the integration tests.

#![allow(dead_code)]

use bridge_core::{Auction, Call, Card, Deal, Hand, Seat, Vulnerability};
use proptest::prelude::*;
use proptest::sample::Index;

/// A random subset of the deck (any size).
pub fn arb_hand() -> impl Strategy<Value = Hand> {
    any::<u64>().prop_map(|bits| Hand::from_bits(bits & Hand::FULL.bits()).unwrap())
}

/// A random deal: a Fisher–Yates shuffle of the deck driven by 51 sample indices.
pub fn arb_deal() -> impl Strategy<Value = Deal> {
    prop::collection::vec(any::<Index>(), 51).prop_map(|indices| {
        let mut cards: Vec<u8> = (0..52).collect();
        for (i, ix) in indices.iter().enumerate() {
            let j = i + ix.index(52 - i);
            cards.swap(i, j);
        }
        let mut hands = [Hand::EMPTY; 4];
        for (k, c) in cards.iter().enumerate() {
            hands[k / 13] = hands[k / 13].with(Card::from_index(*c).unwrap());
        }
        Deal::new(hands).unwrap()
    })
}

pub fn arb_seat() -> impl Strategy<Value = Seat> {
    (0u8..4).prop_map(Seat::from_index)
}

pub fn arb_vulnerability() -> impl Strategy<Value = Vulnerability> {
    (0u8..4).prop_map(Vulnerability::from_index)
}

/// A random *complete* legal auction: at every step, pass with probability ~1/2, otherwise a
/// uniformly chosen legal call. Bids strictly increase, so 320 steps always suffice.
pub fn arb_auction() -> impl Strategy<Value = Auction> {
    (
        arb_seat(),
        arb_vulnerability(),
        prop::collection::vec((any::<bool>(), any::<Index>()), 320),
    )
        .prop_map(|(dealer, vul, steps)| {
            let mut auction = Auction::new(dealer, vul);
            for (pass, ix) in steps {
                if auction.is_complete() {
                    break;
                }
                let call = if pass {
                    Call::Pass
                } else {
                    let legal: Vec<Call> = auction.legal_calls().collect();
                    legal[ix.index(legal.len())]
                };
                auction.push(call).unwrap();
            }
            auction
        })
}
