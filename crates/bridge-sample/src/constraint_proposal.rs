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
//! **§6.4 (c), coarse re-preparation.** Every `Sampled` seat other than the first (cached) and
//! the last (only ever `satisfies`-checked, never `Sampler::prepare`d) is re-prepared on every
//! `propose`/`log_prob` call. Those re-prepared seats use [`coarsen`] instead of their original
//! candidates: a literal-free atom or flat `Or` of literal-free atoms (the policy mirror's
//! exclusive pieces) is kept unchanged; anything else is replaced by its literal-free superset on
//! the (shape, HCP) grid (`bridge_constraint::grid::bounds(..).sup`, at most
//! [`COARSE_ATOM_CAP`] atoms), which drops `cards` / `eval` detail but keeps a tree's `Not`s and
//! `Or`s where the grid can express them. It always covers a superset of the original candidate,
//! so it never turns a satisfiable candidate unsatisfiable. The resulting proposal can land on
//! hands the fine candidate would have rejected; the likelihood still scores those against the
//! fine constraint, so the importance weight absorbs the mismatch — ESS drops but stays finite,
//! exactly as §6.4 describes. `log_prob` replays the same coarsened candidates for these seats,
//! since it must match the density `propose` actually drew from.
//!
//! **§6.4 (d), light folding** ([`ConstraintProposal::light_threshold`]). At those re-prepared
//! seats, alternatives carrying a negligible share of the mass relative to their share of the
//! hands (the policy mirror's `1e-5`-weight fallback pieces) are not prepared on every draw: they
//! are folded into one uniform component with a fixed draw probability, whose density
//! `1 / C(|pool|, needed)` is closed-form. Per draw only the few alternatives that carry the mass
//! are prepared; the density stays exact.
//!
//! **§6.5, residual rejection** ([`ConstraintProposal::residual_rejection`]). The last seat can
//! be accepted with probability proportional to its own mixture at the residual hand, which moves
//! that seat's likelihood factor out of the importance weight and into the acceptance rate. Off
//! by default (it does not lower the wall time per effective sample on the ESS suite).
//!
//! This is *not* a fix for a per-literal filtering cost at `Sampler::prepare` — measured on the
//! bench cases (`09-sample.md` §10.1) and on `Sampler::prepare` directly, a `cards` literal adds
//! no measurable cost on the pool sizes (c) applies to (39 and 26 unknown cards): the DNF/atom
//! evaluation there is dominated by the shape/HCP walk regardless of whether a `cards` literal
//! rides along. (c) *does* help when coarsening removes something that changes which code path
//! `Sampler::prepare` takes — most obviously when the summary collapses to (or near) `ANY`, or
//! drops enough of the HCP window that shape-DP pruning does less work — and it always costs
//! acceptance and ESS, because it drops `cards` / `eval`, `Not`-inferences and card-level hard
//! play constraints at every re-prepared seat, whether or not that seat's own bench case happens
//! to exercise them.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use bridge_constraint::{Atom, Dnf, DnfOptions, HandConstraint, HcpShapeGrid, Sampler};
use bridge_core::{Deal, Hand, Seat};

use crate::uniform::{draw_subset, ln_choose};
use crate::{PreparedProposal, Proposal, SampleContext, SampleError};

// `sample_deals`'s own `process_slot` (in `lib.rs`) always calls `propose` and then immediately
// `log_prob` for the *same* accepted deal, on the same thread, before touching any other
// `PreparedProposal` — so for every re-prepared seat (every position but the cached first and the
// residual last, §6.4 (c)), `log_prob`'s walk hits exactly the `(pool, fixed)` `propose` just built
// a `Sampler` for. Without this cache, `log_prob` re-runs `prepare_components` (re-preparing a
// `Sampler` per surviving candidate) from scratch, duplicating work `propose` already did a moment
// earlier — measured on `four_call_three_seats` (09-sample.md §10.1), `propose` and `log_prob` cost
// almost exactly the same, so this removes close to half the per-deal cost at those seats.
//
// A thread-local, not a field on `PreparedConstraint`, because `PreparedProposal` must stay
// `Send + Sync` (it is shared as `&(dyn PreparedProposal + Send + Sync)` across rayon's pool in
// `Threads::Auto`, per `lib.rs`'s `run_parallel`); a `RefCell` field would break `Sync`. Each slot
// records which `PreparedConstraint` wrote it (its `id`, unique per `prepare_constraint` call and
// never reused, unlike an address) together with the `(pool, fixed)` it was built for, and
// `log_prob` trusts a slot only when all three match its own. Anything else — a different deal, a
// different proposal on the same thread (e.g. scoring one proposal's deal under another), or
// `log_prob` with no preceding `propose` on this thread — falls back to rebuilding from its own
// coarse candidates, so the cache never changes the density `log_prob` returns.
/// One re-prepared seat's cached samplers, keyed by seat position in `REPREPARE_CACHE`.
type CachedSeatComponents = Option<SeatCache>;

/// What `propose` prepared for one re-prepared seat, tagged with the proposal and the
/// `(pool, fixed)` it was prepared against.
struct SeatCache {
    id: u64,
    pool: Hand,
    fixed: Hand,
    /// The seat's mixture on that pool.
    mix: SeatMix,
}

/// Source of [`PreparedConstraint::id`]: a fresh value per `prepare_constraint` call.
static NEXT_PREPARED_ID: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static REPREPARE_CACHE: RefCell<Vec<CachedSeatComponents>> =
        const { RefCell::new(Vec::new()) };
}

