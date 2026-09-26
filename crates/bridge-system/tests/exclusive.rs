//! The exclusive-region index (docs/design/15-phase4-plan.md D19): rank order, `Lookup.parent`,
//! and the defining property of the pieces -- a hand lies in the pieces of call `c` exactly when
//! the first satisfied member (in `rank_cmp` order) has call `c`, and in the complement exactly
//! when no member is satisfied.

use std::cmp::Ordering;

use bridge_core::{Bid, Call, Card, Hand, Strain};
use bridge_system::exclusive::{ExclusiveIndex, class_conditions, condition_class, rank_cmp};
use bridge_system::lexer::MemLoader;
use bridge_system::trie::{LookupKey, RelVul, TrieId};
use bridge_system::{CompileOptions, SystemIR};

const SOURCE: &str = "#+TITLE: exclusive test
#+TIEBREAK: row-order

1S = {prio:10} 12--21 hcp, 5+!s
1H = {prio:10} 12--21 hcp, 5+!h
1N = {prio:20} 15--17 hcp, bal
1C = 12--21 hcp

1C-
1D = 6+ hcp
1H = 6+ hcp, 4+!h
";

fn compile() -> SystemIR {
    let opts = CompileOptions {
        coverage_samples: 0,
        ..CompileOptions::default()
    };
    let (ir, _) = bridge_system::compile("inline.bml", SOURCE, &MemLoader::default(), &opts);
    ir
}

fn bid(level: u8, strain: Strain) -> Call {
    Call::Bid(Bid::new(level, strain).unwrap())
}

