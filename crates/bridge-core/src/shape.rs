//! Suit-length patterns: ordered shapes, order-free classes, and sets of shapes.
//!
//! There are `C(16, 3) = 560` ordered 13-card shapes and 39 order-free classes. Both tables are
//! built at compile time by `const fn`s and are the basis of [`ShapeSet`], the single source of
//! truth for every length constraint in the workspace.

use core::ops::RangeInclusive;

use crate::Suit;

/// Maximum high-card points a suit of the given length can hold (`MAX_HCP[len]`).
pub const MAX_HCP: [u8; 14] = [0, 4, 7, 9, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10];
/// Minimum high-card points a suit of the given length must hold (`MIN_HCP[len]`).
pub const MIN_HCP: [u8; 14] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 3, 6, 10];

/// The four suit lengths of a hand, packed as nibbles in suit order.
///
/// Clubs occupy bits `0..4`, spades bits `12..16`. A `Shape` is "ordered": 5=4=3=1 and 4=5=3=1
/// are different shapes. The order-free pattern is [`ShapeClass`].
///
/// `Display` writes the PBN order spades-first with `=` separators (`5=4=3=1`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shape(u16);

impl Shape {
    /// Builds a shape from the four suit lengths in suit order (clubs first).
    ///
    /// # Panics
    /// Debug-asserts that every length is at most 13.
    pub const fn new(clubs: u8, diamonds: u8, hearts: u8, spades: u8) -> Shape {
        debug_assert!(clubs <= 13 && diamonds <= 13 && hearts <= 13 && spades <= 13);
        Shape(
            (clubs as u16) | (diamonds as u16) << 4 | (hearts as u16) << 8 | (spades as u16) << 12,
        )
    }

    /// Builds a shape from lengths indexed by [`Suit`] (clubs first).
    pub const fn from_lens(lens: [u8; 4]) -> Shape {
        Shape::new(lens[0], lens[1], lens[2], lens[3])
    }

    /// The raw nibble pattern.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Length of `suit`.
    pub const fn len(self, suit: Suit) -> u8 {
        ((self.0 >> (4 * suit.index())) & 0xF) as u8
    }

    /// The four lengths indexed by [`Suit`] (clubs first).
    pub const fn lens(self) -> [u8; 4] {
        [
            (self.0 & 0xF) as u8,
            ((self.0 >> 4) & 0xF) as u8,
            ((self.0 >> 8) & 0xF) as u8,
            ((self.0 >> 12) & 0xF) as u8,
        ]
    }

    /// Sum of the four lengths.
    pub const fn total(self) -> u8 {
        let l = self.lens();
        l[0] + l[1] + l[2] + l[3]
    }

    /// The order-free pattern of this shape.
    pub const fn class(self) -> ShapeClass {
        ShapeClass::new(self.lens())
    }

    /// Rank of this 13-card shape in lexicographic `(clubs, diamonds, hearts)` order, `0..560`.
    ///
    /// # Panics
    /// Debug-asserts `total() == 13`.
    pub const fn index(self) -> u16 {
        debug_assert!(self.total() == 13);
        let l = self.lens();
        shape_index(l[0], l[1], l[2])
    }

    /// The 13-card shape with the given index (see [`SHAPES`]).
    pub const fn from_index(i: u16) -> Shape {
        SHAPES[i as usize]
    }

    /// Nibble-wise sum. Valid while every resulting length is at most 13.
    pub const fn add(self, other: Shape) -> Shape {
        let a = self.lens();
        let b = other.lens();
        Shape::new(a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3])
    }

    /// Nibble-wise difference, or `None` if any length of `other` exceeds the corresponding
    /// length of `self`.
    pub const fn checked_sub(self, other: Shape) -> Option<Shape> {
        let a = self.lens();
        let b = other.lens();
        if b[0] > a[0] || b[1] > a[1] || b[2] > a[2] || b[3] > a[3] {
            None
        } else {
            Some(Shape::new(
                a[0] - b[0],
                a[1] - b[1],
                a[2] - b[2],
                a[3] - b[3],
            ))
        }
    }

    /// The longest suit length.
    pub const fn longest(self) -> u8 {
        self.class().lens()[0]
    }

    /// The shortest suit length.
    pub const fn shortest(self) -> u8 {
        self.class().lens()[3]
    }

    /// `true` for 4333, 4432 and 5332 patterns.
    pub const fn is_balanced(self) -> bool {
        self.class().is_balanced()
    }
}

