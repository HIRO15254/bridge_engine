//! `interpret`: auction → constraints.
//!
//! **Step A (per call).** For call `j` by seat `s`, resolve in `table.systems[s]`. `Exact`
//! yields one alternative per top-level `Or` branch (weights from `branch_weights` or equal);
//! `Partial` first tries `resolve_lenient`, then the node's own constraint; `Natural` asks the
//! natural engine. Every alternative is scaled by `1 − ε` and a defensive branch `(ANY, ε,
//! Fallback)` is appended, with `ε` depending on the resolution kind. This is how lower
//! confidence is represented: more mass on the unconstrained alternative, never an ad-hoc
//! loosening of the constraint; the sampler's importance weights correct the mixture afterwards.
//!
//! **Step B (per seat).** The alternatives of a seat's calls are combined by cross product
//! (`and`, unsatisfiable combinations dropped, deduplicated by node/kind, truncated to `K` by
//! weight, renormalised). Each call contributes only its own node's constraint; calls before
//! the divergence point keep their `Exact` confidence, which is the operational meaning of
//! "weaken later constraints, not earlier ones".

use bridge_constraint::HandConstraint;
use bridge_core::{Auction, Call, Hand, Seat};

use crate::{NodeId, Table};

/// How a call was resolved. Ordered from most to least confident.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ResolutionKind {
    /// The sequence is in the system.
    Exact,
    /// A prefix is in the system.
    Partial {
        /// Matched prefix length.
        matched_depth: usize,
    },
    /// Natural inference.
    Natural,
    /// The defensive `ANY` branch.
    Fallback,
}

/// Explanation of one call.
#[derive(Clone, Debug)]
pub struct CallExplanation {
    /// Index of the call in the auction.
    pub call_index: usize,
    /// The call.
    pub call: Call,
    /// The node, if any.
    pub node: Option<NodeId>,
    /// Resolution kind.
    pub kind: ResolutionKind,
    /// The node's description or the natural rule text; empty for `Fallback`.
    pub text: String,
}

/// Explanation of one alternative for one seat.
#[derive(Clone, Debug)]
pub struct Explanation {
    /// The parts joined with ` / `.
    pub text: String,
    /// The node of the seat's most recent call.
    pub node: Option<NodeId>,
    /// The least confident kind among the parts.
    pub resolution: ResolutionKind,
    /// One part per call of this seat.
    pub parts: Vec<CallExplanation>,
}

/// The weighted disjunction for one call, before combination.
#[derive(Clone, Debug)]
pub struct CallInterpretation {
    /// Index of the call.
    pub call_index: usize,
    /// Its seat.
    pub seat: Seat,
    /// The call.
    pub call: Call,
    /// Resolution kind.
    pub kind: ResolutionKind,
    /// Alternatives; weights sum to 1.
    pub alternatives: Vec<(HandConstraint, f32, CallExplanation)>,
}

/// The result of [`interpret`].
#[derive(Clone, Debug)]
pub struct Interpretation {
    /// Per seat: weighted alternatives summing to 1.
    pub seats: [Vec<(HandConstraint, f32, Explanation)>; 4],
    /// Per call, before combination (for display and likelihoods).
    pub per_call: Vec<CallInterpretation>,
    /// The first call index that was not resolved `Exact`, if any.
    pub divergence: Option<usize>,
}

impl Interpretation {
    /// Whether `hand` satisfies at least one non-`Fallback` alternative of `seat` (the strict
    /// check used by the consistency test).
    ///
    /// Defined over `per_call` rather than the truncated `seats` mixture: a seat's true
    /// disjunction is the union, over its calls, of the AND of one alternative per call, and
    /// membership in that union is equivalent to every call having some non-fallback,
    /// positive-weight alternative that contains `hand`. Checking against `seats` instead would
    /// false-positive on nodes with many branches, since `seats` is capped to `K` alternatives.
    /// A seat with no calls is vacuously satisfied.
    pub fn satisfied_by(&self, seat: Seat, hand: Hand) -> bool {
        self.per_call
            .iter()
            .filter(|call| call.seat == seat)
            .all(|call| {
                call.alternatives
                    .iter()
                    .any(|(constraint, weight, explanation)| {
                        *weight > 0.0
                            && explanation.kind != ResolutionKind::Fallback
                            && constraint.satisfies(hand)
                    })
            })
    }