/// A deterministic pseudo-random 13-card hand (splitmix64 Fisher-Yates).
fn random_hand(seed: &mut u64) -> Hand {
    let mut next = || {
        *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut cards: Vec<u8> = (0..52).collect();
    let mut hand = Hand::EMPTY;
    for i in 0..13 {
        let j = i + (next() % (52 - i as u64)) as usize;
        cards.swap(i, j);
        hand = hand.with(Card::from_index(cards[i]).expect("index < 52"));
    }
    hand
}

const VUL: RelVul = RelVul {
    we: false,
    they: false,
};

#[test]
fn rank_cmp_orders_priority_then_row_then_call() {
    let ir = compile();
    let mut children = ir.index.children(TrieId(0), 1, VUL);
    children.sort_by(|a, b| rank_cmp(&ir, *a, *b));
    let calls: Vec<Call> = children.iter().map(|(c, _)| *c).collect();
    // 1N (prio 20), then the prio-10 majors in row order (1S was written first), then 1C.
    assert_eq!(
        calls,
        vec![
            bid(1, Strain::NoTrump),
            bid(1, Strain::Spades),
            bid(1, Strain::Hearts),
            bid(1, Strain::Clubs),
        ]
    );
    for w in children.windows(2) {
        assert_eq!(rank_cmp(&ir, w[0], w[1]), Ordering::Less);
        assert_eq!(rank_cmp(&ir, w[1], w[0]), Ordering::Greater);
    }
    assert_eq!(rank_cmp(&ir, children[0], children[0]), Ordering::Equal);
}

#[test]
fn lookup_parent_is_the_position_of_the_last_matched_call() {
    let ir = compile();
    let calls = [bid(1, Strain::Clubs), Call::Pass, bid(1, Strain::Hearts)];
    let key = LookupKey {
        we_opened: true,
        calls: &calls,
        opener_pos: 1,
        vul: VUL,
    };
    let full = ir.index.resolve(&key);
    assert_eq!(full.matched_depth, 3);
    let short = ir.index.resolve(&LookupKey {
        calls: &calls[..2],
        ..key
    });
    assert_eq!(full.parent, short.end);
    // The siblings of 1H are the responses to 1C.
    let siblings = ir.index.children(full.parent, 1, VUL);
    assert!(siblings.iter().any(|(c, _)| *c == bid(1, Strain::Hearts)));
    assert!(siblings.iter().any(|(c, _)| *c == bid(1, Strain::Diamonds)));
    // Nothing matched: parent == end == root.
    let none = ir.index.resolve(&LookupKey {
        calls: &[bid(7, Strain::NoTrump)],
        ..key
    });
    assert_eq!(none.matched_depth, 0);
    assert_eq!(none.parent, none.end);
}

#[test]
fn pieces_are_the_first_satisfied_member_regions() {
    let ir = compile();
    let index = ir.exclusive();
    assert!(!index.is_empty());
    let parents = [
        TrieId(0),
        ir.index
            .resolve(&LookupKey {
                we_opened: true,
                calls: &[bid(1, Strain::Clubs), Call::Pass],
                opener_pos: 1,
                vul: VUL,
            })
            .end,
    ];
    let mut seed = 0xE5C1_0001u64;
    for parent in parents {
        let group = index
            .group(parent, condition_class(1, VUL))
            .expect("a group at every position with candidates");
        assert!(
            group
                .members
                .windows(2)
                .all(|w| rank_cmp(&ir, w[0], w[1]) == Ordering::Less)
        );
        for _ in 0..3_000 {
            let hand = random_hand(&mut seed);
            let first = group
                .members
                .iter()
                .find(|(_, node)| ir.node(*node).constraint.satisfies(hand))
                .map(|(call, _)| *call);
            let mut containing = 0;
            for (call, pieces) in &group.per_call {
                let hits = pieces
                    .iter()
                    .filter(|p| p.constraint.satisfies(hand))
                    .count();
                assert!(hits <= 1, "pieces of {call} overlap");
                assert_eq!(
                    hits == 1,
                    first == Some(*call),
                    "call {call}, hand {hand:?}"
                );
                containing += hits;
            }
            assert_eq!(group.complement.satisfies(hand), first.is_none());
            assert!(containing <= 1);
        }
    }
}

#[test]
fn a_covered_member_is_shadowed() {
    // 1C-1D (6+ hcp) ranks above 1C-1H (6+ hcp, 4+ hearts) by row order at equal priority and
    // covers it, so 1H is never chosen there.
    let ir = compile();
    let parent = ir
        .index
        .resolve(&LookupKey {
            we_opened: true,
            calls: &[bid(1, Strain::Clubs), Call::Pass],
            opener_pos: 1,
            vul: VUL,
        })
        .end;
    let group = ir.exclusive().group_for(parent, 1, VUL).unwrap();
    assert!(group.is_shadowed(bid(1, Strain::Hearts)));
    assert!(!group.is_shadowed(bid(1, Strain::Diamonds)));
    assert_eq!(group.rank_of(bid(1, Strain::Diamonds)), Some(0));
    assert_eq!(
        ir.exclusive()
            .pieces(parent, 1, VUL, bid(1, Strain::Hearts))
            .map(<[_]>::len),
        Some(0)
    );
    assert!(
        ir.exclusive()
            .pieces(parent, 1, VUL, bid(2, Strain::Hearts))
            .is_none()
    );
}

#[cfg(feature = "cache")]
#[test]
fn the_exclusive_index_does_not_change_the_serialised_ir() {
    let ir = compile();
    let before = postcard::to_allocvec(&ir).expect("postcard encode");
    let _ = ir.exclusive();
    assert!(ir.exclusive_cell.get().is_some());
    let after = postcard::to_allocvec(&ir).expect("postcard encode");
    assert_eq!(before, after);
    let back: SystemIR = postcard::from_bytes(&after).expect("postcard decode");
    assert!(back.exclusive_cell.get().is_none());
    assert_eq!(back.exclusive().group_count(), ir.exclusive().group_count());
}

fn compile_sayc() -> SystemIR {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../systems/sayc/sayc.bml");
    let text = std::fs::read_to_string(&path).expect("systems/sayc/sayc.bml");
    let opts = CompileOptions {
        coverage_samples: 0,
        ..CompileOptions::default()
    };
    let (ir, _) = bridge_system::compile(
        &path.to_string_lossy(),
        &text,
        &bridge_system::lexer::FsLoader,
        &opts,
    );
    ir
}

/// The same defining property over every group of the real SAYC system (a few random hands per
/// group), plus the tree-fallback share.
#[test]
fn sayc_groups_satisfy_the_first_satisfied_member_property() {
    let ir = compile_sayc();
    assert!(
        ir.exclusive_cell.get().is_some(),
        "compile() builds the index eagerly"
    );
    let started = std::time::Instant::now();
    let index = ExclusiveIndex::build(&ir);
    let index = &index;
    eprintln!(
        "SAYC exclusive index: {} groups, {} keys, built in {:?}",
        index.group_count(),
        index.key_count(),
        started.elapsed()
    );
    assert!(index.group_count() > 100);
    let mut seed = 0xE5C1_0002u64;
    let (mut pieces, mut trees) = (0usize, 0usize);
    for group in index.groups() {
        for (_, ps) in &group.per_call {
            pieces += ps.len();
            trees += ps.iter().filter(|p| !p.flat).count();
        }
        for _ in 0..20 {
            let hand = random_hand(&mut seed);
            let first = group
                .members
                .iter()
                .find(|(_, node)| ir.node(*node).constraint.satisfies(hand))
                .map(|(call, _)| *call);
            for (call, ps) in &group.per_call {
                let hits = ps.iter().filter(|p| p.constraint.satisfies(hand)).count();
                assert!(hits <= 1, "pieces of {call} overlap");
                assert_eq!(hits == 1, first == Some(*call));
            }
            assert_eq!(group.complement.satisfies(hand), first.is_none());
        }
    }
    eprintln!("SAYC exclusive pieces: {pieces}, tree fallback: {trees}");
}

/// Checks `n` random `(position, hand)` pairs of SAYC against the defining property, computed
/// independently from the trie: a key `(parent, class)` is drawn uniformly from the index, its
/// sibling list is recomputed with `AuctionTrie::children` and sorted by `rank_cmp`, and for a
/// uniform random hand
/// - the hand lies in a piece of call `c` iff the first satisfied sibling has call `c`,
/// - it lies in the complement iff no sibling is satisfied,
/// - it lies in at most one piece of the whole group (so the pieces of one node, and of one
///   call, are pairwise disjoint).
///
/// Returns `(pairs checked, pairs whose hand was in some piece)`.
fn check_sayc_membership(n: usize, seed: u64) -> (usize, usize) {
    let ir = compile_sayc();
    let index = ir.exclusive();
    let keys: Vec<_> = index.entries().collect();
    assert!(!keys.is_empty());
    let mut seed = seed;
    let mut next = |bound: usize| -> usize {
        seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % bound as u64) as usize
    };
    let mut hand_seed = 0xE5C1_0003u64;
    let (mut mismatches, mut overlaps, mut complement_mismatches, mut inside) = (0, 0, 0, 0);
    for _ in 0..n {
        let (parent, class, group) = keys[next(keys.len())];
        let (opener_pos, vul) = class_conditions(class);
        let mut siblings = ir.index.children(parent, opener_pos, vul);
        siblings.sort_by(|a, b| rank_cmp(&ir, *a, *b));
        assert_eq!(
            siblings, group.members,
            "group members == ranked trie children"
        );
        let hand = random_hand(&mut hand_seed);
        let first = siblings
            .iter()
            .find(|(_, node)| ir.node(*node).constraint.satisfies(hand))
            .map(|(call, _)| *call);
        let mut hits_total = 0;
        for (call, pieces) in &group.per_call {
            let hits = pieces
                .iter()
                .filter(|p| p.constraint.satisfies(hand))
                .count();
            hits_total += hits;
            if (hits >= 1) != (first == Some(*call)) {
                mismatches += 1;
            }
        }
        if hits_total > 1 {
            overlaps += 1;
        }
        if hits_total > 0 {
            inside += 1;
        }
        if group.complement.satisfies(hand) != first.is_none() {
            complement_mismatches += 1;
        }
    }
    assert_eq!(mismatches, 0, "X_c membership mismatches");
    assert_eq!(complement_mismatches, 0, "complement mismatches");
    assert_eq!(overlaps, 0, "hands in two pieces of one group");
    (n, inside)
}