impl core::fmt::Debug for Shape {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(self, f)
    }
}

/// Bijection `{(c, d, h) : c + d + h <= 13} -> 0..560`, lexicographic on `(c, d, h)`.
///
/// `T3[c]` is the number of `(d, h)` pairs for all smaller club lengths; within a fixed `c`,
/// the pairs are ranked by `d` then `h`.
pub const fn shape_index(clubs: u8, diamonds: u8, hearts: u8) -> u16 {
    const T3: [u16; 14] = [
        0, 105, 196, 274, 340, 395, 440, 476, 504, 525, 540, 550, 556, 559,
    ];
    let (c, d, h) = (clubs as u16, diamonds as u16, hearts as u16);
    T3[c as usize] + d * (14 - c) - (d * d - d) / 2 + h
}

/// Every ordered 13-card shape, in lexicographic `(clubs, diamonds, hearts)` order.
///
/// `SHAPES[s.index()] == s` for every 13-card shape `s`.
pub const SHAPES: [Shape; 560] = build_shapes();

const fn build_shapes() -> [Shape; 560] {
    let mut out = [Shape(0); 560];
    let mut i = 0usize;
    let mut c = 0u8;
    while c <= 13 {
        let mut d = 0u8;
        while d <= 13 - c {
            let mut h = 0u8;
            while h <= 13 - c - d {
                let s = 13 - c - d - h;
                out[i] = Shape::new(c, d, h, s);
                i += 1;
                h += 1;
            }
            d += 1;
        }
        c += 1;
    }
    assert!(i == 560);
    out
}

/// An order-free hand pattern such as 5-4-3-1.
///
/// Stored as four nibbles sorted in descending order (longest suit in bits `12..16`), so the
/// derived `Ord` sorts patterns by their longest suit first. There are 39 classes for 13 cards
/// (see [`CLASSES`]). `Display` writes `5-4-3-1`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ShapeClass(u16);

impl ShapeClass {
    /// 4-3-3-3
    pub const C4333: ShapeClass = ShapeClass::new([4, 3, 3, 3]);
    /// 4-4-3-2
    pub const C4432: ShapeClass = ShapeClass::new([4, 4, 3, 2]);
    /// 5-3-3-2
    pub const C5332: ShapeClass = ShapeClass::new([5, 3, 3, 2]);
    /// 5-4-2-2
    pub const C5422: ShapeClass = ShapeClass::new([5, 4, 2, 2]);
    /// 6-3-2-2
    pub const C6322: ShapeClass = ShapeClass::new([6, 3, 2, 2]);
    /// 4-4-4-1
    pub const C4441: ShapeClass = ShapeClass::new([4, 4, 4, 1]);
    /// 5-4-3-1
    pub const C5431: ShapeClass = ShapeClass::new([5, 4, 3, 1]);
    /// 5-5-2-1
    pub const C5521: ShapeClass = ShapeClass::new([5, 5, 2, 1]);
    /// 6-3-3-1
    pub const C6331: ShapeClass = ShapeClass::new([6, 3, 3, 1]);
    /// 7-2-2-2
    pub const C7222: ShapeClass = ShapeClass::new([7, 2, 2, 2]);

    /// Builds a class from four lengths in any order.
    pub const fn new(lens: [u8; 4]) -> ShapeClass {
        let l = sort4_desc(lens);
        ShapeClass((l[0] as u16) << 12 | (l[1] as u16) << 8 | (l[2] as u16) << 4 | l[3] as u16)
    }

