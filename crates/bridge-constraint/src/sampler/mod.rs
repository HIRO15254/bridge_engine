//! Exact, rejection-free sampling of hands that satisfy a constraint.
//!
//! # Method
//!
//! For each DNF term and each suit, every sub-holding of the unknown pool is enumerated once
//! (at most 2^13 per suit), the fixed (already known) cards of that suit are folded in, and the
//! result is bucketed by `(length, key)` where `key = hcp | (x << 6)` packs the HCP with one
//! optional additive feature `x` (controls, aces, losers or quick tricks). For every reachable
//! ordered shape the four per-suit count vectors are convolved; the weight of a shape is the
//! number of hands inside the HCP window. Sampling then draws a term ∝ its count, a shape ∝ its
//! weight, splits the HCP across the suits by backward sampling through the convolution, and
//! picks one holding uniformly from each bucket. The result is exactly uniform over the term
//! and the set size is known, so [`Sampler::log_prob`] is exact.
//!
//! Literals the exact path cannot express (`Custom`, DNF residuals, a second additive feature)
//! are handled by bounded rejection with an estimated acceptance rate; [`Sampler::is_exact`]
//! reports which case applies.
//!
//! # Cost
//!
//! `prepare` is the expensive step (20–60 µs for a full deck without per-suit filters, up to
//! +65 µs per suit that needs enumeration, 3–10 µs mid-play); `sample` costs about 0.3–0.5 µs.
//! Callers keep a prepared `Sampler` and sample from it many times.

mod suit_table;
mod term;

use bridge_core::Hand;

use crate::{HandConstraint, PrepareError};

/// Options for [`Sampler::prepare`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SampleOptions {
    /// Maximum rejection retries per sample (default 256).
    pub max_tries: u32,
    /// Number of extra additive features the key may carry (`0` or `1`; default 1).
    pub extra_features: u8,
    /// Number of burn-in draws used to estimate the acceptance rate of rejected literals
    /// (default 256).
    pub burn_in: u32,
    /// Whether custom predicates and residuals may be rejection-sampled (default `true`).
    pub allow_rejection: bool,
}

impl Default for SampleOptions {
    fn default() -> SampleOptions {
        SampleOptions {
            max_tries: 256,
            extra_features: 1,
            burn_in: 256,
            allow_rejection: true,
        }
    }
}

/// One sampled hand.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Sample {
    /// The full 13-card hand (`fixed ∪ drawn`).
    pub hand: Hand,
    /// `ln P(hand)` under the sampler's distribution.
    pub log_prob: f64,
    /// Number of draws needed (1 unless rejection was involved).
    pub tries: u32,
}

/// A prepared sampler for one constraint, pool and fixed part.
///
/// Immutable after `prepare` (`Send + Sync`); acceptance statistics come back through
/// [`Sample::tries`].
pub struct Sampler {
    terms: Vec<term::PreparedTerm>,
    cum: Vec<u64>,
    total: u64,
    pool: Hand,
    fixed: Hand,
    exact: bool,
}

impl Sampler {
    /// Prepares a sampler for hands `h` with `fixed ⊆ h ⊆ fixed ∪ pool`, `|h| = 13`, satisfying
    /// `constraint`.
    ///
    /// `pool ∩ fixed` must be empty. An unsatisfiable constraint is not an error: the sampler
    /// is returned with `count() == 0` and `sample` yields `None`.
    pub fn prepare(
        constraint: &HandConstraint,
        pool: Hand,
        fixed: Hand,
        opts: &SampleOptions,
    ) -> Result<Sampler, PrepareError> {
        todo!("phase 2")
    }

    /// Number of hands in the union of the terms (exact for the exact path; terms produced by
    /// negation are disjoint, so this is the size of the constraint's set).
    pub fn count(&self) -> u64 {
        self.total
    }

    /// `true` when no literal needs rejection.
    pub fn is_exact(&self) -> bool {
        self.exact
    }

    /// Draws one hand, or `None` when `count() == 0` or `max_tries` was exhausted.
    pub fn sample<R: rand_core::Rng + ?Sized>(&self, rng: &mut R) -> Option<Sample> {
        todo!("phase 2")
    }

    /// `ln P(hand)`: `ln(Σ_{terms ∋ hand} 1/α_i) − ln Σ_i c_i` (α = 1 for exact terms), or
    /// `-∞` when no term contains `hand`.
    pub fn log_prob(&self, hand: Hand) -> f64 {
        todo!("phase 2")
    }

    /// The pool this sampler draws from.
    pub fn pool(&self) -> Hand {
        self.pool
    }

    /// The fixed part every sampled hand contains.
    pub fn fixed(&self) -> Hand {
        self.fixed
    }
}
