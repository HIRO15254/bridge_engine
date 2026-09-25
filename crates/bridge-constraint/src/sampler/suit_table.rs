//! Per-suit enumeration of candidate holdings, bucketed by `(length, key)`.

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
/// `holdings` is sorted by `(len, key)` (counting sort); `start[len][key]..start[len][key + 1]`
/// delimits one bucket. Every holding already includes the suit's fixed cards.
pub(crate) struct SuitTable {
    pub(crate) holdings: Vec<u16>,
    pub(crate) start: Box<[[u32; KEYS + 1]; 14]>,
    /// `counts[len]` = sparse `(key, n)` list with `n > 0`.
    pub(crate) counts: [SparseVec; 14],
}

/// A sparse count vector.
#[derive(Clone, Debug, Default)]
pub(crate) struct SparseVec(pub(crate) Vec<(u16, u64)>);

/// The convolution of two suits' count vectors for one `(len_a, len_b)` pair, with prefix sums
/// over the `(hcp, x)` axes so that any box (HCP window × feature window) is a four-lookup sum.
///
/// Built in [`super::term`] (which knows how to combine two [`SuitTable`]s); the type itself is
/// just data plus the box-sum query.
pub(crate) struct PairConv {
    pub(crate) p: SparseVec,
    pub(crate) prefix: Box<[u64]>,
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
        // Pass 1: histogram of (len, key) -> count.
        let mut hist = vec![0u32; 14 * KEYS];
        for sub in pool.submasks() {
            let h = sub.union(fixed);
            if !filter(h) {
                continue;
            }
            let len = h.len() as usize;
            let k = key(h) as usize;
            debug_assert!(k < KEYS, "key() must stay within 0..KEYS");
            hist[len * KEYS + k] += 1;
        }

        // Prefix sums over (len, key) in row-major order give each bucket's start offset.
        let mut start: Box<[[u32; KEYS + 1]; 14]> = Box::new([[0u32; KEYS + 1]; 14]);
        let mut offset = 0u32;
        for len in 0..14 {
            for k in 0..KEYS {
                start[len][k] = offset;
                offset += hist[len * KEYS + k];
            }
            start[len][KEYS] = offset;
        }
        let total = offset as usize;

        // Pass 2: place each holding into its bucket (a mutable copy of `start` as the cursor).
        let mut cursor = vec![0u32; 14 * KEYS];
        for len in 0..14 {
            cursor[len * KEYS..(len + 1) * KEYS].copy_from_slice(&start[len][..KEYS]);
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

        SuitTable {
            holdings,
            start,
            counts,
        }
    }

    /// The bucket for `(len, key)`.
    pub(crate) fn bucket(&self, len: u8, key: u16) -> &[u16] {
        let s = &self.start[len as usize];
        &self.holdings[s[key as usize] as usize..s[key as usize + 1] as usize]
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
