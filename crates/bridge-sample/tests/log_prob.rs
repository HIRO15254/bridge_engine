//! `ConstraintProposal::log_prob` must be exact against its own `propose` (§6.3 of
//! `09-sample.md`): a small pool (8 unknown cards) with overlapping alternatives (both can hold
//! the same hand) and an overlapping DNF term inside one of them (an `Or` whose branches are not
//! disjoint) is fully enumerated (`C(8, 4) = 70` splits), and `10^5` proposals are checked
//! against `exp(log_prob)` by chi-square.

use bridge_bidding::{Explanation, Interpretation, ResolutionKind};
use bridge_constraint::{Atom, CardRequirement, HandConstraint, KnownCards, ShapeSet};
use bridge_core::{Card, Hand, Holding, Seat, Suit};
use bridge_sample::{ConstraintProposal, Proposal, SampleContext, rng_for};

mod support;
use support::{
    chi_square_p_value, chi_square_statistic, multi_component_rejecting_last_seat, subsets_of_size,
};

fn cards_to_hand(cards: &[Card]) -> Hand {
    cards.iter().fold(Hand::EMPTY, |h, &c| h.with(c))
}

fn atom(cards: Vec<CardRequirement>) -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: ShapeSet::ALL,
        hcp: 0..=37,
        cards,
        eval: Vec::new(),
    })
}

fn empty_explanation() -> Explanation {
    Explanation {
        text: String::new(),
        node: None,
        resolution: ResolutionKind::Exact,
        parts: Vec::new(),
    }
}

/// North holds all clubs, East all diamonds (both fully known, `needed == 0`); the 18 remaining
/// hearts-and-low-spades are split 9/9 between South and West's fixed cards; the top 8 spades
/// (`A K Q J T 9 8 7`) are the unknown pool, split 4/4 between South and West.
fn small_pool_context() -> (KnownCards, Vec<Card>) {
    let clubs = Hand::EMPTY.with_holding(Suit::Clubs, Holding::FULL);
    let diamonds = Hand::EMPTY.with_holding(Suit::Diamonds, Holding::FULL);
    let spades_full = Hand::EMPTY.with_holding(Suit::Spades, Holding::FULL);
    let spades_pool_hand = Hand::EMPTY.with_holding(Suit::Spades, Holding::top_ranks(8));
    let spades_low = spades_full.difference(spades_pool_hand);
    let hearts = Hand::EMPTY.with_holding(Suit::Hearts, Holding::FULL);

    let fixed_pool: Vec<Card> = hearts.union(spades_low).cards().collect();
    assert_eq!(fixed_pool.len(), 18);
    let south_fixed = cards_to_hand(&fixed_pool[0..9]);
    let west_fixed = cards_to_hand(&fixed_pool[9..18]);

    let known = KnownCards::new([clubs, diamonds, south_fixed, west_fixed])
        .expect("the four hands are pairwise disjoint by construction");
    assert_eq!(known.pool(), spades_pool_hand);
    assert_eq!(known.needed(Seat::North), 0);
    assert_eq!(known.needed(Seat::East), 0);
    assert_eq!(known.needed(Seat::South), 4);
    assert_eq!(known.needed(Seat::West), 4);

    let pool: Vec<Card> = spades_pool_hand.cards().collect();
    (known, pool)
}

