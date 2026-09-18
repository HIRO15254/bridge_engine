//! Auction legality (Laws 17–19), contract derivation and the seat helpers.

use bridge_core::{Auction, AuctionError, Bid, Call, Doubling, Seat, Strain, Vulnerability};
use proptest::prelude::*;

mod common;
use common::arb_auction;

fn call(s: &str) -> Call {
    s.parse().unwrap()
}

fn bid(s: &str) -> Bid {
    s.parse().unwrap()
}

/// Builds an auction from PBN-style call text (dealer North, no vulnerability).
fn auction(dealer: Seat, text: &str) -> Auction {
    Auction::from_calls(
        dealer,
        Vulnerability::None,
        text.split_whitespace().map(call),
    )
    .unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn north(text: &str) -> Auction {
    auction(Seat::North, text)
}

#[test]
fn completion_edges() {
    assert!(!north("").is_complete());
    assert!(!north("P").is_complete());
    assert!(!north("P P P").is_complete());
    assert!(north("P P P P").is_complete());
    assert!(north("P P P P").is_passed_out());
    assert!(north("1C P P P").is_complete());
    assert!(!north("1C P P P").is_passed_out());
    assert!(!north("1C P P").is_complete());
    assert!(!north("P P P 1C").is_complete());
    assert!(!north("P P P 1C P P").is_complete());
    assert!(north("P P P 1C P P P").is_complete());
    assert!(north("1S X P P P").is_complete());
}

#[test]
fn nothing_is_legal_after_completion() {
    for text in ["P P P P", "1C P P P", "1S X XX P P P"] {
        let a = north(text);
        for i in 0..38 {
            assert!(!a.is_legal(Call::from_index(i).unwrap()), "{text} {i}");
        }
        assert_eq!(a.legal_calls().count(), 0);
        let mut b = a.clone();
        assert_eq!(
            b.push(Call::Pass),
            Err(AuctionError::IllegalCall {
                call: Call::Pass,
                index: a.len()
            })
        );
        assert_eq!(b, a);
    }
}

#[test]
fn bids_must_be_higher() {
    let a = north("1H");
    assert!(a.is_legal(call("1S")));
    assert!(a.is_legal(call("2C")));
    assert!(a.is_legal(call("7NT")));
    assert!(!a.is_legal(call("1H")));
    assert!(!a.is_legal(call("1D")));
    assert!(!a.is_legal(call("1C")));
    let top = north("7NT");
    assert_eq!(
        top.legal_calls().collect::<Vec<_>>(),
        vec![Call::Pass, Call::Double]
    );
    let empty = north("");
    assert_eq!(empty.legal_calls().count(), 36);
    assert_eq!(empty.legal_calls().next(), Some(Call::Pass));
    assert_eq!(empty.legal_calls().nth(1), Some(call("1C")));
}

#[test]
fn double_legality() {
    // Dealer North. 1C by N.
    assert!(north("1C").is_legal(Call::Double)); // E, over RHO's bid
    assert!(!north("1C P").is_legal(Call::Double)); // S may not double partner
    assert!(north("1C P P").is_legal(Call::Double)); // W, balancing
    assert!(!north("1C X").is_legal(Call::Double)); // last non-pass is a double
    assert!(!north("1C X XX").is_legal(Call::Double));
    assert!(!north("").is_legal(Call::Double));
    assert!(!north("P").is_legal(Call::Double));
    assert!(!north("1C X P P").is_legal(Call::Double)); // N: last non-pass is E's double
    assert!(north("1C X P P 1D").is_legal(Call::Double)); // E over N's 1D
}

#[test]
fn redouble_legality() {
    assert!(north("1C X").is_legal(Call::Redouble)); // S over E's double
    assert!(!north("1C X P").is_legal(Call::Redouble)); // W: partner doubled
    assert!(north("1C X P P").is_legal(Call::Redouble)); // N over E's double
    assert!(!north("1C").is_legal(Call::Redouble));
    assert!(!north("1C X XX").is_legal(Call::Redouble));
    assert!(!north("1C X XX P").is_legal(Call::Redouble));
    assert!(!north("").is_legal(Call::Redouble));
}

#[test]
fn legal_calls_agree_with_is_legal_and_are_in_index_order() {
    for text in [
        "", "P", "1C", "1C P", "1C X", "1C X P P", "3NT P P", "7NT", "P P P",
    ] {
        let a = north(text);
        let listed: Vec<Call> = a.legal_calls().collect();
        let expected: Vec<Call> = (0..38)
            .map(|i| Call::from_index(i).unwrap())
            .filter(|c| a.is_legal(*c))
            .collect();
        assert_eq!(listed, expected, "{text}");
        assert!(listed.windows(2).all(|w| w[0].index() < w[1].index()));
    }
}

#[test]
fn push_leaves_the_auction_unchanged_on_error() {
    let mut a = north("1H");
    let before = a.clone();
    assert_eq!(
        a.push(call("1C")),
        Err(AuctionError::IllegalCall {
            call: call("1C"),
            index: 1
        })
    );
    assert_eq!(a, before);
    assert_eq!(
        a.with(Call::Redouble),
        Err(AuctionError::IllegalCall {
            call: Call::Redouble,
            index: 1
        })
    );
    assert_eq!(
        a.with(call("1S")).unwrap().calls(),
        &[call("1H"), call("1S")]
    );
    let err = Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        [call("1C"), Call::Pass, Call::Double],
    )
    .unwrap_err();
    assert_eq!(
        err,
        AuctionError::IllegalCall {
            call: Call::Double,
            index: 2
        }
    );
    assert_eq!(err.to_string(), "call X is illegal at position 2");
}

