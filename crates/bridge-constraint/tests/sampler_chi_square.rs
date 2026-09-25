//! Statistical tests (2.4-2.6): a small pool's `10^5` samples land uniformly across every
//! satisfying hand, and the full deck's 15-17-balanced samples reproduce the exact per-HCP
//! marginal from `prepare`. Both use a fixed seed for reproducibility.

use std::collections::HashMap;

use bridge_constraint::{Atom, HandConstraint, SampleOptions, Sampler, ShapeSet};
use bridge_core::{Hand, Holding, Suit};
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;

/// Lanczos approximation to `ln(Gamma(x))` (g = 7, n = 9 coefficients), accurate to about
/// `1e-13` for `x > 0` - standard textbook constants (Numerical Recipes).
fn ln_gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const COEF: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_312e-7,
    ];
    if x < 0.5 {
        // Reflection formula (not hit by the `df/2 >= 1` inputs this file uses, kept for safety).
        (std::f64::consts::PI / (std::f64::consts::PI * x).sin()).ln() - ln_gamma(1.0 - x)
    } else {
        let x = x - 1.0;
        let t = x + G + 0.5;
        let mut a = COEF[0];
        for (i, &c) in COEF.iter().enumerate().skip(1) {
            a += c / (x + i as f64);
        }
        0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
    }
}

/// The regularized lower incomplete gamma function `P(a, x)` by its series expansion (valid for
/// `x < a + 1`; Numerical Recipes §6.2).
fn gamma_p_series(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let gln = ln_gamma(a);
    let mut ap = a;
    let mut sum = 1.0 / a;
    let mut del = sum;
    for _ in 0..200 {
        ap += 1.0;
        del *= x / ap;
        sum += del;
        if del.abs() < sum.abs() * 1e-14 {
            break;
        }
    }
    sum * (-x + a * x.ln() - gln).exp()
}

/// The regularized upper incomplete gamma function `Q(a, x)` by its continued fraction (valid for
/// `x >= a + 1`; Numerical Recipes §6.2).
fn gamma_q_cf(a: f64, x: f64) -> f64 {
    let gln = ln_gamma(a);
    let fpmin = 1e-300;
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / fpmin;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..200 {
        let an = -(f64::from(i)) * (f64::from(i) - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < fpmin {
            d = fpmin;
        }
        c = b + an / c;
        if c.abs() < fpmin {
            c = fpmin;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-14 {
            break;
        }
    }
    (-x + a * x.ln() - gln).exp() * h
}

/// `Q(a, x) = 1 - P(a, x)`, the regularized upper incomplete gamma function, picking whichever of
/// the series or the continued fraction converges quickly for the given `(a, x)`.
fn gamma_q(a: f64, x: f64) -> f64 {
    if x < a + 1.0 {
        1.0 - gamma_p_series(a, x)
    } else {
        gamma_q_cf(a, x)
    }
}

/// The upper-tail p-value of a chi-square statistic with `df` degrees of freedom:
/// `P(X > chi2) = Q(df/2, chi2/2)`, exact (to float precision) for any `df`, small or large.
fn chi_square_p_value(chi2: f64, df: f64) -> f64 {
    gamma_q(df / 2.0, chi2 / 2.0)
}

fn chi_square_statistic(observed: &[u64], expected: f64) -> f64 {
    observed
        .iter()
        .map(|&o| {
            let d = o as f64 - expected;
            d * d / expected
        })
        .sum()
}

/// A 16-card pool (the jack, queen, king and ace of every suit) with no fixed cards: `C(16, 13) =
/// C(16, 3) = 560` completions, all satisfying `HandConstraint::ANY`.
fn honors_pool() -> Hand {
    let mut pool = Hand::EMPTY;
    for suit in Suit::ALL {
        pool = pool.with_holding(suit, Holding::top_ranks(4));
    }
    pool
}

#[test]
fn small_pool_samples_are_uniform() {
    let pool = honors_pool();
    let sampler = Sampler::prepare(
        &HandConstraint::ANY,
        pool,
        Hand::EMPTY,
        &SampleOptions::default(),
    )
    .unwrap();
    assert!(sampler.is_exact());
    let count = sampler.count();
    assert!(
        (100..=2000).contains(&count),
        "expected a few hundred satisfying hands, got {count}"
    );

    // Index every one of the `count` completions once.
    let mut index_of: HashMap<Hand, usize> = HashMap::new();
    let cards: Vec<_> = pool.cards().collect();
    let n = cards.len();
    let m = 13usize;
    let mut idxs: Vec<usize> = (0..m).collect();
    loop {
        let mut hand = Hand::EMPTY;
        for &ix in &idxs {
            hand = hand.with(cards[ix]);
        }
        let next_index = index_of.len();
        index_of.insert(hand, next_index);
        let mut i = m;
        let mut done = false;
        loop {
            if i == 0 {
                done = true;
                break;
            }
            i -= 1;
            if idxs[i] != i + n - m {
                idxs[i] += 1;
                for j in i + 1..m {
                    idxs[j] = idxs[j - 1] + 1;
                }
                break;
            }
        }
        if done {
            break;
        }
    }
    assert_eq!(index_of.len() as u64, count);

    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x00C0_FFEE);
    let draws = 100_000u64;
    let mut observed = vec![0u64; count as usize];
    for _ in 0..draws {
        let sample = sampler.sample(&mut rng).unwrap();
        let idx = index_of[&sample.hand];
        observed[idx] += 1;
    }

    let expected = draws as f64 / count as f64;
    let chi2 = chi_square_statistic(&observed, expected);
    let df = (count - 1) as f64;
    let p = chi_square_p_value(chi2, df);
    assert!(
        p > 1e-3,
        "chi2={chi2}, df={df}, p={p} (uniformity rejected)"
    );
}

#[test]
fn full_deck_hcp_marginal_matches_exact_counts() {
    let atom_for = |lo: u8, hi: u8| {
        HandConstraint::Atom(Atom {
            shapes: ShapeSet::BALANCED,
            ..Atom::ANY.with_hcp(lo..=hi)
        })
    };
    let hcps = [15u8, 16, 17];
    let exact_counts: Vec<u64> = hcps
        .iter()
        .map(|&hcp| {
            Sampler::prepare(
                &atom_for(hcp, hcp),
                Hand::FULL,
                Hand::EMPTY,
                &SampleOptions::default(),
            )
            .unwrap()
            .count()
        })
        .collect();
    let total_exact: u64 = exact_counts.iter().sum();
    assert_eq!(total_exact, 30_897_212_184);

    let sampler = Sampler::prepare(
        &atom_for(15, 17),
        Hand::FULL,
        Hand::EMPTY,
        &SampleOptions::default(),
    )
    .unwrap();
    assert!(sampler.is_exact());

    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x00C0_FFEE);
    let draws = 200_000u64;
    let mut observed = [0u64; 3];
    for _ in 0..draws {
        let sample = sampler.sample(&mut rng).unwrap();
        let hcp = bridge_eval::hcp(sample.hand);
        let idx = (hcp - 15) as usize;
        observed[idx] += 1;
    }

    let expected: Vec<f64> = exact_counts
        .iter()
        .map(|&c| draws as f64 * c as f64 / total_exact as f64)
        .collect();
    let chi2: f64 = observed
        .iter()
        .zip(expected.iter())
        .map(|(&o, &e)| {
            let d = o as f64 - e;
            d * d / e
        })
        .sum();
    let df = 2.0;
    let p = chi_square_p_value(chi2, df);
    assert!(
        p > 1e-3,
        "chi2={chi2}, observed={observed:?}, expected={expected:?}, p={p}"
    );
}
