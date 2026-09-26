//! Shared fixtures for `bridge-sample`'s integration tests.
//!
//! Not a test binary itself: Cargo only auto-registers files directly under `tests/`, not
//! subdirectories, so this is pulled in with `mod support;` by the files that need it (each
//! integration test file is its own crate, so this module is compiled once per caller — hence
//! the blanket `#![allow(dead_code)]` below, since not every caller uses every item).

#![allow(dead_code)]

use bridge_bidding::{Explanation, Interpretation, ResolutionKind};
use bridge_constraint::{Atom, CardRequirement, HandConstraint, KnownCards, ShapeSet};
use bridge_core::{Card, Hand, Holding, Rank, Seat, Suit};

fn cards_to_hand(cards: &[Card]) -> Hand {
    cards.iter().fold(Hand::EMPTY, |h, &c| h.with(c))
}

fn empty_explanation() -> Explanation {
    Explanation {
        text: String::new(),
        node: None,
        resolution: ResolutionKind::Exact,
        parts: Vec::new(),
    }
}

fn atom_cards(cards: Vec<CardRequirement>) -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: ShapeSet::ALL,
        hcp: 0..=37,
        cards,
        eval: Vec::new(),
    })
}

fn atom_hcp(hcp: core::ops::RangeInclusive<u8>) -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: ShapeSet::ALL,
        hcp,
        cards: Vec::new(),
        eval: Vec::new(),
    })
}

/// A `SampleContext`-shaped fixture built to exercise two gaps a review found in
/// `tests/log_prob.rs`'s and `tests/determinism.rs`'s existing coverage: a re-prepared *middle*
/// seat (§6.4 (c) of `09-sample.md`) whose mixture has two genuinely overlapping components that
/// survive [`coarsen`](crate::constraint_proposal) unchanged (an HCP window, not a bare `cards`
/// atom, which coarsening would otherwise reduce to `ANY`), and a *last* seat that is `Sampled`
/// (not `Direct`) and can fail its own `satisfies` check for some residual pools, so `log_prob`'s
/// last-seat branch actually returns `-inf` for some deals instead of never being exercised (both
/// existing tests only ever left the last seat `Direct`/unconstrained).
///
/// Nine unknown cards: the ace, king and queen of spades, hearts and diamonds (clubs is a
/// complete suit fixed to North, `needed == 0`, and irrelevant to every constraint below). East
/// (`needed = 2`) must hold the spade ace; South (`needed = 3`) must hold 8-10 or 6-8 HCP among
/// its three pool cards (the windows overlap at 8, so both components can produce the same
/// hand); West (`needed = 4`, the residual seat) must hold the diamond ace. Every seat's own
/// fixed (non-pool) cards are worth 0 HCP (East's and South's fixed cards are plain spot cards;
/// West's fixed cards absorb the three jacks, irrelevant since West's own constraint is
/// card-identity, not HCP), so South's HCP constraint depends only on its own three drawn cards.
///
/// Mass ordering (§6.1 point 3, computed against the full 9-card pool): East `1.0 · C(8, 1) = 8`;
/// South `0.6 · 64 + 0.4 · 28 = 49.6` (64 and 28 are the counts of 3-card subsets of the 9-card
/// pool — three aces (4 pts), three kings (3 pts), three queens (2 pts) — with HCP in `8..=10`
/// and `6..=8` respectively); West `1.0 · C(8, 3) = 56`. East sorts first (cached), South second
/// (the re-prepared middle seat), West last.
pub struct MultiComponentContext {
    pub known: KnownCards,
    pub interpretation: Interpretation,
    pub spade_ace: Card,
    pub diamond_ace: Card,
}

