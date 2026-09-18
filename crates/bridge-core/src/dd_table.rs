//! Double-dummy result tables.

use crate::{Seat, Strain};

/// Double-dummy tricks for every `(strain, declarer)` pair.
///
/// A plain data type: it is produced by `bridge-dds`, read from PBN `OptimumResultTable`
/// sections by `bridge-format`, and consumed by applications, so it lives here where all of
/// them can see it without depending on the solver.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DdTable {
    /// `tricks[strain][declarer]` in `bridge-core` order.
    tricks: [[u8; 4]; 5],
}

impl DdTable {
    /// Builds a table from `[strain][declarer]` tricks (strain order clubs … notrump, seat
    /// order North … West).
    pub const fn new(tricks: [[u8; 4]; 5]) -> DdTable {
        DdTable { tricks }
    }

    /// Tricks `declarer` makes in `strain`.
    pub const fn tricks(&self, strain: Strain, declarer: Seat) -> u8 {
        self.tricks[strain.index() as usize][declarer.index() as usize]
    }

    /// The raw `[strain][declarer]` array.
    pub const fn as_array(&self) -> [[u8; 4]; 5] {
        self.tricks
    }

    /// The best strain and trick count for `declarer` (highest tricks; ties resolved toward
    /// the higher-scoring strain, i.e. later in bidding order).
    pub fn best_for(&self, declarer: Seat) -> (Strain, u8) {
        todo!("phase 5")
    }
}
