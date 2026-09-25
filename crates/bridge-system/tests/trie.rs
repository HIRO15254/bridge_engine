//! Integration tests for `AuctionTrie` / `LookupKey` / `Lookup`, driven only through the public
//! API with hand-built tries (`NodeId` values only; no compiled system is needed).

use bridge_core::{Auction, Bid, Call, Seat, Strain, Vulnerability};
use bridge_system::ast::{SeatCond, Tri, VulCond};
use bridge_system::trie::{AuctionTrie, Edge, LookupKey, RelVul, TrieId};
use bridge_system::{NodeId, OppClass};

fn bid(level: u8, strain: Strain) -> Call {
    Call::Bid(Bid::new(level, strain).unwrap())
}

const PASS: Call = Call::Pass;
const DBL: Call = Call::Double;

fn no_vul() -> RelVul {
    RelVul {
        we: false,
        they: false,
    }
}

/// SAYC-ish opening structure used by several tests below: `1C` opens, `1H` is a natural
/// response, and any suit overcall from the opponents is doubled for penalties/takeout.
fn sayc_like_trie() -> (AuctionTrie, NodeId, NodeId, NodeId) {
    let mut trie = AuctionTrie::new();
    let opening = NodeId(10);
    let response = NodeId(11);
    let double_over_overcall = NodeId(12);

    trie.insert(
        true,
        &[bid(1, Strain::Clubs)],
        SeatCond::Any,
        VulCond::default(),
        opening,
    )
    .unwrap();
    trie.insert(
        true,
        &[bid(1, Strain::Clubs), PASS, bid(1, Strain::Hearts)],
        SeatCond::Any,
        VulCond::default(),
        response,
    )
    .unwrap();
    trie.insert_path(
        true,
        &[
            Edge::Call(bid(1, Strain::Clubs)),
            Edge::Class(OppClass::AnySuitBid),
            Edge::Call(DBL),
        ],
        SeatCond::Any,
        VulCond::default(),
        double_over_overcall,
    )
    .unwrap();

    (trie, opening, response, double_over_overcall)
}

#[test]
fn exact_and_wildcard_paths_both_resolve() {
    let (trie, opening, response, double_over_overcall) = sayc_like_trie();

    // Exact: 1C - (P) - 1H.
    let calls = [bid(1, Strain::Clubs), PASS, bid(1, Strain::Hearts)];
    let key = LookupKey {
        we_opened: true,
        calls: &calls,
        opener_pos: 1,
        vul: no_vul(),
    };
    let lookup = trie.resolve(&key);
    assert!(lookup.is_exact(&key));
    assert_eq!(lookup.via_class, 0);
    assert_eq!(
        lookup.by_depth.into_iter().collect::<Vec<_>>(),
        vec![Some(opening), None, Some(response)]
    );

    // Wildcard: 1C - (any suit overcall, e.g. 2H) - X. Matched via the `AnySuitBid` class edge,
    // regardless of which suit the opponents actually bid.
    for overcall in [
        bid(1, Strain::Spades),
        bid(2, Strain::Hearts),
        bid(3, Strain::Diamonds),
    ] {
        let calls = [bid(1, Strain::Clubs), overcall, DBL];
        let key = LookupKey {
            we_opened: true,
            calls: &calls,
            opener_pos: 1,
            vul: no_vul(),
        };
        let lookup = trie.resolve(&key);
        assert!(
            lookup.is_exact(&key),
            "overcall {overcall:?} should resolve"
        );
        assert_eq!(lookup.via_class, 1);
        assert_eq!(lookup.by_depth[2], Some(double_over_overcall));
    }

    // A notrump overcall is not a *suit* bid, so the `AnySuitBid` class edge does not match it.
    let calls = [bid(1, Strain::Clubs), bid(1, Strain::NoTrump), DBL];
    let key = LookupKey {
        we_opened: true,
        calls: &calls,
        opener_pos: 1,
        vul: no_vul(),
    };
    let lookup = trie.resolve(&key);
    assert_eq!(lookup.matched_depth, 1);
}

