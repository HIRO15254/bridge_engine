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
//! `prepare` is the expensive step: about 18 µs for the full 52-card deck with a shape and HCP
//! window (shared `FULL_SUIT` tables, no per-suit enumeration needed), about 100 µs for a
//! mid-play position (26 unknown cards, 6 fixed, one HCP window) where every suit's table is
//! rebuilt for the smaller pool. `sample` costs about 150–200 ns once a `Sampler` is prepared
//! (release build; see `benches/sampler.rs`). Callers keep a prepared `Sampler` and sample from it
//! many times.

mod rand_util;
mod suit_table;
mod term;

use bridge_core::Hand;

use crate::{DnfOptions, HandConstraint, PrepareError};
use rand_util::random_below;

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
    /// Whether a literal the exact path cannot express (a custom predicate, a DNF residual, a
    /// second additive feature, or a non-shape-only `DistMethod`/`TotalPoints`, i.e.
    /// `BergenStarting`) may be rejection-sampled (default `true`); `false` makes any of them a
    /// `PrepareError::NotSamplable`.
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
    /// `Σ_i c_i · s_i`, the normalising constant [`Sampler::log_prob`] actually uses (`s_i = 1`
    /// for an exact term, so `z == total as f64` when [`Sampler::is_exact`]).
    z: f64,
    pool: Hand,
    fixed: Hand,
    exact: bool,
    max_tries: u32,
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
        if !pool.is_disjoint(fixed) {
            return Err(PrepareError::Overlap);
        }
        let fixed_len = fixed.len();
        if fixed_len > 13 {
            return Err(PrepareError::TooManyFixed(fixed_len));
        }
        let samplable = constraint.is_samplable();
        if !opts.allow_rejection && !samplable {
            return Err(PrepareError::NotSamplable);
        }
        if !samplable {
            // `is_samplable() == false` means a `Custom` literal occurs somewhere in `constraint`
            // (`HandConstraint::is_samplable`'s doc comment); `allow_rejection` being `true` here
            // (the `NotSamplable` case above already returned otherwise) means every such literal
            // is about to be checked by bounded rejection instead of the exact path, with an
            // estimated acceptance rate (05-constraint.md §8.2 step 3).
            tracing::warn!(
                "constraint contains a Custom predicate; sampling degrades to rejection with an estimated acceptance rate"
            );
        }

        let dnf = constraint
            .to_dnf(&DnfOptions::default())
            .expect("DnfOptions::default uses Overflow::Residual, which never errors");

        let mut terms = Vec::with_capacity(dnf.terms.len());
        for dnf_term in dnf.terms {
            let prepared = term::PreparedTerm::prepare(dnf_term, pool, fixed, opts);
            if !opts.allow_rejection && prepared.alpha.is_some() {
                return Err(PrepareError::NotSamplable);
            }
            terms.push(prepared);
        }

        let mut cum = Vec::with_capacity(terms.len());
        let mut running = 0u64;
        let mut z = 0.0f64;
        for t in &terms {
            running += t.total;
            cum.push(running);
            z += t.total as f64 * t.s;
        }
        let total = running;
        let exact = terms.iter().all(|t| t.alpha.is_none());

        Ok(Sampler {
            terms,
            cum,
            total,
            z,
            pool,
            fixed,
            exact,
            max_tries: opts.max_tries,
        })
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
    ///
    /// The term is chosen once, proportional to its count (§8.2); retries (up to
    /// `opts.max_tries`) redraw within that same term, matching the definition of its estimated
    /// acceptance rate `alpha` (§8.3).
    pub fn sample<R: rand_core::Rng + ?Sized>(&self, rng: &mut R) -> Option<Sample> {
        if self.total == 0 {
            return None;
        }
        let target = random_below(rng, self.total);
        let term_idx = self.cum.partition_point(|&c| c <= target);
        let term = &self.terms[term_idx];

        let tries_limit = self.max_tries.max(1);
        for tries in 1..=tries_limit {
            let hand = term.draw(rng);
            if term.alpha.is_none() {
                debug_assert!(
                    term.term.atom.satisfies(hand),
                    "an exact term produced a hand violating its own atom"
                );
                let log_prob = self.log_prob(hand);
                return Some(Sample {
                    hand,
                    log_prob,
                    tries,
                });
            }
            if term.term.satisfies(hand) {
                let log_prob = self.log_prob(hand);
                return Some(Sample {
                    hand,
                    log_prob,
                    tries,
                });
            }
        }
        None
    }

    /// `ln P(hand)` under the distribution [`Sampler::sample`] actually returns, i.e. conditioned
    /// on it returning `Some` (a caller redrawing on `None`, as `bridge-sample` does, samples
    /// exactly that conditional distribution): `ln(Σ_{terms ∋ hand} s_i/α_i) − ln Σ_i c_i·s_i`,
    /// where `α_i = 1` and `s_i = 1` for an exact term. `s_i < 1` accounts for `sample` sometimes
    /// exhausting `max_tries` inside a rejection term without ever accepting; ignoring it (using
    /// `1/α_i` and `Σ c_i` as design §8.3 originally did) systematically over-weights a rejection
    /// term whose `α_i · max_tries` is small, since such a term returns `None` disproportionately
    /// often instead of contributing a sample at its raw `1/α_i` share. Returns `-∞` when no term
    /// contains `hand`.
    pub fn log_prob(&self, hand: Hand) -> f64 {
        if hand.len() != 13
            || self.total == 0
            || !self.fixed.is_subset(hand)
            || !hand.is_subset(self.fixed.union(self.pool))
        {
            return f64::NEG_INFINITY;
        }
        let mut sum = 0.0f64;
        for term in &self.terms {
            if term.total == 0 {
                continue;
            }
            if term.term.satisfies(hand) {
                let alpha = term.alpha.unwrap_or(1.0);
                debug_assert!(
                    alpha > 0.0,
                    "a term with total > 0 must carry a strictly positive alpha estimate \
                     (see PreparedTerm::prepare's Jeffreys estimate)"
                );
                sum += term.s / alpha;
            }
        }
        if sum <= 0.0 {
            f64::NEG_INFINITY
        } else {
            sum.ln() - self.z.ln()
        }
    }

    /// The pool this sampler draws from.
    pub fn pool(&self) -> Hand {
        self.pool
    }

    /// The fixed part every sampled hand contains.
    pub fn fixed(&self) -> Hand {
        self.fixed
    }

    /// Whether at least one term has a non-empty exact superset (design §5 stage 5: `count() >
    /// 0`, equivalently `self.terms.iter().any(|t| t.total > 0)`).
    ///
    /// This is the rule [`HandConstraint::is_satisfiable`](crate::HandConstraint::is_satisfiable)
    /// uses. It is exact for a term with no rejection literal (`alpha.is_none()`): its superset
    /// *is* its satisfying set. For a term that needs rejection (`Custom`, a DNF residual,
    /// `DistMethod::BergenStarting`, or more additive features than the slot allows), `total > 0`
    /// only says the superset is non-empty, not that the full check accepts anything in it; this
    /// deliberately does not consult the term's burn-in `alpha` estimate (a burn-in probe of
    /// `opts.burn_in` draws can easily miss a real but narrow accepting set, and a false "not
    /// satisfiable" is the unsafe direction here). So the answer over-approximates for non-exact
    /// terms: it can report "maybe satisfiable" for a term whose full check in fact accepts
    /// nothing (a false positive), but it never reports "not satisfiable" for a term that is in
    /// fact satisfiable (no false negatives), and it never reports an empty constraint
    /// (`count() == 0` for every term) as satisfiable.
    pub(crate) fn any_definitely_satisfiable(&self) -> bool {
        self.terms.iter().any(|t| t.total > 0)
    }
}
