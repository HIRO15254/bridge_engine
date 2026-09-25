//! Atoms: conjunctions of literals over the original 13-card hand.

use core::ops::RangeInclusive;

use bridge_core::{Hand, Holding, ShapeSet, Suit};
use bridge_eval::{
    DistMethod, LtcMethod, controls, distribution_points, hcp, losers_with, quick_tricks,
    suit_quality, total_points,
};

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
        if self.mask.is_empty() {
            return None;
        }
        Suit::ALL
            .into_iter()
            .find(|suit| self.mask.bits() & !suit.mask() == 0)
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
        match self {
            Metric::Controls => controls(hand),
            Metric::Losers(method) => losers_with(hand, method).halves(),
            Metric::QuickTricks => quick_tricks(hand).halves(),
            Metric::DistPoints(method) => distribution_points(hand, method).max(0) as u8,
            Metric::TotalPoints(method) => total_points(hand, method),
            Metric::SuitQuality(suit) => suit_quality(hand.holding(suit)),
        }
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
        debug_assert_eq!(hand.len(), 13);
        self.shapes.contains(hand.shape())
            && self.hcp.contains(&hcp(hand))
            && self.cards.iter().all(|req| req.holds(hand))
            && self.eval.iter().all(|req| req.holds(hand))
    }

    /// Conjunction: `shapes ∩`, `hcp ∩`, and the literal lists merged by key.
    pub fn intersect(&self, other: &Atom) -> Atom {
        let shapes = self.shapes.intersect(other.shapes);
        let lo = (*self.hcp.start()).max(*other.hcp.start());
        let hi = (*self.hcp.end()).min(*other.hcp.end());
        let mut cards = self.cards.clone();
        cards.extend(other.cards.iter().cloned());
        let mut eval = self.eval.clone();
        eval.extend(other.eval.iter().cloned());
        let mut atom = Atom {
            shapes,
            hcp: lo..=hi,
            cards,
            eval,
        };
        atom.normalize();
        atom
    }

    /// Negation as an exclusive chain of pairwise-disjoint atoms:
    /// `¬(L1 ∧ … ∧ Ln) = ⋁_j (L1 ∧ … ∧ L(j−1) ∧ ¬Lj)`.
    ///
    /// `¬(shape ∈ S)` is the complement set; `¬(x ∈ [lo, hi])` splits into `[0, lo−1]` and
    /// `[hi+1, max]`. At most `3 + 2·(|cards| + |eval|)` atoms result.
    pub fn negate(&self) -> Vec<Atom> {
        let mut a = self.clone();
        a.normalize();

        // An always-false atom negates to `ANY` (always-true). This also sidesteps a case the
        // per-literal complement below cannot handle: if `normalize` merged two literals of the
        // same key (e.g. two `Controls` requirements) into an empty range, that single literal is
        // itself a contradiction, and `[0, lo-1] ∪ [hi+1, max]` (built for a proper subrange)
        // would overlap on `[hi+1, lo-1]` instead of staying disjoint. Every way a single literal
        // can end up self-contradictory is exactly what `is_trivially_unsat`'s stages 1/2/4 check
        // (empty shapes, empty HCP range, empty count/metric range), so checking it first keeps
        // the exclusive-chain construction below valid for every atom it still has to handle.
        if a.is_trivially_unsat() {
            return vec![Atom::ANY];
        }

        let mut out = Vec::new();

        // L1: shape ∈ S.
        if a.shapes != ShapeSet::ALL {
            out.push(Atom {
                shapes: a.shapes.complement(),
                hcp: 0..=37,
                cards: Vec::new(),
                eval: Vec::new(),
            });
        }

        // L2: hcp ∈ [lo, hi]. Every later term keeps L1 true (shapes = a.shapes).
        let lo = *a.hcp.start();
        let hi = *a.hcp.end();
        if lo > 0 {
            out.push(Atom {
                shapes: a.shapes,
                hcp: 0..=(lo - 1),
                cards: Vec::new(),
                eval: Vec::new(),
            });
        }
        if hi < 37 {
            out.push(Atom {
                shapes: a.shapes,
                hcp: (hi + 1)..=37,
                cards: Vec::new(),
                eval: Vec::new(),
            });
        }

        // L3..: card requirements, in normalized (mask) order. Every later term keeps L1, L2 and
        // the earlier card requirements true.
        for (j, req) in a.cards.iter().enumerate() {
            let popcount = req.mask.len();
            let clo = *req.count.start();
            let chi = *req.count.end();
            let prefix = &a.cards[..j];
            if clo > 0 {
                let mut cards = prefix.to_vec();
                cards.push(CardRequirement {
                    mask: req.mask,
                    count: 0..=(clo - 1),
                });
                let mut atom = Atom {
                    shapes: a.shapes,
                    hcp: a.hcp.clone(),
                    cards,
                    eval: Vec::new(),
                };
                atom.normalize();
                out.push(atom);
            }
            if chi < popcount {
                let mut cards = prefix.to_vec();
                cards.push(CardRequirement {
                    mask: req.mask,
                    count: (chi + 1)..=popcount,
                });
                let mut atom = Atom {
                    shapes: a.shapes,
                    hcp: a.hcp.clone(),
                    cards,
                    eval: Vec::new(),
                };
                atom.normalize();
                out.push(atom);
            }
        }

        // L(3+|cards|)..: eval requirements, in normalized (metric) order. Every later term keeps
        // L1, L2, every card requirement and the earlier eval requirements true.
        for (j, req) in a.eval.iter().enumerate() {
            let max = req.metric.max();
            let elo = *req.range.start();
            let ehi = *req.range.end();
            let prefix = &a.eval[..j];
            if elo > 0 {
                let mut eval = prefix.to_vec();
                eval.push(EvalRequirement {
                    metric: req.metric,
                    range: 0..=(elo - 1),
                });
                let mut atom = Atom {
                    shapes: a.shapes,
                    hcp: a.hcp.clone(),
                    cards: a.cards.clone(),
                    eval,
                };
                atom.normalize();
                out.push(atom);
            }
            if ehi < max {
                let mut eval = prefix.to_vec();
                eval.push(EvalRequirement {
                    metric: req.metric,
                    range: (ehi + 1)..=max,
                });
                let mut atom = Atom {
                    shapes: a.shapes,
                    hcp: a.hcp.clone(),
                    cards: a.cards.clone(),
                    eval,
                };
                atom.normalize();
                out.push(atom);
            }
        }

        out
    }

    /// Cheap unsatisfiability checks: empty shape set, inverted or infeasible HCP range for the
    /// shapes (`hcp.start > shapes.max_hcp()` or `hcp.end < shapes.min_hcp()`), empty count or
    /// metric ranges. The definitive check is `Sampler::prepare(...).count() == 0`.
    pub fn is_trivially_unsat(&self) -> bool {
        if self.shapes.is_empty() {
            return true;
        }
        if self.hcp.is_empty() {
            return true;
        }
        if *self.hcp.start() > self.shapes.max_hcp() || *self.hcp.end() < self.shapes.min_hcp() {
            return true;
        }
        for req in &self.cards {
            if req.count.is_empty() || *req.count.start() > req.mask.len() {
                return true;
            }
        }
        for req in &self.eval {
            if req.range.is_empty() || *req.range.start() > req.metric.max() {
                return true;
            }
        }
        false
    }

    /// Sorts and merges the literal lists and clamps every range to its bounds.
    pub fn normalize(&mut self) {
        let hi = (*self.hcp.end()).min(37);
        self.hcp = *self.hcp.start()..=hi;

        self.cards.sort_by_key(|req| req.mask.bits());
        let mut cards: Vec<CardRequirement> = Vec::with_capacity(self.cards.len());
        for req in self.cards.drain(..) {
            let hi = (*req.count.end()).min(req.mask.len());
            match cards.last_mut() {
                Some(last) if last.mask == req.mask => {
                    let lo = (*last.count.start()).max(*req.count.start());
                    let new_hi = (*last.count.end()).min(hi);
                    last.count = lo..=new_hi;
                }
                _ => cards.push(CardRequirement {
                    mask: req.mask,
                    count: *req.count.start()..=hi,
                }),
            }
        }
        self.cards = cards;

        self.eval.sort_by_key(|req| metric_key(req.metric));
        let mut eval: Vec<EvalRequirement> = Vec::with_capacity(self.eval.len());
        for req in self.eval.drain(..) {
            let hi = (*req.range.end()).min(req.metric.max());
            match eval.last_mut() {
                Some(last) if last.metric == req.metric => {
                    let lo = (*last.range.start()).max(*req.range.start());
                    let new_hi = (*last.range.end()).min(hi);
                    last.range = lo..=new_hi;
                }
                _ => eval.push(EvalRequirement {
                    metric: req.metric,
                    range: *req.range.start()..=hi,
                }),
            }
        }
        self.eval = eval;
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

/// A deterministic total order over [`Metric`], used to canonicalise `Atom::eval` in
/// [`Atom::normalize`] independently of insertion order.
fn metric_key(metric: Metric) -> (u8, u32) {
    fn ltc_key(method: LtcMethod) -> u32 {
        match method {
            LtcMethod::Classic => 0,
            LtcMethod::New => 1,
        }
    }
    fn dist_key(method: DistMethod) -> u32 {
        match method {
            DistMethod::ShortSuit {
                void,
                singleton,
                doubleton,
            } => (void as u32) << 16 | (singleton as u32) << 8 | doubleton as u32,
            DistMethod::LongSuit => 1 << 24,
            DistMethod::BergenStarting => 2 << 24,
        }
    }
    match metric {
        Metric::Controls => (0, 0),
        Metric::Losers(method) => (1, ltc_key(method)),
        Metric::QuickTricks => (2, 0),
        Metric::DistPoints(method) => (3, dist_key(method)),
        Metric::TotalPoints(method) => (4, dist_key(method)),
        Metric::SuitQuality(suit) => (5, suit.index() as u32),
    }
}