/// At most `K` alternatives per seat are sampled: the `K - 1` highest-weighted plus, when more
/// exist, a catch-all carrying the rest (D11, §6.1 point 1; see [`truncate_keeping_support`]).
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
    ///
    /// This default is lower than [`bridge_constraint::SampleOptions`]'s own default of 256 (the
    /// value `sample_deals`'s own §2.3 support probe uses). For a rejection-sampled term with
    /// acceptance rate `α`, the probability the term's `Sampler` gives up within `max_retries`
    /// draws is `(1 − α)^max_retries`, so a lower `max_retries` gives up more often, and does so
    /// unevenly across terms and components with different `α` — which skews `log_prob` for the
    /// (already only approximate; see `SampleWarning::CustomConstraint`) inexact case. Raising
    /// this default to 256 would even out that skew at the cost of up to 16× more retries per
    /// inexact term; `sample_deals`'s own bench (`benches/deals.rs`) has no inexact terms in its
    /// cases, so this trade was left as `SampleWarning::CustomConstraint`'s documented caveat
    /// rather than measured and changed here.
    pub max_retries: u32,
    /// Residual rejection (`09-sample.md` §6.5; default `false`): the residual last seat, which
    /// receives whatever cards remain, is accepted with probability `a(h) = min(1, m(h) / T)`,
    /// where `m(h) = Σ_{i: h ∈ C_i} w_i` is the seat's own alternative mixture at its hand and
    /// `T` a threshold fixed once in `prepare`, and `log_prob` adds `ln a`.
    ///
    /// Without it the residual seat's whole likelihood factor lands in the importance weight;
    /// with it, the accepted deals' density is `π_others(d) · a(h_last) / P_acc`, with a
    /// deal-independent acceptance normaliser `P_acc` that self-normalised weights absorb, so the
    /// estimator stays exact for any fixed `T > 0`: the weight of an accepted deal carries
    /// `max(m(h), T)` instead of `m(h)`, so `T` only moves variance between the weights and the
    /// rejection rate. `T` is the smallest of
    ///
    /// - `U`, a bound on `m` over every hand (the largest, over the alternatives `i`, of the
    ///   total weight of the alternatives whose (shape, HCP) grid superset meets `i`'s; `max_i
    ///   w_i` for the policy mirror's disjoint pieces): above it rejection buys nothing;
    /// - the largest `m` seen on 128 pilot proposals (a fixed RNG stream, so
    ///   `T` is a deterministic function of the context): above it every pilot weight is already
    ///   flat;
    /// - the `T` at which the pilot's mean acceptance falls to
    ///   [`ConstraintProposal::residual_min_acceptance`], so a residual seat that rarely lands in
    ///   its heavy alternatives does not exhaust the attempt budget.
    ///
    /// Rejected attempts are ordinary rejected proposals: they count against `SampleOptions`'
    /// attempt budget and show up in `SampleReport::acceptance_rate` and `ess_per_attempt`. A
    /// rejected attempt skips `log_prob` and the likelihood, so it costs less than a produced
    /// deal; whether the trade pays depends on what a produced deal costs downstream (a
    /// double-dummy solve in the lead advisor costs far more than an attempt).
    ///
    /// Off by default. The phase-4 rule (D18-D20 of `15-phase4-plan.md`) enables it by default
    /// only if it lowers the wall time per effective sample, and on the ESS suite it does not:
    /// it raises ESS/n but costs more sampling time than that buys (the measurements are in
    /// `09-sample.md` §6.5 and §10.2). Turn it on where a produced deal costs far more
    /// downstream than a proposal attempt, as `bridge_lead::lead_proposal` does for the
    /// double-dummy solves of the lead advisor.
    pub residual_rejection: bool,
    /// The pilot acceptance residual rejection's threshold is kept above (default 0.5, at most
    /// about 2 attempts per produced deal; only used when
    /// [`ConstraintProposal::residual_rejection`] is on). Lower values reject more and flatten
    /// the weights further for more sampling time, which pays when every produced deal is
    /// expensive downstream (`bridge_lead::lead_proposal` uses 0.125). `09-sample.md` §6.5 has
    /// the tuning sweep.
    pub residual_min_acceptance: f64,
    /// Light-alternative folding (`09-sample.md` §6.4 (d); default `1e-2`). At a seat
    /// re-prepared on every draw, a coarse alternative `j` whose share of the full-pool mass,
    /// `π_j = w_j · cnt_j / Σ_i w_i · cnt_i`, is at most this times its share of the hands,
    /// `f_j = cnt_j / C(|pool|, needed)`, is *light*: instead of being prepared on every draw it
    /// is folded into one uniform component (a uniform draw of the seat's missing cards from the
    /// remaining pool) with draw probability `π_L = Σ_light π_j`. The seat's density stays exact
    /// for any value, and the uniform component covers every light alternative's hands; the
    /// ratio bounds what each light alternative can add to `E[w²]` (about `π_j / f_j`), so the
    /// default costs at most a few percent of ESS. `0.0` turns folding off.
    ///
    /// The shares are fixed on the full pool, but earlier seats' draws can leave the kept
    /// alternatives almost no room on the pool a draw actually sees, while the folded ones still
    /// fit. So on every draw the kept alternatives' mass `Σ w_i · cnt_i(pool)` is compared with
    /// their full-pool mass scaled by how much the number of hands shrank; when it fell below a
    /// tenth of that, the seat is drawn from every alternative, adaptively, instead (a choice
    /// that depends only on the pool, so `log_prob` replays it exactly).
    pub light_threshold: f64,
}

impl Default for ConstraintProposal {
    fn default() -> ConstraintProposal {
        ConstraintProposal {
            max_retries: 16,
            residual_rejection: false,
            residual_min_acceptance: 0.5,
            light_threshold: 1e-2,
        }
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
    /// Drawn by choosing one candidate proportional to `w_i · count_i` — the same count-weighting
    /// that orders seats by `mass_s` (§6.1 point 4, §6.2 step 3), applied per candidate this time
    /// instead of summed over all of them — then sampling uniformly within it.
    ///
    /// `coarse` (§6.4 (c)) is the same candidates with [`coarsen`] applied, used instead of
    /// `candidates` whenever this seat is re-prepared per draw (every position except the cached
    /// first seat and the residual last seat, which need `candidates` unchanged: the first is
    /// prepared once regardless, and the last only ever calls `HandConstraint::satisfies`, never
    /// `Sampler::prepare`).
    ///
    /// `coarse` is deduplicated: candidates whose summaries coincide (common in real
    /// interpretations, where several alternatives differ only in `cards` / `eval` detail) are
    /// merged into one component carrying the sum of their weights. That leaves the mixture
    /// unchanged — identical components `U` with shares `v_1, v_2` are the single component `U`
    /// with share `v_1 + v_2`, since `v_i ∝ w_i · cnt` and `cnt` is shared — while preparing each
    /// distinct summary only once per draw.
    ///
    /// `coarse_direct` is `true` when the deduplicated `coarse` is the single unconstrained atom:
    /// a uniform draw of `needed` cards from the pool, exactly what [`SeatPlan::Direct`] does, so
    /// a re-prepared seat whose every alternative coarsens to `ANY` (e.g. a passing seat whose
    /// passes only carry `cards` / `eval` detail) skips `Sampler::prepare` entirely.
    ///
    /// At a re-prepared seat the *light* alternatives of `coarse` (see
    /// [`ConstraintProposal::light_threshold`]) are removed from it and folded into one uniform
    /// component drawn with the fixed probability `π_L` (`fold`, §6.4 (d) of `09-sample.md`);
    /// the rest keep the per-draw adaptive shares. See [`SeatMix`] for the density.
    Sampled {
        candidates: Vec<Candidate>,
        coarse: Vec<Candidate>,
        /// `coarse[i].constraint.to_dnf(..)`, converted once here instead of inside every
        /// per-draw `Sampler::prepare` (see `Sampler::prepare_many_dnf`).
        coarse_dnfs: Vec<Dnf>,
        coarse_direct: bool,
        /// The light alternatives folded out of `coarse` (`None` unless this is a re-prepared
        /// seat with at least one light alternative).
        fold: Option<Fold>,
    },
}

/// The light alternatives folded out of a re-prepared seat's `coarse` (§6.4 (d); see
/// [`ConstraintProposal::light_threshold`]).
struct Fold {
    /// `π_L`, the uniform component's draw probability.
    share: f64,
    /// The seat's coarse candidates before folding, with their DNFs: the mixture a draw falls
    /// back to when the kept alternatives lost most of their room on its pool.
    all: Vec<Candidate>,
    all_dnfs: Vec<Dnf>,
    /// `Σ_kept w_i · cnt_i` on the full pool.
    kept_mass_full: f64,
    /// `ln C(|full pool|, needed)`.
    ln_hands_full: f64,
}

/// A draw falls back from the folded mixture to the full one when the kept alternatives' mass
/// shrank, relative to the number of hands, below this fraction of its full-pool value.
const FOLD_FALLBACK_RATIO: f64 = 0.1;

/// A re-prepared seat's mixture on one `(pool, fixed)`.
///
/// - `Folded`: `q(h) = (1 − π_L) · Σ_{i kept, h ∈ C_i} v_i(pool) / cnt_i(pool) + π_L / C(|pool|,
///   needed)`, with `v_i ∝ w_i · cnt_i(pool)` over the kept alternatives.
/// - `Full`: `q(h) = Σ_{i, h ∈ C_i} v_i(pool) / cnt_i(pool)` over every coarse alternative (no
///   folding at this seat, or the fallback of [`Fold`]); `None` when none has support.
///
/// Which one a pool gets depends only on the pool, so both are exact densities of what
/// `propose` draws.
enum SeatMix {
    Folded {
        kept: Vec<(Sampler, f64)>,
        share: f64,
    },
    Full(Option<Vec<(Sampler, f64)>>),
}

impl SeatMix {
    fn prepare(
        coarse: &[Candidate],
        coarse_dnfs: &[Dnf],
        fold: Option<&Fold>,
        pool: Hand,
        fixed: Hand,
        needed: u8,
        opts: &bridge_constraint::SampleOptions,
    ) -> SeatMix {
        let Some(fold) = fold else {
            return SeatMix::Full(prepare_components(
                coarse,
                Some(coarse_dnfs),
                pool,
                fixed,
                opts,
            ));
        };
        let hands_shrink = (ln_choose(pool.len(), needed) - fold.ln_hands_full).exp();
        match prepare_components_with_mass(coarse, Some(coarse_dnfs), pool, fixed, opts) {
            Some((kept, mass))
                if mass >= FOLD_FALLBACK_RATIO * hands_shrink * fold.kept_mass_full =>
            {
                SeatMix::Folded {
                    kept,
                    share: fold.share,
                }
            }
            _ => SeatMix::Full(prepare_components(
                &fold.all,
                Some(&fold.all_dnfs),
                pool,
                fixed,
                opts,
            )),
        }
    }

