//! Per-card statistics, equivalence grouping and ranking (`14-lead.md` §3 steps 7-9).
//!
//! Grouping is done with a plain O(cards²) scan rather than a `HashMap`, deliberately: with at
//! most 13 cards this is trivial, and it keeps grouping (and hence, before the final
//! deterministic sort, group *creation* order) independent of hash-seed randomisation, matching
//! the project's general avoidance of hash-order-sensitive aggregation
//! (`bridge-bidding/src/policy.rs`'s `call_distribution` doc comment explains the same concern).
//! The final sort in [`rank_and_truncate`] always breaks ties by card index, so the result is
//! deterministic regardless of how the groups were built.

use bridge_core::{Card, Contract};

use crate::advice::LeadScore;
use crate::options::LeadScoring;
use crate::scoring::declarer_score;

/// One card's raw per-sample defence-trick scores, in the same order as the importance weights.
struct CardScores {
    card: Card,
    scores: Vec<u8>,
}

/// Builds one [`CardScores`] per card in `cards`, reading each deal's `(Card, u8)` pairs by
/// linear lookup (at most 13 cards, so this is cheap: at most `13 × samples × 13` comparisons).
///
/// # Panics
/// Panics if some deal's scores are missing one of `cards`: a [`bridge::dd::DoubleDummy`]
/// implementation is contractually required to return every card of the leader's hand
/// (`crates/bridge/tests/dds.rs`'s `assert_eq!(scores.len(), 13, ..)` checks this for the DDS
/// backend), and the leader's hand is fixed and known across every sample, so this is a contract
/// violation by `dd`, not a possible runtime state.
fn scores_by_card(cards: &[Card], per_deal: &[Vec<(Card, u8)>]) -> Vec<CardScores> {
    cards
        .iter()
        .map(|&card| {
            let scores = per_deal
                .iter()
                .map(|deal_scores| {
                    deal_scores
                        .iter()
                        .find(|(c, _)| *c == card)
                        .map(|&(_, score)| score)
                        .unwrap_or_else(|| {
                            panic!(
                                "DoubleDummy::lead_scores did not return {card:?}, one of the \
                                 fixed and known leader's cards"
                            )
                        })
                })
                .collect();
            CardScores { card, scores }
        })
        .collect()
}

/// Weighted mean and standard error of `scores` under `weights` (which must sum to 1, e.g.
/// [`bridge_sample::WeightedDeal::normalized_weights`]) and the deal set's effective sample size.
/// The standard error of a weighted mean under self-normalised importance weights is
/// `sqrt(weighted variance / ESS)` (`14-lead.md` §3 step 7); `ess <= 0.0` (no samples carried any
/// weight) reports a standard error of `0.0` rather than dividing by zero or producing `NaN`.
fn mean_and_std_error(scores: &[u8], weights: &[f64], ess: f64) -> (f64, f64) {
    let mean: f64 = weights
        .iter()
        .zip(scores)
        .map(|(&w, &s)| w * f64::from(s))
        .sum();
    let variance: f64 = weights
        .iter()
        .zip(scores)
        .map(|(&w, &s)| w * (f64::from(s) - mean).powi(2))
        .sum();
    let std_error = if ess > 0.0 {
        (variance / ess).sqrt()
    } else {
        0.0
    };
    (mean, std_error)
}

/// `P(score >= threshold)` under `weights`.
fn set_probability(scores: &[u8], weights: &[f64], threshold: u8) -> f64 {
    weights
        .iter()
        .zip(scores)
        .filter(|&(_, &s)| s >= threshold)
        .map(|(&w, _)| w)
        .sum()
}

/// A group of cards that scored identically (defence tricks) in every sample.
struct Group {
    /// The representative: the highest-ranking card in the group.
    card: Card,
    /// The rest of the group, highest rank first.
    equivalents: Vec<Card>,
    mean: f64,
    std_error: f64,
    set_probability: f64,
    /// Weighted mean declarer duplicate score, for [`LeadScoring::Score`].
    mean_declarer_score: f64,
}

