//! Per-suit enumeration of candidate holdings, bucketed by `(length, key)`.

use std::sync::{Arc, LazyLock};

use bridge_core::Holding;

/// Number of bits the HCP component of a packed key uses (`0..=37` fits in 6 bits, and a single
/// suit's HCP is at most 10, well inside that).
pub(crate) const HCP_BITS: u32 = 6;
/// Number of bits the optional additive-feature component uses.
pub(crate) const X_BITS: u32 = 5;

/// The largest HCP a single suit's holding can carry (A+K+Q+J = 10).
const SUIT_HCP_MAX: usize = 10;
/// The largest additive-feature value a single suit's holding can carry: every additive feature
/// (`Controls`, `Losers`, `QuickTricks`, or a `CardRequirement`'s per-suit popcount) is bounded by
/// the suit's own length, so `0..=13` covers all of them.
const SUIT_X_MAX: usize = 13;

/// Size of the dense per-length index domain when a term carries no additive feature (K=1): every
/// entry's `x` is `0`, so only the HCP axis (`0..=10`) matters.
pub(crate) const DENSE_NK_NO_X: usize = SUIT_HCP_MAX + 1;
/// Size of the dense per-length index domain when a term carries one additive feature (K=2):
/// `hcp` (`0..=10`) times `x` (`0..=13`).
pub(crate) const DENSE_NK_WITH_X: usize = (SUIT_HCP_MAX + 1) * (SUIT_X_MAX + 1);

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

/// Maps a packed `(hcp, x)` key to a dense index `hcp + (SUIT_HCP_MAX + 1) * x` in `0..nk`. `nk`
/// is either [`DENSE_NK_NO_X`] (only `x = 0` is ever produced) or [`DENSE_NK_WITH_X`] (the full
/// per-suit `x` range); either way a single suit's `(hcp, x)` pair always lands inside `0..nk`, so
/// this is a plain array index rather than a hash lookup or a binary search.
fn dense_index(key: u16, nk: usize) -> usize {
    let (hcp, x) = unpack_key(key);
    let d = hcp as usize + (SUIT_HCP_MAX + 1) * x as usize;
    debug_assert!(d < nk, "dense index {d} out of range for nk={nk}");
    d
}

