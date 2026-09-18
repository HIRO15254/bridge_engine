//! Atoms: conjunctions of literals over the original 13-card hand.

use core::ops::RangeInclusive;

use bridge_core::{Hand, Holding, ShapeSet, Suit};
use bridge_eval::{DistMethod, LtcMethod};

/// `popcount(hand ∩ mask) ∈ count`.
///
/// Covers every card condition the system compiler and the play rules need:
///
/// | Condition | `mask` | `count` |
/// | --- | --- | --- |
/// | ♦ has A or K | {♦A, ♦K} | `1..=2` |
/// | 2 of the top 3 in ♠ | {♠A, ♠K, ♠Q} | `2..=3` |
/// | no ♠A | {♠A} | `0..=0` |
/// | 2+ aces | all aces | `2..=4` |
/// | exactly 3 cards above the led rank `r` in suit `u` (4th best) | ranks `> r` of `u` | `3..=3` |
///
/// Negation is the complement of `count`, so no separate "negated" flag is needed.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CardRequirement {
    /// The cards that count.
    pub mask: Hand,
    /// Allowed number of held cards among `mask`.
    pub count: RangeInclusive<u8>,
}

impl CardRequirement {
    /// A requirement on the ranks of one suit.
    pub fn in_suit(suit: Suit, ranks: Holding, count: RangeInclusive<u8>) -> CardRequirement {
        CardRequirement {
            mask: Hand::EMPTY.with_holding(suit, ranks),
            count,
        }
    }

    /// `Some(suit)` when every card of `mask` lies in one suit (the sampler then applies the
    /// requirement exactly while enumerating that suit).
    pub fn single_suit(&self) -> Option<Suit> {
        todo!("phase 2")
    }

    /// Whether `hand` meets the requirement.
    pub fn holds(&self, hand: Hand) -> bool {
        self.count.contains(&hand.intersect(self.mask).len())
    }
}

/// A whole-hand metric that an [`EvalRequirement`] constrains.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Metric {
    /// Controls (A = 2, K = 1), `0..=12`.
    Controls,
    /// Losing-trick count in half units, `0..=24`.
    Losers(LtcMethod),
    /// Quick tricks in half units, `0..=16`.
    QuickTricks,
    /// Distribution points (clamped at 0).
    DistPoints(DistMethod),
    /// HCP plus distribution points.
    TotalPoints(DistMethod),
    /// Honours (A K Q J T) in one suit, `0..=5`.
    SuitQuality(Suit),
}

impl Metric {
    /// The largest value this metric can take.
    pub const fn max(self) -> u8 {
        match self {
            Metric::Controls => 12,
            Metric::Losers(_) => 24,
            Metric::QuickTricks => 16,
            Metric::DistPoints(_) => 40,
            Metric::TotalPoints(_) => 77,
            Metric::SuitQuality(_) => 5,
        }
    }

    /// Evaluates the metric on `hand`.
    pub fn eval(self, hand: Hand) -> u8 {
        todo!("phase 2")
    }
}

/// `metric(hand) ∈ range`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EvalRequirement {
    /// The metric.
    pub metric: Metric,
    /// Allowed values.
    pub range: RangeInclusive<u8>,
}

impl EvalRequirement {
    /// Whether `hand` meets the requirement.
    pub fn holds(&self, hand: Hand) -> bool {
        self.range.contains(&self.metric.eval(hand))
    }
}

/// A conjunction of literals over the original 13-card hand.
///
/// `shapes` carries every length condition (suit-length ranges are constructed into it and read
/// back as projections); `hcp` is the high-card range; `cards` and `eval` are extra literals.
/// The canonical form keeps `cards` sorted by mask and `eval` sorted by metric, merging equal
/// keys by intersecting their ranges.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Atom {
    /// Allowed shapes.
    pub shapes: ShapeSet,
    /// Allowed high-card points, within `0..=37`.
    pub hcp: RangeInclusive<u8>,
    /// Card requirements.
    pub cards: Vec<CardRequirement>,
    /// Evaluation requirements.
    pub eval: Vec<EvalRequirement>,
}

impl Atom {
    /// The unconstrained atom (every hand).
    pub const ANY: Atom = Atom {
        shapes: ShapeSet::ALL,
        hcp: RangeInclusive::new(0, 37),
        cards: Vec::new(),
        eval: Vec::new(),
    };

    /// Whether `hand` satisfies every literal.
    pub fn satisfies(&self, hand: Hand) -> bool {
        todo!("phase 2")
    }

    /// Conjunction: `shapes ∩`, `hcp ∩`, and the literal lists merged by key.
    pub fn intersect(&self, other: &Atom) -> Atom {
        todo!("phase 2")
    }

    /// Negation as an exclusive chain of pairwise-disjoint atoms:
    /// `¬(L1 ∧ … ∧ Ln) = ⋁_j (L1 ∧ … ∧ L(j−1) ∧ ¬Lj)`.
    ///
    /// `¬(shape ∈ S)` is the complement set; `¬(x ∈ [lo, hi])` splits into `[0, lo−1]` and
    /// `[hi+1, max]`. At most `3 + 2·(|cards| + |eval|)` atoms result.
    pub fn negate(&self) -> Vec<Atom> {
        todo!("phase 2")
    }

    /// Cheap unsatisfiability checks: empty shape set, inverted or infeasible HCP range for the
    /// shapes (`hcp.start > shapes.max_hcp()` or `hcp.end < shapes.min_hcp()`), empty count or
    /// metric ranges. The definitive check is `Sampler::prepare(...).count() == 0`.
    pub fn is_trivially_unsat(&self) -> bool {
        todo!("phase 2")
    }

    /// Sorts and merges the literal lists and clamps every range to its bounds.
    pub fn normalize(&mut self) {
        todo!("phase 2")
    }

    /// The HCP range.
    pub fn hcp_range(&self) -> RangeInclusive<u8> {
        self.hcp.clone()
    }

    /// Projection of the shapes onto one suit's length (a summary, not a constraint).
    pub fn suit_len(&self, suit: Suit) -> RangeInclusive<u8> {
        self.shapes
            .suit_len(suit)
            .unwrap_or(RangeInclusive::new(1, 0))
    }

    /// Restricts the shapes to those whose length in `suit` lies in `range`.
    pub fn with_suit_len(mut self, suit: Suit, range: RangeInclusive<u8>) -> Atom {
        self.shapes =
            self.shapes
                .intersect(ShapeSet::from_suit_len(suit, *range.start(), *range.end()));
        self
    }

    /// Restricts the HCP range.
    pub fn with_hcp(mut self, range: RangeInclusive<u8>) -> Atom {
        self.hcp = range;
        self
    }

    /// Adds a card requirement.
    pub fn with_cards(mut self, req: CardRequirement) -> Atom {
        self.cards.push(req);
        self
    }

    /// Adds an evaluation requirement.
    pub fn with_eval(mut self, req: EvalRequirement) -> Atom {
        self.eval.push(req);
        self
    }
}
