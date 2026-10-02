//! Error cases: an incomplete auction, a passed-out auction, a leader hand of the wrong size
//! (`docs/design/14-lead.md` §3 steps 1-2), and zero deals produced. The first three should never
//! touch the proposal or the double-dummy solver, so [`common::FakeDd`] and
//! [`bridge_sample::UniformProposal`] are used unconditionally.

mod common;

use bridge_core::{Bid, Call, Hand, Seat, Strain, Vulnerability};
use bridge_lead::{LeadError, LeadOptions, LeadQuery, advise};
use bridge_sample::UniformProposal;
use common::{FakeDd, empty_table};

fn full_hand() -> Hand {
    Hand::FULL
        .cards()
        .take(13)
        .fold(Hand::EMPTY, |h, c| h.with(c))
}

#[test]
fn incomplete_auction_is_an_error() {
    let table = empty_table();
    let auction = bridge_core::Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        [Call::Bid(Bid::new(1, Strain::Clubs).unwrap())],
    )
    .unwrap();
    let query = LeadQuery {
        auction: &auction,
        leader_hand: full_hand(),
    };
    let err = advise(
        &table,
        &query,
        &UniformProposal,
        &FakeDd,
        &LeadOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, LeadError::IncompleteAuction));
}

#[test]
fn passed_out_is_an_error() {
    let table = empty_table();
    let auction = bridge_core::Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        [Call::Pass, Call::Pass, Call::Pass, Call::Pass],
    )
    .unwrap();
    assert!(auction.is_passed_out());
    let query = LeadQuery {
        auction: &auction,
        leader_hand: full_hand(),
    };
    let err = advise(
        &table,
        &query,
        &UniformProposal,
        &FakeDd,
        &LeadOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, LeadError::PassedOut));
}

#[test]
fn wrong_hand_size_is_an_error() {
    let table = empty_table();
    let auction = bridge_core::Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        [
            Call::Bid(Bid::new(1, Strain::NoTrump).unwrap()),
            Call::Pass,
            Call::Pass,
            Call::Pass,
        ],
    )
    .unwrap();
    let mut short_hand = Hand::EMPTY;
    for card in Hand::FULL.cards().take(12) {
        short_hand = short_hand.with(card);
    }
    let query = LeadQuery {
        auction: &auction,
        leader_hand: short_hand,
    };
    let err = advise(
        &table,
        &query,
        &UniformProposal,
        &FakeDd,
        &LeadOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, LeadError::WrongHandSize { got: 12 }));
}

/// `samples: 0` produces zero deals, and `advise` must report [`LeadError::NoSamples`] rather
/// than fabricating advice from an empty sample (review finding: previously this returned `Ok`
/// with every card merged into one bogus "equivalence" group, mean 0, std_error 0 and set
/// probability 0, as if these were measured statistics).
#[test]
fn zero_samples_is_an_error() {
    let table = empty_table();
    let auction = bridge_core::Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        [
            Call::Bid(Bid::new(1, Strain::NoTrump).unwrap()),
            Call::Pass,
            Call::Pass,
            Call::Pass,
        ],
    )
    .unwrap();
    let query = LeadQuery {
        auction: &auction,
        leader_hand: full_hand(),
    };
    let opts = LeadOptions {
        samples: 0,
        ..LeadOptions::default()
    };
    let err = advise(&table, &query, &UniformProposal, &FakeDd, &opts).unwrap_err();
    assert!(matches!(err, LeadError::NoSamples { .. }));
}