#[test]
fn opp_class_predicates() {
    assert!(OppClass::AnyCall.matches(PASS));
    assert!(OppClass::AnyCall.matches(bid(7, Strain::NoTrump)));

    assert!(!OppClass::AnyBid.matches(PASS));
    assert!(OppClass::AnyBid.matches(bid(1, Strain::NoTrump)));

    assert!(OppClass::AnySuitBid.matches(bid(1, Strain::Spades)));
    assert!(!OppClass::AnySuitBid.matches(bid(1, Strain::NoTrump)));
    assert!(!OppClass::AnySuitBid.matches(PASS));

    assert!(OppClass::AnyBidAtLevel(2).matches(bid(2, Strain::Clubs)));
    assert!(!OppClass::AnyBidAtLevel(2).matches(bid(3, Strain::Clubs)));

    assert!(OppClass::Double.matches(DBL));
    assert!(!OppClass::Double.matches(Call::Redouble));

    assert!(OppClass::Pass.matches(PASS));
    assert!(!OppClass::Pass.matches(DBL));
}

#[test]
fn seat_specificity_beats_any_and_ties_go_to_first_definition() {
    let mut trie = AuctionTrie::new();
    let any = NodeId(1);
    let first_or_second = NodeId(2);
    let third_or_fourth = NodeId(3);
    let exact_seat = NodeId(4);

    trie.insert(
        true,
        &[bid(1, Strain::NoTrump)],
        SeatCond::Any,
        VulCond::default(),
        any,
    )
    .unwrap();
    trie.insert(
        true,
        &[bid(1, Strain::NoTrump)],
        SeatCond::FirstOrSecond,
        VulCond::default(),
        first_or_second,
    )
    .unwrap();
    trie.insert(
        true,
        &[bid(1, Strain::NoTrump)],
        SeatCond::ThirdOrFourth,
        VulCond::default(),
        third_or_fourth,
    )
    .unwrap();
    trie.insert(
        true,
        &[bid(1, Strain::NoTrump)],
        SeatCond::Second,
        VulCond::default(),
        exact_seat,
    )
    .unwrap();

    let calls = [bid(1, Strain::NoTrump)];
    fn resolve_at(trie: &AuctionTrie, calls: &[Call], opener_pos: u8) -> Option<NodeId> {
        let key = LookupKey {
            we_opened: true,
            calls,
            opener_pos,
            vul: no_vul(),
        };
        trie.resolve(&key).by_depth[0]
    }

    assert_eq!(resolve_at(&trie, &calls, 1), Some(first_or_second)); // FirstOrSecond beats Any.
    assert_eq!(resolve_at(&trie, &calls, 2), Some(exact_seat)); // Second beats FirstOrSecond.
    assert_eq!(resolve_at(&trie, &calls, 3), Some(third_or_fourth));
    assert_eq!(resolve_at(&trie, &calls, 4), Some(third_or_fourth));

    // Re-inserting the identical (seat, vul) condition is rejected; the first definition wins.
    let dup = trie.insert(
        true,
        &[bid(1, Strain::NoTrump)],
        SeatCond::Second,
        VulCond::default(),
        NodeId(99),
    );
    assert_eq!(dup, Err(exact_seat));
    assert_eq!(resolve_at(&trie, &calls, 2), Some(exact_seat));
}