    /// `Π_{j ∈ calls(seat)} Σ_i w_{j,i} · [C_{j,i} ∋ hand]`: the set-membership mass of `hand`
    /// under the mixture, one factor per call (including its `Fallback` branch). This is not
    /// the bidding-policy likelihood (see `sequence_log_likelihood`); it matches the
    /// pre-truncation mass and needs no renormalisation. A seat with no calls has likelihood 1.
    pub fn likelihood(&self, seat: Seat, hand: Hand) -> f32 {
        self.per_call
            .iter()
            .filter(|call| call.seat == seat)
            .map(|call| {
                call.alternatives
                    .iter()
                    .filter(|(constraint, _, _)| constraint.satisfies(hand))
                    .map(|(_, weight, _)| *weight)
                    .sum::<f32>()
            })
            .product()
    }
}

/// Options for [`interpret`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct InterpretOptions {
    /// Maximum alternatives kept per seat (default 8).
    pub max_alternatives: usize,
    /// Fallback mass for `Exact` resolutions (default 0.02).
    pub eps_exact: f32,
    /// Fallback mass for `Partial` resolutions (default 0.15).
    pub eps_partial: f32,
    /// Fallback mass for `Natural` resolutions (default 0.30).
    pub eps_natural: f32,
    /// No fallback branches at all (property tests).
    pub strict: bool,
    /// Weight multiplier per opponents'-call substitution in `resolve_lenient` (default 0.5).
    pub lenient_decay: f32,
}

impl Default for InterpretOptions {
    fn default() -> InterpretOptions {
        InterpretOptions {
            max_alternatives: 8,
            eps_exact: 0.02,
            eps_partial: 0.15,
            eps_natural: 0.30,
            strict: false,
            lenient_decay: 0.5,
        }
    }
}

/// Interprets `auction` under the four systems of `table`.
pub fn interpret(table: &Table, auction: &Auction, opts: &InterpretOptions) -> Interpretation {
    todo!("phase 3")
}

#[cfg(test)]
mod tests {
    use bridge_constraint::{Atom, HandConstraint, ShapeSet};
    use bridge_core::{Bid, Call, Hand, Holding, Rank, Strain, Suit};

    use super::*;

    fn balanced_15_17() -> HandConstraint {
        HandConstraint::Atom(Atom {
            shapes: ShapeSet::BALANCED,
            hcp: 15..=17,
            cards: Vec::new(),
            eval: Vec::new(),
        })
    }

    fn call_explanation(kind: ResolutionKind) -> CallExplanation {
        CallExplanation {
            call_index: 0,
            call: Call::Bid(Bid::new(1, Strain::NoTrump).unwrap()),
            node: None,
            kind,
            text: String::new(),
        }
    }

    /// North bid 1NT with a single non-fallback alternative (weight 0.98) plus the defensive
    /// fallback branch (weight 0.02, `ResolutionKind::Fallback`).
    fn one_call_interpretation() -> Interpretation {
        let alternatives = vec![
            (
                balanced_15_17(),
                0.98,
                call_explanation(ResolutionKind::Exact),
            ),
            (
                HandConstraint::ANY,
                0.02,
                call_explanation(ResolutionKind::Fallback),
            ),
        ];
        let per_call = vec![CallInterpretation {
            call_index: 0,
            seat: Seat::North,
            call: Call::Bid(Bid::new(1, Strain::NoTrump).unwrap()),
            kind: ResolutionKind::Exact,
            alternatives,
        }];
        Interpretation {
            seats: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            per_call,
            divergence: None,
        }
    }

    fn holding_of(ranks: &[Rank]) -> Holding {
        ranks.iter().fold(Holding::EMPTY, |h, &r| h.with(r))
    }