/// South's two alternatives overlap (a hand can hold both the spade ace and the spade king), and
/// the first alternative is itself an `Or` of two overlapping card requirements (holding the ace
/// or holding the king, which are not disjoint from the outer alternative split either): both
/// `Sampler::log_prob`'s own term-level mixture and `ConstraintProposal`'s alternative-level
/// mixture (§6.3) are exercised at once. West has no calls (`ANY`, direct dealing, §6.4 (a)).
fn overlapping_interpretation() -> Interpretation {
    let ace = Holding::top_ranks(1);
    let king = Holding::top_ranks(2).without(ace.highest().expect("top_ranks(1) is non-empty"));
    let queen = Holding::top_ranks(3)
        .without(ace.highest().expect("non-empty"))
        .without(king.highest().expect("non-empty"));

    let has_ace = CardRequirement::in_suit(Suit::Spades, ace, 1..=1);
    let has_king = CardRequirement::in_suit(Suit::Spades, king, 1..=1);
    let has_queen = CardRequirement::in_suit(Suit::Spades, queen, 1..=1);

    // Alternative 1: "ace or queen" (an `Or`, so its two branches are separate DNF terms; a hand
    // with both the ace and the queen is counted once by the sampler but satisfies either
    // branch, exercising the term-overlap path).
    let ace_or_queen = HandConstraint::Or(vec![atom(vec![has_ace]), atom(vec![has_queen])]);
    // Alternative 2: "king" alone. Overlaps alternative 1 whenever a hand holds both the ace and
    // the king (or the queen and the king).
    let has_king_only = atom(vec![has_king]);

    let south = vec![
        (ace_or_queen, 0.6, empty_explanation()),
        (has_king_only, 0.4, empty_explanation()),
    ];

    Interpretation {
        seats: [Vec::new(), Vec::new(), south, Vec::new()],
        per_call: Vec::new(),
        divergence: None,
    }
}

/// North fully known; East and South each need 3 cards from a 9-card spade-top pool and carry a
/// `cards`-only alternative (`shapes = ALL`, `hcp = 0..=37`): East must hold the pool's ace,
/// South the king. West is left completely unconstrained (direct-dealt, §6.4 (a)), so `propose`
/// never rejects at the last seat — the point of this context is to isolate the "middle seat"
/// itself (§6.4 (c)), not add a second source of rejection on top of it. East's mass (`1.0 ×
/// C(8, 2)` at the full pool) is smaller than South's or West's direct-deal mass (`C(9, 3)`), so
/// East is cached (k=0) and South — genuinely re-prepared every draw, the seat §6.4 (c) actually
/// applies to — sits at k=1, neither first nor last.
fn three_seat_pool_context() -> (KnownCards, Vec<Card>) {
    let clubs = Hand::EMPTY.with_holding(Suit::Clubs, Holding::FULL);
    let diamonds = Hand::EMPTY.with_holding(Suit::Diamonds, Holding::FULL);
    let hearts = Hand::EMPTY.with_holding(Suit::Hearts, Holding::FULL);
    let spades_full = Hand::EMPTY.with_holding(Suit::Spades, Holding::FULL);
    let spades_pool_hand = Hand::EMPTY.with_holding(Suit::Spades, Holding::top_ranks(9));
    let spades_low = spades_full.difference(spades_pool_hand);

    let north = clubs;
    let fixed_pool: Vec<Card> = diamonds.union(hearts).union(spades_low).cards().collect();
    assert_eq!(fixed_pool.len(), 30);
    let east_fixed = cards_to_hand(&fixed_pool[0..10]);
    let south_fixed = cards_to_hand(&fixed_pool[10..20]);
    let west_fixed = cards_to_hand(&fixed_pool[20..30]);

    let known = KnownCards::new([north, east_fixed, south_fixed, west_fixed])
        .expect("the four hands are pairwise disjoint by construction");
    assert_eq!(known.pool(), spades_pool_hand);
    assert_eq!(known.needed(Seat::North), 0);
    assert_eq!(known.needed(Seat::East), 3);
    assert_eq!(known.needed(Seat::South), 3);
    assert_eq!(known.needed(Seat::West), 3);

    let pool: Vec<Card> = spades_pool_hand.cards().collect();
    (known, pool)
}

fn three_seat_interpretation() -> Interpretation {
    let ace = Holding::top_ranks(1);
    let king = Holding::top_ranks(2).without(ace.highest().expect("non-empty"));

    let holds_ace = atom(vec![CardRequirement::in_suit(Suit::Spades, ace, 1..=1)]);
    let holds_king = atom(vec![CardRequirement::in_suit(Suit::Spades, king, 1..=1)]);

    Interpretation {
        seats: [
            Vec::new(),
            vec![(holds_ace, 1.0, empty_explanation())],
            vec![(holds_king, 1.0, empty_explanation())],
            Vec::new(),
        ],
        per_call: Vec::new(),
        divergence: None,
    }
}