/// Groups cards whose per-sample score vectors are identical (`14-lead.md` §3 step 8), then
/// computes each group's statistics from its (shared) score vector.
fn group_equivalents(
    per_card: Vec<CardScores>,
    weights: &[f64],
    ess: f64,
    threshold: u8,
    contract: Contract,
    vulnerable: bool,
) -> Vec<Group> {
    let mut clusters: Vec<Vec<CardScores>> = Vec::new();
    'cards: for cs in per_card {
        for cluster in clusters.iter_mut() {
            if cluster[0].scores == cs.scores {
                cluster.push(cs);
                continue 'cards;
            }
        }
        clusters.push(vec![cs]);
    }

    clusters
        .into_iter()
        .map(|mut cluster| {
            cluster.sort_by_key(|cs| std::cmp::Reverse(cs.card.rank()));
            let vector = cluster[0].scores.clone();
            let card = cluster[0].card;
            let equivalents: Vec<Card> = cluster[1..].iter().map(|cs| cs.card).collect();
            let (mean, std_error) = mean_and_std_error(&vector, weights, ess);
            let set_probability = set_probability(&vector, weights, threshold);
            let mean_declarer_score: f64 = weights
                .iter()
                .zip(&vector)
                .map(|(&w, &defence_tricks)| {
                    w * f64::from(declarer_score(contract, vulnerable, 13 - defence_tricks))
                })
                .sum();
            Group {
                card,
                equivalents,
                mean,
                std_error,
                set_probability,
                mean_declarer_score,
            }
        })
        .collect()
}

/// Sorts groups by `scoring`, always breaking ties by the representative card's index so the
/// order is fully deterministic regardless of how the groups were built (module doc), then
/// truncates to `top_k` and assigns 1-based `rank`.
fn rank_and_truncate(mut groups: Vec<Group>, scoring: LeadScoring, top_k: usize) -> Vec<LeadScore> {
    match scoring {
        LeadScoring::Tricks => groups.sort_by(|a, b| {
            b.mean
                .total_cmp(&a.mean)
                .then_with(|| a.card.index().cmp(&b.card.index()))
        }),
        LeadScoring::SetProbability => groups.sort_by(|a, b| {
            b.set_probability
                .total_cmp(&a.set_probability)
                .then_with(|| a.card.index().cmp(&b.card.index()))
        }),
        LeadScoring::Score => groups.sort_by(|a, b| {
            a.mean_declarer_score
                .total_cmp(&b.mean_declarer_score)
                .then_with(|| a.card.index().cmp(&b.card.index()))
        }),
    }
    groups.truncate(top_k);
    groups
        .into_iter()
        .enumerate()
        .map(|(i, g)| LeadScore {
            card: g.card,
            equivalents: g.equivalents,
            mean_defence_tricks: g.mean,
            std_error: g.std_error,
            set_probability: g.set_probability,
            rank: i + 1,
        })
        .collect()
}

/// The full pipeline of `14-lead.md` §3 steps 7-9: per-card statistics, equivalence grouping,
/// ranking and truncation.
///
/// Nine arguments, each an independent piece of `advise`'s state (no natural pair or triple to
/// group into a struct that wouldn't just be `AggregateArgs`); this is the crate's single
/// call site, so the ceremony of a wrapper type would not pay for itself.
#[allow(clippy::too_many_arguments)]
pub(crate) fn aggregate(
    cards: &[Card],
    per_deal: &[Vec<(Card, u8)>],
    weights: &[f64],
    ess: f64,
    threshold: u8,
    contract: Contract,
    vulnerable: bool,
    scoring: LeadScoring,
    top_k: usize,
) -> Vec<LeadScore> {
    let per_card = scores_by_card(cards, per_deal);
    let groups = group_equivalents(per_card, weights, ess, threshold, contract, vulnerable);
    rank_and_truncate(groups, scoring, top_k)
}

#[cfg(test)]
mod tests {
    use bridge_core::{Bid, Rank, Seat, Suit};

    use super::*;

    fn card(suit: Suit, rank: Rank) -> Card {
        Card::new(suit, rank)
    }

    fn contract(level: u8, strain: bridge_core::Strain) -> Contract {
        Contract {
            bid: Bid::new(level, strain).unwrap(),
            declarer: Seat::North,
            doubling: bridge_core::Doubling::Undoubled,
        }
    }

    #[test]
    fn mean_and_std_error_match_hand_computed_values() {
        // Equal weights over [4, 5, 6, 5, 4]: mean = 24/5 = 4.8.
        let scores = [4u8, 5, 6, 5, 4];
        let weights = [0.2; 5];
        let ess = 5.0; // equal weights => ESS == n.
        let (mean, std_error) = mean_and_std_error(&scores, &weights, ess);
        assert!((mean - 4.8).abs() < 1e-12, "mean = {mean}");
        // variance = mean((x-mean)^2) = (0.64+0.04+1.44+0.04+0.64)/5 = 0.56
        let expected_variance = 0.56;
        let expected_se = (expected_variance / ess).sqrt();
        assert!(
            (std_error - expected_se).abs() < 1e-12,
            "std_error = {std_error}, expected {expected_se}"
        );
    }

    #[test]
    fn std_error_is_zero_when_ess_is_zero() {
        let (_, std_error) = mean_and_std_error(&[1, 2, 3], &[0.0, 0.0, 0.0], 0.0);
        assert_eq!(std_error, 0.0);
    }

