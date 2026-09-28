//! Residual rejection and the phase-4 coarsening (`09-sample.md` §6.4 (c), §6.5) keep
//! `ConstraintProposal` exact.
//!
//! The fixture is `support::multi_component_rejecting_last_seat`'s nine-card pool (East cached,
//! South re-prepared in the middle, West residual), with West given two weighted alternatives
//! so that residual rejection's acceptance `a(h) = m(h) / U` actually varies with the hand:
//!
//! - overlapping: `[holds the diamond ace: 0.7, ANY: 0.3]` (both are the whole grid, so the
//!   bound is `U = 1.0` and `a ∈ {1, 0.3}`);
//! - disjoint: `[HCP 0..=14: 0.6, HCP 15..=37: 0.4]` (West holds the three jacks and 4 of the
//!   pool's aces, kings and queens: 12-18 HCP) (disjoint on the grid, so `U = 0.6`, the largest
//!   weight, and `a ∈ {1, 2/3}`);
//! - clipped: `[holds the diamond and heart aces: 0.9, ANY: 0.1]` with the default acceptance
//!   floor of 0.5. West rarely holds both aces, so the pilot's acceptance at `U = 1` is below the
//!   floor and the threshold is bisected to some `T` in `(0.1, 1)`: `a = 1` (clipped) for the
//!   hands holding both aces and `0.1 / T` for the rest. This is the path most real cases take.
//!
//! The clipped variant also runs with South's alternatives replaced by `[HCP 8..=10 and the
//! heart king: 0.6, HCP 6..=8: 0.4, ANY: 0.001]`: the `cards` literal makes the re-prepared
//! middle seat's coarsening (§6.4 (c)) drop it, and the `ANY` alternative is light enough to be
//! folded into the uniform component (§6.4 (d)).
//!
//! The first three checks run for the overlapping and disjoint variants (checks 1 and 2 with the
//! floor off, so `T = U`); `clipped_threshold_*` runs checks 1 and 2 for the clipped variant,
//! and check 3 covers every variant:
//!
//! 1. `log_prob` with residual rejection equals `log_prob` without it plus `ln a(h_West)`, on
//!    every enumerated deal.
//! 2. `propose` (retried on rejection) matches `exp(log_prob) / Σ exp(log_prob)` by chi-square.
//! 3. Unbiasedness: `sample_deals` with the per-seat mixtures as the target likelihood (one call
//!    per seat, `bidding: None`) gives weighted estimates of "who holds the heart ace × who
//!    holds the diamond ace" that match the exact posterior from full enumeration (Wald
//!    chi-square with the self-normalised covariance, p > 0.01), with residual rejection on and
//!    off.

use bridge_bidding::{CallExplanation, CallInterpretation, Explanation, ResolutionKind};
use bridge_constraint::{Atom, CardRequirement, HandConstraint, ShapeSet};
use bridge_core::{Call, Card, Deal, Hand, Holding, Rank, Seat, Suit};
use bridge_sample::{
    ConstraintProposal, Proposal, SampleContext, SampleOptions, Threads, WeightedDeal, rng_for,
    sample_deals,
};

mod support;
use support::{
    MultiComponentContext, chi_square_p_value, chi_square_statistic,
    multi_component_rejecting_last_seat, subsets_of_size,
};

fn explanation() -> Explanation {
    Explanation {
        text: String::new(),
        node: None,
        resolution: ResolutionKind::Exact,
        parts: Vec::new(),
    }
}

fn hcp_atom(hcp: core::ops::RangeInclusive<u8>) -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: ShapeSet::ALL,
        hcp,
        cards: Vec::new(),
        eval: Vec::new(),
    })
}

fn holds_diamond_ace() -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: ShapeSet::ALL,
        hcp: 0..=37,
        cards: vec![CardRequirement::in_suit(
            Suit::Diamonds,
            Holding::top_ranks(1),
            1..=1,
        )],
        eval: Vec::new(),
    })
}

fn holds_diamond_and_heart_aces() -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: ShapeSet::ALL,
        hcp: 0..=37,
        cards: vec![
            CardRequirement::in_suit(Suit::Diamonds, Holding::top_ranks(1), 1..=1),
            CardRequirement::in_suit(Suit::Hearts, Holding::top_ranks(1), 1..=1),
        ],
        eval: Vec::new(),
    })
}

