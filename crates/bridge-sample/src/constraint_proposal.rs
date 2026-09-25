//! The v1 proposal: hierarchical sampling from the constraints.
//!
//! **prepare.** Per seat, alternatives = `interpretation.seats[s] ⊗ play_soft[s]`, each AND-ed
//! with `play_constraints[s]` and put in DNF. Restrictiveness `mass_s = Σ w_i · count(term)`
//! orders the seats (most constrained first); the first seat's samplers are cached. Seats whose
//! constraint is unconstrained are dealt combinatorially without a sampler.
//!
//! **propose.** Seats 1..=3 in order: re-prepare on the shrinking pool, draw a component ∝
//! `w_i · count_i`, draw a hand uniformly within it; the last seat gets the remainder and is
//! checked (rejected attempts are retried within the sample's own RNG stream).
//!
//! **log_prob.** Replay the same order and pools; `π_k(h) = Σ_{components ∋ h} u / count`,
//! summing over every component that could have produced `h`; the last seat contributes 0.
//!
//! **§6.4 (c), coarse re-preparation.** A bench (task 5.4; see `09-sample.md` §10 row 1) showed
//! the cached first seat and the direct-dealt seats (§6.4 (a)) already meet the 10^4 deals/s/core
//! target, but a seat re-prepared on every `propose`/`log_prob` call (every `Sampled` seat other
//! than the first, which is cached, and the last, which only needs a `satisfies` check) did not:
//! its candidates keep their original `cards` / `eval` detail, so every draw pays for the
//! detailed literal filtering `Sampler::prepare` needs for that. Those re-prepared seats use
//! [`coarsen`] instead: a summary `Atom` (shapes + HCP range only, from `HandConstraint::shapes`
//! / `HandConstraint::hcp_range`) that always covers a superset of the original candidate (`Or`
//! unions, `And` intersects, so the summary's satisfying set is never smaller), so it never turns
//! a satisfiable candidate unsatisfiable. `Sampler::prepare` on a bare `Atom` takes the shape-DP
//! fast path (05-constraint.md) instead of literal-level filtering. The resulting proposal can
//! land on hands the fine candidate would have rejected; `Interpretation::likelihood` still
//! scores those against the fine constraint, so the importance weight absorbs the mismatch — ESS
//! drops but stays finite, exactly as §6.4 describes. `log_prob` replays the same coarsened
//! candidates for these seats, since it must match the density `propose` actually drew from.

use bridge_constraint::{Atom, HandConstraint, Sampler};
use bridge_core::{Deal, Hand, Seat};

use crate::uniform::{draw_subset, ln_choose};
use crate::{PreparedProposal, Proposal, SampleContext, SampleError};

/// Alternatives are truncated to the `K` highest-weighted before sampling (D11, §6.1 point 1).
const MAX_ALTERNATIVES: usize = 8;

/// Hierarchical constraint sampling.
#[derive(Clone, Debug)]
pub struct ConstraintProposal {
    /// Retries per proposed deal before giving up (default 16).
    ///
    /// Passed through as [`bridge_constraint::SampleOptions::max_tries`] to every `Sampler` this
    /// proposal prepares, so a literal that can only be rejection-sampled (`Custom`, a DNF
    /// residual, a second additive feature) gets this many draws before its `Sampler::sample`
    /// gives up. `sample_deals`'s own outer retry loop (`SampleOptions::max_attempts_per_sample`
    /// in `report.rs`) is a separate knob: it retries the whole `propose` → `log_prob` pair, not
    /// a single `Sampler`'s internal rejection loop.
    pub max_retries: u32,
}

impl Default for ConstraintProposal {
    fn default() -> ConstraintProposal {
        ConstraintProposal { max_retries: 16 }
    }
}

/// One candidate alternative for a seat: a hand constraint (already AND-ed with the seat's hard
/// play constraint) and its raw, pre-normalisation weight `w_i`.
#[derive(Clone)]
struct Candidate {
    constraint: HandConstraint,
    weight: f64,
}

