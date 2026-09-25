//! End-to-end tests of [`bridge_lead::advise`] against [`common::FakeDd`], whose score depends
//! only on the leader's own (fixed, known) hand, so every expected value below is exactly
//! hand-computable and identical on every sampled deal regardless of the other three hands or
//! the importance weights (`docs/design/14-lead.md` §4).

mod common;

use bridge_core::{Auction, Bid, Call, Card, Hand, Rank, Seat, Strain, Suit, Vulnerability};
use bridge_lead::{LeadOptions, LeadQuery, LeadScoring, advise};
use bridge_sample::UniformProposal;
use common::{FakeDd, empty_table};

/// Spades A-K-Q-J-T (an unbroken top run of 5), plus 8 cards elsewhere that never touch an ace,
/// so [`common::FakeDd`] scores all of them 0: hearts 6-4-2, diamonds 7-5-3, clubs 4-2.
fn hand_with_a_spade_run() -> Hand {
    let mut hand = Hand::EMPTY;
    for rank in [Rank::Ace, Rank::King, Rank::Queen, Rank::Jack, Rank::Ten] {
        hand = hand.with(Card::new(Suit::Spades, rank));
    }
    for rank in [Rank::Six, Rank::Four, Rank::Two] {
        hand = hand.with(Card::new(Suit::Hearts, rank));
    }
    for rank in [Rank::Seven, Rank::Five, Rank::Three] {
        hand = hand.with(Card::new(Suit::Diamonds, rank));
    }
    for rank in [Rank::Four, Rank::Two] {
        hand = hand.with(Card::new(Suit::Clubs, rank));
    }
    hand
}

