//! Per-suit enumeration of candidate holdings, bucketed by `(length, key)`.

use bridge_core::Holding;

/// Number of distinct keys: HCP `0..=37` in the low 6 bits, one extra feature in bits `6..`.
pub(crate) const KEYS: usize = 64 * 32;

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
/// over the HCP axis so that any HCP window is a two-lookup box sum.
pub(crate) struct PairConv {
    pub(crate) p: SparseVec,
    pub(crate) prefix: Box<[u64]>,
}

impl SuitTable {
    /// Enumerates `sub ⊆ pool`, folds in `fixed`, applies `filter`, and bucket-sorts.
    pub(crate) fn build(
        pool: Holding,
        fixed: Holding,
        filter: &dyn Fn(Holding) -> bool,
        key: &dyn Fn(Holding) -> u16,
    ) -> SuitTable {
        todo!("phase 2")
    }

    /// The bucket for `(len, key)`.
    pub(crate) fn bucket(&self, len: u8, key: u16) -> &[u16] {
        let s = &self.start[len as usize];
        &self.holdings[s[key as usize] as usize..s[key as usize + 1] as usize]
    }
}