/// How a seat is drawn.
enum SeatPlan {
    /// The seat's surviving alternative reduces to `ANY` (§6.4 (a)): drawn combinatorially,
    /// uniformly at random from whatever pool remains, no `Sampler` involved. `log_prob =
    /// −ln C(|pool|, needed)`.
    Direct,
    /// Drawn by choosing one candidate proportional to its weight (a genuine prior over "which
    /// alternative is true"; the count-weighting that orders seats by `mass_s` does not apply
    /// here — see §6.1 point 4 and §6.2 step 3), then sampling uniformly within it.
    ///
    /// `coarse` (§6.4 (c)) is the same candidates with [`coarsen`] applied, used instead of
    /// `candidates` whenever this seat is re-prepared per draw (every position except the cached
    /// first seat and the residual last seat, which need `candidates` unchanged: the first is
    /// prepared once regardless, and the last only ever calls `HandConstraint::satisfies`, never
    /// `Sampler::prepare`).
    Sampled {
        candidates: Vec<Candidate>,
        coarse: Vec<Candidate>,
    },
}

/// One seat in the sampling order `σ` (§6.1 point 3): most constrained first, so failure is
/// discovered as early (and as cheaply) as possible.
struct SeatEntry {
    seat: Seat,
    plan: SeatPlan,
}

/// The first seat's prepared samplers, cached because its pool (`known.pool()`) never shrinks
/// before it is drawn (§6.1 point 4): `(Sampler, v_i)` with `v_i = w_i / Σ_j w_j` normalised over
/// the candidates that survived (`count() > 0`).
struct CachedFirst {
    components: Vec<(Sampler, f64)>,
}

impl Proposal for ConstraintProposal {
    fn prepare<'c>(
        &self,
        ctx: &'c SampleContext<'c>,
    ) -> Result<Box<dyn PreparedProposal + Send + Sync + 'c>, SampleError> {
        let sampler_opts = bridge_constraint::SampleOptions {
            max_tries: self.max_retries.max(1),
            ..bridge_constraint::SampleOptions::default()
        };
        let pool = ctx.known.pool();

        let mut order: Vec<(SeatEntry, f64)> = Vec::new();
        for seat in Seat::ALL {
            let needed = ctx.known.needed(seat);
            if needed == 0 {
                continue;
            }
            let fixed = ctx.known.known[seat.index() as usize];
            let hard = &ctx.play_constraints[seat.index() as usize];

            let mut candidates = seat_candidates(ctx, seat, hard);
            candidates.sort_by(|a, b| {
                b.weight
                    .partial_cmp(&a.weight)
                    .unwrap_or(core::cmp::Ordering::Equal)
            });
            candidates.truncate(MAX_ALTERNATIVES);

            if candidates.len() == 1 && is_unconstrained(&candidates[0].constraint) {
                let mass = ln_choose(pool.len(), needed);
                order.push((
                    SeatEntry {
                        seat,
                        plan: SeatPlan::Direct,
                    },
                    mass,
                ));
                continue;
            }

            let mut alts: Vec<(Candidate, u64)> = Vec::with_capacity(candidates.len());
            for candidate in candidates {
                let sampler = Sampler::prepare(&candidate.constraint, pool, fixed, &sampler_opts)
                    .map_err(|e| SampleError::Prepare(e.to_string()))?;
                let count = sampler.count();
                if count == 0 {
                    continue;
                }
                alts.push((candidate, count));
            }
            if alts.is_empty() {
                // `sample_deals` (§2.3 of `09-sample.md`) already checked that at least one
                // interpretation alternative survives AND-ing with `hard` against the full pool;
                // this can still be empty when `play_soft` narrows further than that check
                // accounts for. Fall back to the hard constraint alone (already known
                // satisfiable, or `sample_deals` would have returned `SampleError::EmptySupport`)
                // rather than leaving this seat with no way to be drawn at all.
                let sampler = Sampler::prepare(hard, pool, fixed, &sampler_opts)
                    .map_err(|e| SampleError::Prepare(e.to_string()))?;
                let count = sampler.count().max(1);
                alts.push((
                    Candidate {
                        constraint: hard.clone(),
                        weight: 1.0,
                    },
                    count,
                ));
            }

            let mass: f64 = alts.iter().map(|(c, count)| c.weight * *count as f64).sum();
            let candidates: Vec<Candidate> = alts.into_iter().map(|(c, _)| c).collect();
            let coarse = candidates
                .iter()
                .map(|c| Candidate {
                    constraint: coarsen(&c.constraint),
                    weight: c.weight,
                })
                .collect();
            order.push((
                SeatEntry {
                    seat,
                    plan: SeatPlan::Sampled { candidates, coarse },
                },
                mass.ln(),
            ));
        }

        order.sort_by(|(a, mass_a), (b, mass_b)| {
            mass_a
                .partial_cmp(mass_b)
                .unwrap_or(core::cmp::Ordering::Equal)
                .then_with(|| a.seat.index().cmp(&b.seat.index()))
        });
        let order: Vec<SeatEntry> = order.into_iter().map(|(entry, _)| entry).collect();

        let cached_first = match order.first() {
            Some(SeatEntry {
                seat,
                plan: SeatPlan::Sampled { candidates, .. },
            }) => {
                let fixed = ctx.known.known[seat.index() as usize];
                Some(CachedFirst {
                    components: prepare_components(candidates, pool, fixed, &sampler_opts)
                        .ok_or_else(|| {
                            SampleError::Prepare(
                                "the first seat's alternatives lost all support against the \
                                 full pool, which its own prepare pass should have prevented"
                                    .to_string(),
                            )
                        })?,
                })
            }
            _ => None,
        };

        Ok(Box::new(PreparedConstraint {
            ctx,
            sampler_opts,
            order,
            cached_first,
        }))
    }
}