#[derive(Clone, Copy, Debug)]
enum WestVariant {
    Overlapping,
    Disjoint,
    Clipped,
}

/// West's alternatives and their bound `U` (the residual threshold `T` without an acceptance
/// floor).
fn west_alternatives(variant: WestVariant) -> (Vec<(HandConstraint, f32)>, f64) {
    match variant {
        WestVariant::Overlapping => (
            vec![(holds_diamond_ace(), 0.7), (HandConstraint::ANY, 0.3)],
            1.0,
        ),
        WestVariant::Disjoint => (
            vec![(hcp_atom(0..=14), 0.6), (hcp_atom(15..=37), 0.4)],
            f64::from(0.6f32),
        ),
        WestVariant::Clipped => (
            vec![
                (holds_diamond_and_heart_aces(), 0.9),
                (HandConstraint::ANY, 0.1),
            ],
            1.0,
        ),
    }
}

/// South's alternatives: `support`'s two literal-free HCP windows, or a variant whose first
/// window carries a `cards` literal (the heart king), which the middle seat's coarsening drops,
/// plus a light `ANY` alternative, which folding moves into the uniform component.
#[derive(Clone, Copy, Debug)]
enum SouthVariant {
    Plain,
    Coarsened,
}

fn holds_heart_king_with_hcp(hcp: core::ops::RangeInclusive<u8>) -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: ShapeSet::ALL,
        hcp,
        cards: vec![CardRequirement::in_suit(
            Suit::Hearts,
            Holding::EMPTY.with(Rank::King),
            1..=1,
        )],
        eval: Vec::new(),
    })
}

/// [`fixture_with`] with `support`'s South.
fn fixture(variant: WestVariant) -> (MultiComponentContext, f64) {
    fixture_with(variant, SouthVariant::Plain)
}

/// The fixture with West replaced by `variant` and South by `south`, and `per_call` holding one
/// call per constrained seat whose alternatives are that seat's, so `Interpretation::likelihood`
/// (the target when `bidding` is `None`) is the product of the seats' own mixtures.
///
/// Mass ordering with the coarsened South (§6.1 point 3, fine candidates on the full pool): East
/// 8; South `0.6 · 22 + 0.4 · 28 + 0.001 · 84 ≈ 24.5` (22 three-card subsets hold the heart
/// king with 8-10 HCP); clipped West `0.9 · 21 + 0.1 · 126 = 31.5`. East stays cached, South
/// the re-prepared middle seat and West the residual seat, as in `support`. (With `support`'s
/// South, 49.6, the clipped West would sort before South, so the clipped variant is only used
/// with the coarsened South.)
fn fixture_with(variant: WestVariant, south: SouthVariant) -> (MultiComponentContext, f64) {
    let mut fixture = multi_component_rejecting_last_seat();
    let (west, bound) = west_alternatives(variant);
    fixture.interpretation.seats[Seat::West.index() as usize] = west
        .into_iter()
        .map(|(c, w)| (c, w, explanation()))
        .collect();
    if let SouthVariant::Coarsened = south {
        fixture.interpretation.seats[Seat::South.index() as usize] = vec![
            (holds_heart_king_with_hcp(8..=10), 0.6, explanation()),
            (hcp_atom(6..=8), 0.4, explanation()),
            (HandConstraint::ANY, 0.001, explanation()),
        ];
    }
    let mut per_call = Vec::new();
    for seat in [Seat::East, Seat::South, Seat::West] {
        let alternatives = fixture.interpretation.seats[seat.index() as usize]
            .iter()
            .map(|(c, w, _)| {
                (
                    c.clone(),
                    *w,
                    CallExplanation {
                        call_index: per_call.len(),
                        call: Call::Pass,
                        node: None,
                        kind: ResolutionKind::Exact,
                        text: String::new(),
                    },
                )
            })
            .collect();
        per_call.push(CallInterpretation {
            call_index: per_call.len(),
            seat,
            call: Call::Pass,
            kind: ResolutionKind::Exact,
            alternatives,
            log_scale: 0.0,
            shadowed: false,
        });
    }
    fixture.interpretation.per_call = per_call;
    (fixture, bound)
}

/// No play constraints.
const PLAY: [HandConstraint; 4] = [HandConstraint::ANY; 4];

