//! Per-suit enumeration of candidate holdings, bucketed by `(length, key)`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use bridge_core::Holding;

/// Number of bits the HCP component of a packed key uses (`0..=37` fits in 6 bits, and a single
/// suit's HCP is at most 10, well inside that).
pub(crate) const HCP_BITS: u32 = 6;
/// Number of bits the optional additive-feature component uses.
pub(crate) const X_BITS: u32 = 5;
/// Number of distinct keys: HCP `0..=37` in the low 6 bits, one extra feature in bits `6..`.
pub(crate) const KEYS: usize = 64 * 32;

/// The largest combined HCP of a two-suit pair (`2 × 10`).
pub(crate) const PAIR_HCP_MAX: u8 = 20;
/// The largest combined additive-feature value of a two-suit pair (`2 × 13`, a generous bound
/// that covers every additive feature: each is a per-suit popcount or a table value bounded by
/// the suit's length).
pub(crate) const PAIR_X_MAX: u8 = 26;

/// Packs an HCP component and an additive-feature component into one key.
pub(crate) fn pack_key(hcp: u8, x: u8) -> u16 {
    debug_assert!((hcp as u32) < (1 << HCP_BITS));
    debug_assert!((x as u32) < (1 << X_BITS));
    u16::from(hcp) | (u16::from(x) << HCP_BITS)
}

/// Inverse of [`pack_key`]: `(hcp, x)`.
pub(crate) fn unpack_key(key: u16) -> (u8, u8) {
    ((key & ((1 << HCP_BITS) - 1)) as u8, (key >> HCP_BITS) as u8)
}

/// The candidate holdings of one suit for one prepared term.
///
/// `holdings` is sorted by `(len, key)` (counting sort). Unlike `counts`/`bucket`, this sort is
/// only ever over the `(len, key)` pairs that actually occur (D9: `pool.submasks()` is at most
/// `2^13`, almost always far fewer than the `14 * KEYS` a dense table over the whole packed-key
/// domain would force every build to zero and scan regardless of how small `pool` is). `offsets`
/// gives each `counts[len]` entry's start index into `holdings`, parallel to `counts[len].0`
/// (both sorted ascending by key); `bucket` locates a key with a binary search over `counts[len]`
/// instead of an O(1) dense-array lookup. Every holding already includes the suit's fixed cards.
pub(crate) struct SuitTable {
    pub(crate) holdings: Vec<u16>,
    /// `offsets[len][i]` is `holdings`' start index for `counts[len].0[i]` (same length, same
    /// order).
    offsets: [Vec<u32>; 14],
    /// `counts[len]` = sparse `(key, n)` list with `n > 0`, sorted ascending by key.
    pub(crate) counts: [SparseVec; 14],
}

/// A sparse count vector.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SparseVec(pub(crate) Vec<(u16, u64)>);

/// The convolution of two suits' count vectors for one `(len_a, len_b)` pair, with prefix sums
/// over the `(hcp, x)` axes so that any box (HCP window × feature window) is a four-lookup sum.
///
/// Built in [`super::term`] (which knows how to combine two [`SuitTable`]s); the type itself is
/// just data plus the box-sum query. `width` is the size of the `x` axis actually built: `1` when
/// the term carries no additive feature (every entry's `x` is 0, so a K=1 term's convolution has
/// nothing to convolve on that axis) instead of always the generous `PAIR_X_MAX + 1` upper bound
/// a K=2 term (one additive feature) might need.
pub(crate) struct PairConv {
    pub(crate) p: SparseVec,
    pub(crate) prefix: Box<[u64]>,
    pub(crate) width: usize,
}