#[test]
fn vul_specificity_and_first_definition_wins() {
    let mut trie = AuctionTrie::new();
    let any = NodeId(1);
    let we_vul = NodeId(2);
    let both_specific = NodeId(3);

    trie.insert(
        true,
        &[bid(1, Strain::Clubs)],
        SeatCond::Any,
        VulCond::default(),
        any,
    )
    .unwrap();
    trie.insert(
        true,
        &[bid(1, Strain::Clubs)],
        SeatCond::Any,
        VulCond {
            we: Tri::Yes,
            they: Tri::Any,
        },
        we_vul,
    )
    .unwrap();
    trie.insert(
        true,
        &[bid(1, Strain::Clubs)],
        SeatCond::Any,
        VulCond {
            we: Tri::Yes,
            they: Tri::No,
        },
        both_specific,
    )
    .unwrap();

    let calls = [bid(1, Strain::Clubs)];
    let resolve_vul = |vul: RelVul| {
        let key = LookupKey {
            we_opened: true,
            calls: &calls,
            opener_pos: 1,
            vul,
        };
        trie.resolve(&key).by_depth[0]
    };

    assert_eq!(
        resolve_vul(RelVul {
            we: false,
            they: false
        }),
        Some(any)
    );
    assert_eq!(
        resolve_vul(RelVul {
            we: true,
            they: true
        }),
        Some(we_vul)
    );
    assert_eq!(
        resolve_vul(RelVul {
            we: true,
            they: false
        }),
        Some(both_specific)
    );
}

#[test]
fn leading_passes_give_opener_position_and_select_the_matching_root() {
    let mut trie = AuctionTrie::new();
    let we_open_1c = NodeId(1);
    let they_open_1c = NodeId(2);
    trie.insert(
        true,
        &[bid(1, Strain::Clubs)],
        SeatCond::Any,
        VulCond::default(),
        we_open_1c,
    )
    .unwrap();
    trie.insert(
        false,
        &[bid(1, Strain::Clubs)],
        SeatCond::Any,
        VulCond::default(),
        they_open_1c,
    )
    .unwrap();

    let dealer = Seat::North;

    // North (owner) opens directly: no leading passes, opener_pos 1, we_opened.
    let auction =
        Auction::from_calls(dealer, Vulnerability::None, [bid(1, Strain::Clubs)]).unwrap();
    let key = LookupKey::for_auction(&auction, Seat::North).unwrap();
    assert!(key.we_opened);
    assert_eq!(key.opener_pos, 1);
    assert_eq!(trie.resolve(&key).by_depth[0], Some(we_open_1c));

    // Two passes, then South (North's partner) opens: still `we_opened` for owner North,
    // opener_pos 3.
    let auction = Auction::from_calls(
        dealer,
        Vulnerability::None,
        [PASS, PASS, bid(1, Strain::Clubs)],
    )
    .unwrap();
    let key = LookupKey::for_auction(&auction, Seat::North).unwrap();
    assert!(key.we_opened);
    assert_eq!(key.opener_pos, 3);
    assert_eq!(key.calls, &[bid(1, Strain::Clubs)]);
    assert_eq!(trie.resolve(&key).by_depth[0], Some(we_open_1c));

    // Same auction, but the owner is East (opponents' side): `we_opened` is false.
    let key = LookupKey::for_auction(&auction, Seat::East).unwrap();
    assert!(!key.we_opened);
    assert_eq!(key.opener_pos, 3);
    assert_eq!(trie.resolve(&key).by_depth[0], Some(they_open_1c));
}

#[test]
fn empty_and_passed_out_auctions_have_no_lookup_key() {
    let dealer = Seat::South;
    let empty = Auction::new(dealer, Vulnerability::Both);
    assert!(LookupKey::for_auction(&empty, Seat::South).is_none());

    let passed_out =
        Auction::from_calls(dealer, Vulnerability::Both, [PASS, PASS, PASS, PASS]).unwrap();
    assert!(LookupKey::for_auction(&passed_out, Seat::South).is_none());
}