    /// The raw nibble pattern (descending).
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// The four lengths in descending order.
    pub const fn lens(self) -> [u8; 4] {
        [
            ((self.0 >> 12) & 0xF) as u8,
            ((self.0 >> 8) & 0xF) as u8,
            ((self.0 >> 4) & 0xF) as u8,
            (self.0 & 0xF) as u8,
        ]
    }

    /// Sum of the four lengths.
    pub const fn total(self) -> u8 {
        let l = self.lens();
        l[0] + l[1] + l[2] + l[3]
    }

    /// Position of this 13-card class in [`CLASSES`], `0..39`.
    ///
    /// # Panics
    /// Panics if the class does not total 13 cards (it is then not in the table).
    pub const fn index(self) -> u8 {
        let mut i = 0usize;
        while i < CLASSES.len() {
            if CLASSES[i].0 == self.0 {
                return i as u8;
            }
            i += 1;
        }
        panic!("ShapeClass::index: not a 13-card class")
    }

    /// The class with the given index (see [`CLASSES`]).
    pub const fn from_index(i: u8) -> ShapeClass {
        CLASSES[i as usize]
    }

    /// The longest suit length.
    pub const fn longest(self) -> u8 {
        self.lens()[0]
    }

    /// The shortest suit length.
    pub const fn shortest(self) -> u8 {
        self.lens()[3]
    }

    /// 4-3-3-3, 4-4-3-2 or 5-3-3-2.
    pub const fn is_balanced(self) -> bool {
        self.0 == Self::C4333.0 || self.0 == Self::C4432.0 || self.0 == Self::C5332.0
    }

    /// Balanced, or 5-4-2-2 or 6-3-2-2.
    pub const fn is_semi_balanced(self) -> bool {
        self.is_balanced() || self.0 == Self::C5422.0 || self.0 == Self::C6322.0
    }
}

impl core::fmt::Debug for ShapeClass {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(self, f)
    }
}

/// Sorts four lengths in descending order with a fixed compare-exchange network.
const fn sort4_desc(mut a: [u8; 4]) -> [u8; 4] {
    const NET: [(usize, usize); 5] = [(0, 1), (2, 3), (0, 2), (1, 3), (1, 2)];
    let mut k = 0usize;
    while k < NET.len() {
        let (i, j) = NET[k];
        if a[i] < a[j] {
            let t = a[i];
            a[i] = a[j];
            a[j] = t;
        }
        k += 1;
    }
    a
}

/// The 39 order-free 13-card patterns, in order of first appearance in [`SHAPES`].
pub const CLASSES: [ShapeClass; 39] = build_classes();

const fn build_classes() -> [ShapeClass; 39] {
    let mut out = [ShapeClass(0); 39];
    let mut n = 0usize;
    let mut i = 0usize;
    while i < SHAPES.len() {
        let c = SHAPES[i].class();
        let mut j = 0usize;
        let mut found = false;
        while j < n {
            if out[j].0 == c.0 {
                found = true;
                break;
            }
            j += 1;
        }
        if !found {
            assert!(n < 39);
            out[n] = c;
            n += 1;
        }
        i += 1;
    }
    assert!(n == 39);
    out
}

/// `CLASS_OF[i]` is the index into [`CLASSES`] of the class of `SHAPES[i]`.
pub const CLASS_OF: [u8; 560] = build_class_of();

const fn build_class_of() -> [u8; 560] {
    let mut out = [0u8; 560];
    let mut i = 0usize;
    while i < SHAPES.len() {
        out[i] = SHAPES[i].class().index();
        i += 1;
    }
    out
}

/// A set of ordered 13-card shapes: bit `i` is set when `SHAPES[i]` is a member.
///
/// Bits `560..576` are always zero; [`ShapeSet::complement`] masks against [`ShapeSet::ALL`].
/// This is the single representation of every length constraint (suit-length ranges, named
/// patterns such as "balanced", and arbitrary unions), which keeps the constraint language free
/// of a second source of truth. Projections such as [`ShapeSet::suit_len`] are summaries only.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShapeSet([u64; 9]);