/// Every way to split the 9-card pool 3/3/3 between East, South and West (`C(9,3) · C(6,3) =
/// 1680`); the raw combinatorial space, not filtered by any seat's own constraint (log_prob
/// itself assigns `-∞` wherever East doesn't hold the ace — the middle seat, South, has no gate
/// at all once its sampler is coarsened to `ANY`, §6.4 (c), and West is unconstrained so it never
/// rejects, so every one of the 1680 splits is in fact in the support).
fn enumerate_three_way_splits(pool: Hand, pool_cards: &[Card]) -> Vec<(Hand, Hand, Hand)> {
    let mut out = Vec::new();
    for east in subsets_of_size(pool_cards, 3) {
        let after_east = pool.difference(east);
        let remaining: Vec<Card> = after_east.cards().collect();
        for south in subsets_of_size(&remaining, 3) {
            let west = after_east.difference(south);
            out.push((east, south, west));
        }
    }
    out
}

/// §6.4 (c)'s middle-seat coarsening must not break `log_prob`'s exactness against `propose`:
/// with three genuinely `Sampled` seats (East cached, South re-prepared every draw — the actual
/// "middle" position — West residual), the enumerated support must still sum to 1 and 10^5
/// proposals must still match `exp(log_prob)` by chi-square, the same two checks as
/// `log_prob_consistency` above.
#[test]
fn middle_seat_coarse_log_prob_consistency() {
    for proposal in with_and_without_residual(ConstraintProposal::default()) {
        check_three_seat_consistency(&three_seat_interpretation(), 20260926, &proposal);
    }
}

/// `base` without and with residual rejection (`09-sample.md` §6.5, off by default), so each
/// exactness check covers both densities.
fn with_and_without_residual(base: ConstraintProposal) -> [ConstraintProposal; 2] {
    [
        ConstraintProposal {
            residual_rejection: false,
            ..base.clone()
        },
        ConstraintProposal {
            residual_rejection: true,
            ..base
        },
    ]
}

/// High-card points of `hand` (A = 4, K = 3, Q = 2, J = 1).
fn hcp(hand: Hand) -> u8 {
    hand.cards()
        .map(|c| match c.rank() {
            bridge_core::Rank::Ace => 4,
            bridge_core::Rank::King => 3,
            bridge_core::Rank::Queen => 2,
            bridge_core::Rank::Jack => 1,
            _ => 0,
        })
        .sum()
}

/// Like [`three_seat_interpretation`], but South (the re-prepared middle seat) has three
/// alternatives, two of which differ only in a `cards` literal and so coarsen to the *same*
/// shape + HCP summary (§6.4 (c)); `ConstraintProposal` merges those into one component. The
/// third has a different HCP window. The merged mixture must stay exact against `propose`.
fn three_seat_merged_summary_interpretation(south_fixed: Hand) -> Interpretation {
    let ace = Holding::top_ranks(1);
    let king = Holding::top_ranks(2).without(ace.highest().expect("non-empty"));
    let queen = Holding::top_ranks(3)
        .without(ace.highest().expect("non-empty"))
        .without(king.highest().expect("non-empty"));
    let h0 = hcp(south_fixed);
    let with_hcp = |range: core::ops::RangeInclusive<u8>, cards: Vec<CardRequirement>| {
        HandConstraint::Atom(Atom {
            shapes: ShapeSet::ALL,
            hcp: range,
            cards,
            eval: Vec::new(),
        })
    };

    let holds_ace = atom(vec![CardRequirement::in_suit(Suit::Spades, ace, 1..=1)]);
    let south = vec![
        (
            with_hcp(
                h0 + 3..=h0 + 7,
                vec![CardRequirement::in_suit(Suit::Spades, king, 1..=1)],
            ),
            0.5,
            empty_explanation(),
        ),
        (
            with_hcp(
                h0 + 3..=h0 + 7,
                vec![CardRequirement::in_suit(Suit::Spades, queen, 1..=1)],
            ),
            0.3,
            empty_explanation(),
        ),
        (with_hcp(h0..=h0 + 2, Vec::new()), 0.2, empty_explanation()),
    ];

    Interpretation {
        seats: [
            Vec::new(),
            vec![(holds_ace, 1.0, empty_explanation())],
            south,
            Vec::new(),
        ],
        per_call: Vec::new(),
        divergence: None,
    }
}