/// The candidate holdings of one suit for one prepared term.
///
/// `holdings` is sorted by `(len, key)` (counting sort). The per-length index domain is the
/// *dense* but *small* `nk` from [`dense_index`] (`11` or `154`, never the `14 * KEYS` a table
/// over the full 16-bit packed-key domain would force every build to zero and scan regardless of
/// how small `pool` is - see §9/D9). `start[len * (nk+1) + d] ..= start[len * (nk+1) + d+1]` gives
/// `holdings`' range for dense index `d`, so [`SuitTable::bucket`] is an O(1) array lookup rather
/// than a hash lookup or a binary search.
pub(crate) struct SuitTable {
    pub(crate) holdings: Vec<u16>,
    /// Flattened `[14][nk+1]` offset table (row-major, row length `nk + 1`).
    start: Vec<u32>,
    /// The dense-key domain size this table was built with ([`DENSE_NK_NO_X`] or
    /// [`DENSE_NK_WITH_X`]); `bucket` must derive `d` the same way `build` did.
    nk: usize,
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
    ///
    /// `nk` is the dense per-length index domain size ([`DENSE_NK_NO_X`] when the caller's `key`
    /// never produces a nonzero `x` component, [`DENSE_NK_WITH_X`] otherwise): every `key(h)` this
    /// call ever produces must map to a [`dense_index`] under `nk`.
    pub(crate) fn build(
        pool: Holding,
        fixed: Holding,
        filter: &dyn Fn(Holding) -> bool,
        key: &dyn Fn(Holding) -> u16,
        nk: usize,
    ) -> SuitTable {
        // Pass 1: histogram of (len, d) -> count, a dense `14 * nk` array. `nk` is `11` or `154`
        // (never the `14 * KEYS = 14 * 2048` a table over the whole packed-key domain would need),
        // so this is cheap regardless of how small `pool` is.
        let mut hist = vec![0u32; 14 * nk];
        for sub in pool.submasks() {
            let h = sub.union(fixed);
            if !filter(h) {
                continue;
            }
            let len = h.len() as usize;
            let d = dense_index(key(h), nk);
            hist[len * nk + d] += 1;
        }

        // Prefix-sum each length's row into `start` (row width `nk + 1`, the trailing entry being
        // the row's total).
        let mut start = vec![0u32; 14 * (nk + 1)];
        let mut offset = 0u32;
        for len in 0..14 {
            for d in 0..nk {
                start[len * (nk + 1) + d] = offset;
                offset += hist[len * nk + d];
            }
            start[len * (nk + 1) + nk] = offset;
        }
        let total = offset as usize;

        // Pass 2: place each holding into its bucket. `hist` is reused as the placement cursor
        // (one allocation instead of two): every `hist[len*nk+d]` slot is overwritten with its
        // `start` offset before any cursor read touches it.
        for len in 0..14 {
            hist[len * nk..len * nk + nk]
                .copy_from_slice(&start[len * (nk + 1)..len * (nk + 1) + nk]);
        }
        let mut holdings = vec![0u16; total];
        for sub in pool.submasks() {
            let h = sub.union(fixed);
            if !filter(h) {
                continue;
            }
            let len = h.len() as usize;
            let d = dense_index(key(h), nk);
            let slot = &mut hist[len * nk + d];
            holdings[*slot as usize] = h.bits();
            *slot += 1;
        }

        // `counts`: the sparse (key, n>0) list per length, in ascending dense-index order. Since
        // `nk` is exactly `SUIT_HCP_MAX + 1` (no gap between consecutive `x` blocks), ascending
        // dense index implies ascending packed key too, so this matches the order a full
        // packed-key-domain scan would produce.
        let counts: [SparseVec; 14] = core::array::from_fn(|len| {
            let mut v = Vec::new();
            for d in 0..nk {
                let n = u64::from(start[len * (nk + 1) + d + 1] - start[len * (nk + 1) + d]);
                if n > 0 {
                    let hcp = (d % (SUIT_HCP_MAX + 1)) as u8;
                    let x = (d / (SUIT_HCP_MAX + 1)) as u8;
                    v.push((pack_key(hcp, x), n));
                }
            }
            SparseVec(v)
        });

        SuitTable {
            holdings,
            start,
            nk,
            counts,
        }
    }

    /// The bucket for `(len, key)`, or an empty slice when that key was never observed.
    pub(crate) fn bucket(&self, len: u8, key: u16) -> &[u16] {
        let len = len as usize;
        let d = dense_index(key, self.nk);
        let base = len * (self.nk + 1);
        let start = self.start[base + d] as usize;
        let end = self.start[base + d + 1] as usize;
        &self.holdings[start..end]
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
        DENSE_NK_NO_X,
    ))
});

/// A cheap `Arc` clone of the shared full-suit table.
pub(crate) fn full_suit() -> Arc<SuitTable> {
    Arc::clone(&FULL_SUIT)
}

/// A flat, allocation-free index from a `(len_a, len_b)` pair (each `0..=13`) to a lazily-built
/// [`PairConv`] (§10: replaces a `HashMap<(u8,u8), PairConv>`, whose default hasher is SipHash over
/// a key space that is really just `14 * 14 = 196` slots).
pub(crate) struct PairMap {
    /// `idx[len_a * 14 + len_b]` is the index into `convs`, or `u16::MAX` when not yet built.
    idx: [u16; 196],
    convs: Vec<PairConv>,
}

impl PairMap {
    pub(crate) fn new() -> PairMap {
        PairMap {
            idx: [u16::MAX; 196],
            convs: Vec::new(),
        }
    }

    /// The `PairConv` for `(len_a, len_b)`, building and caching it via `build` on first use.
    pub(crate) fn get_or_build(
        &mut self,
        len_a: u8,
        len_b: u8,
        build: impl FnOnce() -> PairConv,
    ) -> &PairConv {
        let slot = len_a as usize * 14 + len_b as usize;
        if self.idx[slot] == u16::MAX {
            let i = self.convs.len();
            debug_assert!(i < usize::from(u16::MAX), "at most 196 distinct pairs");
            self.convs.push(build());
            self.idx[slot] = i as u16;
        }
        &self.convs[self.idx[slot] as usize]
    }