fn context<'a>(
    fixture: &'a MultiComponentContext,
    play: &'a [HandConstraint; 4],
) -> SampleContext<'a> {
    SampleContext {
        known: fixture.known,
        interpretation: &fixture.interpretation,
        play_constraints: play,
        play_soft: None,
        bidding: None,
    }
}

/// Every 2/3/4 split of the nine-card pool between East, South and West (`36 · 35 = 1260`).
fn all_deals(fixture: &MultiComponentContext) -> Vec<Deal> {
    let known = fixture.known;
    let pool = known.pool();
    let pool_cards: Vec<Card> = pool.cards().collect();
    let north = known.known[Seat::North.index() as usize];
    let east_fixed = known.known[Seat::East.index() as usize];
    let south_fixed = known.known[Seat::South.index() as usize];
    let west_fixed = known.known[Seat::West.index() as usize];
    let mut deals = Vec::new();
    for east in subsets_of_size(&pool_cards, 2) {
        let after_east = pool.difference(east);
        let remaining: Vec<Card> = after_east.cards().collect();
        for south in subsets_of_size(&remaining, 3) {
            let west = after_east.difference(south);
            deals.push(
                Deal::new([
                    north,
                    east_fixed.union(east),
                    south_fixed.union(south),
                    west_fixed.union(west),
                ])
                .expect("four disjoint 13-card hands"),
            );
        }
    }
    assert_eq!(deals.len(), 1260);
    deals
}

/// `Σ_{i: hand ∈ C_i} w_i` over `alternatives`.
fn mixture(alternatives: &[(HandConstraint, f32, Explanation)], hand: Hand) -> f64 {
    alternatives
        .iter()
        .filter(|(c, _, _)| c.satisfies(hand))
        .map(|(_, w, _)| f64::from(*w))
        .sum()
}

/// Check 1: `log_prob` with residual rejection = without + `ln(m(h_West) / U)`.
#[test]
fn residual_log_prob_adds_ln_acceptance() {
    for variant in [WestVariant::Overlapping, WestVariant::Disjoint] {
        let (fixture, bound) = fixture(variant);
        let play = PLAY;
        let ctx = context(&fixture, &play);
        let plain = ConstraintProposal {
            residual_rejection: false,
            ..ConstraintProposal::default()
        }
        .prepare(&ctx)
        .expect("prepares");
        // No acceptance floor, so the threshold is the bound itself.
        let residual = ConstraintProposal {
            residual_rejection: true,
            residual_min_acceptance: 0.0,
            ..ConstraintProposal::default()
        }
        .prepare(&ctx)
        .expect("prepares");
        let west = &fixture.interpretation.seats[Seat::West.index() as usize];
        let mut distinct = Vec::new();
        for deal in all_deals(&fixture) {
            let a = plain.log_prob(&deal);
            let b = residual.log_prob(&deal);
            if !a.is_finite() {
                assert_eq!(b, f64::NEG_INFINITY, "{variant:?}: support differs");
                continue;
            }
            let ln_accept = (mixture(west, deal.hand(Seat::West)) / bound).ln();
            assert!(
                (b - (a + ln_accept)).abs() < 1e-9,
                "{variant:?}: {b} != {a} + {ln_accept}"
            );
            if !distinct.iter().any(|&x: &f64| (x - ln_accept).abs() < 1e-9) {
                distinct.push(ln_accept);
            }
        }
        assert_eq!(
            distinct.len(),
            2,
            "{variant:?}: the fixture must make the acceptance vary"
        );
        assert!(
            distinct.iter().any(|&x| x.abs() < 1e-9),
            "some hand has a = 1"
        );
    }
}