/// North opens 3NT (a legal, if unnatural, auction: `Auction::is_legal` only checks bidding
/// mechanics, not any system), everyone passes. Declarer is North (the only player who named
/// notrump), so the leader is East.
fn three_nt_by_north() -> Auction {
    Auction::from_calls(
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

fn small_options(top_k: usize) -> LeadOptions {
    LeadOptions {
        samples: 30,
        seed: 7,
        top_k,
        ..LeadOptions::default()
    }
}

#[test]
fn contract_declarer_and_leader_are_derived_from_the_auction() {
    let table = empty_table();
    let auction = three_nt_by_north();
    let query = LeadQuery {
        auction: &auction,
        leader_hand: hand_with_a_spade_run(),
    };
    let opts = small_options(3);
    let advice = advise(&table, &query, &UniformProposal, &FakeDd, &opts)
        .expect("a complete, non-passed-out 3NT auction advises fine");

    assert_eq!(advice.contract, auction.contract().unwrap());
    assert_eq!(advice.declarer, Seat::North);
    assert_eq!(advice.leader, Seat::East);
}

#[test]
fn obvious_best_lead_is_the_ace_of_the_five_card_run() {
    let table = empty_table();
    let auction = three_nt_by_north();
    let query = LeadQuery {
        auction: &auction,
        leader_hand: hand_with_a_spade_run(),
    };
    // 3NT: the defence needs 8 - 3 = 5 tricks to defeat it, exactly the run's length.
    let opts = small_options(1);
    let advice = advise(&table, &query, &UniformProposal, &FakeDd, &opts).unwrap();

    assert_eq!(advice.leads.len(), 1, "top_k = 1");
    let top = &advice.leads[0];
    assert_eq!(top.rank, 1);
    assert_eq!(top.card, Card::new(Suit::Spades, Rank::Ace));
    assert_eq!(
        top.equivalents,
        vec![
            Card::new(Suit::Spades, Rank::King),
            Card::new(Suit::Spades, Rank::Queen),
            Card::new(Suit::Spades, Rank::Jack),
            Card::new(Suit::Spades, Rank::Ten),
        ],
        "touching honours, highest rank first, excluding the representative"
    );
    assert!(
        (top.mean_defence_tricks - 5.0).abs() < 1e-9,
        "mean = {}",
        top.mean_defence_tricks
    );
    assert!(
        top.std_error < 1e-9,
        "the fake score is constant across every sample, so std_error should be ~0, got {}",
        top.std_error
    );
    assert!(
        (top.set_probability - 1.0).abs() < 1e-9,
        "5 >= threshold(5) on every sample, so set_probability should be 1.0, got {}",
        top.set_probability
    );
}

#[test]
fn the_rest_of_the_hand_is_one_group_scoring_zero() {
    let table = empty_table();
    let auction = three_nt_by_north();
    let query = LeadQuery {
        auction: &auction,
        leader_hand: hand_with_a_spade_run(),
    };
    let opts = small_options(2);
    let advice = advise(&table, &query, &UniformProposal, &FakeDd, &opts).unwrap();

    assert_eq!(
        advice.leads.len(),
        2,
        "exactly two equivalence groups exist"
    );
    let second = &advice.leads[1];
    assert_eq!(second.rank, 2);
    // Diamonds 7 is the highest rank among the eight cards that never touch an ace.
    assert_eq!(second.card, Card::new(Suit::Diamonds, Rank::Seven));
    assert_eq!(second.equivalents.len(), 7);
    assert!((second.mean_defence_tricks - 0.0).abs() < 1e-9);
    assert!((second.set_probability - 0.0).abs() < 1e-9);
}

#[test]
fn same_seed_gives_identical_advice() {
    let table = empty_table();
    let auction = three_nt_by_north();
    let query = LeadQuery {
        auction: &auction,
        leader_hand: hand_with_a_spade_run(),
    };
    let opts = small_options(3);

    let a = advise(&table, &query, &UniformProposal, &FakeDd, &opts).unwrap();
    let b = advise(&table, &query, &UniformProposal, &FakeDd, &opts).unwrap();

    assert_eq!(a.samples_used, b.samples_used);
    assert_eq!(a.sample_report.produced, b.sample_report.produced);
    assert_eq!(a.sample_report.attempts, b.sample_report.attempts);
    assert_eq!(a.ess, b.ess);
    assert_eq!(a.leads.len(), b.leads.len());
    for (la, lb) in a.leads.iter().zip(&b.leads) {
        assert_eq!(la.card, lb.card);
        assert_eq!(la.equivalents, lb.equivalents);
        assert_eq!(la.rank, lb.rank);
        assert_eq!(la.mean_defence_tricks, lb.mean_defence_tricks);
        assert_eq!(la.std_error, lb.std_error);
        assert_eq!(la.set_probability, lb.set_probability);
    }
}

/// Compiled with `--features parallel`, `lead_scores_for_all` runs the DD calls on rayon's
/// global pool instead of sequentially; either way the fake scorer is deal-independent, so this
/// checks the same hand-computed values as `obvious_best_lead_is_the_ace_of_the_five_card_run`
/// (the closest same-process substitute for "single vs parallel are identical" without linking
/// two differently-featured copies of the crate into one test binary).
#[test]
fn parallel_or_not_gives_the_same_hand_computed_result() {
    let table = empty_table();
    let auction = three_nt_by_north();
    let query = LeadQuery {
        auction: &auction,
        leader_hand: hand_with_a_spade_run(),
    };
    let opts = small_options(1);
    let advice = advise(&table, &query, &UniformProposal, &FakeDd, &opts).unwrap();
    assert_eq!(advice.leads[0].card, Card::new(Suit::Spades, Rank::Ace));
    assert!((advice.leads[0].mean_defence_tricks - 5.0).abs() < 1e-9);
}

#[test]
fn scoring_by_set_probability_ranks_the_setting_group_first() {
    let table = empty_table();
    let auction = three_nt_by_north();
    let query = LeadQuery {
        auction: &auction,
        leader_hand: hand_with_a_spade_run(),
    };
    let opts = LeadOptions {
        scoring: LeadScoring::SetProbability,
        ..small_options(2)
    };
    let advice = advise(&table, &query, &UniformProposal, &FakeDd, &opts).unwrap();
    assert_eq!(advice.leads[0].card, Card::new(Suit::Spades, Rank::Ace));
}

#[test]
fn scoring_by_score_ranks_the_most_damaging_group_first() {
    let table = empty_table();
    let auction = three_nt_by_north();
    let query = LeadQuery {
        auction: &auction,
        leader_hand: hand_with_a_spade_run(),
    };
    let opts = LeadOptions {
        scoring: LeadScoring::Score,
        ..small_options(2)
    };
    let advice = advise(&table, &query, &UniformProposal, &FakeDd, &opts).unwrap();
    // Setting the contract (defence takes 5 of 13, declarer only 8 < 9 required) is worse for
    // declarer than any group that lets it make, so the spade run sorts first under `Score` too.
    assert_eq!(advice.leads[0].card, Card::new(Suit::Spades, Rank::Ace));
}