/// `interpretation.seats[s] ⊗ play_soft[s]`, each AND-ed with `hard` (§6.1 point 1). A seat with
/// no calls at all is `[(ANY, 1.0)]` (Step B's convention, 07-bidding.md §4.4), matching so that
/// an unconstrained seat with no soft information reduces to a single `ANY` candidate and is
/// caught by [`is_unconstrained`].
fn seat_candidates(ctx: &SampleContext<'_>, seat: Seat, hard: &HandConstraint) -> Vec<Candidate> {
    let base = &ctx.interpretation.seats[seat.index() as usize];
    let soft = ctx
        .play_soft
        .map(|soft| &soft[seat.index() as usize])
        .filter(|soft| !soft.is_empty());

    let mut out = Vec::new();
    match soft {
        None => {
            if base.is_empty() {
                out.push(Candidate {
                    constraint: hard.clone(),
                    weight: 1.0,
                });
            } else {
                for (constraint, weight, _) in base {
                    out.push(Candidate {
                        constraint: and_opt(constraint.clone(), hard.clone()),
                        weight: f64::from(*weight),
                    });
                }
            }
        }
        Some(soft) => {
            if base.is_empty() {
                for (soft_constraint, soft_weight) in soft {
                    out.push(Candidate {
                        constraint: and_opt(soft_constraint.clone(), hard.clone()),
                        weight: f64::from(*soft_weight),
                    });
                }
            } else {
                for (constraint, weight, _) in base {
                    for (soft_constraint, soft_weight) in soft {
                        let combined = and_opt(
                            and_opt(constraint.clone(), soft_constraint.clone()),
                            hard.clone(),
                        );
                        out.push(Candidate {
                            constraint: combined,
                            weight: f64::from(*weight) * f64::from(*soft_weight),
                        });
                    }
                }
            }
        }
    }
    out
}

/// `a ∧ b`, skipping the conjunction (and the `And` wrapping it would otherwise introduce) when
/// one side is literally the unconstrained atom, so that "unconstrained AND unconstrained" stays
/// recognisable to [`is_unconstrained`] instead of becoming an `And` of two `ANY` atoms.
fn and_opt(a: HandConstraint, b: HandConstraint) -> HandConstraint {
    if is_unconstrained(&a) {
        return b;
    }
    if is_unconstrained(&b) {
        return a;
    }
    a.and(b)
}

/// Whether `constraint` is exactly the unconstrained atom (§6.1 point 5: "shapes = ALL, hcp =
/// 0..=37, cards / eval empty"). Only a bare, literal `ANY` is recognised — a semantically
/// unconstrained but structurally different tree (e.g. `Or` of every shape) still goes through
/// the sampler, which handles it exactly at the cost of one `prepare` call.
fn is_unconstrained(constraint: &HandConstraint) -> bool {
    matches!(
        constraint,
        HandConstraint::Atom(Atom {
            shapes,
            hcp,
            cards,
            eval,
        }) if *shapes == bridge_constraint::ShapeSet::ALL
            && *hcp == (0..=37)
            && cards.is_empty()
            && eval.is_empty()
    )
}