/// The merged-summary fast path (identical coarse summaries share one component) must keep
/// `log_prob` exact: same enumeration + chi-square checks as the test above.
#[test]
fn middle_seat_merged_summaries_log_prob_consistency() {
    let (known, _) = three_seat_pool_context();
    let south_fixed = known.known[Seat::South.index() as usize];
    for proposal in with_and_without_residual(ConstraintProposal::default()) {
        check_three_seat_consistency(
            &three_seat_merged_summary_interpretation(south_fixed),
            20260927,
            &proposal,
        );
    }
}

/// §6.4 (d)'s light folding (light alternatives replaced by one uniform component with a fixed
/// draw probability) must keep `log_prob` exact too. An infinite threshold folds every
/// alternative but the heaviest: the merged summary is kept and the low-HCP one is folded.
#[test]
fn middle_seat_light_tier_log_prob_consistency() {
    let (known, _) = three_seat_pool_context();
    let south_fixed = known.known[Seat::South.index() as usize];
    for proposal in with_and_without_residual(ConstraintProposal {
        light_threshold: f64::INFINITY,
        ..ConstraintProposal::default()
    }) {
        check_three_seat_consistency(
            &three_seat_merged_summary_interpretation(south_fixed),
            20260928,
            &proposal,
        );
    }
}

/// Enumerates every 3/3/3 split of [`three_seat_pool_context`]'s pool, sums `exp(log_prob)` over
/// them (the probability a proposal succeeds: 1 unless a light-tier draw can fail), checks that
/// 10^5 proposals fail at that rate, and checks the successful ones against `exp(log_prob)` by
/// chi-square.
fn check_three_seat_consistency(
    interpretation: &Interpretation,
    seed: u64,
    proposal: &ConstraintProposal,
) {
    let (known, pool) = three_seat_pool_context();
    let play_constraints = [
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
    ];
    let ctx = SampleContext {
        known,
        interpretation,
        play_constraints: &play_constraints,
        play_soft: None,
        bidding: None,
    };

    let prepared = proposal
        .prepare(&ctx)
        .expect("prepare succeeds: every seat has support");

    let north = known.known[Seat::North.index() as usize];
    let east_fixed = known.known[Seat::East.index() as usize];
    let south_fixed = known.known[Seat::South.index() as usize];
    let west_fixed = known.known[Seat::West.index() as usize];

    let splits = enumerate_three_way_splits(known.pool(), &pool);
    assert_eq!(splits.len(), 1680);
    let deals: Vec<bridge_core::Deal> = splits
        .iter()
        .map(|&(e, s, w)| {
            bridge_core::Deal::new([
                north,
                east_fixed.union(e),
                south_fixed.union(s),
                west_fixed.union(w),
            ])
            .expect("four disjoint 13-card hands covering the deck")
        })
        .collect();

    let log_probs: Vec<f64> = deals.iter().map(|d| prepared.log_prob(d)).collect();
    let total: f64 = log_probs.iter().map(|lp| lp.exp()).sum();
    assert!(
        total > 0.0 && total < 1.0 + 1e-9,
        "Σ exp(log_prob) over the enumerated support = {total}, expected a probability"
    );

    let attempts = 100_000u64;
    let mut rng = rng_for(seed, 0);
    let mut observed = vec![0u64; deals.len()];
    let mut unmatched = 0u64;
    let mut failed = 0u64;
    for _ in 0..attempts {
        let Some(deal) = prepared.propose(&mut rng) else {
            failed += 1;
            continue;
        };
        match deals.iter().position(|d| d == &deal) {
            Some(i) => observed[i] += 1,
            None => unmatched += 1,
        }
    }
    assert_eq!(
        unmatched, 0,
        "every proposed deal must be one of the 1680 enumerated splits"
    );

    let fail_rate = (1.0 - total).max(0.0);
    let sigma = (attempts as f64 * fail_rate * (1.0 - fail_rate)).sqrt();
    assert!(
        (failed as f64 - attempts as f64 * fail_rate).abs() <= 5.0 * sigma + 1e-6,
        "{failed} of {attempts} proposals failed, expected about {}",
        attempts as f64 * fail_rate
    );
    let n = attempts - failed;

    let expected: Vec<f64> = log_probs
        .iter()
        .map(|lp| lp.exp() / total * n as f64)
        .collect();
    let mut used_observed = Vec::new();
    let mut used_expected = Vec::new();
    for (i, &e) in expected.iter().enumerate() {
        if e > 0.0 {
            used_observed.push(observed[i]);
            used_expected.push(e);
        } else {
            assert_eq!(observed[i], 0, "a zero-probability deal was proposed");
        }
    }
    assert!(
        used_expected.len() > 1,
        "the support must have more than one deal"
    );

    let chi2 = chi_square_statistic(&used_observed, &used_expected);
    let df = (used_expected.len() - 1) as f64;
    let p_value = chi_square_p_value(chi2, df);
    assert!(
        p_value > 0.01,
        "chi-square = {chi2} (df = {df}) rejects at the 0.01 level (p = {p_value})"
    );
}

