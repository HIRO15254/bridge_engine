//! Deterministic per-sample random number generation.
//!
//! `SmallRng` is Xoshiro256++ on 64-bit targets but Xoshiro128++ on 32-bit ones (wasm32), so
//! the generator is named explicitly. Only `from_seed` is used (its behaviour is fixed by the
//! algorithm; `seed_from_u64` is not guaranteed stable across `rand` versions).

use rand_core::SeedableRng;

/// The sampler's generator.
pub type SampleRng = rand_xoshiro::Xoshiro256PlusPlus;

/// Vigna's SplitMix64 step.
pub fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The generator for sample `index` under `master` seed; independent of every other sample.
pub fn rng_for(master: u64, index: u64) -> SampleRng {
    let mut z = master ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    let mut seed = [0u8; 32];
    for chunk in seed.chunks_mut(8) {
        chunk.copy_from_slice(&splitmix64(&mut z).to_le_bytes());
    }
    SampleRng::from_seed(seed)
}