/// Default-suite version of the lane-S acceptance check (1e4 pairs).
#[test]
fn sayc_exclusive_membership_random_positions() {
    let (n, inside) = check_sayc_membership(10_000, 0x5EED_0001);
    eprintln!("SAYC exclusive membership: {n} pairs, {inside} inside some piece, 0 mismatches");
}

/// The acceptance-size version: 1e5 random `(position, hand)` pairs (release:
/// `cargo test -p bridge-system --release --test exclusive -- --ignored`).
#[test]
#[ignore = "1e5 pairs; run in release with --ignored"]
fn sayc_exclusive_membership_random_positions_1e5() {
    let started = std::time::Instant::now();
    let (n, inside) = check_sayc_membership(100_000, 0x5EED_0002);
    eprintln!(
        "SAYC exclusive membership: {n} pairs, {inside} inside some piece, 0 mismatches, {:?}",
        started.elapsed()
    );
}

/// The index statistics and the build/compile cost on SAYC (reported; the tree fallback share
/// is asserted to stay <= 1% of pieces).
#[test]
fn sayc_exclusive_index_stats() {
    let started = std::time::Instant::now();
    let ir = compile_sayc();
    let compile = started.elapsed();
    let started = std::time::Instant::now();
    let rebuilt = ExclusiveIndex::build(&ir);
    let build = started.elapsed();
    let stats = rebuilt.stats(&ir);
    assert_eq!(stats, ir.exclusive().stats(&ir));
    eprintln!("SAYC compile (with index) {compile:?}, index build {build:?}: {stats:?}");
    assert!(
        stats.tree_pieces * 100 <= stats.pieces,
        "tree fallback > 1%"
    );
    #[cfg(feature = "cache")]
    {
        let bytes = postcard::to_allocvec(&ir).expect("postcard encode");
        eprintln!(
            "SAYC postcard IR: {} bytes (IR_FORMAT {})",
            bytes.len(),
            bridge_system::IR_FORMAT
        );
    }
}