#[test]
fn log_prob_consistency() {
    let (known, pool) = small_pool_context();
    let interpretation = overlapping_interpretation();
    let play_constraints = [
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
    ];
    let ctx = SampleContext {
        known,
        interpretation: &interpretation,
        play_constraints: &play_constraints,
        play_soft: None,
        bidding: None,
    };

    let proposal = ConstraintProposal::default();
    let prepared = proposal
        .prepare(&ctx)
        .expect("prepare succeeds: every seat has support");

    // Enumerate all 70 ways to split the 8-card pool 4/4 between South and West.
    let south_hands = subsets_of_size(&pool, 4);
    assert_eq!(south_hands.len(), 70);
    let south_fixed = known.known[Seat::South.index() as usize];
    let west_fixed = known.known[Seat::West.index() as usize];
    let north = known.known[Seat::North.index() as usize];
    let east = known.known[Seat::East.index() as usize];

    let deals: Vec<bridge_core::Deal> = south_hands
        .iter()
        .map(|&south_drawn| {
            let south = south_fixed.union(south_drawn);
            let west = west_fixed.union(known.pool().difference(south_drawn));
            bridge_core::Deal::new([north, east, south, west])
                .expect("four disjoint 13-card hands covering the deck")
        })
        .collect();

    let log_probs: Vec<f64> = deals.iter().map(|d| prepared.log_prob(d)).collect();

    // The enumerated support sums to 1 (within the tolerance `09-sample.md` §9 asks for).
    let total: f64 = log_probs.iter().map(|lp| lp.exp()).sum();
    assert!(
        (total - 1.0).abs() < 1e-9,
        "Σ exp(log_prob) over the enumerated support = {total}, expected 1"
    );

    // Draw 10^5 proposals from a single deterministic stream and histogram them by which of the
    // 70 enumerated deals they match.
    let n = 100_000u64;
    let mut rng = rng_for(20260925, 0);
    let mut observed = vec![0u64; deals.len()];
    let mut unmatched = 0u64;
    for _ in 0..n {
        let deal = prepared
            .propose(&mut rng)
            .expect("this context always has support");
        match deals.iter().position(|d| d == &deal) {
            Some(i) => observed[i] += 1,
            None => unmatched += 1,
        }
    }
    assert_eq!(
        unmatched, 0,
        "every proposed deal must be one of the 70 enumerated ones"
    );

    // Chi-square over the bins with positive expected mass (an exact sampler should never place
    // any draws in a zero-probability bin, so every observed bin has one).
    let expected: Vec<f64> = log_probs.iter().map(|lp| lp.exp() * n as f64).collect();
    let mut used_observed = Vec::new();
    let mut used_expected = Vec::new();
    for (i, &e) in expected.iter().enumerate() {
        if e > 0.0 {
            used_observed.push(observed[i]);
            used_expected.push(e);
        } else {
            assert_eq!(observed[i], 0, "a zero-probability deal was proposed");
        }
    }
    assert!(
        used_expected.len() > 1,
        "the support must have more than one deal"
    );

    let chi2 = chi_square_statistic(&used_observed, &used_expected);
    let df = (used_expected.len() - 1) as f64;
    let p_value = chi_square_p_value(chi2, df);
    assert!(
        p_value > 0.01,
        "chi-square = {chi2} (df = {df}) rejects at the 0.01 level (p = {p_value})"
    );
}