    /// The `PairConv` for `(len_a, len_b)` if it was already built via `get_or_build`.
    pub(crate) fn get(&self, len_a: u8, len_b: u8) -> Option<&PairConv> {
        let slot = len_a as usize * 14 + len_b as usize;
        match self.idx[slot] {
            u16::MAX => None,
            i => Some(&self.convs[i as usize]),
        }
    }
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
    /// `14 * KEYS`-sized histogram and prefix-sum scan over the *whole* 16-bit packed-key domain,
    /// instead of the small `14 * nk` domain the real `nk`-aware `dense_index` needs). Used only
    /// to check the optimized `build` produces bit-identical `holdings`/`counts`: the fix must
    /// change how fast a table is built, never what a bucket contains or in what order (§9's
    /// `pick_holding` selects uniformly within a bucket by index, so the bucket's element order is
    /// observable through which hand a given RNG draw returns).
    fn build_dense_reference(
        pool: Holding,
        fixed: Holding,
        filter: &dyn Fn(Holding) -> bool,
        key: &dyn Fn(Holding) -> u16,
    ) -> (Vec<u16>, [SparseVec; 14]) {
        const KEYS: usize = 64 * 32;
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
            usize,
        );
        let cases: &[Case<'_>] = &[
            // The FULL_SUIT case itself.
            (
                Holding::FULL,
                Holding::EMPTY,
                trivial_filter,
                hcp_only_key,
                DENSE_NK_NO_X,
            ),
            // A small pool (the common "mostly fixed, small remaining pool" case §9 is about).
            (
                Holding::from_bits(0b0000_0000_0111).unwrap(),
                Holding::from_bits(0b1000_0000_0000).unwrap(),
                trivial_filter,
                hcp_only_key,
                DENSE_NK_NO_X,
            ),
            // Empty pool (every holding is exactly `fixed`).
            (
                Holding::EMPTY,
                Holding::from_bits(0b1010_0000_0101).unwrap(),
                trivial_filter,
                hcp_only_key,
                DENSE_NK_NO_X,
            ),
            // A medium pool with a nonzero additive feature.
            (
                Holding::from_bits(0b0111_1111_1111).unwrap(),
                Holding::EMPTY,
                trivial_filter,
                with_feature_key,
                DENSE_NK_WITH_X,
            ),
            // A pool with a non-trivial filter and some fixed cards.
            (
                Holding::from_bits(0b0111_1111_0000).unwrap(),
                Holding::from_bits(0b0000_0000_1100).unwrap(),
                no_ace_filter,
                hcp_only_key,
                DENSE_NK_NO_X,
            ),
        ];

        for &(pool, fixed, filter, key, nk) in cases {
            let optimized = SuitTable::build(pool, fixed, filter, key, nk);
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

    #[test]
    fn pair_map_builds_once_and_caches() {
        use core::cell::Cell;

        let mut map = PairMap::new();
        let builds = Cell::new(0u32);
        {
            let conv = map.get_or_build(3, 5, || {
                builds.set(builds.get() + 1);
                PairConv {
                    p: SparseVec(vec![(pack_key(1, 0), 7)]),
                    prefix: vec![7u64].into_boxed_slice(),
                    width: 1,
                }
            });
            assert_eq!(conv.p.0, vec![(pack_key(1, 0), 7)]);
        }
        assert_eq!(builds.get(), 1);
        assert!(map.get(3, 5).is_some());
        assert!(map.get(0, 0).is_none());

        // A second `get_or_build` for the same pair reuses the cached entry.
        map.get_or_build(3, 5, || {
            builds.set(builds.get() + 1);
            PairConv {
                p: SparseVec(Vec::new()),
                prefix: Box::new([]),
                width: 1,
            }
        });
        assert_eq!(builds.get(), 1);
    }
}
