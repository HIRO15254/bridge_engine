//! A real DDS smoke test on a fixed deal and auction, a few samples (`docs/design/14-lead.md`
//! §4). Gracefully does nothing when the DDS sources were not vendored, matching
//! `crates/bridge/tests/dds.rs`'s own convention.
#![cfg(feature = "dds")]

mod common;

use bridge_core::{Bid, Call, Hand, Seat, Strain, Vulnerability};
use bridge_lead::{LeadOptions, LeadQuery, advise};
use bridge_sample::UniformProposal;
use common::empty_table;

#[test]
fn dds_backend_advises_on_a_fixed_deal() {
    let Some(dd) = bridge::dd::dds() else {
        eprintln!("DDS not vendored in this build (cargo xtask dds vendor); skipping");
        return;
    };

    let table = empty_table();
    let auction = bridge_core::Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        [
            Call::Bid(Bid::new(3, Strain::NoTrump).unwrap()),
            Call::Pass,
            Call::Pass,
            Call::Pass,
        ],
    )
    .expect("3NT opening, three passes, is a legal complete auction");

    // Any fixed 13-card hand; a real double-dummy solve does not need special structure.
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

    assert!(!advice.leads.is_empty());
    assert!(advice.leads.len() <= 3);
    assert_eq!(advice.declarer, Seat::North);
    assert_eq!(advice.leader, Seat::East);
    for lead in &advice.leads {
        assert!(lead.mean_defence_tricks >= 0.0 && lead.mean_defence_tricks <= 13.0);
        assert!(lead.set_probability >= 0.0 && lead.set_probability <= 1.0);
        assert!(lead.std_error >= 0.0);
    }
}