impl SuitTable {
    /// Enumerates `sub ⊆ pool`, folds in `fixed`, applies `filter`, and bucket-sorts by
    /// `(len(H), key(H))` where `H = sub ∪ fixed` (D3: the fixed cards are composed in here, so
    /// every downstream quantity is already exact for the original 13-card hand).
    pub(crate) fn build(
        pool: Holding,
        fixed: Holding,
        filter: &dyn Fn(Holding) -> bool,
        key: &dyn Fn(Holding) -> u16,
    ) -> SuitTable {
        // Pass 1: histogram of (len, key) -> count, one hash map per length rather than a dense
        // `14 * KEYS` array: this costs O(pool.submasks()), never O(14 * KEYS) regardless of how
        // small `pool` is (`KEYS` is a generous upper bound on the packed-key domain - `0..=10`
        // HCP times up to 14 additive-feature values - almost never anywhere near saturated).
        let mut hist: [HashMap<u16, u32>; 14] = core::array::from_fn(|_| HashMap::new());
        for sub in pool.submasks() {
            let h = sub.union(fixed);
            if !filter(h) {
                continue;
            }
            let len = h.len() as usize;
            let k = key(h);
            debug_assert!((k as usize) < KEYS, "key() must stay within 0..KEYS");
            *hist[len].entry(k).or_insert(0) += 1;
        }

        // Sort each length's observed keys ascending (matching the dense implementation's
        // row-major (len, key) order, since an unobserved key contributed zero width there too)
        // and lay out each bucket's start offset in that same order.
        let mut offsets: [Vec<u32>; 14] = core::array::from_fn(|_| Vec::new());
        let mut counts: [SparseVec; 14] = core::array::from_fn(|_| SparseVec::default());
        let mut total = 0u32;
        for len in 0..14 {
            let mut entries: Vec<(u16, u32)> = hist[len].drain().collect();
            entries.sort_unstable_by_key(|&(k, _)| k);
            let mut offs = Vec::with_capacity(entries.len());
            for &(_, n) in &entries {
                offs.push(total);
                total += n;
            }
            offsets[len] = offs;
            counts[len] = SparseVec(
                entries
                    .into_iter()
                    .map(|(k, n)| (k, u64::from(n)))
                    .collect(),
            );
        }

        // Pass 2: place each holding into its bucket (a mutable copy of `offsets` as the cursor,
        // located via the same binary search `bucket` uses).
        let mut cursor = offsets.clone();
        let mut holdings = vec![0u16; total as usize];
        for sub in pool.submasks() {
            let h = sub.union(fixed);
            if !filter(h) {
                continue;
            }
            let len = h.len() as usize;
            let k = key(h);
            let idx = counts[len]
                .0
                .binary_search_by_key(&k, |&(kk, _)| kk)
                .expect("every key placed here was counted in pass 1");
            let slot = &mut cursor[len][idx];
            holdings[*slot as usize] = h.bits();
            *slot += 1;
        }

        SuitTable {
            holdings,
            offsets,
            counts,
        }
    }

    /// The bucket for `(len, key)`, or an empty slice when that key was never observed.
    pub(crate) fn bucket(&self, len: u8, key: u16) -> &[u16] {
        let len = len as usize;
        match self.counts[len].0.binary_search_by_key(&key, |&(k, _)| k) {
            Ok(idx) => {
                let start = self.offsets[len][idx] as usize;
                let end = start + self.counts[len].0[idx].1 as usize;
                &self.holdings[start..end]
            }
            Err(_) => &[],
        }
    }
}

/// The shared table for the common case: a full 13-card suit pool, no fixed cards, no per-suit
/// filter and no additive feature (key = HCP alone). Built once and reused by every term that
/// needs it (§6.3, §9: "共有 `FULL_SUIT`").
static FULL_SUIT: LazyLock<Arc<SuitTable>> = LazyLock::new(|| {
    Arc::new(SuitTable::build(
        Holding::FULL,
        Holding::EMPTY,
        &|_| true,
        &|h| u16::from(bridge_eval::holding_hcp(h)),
    ))
});

