//! Exact full-deck counts, including a naive independent DP cross-check (2.4, 2.5, 2.6).

use bridge_constraint::{Atom, HandConstraint, SampleOptions, Sampler, ShapeSet};
use bridge_core::{Hand, Shape, ShapeClass, Suit};

/// HCP of a 13-bit suit holding, computed independently of `bridge_eval` (Ace = bit 12,
/// King = bit 11, Queen = bit 10, Jack = bit 9 - see `bridge_core::Rank`).
fn suit_hcp(bits: u16) -> u32 {
    4 * ((bits >> 12) & 1) as u32
        + 3 * ((bits >> 11) & 1) as u32
        + 2 * ((bits >> 10) & 1) as u32
        + ((bits >> 9) & 1) as u32
}

/// `hist[len][hcp]` = number of the 8192 holdings of one suit with that length and HCP.
fn suit_histogram() -> [[u64; 11]; 14] {
    let mut hist = [[0u64; 11]; 14];
    for bits in 0u16..8192 {
        let len = bits.count_ones() as usize;
        let hcp = suit_hcp(bits) as usize;
        hist[len][hcp] += 1;
    }
    hist
}

/// Convolves two per-suit `(len, hcp)` histograms restricted to lengths `la`/`lb`, giving a
/// combined `hcp -> count` histogram (max combined HCP is `20`).
fn convolve(hist: &[[u64; 11]; 14], la: usize, lb: usize) -> [u64; 21] {
    let mut out = [0u64; 21];
    for ha in 0..11 {
        let na = hist[la][ha];
        if na == 0 {
            continue;
        }
        for hb in 0..11 {
            let nb = hist[lb][hb];
            if nb == 0 {
                continue;
            }
            out[ha + hb] += na * nb;
        }
    }
    out
}

/// Naive DP: the number of full-deck hands with the given ordered shape and HCP in `hcp_range`,
/// by convolving all four suits' `(len, hcp)` histograms (per-suit hist × 4, folded pairwise).
fn naive_shape_hcp_count(hist: &[[u64; 11]; 14], shape: Shape, hcp_lo: u8, hcp_hi: u8) -> u64 {
    let lens = shape.lens();
    let ab = convolve(hist, lens[0] as usize, lens[1] as usize);
    let cd = convolve(hist, lens[2] as usize, lens[3] as usize);
    let mut total = 0u64;
    for (a, &na) in ab.iter().enumerate() {
        if na == 0 {
            continue;
        }
        for (c, &nc) in cd.iter().enumerate() {
            if nc == 0 {
                continue;
            }
            let hcp = (a + c) as u8;
            if hcp >= hcp_lo && hcp <= hcp_hi {
                total += na * nc;
            }
        }
    }
    total
}

/// Naive DP over every shape in `shapes`, `hcp` in `[hcp_lo, hcp_hi]`.
fn naive_count(shapes: ShapeSet, hcp_lo: u8, hcp_hi: u8) -> u64 {
    let hist = suit_histogram();
    shapes
        .iter()
        .map(|shape| naive_shape_hcp_count(&hist, shape, hcp_lo, hcp_hi))
        .sum()
}

fn sampler_count(shapes: ShapeSet, hcp_lo: u8, hcp_hi: u8) -> u64 {
    let atom = Atom {
        shapes,
        ..Atom::ANY.with_hcp(hcp_lo..=hcp_hi)
    };
    let sampler = Sampler::prepare(
        &HandConstraint::Atom(atom),
        Hand::FULL,
        Hand::EMPTY,
        &SampleOptions::default(),
    )
    .expect("Hand::FULL/Hand::EMPTY never overlap");
    assert!(sampler.is_exact());
    sampler.count()
}

#[test]
fn full_deck_any_matches_c_52_13() {
    let sampler = Sampler::prepare(
        &HandConstraint::ANY,
        Hand::FULL,
        Hand::EMPTY,
        &SampleOptions::default(),
    )
    .unwrap();
    assert!(sampler.is_exact());
    assert_eq!(sampler.count(), 635_013_559_600);
}

#[test]
fn full_deck_15_17_balanced_matches_known_value() {
    // Independently verified reference value (2.4's completion criterion).
    assert_eq!(sampler_count(ShapeSet::BALANCED, 15, 17), 30_897_212_184);
}

#[test]
fn full_deck_counts_match_naive_dp() {
    let cases: &[(ShapeSet, u8, u8)] = &[
        (ShapeSet::BALANCED, 15, 17),
        (ShapeSet::BALANCED, 12, 14),
        (ShapeSet::from_class(ShapeClass::C4432), 10, 40),
        (ShapeSet::from_suit_len(Suit::Spades, 5, 13), 0, 37),
        (ShapeSet::ALL, 20, 25),
        (ShapeSet::SEMI_BALANCED, 8, 11),
    ];
    for &(shapes, lo, hi) in cases {
        let expected = naive_count(shapes, lo, hi);
        let got = sampler_count(shapes, lo, hi);
        assert_eq!(got, expected, "shapes/hcp {lo}..={hi} mismatch");
    }
}

#[test]
fn full_deck_hcp_marginal_matches_naive_dp() {
    // Sum of exact per-HCP counts (via `prepare`) equals C(52,13); spot check a few values
    // against the naive DP too.
    let hist = suit_histogram();
    let mut total = 0u64;
    for hcp in 0..=37u8 {
        let expected = naive_count(ShapeSet::ALL, hcp, hcp);
        let got = sampler_count(ShapeSet::ALL, hcp, hcp);
        assert_eq!(got, expected, "hcp={hcp} mismatch");
        total += got;
    }
    assert_eq!(total, 635_013_559_600);
    let _ = hist;
}