/// §6.4 (c): a "coarse" summary of `constraint`, keeping only its shape set and HCP range
/// (`HandConstraint::shapes` / `HandConstraint::hcp_range`) and dropping any `cards` / `eval`
/// detail. Always a superset of `constraint`'s own satisfying set (`Or` unions the branches'
/// summaries, `And` intersects them; see those methods' doc comments), so it never turns a
/// satisfiable candidate unsatisfiable — only ever wider, never narrower. Used only for seats
/// re-prepared on every `propose` / `log_prob` call; see the module doc comment.
fn coarsen(constraint: &HandConstraint) -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: constraint.shapes(),
        hcp: constraint.hcp_range(),
        cards: Vec::new(),
        eval: Vec::new(),
    })
}

/// Prepares one `Sampler` per candidate against `(pool, fixed)`, drops the ones with
/// `count() == 0`, and normalises the survivors' weights into `v_i = w_i / Σ_j w_j` (§6.1 point
/// 4, §6.2 step 2). `None` when nothing survives (`propose`/`log_prob` treat that as "no deal is
/// possible from here").
fn prepare_components(
    candidates: &[Candidate],
    pool: Hand,
    fixed: Hand,
    opts: &bridge_constraint::SampleOptions,
) -> Option<Vec<(Sampler, f64)>> {
    let mut survivors: Vec<(Sampler, f64)> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let sampler = Sampler::prepare(&candidate.constraint, pool, fixed, opts).ok()?;
        if sampler.count() == 0 {
            continue;
        }
        survivors.push((sampler, candidate.weight));
    }
    let total: f64 = survivors.iter().map(|(_, w)| *w).sum();
    if survivors.is_empty() || total <= 0.0 {
        return None;
    }
    for (_, w) in &mut survivors {
        *w /= total;
    }
    Some(survivors)
}

/// Draws a component index proportional to its weight (weights need not sum to 1: `total` is
/// computed from what is passed in), using a 53-bit uniform double so the draw depends only on
/// `rng.next_u64()`.
fn choose_component(components: &[(Sampler, f64)], rng: &mut dyn rand_core::Rng) -> usize {
    let total: f64 = components.iter().map(|(_, w)| *w).sum();
    let u = (rng.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64);
    let target = u * total;
    let mut acc = 0.0;
    for (i, (_, w)) in components.iter().enumerate() {
        acc += w;
        if target < acc {
            return i;
        }
    }
    components.len() - 1
}

/// `Σ_{i: hand ∈ components[i]} v_i · exp(sampler_i.log_prob(hand))`, in the log domain;
/// `−∞` when no component contains `hand` (§6.3).
fn mixture_log_prob(components: &[(Sampler, f64)], hand: Hand) -> f64 {
    let mut mix = 0.0f64;
    for (sampler, v) in components {
        let lp = sampler.log_prob(hand);
        if lp.is_finite() {
            mix += v * lp.exp();
        }
    }
    if mix <= 0.0 {
        f64::NEG_INFINITY
    } else {
        mix.ln()
    }
}

struct PreparedConstraint<'c> {
    ctx: &'c SampleContext<'c>,
    sampler_opts: bridge_constraint::SampleOptions,
    /// Seats needing cards, most constrained first (§6.1 point 3); the last entry is the
    /// residual seat that receives whatever remains of the pool.
    order: Vec<SeatEntry>,
    /// `Some` when `order`'s first entry is `Sampled` (its pool is the full pool and never
    /// shrinks before it is drawn, so it is prepared once here rather than in every `propose`).
    cached_first: Option<CachedFirst>,
}

