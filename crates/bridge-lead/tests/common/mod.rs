//! Test-only helpers shared by `bridge-lead`'s integration tests.
//!
//! So that these tests do not depend on any particular system's bidding table (the real SAYC
//! system is exercised by `corpus_eval.rs`), every [`Table`] here is hand-built from
//! `bridge_system::ir`/`trie` types rather than compiled from BML source, exactly as
//! `crates/bridge-bidding/tests/common/mod.rs` does. An empty [`SystemIR`] is enough: every call
//! in the auction falls straight through Step A to natural inference (`07-bidding.md` §4.1
//! step 6), which is all these tests need, since they only exercise the sampling/aggregation
//! pipeline.
#![allow(dead_code)]

use std::sync::Arc;

use bridge::dd::{DdError, DoubleDummy};
use bridge_bidding::{NaturalInference, SystemIR, Table};
use bridge_core::{Card, DdTable, Deal, Hand, Rank, Seat, Strain, Suit};
use bridge_system::{AuctionTrie, SystemMeta};

/// A `Table` with no system rows at all: every call resolves through natural inference.
pub fn empty_table() -> Table {
    let ir = SystemIR {
        meta: SystemMeta::default(),
        rows: Vec::new(),
        nodes: Vec::new(),
        index: AuctionTrie::new(),
        lints: Vec::new(),
    };
    Table::uniform(Arc::new(ir), Arc::new(NaturalInference::default()))
}

/// A [`DoubleDummy`] that never actually solves anything: it scores each of the leader's cards
/// by a rule that depends only on the leader's own hand, so the expected outcome of sampling is
/// hand-computable and identical on every sampled deal (`14-lead.md` §4).
///
/// The rule: a card scores the length of the unbroken run of top-of-suit cards (starting at the
/// ace) that the leader holds in that suit, if the card itself is part of that run (leading any
/// card of a run of touching honours "cashes" the whole run); otherwise it scores 0. E.g. holding
/// A-K-Q-J-T-6-3 of a suit, the run is 5 (A through T): leading the ace, king, queen, jack or ten
/// all score 5 (and are therefore one equivalence group headed by the ace), while the 6 and the 3
/// score 0.
pub struct FakeDd;

impl DoubleDummy for FakeDd {
    fn dd_table(&self, _deal: &Deal) -> Result<DdTable, DdError> {
        unimplemented!("bridge-lead's tests only ever call lead_scores")
    }

    fn lead_scores(
        &self,
        deal: &Deal,
        _trump: Strain,
        leader: Seat,
    ) -> Result<Vec<(Card, u8)>, DdError> {
        let hand = deal.hand(leader);
        Ok(hand
            .cards()
            .map(|card| (card, fake_score(hand, card)))
            .collect())
    }
}

/// The length of the unbroken run of top-of-`suit` cards `hand` holds, starting at the ace.
fn suit_top_run(hand: Hand, suit: Suit) -> u8 {
    let mut run = 0u8;
    for i in (0..13).rev() {
        if hand.contains(Card::new(suit, Rank::from_index(i))) {
            run += 1;
        } else {
            break;
        }
    }
    run
}

fn fake_score(hand: Hand, card: Card) -> u8 {
    let run = suit_top_run(hand, card.suit());
    if run > 0 && i32::from(card.rank().index()) > 12 - i32::from(run) {
        run
    } else {
        0
    }
}

/// A [`DoubleDummy`] fake that, unlike [`FakeDd`], also reads the other three hands: it scores
/// each of the leader's cards by how many cards the leader's partner (dummy, on this contract)
/// holds in that card's suit. This makes the score genuinely deal-dependent (different on every
/// sample), so a real aggregation bug — a score matched to the wrong card, an accidental
/// dependence on deal iteration order — could actually change the computed statistics, which
/// [`FakeDd`]'s deal-independent scores cannot exercise (`docs/design/14-lead.md` §4).
pub struct DealDependentFakeDd;

impl DoubleDummy for DealDependentFakeDd {
    fn dd_table(&self, _deal: &Deal) -> Result<DdTable, DdError> {
        unimplemented!("bridge-lead's tests only ever call lead_scores")
    }

    fn lead_scores(
        &self,
        deal: &Deal,
        _trump: Strain,
        leader: Seat,
    ) -> Result<Vec<(Card, u8)>, DdError> {
        let partner_hand = deal.hand(leader.partner());
        Ok(deal
            .hand(leader)
            .cards()
            .map(|card| {
                let score = partner_hand
                    .cards()
                    .filter(|c| c.suit() == card.suit())
                    .count();
                (card, score as u8)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_score_is_the_run_length_for_touching_honours() {
        // Spades: A K Q J T 6 3 (run of 5), rest arbitrary but disjoint.
        let mut spades = Hand::EMPTY;
        for rank in [
            Rank::Ace,
            Rank::King,
            Rank::Queen,
            Rank::Jack,
            Rank::Ten,
            Rank::Six,
            Rank::Three,
        ] {
            spades = spades.with(Card::new(Suit::Spades, rank));
        }
        assert_eq!(suit_top_run(spades, Suit::Spades), 5);
        for rank in [Rank::Ace, Rank::King, Rank::Queen, Rank::Jack, Rank::Ten] {
            assert_eq!(fake_score(spades, Card::new(Suit::Spades, rank)), 5);
        }
        assert_eq!(fake_score(spades, Card::new(Suit::Spades, Rank::Six)), 0);
        assert_eq!(fake_score(spades, Card::new(Suit::Spades, Rank::Three)), 0);
    }

    #[test]
    fn fake_score_is_zero_without_the_ace() {
        let hand = Hand::EMPTY
            .with(Card::new(Suit::Hearts, Rank::King))
            .with(Card::new(Suit::Hearts, Rank::Queen));
        assert_eq!(fake_score(hand, Card::new(Suit::Hearts, Rank::King)), 0);
    }
}
