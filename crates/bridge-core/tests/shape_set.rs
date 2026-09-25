//! `ShapeSet` projections and iteration, plus `Holding` sub-mask enumeration.

use bridge_core::{CLASSES, Holding, SHAPES, Shape, ShapeClass, ShapeSet, Suit};
use proptest::prelude::*;

#[test]
fn balanced_projections() {
    for suit in Suit::ALL {
        assert_eq!(ShapeSet::BALANCED.suit_len(suit), Some(2..=5));
        assert_eq!(ShapeSet::SEMI_BALANCED.suit_len(suit), Some(2..=6));
    }
    assert_eq!(ShapeSet::BALANCED.factor(), None);
    let classes = ShapeSet::BALANCED.classes();
    assert_eq!(classes.count_ones(), 3);
    for c in [ShapeClass::C4333, ShapeClass::C4432, ShapeClass::C5332] {
        assert_ne!(classes & (1 << c.index()), 0);
    }
    assert_eq!(ShapeSet::BALANCED.min_hcp(), 0);
    assert_eq!(ShapeSet::BALANCED.max_hcp(), 37);
}

#[test]
fn factor_of_products() {
    let lens = [(2u8, 5u8), (3, 3), (0, 13), (5, 13)];
    let set = ShapeSet::from_suit_lens(lens);
    let ranges = set.factor().expect("a product factors");
    for (r, (lo, hi)) in ranges.iter().zip(lens) {
        assert!(lo <= *r.start() && *r.end() <= hi);
        assert!(r.start() <= r.end());
    }
    let bounds = ranges.clone().map(|r| (*r.start(), *r.end()));
    assert_eq!(ShapeSet::from_suit_lens(bounds), set);
    for suit in Suit::ALL {
        assert_eq!(
            set.suit_len(suit),
            Some(ranges[suit.index() as usize].clone())
        );
    }
    // A single shape is a product of four points.
    let one = ShapeSet::EMPTY.insert(Shape::new(1, 3, 4, 5));
    assert_eq!(one.factor(), Some([1..=1, 3..=3, 4..=4, 5..=5]));
    // "Either 5-5" is not a product.
    let five_five = ShapeSet::from_suit_lens([(0, 13), (0, 13), (5, 5), (5, 5)])
        .union(ShapeSet::from_suit_lens([(5, 5), (5, 5), (0, 13), (0, 13)]));
    assert_eq!(five_five.factor(), None);
    assert_eq!(five_five.suit_len(Suit::Spades), Some(0..=5));
}

#[test]
fn hcp_bounds_detect_contradictions() {
    // 6-6 in the minors leaves room for at most 24 HCP; 13 cards in one suit force 10.
    let six_six = ShapeSet::from_suit_lens([(6, 6), (6, 6), (0, 13), (0, 13)]);
    assert_eq!(six_six.max_hcp(), 24);
    assert_eq!(six_six.min_hcp(), 0);
    let solid = ShapeSet::EMPTY.insert(Shape::new(0, 0, 0, 13));
    assert_eq!(solid.min_hcp(), 10);
    assert_eq!(solid.max_hcp(), 10);
    let long = ShapeSet::from_suit_len(Suit::Hearts, 10, 13);
    assert_eq!(long.min_hcp(), 1);
    assert_eq!(ShapeSet::ALL.max_hcp(), 37);
    assert_eq!(ShapeSet::EMPTY.max_hcp(), 0);
    assert_eq!(ShapeSet::EMPTY.min_hcp(), 0);
}

#[test]
fn iteration_count_equals_len() {
    for class in CLASSES {
        let set = ShapeSet::from_class(class);
        assert_eq!(set.iter().count(), set.len() as usize);
        assert!(set.iter().all(|s| s.class() == class));
        assert_eq!(set.classes(), 1 << class.index());
    }
    for (lo, hi) in [(0, 0), (5, 13), (4, 4), (7, 6)] {
        let set = ShapeSet::from_suit_len(Suit::Diamonds, lo, hi);
        assert_eq!(set.iter().count(), set.len() as usize);
        assert!(
            set.iter()
                .all(|s| (lo..=hi).contains(&s.len(Suit::Diamonds)))
        );
    }
    let filtered = ShapeSet::filter(|s| s.longest() >= 7);
    assert_eq!(filtered.iter().count(), filtered.len() as usize);
    assert!(filtered.iter().all(|s| s.longest() >= 7));
    assert_eq!(ShapeSet::ALL.iter().collect::<Vec<_>>(), SHAPES.to_vec());
}

#[test]
fn holding_submasks() {
    let akq: Holding = "AKQ".parse().unwrap();
    let subs: Vec<Holding> = akq.submasks().collect();
    assert_eq!(subs.len(), 8);
    assert_eq!(subs[0], akq);
    assert_eq!(subs[7], Holding::EMPTY);
    assert!(subs.windows(2).all(|w| w[0].bits() > w[1].bits()));
    assert!(subs.iter().all(|s| s.is_subset(akq)));
    assert_eq!(Holding::EMPTY.submasks().count(), 1);
    assert_eq!(Holding::FULL.submasks().count(), 8192);
}

proptest! {
    #[test]
    fn random_sets_are_consistent(words in prop::array::uniform8(any::<u64>()), last in any::<u64>()) {
        let mut w = [0u64; 9];
        w[..8].copy_from_slice(&words);
        w[8] = last & ((1 << 48) - 1);
        let set = ShapeSet::from_words(w).unwrap();
        let members: Vec<Shape> = set.iter().collect();
        prop_assert_eq!(members.len(), set.len() as usize);
        prop_assert_eq!(set.iter().len(), set.len() as usize);
        prop_assert!(members.windows(2).all(|p| p[0].index() < p[1].index()));
        for suit in Suit::ALL {
            match set.suit_len(suit) {
                None => prop_assert!(set.is_empty()),
                Some(r) => {
                    prop_assert!(members.iter().all(|s| r.contains(&s.len(suit))));
                    prop_assert!(members.iter().any(|s| s.len(suit) == *r.start()));
                    prop_assert!(members.iter().any(|s| s.len(suit) == *r.end()));
                }
            }
        }
        if let Some(r) = set.factor() {
            let bounds = r.map(|r| (*r.start(), *r.end()));
            prop_assert_eq!(ShapeSet::from_suit_lens(bounds), set);
        }
        prop_assert!(set.min_hcp() <= set.max_hcp());
        prop_assert!(set.max_hcp() <= 37);
        let classes = set.classes();
        prop_assert_eq!(classes >> 39, 0);
        prop_assert!(members.iter().all(|s| classes & (1 << s.class().index()) != 0));
    }
}
