//! Compile-time tables indexed by the 13-bit [`Holding`](bridge_core::Holding).

/// Per-suit metrics for every one of the 8192 holdings.
///
/// Built once at compile time by a `const fn`; about 40 KB in `.rodata`. Half-unit metrics are
/// stored doubled (`losers2 = 2 × LTC`) so that everything is an integer.
pub struct SuitTables {
    /// High-card points (A = 4, K = 3, Q = 2, J = 1).
    pub hcp: [u8; 8192],
    /// Classic losing-trick count, doubled: `2 × (min(len, 3) − A − [K ∧ len ≥ 2] − [Q ∧ len ≥ 3])`.
    pub losers2: [u8; 8192],
    /// "New" losing-trick count, doubled: missing A = 3, missing K = 2, missing Q = 1, each
    /// counted only while the suit is long enough for that honour to matter (A: len ≥ 1,
    /// K: len ≥ 2, Q: len ≥ 3).
    pub nltc2: [u8; 8192],
    /// Quick tricks, doubled: AK = 4, AQ = 3, A = 2, KQ = 2, Kx (len ≥ 2) = 1.
    pub qt2: [u8; 8192],
    /// Number of honours among A K Q J T.
    pub honors5: [u8; 8192],
}

/// The per-suit tables.
pub static SUIT: SuitTables = SuitTables::build();

const ACE: u16 = 1 << 12;
const KING: u16 = 1 << 11;
const QUEEN: u16 = 1 << 10;
const JACK: u16 = 1 << 9;
const TEN: u16 = 1 << 8;
const TOP5: u16 = ACE | KING | QUEEN | JACK | TEN;

impl SuitTables {
    const fn build() -> SuitTables {
        let mut t = SuitTables {
            hcp: [0; 8192],
            losers2: [0; 8192],
            nltc2: [0; 8192],
            qt2: [0; 8192],
            honors5: [0; 8192],
        };
        let mut h: usize = 0;
        while h < 8192 {
            let bits = h as u16;
            let len = bits.count_ones() as u8;
            let a = bits & ACE != 0;
            let k = bits & KING != 0;
            let q = bits & QUEEN != 0;
            let j = bits & JACK != 0;

            t.hcp[h] = (a as u8) * 4 + (k as u8) * 3 + (q as u8) * 2 + (j as u8);

            t.losers2[h] = if len == 0 {
                0
            } else {
                let base = if len < 3 { len } else { 3 };
                2 * (base - a as u8 - (k && len >= 2) as u8 - (q && len >= 3) as u8)
            };

            t.nltc2[h] =
                (len >= 1 && !a) as u8 * 3 + (len >= 2 && !k) as u8 * 2 + (len >= 3 && !q) as u8;

            t.qt2[h] = if a && k {
                4
            } else if a && q {
                3
            } else if a || (k && q) {
                2
            } else if k && len >= 2 {
                1
            } else {
                0
            };

            t.honors5[h] = (bits & TOP5).count_ones() as u8;
            h += 1;
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spot_checks() {
        let akq = (ACE | KING | QUEEN) as usize;
        assert_eq!(SUIT.hcp[akq], 9);
        assert_eq!(SUIT.losers2[akq], 0);
        assert_eq!(SUIT.qt2[akq], 4);
        assert_eq!(SUIT.honors5[akq], 3);
        let kx = (KING | 1) as usize;
        assert_eq!(SUIT.losers2[kx], 2);
        assert_eq!(SUIT.qt2[kx], 1);
        assert_eq!(SUIT.nltc2[kx], 3);
        assert_eq!(SUIT.losers2[0], 0);
        assert_eq!(SUIT.losers2[0b111], 6);
    }
}