impl ShapeSet {
    /// No shapes.
    pub const EMPTY: ShapeSet = ShapeSet([0; 9]);
    /// Every 13-card shape.
    pub const ALL: ShapeSet = ShapeSet([
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        (1u64 << 48) - 1,
    ]);
    /// 4-3-3-3, 4-4-3-2 and 5-3-3-2 in every suit order (28 shapes).
    pub const BALANCED: ShapeSet =
        ShapeSet::from_classes(&[ShapeClass::C4333, ShapeClass::C4432, ShapeClass::C5332]);
    /// Balanced plus 5-4-2-2 and 6-3-2-2 (52 shapes).
    pub const SEMI_BALANCED: ShapeSet = ShapeSet::BALANCED.union(ShapeSet::from_classes(&[
        ShapeClass::C5422,
        ShapeClass::C6322,
    ]));

    /// Builds a set from its words, or `None` if any bit at or above 560 is set.
    pub const fn from_words(words: [u64; 9]) -> Option<ShapeSet> {
        if words[8] >> 48 != 0 {
            None
        } else {
            Some(ShapeSet(words))
        }
    }

    /// The raw words.
    pub const fn words(self) -> [u64; 9] {
        self.0
    }

    /// Whether `shape` (which must total 13 cards) is a member.
    pub const fn contains(self, shape: Shape) -> bool {
        let i = shape.index() as usize;
        (self.0[i / 64] >> (i % 64)) & 1 == 1
    }

    /// This set plus `shape`.
    pub const fn insert(self, shape: Shape) -> ShapeSet {
        let i = shape.index() as usize;
        let mut w = self.0;
        w[i / 64] |= 1u64 << (i % 64);
        ShapeSet(w)
    }

    /// This set minus `shape`.
    pub const fn remove(self, shape: Shape) -> ShapeSet {
        let i = shape.index() as usize;
        let mut w = self.0;
        w[i / 64] &= !(1u64 << (i % 64));
        ShapeSet(w)
    }

    /// Set union.
    pub const fn union(self, other: ShapeSet) -> ShapeSet {
        let mut w = self.0;
        let mut i = 0;
        while i < 9 {
            w[i] |= other.0[i];
            i += 1;
        }
        ShapeSet(w)
    }

    /// Set intersection.
    pub const fn intersect(self, other: ShapeSet) -> ShapeSet {
        let mut w = self.0;
        let mut i = 0;
        while i < 9 {
            w[i] &= other.0[i];
            i += 1;
        }
        ShapeSet(w)
    }

    /// Set difference `self \ other`.
    pub const fn difference(self, other: ShapeSet) -> ShapeSet {
        let mut w = self.0;
        let mut i = 0;
        while i < 9 {
            w[i] &= !other.0[i];
            i += 1;
        }
        ShapeSet(w)
    }

    /// Complement within [`ShapeSet::ALL`] (never a plain bitwise NOT).
    pub const fn complement(self) -> ShapeSet {
        let mut w = self.0;
        let mut i = 0;
        while i < 9 {
            w[i] ^= ShapeSet::ALL.0[i];
            i += 1;
        }
        ShapeSet(w)
    }

    /// `true` when no shape is a member.
    pub const fn is_empty(self) -> bool {
        let mut i = 0;
        while i < 9 {
            if self.0[i] != 0 {
                return false;
            }
            i += 1;
        }
        true
    }

    /// Number of member shapes.
    pub const fn len(self) -> u16 {
        let mut n = 0u32;
        let mut i = 0;
        while i < 9 {
            n += self.0[i].count_ones();
            i += 1;
        }
        n as u16
    }

    /// `true` when `self ⊆ other`.
    pub const fn is_subset(self, other: ShapeSet) -> bool {
        let mut i = 0;
        while i < 9 {
            if self.0[i] & !other.0[i] != 0 {
                return false;
            }
            i += 1;
        }
        true
    }