    /// 4=3=4=2 (clubs/diamonds/hearts/spades) shape, exactly 16 HCP: a balanced hand inside
    /// [`balanced_15_17`]'s range.
    fn balanced_16_hcp_hand() -> Hand {
        Hand::EMPTY
            .with_holding(
                Suit::Clubs,
                holding_of(&[Rank::Ace, Rank::King, Rank::Queen, Rank::Two]), // 9 HCP
            )
            .with_holding(
                Suit::Diamonds,
                holding_of(&[Rank::Ace, Rank::Two, Rank::Three]),
            ) // 4 HCP
            .with_holding(
                Suit::Hearts,
                holding_of(&[Rank::King, Rank::Two, Rank::Three, Rank::Four]), // 3 HCP
            )
            .with_holding(Suit::Spades, holding_of(&[Rank::Two, Rank::Three])) // 0 HCP
    }

    /// A flat, honour-free 4=3=3=3 hand: 0 HCP, well outside `balanced_15_17`'s range.
    fn zero_hcp_hand() -> Hand {
        Hand::EMPTY
            .with_holding(
                Suit::Clubs,
                holding_of(&[Rank::Two, Rank::Three, Rank::Four, Rank::Five]),
            )
            .with_holding(
                Suit::Diamonds,
                holding_of(&[Rank::Two, Rank::Three, Rank::Four]),
            )
            .with_holding(
                Suit::Hearts,
                holding_of(&[Rank::Two, Rank::Three, Rank::Four]),
            )
            .with_holding(
                Suit::Spades,
                holding_of(&[Rank::Two, Rank::Three, Rank::Four]),
            )
    }

    #[test]
    fn satisfied_by_is_vacuously_true_for_a_seat_with_no_calls() {
        let interpretation = one_call_interpretation();
        let hand = balanced_16_hcp_hand();
        assert!(interpretation.satisfied_by(Seat::East, hand));
        assert!(interpretation.satisfied_by(Seat::South, hand));
        assert!(interpretation.satisfied_by(Seat::West, hand));
    }

    #[test]
    fn satisfied_by_ignores_the_fallback_branch() {
        let interpretation = one_call_interpretation();
        // A 0-HCP hand satisfies only the `ANY`/`Fallback` alternative, never the strict
        // 15-17 balanced one, so `satisfied_by` (which excludes `Fallback`) must reject it.
        let weak_hand = zero_hcp_hand();
        assert_eq!(weak_hand.len(), 13);
        assert!(!interpretation.satisfied_by(Seat::North, weak_hand));
    }

    #[test]
    fn satisfied_by_accepts_a_hand_matching_the_strict_alternative() {
        let interpretation = one_call_interpretation();
        let hand = balanced_16_hcp_hand();
        assert!(interpretation.satisfied_by(Seat::North, hand));
    }

    #[test]
    fn likelihood_sums_only_the_alternatives_containing_the_hand() {
        let interpretation = one_call_interpretation();
        let hand = balanced_16_hcp_hand();
        // The hand is in the strict alternative (weight 0.98) and in `ANY` (weight 0.02): the
        // mixture mass is their sum.
        let likelihood = interpretation.likelihood(Seat::North, hand);
        assert!((likelihood - 1.0).abs() < 1e-6, "likelihood = {likelihood}");
    }

    #[test]
    fn likelihood_is_the_fallback_mass_alone_outside_the_strict_alternative() {
        let interpretation = one_call_interpretation();
        let weak_hand = zero_hcp_hand();
        let likelihood = interpretation.likelihood(Seat::North, weak_hand);
        assert!(
            (likelihood - 0.02).abs() < 1e-6,
            "likelihood = {likelihood}"
        );
    }

    #[test]
    fn likelihood_of_a_seat_with_no_calls_is_one() {
        let interpretation = one_call_interpretation();
        let hand = balanced_16_hcp_hand();
        assert_eq!(interpretation.likelihood(Seat::East, hand), 1.0);
    }
}
