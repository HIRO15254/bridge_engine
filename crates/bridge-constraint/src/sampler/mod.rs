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

use crate::{Dnf, DnfOptions, DnfTerm, HandConstraint, PrepareError};
use rand_util::random_below;

/// Hands `shared`'s frozen pair maps to every term of `samplers` that was prepared against them
/// (`term::SharedPlain`); must run before any of them is sampled from.
fn attach_shared(samplers: &mut [Sampler], shared: term::SharedPlain) {
    let maps = shared.freeze();
    for sampler in samplers {
        for t in &mut sampler.terms {
            t.attach_shared(&maps);
        }
    }
}

/// The argument checks shared by [`Sampler::prepare`] and [`Sampler::prepare_many`].
fn check_pool_and_fixed(pool: Hand, fixed: Hand) -> Result<(), PrepareError> {
    if !pool.is_disjoint(fixed) {
        return Err(PrepareError::Overlap);
    }
    let fixed_len = fixed.len();
    if fixed_len > 13 {
        return Err(PrepareError::TooManyFixed(fixed_len));
    }
    Ok(())
}

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
        check_pool_and_fixed(pool, fixed)?;
        let mut shared = term::SharedPlain::new(pool, fixed);
        let mut sampler = Sampler::prepare_shared(constraint, pool, fixed, opts, &mut shared)?;
        attach_shared(core::slice::from_mut(&mut sampler), shared);
        Ok(sampler)
    }

    /// Prepares one sampler per constraint, all against the same `pool` and `fixed`, in order.
    ///
    /// Each returned `Sampler` is identical to what [`Sampler::prepare`] would return for that
    /// constraint alone. The difference is cost: DNF terms whose per-suit tables do not depend
    /// on the term (terms with no single-suit `cards` / `SuitQuality` literal and no additive
    /// feature — in particular every plain shape + HCP atom) share the four per-suit tables and
    /// the pair convolutions across *all* the constraints, so preparing `n` such constraints
    /// against a shrunken pool costs roughly one table build instead of `n`. This is the call a
    /// caller that re-prepares a mixture of alternatives on every draw should use.
    ///
    /// Errors as [`Sampler::prepare`] does, on the first constraint that fails.
    pub fn prepare_many<'a>(
        constraints: impl IntoIterator<Item = &'a HandConstraint>,
        pool: Hand,
        fixed: Hand,
        opts: &SampleOptions,
    ) -> Result<Vec<Sampler>, PrepareError> {
        check_pool_and_fixed(pool, fixed)?;
        let mut shared = term::SharedPlain::new(pool, fixed);
        let mut samplers = constraints
            .into_iter()
            .map(|c| Sampler::prepare_shared(c, pool, fixed, opts, &mut shared))
            .collect::<Result<Vec<_>, _>>()?;
        attach_shared(&mut samplers, shared);
        Ok(samplers)
    }

    /// [`Sampler::prepare_many`] for constraints already in disjunctive normal form: each `dnf`
    /// must be `constraint.to_dnf(&DnfOptions::default())` of the constraint it stands for, and
    /// the result is then identical to `prepare_many` on those constraints.
    ///
    /// A caller that re-prepares the same constraints against a different pool on every draw
    /// converts them once and calls this, skipping the per-call DNF conversion (whose atom
    /// normalisation and trivial-unsatisfiability checks walk every shape in the atom's set).
    /// A `Custom` literal is detected from the terms' `custom` lists instead of the original
    /// tree, so a `Custom` that only occurred inside a term the conversion dropped as trivially
    /// unsatisfiable no longer triggers the rejection warning or `PrepareError::NotSamplable`.
    pub fn prepare_many_dnf<'a>(
        dnfs: impl IntoIterator<Item = &'a Dnf>,
        pool: Hand,
        fixed: Hand,
        opts: &SampleOptions,
    ) -> Result<Vec<Sampler>, PrepareError> {
        check_pool_and_fixed(pool, fixed)?;
        let mut shared = term::SharedPlain::new(pool, fixed);
        let mut samplers = dnfs
            .into_iter()
            .map(|dnf| {
                let samplable = dnf.terms.iter().all(|t| t.custom.is_empty());
                Sampler::prepare_terms(
                    dnf.terms.iter().cloned(),
                    samplable,
                    pool,
                    fixed,
                    opts,
                    &mut shared,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        attach_shared(&mut samplers, shared);
        Ok(samplers)
    }

    fn prepare_shared(
        constraint: &HandConstraint,
        pool: Hand,
        fixed: Hand,
        opts: &SampleOptions,
        shared: &mut term::SharedPlain,
    ) -> Result<Sampler, PrepareError> {
        let dnf = constraint
            .to_dnf(&DnfOptions::default())
            .expect("DnfOptions::default uses Overflow::Residual, which never errors");
        Sampler::prepare_terms(
            dnf.terms,
            constraint.is_samplable(),
            pool,
            fixed,
            opts,
            shared,
        )
    }

    fn prepare_terms(
        dnf_terms: impl IntoIterator<Item = DnfTerm>,
        samplable: bool,
        pool: Hand,
        fixed: Hand,
        opts: &SampleOptions,
        shared: &mut term::SharedPlain,
    ) -> Result<Sampler, PrepareError> {
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

        let dnf_terms = dnf_terms.into_iter();
        let mut terms = Vec::with_capacity(dnf_terms.size_hint().0);
        for dnf_term in dnf_terms {
            let prepared = term::PreparedTerm::prepare(dnf_term, pool, fixed, opts, shared);
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

#[cfg(test)]
mod tests {
    use bridge_core::{Card, Holding, Suit};
    use rand_xoshiro::Xoshiro256PlusPlus;
    use rand_xoshiro::rand_core::SeedableRng;

    use super::*;
    use crate::{Atom, CardRequirement, ShapeSet};

    fn atom(shapes: ShapeSet, hcp: core::ops::RangeInclusive<u8>) -> HandConstraint {
        HandConstraint::Atom(Atom {
            shapes,
            hcp,
            ..Atom::ANY
        })
    }

    /// Differential test for the table/convolution sharing in [`Sampler::prepare_many`]: every
    /// sampler it returns must behave exactly like the one [`Sampler::prepare`] builds alone —
    /// same count and exactness, the same hand for the same RNG stream, and the same `log_prob`
    /// on every hand either one draws. The mix covers plain atoms (which share tables), an atom
    /// with a single-suit card filter and a multi-suit (additive) card requirement (which do
    /// not), `ANY`, a two-term `Or`, and an unsatisfiable atom, on the full deck and on a
    /// shrunken mid-deal pool with fixed cards.
    #[test]
    fn prepare_many_matches_prepare_one_by_one() {
        let spade_ace = CardRequirement::in_suit(Suit::Spades, Holding::top_ranks(1), 1..=1);
        let two_aces = CardRequirement {
            mask: Hand::EMPTY
                .with_holding(Suit::Hearts, Holding::top_ranks(1))
                .with_holding(Suit::Clubs, Holding::top_ranks(1)),
            count: 1..=2,
        };
        let constraints = vec![
            atom(ShapeSet::BALANCED, 15..=17),
            atom(ShapeSet::from_suit_len(Suit::Spades, 5, 13), 11..=21),
            HandConstraint::ANY,
            atom(ShapeSet::BALANCED, 15..=17),
            HandConstraint::Atom(Atom {
                cards: vec![spade_ace],
                ..Atom::ANY.with_hcp(8..=37)
            }),
            HandConstraint::Atom(Atom {
                cards: vec![two_aces],
                ..Atom::ANY.with_hcp(0..=12)
            }),
            HandConstraint::Or(vec![
                atom(ShapeSet::from_suit_len(Suit::Hearts, 6, 13), 5..=10),
                atom(ShapeSet::ALL, 0..=5),
            ]),
            atom(ShapeSet::from_suit_len(Suit::Hearts, 4, 13), 0..=37),
            atom(ShapeSet::BALANCED, 37..=37),
        ];

        let mut mid_pool = Hand::EMPTY;
        let mut mid_fixed = Hand::EMPTY;
        for i in 0..52u8 {
            let card = Card::from_index(i).expect("index < 52");
            match i % 4 {
                0 if mid_fixed.len() < 4 => mid_fixed = mid_fixed.with(card),
                1 => {}
                _ => mid_pool = mid_pool.with(card),
            }
        }
        let opts = SampleOptions::default();

        for (pool, fixed) in [(Hand::FULL, Hand::EMPTY), (mid_pool, mid_fixed)] {
            let many = Sampler::prepare_many(&constraints, pool, fixed, &opts)
                .expect("pool and fixed are disjoint");
            assert_eq!(many.len(), constraints.len());
            let dnfs: Vec<Dnf> = constraints
                .iter()
                .map(|c| c.to_dnf(&DnfOptions::default()).expect("never errors"))
                .collect();
            let many_dnf = Sampler::prepare_many_dnf(&dnfs, pool, fixed, &opts)
                .expect("pool and fixed are disjoint");
            let alone: Vec<Sampler> = constraints
                .iter()
                .map(|c| Sampler::prepare(c, pool, fixed, &opts).expect("disjoint"))
                .collect();
            for (i, (shared, solo)) in many.iter().zip(&alone).enumerate() {
                let via_dnf = &many_dnf[i];
                assert_eq!(via_dnf.count(), solo.count(), "dnf count, constraint {i}");
                assert_eq!(
                    via_dnf.is_exact(),
                    solo.is_exact(),
                    "dnf exactness, constraint {i}"
                );
                let mut rng_c = Xoshiro256PlusPlus::seed_from_u64(1000 + i as u64);
                for _ in 0..50 {
                    let mut rng_d = rng_c.clone();
                    assert_eq!(
                        via_dnf.sample(&mut rng_c),
                        solo.sample(&mut rng_d),
                        "dnf sample, constraint {i}"
                    );
                }
                assert_eq!(shared.count(), solo.count(), "count, constraint {i}");
                assert_eq!(
                    shared.is_exact(),
                    solo.is_exact(),
                    "exactness, constraint {i}"
                );
                let mut rng_a = Xoshiro256PlusPlus::seed_from_u64(1000 + i as u64);
                let mut rng_b = Xoshiro256PlusPlus::seed_from_u64(1000 + i as u64);
                for _ in 0..200 {
                    let a = shared.sample(&mut rng_a);
                    let b = solo.sample(&mut rng_b);
                    assert_eq!(a, b, "sample, constraint {i}");
                    if let Some(sample) = a {
                        for (j, (s, o)) in many.iter().zip(&alone).enumerate() {
                            assert_eq!(
                                s.log_prob(sample.hand).to_bits(),
                                o.log_prob(sample.hand).to_bits(),
                                "log_prob under constraint {j} of a hand from constraint {i}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// The argument checks still apply to the batch entry point.
    #[test]
    fn prepare_many_rejects_overlapping_pool_and_fixed() {
        let card = Hand::EMPTY.with(Card::from_index(0).expect("index < 52"));
        let result = Sampler::prepare_many(
            [&HandConstraint::ANY],
            Hand::FULL,
            card,
            &SampleOptions::default(),
        );
        assert!(matches!(result, Err(PrepareError::Overlap)));
    }
}