    /// Every suit ordering of `class`.
    pub const fn from_class(class: ShapeClass) -> ShapeSet {
        let target = class.index();
        let mut w = [0u64; 9];
        let mut i = 0usize;
        while i < 560 {
            if CLASS_OF[i] == target {
                w[i / 64] |= 1u64 << (i % 64);
            }
            i += 1;
        }
        ShapeSet(w)
    }

    /// Union of [`ShapeSet::from_class`] over `classes`.
    pub const fn from_classes(classes: &[ShapeClass]) -> ShapeSet {
        let mut set = ShapeSet::EMPTY;
        let mut k = 0usize;
        while k < classes.len() {
            set = set.union(ShapeSet::from_class(classes[k]));
            k += 1;
        }
        set
    }

    /// Every shape whose length in `suit` lies in `lo..=hi`.
    pub const fn from_suit_len(suit: Suit, lo: u8, hi: u8) -> ShapeSet {
        let mut w = [0u64; 9];
        let mut i = 0usize;
        while i < 560 {
            let l = SHAPES[i].len(suit);
            if l >= lo && l <= hi {
                w[i / 64] |= 1u64 << (i % 64);
            }
            i += 1;
        }
        ShapeSet(w)
    }

    /// Every shape whose lengths lie in the given `(lo, hi)` ranges, indexed by [`Suit`].
    pub const fn from_suit_lens(lens: [(u8, u8); 4]) -> ShapeSet {
        ShapeSet::from_suit_len(Suit::Clubs, lens[0].0, lens[0].1)
            .intersect(ShapeSet::from_suit_len(
                Suit::Diamonds,
                lens[1].0,
                lens[1].1,
            ))
            .intersect(ShapeSet::from_suit_len(Suit::Hearts, lens[2].0, lens[2].1))
            .intersect(ShapeSet::from_suit_len(Suit::Spades, lens[3].0, lens[3].1))
    }

    /// Every shape for which `pred` holds (general, non-const builder).
    pub fn filter(pred: impl Fn(Shape) -> bool) -> ShapeSet {
        let mut w = [0u64; 9];
        for (i, s) in SHAPES.iter().enumerate() {
            if pred(*s) {
                w[i / 64] |= 1u64 << (i % 64);
            }
        }
        ShapeSet(w)
    }

    /// Member shapes in index order.
    pub fn iter(self) -> ShapeSetIter {
        ShapeSetIter {
            words: self.0,
            word: 0,
        }
    }

    /// Indices into [`SHAPES`] of the members, ascending.
    fn indices(self) -> impl Iterator<Item = usize> {
        self.0.into_iter().enumerate().flat_map(|(w, mut word)| {
            core::iter::from_fn(move || {
                if word == 0 {
                    None
                } else {
                    let tz = word.trailing_zeros() as usize;
                    word &= word - 1;
                    Some(w * 64 + tz)
                }
            })
        })
    }

    /// Projection: the range of lengths of `suit` over the members, or `None` when empty.
    ///
    /// This is an over-approximation for sets that are not products of per-suit ranges; it is
    /// a summary for pruning and display, never a substitute for [`ShapeSet::contains`].
    pub fn suit_len(self, suit: Suit) -> Option<RangeInclusive<u8>> {
        let mut lo = 13u8;
        let mut hi = 0u8;
        let mut any = false;
        for i in self.indices() {
            let l = SHAPES[i].len(suit);
            lo = lo.min(l);
            hi = hi.max(l);
            any = true;
        }
        any.then_some(lo..=hi)
    }

    /// Projection: the classes that have at least one member ordering, as a 39-bit mask
    /// indexed like [`CLASSES`].
    pub fn classes(self) -> u64 {
        self.indices()
            .fold(0u64, |mask, i| mask | (1u64 << CLASS_OF[i]))
    }

    /// `Some(ranges)` exactly when the set equals the product of its per-suit projections
    /// (i.e. it can be written as four independent length ranges).
    pub fn factor(self) -> Option<[RangeInclusive<u8>; 4]> {
        let ranges = [
            self.suit_len(Suit::Clubs)?,
            self.suit_len(Suit::Diamonds)?,
            self.suit_len(Suit::Hearts)?,
            self.suit_len(Suit::Spades)?,
        ];
        let bounds = |r: &RangeInclusive<u8>| (*r.start(), *r.end());
        let product = ShapeSet::from_suit_lens([
            bounds(&ranges[0]),
            bounds(&ranges[1]),
            bounds(&ranges[2]),
            bounds(&ranges[3]),
        ]);
        (product == self).then_some(ranges)
    }