#[test]
fn contract_declarer_is_the_first_to_name_the_strain() {
    // N 1C, E P, S 1H, W P, N 3H, E P, S 4H, W P, N P, E P: South bid hearts first for NS.
    let a = north("1C P 1H P 3H P 4H P P P");
    let c = a.contract().unwrap();
    assert_eq!(c.bid, bid("4H"));
    assert_eq!(c.declarer, Seat::South);
    assert_eq!(c.doubling, Doubling::Undoubled);
    assert_eq!(c.leader(), Seat::West);

    // North names notrump first even though South makes the final bid.
    let a = north("1NT P 3NT P P P");
    assert_eq!(a.contract().unwrap().declarer, Seat::North);

    // The opponents' earlier heart bid does not count.
    let a = north("1H X P 2H P P P");
    // N 1H, E X, S P, W 2H (cue), N P, E P, S P → EW play 2H; W named hearts first for EW.
    let c = a.contract().unwrap();
    assert_eq!(c.bid, bid("2H"));
    assert_eq!(c.declarer, Seat::West);

    // A different dealer shifts the seats.
    let a = auction(Seat::East, "1C P 1H P 3H P 4H P P P");
    assert_eq!(a.contract().unwrap().declarer, Seat::West);
}

#[test]
fn contract_doubling_comes_from_the_last_non_pass() {
    let a = north("1S X P P P");
    let c = a.contract().unwrap();
    assert_eq!(c.declarer, Seat::North);
    assert_eq!(c.doubling, Doubling::Doubled);
    assert_eq!(c.to_string(), "1SX");

    let a = north("1S X XX P P P");
    assert_eq!(a.contract().unwrap().doubling, Doubling::Redoubled);

    // A double that is followed by a bid is gone.
    let a = north("1S X 2S P P P");
    let c = a.contract().unwrap();
    assert_eq!(c.doubling, Doubling::Undoubled);
    assert_eq!(c.bid, bid("2S"));
    assert_eq!(c.declarer, Seat::North);
}

#[test]
fn contract_is_none_when_incomplete_or_passed_out() {
    assert_eq!(north("").contract(), None);
    assert_eq!(north("1C P P").contract(), None);
    assert_eq!(north("P P P").contract(), None);
    assert_eq!(north("P P P P").contract(), None);
    assert!(north("P P P P").is_passed_out());
    assert_eq!(north("P P P P").last_bid(), None);
}

