//! Importance weights and the effective sample size.

use bridge_core::Deal;

/// A sampled deal with its log importance weight.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WeightedDeal {
    /// The deal.
    pub deal: Deal,
    /// `ln w = ln L − ln π` (constant factors cancel under self-normalisation).
    pub log_weight: f64,
}

impl WeightedDeal {
    /// Self-normalised weights summing to 1. Every deal's constant factors cancel here, so an
    /// empty `deals` (or one whose weights are all `-∞`) yields all zeros rather than dividing
    /// by zero.
    pub fn normalized_weights(deals: &[WeightedDeal]) -> Vec<f64> {
        let lse = log_sum_exp(deals.iter().map(|d| d.log_weight));
        if lse == f64::NEG_INFINITY {
            return vec![0.0; deals.len()];
        }
        deals.iter().map(|d| (d.log_weight - lse).exp()).collect()
    }
}

/// `m + ln Σ exp(x − m)` with `m = max x`; `-∞` for an empty slice (and, correctly, for a slice
/// whose values are all `-∞`, since `m` is then `-∞` and no shift is needed).
pub fn log_sum_exp(xs: impl IntoIterator<Item = f64>) -> f64 {
    let xs: Vec<f64> = xs.into_iter().collect();
    let m = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if m == f64::NEG_INFINITY {
        // Either `xs` is empty, or every value is `-∞`; `x - m` would be `NaN` either way.
        return f64::NEG_INFINITY;
    }
    let sum: f64 = xs.iter().map(|&x| (x - m).exp()).sum();
    m + sum.ln()
}

/// `ESS = (Σ w)² / Σ w² = exp(2·LSE(lw) − LSE(2·lw))`.
///
/// An empty slice, or one whose weights are all `-∞` (no sample carries any mass), has `ESS =
/// 0` rather than the `NaN` the formula would give from `-∞ − (-∞)`.
pub fn effective_sample_size(log_weights: &[f64]) -> f64 {
    let lse1 = log_sum_exp(log_weights.iter().copied());
    if lse1 == f64::NEG_INFINITY {
        return 0.0;
    }
    let lse2 = log_sum_exp(log_weights.iter().map(|&x| 2.0 * x));
    (2.0 * lse1 - lse2).exp()
}

#[cfg(test)]
mod tests {
    use bridge_core::Hand;

    use super::*;

    fn deal_fixture() -> Deal {
        // Any valid deal; `WeightedDeal` tests only care about `log_weight`. Cards 0..13, 13..26,
        // … go to North, East, South, West in order.
        let cards: Vec<_> = Hand::FULL.cards().collect();
        let mut hands = [Hand::EMPTY; 4];
        for (seat, chunk) in hands.iter_mut().zip(cards.chunks(13)) {
            for &card in chunk {
                *seat = seat.with(card);
            }
        }
        Deal::new(hands).expect("52 cards split into four 13-card hands")
    }

    #[test]
    fn log_sum_exp_empty_is_neg_infinity() {
        assert_eq!(log_sum_exp(core::iter::empty()), f64::NEG_INFINITY);
    }

    #[test]
    fn log_sum_exp_all_neg_infinity_is_neg_infinity() {
        let xs = [f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        assert_eq!(log_sum_exp(xs), f64::NEG_INFINITY);
    }

    #[test]
    fn log_sum_exp_matches_naive_sum_for_equal_weights() {
        let xs = [1.0, 1.0, 1.0, 1.0];
        let lse = log_sum_exp(xs);
        assert!((lse - (4.0f64.ln() + 1.0)).abs() < 1e-12);
    }

    #[test]
    fn ess_of_empty_is_zero() {
        assert_eq!(effective_sample_size(&[]), 0.0);
    }

    #[test]
    fn ess_of_all_neg_infinity_is_zero() {
        let lw = [f64::NEG_INFINITY; 5];
        assert_eq!(effective_sample_size(&lw), 0.0);
    }

    #[test]
    fn ess_of_equal_weights_is_n() {
        for n in [1usize, 2, 5, 17] {
            let lw = vec![-3.5; n]; // any common constant; ESS is scale-invariant.
            let ess = effective_sample_size(&lw);
            assert!((ess - n as f64).abs() < 1e-9, "n = {n}, ess = {ess}");
        }
    }

    #[test]
    fn ess_of_one_dominant_weight_is_about_one() {
        let mut lw = vec![-50.0; 20];
        lw[0] = 0.0; // dominates every other term by e^50.
        let ess = effective_sample_size(&lw);
        assert!((ess - 1.0).abs() < 1e-9, "ess = {ess}");
    }

    #[test]
    fn normalized_weights_sum_to_one() {
        let deal = deal_fixture();
        let deals = [
            WeightedDeal {
                deal,
                log_weight: -1.0,
            },
            WeightedDeal {
                deal,
                log_weight: 0.5,
            },
            WeightedDeal {
                deal,
                log_weight: -3.0,
            },
        ];
        let weights = WeightedDeal::normalized_weights(&deals);
        let sum: f64 = weights.iter().sum();
        assert!((sum - 1.0).abs() < 1e-12, "sum = {sum}");
        assert!(weights.iter().all(|&w| w >= 0.0));
    }

    #[test]
    fn normalized_weights_of_empty_is_empty() {
        assert!(WeightedDeal::normalized_weights(&[]).is_empty());
    }

    #[test]
    fn normalized_weights_all_neg_infinity_is_all_zero() {
        let deal = deal_fixture();
        let deals = [
            WeightedDeal {
                deal,
                log_weight: f64::NEG_INFINITY,
            },
            WeightedDeal {
                deal,
                log_weight: f64::NEG_INFINITY,
            },
        ];
        let weights = WeightedDeal::normalized_weights(&deals);
        assert_eq!(weights, vec![0.0, 0.0]);
    }
}