pub fn multi_component_rejecting_last_seat() -> MultiComponentContext {
    let suits = [Suit::Spades, Suit::Hearts, Suit::Diamonds];
    let mut jacks = Hand::EMPTY;
    let mut zero_value = Hand::EMPTY;
    for &suit in &suits {
        let full = Hand::EMPTY.with_holding(suit, Holding::FULL);
        let top3 = Hand::EMPTY.with_holding(suit, Holding::top_ranks(3)); // A, K, Q
        let top4 = Hand::EMPTY.with_holding(suit, Holding::top_ranks(4)); // A, K, Q, J
        jacks = jacks.union(top4.difference(top3));
        zero_value = zero_value.union(full.difference(top4));
    }
    let clubs = Hand::EMPTY.with_holding(Suit::Clubs, Holding::FULL);

    let zero_cards: Vec<Card> = zero_value.cards().collect();
    assert_eq!(zero_cards.len(), 27);
    let south_fixed = cards_to_hand(&zero_cards[0..10]);
    let east_fixed = cards_to_hand(&zero_cards[10..21]);
    let west_zero = cards_to_hand(&zero_cards[21..27]);
    let west_fixed = west_zero.union(jacks);

    let known = KnownCards::new([clubs, east_fixed, south_fixed, west_fixed])
        .expect("the four fixed hands are pairwise disjoint by construction");
    assert_eq!(known.needed(Seat::North), 0);
    assert_eq!(known.needed(Seat::East), 2);
    assert_eq!(known.needed(Seat::South), 3);
    assert_eq!(known.needed(Seat::West), 4);
    assert_eq!(known.pool().len(), 9);

    let spade_ace = Card::new(Suit::Spades, Rank::Ace);
    let diamond_ace = Card::new(Suit::Diamonds, Rank::Ace);

    let holds_spade_ace = atom_cards(vec![CardRequirement::in_suit(
        Suit::Spades,
        Holding::top_ranks(1),
        1..=1,
    )]);
    let holds_diamond_ace = atom_cards(vec![CardRequirement::in_suit(
        Suit::Diamonds,
        Holding::top_ranks(1),
        1..=1,
    )]);
    let south_high = atom_hcp(8..=10);
    let south_low = atom_hcp(6..=8);

    let interpretation = Interpretation {
        seats: [
            Vec::new(),
            vec![(holds_spade_ace, 1.0, empty_explanation())],
            vec![
                (south_high, 0.6, empty_explanation()),
                (south_low, 0.4, empty_explanation()),
            ],
            vec![(holds_diamond_ace, 1.0, empty_explanation())],
        ],
        per_call: Vec::new(),
        divergence: None,
    };

    MultiComponentContext {
        known,
        interpretation,
        spade_ace,
        diamond_ace,
    }
}

/// Lanczos approximation to `ln(Gamma(x))`, matching
/// `bridge-constraint/tests/sampler_chi_square.rs` (duplicated rather than shared: there is no
/// cross-crate test-support crate in this workspace and the function is a dozen lines).
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

fn gamma_q(a: f64, x: f64) -> f64 {
    if x < a + 1.0 {
        1.0 - gamma_p_series(a, x)
    } else {
        gamma_q_cf(a, x)
    }
}

/// The upper-tail p-value of a chi-square statistic with `df` degrees of freedom.
pub fn chi_square_p_value(chi2: f64, df: f64) -> f64 {
    gamma_q(df / 2.0, chi2 / 2.0)
}

pub fn chi_square_statistic(observed: &[u64], expected: &[f64]) -> f64 {
    observed
        .iter()
        .zip(expected)
        .map(|(&o, &e)| {
            let d = o as f64 - e;
            d * d / e
        })
        .sum()
}

/// All `C(pool.len(), k)` subsets of `pool`, as hands (standard lexicographic combination
/// enumeration: at each step, the rightmost index still below its maximum is incremented and
/// everything to its right is reset to consecutive values).
pub fn subsets_of_size(pool: &[Card], k: usize) -> Vec<Hand> {
    let n = pool.len();
    let mut out = Vec::new();
    if k > n {
        return out;
    }
    let mut indices: Vec<usize> = (0..k).collect();
    loop {
        let cards: Vec<Card> = indices.iter().map(|&i| pool[i]).collect();
        out.push(cards_to_hand(&cards));

        let mut advanced = false;
        let mut i = k;
        while i > 0 {
            i -= 1;
            if indices[i] < i + n - k {
                indices[i] += 1;
                for j in (i + 1)..k {
                    indices[j] = indices[j - 1] + 1;
                }
                advanced = true;
                break;
            }
        }
        if !advanced {
            break;
        }
    }
    out
}