    fn sample(
        &self,
        pool: Hand,
        fixed: Hand,
        needed: u8,
        rng: &mut dyn rand_core::Rng,
    ) -> Option<Hand> {
        let components = match self {
            SeatMix::Folded { kept, share } => {
                if uniform01(rng) < *share {
                    return Some(fixed.union(draw_subset(pool, needed, rng)));
                }
                kept
            }
            SeatMix::Full(components) => components.as_ref()?,
        };
        let i = choose_component(components, rng);
        Some(components[i].0.sample(rng)?.hand)
    }

    fn ln_density(&self, pool: Hand, needed: u8, hand: Hand) -> f64 {
        match self {
            SeatMix::Folded { kept, share } => {
                let density = (1.0 - share) * mixture_log_prob(kept, hand).exp()
                    + share * (-ln_choose(pool.len(), needed)).exp();
                if density > 0.0 {
                    density.ln()
                } else {
                    f64::NEG_INFINITY
                }
            }
            SeatMix::Full(Some(components)) => mixture_log_prob(components, hand),
            SeatMix::Full(None) => f64::NEG_INFINITY,
        }
    }
}

/// One seat in the sampling order `σ` (§6.1 point 3): most constrained first, so failure is
/// discovered as early (and as cheaply) as possible.
struct SeatEntry {
    seat: Seat,
    plan: SeatPlan,
}

/// The first seat's prepared samplers, cached because its pool (`known.pool()`) never shrinks
/// before it is drawn (§6.1 point 4): `(Sampler, v_i)` with `v_i = w_i · cnt_i / Σ_j w_j · cnt_j`
/// normalised over the candidates that survived (`count() > 0`).
struct CachedFirst {
    components: Vec<(Sampler, f64)>,
}

impl Proposal for ConstraintProposal {
    fn prepare<'c>(
        &self,
        ctx: &'c SampleContext<'c>,
    ) -> Result<Box<dyn PreparedProposal + Send + Sync + 'c>, SampleError> {
        Ok(Box::new(self.prepare_constraint(ctx)?))
    }
}