#[test]
fn partial_depth_is_the_longest_prefix_with_all_our_calls_matched() {
    let mut trie = AuctionTrie::new();
    let opening = NodeId(1);
    let rebid = NodeId(2);
    // The rebid entry only applies when we opened in first seat.
    trie.insert(
        true,
        &[bid(1, Strain::Clubs)],
        SeatCond::Any,
        VulCond::default(),
        opening,
    )
    .unwrap();
    trie.insert(
        true,
        &[bid(1, Strain::Clubs), PASS, bid(2, Strain::Clubs)],
        SeatCond::First,
        VulCond::default(),
        rebid,
    )
    .unwrap();

    let calls = [bid(1, Strain::Clubs), PASS, bid(2, Strain::Clubs)];

    // opener_pos 1: the rebid's SeatCond::First holds, so the whole auction is exact.
    let key = LookupKey {
        we_opened: true,
        calls: &calls,
        opener_pos: 1,
        vul: no_vul(),
    };
    let lookup = trie.resolve(&key);
    assert!(lookup.is_exact(&key));
    assert_eq!(lookup.end, TrieId(lookup_node_index(&trie, &key)));

    // opener_pos 2: the rebid's condition no longer holds. The prefix still extends through
    // the implicit pass (their side, which never needs an entry), so it stops at depth 2 (the
    // opening bid plus the implicit pass); the unmatched rebid stays `None` and is excluded.
    let key = LookupKey {
        we_opened: true,
        calls: &calls,
        opener_pos: 2,
        vul: no_vul(),
    };
    let lookup = trie.resolve(&key);
    assert!(!lookup.is_exact(&key));
    assert_eq!(lookup.matched_depth, 2);
    assert_eq!(lookup.by_depth[0], Some(opening));
    assert_eq!(lookup.by_depth[1], None);
    assert_eq!(lookup.by_depth[2], None);
}

/// Helper for the assertion above: resolves and returns the raw index reached, purely so the
/// test above can assert `end` without hard-coding trie internals.
fn lookup_node_index(trie: &AuctionTrie, key: &LookupKey<'_>) -> u32 {
    trie.resolve(key).end.0
}

#[test]
fn children_only_offers_exact_edges_whose_conditions_hold() {
    let mut trie = AuctionTrie::new();
    trie.insert(
        true,
        &[bid(1, Strain::Clubs)],
        SeatCond::Any,
        VulCond::default(),
        NodeId(1),
    )
    .unwrap();
    trie.insert(
        true,
        &[bid(1, Strain::Diamonds)],
        SeatCond::Any,
        VulCond::default(),
        NodeId(2),
    )
    .unwrap();
    trie.insert(
        true,
        &[bid(1, Strain::Hearts)],
        SeatCond::First,
        VulCond::default(),
        NodeId(3),
    )
    .unwrap();
    // A wildcard (opponents') edge at the root must never show up as one of our candidates.
    trie.insert_path(
        true,
        &[Edge::Class(OppClass::AnyBid)],
        SeatCond::Any,
        VulCond::default(),
        NodeId(99),
    )
    .unwrap();

    let root = TrieId(0);
    let candidates = trie.children(root, 2, no_vul());
    assert_eq!(
        candidates,
        vec![
            (bid(1, Strain::Clubs), NodeId(1)),
            (bid(1, Strain::Diamonds), NodeId(2)),
        ]
    );

    let candidates = trie.children(root, 1, no_vul());
    assert_eq!(
        candidates,
        vec![
            (bid(1, Strain::Clubs), NodeId(1)),
            (bid(1, Strain::Diamonds), NodeId(2)),
            (bid(1, Strain::Hearts), NodeId(3)),
        ]
    );
}

