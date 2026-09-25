//! Statistical tests (2.4-2.6): a small pool's `10^5` samples land uniformly across every
//! satisfying hand, and the full deck's 15-17-balanced samples reproduce the exact per-HCP
//! marginal from `prepare`. Both use a fixed seed for reproducibility.

use std::collections::HashMap;

use bridge_constraint::{
    Atom, CardRequirement, EvalRequirement, HandConstraint, Metric, SampleOptions, Sampler,
    ShapeSet,
};
use bridge_core::{Card, Hand, Holding, SHAPES, Shape, Suit};
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

/// Every 13-card completion `fixed ∪ (13 - |fixed| cards from pool)` that satisfies `pred`
/// (`pool`/`fixed` disjoint, `pool.len() + fixed.len() >= 13`).
fn satisfying_completions(pool: Hand, fixed: Hand, pred: impl Fn(Hand) -> bool) -> Vec<Hand> {
    let cards: Vec<Card> = pool.cards().collect();
    let n = cards.len();
    let m = 13 - fixed.len() as usize;
    let mut idxs: Vec<usize> = (0..m).collect();
    let mut out = Vec::new();
    loop {
        let mut hand = fixed;
        for &ix in &idxs {
            hand = hand.with(cards[ix]);
        }
        if pred(hand) {
            out.push(hand);
        }
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
    out
}

/// Draws `draws` samples from `sampler` and checks (a) every drawn hand is one of `satisfying`
/// (the general path's `draw` must never emit a hand outside its own term's exact superset, let
/// alone one that fails the atom) and (b) the draws land uniformly across `satisfying` (a χ²
/// goodness-of-fit test, same technique as `small_pool_samples_are_uniform`).
///
/// This exercises the general path's actual `draw` (shape selection, then the `PairConv`
/// box-sum-weighted `(hcp, x)` split, then per-suit `SuitTable::bucket` lookups), not just
/// `count()`/`log_prob()`: those are computed from the same weight tables `draw` samples from, so
/// a bug specific to `draw` (e.g. an off-by-one in the window shift between the two suit pairs,
/// or picking the wrong holding within a bucket) would not necessarily show up as a wrong count
/// or a wrong `log_prob`, only as a wrongly- or unevenly-drawn hand.
fn assert_general_path_draw_is_uniform(
    sampler: &Sampler,
    satisfying: &[Hand],
    seed: u64,
    draws: u64,
) {
    let mut index_of: HashMap<Hand, usize> = HashMap::new();
    for &h in satisfying {
        let next = index_of.len();
        index_of.insert(h, next);
    }
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
    let mut observed = vec![0u64; satisfying.len()];
    for _ in 0..draws {
        let sample = sampler.sample(&mut rng).unwrap();
        let idx = *index_of
            .get(&sample.hand)
            .unwrap_or_else(|| panic!("drew a hand outside the exact superset: {:?}", sample.hand));
        observed[idx] += 1;
    }
    let expected = draws as f64 / satisfying.len() as f64;
    let chi2 = chi_square_statistic(&observed, expected);
    let df = (satisfying.len() - 1) as f64;
    let p = chi_square_p_value(chi2, df);
    assert!(
        p > 1e-3,
        "chi2={chi2}, df={df}, p={p} (uniformity rejected), n={}",
        satisfying.len()
    );
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

/// The general path's `draw`, exercised through a single-suit `CardRequirement` (a literal
/// `full_deck_hcp_marginal_matches_exact_counts` never has: that test's atom is shape+HCP only).
/// A single-suit requirement routes through `suit_filters`/`SuitTable::bucket` rather than the
/// additive-feature slot (`classify`), so this specifically checks that per-suit bucket lookup
/// during a real draw, not just at `prepare` time.
#[test]
fn general_path_card_requirement_samples_are_uniform() {
    let pool = honors_pool();
    let atom = Atom {
        cards: vec![CardRequirement::in_suit(
            Suit::Spades,
            Holding::top_ranks(2), // ♠A, ♠K
            1..=2,
        )],
        ..Atom::ANY
    };
    let c = HandConstraint::Atom(atom.clone());
    let sampler = Sampler::prepare(&c, pool, Hand::EMPTY, &SampleOptions::default()).unwrap();
    assert!(sampler.is_exact());

    let satisfying = satisfying_completions(pool, Hand::EMPTY, |h| atom.satisfies(h));
    assert_eq!(satisfying.len() as u64, sampler.count());
    assert!(
        (50..=560).contains(&satisfying.len()),
        "expected a sizeable but proper subset of the 560 completions, got {}",
        satisfying.len()
    );

    assert_general_path_draw_is_uniform(&sampler, &satisfying, 0x5EED_0001, 100_000);
}

/// Same as `general_path_card_requirement_samples_are_uniform`, but the atom also carries an
/// eval requirement (`Controls`) that competes for the sampler's one additive-feature slot: the
/// general path's `draw` then has to split the shape's combined weight across the `(hcp, x)`
/// plane (`PairConv::box_sum`'s `x` axis), on top of the plain single-suit card filter, and a
/// fixed part is included so the pool/fixed split itself is exercised too.
#[test]
fn general_path_additive_feature_and_fixed_samples_are_uniform() {
    let pool = honors_pool();
    let fixed = Hand::EMPTY.with(Card::from_index(0).expect("index 0 is a valid card"));
    let atom = Atom {
        cards: vec![CardRequirement::in_suit(
            Suit::Hearts,
            Holding::top_ranks(2), // ♥A, ♥K
            0..=1,
        )],
        eval: vec![EvalRequirement {
            metric: Metric::Controls,
            range: 2..=6,
        }],
        ..Atom::ANY
    };
    let c = HandConstraint::Atom(atom.clone());
    let sampler = Sampler::prepare(&c, pool, fixed, &SampleOptions::default()).unwrap();
    assert!(sampler.is_exact());

    let satisfying = satisfying_completions(pool, fixed, |h| atom.satisfies(h));
    assert_eq!(satisfying.len() as u64, sampler.count());
    assert!(
        (20..=560).contains(&satisfying.len()),
        "expected a sizeable but proper subset of the completions, got {}",
        satisfying.len()
    );

    assert_general_path_draw_is_uniform(&sampler, &satisfying, 0x5EED_0002, 100_000);
}

/// The general path's `draw` when the shared `hcp` window is a genuine multi-value sub-range
/// (neither the unrestricted `0..=37` every other `honors_pool` test here uses, nor a single fixed
/// point like `full_deck_hcp_marginal_matches_exact_counts`'s per-value samplers): steps 3-4 of
/// `GeneralTerm::draw` shift that shared window by each candidate `(hcp, x)` before querying the
/// other suit-pair's `box_sum`, and a bug in that shift (e.g. an off-by-one, or using the
/// unshifted window) would only show up once the window actually excludes some `(hcp, x)`
/// combinations, which a fixed-point or unrestricted window cannot do.
#[test]
fn general_path_hcp_window_with_fixed_cards_samples_are_uniform() {
    let pool = honors_pool();
    let fixed = Hand::EMPTY
        .with(Card::from_index(0).expect("index 0 is a valid card")) // low card, outside the pool
        .with(Card::from_index(13).expect("index 13 is a valid card")); // ditto, another suit
    let atom = Atom::ANY.with_hcp(21..=24);
    let c = HandConstraint::Atom(atom.clone());
    let sampler = Sampler::prepare(&c, pool, fixed, &SampleOptions::default()).unwrap();
    assert!(sampler.is_exact());

    let satisfying = satisfying_completions(pool, fixed, |h| atom.satisfies(h));
    assert_eq!(satisfying.len() as u64, sampler.count());
    assert!(
        (100..=560).contains(&satisfying.len()),
        "expected a sizeable but proper subset of the completions, got {}",
        satisfying.len()
    );

    assert_general_path_draw_is_uniform(&sampler, &satisfying, 0x5EED_0003, 100_000);
}

/// The design's own uniformity check (05-constraint.md §10, 11-testing.md §4): draw `10^6` hands
/// from the exact general path for a realistic constraint (15-17 balanced, full deck) and check
/// both the per-shape marginal (13 balanced shapes) and the per-HCP marginal (15/16/17) against
/// the exact counts `Sampler::prepare` reports for each - not against each other, so this catches
/// what the small χ² tests above cannot: a sampler that draws a uniformly random *shape* first
/// (ignoring each shape's true weight, e.g. `sampler::term::tests`' `M2` mutation from the review)
/// would still pass a hand-level χ² test *within* one shape's own bucket, but would fail this
/// per-shape marginal, since balanced shapes do not all have the same number of 15-17-HCP
/// completions.
///
/// `#[ignore]`d: `10^6` draws is slow for routine `cargo test`. Run explicitly with
/// `cargo test -p bridge-constraint --test sampler_chi_square -- --ignored`.
#[test]
#[ignore = "10^6 draws; run explicitly with `-- --ignored`"]
fn full_deck_15_17_balanced_shape_and_hcp_marginal_chi_square_1e6() {
    const HCP_LO: u8 = 15;
    const HCP_HI: u8 = 17;

    let atom = Atom {
        shapes: ShapeSet::BALANCED,
        ..Atom::ANY.with_hcp(HCP_LO..=HCP_HI)
    };
    let sampler = Sampler::prepare(
        &HandConstraint::Atom(atom),
        Hand::FULL,
        Hand::EMPTY,
        &SampleOptions::default(),
    )
    .unwrap();
    assert!(sampler.is_exact());
    let total_exact = sampler.count();

    // Exact per-shape counts (§10): one `Sampler::prepare` per balanced shape, restricted to the
    // same HCP window, whose `count()`s must sum back to `total_exact`.
    let balanced_shapes: Vec<Shape> = SHAPES
        .iter()
        .copied()
        .filter(|&s| ShapeSet::BALANCED.contains(s))
        .collect();
    let shape_index: HashMap<Shape, usize> = balanced_shapes
        .iter()
        .enumerate()
        .map(|(i, &s)| (s, i))
        .collect();
    let shape_exact: Vec<u64> = balanced_shapes
        .iter()
        .map(|&shape| {
            let a = Atom {
                shapes: ShapeSet::EMPTY.insert(shape),
                ..Atom::ANY.with_hcp(HCP_LO..=HCP_HI)
            };
            Sampler::prepare(
                &HandConstraint::Atom(a),
                Hand::FULL,
                Hand::EMPTY,
                &SampleOptions::default(),
            )
            .unwrap()
            .count()
        })
        .collect();
    assert_eq!(shape_exact.iter().sum::<u64>(), total_exact);

    // Exact per-HCP counts (already established by `full_deck_hcp_marginal_matches_exact_counts`,
    // recomputed here so this test is self-contained).
    let hcp_exact: Vec<u64> = (HCP_LO..=HCP_HI)
        .map(|hcp| {
            let a = Atom {
                shapes: ShapeSet::BALANCED,
                ..Atom::ANY.with_hcp(hcp..=hcp)
            };
            Sampler::prepare(
                &HandConstraint::Atom(a),
                Hand::FULL,
                Hand::EMPTY,
                &SampleOptions::default(),
            )
            .unwrap()
            .count()
        })
        .collect();
    assert_eq!(hcp_exact.iter().sum::<u64>(), total_exact);

    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x1E6_1E6_1E6);
    let draws = 1_000_000u64;
    let mut shape_observed = vec![0u64; balanced_shapes.len()];
    let mut hcp_observed = vec![0u64; (HCP_HI - HCP_LO + 1) as usize];
    for _ in 0..draws {
        let sample = sampler.sample(&mut rng).unwrap();
        let shape = sample.hand.shape();
        let idx = *shape_index
            .get(&shape)
            .unwrap_or_else(|| panic!("drew a non-balanced shape: {shape:?}"));
        shape_observed[idx] += 1;
        let hcp = bridge_eval::hcp(sample.hand);
        hcp_observed[(hcp - HCP_LO) as usize] += 1;
    }

    let shape_expected: Vec<f64> = shape_exact
        .iter()
        .map(|&c| draws as f64 * c as f64 / total_exact as f64)
        .collect();
    let shape_chi2: f64 = shape_observed
        .iter()
        .zip(shape_expected.iter())
        .map(|(&o, &e)| {
            let d = o as f64 - e;
            d * d / e
        })
        .sum();
    let shape_df = (balanced_shapes.len() - 1) as f64;
    let shape_p = chi_square_p_value(shape_chi2, shape_df);
    assert!(
        shape_p > 1e-3,
        "per-shape marginal: chi2={shape_chi2}, df={shape_df}, p={shape_p} (uniformity rejected)"
    );

    let hcp_expected: Vec<f64> = hcp_exact
        .iter()
        .map(|&c| draws as f64 * c as f64 / total_exact as f64)
        .collect();
    // `chi_square_statistic` assumes one shared `expected`; the three HCP bins have distinct
    // exact counts, so compute it by hand instead (same formula as the per-shape one above).
    let hcp_chi2: f64 = hcp_observed
        .iter()
        .zip(hcp_expected.iter())
        .map(|(&o, &e)| {
            let d = o as f64 - e;
            d * d / e
        })
        .sum();
    let hcp_df = (hcp_exact.len() - 1) as f64;
    let hcp_p = chi_square_p_value(hcp_chi2, hcp_df);
    assert!(
        hcp_p > 1e-3,
        "per-HCP marginal: chi2={hcp_chi2}, observed={hcp_observed:?}, expected={hcp_expected:?}, p={hcp_p}"
    );
}