/// Check 2: proposals match the normalised `exp(log_prob)` by chi-square.
#[test]
fn residual_proposals_match_log_prob() {
    for (variant, seed) in [
        (WestVariant::Overlapping, 0x0E51_0001u64),
        (WestVariant::Disjoint, 0x0E51_0002u64),
    ] {
        let (fixture, _) = fixture(variant);
        let play = PLAY;
        let ctx = context(&fixture, &play);
        let prepared = ConstraintProposal {
            residual_rejection: true,
            ..ConstraintProposal::default()
        }
        .prepare(&ctx)
        .expect("prepares");
        let deals = all_deals(&fixture);
        let log_probs: Vec<f64> = deals.iter().map(|d| prepared.log_prob(d)).collect();
        let total: f64 = log_probs
            .iter()
            .filter(|lp| lp.is_finite())
            .map(|lp| lp.exp())
            .sum();
        assert!(
            total > 0.0 && total < 1.0,
            "{variant:?}: Σ exp(log_prob) = {total}"
        );

        // As in `log_prob.rs`'s multi-component test, South's two HCP windows are re-prepared on
        // every draw, so `n` is kept small for debug builds; the enumerated probabilities span a
        // small range, so every bin still expects well over 5 draws.
        let n = 4_000u64;
        let mut rng = rng_for(seed, 0);
        let mut observed = vec![0u64; deals.len()];
        let mut produced = 0u64;
        let mut attempts = 0u64;
        while produced < n {
            attempts += 1;
            assert!(attempts < n * 1000, "{variant:?}: acceptance too low");
            let Some(deal) = prepared.propose(&mut rng) else {
                continue;
            };
            produced += 1;
            let i = deals
                .iter()
                .position(|d| d == &deal)
                .expect("proposed deals are enumerated splits");
            observed[i] += 1;
        }
        let mut used_observed = Vec::new();
        let mut used_expected = Vec::new();
        for (i, lp) in log_probs.iter().enumerate() {
            if lp.is_finite() {
                used_observed.push(observed[i]);
                used_expected.push(lp.exp() / total * n as f64);
            } else {
                assert_eq!(
                    observed[i], 0,
                    "{variant:?}: a zero-probability deal was drawn"
                );
            }
        }
        let chi2 = chi_square_statistic(&used_observed, &used_expected);
        let df = (used_expected.len() - 1) as f64;
        let p = chi_square_p_value(chi2, df);
        assert!(
            p > 0.01,
            "{variant:?}: chi-square {chi2} (df {df}), p = {p}"
        );
    }
}

/// Which seat (East, South, West as 0, 1, 2) holds `card`.
fn holder(deal: &Deal, card: Card) -> usize {
    [Seat::East, Seat::South, Seat::West]
        .iter()
        .position(|&s| deal.hand(s).contains(card))
        .expect("the pool cards are held by East, South or West")
}

/// The category of a deal: who holds the heart ace × who holds the diamond ace.
fn category(deal: &Deal) -> usize {
    3 * holder(deal, Card::new(Suit::Hearts, Rank::Ace))
        + holder(deal, Card::new(Suit::Diamonds, Rank::Ace))
}

const CATEGORIES: usize = 9;