#[test]
fn resolve_lenient_orders_alternatives_by_substitution_count_best_first() {
    let (trie, _opening, response, _double) = sayc_like_trie();

    // The opponents actually doubled our opening instead of passing; "system on" treats that
    // as if they had passed, at the cost of one substitution.
    let calls = [bid(1, Strain::Clubs), DBL, bid(1, Strain::Hearts)];
    let key = LookupKey {
        we_opened: true,
        calls: &calls,
        opener_pos: 1,
        vul: no_vul(),
    };

    let base = trie.resolve(&key);
    assert_eq!(base.matched_depth, 1);

    let alternatives = trie.resolve_lenient(&key, 3);
    assert!(alternatives.len() >= 2);
    // Sorted ascending by substitution count ("best first").
    for pair in alternatives.windows(2) {
        assert!(pair[0].1 <= pair[1].1);
    }
    assert_eq!(alternatives[0].1, 0);
    assert_eq!(alternatives[0].0.matched_depth, 1);

    let improved = alternatives
        .iter()
        .find(|(lookup, _)| lookup.is_exact(&key))
        .expect("one substitution should make the auction exact");
    assert_eq!(improved.1, 1);
    assert_eq!(improved.0.by_depth[2], Some(response));
}

#[test]
fn resolve_lenient_is_deterministic() {
    let (trie, ..) = sayc_like_trie();
    let calls = [bid(1, Strain::Clubs), DBL, bid(1, Strain::Hearts)];
    let key = LookupKey {
        we_opened: true,
        calls: &calls,
        opener_pos: 1,
        vul: no_vul(),
    };

    let first: Vec<_> = trie
        .resolve_lenient(&key, 2)
        .into_iter()
        .map(|(lookup, n)| (lookup.matched_depth, lookup.by_depth, n))
        .collect();
    let second: Vec<_> = trie
        .resolve_lenient(&key, 2)
        .into_iter()
        .map(|(lookup, n)| (lookup.matched_depth, lookup.by_depth, n))
        .collect();
    assert_eq!(first, second);
}

#[test]
fn insert_all_exact_convenience_matches_insert_path() {
    let mut via_insert = AuctionTrie::new();
    via_insert
        .insert(
            true,
            &[bid(1, Strain::Clubs), bid(1, Strain::Hearts)],
            SeatCond::Any,
            VulCond::default(),
            NodeId(7),
        )
        .unwrap();

    let mut via_path = AuctionTrie::new();
    via_path
        .insert_path(
            true,
            &[
                Edge::Call(bid(1, Strain::Clubs)),
                Edge::Call(bid(1, Strain::Hearts)),
            ],
            SeatCond::Any,
            VulCond::default(),
            NodeId(7),
        )
        .unwrap();

    assert_eq!(via_insert.len(), via_path.len());

    let calls = [bid(1, Strain::Clubs), bid(1, Strain::Hearts)];
    let key = LookupKey {
        we_opened: true,
        calls: &calls,
        opener_pos: 1,
        vul: no_vul(),
    };
    assert_eq!(
        via_insert.resolve(&key).by_depth,
        via_path.resolve(&key).by_depth
    );
}

#[cfg(feature = "cache")]
#[test]
fn trie_survives_a_postcard_round_trip() {
    let (trie, opening, response, double_over_overcall) = sayc_like_trie();

    let encoded = postcard::to_allocvec(&trie).unwrap();
    let decoded: AuctionTrie = postcard::from_bytes(&encoded).unwrap();
    assert_eq!(decoded.len(), trie.len());

    let calls = [bid(1, Strain::Clubs), PASS, bid(1, Strain::Hearts)];
    let key = LookupKey {
        we_opened: true,
        calls: &calls,
        opener_pos: 1,
        vul: no_vul(),
    };
    assert_eq!(
        decoded
            .resolve(&key)
            .by_depth
            .into_iter()
            .collect::<Vec<_>>(),
        vec![Some(opening), None, Some(response)]
    );

    let calls = [bid(1, Strain::Clubs), bid(2, Strain::Spades), DBL];
    let key = LookupKey {
        we_opened: true,
        calls: &calls,
        opener_pos: 1,
        vul: no_vul(),
    };
    assert_eq!(
        decoded.resolve(&key).by_depth[2],
        Some(double_over_overcall)
    );
}