    /// Minimum over members of the sum of `MIN_HCP` per suit (a lower bound on the HCP any
    /// member hand must hold). `0` for the empty set.
    pub fn min_hcp(self) -> u8 {
        self.indices()
            .map(|i| hcp_bound(SHAPES[i], &MIN_HCP))
            .min()
            .unwrap_or(0)
    }

    /// Maximum over members of the sum of `MAX_HCP` per suit (an upper bound on the HCP any
    /// member hand can hold). `0` for the empty set.
    pub fn max_hcp(self) -> u8 {
        self.indices()
            .map(|i| hcp_bound(SHAPES[i], &MAX_HCP))
            .max()
            .unwrap_or(0)
    }
}

/// Sum over the four suits of `table[len]`.
fn hcp_bound(shape: Shape, table: &[u8; 14]) -> u8 {
    shape.lens().iter().map(|&l| table[l as usize]).sum()
}

/// Iterator over the members of a [`ShapeSet`] in index order.
#[derive(Clone, Debug)]
pub struct ShapeSetIter {
    words: [u64; 9],
    word: usize,
}

impl Iterator for ShapeSetIter {
    type Item = Shape;

    #[inline]
    fn next(&mut self) -> Option<Shape> {
        while self.word < 9 {
            let w = self.words[self.word];
            if w == 0 {
                self.word += 1;
                continue;
            }
            let tz = w.trailing_zeros() as usize;
            self.words[self.word] = w & (w - 1);
            return Some(SHAPES[self.word * 64 + tz]);
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.words[self.word.min(9)..]
            .iter()
            .map(|w| w.count_ones() as usize)
            .sum();
        (n, Some(n))
    }
}

impl ExactSizeIterator for ShapeSetIter {}
impl core::iter::FusedIterator for ShapeSetIter {}

impl IntoIterator for ShapeSet {
    type Item = Shape;
    type IntoIter = ShapeSetIter;

