//! Small, dependency-free random-number helpers used only inside the sampler.

/// Draws a uniform `u64` in `0..bound` (`bound > 0`).
///
/// Uses Lemire's widening-multiply method: no floating-point rounding and no modulo bias, so the
/// counts and cumulative weights (D2, D13: exact `u64`s) are sampled from exactly, matching the
/// design's "整数の `random_range`、厳密、浮動小数の丸めなし".
///
/// # Panics
/// Debug-asserts `bound > 0`; every call site only ever passes a positive `total` or `weight`.
pub(crate) fn random_below<R: rand_core::Rng + ?Sized>(rng: &mut R, bound: u64) -> u64 {
    debug_assert!(bound > 0, "random_below called with bound == 0");
    if bound == 1 {
        return 0;
    }
    loop {
        let x = rng.next_u64();
        let product = u128::from(x) * u128::from(bound);
        let low = product as u64;
        if low < bound {
            let threshold = bound.wrapping_neg() % bound;
            if low < threshold {
                continue;
            }
        }
        return (product >> 64) as u64;
    }
}

/// A tiny SplitMix64 generator used only to estimate a rejection term's acceptance rate during
/// [`super::term::PreparedTerm::prepare`] (§7 step 7). Library code cannot depend on
/// `rand_xoshiro`, which is a dev-dependency only, and this estimate needs no cryptographic or
/// even particularly high-quality randomness — just a deterministic, reproducible stream so that
/// `prepare` stays a pure function of its arguments.
pub(crate) struct SplitMix64(u64);

impl SplitMix64 {
    /// Seeds the generator.
    pub(crate) fn new(seed: u64) -> SplitMix64 {
        SplitMix64(seed)
    }

    fn next_raw(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

impl rand_core::TryRng for SplitMix64 {
    type Error = core::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok((self.next_raw() >> 32) as u32)
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(self.next_raw())
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        rand_core::utils::fill_bytes_via_next_word(dst, || self.try_next_u64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_below_stays_in_range() {
        let mut rng = SplitMix64::new(0x00C0_FFEE);
        for bound in [1u64, 2, 3, 7, 1_000, 635_013_559_600] {
            for _ in 0..1000 {
                let v = random_below(&mut rng, bound);
                assert!(v < bound, "{v} not in 0..{bound}");
            }
        }
    }
}