impl PreparedProposal for PreparedConstraint<'_> {
    fn propose(&self, rng: &mut dyn rand_core::Rng) -> Option<Deal> {
        let known = &self.ctx.known;
        let mut hands = known.known;
        let mut pool = known.pool();
        let m = self.order.len();

        for (k, entry) in self.order.iter().enumerate() {
            let seat = entry.seat;
            let fixed = known.known[seat.index() as usize];

            if k == m - 1 {
                let hand = fixed.union(pool);
                if !self.ctx.play_constraints[seat.index() as usize].satisfies(hand) {
                    return None;
                }
                let satisfied = match &entry.plan {
                    SeatPlan::Direct => true,
                    SeatPlan::Sampled { candidates, .. } => {
                        candidates.iter().any(|c| c.constraint.satisfies(hand))
                    }
                };
                if !satisfied {
                    return None;
                }
                hands[seat.index() as usize] = hand;
                break;
            }

            let hand = match &entry.plan {
                SeatPlan::Direct => {
                    let needed = known.needed(seat);
                    fixed.union(draw_subset(pool, needed, rng))
                }
                SeatPlan::Sampled { coarse, .. } if k != 0 => {
                    // §6.4 (c): re-prepared every draw, so use the coarse candidates.
                    let components = prepare_components(coarse, pool, fixed, &self.sampler_opts)?;
                    let i = choose_component(&components, rng);
                    components[i].0.sample(rng)?.hand
                }
                SeatPlan::Sampled { .. } => {
                    let components = self
                        .cached_first
                        .as_ref()
                        .expect("order[0] is Sampled, so cached_first was built in prepare")
                        .components
                        .as_slice();
                    let i = choose_component(components, rng);
                    components[i].0.sample(rng)?.hand
                }
            };

            let drawn = hand.difference(fixed);
            pool = pool.difference(drawn);
            hands[seat.index() as usize] = hand;
        }

        Some(Deal::new(hands).expect(
            "the known cards partition disjointly and every seat's drawn cards come from the \
             shrinking pool, so the four hands always partition the deck",
        ))
    }

    fn log_prob(&self, deal: &Deal) -> f64 {
        let known = &self.ctx.known;
        for seat in Seat::ALL {
            if !known.known[seat.index() as usize].is_subset(deal.hand(seat)) {
                return f64::NEG_INFINITY;
            }
        }

        let mut pool = known.pool();
        let mut ln_pi = 0.0f64;
        let m = self.order.len();

        for (k, entry) in self.order.iter().enumerate() {
            let seat = entry.seat;
            let fixed = known.known[seat.index() as usize];
            let hand = deal.hand(seat);

            if k == m - 1 {
                if !self.ctx.play_constraints[seat.index() as usize].satisfies(hand) {
                    return f64::NEG_INFINITY;
                }
                let satisfied = match &entry.plan {
                    SeatPlan::Direct => true,
                    SeatPlan::Sampled { candidates, .. } => {
                        candidates.iter().any(|c| c.constraint.satisfies(hand))
                    }
                };
                if !satisfied {
                    return f64::NEG_INFINITY;
                }
                break;
            }

            match &entry.plan {
                SeatPlan::Direct => {
                    let needed = known.needed(seat);
                    ln_pi += -ln_choose(pool.len(), needed);
                }
                SeatPlan::Sampled { coarse, .. } if k != 0 => {
                    // Sum over every component that could have produced `hand` (they overlap):
                    // the mixture density is only correct when every one is counted (§6.3).
                    // §6.4 (c): replay the same coarse candidates `propose` drew this seat from.
                    let ln_component =
                        match prepare_components(coarse, pool, fixed, &self.sampler_opts) {
                            Some(components) => mixture_log_prob(&components, hand),
                            None => f64::NEG_INFINITY,
                        };
                    if !ln_component.is_finite() {
                        return f64::NEG_INFINITY;
                    }
                    ln_pi += ln_component;
                }
                SeatPlan::Sampled { .. } => {
                    let components = &self
                        .cached_first
                        .as_ref()
                        .expect("order[0] is Sampled, so cached_first was built in prepare")
                        .components;
                    let ln_component = mixture_log_prob(components, hand);
                    if !ln_component.is_finite() {
                        return f64::NEG_INFINITY;
                    }
                    ln_pi += ln_component;
                }
            }

            let drawn = hand.difference(fixed);
            pool = pool.difference(drawn);
        }

        ln_pi
    }
}
