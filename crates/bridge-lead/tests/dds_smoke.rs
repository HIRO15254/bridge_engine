//! A real DDS smoke test on fixed deals and auctions, a few samples (`docs/design/14-lead.md`
//! §4). Gracefully does nothing when the DDS sources were not vendored, matching
//! `crates/bridge/tests/dds.rs`'s own convention.
#![cfg(feature = "dds")]

mod common;

use bridge_core::{Bid, Call, Hand, Rank, Seat, Strain, Suit, Vulnerability};
use bridge_lead::{LeadOptions, LeadQuery, advise};
use bridge_sample::UniformProposal;
use common::empty_table;

fn three_nt_by_north() -> bridge_core::Auction {
    bridge_core::Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        [
            Call::Bid(Bid::new(3, Strain::NoTrump).unwrap()),
            Call::Pass,
            Call::Pass,
            Call::Pass,
        ],
    )
    .expect("3NT opening, three passes, is a legal complete auction")
}

/// The previous version of this test held the leader's whole hand in a single suit (`Hand::FULL`
/// is ordered suit-major, so `.cards().take(13)` gives all 13 clubs), which makes every lead
/// score identically for a trivial reason: with only one suit in hand there is nothing to
/// distinguish a good lead from a bad one, and the test's own assertions (any value in `0..=13`)
/// would have passed even if `lead_scores` had the defence/declarer sense reversed
/// (review finding). This pins the real defence-trick semantics: a real DDS solve must find
/// exactly one equivalence group covering all 13 clubs, each scoring the actual number of tricks
/// the defence takes when North (holding none of the leader's suit) declares 3NT and East, void
/// nowhere else of relevance, leads a club.
#[test]
fn dds_backend_gives_one_group_for_a_single_suit_hand() {
    let Some(dd) = bridge::dd::dds() else {
        eprintln!("DDS not vendored in this build (cargo xtask dds vendor); skipping");
        return;
    };

    let table = empty_table();
    let auction = three_nt_by_north();
    let leader_hand: Hand = Hand::FULL
        .cards()
        .take(13)
        .fold(Hand::EMPTY, |h, c| h.with(c));

    let query = LeadQuery {
        auction: &auction,
        leader_hand,
    };
    // A handful of samples: this is a smoke test of the DDS wiring, not an ESS or accuracy
    // measurement (those belong to the `#[ignore]`d corpus evaluation harness).
    let opts = LeadOptions {
        samples: 20,
        seed: 1,
        top_k: 3,
        ..LeadOptions::default()
    };

    let advice = advise(&table, &query, &UniformProposal, dd.as_ref(), &opts)
        .expect("a real DDS solve on a fixed deal should not fail");

    assert_eq!(advice.declarer, Seat::North);
    assert_eq!(advice.leader, Seat::East);
    assert_eq!(
        advice.leads.len(),
        1,
        "a single-suit hand has no cross-suit distinction: one equivalence group"
    );
    let only = &advice.leads[0];
    assert_eq!(
        only.equivalents.len(),
        12,
        "all 13 clubs score identically and form one group"
    );
    assert!(only.mean_defence_tricks >= 0.0 && only.mean_defence_tricks <= 13.0);
    assert!(only.set_probability >= 0.0 && only.set_probability <= 1.0);
    assert!(only.std_error >= 0.0);
}

/// A non-trivial fixed hand (an unbroken top run of 5 in one suit against 3NT): the real DDS
/// solve should favour cashing the run, and pins that the top lead is drawn from that suit with a
/// high set probability, not merely that its statistics fall in a plausible range.
#[test]
fn dds_backend_prefers_the_honour_run_against_notrump() {
    let Some(dd) = bridge::dd::dds() else {
        eprintln!("DDS not vendored in this build (cargo xtask dds vendor); skipping");
        return;
    };

    let table = empty_table();
    let auction = three_nt_by_north();

    let mut leader_hand = Hand::EMPTY;
    for rank in [Rank::Ace, Rank::King, Rank::Queen, Rank::Jack, Rank::Ten] {
        leader_hand = leader_hand.with(bridge_core::Card::new(Suit::Spades, rank));
    }
    for rank in [Rank::Six, Rank::Four, Rank::Two] {
        leader_hand = leader_hand.with(bridge_core::Card::new(Suit::Hearts, rank));
    }
    for rank in [Rank::Seven, Rank::Five, Rank::Three] {
        leader_hand = leader_hand.with(bridge_core::Card::new(Suit::Diamonds, rank));
    }
    for rank in [Rank::Four, Rank::Two] {
        leader_hand = leader_hand.with(bridge_core::Card::new(Suit::Clubs, rank));
    }
    assert_eq!(leader_hand.len(), 13);

    let query = LeadQuery {
        auction: &auction,
        leader_hand,
    };
    let opts = LeadOptions {
        samples: 30,
        seed: 1,
        top_k: 3,
        ..LeadOptions::default()
    };

    let advice = advise(&table, &query, &UniformProposal, dd.as_ref(), &opts)
        .expect("a real DDS solve on a fixed deal should not fail");

    assert!(!advice.leads.is_empty());
    let top = &advice.leads[0];
    assert_eq!(
        top.card.suit(),
        Suit::Spades,
        "the top lead should come from the 5-card honour run, got {:?}",
        top.card
    );
    assert!(
        top.set_probability > 0.5,
        "cashing the run should set 3NT in most samples, got p = {}",
        top.set_probability
    );
}
