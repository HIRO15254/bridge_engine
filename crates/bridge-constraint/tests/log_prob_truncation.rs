//! `Sampler::log_prob` must describe the distribution of hands `Sampler::sample` actually
//! returns, i.e. conditioned on `sample` returning `Some` within `opts.max_tries`. When a
//! rejection term is combined with an exact term (an `Or`), the rejection term's contribution to
//! `log_prob`'s normalising constant depends on `opts.max_tries`: a smaller `max_tries` makes that
//! term less likely to ever produce a hand, shrinking its share of the returned samples, so an
//! *exact*-term hand's `log_prob` must correspondingly be less negative (closer to its share alone)
//! at smaller `max_tries`. Before the fix, `log_prob` never referenced `max_tries` at all, so it
//! returned bit-identical values regardless of it.

use std::sync::Arc;

use bridge_constraint::{Atom, CustomPred, HandConstraint, SampleOptions, Sampler};
use bridge_core::Hand;

/// A 0-HCP hand: the lowest card of every suit's rank order, 13 zero-point cards. Deterministic
/// regardless of `Card`'s enumeration order (filters by `hcp == 0` rather than assuming a layout).
fn zero_hcp_hand() -> Hand {
    let mut hand = Hand::EMPTY;
    for card in Hand::FULL.cards() {
        if hand.len() == 13 {
            break;
        }
        if bridge_eval::hcp(Hand::EMPTY.with(card)) == 0 {
            hand = hand.with(card);
        }
    }
    assert_eq!(
        hand.len(),
        13,
        "the full deck has at least 13 zero-point cards"
    );
    hand
}

/// `Or(exact hcp<=10 atom, Custom "hcp >= 20")`: the two branches are disjoint (10 < 20), and the
/// `Custom` branch always needs rejection (no `Custom` is ever exact).
fn mixed_constraint() -> (HandConstraint, HandConstraint) {
    let exact = HandConstraint::Atom(Atom::ANY.with_hcp(0..=10));
    let pred = CustomPred {
        name: "hcp >= 20".to_string(),
        f: Arc::new(|h: Hand| bridge_eval::hcp(h) >= 20),
    };
    let c = exact.clone().or(HandConstraint::Custom(pred));
    (c, exact)
}

#[test]
fn log_prob_of_an_exact_term_hand_depends_on_max_tries_of_a_combined_rejection_term() {
    let (c, exact) = mixed_constraint();
    let hand = zero_hcp_hand();
    assert!(
        exact.satisfies(hand),
        "the hand must be in the exact branch"
    );
    assert!(
        bridge_eval::hcp(hand) < 20,
        "sanity: a 0-hcp hand can never also be in the Custom (hcp >= 20) branch"
    );
    assert!(c.satisfies(hand));

    // Same `burn_in` (and so the same deterministic burn-in draws) for both configurations:
    // `PreparedTerm::prepare`'s alpha estimate does not depend on `max_tries` at all, so any
    // difference in `log_prob` below is attributable only to the `s_i` (retry-success) factor.
    let opts_small = SampleOptions {
        max_tries: 2,
        burn_in: 100_000,
        ..SampleOptions::default()
    };
    let opts_large = SampleOptions {
        max_tries: 1000,
        burn_in: 100_000,
        ..SampleOptions::default()
    };

    let sampler_small =
        Sampler::prepare(&c, Hand::FULL, Hand::EMPTY, &opts_small).expect("prepares");
    let sampler_large =
        Sampler::prepare(&c, Hand::FULL, Hand::EMPTY, &opts_large).expect("prepares");

    let lp_small = sampler_small.log_prob(hand);
    let lp_large = sampler_large.log_prob(hand);

    assert!(lp_small.is_finite() && lp_large.is_finite());
    assert_ne!(
        lp_small.to_bits(),
        lp_large.to_bits(),
        "log_prob of an exact-term hand must depend on max_tries when combined with a \
         rejection term via Or (it was bit-identical before the fix)"
    );
    // A smaller `max_tries` makes the rejection term less likely to ever succeed, shrinking its
    // share of `z` and so making the exact-term hand's probability *larger* (less negative).
    assert!(
        lp_small > lp_large,
        "lp_small={lp_small} should exceed lp_large={lp_large}"
    );
}