#[test]
fn leading_passes_and_positions() {
    assert_eq!(north("").leading_passes(), 0);
    assert_eq!(north("1C").leading_passes(), 0);
    assert_eq!(north("P").leading_passes(), 1);
    assert_eq!(north("P P 1C").leading_passes(), 2);
    assert_eq!(north("P P P 1C P P P").leading_passes(), 3);
    assert_eq!(north("P P P P").leading_passes(), 4);

    let a = auction(Seat::South, "");
    assert_eq!(a.position_of(Seat::South), 1);
    assert_eq!(a.position_of(Seat::West), 2);
    assert_eq!(a.position_of(Seat::North), 3);
    assert_eq!(a.position_of(Seat::East), 4);
    for dealer in Seat::ALL {
        let a = auction(dealer, "P P");
        let k = a.leading_passes();
        assert_eq!(a.position_of(a.seat_at(k)) as usize, k + 1);
    }
}

#[test]
fn seats_and_calls_by() {
    let a = auction(Seat::East, "1C P 1H P 3H P");
    assert_eq!(a.dealer(), Seat::East);
    assert_eq!(a.vulnerability(), Vulnerability::None);
    assert_eq!(a.seat_at(0), Seat::East);
    assert_eq!(a.seat_at(3), Seat::North);
    assert_eq!(a.seat_at(4), Seat::East);
    assert_eq!(a.next_seat(), Seat::West);
    assert_eq!(a.len(), 6);
    assert!(!a.is_empty());
    assert_eq!(
        a.calls_by(Seat::East).collect::<Vec<_>>(),
        vec![(0, call("1C")), (4, call("3H"))]
    );
    assert_eq!(
        a.calls_by(Seat::West).collect::<Vec<_>>(),
        vec![(2, call("1H"))]
    );
    assert_eq!(
        a.calls_by(Seat::North).collect::<Vec<_>>(),
        vec![(3, Call::Pass)]
    );
    assert_eq!(a.calls_by(Seat::South).count(), 2);
    assert_eq!(a.last_bid(), Some((4, bid("3H"))));
    assert_eq!(a.last_non_pass(), Some((4, call("3H"))));
    assert_eq!(north("1C X P").last_non_pass(), Some((1, Call::Double)));
}

/// The longest legal auction: three passes, then every bid separated by
/// `P P X P P XX P P`, then `P P X P P XX P P P` after 7NT: 319 calls.
#[test]
fn longest_auction_fits_in_320_calls() {
    let mut a = Auction::new(Seat::North, Vulnerability::Both);
    for _ in 0..3 {
        a.push(Call::Pass).unwrap();
    }
    let between = [
        Call::Pass,
        Call::Pass,
        Call::Double,
        Call::Pass,
        Call::Pass,
        Call::Redouble,
        Call::Pass,
        Call::Pass,
    ];
    for i in 0..35 {
        a.push(Call::Bid(Bid::from_index(i).unwrap())).unwrap();
        for c in between {
            a.push(c).unwrap();
        }
    }
    a.push(Call::Pass).unwrap();
    assert!(a.is_complete());
    assert_eq!(a.len(), 319);
    let c = a.contract().unwrap();
    assert_eq!(c.bid, Bid::new(7, Strain::NoTrump).unwrap());
    assert_eq!(c.doubling, Doubling::Redoubled);
    let again =
        Auction::from_calls(a.dealer(), a.vulnerability(), a.calls().iter().copied()).unwrap();
    assert_eq!(again, a);
}

proptest! {
    #[test]
    fn random_auctions_terminate_and_round_trip(a in arb_auction()) {
        prop_assert!(a.is_complete());
        prop_assert!(a.len() <= 320);
        let again = Auction::from_calls(a.dealer(), a.vulnerability(), a.calls().iter().copied()).unwrap();
        prop_assert_eq!(&again, &a);
        prop_assert_eq!(a.legal_calls().count(), 0);
        // Either passed out or there is a contract whose bid is the last bid.
        match a.contract() {
            None => prop_assert!(a.is_passed_out()),
            Some(c) => {
                prop_assert_eq!(Some(c.bid), a.last_bid().map(|(_, b)| b));
                let (i, _) = a.last_bid().unwrap();
                prop_assert_eq!(c.declarer.side(), a.seat_at(i).side());
            }
        }
        // Every prefix is legal and every call was legal when it was made.
        let mut prefix = Auction::new(a.dealer(), a.vulnerability());
        for c in a.calls() {
            prop_assert!(prefix.is_legal(*c));
            prefix.push(*c).unwrap();
        }
    }
}