/// Covers two gaps `middle_seat_coarse_log_prob_consistency` leaves open (see `support`'s doc
/// comment): a re-prepared middle seat (South) with a genuine two-component mixture that
/// survives [`coarsen`](bridge_sample) instead of trivialising to `ANY`, and a last seat (West)
/// that is `Sampled` and does fail its own `satisfies` check for some residual pools, so
/// `log_prob`'s last-seat rejection branch is actually exercised (both prior tests kept the last
/// seat unconstrained specifically to avoid this).
///
/// Because the last seat can fail, the enumerated support no longer sums to 1 — only to the
/// single-shot acceptance probability `α < 1` — so this test checks `Σ exp(log_prob) ≤ 1` (not
/// `≈ 1`), and compares `propose` (retried on rejection) against `exp(log_prob) / α`.
#[test]
fn middle_seat_multi_component_and_last_seat_rejection_log_prob_consistency() {
    let fixture = multi_component_rejecting_last_seat();
    let play_constraints = [
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
    ];
    let ctx = SampleContext {
        known: fixture.known,
        interpretation: &fixture.interpretation,
        play_constraints: &play_constraints,
        play_soft: None,
        bidding: None,
    };

    let proposal = ConstraintProposal::default();
    let prepared = proposal
        .prepare(&ctx)
        .expect("prepare succeeds: every seat has support");

    let north = fixture.known.known[Seat::North.index() as usize];
    let east_fixed = fixture.known.known[Seat::East.index() as usize];
    let south_fixed = fixture.known.known[Seat::South.index() as usize];
    let west_fixed = fixture.known.known[Seat::West.index() as usize];
    let pool = fixture.known.pool();
    let pool_cards: Vec<Card> = pool.cards().collect();
    assert_eq!(pool_cards.len(), 9);

    // Every way to split the 9-card pool 2/3/4 between East, South and West.
    let mut deals = Vec::new();
    for east_drawn in subsets_of_size(&pool_cards, 2) {
        let after_east = pool.difference(east_drawn);
        let remaining: Vec<Card> = after_east.cards().collect();
        for south_drawn in subsets_of_size(&remaining, 3) {
            let west_drawn = after_east.difference(south_drawn);
            let deal = bridge_core::Deal::new([
                north,
                east_fixed.union(east_drawn),
                south_fixed.union(south_drawn),
                west_fixed.union(west_drawn),
            ])
            .expect("four disjoint 13-card hands covering the deck");
            deals.push(deal);
        }
    }
    assert_eq!(deals.len(), 36 * 35);

    let log_probs: Vec<f64> = deals.iter().map(|d| prepared.log_prob(d)).collect();
    let total: f64 = log_probs
        .iter()
        .map(|lp| if lp.is_finite() { lp.exp() } else { 0.0 })
        .sum();
    assert!(total <= 1.0 + 1e-9, "Σ exp(log_prob) = {total} exceeds 1");
    assert!(total > 0.0, "expected some deals in the support");
    assert!(
        total < 1.0 - 1e-6,
        "expected real rejection at the last seat (West must hold the diamond ace), but Σ \
         exp(log_prob) = {total} is essentially the full mass"
    );

    // Isolate the last-seat rejection path: among deals where East and South's own alternatives
    // are satisfied (so any -inf can only come from West's last-seat check), West holding the
    // diamond ace must give a finite log_prob and West missing it must give exactly -inf.
    let south_alts = &fixture.interpretation.seats[Seat::South.index() as usize];
    let mut saw_last_seat_rejection = false;
    let mut saw_last_seat_acceptance = false;
    for deal in &deals {
        let east_ok = deal.hand(Seat::East).contains(fixture.spade_ace);
        let south_ok = south_alts
            .iter()
            .any(|(c, _, _)| c.satisfies(deal.hand(Seat::South)));
        if !east_ok || !south_ok {
            continue;
        }
        let west_ok = deal.hand(Seat::West).contains(fixture.diamond_ace);
        let lp = prepared.log_prob(deal);
        if west_ok {
            assert!(
                lp.is_finite(),
                "expected finite log_prob when all three seats are satisfied, got {lp}"
            );
            saw_last_seat_acceptance = true;
        } else {
            assert_eq!(
                lp,
                f64::NEG_INFINITY,
                "expected -inf when only the last seat (West) fails its own constraint"
            );
            saw_last_seat_rejection = true;
        }
    }
    assert!(
        saw_last_seat_rejection,
        "no enumerated deal exercised the last-seat rejection path"
    );
    assert!(
        saw_last_seat_acceptance,
        "no enumerated deal exercised the last-seat acceptance path"
    );

    // Proposals, retrying on rejection (`propose` returning `None`), histogrammed against the
    // *conditional* density `exp(log_prob) / total`. Unlike the other two tests in this file,
    // South's mixture here is genuinely two HCP-window components rather than a candidate that
    // coarsens to `ANY` — `Sampler::prepare` does real shape/HCP work for both on every draw, at
    // roughly two orders of magnitude the per-draw cost of the `ANY` fast path in an unoptimized
    // build (`cargo test`, no `--release`). `n` is scaled down accordingly (the enumerated
    // probabilities span less than a 3x range — checked separately — so `n = 3_000` still keeps
    // every used bin's expected count comfortably above the usual chi-square rule of thumb of 5).
    let n = 3_000u64;
    let mut rng = rng_for(20260927, 0);
    let mut observed = vec![0u64; deals.len()];
    let mut unmatched = 0u64;
    let mut produced = 0u64;
    let mut attempts = 0u64;
    let max_attempts = n * 1000;
    while produced < n {
        attempts += 1;
        assert!(
            attempts <= max_attempts,
            "acceptance rate too low: {produced} of {n} accepted in {attempts} attempts"
        );
        let Some(deal) = prepared.propose(&mut rng) else {
            continue;
        };
        produced += 1;
        match deals.iter().position(|d| d == &deal) {
            Some(i) => observed[i] += 1,
            None => unmatched += 1,
        }
    }
    assert_eq!(
        unmatched, 0,
        "every proposed deal must be one of the enumerated splits"
    );

    let expected: Vec<f64> = log_probs
        .iter()
        .map(|lp| {
            if lp.is_finite() {
                lp.exp() / total * n as f64
            } else {
                0.0
            }
        })
        .collect();
    let mut used_observed = Vec::new();
    let mut used_expected = Vec::new();
    for (i, &e) in expected.iter().enumerate() {
        if e > 0.0 {
            used_observed.push(observed[i]);
            used_expected.push(e);
        } else {
            assert_eq!(observed[i], 0, "a zero-probability deal was proposed");
        }
    }
    assert!(
        used_expected.len() > 1,
        "the support must have more than one deal"
    );

    let chi2 = chi_square_statistic(&used_observed, &used_expected);
    let df = (used_expected.len() - 1) as f64;
    let p_value = chi_square_p_value(chi2, df);
    assert!(
        p_value > 0.01,
        "chi-square = {chi2} (df = {df}) rejects at the 0.01 level (p = {p_value})"
    );
}