/// A cheap `Arc` clone of the shared full-suit table.
pub(crate) fn full_suit() -> Arc<SuitTable> {
    Arc::clone(&FULL_SUIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip() {
        for hcp in 0..10u8 {
            for x in 0..13u8 {
                assert_eq!(unpack_key(pack_key(hcp, x)), (hcp, x));
            }
        }
    }

    /// A reference re-implementation of the pre-optimization dense `SuitTable::build` (a
    /// `14 * KEYS`-sized histogram and prefix-sum scan over the *whole* packed-key domain,
    /// instead of only the keys `pool.submasks()` can actually reach). Used only to check the
    /// optimized `build` produces bit-identical `holdings`/`counts`: the fix must change how fast
    /// a table is built, never what a bucket contains or in what order (§9's `pick_holding`
    /// selects uniformly within a bucket by index, so the bucket's element order is observable
    /// through which hand a given RNG draw returns).
    fn build_dense_reference(
        pool: Holding,
        fixed: Holding,
        filter: &dyn Fn(Holding) -> bool,
        key: &dyn Fn(Holding) -> u16,
    ) -> (Vec<u16>, [SparseVec; 14]) {
        let mut hist = vec![0u32; 14 * KEYS];
        for sub in pool.submasks() {
            let h = sub.union(fixed);
            if !filter(h) {
                continue;
            }
            let len = h.len() as usize;
            let k = key(h) as usize;
            hist[len * KEYS + k] += 1;
        }
        let mut start = vec![0u32; 14 * (KEYS + 1)];
        let mut offset = 0u32;
        for len in 0..14 {
            for k in 0..KEYS {
                start[len * (KEYS + 1) + k] = offset;
                offset += hist[len * KEYS + k];
            }
            start[len * (KEYS + 1) + KEYS] = offset;
        }
        let total = offset as usize;
        let mut cursor = vec![0u32; 14 * KEYS];
        for len in 0..14 {
            cursor[len * KEYS..(len + 1) * KEYS]
                .copy_from_slice(&start[len * (KEYS + 1)..len * (KEYS + 1) + KEYS]);
        }
        let mut holdings = vec![0u16; total];
        for sub in pool.submasks() {
            let h = sub.union(fixed);
            if !filter(h) {
                continue;
            }
            let len = h.len() as usize;
            let k = key(h) as usize;
            let slot = &mut cursor[len * KEYS + k];
            holdings[*slot as usize] = h.bits();
            *slot += 1;
        }
        let counts: [SparseVec; 14] = core::array::from_fn(|len| {
            let mut v = Vec::new();
            for k in 0..KEYS {
                let n = u64::from(hist[len * KEYS + k]);
                if n > 0 {
                    v.push((k as u16, n));
                }
            }
            SparseVec(v)
        });
        (holdings, counts)
    }

    /// Every `(len, key)` bucket the optimized `build` produces - both which holdings it
    /// contains and in what order - must match the dense reference exactly, across a variety of
    /// pool sizes, fixed cards, filters and keys (including one that exercises a nonzero additive
    /// feature, i.e. the `x` component of a packed key).
    #[test]
    fn build_matches_the_dense_reference_implementation_bit_for_bit() {
        let trivial_filter: &dyn Fn(Holding) -> bool = &|_| true;
        let hcp_only_key: &dyn Fn(Holding) -> u16 = &|h| u16::from(bridge_eval::holding_hcp(h));
        // A synthetic additive feature (card count, bounded by suit length like `Additive::Cards`)
        // to exercise nonzero `x` components of the packed key.
        let with_feature_key: &dyn Fn(Holding) -> u16 =
            &|h| pack_key(bridge_eval::holding_hcp(h), h.len().min(13));
        // Filters out anything holding the ace (single-suit `CardRequirement`-style filter).
        let no_ace_filter: &dyn Fn(Holding) -> bool = &|h| !h.contains(bridge_core::Rank::Ace);

        type Case<'a> = (
            Holding,
            Holding,
            &'a dyn Fn(Holding) -> bool,
            &'a dyn Fn(Holding) -> u16,
        );
        let cases: &[Case<'_>] = &[
            // The FULL_SUIT case itself.
            (Holding::FULL, Holding::EMPTY, trivial_filter, hcp_only_key),
            // A small pool (the common "mostly fixed, small remaining pool" case §9 is about).
            (
                Holding::from_bits(0b0000_0000_0111).unwrap(),
                Holding::from_bits(0b1000_0000_0000).unwrap(),
                trivial_filter,
                hcp_only_key,
            ),
            // Empty pool (every holding is exactly `fixed`).
            (
                Holding::EMPTY,
                Holding::from_bits(0b1010_0000_0101).unwrap(),
                trivial_filter,
                hcp_only_key,
            ),
            // A medium pool with a nonzero additive feature.
            (
                Holding::from_bits(0b0111_1111_1111).unwrap(),
                Holding::EMPTY,
                trivial_filter,
                with_feature_key,
            ),
            // A pool with a non-trivial filter and some fixed cards.
            (
                Holding::from_bits(0b0111_1111_0000).unwrap(),
                Holding::from_bits(0b0000_0000_1100).unwrap(),
                no_ace_filter,
                hcp_only_key,
            ),
        ];

        for &(pool, fixed, filter, key) in cases {
            let optimized = SuitTable::build(pool, fixed, filter, key);
            let (dense_holdings, dense_counts) = build_dense_reference(pool, fixed, filter, key);
            assert_eq!(
                optimized.holdings, dense_holdings,
                "holdings differ for pool={pool:?} fixed={fixed:?}"
            );
            assert_eq!(
                optimized.counts, dense_counts,
                "counts differ for pool={pool:?} fixed={fixed:?}"
            );
            // `bucket` must be internally consistent with `holdings`/`counts`: every bucket's
            // length matches its count, and concatenating every bucket in `(len, key)` order
            // reconstructs `holdings` exactly.
            let mut reconstructed = Vec::new();
            for len in 0..14u8 {
                for &(k, n) in &optimized.counts[len as usize].0 {
                    let bucket = optimized.bucket(len, k);
                    assert_eq!(bucket.len() as u64, n);
                    reconstructed.extend_from_slice(bucket);
                }
            }
            assert_eq!(reconstructed, optimized.holdings);
        }
    }

    #[test]
    fn full_suit_has_every_holding_bucketed_by_hcp() {
        let table = full_suit();
        // Every one of the 8192 holdings is present exactly once.
        let total: u64 = table
            .counts
            .iter()
            .flat_map(|c| c.0.iter())
            .map(|&(_, n)| n)
            .sum();
        assert_eq!(total, 8192);
        // Spot-check: AKQ of a suit is 3 cards, 9 HCP.
        let akq = Holding::top_ranks(3);
        let key = pack_key(9, 0);
        let bucket = table.bucket(3, key);
        assert!(bucket.contains(&akq.bits()));
    }
}