/// Solves `a x = b` by Gaussian elimination with partial pivoting (`a` is small and SPD here).
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Vec<f64> {
    let k = b.len();
    for col in 0..k {
        let pivot = (col..k)
            .max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))
            .expect("non-empty");
        a.swap(col, pivot);
        b.swap(col, pivot);
        let d = a[col][col];
        assert!(d.abs() > 1e-15, "singular covariance");
        let pivot_row = a[col].clone();
        for row in col + 1..k {
            let f = a[row][col] / d;
            for (x, p) in a[row].iter_mut().zip(&pivot_row).skip(col) {
                *x -= f * p;
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = vec![0.0; k];
    for row in (0..k).rev() {
        let s: f64 = (row + 1..k).map(|c| a[row][c] * x[c]).sum();
        x[row] = (b[row] - s) / a[row][row];
    }
    x
}

/// The Wald statistic of the self-normalised estimate `p̂` against the exact `p` over the
/// categories with positive `p` (dropping one to remove the sum-to-one degeneracy):
/// `(p̂ − p)ᵀ Σ̂⁻¹ (p̂ − p)` with `Σ̂ = Σ_i w̄_i² (e_i − p)(e_i − p)ᵀ`, the delta-method
/// covariance of a self-normalised importance-sampling estimate. Returns `(statistic, df)`.
fn wald(deals: &[WeightedDeal], exact: &[f64; CATEGORIES]) -> (f64, usize) {
    let weights = WeightedDeal::normalized_weights(deals);
    let used: Vec<usize> = (0..CATEGORIES).filter(|&k| exact[k] > 0.0).collect();
    let used = &used[..used.len() - 1];
    let k = used.len();
    let mut estimate = vec![0.0; k];
    let mut cov = vec![vec![0.0; k]; k];
    for (weighted, &w) in deals.iter().zip(&weights) {
        let cat = category(&weighted.deal);
        let centred: Vec<f64> = used
            .iter()
            .map(|&c| f64::from(u8::from(c == cat)) - exact[c])
            .collect();
        for (a, &ca) in centred.iter().enumerate() {
            estimate[a] += w * ca;
            for (b, &cb) in centred.iter().enumerate() {
                cov[a][b] += w * w * ca * cb;
            }
        }
    }
    let x = solve(cov, estimate.clone());
    let stat = estimate.iter().zip(&x).map(|(a, b)| a * b).sum();
    (stat, k)
}

/// Check 3: weighted posterior estimates match the exact enumeration.
#[test]
fn weighted_estimates_are_unbiased() {
    for (variant, south) in [
        (WestVariant::Overlapping, SouthVariant::Plain),
        (WestVariant::Disjoint, SouthVariant::Plain),
        (WestVariant::Clipped, SouthVariant::Coarsened),
    ] {
        let (fixture, _) = fixture_with(variant, south);
        let play = PLAY;
        let ctx = context(&fixture, &play);

        // The exact posterior: every split weighted by the target likelihood (the uniform prior
        // over splits is flat).
        let mut exact = [0.0f64; CATEGORIES];
        for deal in all_deals(&fixture) {
            let l: f64 = [Seat::East, Seat::South, Seat::West]
                .iter()
                .map(|&s| f64::from(fixture.interpretation.likelihood(s, deal.hand(s))))
                .product();
            exact[category(&deal)] += l;
        }
        let z: f64 = exact.iter().sum();
        for p in &mut exact {
            *p /= z;
        }
        assert!(
            exact.iter().filter(|&&p| p > 0.01).count() >= 4,
            "{variant:?}: the statistic must be informative: {exact:?}"
        );

        for residual_rejection in [false, true] {
            let proposal = ConstraintProposal {
                residual_rejection,
                ..ConstraintProposal::default()
            };
            let opts = SampleOptions {
                seed: 0x0B1A5,
                threads: Threads::Auto,
                ..SampleOptions::default()
            };
            let n = 6_000;
            let (deals, report) = sample_deals(&ctx, &proposal, n, &opts).expect("samples");
            assert_eq!(report.produced, n, "{variant:?}: {report:?}");
            assert!(!report.budget_exhausted);
            let (stat, df) = wald(&deals, &exact);
            let p = chi_square_p_value(stat, df as f64);
            assert!(
                p > 0.01,
                "{variant:?}, residual {residual_rejection}: Wald {stat} (df {df}), p = {p}, \
                 ESS {:.0}",
                report.ess
            );
        }
    }
}

/// Residual rejection's pilot (128 proposals drawn in `prepare` to fix `T`) is reported as
/// `pilot_attempts` and charged to `ess_per_attempt`; a proposal without a pilot reports 0.
#[test]
fn residual_pilot_is_charged_to_ess_per_attempt() {
    let (fixture, _) = fixture(WestVariant::Overlapping);
    let play = PLAY;
    let ctx = context(&fixture, &play);
    let opts = SampleOptions {
        seed: 0x9170,
        threads: Threads::Single,
        ..SampleOptions::default()
    };
    for (residual_rejection, pilot) in [(false, 0u64), (true, 128)] {
        let proposal = ConstraintProposal {
            residual_rejection,
            ..ConstraintProposal::default()
        };
        let (_, report) = sample_deals(&ctx, &proposal, 200, &opts).expect("samples");
        assert_eq!(
            report.pilot_attempts, pilot,
            "residual {residual_rejection}"
        );
        let expected = report.ess / (report.attempts + pilot) as f64;
        assert!(
            (report.ess_per_attempt - expected).abs() < 1e-12,
            "residual {residual_rejection}: {} != {expected}",
            report.ess_per_attempt
        );
        assert!(
            (report.acceptance_rate - report.produced as f64 / report.attempts as f64).abs()
                < 1e-12,
            "the pilot is not part of the acceptance rate"
        );
    }
}

/// A chi-square test of `observed` against `expected` with every bin expecting fewer than 5
/// draws pooled into one (the usual validity rule; the coarsened South's light `ANY` component
/// puts a tiny probability on many deals). Returns the p-value.
fn pooled_chi_square_p(observed: &[u64], expected: &[f64]) -> f64 {
    let mut used_observed = Vec::new();
    let mut used_expected = Vec::new();
    let (mut pooled_observed, mut pooled_expected) = (0u64, 0.0f64);
    for (&o, &e) in observed.iter().zip(expected) {
        if e >= 5.0 {
            used_observed.push(o);
            used_expected.push(e);
        } else {
            pooled_observed += o;
            pooled_expected += e;
        }
    }
    if pooled_expected > 0.0 {
        used_observed.push(pooled_observed);
        used_expected.push(pooled_expected);
    }
    assert!(
        used_expected.len() > 2,
        "too few bins for a chi-square test"
    );
    let chi2 = chi_square_statistic(&used_observed, &used_expected);
    chi_square_p_value(chi2, (used_expected.len() - 1) as f64)
}

/// Checks 1 and 2 on the path most real cases take: the pilot's acceptance at the bound `U` is
/// below the default floor, so `T` is bisected below the largest mixture value and `a(h) =
/// min(1, m(h) / T)` is clipped to 1 for the heavy hands; South, the re-prepared middle seat, is
/// coarsened (its `cards` literal dropped) and has a light alternative folded.
#[test]
fn clipped_threshold_with_coarsened_middle_seat_is_exact() {
    let (fixture, _) = fixture_with(WestVariant::Clipped, SouthVariant::Coarsened);
    let play = PLAY;
    let ctx = context(&fixture, &play);
    let plain = ConstraintProposal {
        residual_rejection: false,
        ..ConstraintProposal::default()
    }
    .prepare(&ctx)
    .expect("prepares");
    let floor = ConstraintProposal::default().residual_min_acceptance;
    let residual = ConstraintProposal {
        residual_rejection: true,
        ..ConstraintProposal::default()
    }
    .prepare(&ctx)
    .expect("prepares");
    let deals = all_deals(&fixture);
    let west = &fixture.interpretation.seats[Seat::West.index() as usize];
    let plain_log_probs: Vec<f64> = deals.iter().map(|d| plain.log_prob(d)).collect();
    let log_probs: Vec<f64> = deals.iter().map(|d| residual.log_prob(d)).collect();

    // South is coarsened: its density ignores the heart-king literal. Every 9-10 HCP South hand
    // is covered only by the coarse 8-10 window and the folded `ANY`, so with East's hand fixed
    // (and West, the residual seat, contributing nothing without rejection) the density is the
    // same whether South holds the heart king or another heart honour.
    let heart = |r: Rank| Card::new(Suit::Hearts, r);
    let south_drawn = |d: &Deal| {
        d.hand(Seat::South)
            .difference(fixture.known.known[Seat::South.index() as usize])
    };
    let hcp_of = |h: Hand| -> u32 {
        h.cards()
            .map(|c| match c.rank() {
                Rank::Ace => 4,
                Rank::King => 3,
                Rank::Queen => 2,
                _ => 0,
            })
            .sum()
    };
    let mut literal_pairs = 0;
    for (i, a) in deals.iter().enumerate() {
        let sa = south_drawn(a);
        if !(9..=10).contains(&hcp_of(sa)) || !sa.contains(heart(Rank::King)) {
            continue;
        }
        for (j, b) in deals.iter().enumerate() {
            let sb = south_drawn(b);
            if a.hand(Seat::East) != b.hand(Seat::East)
                || !(9..=10).contains(&hcp_of(sb))
                || sb.contains(heart(Rank::King))
                || !(sb.contains(heart(Rank::Ace)) || sb.contains(heart(Rank::Queen)))
                || !plain_log_probs[i].is_finite()
            {
                continue;
            }
            assert!(
                (plain_log_probs[i] - plain_log_probs[j]).abs() < 1e-9,
                "the coarsened South density must not see the heart-king literal"
            );
            literal_pairs += 1;
        }
    }
    assert!(literal_pairs > 0, "the fixture must compare such hands");
    // The light `ANY` alternative is folded, not dropped: South hands outside both windows
    // (11-12 HCP) stay in the support.
    assert!(
        deals
            .iter()
            .zip(&plain_log_probs)
            .any(|(d, lp)| hcp_of(south_drawn(d)) >= 11 && lp.is_finite()),
        "the folded ANY component must cover South hands outside both windows"
    );
    // And it is folded (a fixed draw probability) rather than kept as an adaptive component:
    // turning folding off changes the density.
    let unfolded = ConstraintProposal {
        residual_rejection: false,
        light_threshold: 0.0,
        ..ConstraintProposal::default()
    }
    .prepare(&ctx)
    .expect("prepares");
    assert!(
        deals
            .iter()
            .zip(&plain_log_probs)
            .any(|(d, lp)| lp.is_finite() && (unfolded.log_prob(d) - lp).abs() > 1e-6),
        "South's light ANY alternative must be folded at the default light_threshold"
    );

    // Check 1: residual log_prob = plain + ln min(1, m / T). The heavy hands (both aces, m = 1)
    // are clipped (a = 1, so T < 1); the rest carry one constant ln(0.1 / T) that fixes T.
    let mut threshold: Option<f64> = None;
    let mut clipped = 0;
    for ((deal, &a), &b) in deals.iter().zip(&plain_log_probs).zip(&log_probs) {
        if !a.is_finite() {
            assert_eq!(b, f64::NEG_INFINITY, "support differs");
            continue;
        }
        let m = mixture(west, deal.hand(Seat::West));
        if (m - 1.0).abs() < 1e-6 {
            assert!(
                (b - a).abs() < 1e-9,
                "a heavy hand must be clipped to a = 1"
            );
            clipped += 1;
        } else {
            let t = m / (b - a).exp();
            match threshold {
                None => threshold = Some(t),
                Some(t0) => assert!((t - t0).abs() < 1e-9 * t0, "T is one constant: {t} != {t0}"),
            }
        }
    }
    let threshold = threshold.expect("some light West hands");
    assert!(clipped > 0, "some deals must be clipped");
    assert!(
        threshold > 0.1 + 1e-9 && threshold < 1.0 - 1e-9,
        "T = {threshold} must be bisected strictly between min m = 0.1 and max m = U = 1"
    );
    // The fixture really needs the bisection: the exact mean acceptance at `U` under the plain
    // proposal is below the floor, and at the bisected `T` it is near it (the pilot is 128 draws).
    let plain_total: f64 = plain_log_probs
        .iter()
        .filter(|lp| lp.is_finite())
        .map(|lp| lp.exp())
        .sum();
    let acceptance_at = |t: f64| -> f64 {
        deals
            .iter()
            .zip(&plain_log_probs)
            .filter(|(_, lp)| lp.is_finite())
            .map(|(d, lp)| lp.exp() * (mixture(west, d.hand(Seat::West)) / t).min(1.0))
            .sum::<f64>()
            / plain_total
    };
    assert!(
        acceptance_at(1.0) < floor - 0.05,
        "acceptance at U = {} must be below the floor {floor}",
        acceptance_at(1.0)
    );
    let at_threshold = acceptance_at(threshold);
    println!(
        "T = {threshold:.4}; exact acceptance at U {:.4}, at T {at_threshold:.4}",
        acceptance_at(1.0)
    );
    assert!(
        (at_threshold - floor).abs() < 0.15,
        "acceptance at T = {at_threshold}, expected about {floor}"
    );

    // Check 2: proposals (retried on rejection) match the normalised exp(log_prob).
    let total: f64 = log_probs
        .iter()
        .filter(|lp| lp.is_finite())
        .map(|lp| lp.exp())
        .sum();
    assert!(total > 0.0 && total < 1.0, "Σ exp(log_prob) = {total}");
    let n = 4_000u64;
    let mut rng = rng_for(0x0E51_0003, 0);
    let mut observed = vec![0u64; deals.len()];
    let mut produced = 0u64;
    let mut attempts = 0u64;
    while produced < n {
        attempts += 1;
        assert!(attempts < n * 1000, "acceptance too low");
        let Some(deal) = residual.propose(&mut rng) else {
            continue;
        };
        produced += 1;
        let i = deals
            .iter()
            .position(|d| d == &deal)
            .expect("proposed deals are enumerated splits");
        observed[i] += 1;
    }
    let mut used_observed = Vec::new();
    let mut used_expected = Vec::new();
    for (i, lp) in log_probs.iter().enumerate() {
        if lp.is_finite() {
            used_observed.push(observed[i]);
            used_expected.push(lp.exp() / total * n as f64);
        } else {
            assert_eq!(observed[i], 0, "a zero-probability deal was drawn");
        }
    }
    let p = pooled_chi_square_p(&used_observed, &used_expected);
    assert!(p > 0.01, "chi-square p = {p}");
}