    #[test]
    fn set_probability_counts_weight_at_or_above_threshold() {
        let scores = [3u8, 5, 5, 7];
        let weights = [0.1, 0.2, 0.3, 0.4];
        // threshold 5: 0.2 + 0.3 + 0.4 = 0.9
        let p = set_probability(&scores, &weights, 5);
        assert!((p - 0.9).abs() < 1e-12, "p = {p}");
    }

    #[test]
    fn set_probability_of_impossible_threshold_is_zero() {
        let p = set_probability(&[1, 2, 3], &[0.5, 0.3, 0.2], 14);
        assert_eq!(p, 0.0);
    }

    #[test]
    fn grouping_merges_identical_vectors_and_splits_different_ones() {
        // Spades A/K/Q always score 5; the 2 of spades always scores 0.
        let per_card = vec![
            CardScores {
                card: card(Suit::Spades, Rank::Ace),
                scores: vec![5, 5, 5],
            },
            CardScores {
                card: card(Suit::Spades, Rank::King),
                scores: vec![5, 5, 5],
            },
            CardScores {
                card: card(Suit::Spades, Rank::Queen),
                scores: vec![5, 5, 5],
            },
            CardScores {
                card: card(Suit::Spades, Rank::Two),
                scores: vec![0, 0, 0],
            },
        ];
        let weights = [1.0 / 3.0; 3];
        let contract = contract(3, bridge_core::Strain::NoTrump);
        let groups = group_equivalents(per_card, &weights, 3.0, 5, contract, false);
        assert_eq!(groups.len(), 2, "two distinct score vectors => two groups");

        let honours = groups
            .iter()
            .find(|g| g.card == card(Suit::Spades, Rank::Ace))
            .expect("the AKQ group's representative is the ace (highest rank)");
        assert_eq!(
            honours.equivalents,
            vec![
                card(Suit::Spades, Rank::King),
                card(Suit::Spades, Rank::Queen)
            ],
            "equivalents are ordered highest rank first, excluding the representative"
        );
        assert!((honours.mean - 5.0).abs() < 1e-12);
        assert!((honours.set_probability - 1.0).abs() < 1e-12);

        let deuce = groups
            .iter()
            .find(|g| g.card == card(Suit::Spades, Rank::Two))
            .expect("the deuce is its own group");
        assert!(deuce.equivalents.is_empty());
        assert!((deuce.set_probability - 0.0).abs() < 1e-12);
    }

    #[test]
    fn ranking_by_tricks_is_descending_with_deterministic_tie_break() {
        let make_group = |card: Card, mean: f64| Group {
            card,
            equivalents: Vec::new(),
            mean,
            std_error: 0.0,
            set_probability: 0.0,
            mean_declarer_score: 0.0,
        };
        let low = card(Suit::Clubs, Rank::Two);
        let high = card(Suit::Spades, Rank::Ace);
        let groups = vec![make_group(low, 3.0), make_group(high, 5.0)];
        let leads = rank_and_truncate(groups, LeadScoring::Tricks, 3);
        assert_eq!(leads[0].card, high);
        assert_eq!(leads[0].rank, 1);
        assert_eq!(leads[1].card, low);
        assert_eq!(leads[1].rank, 2);
    }

    #[test]
    fn ranking_by_score_is_ascending_declarer_score() {
        let make_group = |card: Card, mean_declarer_score: f64| Group {
            card,
            equivalents: Vec::new(),
            mean: 0.0,
            std_error: 0.0,
            set_probability: 0.0,
            mean_declarer_score,
        };
        // `a` is worse for declarer (more negative / lower score) than `b`; the defence should
        // prefer `a`.
        let a = card(Suit::Clubs, Rank::Ace);
        let b = card(Suit::Diamonds, Rank::Ace);
        let groups = vec![make_group(b, 400.0), make_group(a, -100.0)];
        let leads = rank_and_truncate(groups, LeadScoring::Score, 3);
        assert_eq!(leads[0].card, a);
        assert_eq!(leads[1].card, b);
    }

    #[test]
    fn top_k_truncates() {
        let make_group = |card: Card, mean: f64| Group {
            card,
            equivalents: Vec::new(),
            mean,
            std_error: 0.0,
            set_probability: 0.0,
            mean_declarer_score: 0.0,
        };
        let groups = vec![
            make_group(card(Suit::Clubs, Rank::Two), 1.0),
            make_group(card(Suit::Diamonds, Rank::Two), 2.0),
            make_group(card(Suit::Hearts, Rank::Two), 3.0),
            make_group(card(Suit::Spades, Rank::Two), 4.0),
        ];
        let leads = rank_and_truncate(groups, LeadScoring::Tricks, 2);
        assert_eq!(leads.len(), 2);
        assert_eq!(leads[0].card, card(Suit::Spades, Rank::Two));
        assert_eq!(leads[1].card, card(Suit::Hearts, Rank::Two));
    }
}