    fn into_iter(self) -> ShapeSetIter {
        self.iter()
    }
}

impl core::ops::BitOr for ShapeSet {
    type Output = ShapeSet;
    fn bitor(self, rhs: ShapeSet) -> ShapeSet {
        self.union(rhs)
    }
}

impl core::ops::BitAnd for ShapeSet {
    type Output = ShapeSet;
    fn bitand(self, rhs: ShapeSet) -> ShapeSet {
        self.intersect(rhs)
    }
}

impl core::ops::Sub for ShapeSet {
    type Output = ShapeSet;
    fn sub(self, rhs: ShapeSet) -> ShapeSet {
        self.difference(rhs)
    }
}

impl core::ops::Not for ShapeSet {
    type Output = ShapeSet;
    fn not(self) -> ShapeSet {
        self.complement()
    }
}

impl core::fmt::Debug for ShapeSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ShapeSet({} shapes)", self.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_have_expected_sizes() {
        assert_eq!(SHAPES.len(), 560);
        assert_eq!(CLASSES.len(), 39);
        assert_eq!(ShapeSet::ALL.len(), 560);
        assert_eq!(ShapeSet::BALANCED.len(), 28);
        assert_eq!(ShapeSet::SEMI_BALANCED.len(), 52);
    }

    #[test]
    fn shape_index_is_a_bijection() {
        for (i, s) in SHAPES.iter().enumerate() {
            assert_eq!(s.total(), 13);
            assert_eq!(s.index() as usize, i, "{s:?}");
            assert_eq!(CLASS_OF[i], s.class().index());
        }
    }

    #[test]
    fn hcp_bounds_per_length() {
        assert_eq!(MAX_HCP[4], 10);
        assert_eq!(MIN_HCP[10], 1);
        assert_eq!(MIN_HCP[13], 10);
    }

    /// `MAX_HCP` / `MIN_HCP` agree with a brute force over all 8192 holdings.
    #[test]
    fn hcp_bounds_match_brute_force() {
        let mut max = [0u8; 14];
        let mut min = [u8::MAX; 14];
        for bits in 0u16..8192 {
            let len = bits.count_ones() as usize;
            let hcp: u8 = (0..13u8)
                .filter(|r| (bits >> r) & 1 == 1)
                .map(|r| crate::Rank::from_index(r).hcp())
                .sum();
            max[len] = max[len].max(hcp);
            min[len] = min[len].min(hcp);
        }
        assert_eq!(max, MAX_HCP);
        assert_eq!(min, MIN_HCP);
    }

    #[test]
    fn projections() {
        for suit in Suit::ALL {
            assert_eq!(ShapeSet::BALANCED.suit_len(suit), Some(2..=5));
            assert_eq!(ShapeSet::ALL.suit_len(suit), Some(0..=13));
            assert_eq!(ShapeSet::EMPTY.suit_len(suit), None);
        }
        assert_eq!(ShapeSet::BALANCED.classes().count_ones(), 3);
        assert_eq!(ShapeSet::ALL.classes(), (1u64 << 39) - 1);
        assert_eq!(ShapeSet::EMPTY.classes(), 0);

        // The 13-card total tightens the loose ranges: D <= 13 - 2 - 4 - 3, S <= 13 - 2 - 4.
        let product = ShapeSet::from_suit_lens([(2, 5), (0, 13), (4, 4), (3, 13)]);
        let ranges = product.factor().expect("a product factors");
        assert_eq!(ranges, [2..=5, 0..=4, 4..=4, 3..=7]);
        let tight = ShapeSet::from_suit_lens([(2, 5), (0, 4), (4, 4), (3, 7)]);
        assert_eq!(product, tight);
        assert_eq!(product.factor(), Some([2..=5, 0..=4, 4..=4, 3..=7]));
        assert_eq!(ShapeSet::BALANCED.factor(), None);
        assert_eq!(ShapeSet::EMPTY.factor(), None);
        assert_eq!(
            ShapeSet::ALL.factor(),
            Some([0..=13, 0..=13, 0..=13, 0..=13])
        );

        assert_eq!(ShapeSet::ALL.max_hcp(), 37);
        assert_eq!(ShapeSet::ALL.min_hcp(), 0);
        let six_six = ShapeSet::from_suit_lens([(6, 6), (6, 6), (0, 13), (0, 13)]);
        assert_eq!(six_six.len(), 2);
        assert_eq!(six_six.max_hcp(), 24);
        let thirteen = ShapeSet::EMPTY.insert(Shape::new(13, 0, 0, 0));
        assert_eq!(thirteen.min_hcp(), 10);
        assert_eq!(thirteen.max_hcp(), 10);
        assert_eq!(ShapeSet::EMPTY.min_hcp(), 0);
        assert_eq!(ShapeSet::EMPTY.max_hcp(), 0);
    }

    #[test]
    fn iteration_matches_len_and_contains() {
        for set in [
            ShapeSet::EMPTY,
            ShapeSet::ALL,
            ShapeSet::BALANCED,
            ShapeSet::SEMI_BALANCED,
            ShapeSet::from_suit_len(Suit::Spades, 5, 13),
        ] {
            let members: Vec<Shape> = set.iter().collect();
            assert_eq!(members.len(), set.len() as usize);
            assert_eq!(set.iter().len(), set.len() as usize);
            let mut prev: Option<u16> = None;
            for s in &members {
                assert!(set.contains(*s));
                assert!(prev.is_none_or(|p| p < s.index()));
                prev = Some(s.index());
            }
            assert_eq!(set.complement().union(set), ShapeSet::ALL);
            assert!(set.complement().intersect(set).is_empty());
        }
        assert_eq!(ShapeSet::ALL.into_iter().count(), 560);
    }
}