impl ConstraintProposal {
    /// [`Proposal::prepare`] without the boxing, so this module's tests can reach the prepared
    /// seat plans.
    fn prepare_constraint<'c>(
        &self,
        ctx: &'c SampleContext<'c>,
    ) -> Result<PreparedConstraint<'c>, SampleError> {
        let sampler_opts = bridge_constraint::SampleOptions {
            max_tries: self.max_retries.max(1),
            ..bridge_constraint::SampleOptions::default()
        };
        let pool = ctx.known.pool();

        let mut order: Vec<(SeatEntry, f64)> = Vec::new();
        for seat in Seat::ALL {
            let needed = ctx.known.needed(seat);
            let fixed = ctx.known.known[seat.index() as usize];
            let hard = &ctx.play_constraints[seat.index() as usize];
            if needed == 0 {
                // Fully known already (the viewer, or an exposed dummy): still must satisfy this
                // seat's own hard play constraint, or no deal exists at all (§2.3 of
                // `09-sample.md`). `sample_deals` checks this too before calling `prepare`, but
                // `ConstraintProposal::prepare` is public and can be called directly.
                if !hard.satisfies(fixed) {
                    return Err(SampleError::EmptySupport);
                }
                continue;
            }

            let candidates = seat_candidates(ctx, seat, hard);

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

            let samplers = Sampler::prepare_many(
                candidates.iter().map(|c| &c.constraint),
                pool,
                fixed,
                &sampler_opts,
            )
            .map_err(|e| SampleError::Prepare(e.to_string()))?;
            let mut alts: Vec<(Candidate, u64)> = Vec::with_capacity(candidates.len());
            for (candidate, sampler) in candidates.into_iter().zip(samplers) {
                let count = sampler.count();
                if count == 0 {
                    continue;
                }
                alts.push((candidate, count));
            }
            truncate_by_mass(&mut alts, hard, pool, fixed, &sampler_opts)?;
            if alts.is_empty() {
                // `sample_deals` (§2.3 of `09-sample.md`) already checked that at least one
                // interpretation alternative survives AND-ing with `hard` against the full pool;
                // this can still be empty when `play_soft` narrows further than that check
                // accounts for. Fall back to the hard constraint alone rather than leaving this
                // seat with no way to be drawn at all — but if even `hard` alone has no support
                // against the full pool, no deal exists, and that must surface as
                // `EmptySupport`, not a fabricated single-hand fallback (`count().max(1)` would
                // silently claim support that isn't there).
                let sampler = Sampler::prepare(hard, pool, fixed, &sampler_opts)
                    .map_err(|e| SampleError::Prepare(e.to_string()))?;
                let count = sampler.count();
                if count == 0 {
                    return Err(SampleError::EmptySupport);
                }
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
            let coarse = coarsen_candidates(&candidates);
            let coarse_direct = coarse.len() == 1 && is_unconstrained(&coarse[0].constraint);
            let coarse_dnfs = coarse
                .iter()
                .map(|c| {
                    c.constraint
                        .to_dnf(&DnfOptions::default())
                        .expect("DnfOptions::default uses Overflow::Residual, which never errors")
                })
                .collect();
            order.push((
                SeatEntry {
                    seat,
                    plan: SeatPlan::Sampled {
                        candidates,
                        coarse,
                        coarse_dnfs,
                        coarse_direct,
                        fold: None,
                    },
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
        let mut order: Vec<SeatEntry> = order.into_iter().map(|(entry, _)| entry).collect();
        // Split the re-prepared seats' coarse candidates into heavy and light tiers.
        let m = order.len();
        for entry in order.iter_mut().take(m.saturating_sub(1)).skip(1) {
            let fixed = ctx.known.known[entry.seat.index() as usize];
            if let SeatPlan::Sampled {
                coarse,
                coarse_dnfs,
                coarse_direct: false,
                fold,
                ..
            } = &mut entry.plan
            {
                *fold = fold_light(
                    self.light_threshold,
                    coarse,
                    coarse_dnfs,
                    ctx.known.needed(entry.seat),
                    pool,
                    fixed,
                    &sampler_opts,
                )?;
            }
        }

        let cached_first = match order.first() {
            Some(SeatEntry {
                seat,
                plan: SeatPlan::Sampled { candidates, .. },
            }) => {
                let fixed = ctx.known.known[seat.index() as usize];
                Some(CachedFirst {
                    components: prepare_components(candidates, None, pool, fixed, &sampler_opts)
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

        let mut prepared = PreparedConstraint {
            id: NEXT_PREPARED_ID.fetch_add(1, Ordering::Relaxed),
            ctx,
            sampler_opts,
            order,
            cached_first,
            residual_threshold: None,
            pilot_attempts: 0,
        };
        // Residual rejection's threshold, fixed once here from a bound and a pilot run of the
        // proposal without it.
        if let Some(SeatEntry {
            seat,
            plan: SeatPlan::Sampled { candidates, .. },
        }) = prepared.order.last()
        {
            if self.residual_rejection && prepared.order.len() > 1 {
                let threshold =
                    residual_threshold(&prepared, *seat, candidates, self.residual_min_acceptance);
                prepared.residual_threshold = Some(threshold);
                prepared.pilot_attempts = RESIDUAL_PILOT_DRAWS as u64;
            }
        }
        Ok(prepared)
    }
}

/// Folds a re-prepared seat's light `coarse` candidates (see
/// [`ConstraintProposal::light_threshold`]) out of `coarse` / `coarse_dnfs`; `None` when none is
/// light. Candidates with no support on the full pool are dropped from `coarse` either way (they
/// have none on any smaller pool); the heaviest candidate is always kept.
#[allow(clippy::too_many_arguments)]
fn fold_light(
    threshold: f64,
    coarse: &mut Vec<Candidate>,
    coarse_dnfs: &mut Vec<Dnf>,
    needed: u8,
    pool: Hand,
    fixed: Hand,
    opts: &bridge_constraint::SampleOptions,
) -> Result<Option<Fold>, SampleError> {
    let samplers = Sampler::prepare_many_dnf(coarse_dnfs.iter(), pool, fixed, opts)
        .map_err(|e| SampleError::Prepare(e.to_string()))?;
    let counts: Vec<f64> = samplers.iter().map(|s| s.count() as f64).collect();
    let masses: Vec<f64> = coarse
        .iter()
        .zip(&counts)
        .map(|(c, n)| c.weight * n)
        .collect();
    let total: f64 = masses.iter().sum();
    if total <= 0.0 {
        return Ok(None);
    }
    let ln_hands_full = ln_choose(pool.len(), needed);
    let hands = ln_hands_full.exp();
    let heaviest = masses
        .iter()
        .enumerate()
        .fold(0, |best, (i, m)| if *m > masses[best] { i } else { best });
    let mut share = 0.0;
    let mut kept_mass_full = 0.0;
    let mut all = Vec::with_capacity(coarse.len());
    let mut all_dnfs = Vec::with_capacity(coarse.len());
    let mut kept = Vec::with_capacity(coarse.len());
    let mut kept_dnfs = Vec::with_capacity(coarse.len());
    for (i, (candidate, dnf)) in coarse.drain(..).zip(coarse_dnfs.drain(..)).enumerate() {
        if masses[i] <= 0.0 {
            continue;
        }
        all.push(candidate.clone());
        all_dnfs.push(dnf.clone());
        if i != heaviest && masses[i] / total <= threshold * (counts[i] / hands) {
            share += masses[i] / total;
        } else {
            kept_mass_full += masses[i];
            kept.push(candidate);
            kept_dnfs.push(dnf);
        }
    }
    if share <= 0.0 {
        *coarse = all;
        *coarse_dnfs = all_dnfs;
        return Ok(None);
    }
    *coarse = kept;
    *coarse_dnfs = kept_dnfs;
    Ok(Some(Fold {
        share,
        all,
        all_dnfs,
        kept_mass_full,
        ln_hands_full,
    }))
}

/// A 53-bit uniform double in `[0, 1)` from one `rng.next_u64()`.
fn uniform01(rng: &mut dyn rand_core::Rng) -> f64 {
    (rng.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// Cuts `alts` (candidates with their `count` against the full pool, all positive) to at most
/// [`MAX_ALTERNATIVES`] without shrinking the proposal's support (§6.1 point 1, 07-bidding.md
/// §4.4), in estimated-mass order.
///
/// The candidates are ordered by `w_i · count_i`, their share of the draw, so the ones whose
/// target mass is smallest are the ones dropped (ordering by `w_i` alone would drop a broad,
/// light alternative that carries much of the mass). The product `interpretation.seats[s] ⊗
/// play_soft[s]` can have up to 64 entries; dropping the tail outright would leave every hand
/// covered only by dropped products with positive target likelihood but proposal density 0:
/// never drawn, so the estimator is biased while ESS still looks perfect. Instead the `K - 1`
/// heaviest are kept and the rest are replaced by one catch-all candidate, the seat's `hard`
/// constraint carrying the dropped weight. Every hand the target accepts satisfies `hard`, so it
/// stays proposable, and `log_prob` accounts for the catch-all through the ordinary component
/// sum. Ties keep the input order, so the result is a deterministic function of `alts`.
fn truncate_by_mass(
    alts: &mut Vec<(Candidate, u64)>,
    hard: &HandConstraint,
    pool: Hand,
    fixed: Hand,
    opts: &bridge_constraint::SampleOptions,
) -> Result<(), SampleError> {
    let mass = |(c, count): &(Candidate, u64)| c.weight * *count as f64;
    alts.sort_by(|a, b| {
        mass(b)
            .partial_cmp(&mass(a))
            .unwrap_or(core::cmp::Ordering::Equal)
    });
    if alts.len() <= MAX_ALTERNATIVES {
        return Ok(());
    }
    let dropped: f64 = alts[MAX_ALTERNATIVES - 1..]
        .iter()
        .map(|(c, _)| c.weight)
        .sum();
    alts.truncate(MAX_ALTERNATIVES - 1);
    let count = Sampler::prepare(hard, pool, fixed, opts)
        .map_err(|e| SampleError::Prepare(e.to_string()))?
        .count();
    alts.push((
        Candidate {
            constraint: hard.clone(),
            weight: dropped,
        },
        count,
    ));
    Ok(())
}

/// A bound `U ≥ m(h) = Σ_{i: h ∈ C_i} w_i` over every hand, the largest threshold residual
/// rejection uses (see [`ConstraintProposal::residual_rejection`]).
///
/// `U = max_i Σ_{j: sup_i ∩ sup_j ≠ ∅} w_j`, with `sup_i` the (shape, HCP) grid superset of
/// candidate `i` (`bridge_constraint::grid::bounds`). It is a valid bound: for any hand `h` and
/// any candidate `i` containing it, every candidate `j` that also contains `h` has `h`'s cell in
/// `sup_i ∩ sup_j`, so `m(h)` sums over a subset of `i`'s overlap set. It is tight when the
/// alternatives are pairwise disjoint on the grid (the policy mirror's exclusive pieces), where
/// it is `max_i w_i`, and never above `Σ_i w_i`.
fn residual_bound(candidates: &[Candidate]) -> f64 {
    let sups: Vec<HcpShapeGrid> = candidates
        .iter()
        .map(|c| bridge_constraint::grid::bounds(&c.constraint).sup)
        .collect();
    let mut bound = 0.0f64;
    for (i, sup_i) in sups.iter().enumerate() {
        let overlapping: f64 = candidates
            .iter()
            .zip(&sups)
            .enumerate()
            .filter(|&(j, (_, sup_j))| j == i || sup_i.intersects(sup_j))
            .map(|(_, (c, _))| c.weight)
            .sum();
        bound = bound.max(overlapping);
    }
    bound
}

/// Pilot proposals [`residual_threshold`] draws (from a fixed RNG stream).
pub(crate) const RESIDUAL_PILOT_DRAWS: usize = 128;

/// Master seed of the pilot's RNG stream (only ever used here).
const RESIDUAL_PILOT_SEED: u64 = 0x9E51_D0A1_0000_0001;

/// `m(hand) = Σ_{i: hand ∈ C_i} w_i`.
fn residual_mixture(candidates: &[Candidate], hand: Hand) -> f64 {
    candidates
        .iter()
        .filter(|c| c.constraint.satisfies(hand))
        .map(|c| c.weight)
        .sum()
}

/// Residual rejection's threshold `T` (see [`ConstraintProposal::residual_rejection`]):
/// `min(U, max pilot m, T_min_acceptance)`, from [`RESIDUAL_PILOT_DRAWS`] proposals of `prepared`
/// (whose own threshold is still `None`, so the pilot is the proposal without rejection).
/// Deterministic: the pilot uses its own fixed RNG stream.
fn residual_threshold(
    prepared: &PreparedConstraint<'_>,
    seat: Seat,
    candidates: &[Candidate],
    min_acceptance: f64,
) -> f64 {
    let bound = residual_bound(candidates);
    let mut rng = crate::rng_for(RESIDUAL_PILOT_SEED, 0);
    let mut masses: Vec<f64> = (0..RESIDUAL_PILOT_DRAWS)
        .filter_map(|_| prepared.propose(&mut rng))
        .map(|deal| residual_mixture(candidates, deal.hand(seat)))
        .filter(|&m| m > 0.0)
        .collect();
    if masses.is_empty() {
        return bound;
    }
    masses.sort_by(f64::total_cmp);
    let max = masses[masses.len() - 1];
    // Mean pilot acceptance at threshold `t`: non-increasing in `t`, 1 at `t = min m`.
    let acceptance =
        |t: f64| masses.iter().map(|&m| (m / t).min(1.0)).sum::<f64>() / masses.len() as f64;
    let mut threshold = bound.min(max);
    if acceptance(threshold) < min_acceptance {
        // Bisect in the log domain between `min m` (acceptance 1) and `threshold`.
        let (mut lo, mut hi) = (masses[0].ln(), threshold.ln());
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if acceptance(mid.exp()) >= min_acceptance {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        threshold = lo.exp();
    }
    threshold
}

/// `ln a(hand) = ln min(1, m(hand) / threshold)` of residual rejection (`-∞` outside every
/// candidate).
fn residual_ln_accept(candidates: &[Candidate], threshold: f64, hand: Hand) -> f64 {
    let m = residual_mixture(candidates, hand);
    if m <= 0.0 || threshold <= 0.0 {
        f64::NEG_INFINITY
    } else {
        (m / threshold).min(1.0).ln()
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
    // A literal-free atom, or a flat `Or` of them (the policy mirror's exclusive pieces), is
    // already what `Sampler` handles exactly and cheaply: summarising it would only widen it.
    if is_literal_free_atoms(constraint) {
        return constraint.clone();
    }
    // Otherwise the tightest literal-free superset on the (shape, HCP) grid: exact for a
    // literal-free tree (e.g. `And(C, Not(Or(higher)))`), the literal atoms' boxes otherwise.
    // Capped at `COARSE_ATOM_CAP` atoms, which only ever widens it further.
    let sup = bridge_constraint::grid::bounds(constraint).sup;
    match sup.hull() {
        // No 13-card cell: `constraint` is unsatisfiable, and so is its summary.
        None => HandConstraint::Or(Vec::new()),
        Some((shapes, hcp)) if sup == HcpShapeGrid::from_box(shapes, hcp.clone()) => {
            HandConstraint::Atom(Atom {
                shapes,
                hcp,
                cards: Vec::new(),
                eval: Vec::new(),
            })
        }
        Some(_) => sup.to_constraint(&Atom::ANY, COARSE_ATOM_CAP),
    }
}

/// Most atoms a [`coarsen`]ed summary may have (each is one DNF term `Sampler` prepares on every
/// draw of a re-prepared seat).
const COARSE_ATOM_CAP: usize = 8;

/// Whether `constraint` is a literal-free atom or a non-empty `Or` of them.
fn is_literal_free_atoms(constraint: &HandConstraint) -> bool {
    let free = |c: &HandConstraint| matches!(c, HandConstraint::Atom(a) if a.cards.is_empty() && a.eval.is_empty());
    match constraint {
        HandConstraint::Or(children) => !children.is_empty() && children.iter().all(free),
        other => free(other),
    }
}

/// [`coarsen`] applied to every candidate, with identical summaries merged into one candidate
/// whose weight is the sum of theirs (see [`SeatPlan::Sampled`]). First-occurrence order is kept,
/// so the result is a deterministic function of `candidates`.
fn coarsen_candidates(candidates: &[Candidate]) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::with_capacity(candidates.len());
    for c in candidates {
        let constraint = coarsen(&c.constraint);
        match out
            .iter_mut()
            .find(|o| same_atom(&o.constraint, &constraint))
        {
            Some(existing) => existing.weight += c.weight,
            None => out.push(Candidate {
                constraint,
                weight: c.weight,
            }),
        }
    }
    out
}

/// Whether `a` and `b` are the same atom, or `Or`s of the same atoms in the same order (every
/// [`coarsen`] output is one of these).
fn same_atom(a: &HandConstraint, b: &HandConstraint) -> bool {
    match (a, b) {
        (HandConstraint::Atom(a), HandConstraint::Atom(b)) => a == b,
        (HandConstraint::Or(a), HandConstraint::Or(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same_atom(x, y))
        }
        _ => false,
    }
}

/// Prepares one `Sampler` per candidate against `(pool, fixed)`, drops the ones with
/// `count() == 0`, and normalises the survivors' weights into `v_i = w_i · cnt_i / Σ_j w_j ·
/// cnt_j` (§6.1 point 4, §6.2 step 2 of `09-sample.md`). `None` when nothing survives
/// (`propose`/`log_prob` treat that as "no deal is possible from here").
///
/// The `cnt_i` factor matters: without it, a component's *share of the draw* would not match its
/// *share of the likelihood mass* `Interpretation::likelihood` actually scores against. That
/// target sums `w_i` over every alternative a hand satisfies (07-bidding.md §4.4), so a component
/// with a broader satisfying set contributes more total likelihood mass across the pool, not just
/// more per-hand likelihood — an ε-mixture's defensive `ANY` branch, in particular, satisfies far
/// more hands than the primary alternative it sits beside. Weighting by `w_i` alone would draw
/// `ANY` only in proportion to its raw `ε`, far less often than the mass it actually accounts for;
/// the few hands it then does produce would carry disproportionately large importance weights and
/// ESS would collapse.
///
/// `dnfs`, when given, is each candidate's constraint already in DNF (same order), and is
/// prepared through `Sampler::prepare_many_dnf` — identical samplers without the per-call
/// conversion.
fn prepare_components(
    candidates: &[Candidate],
    dnfs: Option<&[Dnf]>,
    pool: Hand,
    fixed: Hand,
    opts: &bridge_constraint::SampleOptions,
) -> Option<Vec<(Sampler, f64)>> {
    prepare_components_with_mass(candidates, dnfs, pool, fixed, opts).map(|(c, _)| c)
}

/// [`prepare_components`], also returning the mass `Σ_i w_i · cnt_i` the shares were normalised
/// by.
fn prepare_components_with_mass(
    candidates: &[Candidate],
    dnfs: Option<&[Dnf]>,
    pool: Hand,
    fixed: Hand,
    opts: &bridge_constraint::SampleOptions,
) -> Option<(Vec<(Sampler, f64)>, f64)> {
    // One `prepare_many` call rather than a `Sampler::prepare` per candidate: every candidate
    // is prepared against the same `(pool, fixed)`, so terms with plain per-suit tables (all of
    // a re-prepared seat's coarse shape + HCP summaries) share one table build and their pair
    // convolutions (`Sampler::prepare_many`'s doc comment); the samplers are identical either way.
    let samplers = match dnfs {
        Some(dnfs) => Sampler::prepare_many_dnf(dnfs, pool, fixed, opts),
        None => Sampler::prepare_many(candidates.iter().map(|c| &c.constraint), pool, fixed, opts),
    }
    .ok()?;
    let mut survivors: Vec<(Sampler, f64)> = Vec::with_capacity(candidates.len());
    for (candidate, sampler) in candidates.iter().zip(samplers) {
        let count = sampler.count();
        if count == 0 {
            continue;
        }
        survivors.push((sampler, candidate.weight * count as f64));
    }
    let total: f64 = survivors.iter().map(|(_, w)| *w).sum();
    if survivors.is_empty() || total <= 0.0 {
        return None;
    }
    for (_, w) in &mut survivors {
        *w /= total;
    }
    Some((survivors, total))
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

/// Whether the seat at position `k` (not the residual last seat) is drawn as a uniform subset of
/// the remaining pool: always for [`SeatPlan::Direct`], and for a re-prepared (`k != 0`)
/// [`SeatPlan::Sampled`] seat whose coarse summary is exactly `ANY` (`coarse_direct`). The cached
/// first seat keeps its fine candidates, so `coarse_direct` never applies there.
fn draws_direct(entry: &SeatEntry, k: usize) -> bool {
    match &entry.plan {
        SeatPlan::Direct => true,
        SeatPlan::Sampled { coarse_direct, .. } => k != 0 && *coarse_direct,
    }
}

struct PreparedConstraint<'c> {
    /// Unique per `prepare_constraint` call; tags this proposal's `REPREPARE_CACHE` entries so
    /// another proposal on the same thread never reads them.
    id: u64,
    ctx: &'c SampleContext<'c>,
    sampler_opts: bridge_constraint::SampleOptions,
    /// Seats needing cards, most constrained first (§6.1 point 3); the last entry is the
    /// residual seat that receives whatever remains of the pool.
    order: Vec<SeatEntry>,
    /// `Some` when `order`'s first entry is `Sampled` (its pool is the full pool and never
    /// shrinks before it is drawn, so it is prepared once here rather than in every `propose`).
    cached_first: Option<CachedFirst>,
    /// `Some(T)` when residual rejection is on and the last seat is `Sampled`: the threshold of
    /// [`residual_threshold`].
    residual_threshold: Option<f64>,
    /// Proposals [`residual_threshold`]'s pilot drew ([`RESIDUAL_PILOT_DRAWS`] when it ran, else
    /// 0), reported through [`PreparedProposal::pilot_attempts`].
    pilot_attempts: u64,
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
                if let (Some(bound), SeatPlan::Sampled { candidates, .. }) =
                    (self.residual_threshold, &entry.plan)
                {
                    let a = residual_ln_accept(candidates, bound, hand).exp();
                    let u = (rng.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64);
                    if u >= a {
                        return None;
                    }
                }
                hands[seat.index() as usize] = hand;
                break;
            }

            let hand = match &entry.plan {
                _ if draws_direct(entry, k) => {
                    let needed = known.needed(seat);
                    fixed.union(draw_subset(pool, needed, rng))
                }
                SeatPlan::Direct => unreachable!("draws_direct is true for every Direct seat"),
                SeatPlan::Sampled {
                    coarse,
                    coarse_dnfs,
                    fold,
                    ..
                } if k != 0 => {
                    // §6.4 (c): re-prepared every draw, so use the coarse candidates; §6.4 (d):
                    // only the kept ones, the light ones being folded into a uniform draw, unless
                    // this pool left the kept ones too little room (see `SeatMix`). The prepared
                    // mixture is stashed in `REPREPARE_CACHE[k]` for the `log_prob` call
                    // `sample_deals` makes on this same deal right after (see the thread-local's
                    // doc comment above).
                    let hand = REPREPARE_CACHE.with(|cache| -> Option<Hand> {
                        let mut cache = cache.borrow_mut();
                        if cache.len() != m {
                            cache.clear();
                            cache.resize_with(m, || None);
                        }
                        let mix = SeatMix::prepare(
                            coarse,
                            coarse_dnfs,
                            fold.as_ref(),
                            pool,
                            fixed,
                            known.needed(seat),
                            &self.sampler_opts,
                        );
                        let hand = mix.sample(pool, fixed, known.needed(seat), rng);
                        cache[k] = Some(SeatCache {
                            id: self.id,
                            pool,
                            fixed,
                            mix,
                        });
                        hand
                    });
                    hand?
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
                if let (Some(bound), SeatPlan::Sampled { candidates, .. }) =
                    (self.residual_threshold, &entry.plan)
                {
                    ln_pi += residual_ln_accept(candidates, bound, hand);
                }
                break;
            }

            match &entry.plan {
                _ if draws_direct(entry, k) => {
                    let needed = known.needed(seat);
                    ln_pi += -ln_choose(pool.len(), needed);
                }
                SeatPlan::Direct => unreachable!("draws_direct is true for every Direct seat"),
                SeatPlan::Sampled {
                    coarse,
                    coarse_dnfs,
                    fold,
                    ..
                } if k != 0 => {
                    // Sum over every component that could have produced `hand` (they overlap):
                    // the mixture density is only correct when every one is counted (§6.3).
                    // §6.4 (c)/(d): replay the same mixture `propose` drew this seat from —
                    // reusing `REPREPARE_CACHE[k]` when it was left by a `propose` call of this
                    // same proposal on this thread for this exact `(pool, fixed)` (see the
                    // thread-local's doc comment above), rebuilding otherwise.
                    let needed = known.needed(seat);
                    let ln_component = REPREPARE_CACHE.with(|cache| -> f64 {
                        let cache = cache.borrow();
                        match cache.get(k) {
                            Some(Some(entry))
                                if entry.id == self.id
                                    && entry.pool == pool
                                    && entry.fixed == fixed =>
                            {
                                entry.mix.ln_density(pool, needed, hand)
                            }
                            _ => SeatMix::prepare(
                                coarse,
                                coarse_dnfs,
                                fold.as_ref(),
                                pool,
                                fixed,
                                needed,
                                &self.sampler_opts,
                            )
                            .ln_density(pool, needed, hand),
                        }
                    });
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

    fn pilot_attempts(&self) -> u64 {
        self.pilot_attempts
    }
}

#[cfg(test)]
mod tests {
    use bridge_bidding::{Explanation, Interpretation, ResolutionKind};
    use bridge_constraint::{CardRequirement, KnownCards, ShapeSet};
    use bridge_core::{Holding, Suit};

    use super::*;

    /// The pre-optimisation `log_prob`: every re-prepared seat's candidates coarsened one by one
    /// (no merging of identical summaries, no `coarse_direct` shortcut) and prepared from
    /// scratch (no `REPREPARE_CACHE`). The differential tests below hold the optimised
    /// `PreparedConstraint::log_prob` to this reference.
    fn reference_log_prob(prepared: &PreparedConstraint<'_>, deal: &Deal) -> f64 {
        let known = &prepared.ctx.known;
        for seat in Seat::ALL {
            if !known.known[seat.index() as usize].is_subset(deal.hand(seat)) {
                return f64::NEG_INFINITY;
            }
        }
        let mut pool = known.pool();
        let mut ln_pi = 0.0f64;
        let m = prepared.order.len();
        for (k, entry) in prepared.order.iter().enumerate() {
            let seat = entry.seat;
            let fixed = known.known[seat.index() as usize];
            let hand = deal.hand(seat);
            if k == m - 1 {
                if !prepared.ctx.play_constraints[seat.index() as usize].satisfies(hand) {
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
                if let (Some(threshold), SeatPlan::Sampled { candidates, .. }) =
                    (prepared.residual_threshold, &entry.plan)
                {
                    ln_pi += residual_ln_accept(candidates, threshold, hand);
                }
                break;
            }
            let ln_component = match &entry.plan {
                SeatPlan::Direct => -ln_choose(pool.len(), known.needed(seat)),
                SeatPlan::Sampled { candidates, .. } if k == 0 => {
                    match prepare_components(candidates, None, pool, fixed, &prepared.sampler_opts)
                    {
                        Some(components) => mixture_log_prob(&components, hand),
                        None => f64::NEG_INFINITY,
                    }
                }
                SeatPlan::Sampled {
                    candidates,
                    coarse,
                    fold,
                    ..
                } => {
                    // Every original candidate, coarsened and unmerged; the kept ones are those
                    // whose summary survived folding.
                    let all: Vec<Candidate> = candidates
                        .iter()
                        .map(|c| Candidate {
                            constraint: coarsen(&c.constraint),
                            weight: c.weight,
                        })
                        .collect();
                    let kept: Vec<Candidate> = all
                        .iter()
                        .filter(|c| {
                            coarse
                                .iter()
                                .any(|h| same_atom(&h.constraint, &c.constraint))
                        })
                        .cloned()
                        .collect();
                    let needed = known.needed(seat);
                    let opts = &prepared.sampler_opts;
                    let full = |cands: &[Candidate]| match prepare_components(
                        cands, None, pool, fixed, opts,
                    ) {
                        Some(components) => mixture_log_prob(&components, hand),
                        None => f64::NEG_INFINITY,
                    };
                    match fold {
                        None => full(&kept),
                        Some(fold) => {
                            // The fallback rule, recomputed from scratch.
                            let mass_on = |p: Hand| {
                                prepare_components_with_mass(&kept, None, p, fixed, opts)
                                    .map_or(0.0, |(_, mass)| mass)
                            };
                            let full_pool = known.pool();
                            let shrink = (ln_choose(pool.len(), needed)
                                - ln_choose(full_pool.len(), needed))
                            .exp();
                            if mass_on(pool) < FOLD_FALLBACK_RATIO * shrink * mass_on(full_pool) {
                                full(&all)
                            } else {
                                let components = prepare_components(&kept, None, pool, fixed, opts)
                                    .expect("the kept mass is positive");
                                let density = (1.0 - fold.share)
                                    * mixture_log_prob(&components, hand).exp()
                                    + fold.share * (-ln_choose(pool.len(), needed)).exp();
                                if density > 0.0 {
                                    density.ln()
                                } else {
                                    f64::NEG_INFINITY
                                }
                            }
                        }
                    }
                }
            };
            if !ln_component.is_finite() {
                return f64::NEG_INFINITY;
            }
            ln_pi += ln_component;
            pool = pool.difference(hand.difference(fixed));
        }
        ln_pi
    }

    fn explanation() -> Explanation {
        Explanation {
            text: String::new(),
            node: None,
            resolution: ResolutionKind::Exact,
            parts: Vec::new(),
        }
    }

    fn holds(suit: Suit, rank_index: u8) -> CardRequirement {
        let ranks = Holding::from_bits(1 << rank_index).expect("rank_index < 13");
        CardRequirement::in_suit(suit, ranks, 1..=1)
    }

    fn atom(
        shapes: ShapeSet,
        hcp: core::ops::RangeInclusive<u8>,
        cards: Vec<CardRequirement>,
    ) -> HandConstraint {
        HandConstraint::Atom(Atom {
            shapes,
            hcp,
            cards,
            eval: Vec::new(),
        })
    }

    /// A full-deck context shaped like the real SAYC interpretations (§10.1 of `09-sample.md`):
    /// North is the tight opener (cached first seat); South has several alternatives, two pairs
    /// of which coarsen to the same summary (they differ only in a `cards` literal); East's
    /// alternatives all coarsen to `ANY` (`coarse_direct`); West is the residual seat.
    fn sayc_like_interpretation() -> Interpretation {
        let north = vec![
            (
                atom(ShapeSet::BALANCED, 15..=17, Vec::new()),
                0.95,
                explanation(),
            ),
            (HandConstraint::ANY, 0.05, explanation()),
        ];
        let east = vec![
            (
                atom(ShapeSet::ALL, 0..=37, vec![holds(Suit::Hearts, 12)]),
                0.5,
                explanation(),
            ),
            (
                atom(ShapeSet::ALL, 0..=37, vec![holds(Suit::Clubs, 11)]),
                0.3,
                explanation(),
            ),
            (HandConstraint::ANY, 0.2, explanation()),
        ];
        let four_spades = ShapeSet::from_suit_len(Suit::Spades, 4, 13);
        let south = vec![
            (
                atom(four_spades, 8..=37, vec![holds(Suit::Spades, 12)]),
                0.35,
                explanation(),
            ),
            (
                atom(four_spades, 8..=37, vec![holds(Suit::Spades, 11)]),
                0.25,
                explanation(),
            ),
            (
                atom(ShapeSet::BALANCED, 13..=15, vec![holds(Suit::Diamonds, 12)]),
                0.2,
                explanation(),
            ),
            (
                atom(ShapeSet::BALANCED, 13..=15, Vec::new()),
                0.1,
                explanation(),
            ),
            (HandConstraint::ANY, 0.1, explanation()),
            // Light at the default threshold: tiny shares of the full-pool mass.
            (
                atom(ShapeSet::ALL, 24..=37, vec![holds(Suit::Clubs, 12)]),
                0.001,
                explanation(),
            ),
            (atom(ShapeSet::ALL, 0..=1, Vec::new()), 0.001, explanation()),
        ];
        let west = vec![
            (atom(ShapeSet::ALL, 0..=9, Vec::new()), 0.8, explanation()),
            (HandConstraint::ANY, 0.2, explanation()),
        ];
        Interpretation {
            seats: [north, east, south, west],
            per_call: Vec::new(),
            divergence: None,
        }
    }

    /// Differential test for the merged-summary and `coarse_direct` fast paths (and the
    /// `REPREPARE_CACHE` reuse): on deals `propose` actually produces, and on uniformly random
    /// deals (mostly outside the tight seats' support), `log_prob` must equal the unoptimised
    /// reference to floating-point rounding.
    #[test]
    fn log_prob_matches_the_unmerged_reference() {
        let interpretation = sayc_like_interpretation();
        let play_constraints = [
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
        ];
        let ctx = SampleContext {
            known: KnownCards::EMPTY,
            interpretation: &interpretation,
            play_constraints: &play_constraints,
            play_soft: None,
            bidding: None,
        };
        // Residual rejection on (off by default), so the reference's `ln a` term is covered too.
        let prepared = ConstraintProposal {
            residual_rejection: true,
            ..ConstraintProposal::default()
        }
        .prepare_constraint(&ctx)
        .expect("every seat has support");

        // The fixture must actually exercise every fast path.
        let mut merged = false;
        let mut direct = false;
        let mut light = false;
        for (k, entry) in prepared.order.iter().enumerate() {
            if let SeatPlan::Sampled {
                candidates,
                coarse,
                coarse_direct,
                fold,
                ..
            } = &entry.plan
            {
                if k != 0 && k + 1 != prepared.order.len() {
                    merged |= coarse.len() < candidates.len() && !*coarse_direct;
                    direct |= *coarse_direct;
                    light |= fold.is_some();
                }
            }
        }
        assert!(merged, "no re-prepared seat merged identical summaries");
        assert!(direct, "no re-prepared seat took the coarse_direct path");
        assert!(light, "no re-prepared seat folded a light alternative");

        let mut rng = crate::rng_for(0x5EED, 0);
        let mut checked = 0;
        for _ in 0..400 {
            let Some(deal) = prepared.propose(&mut rng) else {
                continue;
            };
            let fast = prepared.log_prob(&deal);
            let reference = reference_log_prob(&prepared, &deal);
            assert!(
                fast.is_finite(),
                "a proposed deal must have finite log_prob"
            );
            assert!(
                (fast - reference).abs() < 1e-9,
                "log_prob {fast} != reference {reference} for {deal:?}"
            );
            checked += 1;
        }
        assert!(checked > 100, "only {checked} proposals succeeded");

        let uniform = crate::UniformProposal;
        let uniform_prepared = uniform.prepare(&ctx).expect("uniform always prepares");
        for _ in 0..400 {
            let deal = uniform_prepared
                .propose(&mut rng)
                .expect("uniform proposals never fail");
            let fast = prepared.log_prob(&deal);
            let reference = reference_log_prob(&prepared, &deal);
            if reference.is_finite() {
                assert!(
                    (fast - reference).abs() < 1e-9,
                    "log_prob {fast} != reference {reference} for {deal:?}"
                );
            } else {
                assert_eq!(fast, f64::NEG_INFINITY, "support differs for {deal:?}");
            }
        }
    }

    /// §6.4 (d)'s fallback: North (drawn first) either holds the four deuces or has 22+ HCP; South
    /// (re-prepared) is 19+ HCP with a light `ANY` folded out. After a 22+ North the kept
    /// alternative has (almost) no room, so the draw must fall back to the full mixture; after
    /// the deuces it keeps the folded one. Both must match the reference exactly.
    #[test]
    fn fold_fallback_matches_the_reference() {
        let deuces = [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs]
            .into_iter()
            .map(|suit| holds(suit, 0))
            .collect();
        let interpretation = Interpretation {
            seats: [
                vec![
                    (atom(ShapeSet::ALL, 0..=37, deuces), 0.5, explanation()),
                    (atom(ShapeSet::ALL, 22..=37, Vec::new()), 0.5, explanation()),
                ],
                vec![(HandConstraint::ANY, 1.0, explanation())],
                vec![
                    (atom(ShapeSet::ALL, 19..=37, Vec::new()), 1.0, explanation()),
                    (HandConstraint::ANY, 1e-5, explanation()),
                ],
                vec![(HandConstraint::ANY, 1.0, explanation())],
            ],
            per_call: Vec::new(),
            divergence: None,
        };
        let play_constraints = [
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
        ];
        let ctx = SampleContext {
            known: KnownCards::EMPTY,
            interpretation: &interpretation,
            play_constraints: &play_constraints,
            play_soft: None,
            bidding: None,
        };
        // Residual rejection on (off by default), so the reference's `ln a` term is covered too.
        let prepared = ConstraintProposal {
            residual_rejection: true,
            ..ConstraintProposal::default()
        }
        .prepare_constraint(&ctx)
        .expect("every seat has support");
        let south = prepared
            .order
            .iter()
            .position(|e| e.seat == Seat::South)
            .expect("South is sampled");
        assert!(
            matches!(
                &prepared.order[south].plan,
                SeatPlan::Sampled { fold: Some(_), .. }
            ),
            "South's ANY must be folded"
        );

        let mut rng = crate::rng_for(0xF01D, 0);
        let (mut folded, mut fallbacks) = (0, 0);
        for _ in 0..600 {
            let Some(deal) = prepared.propose(&mut rng) else {
                continue;
            };
            REPREPARE_CACHE.with(|cache| match &cache.borrow()[south] {
                Some(SeatCache {
                    mix: SeatMix::Folded { .. },
                    ..
                }) => folded += 1,
                Some(SeatCache {
                    mix: SeatMix::Full(_),
                    ..
                }) => fallbacks += 1,
                None => panic!("South's mixture was not cached"),
            });
            let fast = prepared.log_prob(&deal);
            let reference = reference_log_prob(&prepared, &deal);
            assert!(
                fast.is_finite(),
                "a proposed deal must have finite log_prob"
            );
            assert!(
                (fast - reference).abs() < 1e-9,
                "log_prob {fast} != reference {reference} for {deal:?}"
            );
        }
        assert!(
            folded > 50 && fallbacks > 50,
            "folded {folded}, fallbacks {fallbacks}"
        );
    }

    /// Regression: `REPREPARE_CACHE` used to be keyed only by seat position and `(pool, fixed)`,
    /// so `b.log_prob(d)` right after `a.propose()` on the same thread read `a`'s mixture at the
    /// re-prepared middle seat whenever both proposals had the same seat order. The two proposals
    /// here differ only in South's (re-prepared) alternative weights.
    #[test]
    fn log_prob_ignores_another_proposals_cache_entries() {
        let interpretation_a = sayc_like_interpretation();
        let mut interpretation_b = sayc_like_interpretation();
        let south = &mut interpretation_b.seats[Seat::South.index() as usize];
        let n = south.len() as f32;
        for (i, alt) in south.iter_mut().enumerate() {
            // Reverse the weight ordering: the last alternatives now dominate.
            alt.1 = (i as f32 + 1.0) / (n * (n + 1.0) / 2.0);
        }
        let play_constraints = [
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
        ];
        let ctx_a = SampleContext {
            known: KnownCards::EMPTY,
            interpretation: &interpretation_a,
            play_constraints: &play_constraints,
            play_soft: None,
            bidding: None,
        };
        let ctx_b = SampleContext {
            interpretation: &interpretation_b,
            ..ctx_a
        };
        let a = ConstraintProposal::default()
            .prepare_constraint(&ctx_a)
            .expect("every seat has support");
        let b = ConstraintProposal::default()
            .prepare_constraint(&ctx_b)
            .expect("every seat has support");
        let order = |p: &PreparedConstraint<'_>| p.order.iter().map(|e| e.seat).collect::<Vec<_>>();
        assert_eq!(
            order(&a),
            order(&b),
            "the fixture needs both proposals to share one seat order"
        );

        let mut rng = crate::rng_for(0xCAC4E, 0);
        let mut checked = 0;
        for _ in 0..200 {
            let Some(deal) = a.propose(&mut rng) else {
                continue;
            };
            let got = b.log_prob(&deal);
            let reference = reference_log_prob(&b, &deal);
            if reference.is_finite() {
                assert!(
                    (got - reference).abs() < 1e-9,
                    "b.log_prob {got} != reference {reference} after a.propose for {deal:?}"
                );
            } else {
                assert_eq!(got, f64::NEG_INFINITY, "support differs for {deal:?}");
            }
            // And `a` still reads its own entries correctly after `b` ran.
            let own = a.log_prob(&deal);
            assert!((own - reference_log_prob(&a, &deal)).abs() < 1e-9);
            checked += 1;
        }
        assert!(checked > 50, "only {checked} proposals succeeded");
    }

    /// `ConstraintProposal::prepare` is public and can be called directly, without going through
    /// `sample_deals`'s own §2.3 probe. A hard play constraint with no support against the full
    /// pool must surface as `SampleError::EmptySupport`, not a misleading `SampleError::Prepare`
    /// (the `alts.is_empty()` fallback used to mask a zero count with `count().max(1)`, claiming
    /// a single fabricated hand of support that did not exist).
    #[test]
    fn direct_prepare_unsat_hard() {
        // East, South and West are fully known and between them hold every club, diamond and
        // heart; North (needed = 13) draws from what's left — only spades — but its own hard
        // play constraint requires at least one club, which the pool cannot supply.
        let clubs = Hand::EMPTY.with_holding(Suit::Clubs, bridge_core::Holding::FULL);
        let diamonds = Hand::EMPTY.with_holding(Suit::Diamonds, bridge_core::Holding::FULL);
        let hearts = Hand::EMPTY.with_holding(Suit::Hearts, bridge_core::Holding::FULL);
        let known = KnownCards::new([Hand::EMPTY, clubs, diamonds, hearts])
            .expect("the three fixed hands are pairwise disjoint by construction");
        assert_eq!(known.needed(Seat::North), 13);
        assert_eq!(
            known.pool().holding(Suit::Clubs),
            bridge_core::Holding::EMPTY
        );

        let requires_a_club = HandConstraint::Atom(Atom {
            shapes: ShapeSet::ALL,
            hcp: 0..=37,
            cards: vec![CardRequirement {
                mask: clubs,
                count: 1..=13,
            }],
            eval: Vec::new(),
        });
        let play_constraints = [
            requires_a_club,
            HandConstraint::ANY,
            HandConstraint::ANY,
            HandConstraint::ANY,
        ];
        let interpretation = Interpretation {
            seats: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            per_call: Vec::new(),
            divergence: None,
        };
        let ctx = SampleContext {
            known,
            interpretation: &interpretation,
            play_constraints: &play_constraints,
            play_soft: None,
            bidding: None,
        };

        let result = ConstraintProposal::default().prepare(&ctx);
        assert!(
            result.is_err(),
            "expected an error, got a successfully prepared proposal"
        );
        assert!(
            matches!(result.err(), Some(SampleError::EmptySupport)),
            "expected SampleError::EmptySupport"
        );
    }
}
