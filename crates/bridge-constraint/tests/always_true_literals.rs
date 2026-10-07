//! `Atom::normalize` must drop card/eval requirements that clamping reveals to be always true
//! (`count`/`range` covers every value the requirement's cards/metric can take). Before the fix,
//! such a literal survived normalize and reached the sampler's `classify`, which cannot tell a
//! vacuous literal from a real one: a vacuous multi-suit card requirement or a vacuous eval
//! requirement (e.g. `Controls in 0..=12`, always true) counted as an extra additive feature (or
//! forced `needs_full_check` once more than one such literal was present), turning what should be
//! an exact term into a term that needs rejection sampling.

use bridge_constraint::{
    Atom, CardRequirement, EvalRequirement, HandConstraint, Metric, SampleOptions, Sampler,
};
use bridge_core::{Card, Hand, Rank, Suit};

// ---------------------------------------------------------------------------------------------
// (a) `Atom::normalize` drops the vacuous literal outright.
// ---------------------------------------------------------------------------------------------

#[test]
fn normalize_drops_an_always_true_eval_requirement() {
    // `Controls` ranges over `0..=12` (`Metric::max`), and every hand's achievable minimum is 0,
    // so `0..=12` constrains nothing.
    let mut atom = Atom::ANY.with_hcp(10..=15).with_eval(EvalRequirement {
        metric: Metric::Controls,
        range: 0..=12,
    });
    assert_eq!(atom.eval.len(), 1, "sanity: the literal starts out present");
    atom.normalize();
    assert!(
        atom.eval.is_empty(),
        "an always-true eval requirement must be dropped by normalize, not kept as a no-op literal"
    );
}

#[test]
fn normalize_drops_an_always_true_card_requirement() {
    // A 2-card mask (one card from clubs, one from diamonds - not `single_suit`, so `classify`
    // would otherwise treat it as an additive feature) with `count in 0..=2`: every hand holds
    // between 0 and 2 of any 2-card mask, so this constrains nothing.
    let mask = Hand::EMPTY
        .with(Card::new(Suit::Clubs, Rank::Ace))
        .with(Card::new(Suit::Diamonds, Rank::Ace));
    let mut atom = Atom::ANY
        .with_hcp(10..=15)
        .with_cards(CardRequirement { mask, count: 0..=2 });
    assert_eq!(
        atom.cards.len(),
        1,
        "sanity: the literal starts out present"
    );
    atom.normalize();
    assert!(
        atom.cards.is_empty(),
        "an always-true card requirement must be dropped by normalize, not kept as a no-op literal"
    );
}

#[test]
fn normalize_clamps_before_dropping_an_over_wide_range() {
    // `count in 0..=200` on a 2-card mask clamps to `0..=2` before the always-true check runs;
    // the clamp and the drop must compose, not just an exact `0..=len` literal.
    let mask = Hand::EMPTY
        .with(Card::new(Suit::Clubs, Rank::Ace))
        .with(Card::new(Suit::Diamonds, Rank::Ace));
    let mut atom = Atom::ANY.with_cards(CardRequirement {
        mask,
        count: 0..=200,
    });
    atom.normalize();
    assert!(atom.cards.is_empty());

    let mut atom = Atom::ANY.with_eval(EvalRequirement {
        metric: Metric::Controls,
        range: 0..=255,
    });
    atom.normalize();
    assert!(atom.eval.is_empty());
}

// ---------------------------------------------------------------------------------------------
// (b) End-to-end: adding a vacuous literal alongside a real additive one must not change whether
// the sampler treats the term as exact, nor how many hands it counts (`to_dnf` normalizes every
// term before the sampler sees it, so this exercises the fix through the same path a real
// system's compiled constraints take). A single vacuous literal alone is not enough to observe
// the bug: `SampleOptions::default().extra_features == 1` lets `classify` absorb one additive
// candidate (real or vacuous) without forcing `needs_full_check`, so the vacuous literal has to
// compete with a genuine additive literal for that budget to expose the difference.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_vacuous_eval_literal_does_not_use_up_the_additive_feature_budget() {
    let opts = SampleOptions::default();
    let real_additive = EvalRequirement {
        metric: Metric::Controls,
        range: 2..=6,
    };
    let vacuous = EvalRequirement {
        metric: Metric::QuickTricks,
        range: 0..=16, // `Metric::QuickTricks.max() == 16`: always true.
    };

    let baseline =
        HandConstraint::Atom(Atom::ANY.with_hcp(10..=15).with_eval(real_additive.clone()));
    let with_vacuous_literal = HandConstraint::Atom(
        Atom::ANY
            .with_hcp(10..=15)
            .with_eval(real_additive)
            .with_eval(vacuous),
    );

    let baseline = Sampler::prepare(&baseline, Hand::FULL, Hand::EMPTY, &opts).expect("prepares");
    let with_vacuous_literal =
        Sampler::prepare(&with_vacuous_literal, Hand::FULL, Hand::EMPTY, &opts).expect("prepares");

    assert!(
        baseline.is_exact(),
        "sanity: one real additive literal alone fits the default extra_features budget"
    );
    assert!(
        with_vacuous_literal.is_exact(),
        "an always-true eval literal must not spend the additive-feature budget and turn an \
         otherwise-exact term into a rejection term (it did before Atom::normalize dropped it, \
         since classify saw two additive candidates instead of one)"
    );
    assert_eq!(baseline.count(), with_vacuous_literal.count());
}

#[test]
fn a_vacuous_multi_suit_card_literal_does_not_use_up_the_additive_feature_budget() {
    let opts = SampleOptions::default();
    let real_additive = EvalRequirement {
        metric: Metric::Controls,
        range: 2..=6,
    };
    let mask = Hand::EMPTY
        .with(Card::new(Suit::Clubs, Rank::Ace))
        .with(Card::new(Suit::Diamonds, Rank::Ace));

    let baseline =
        HandConstraint::Atom(Atom::ANY.with_hcp(10..=15).with_eval(real_additive.clone()));
    let with_vacuous_literal = HandConstraint::Atom(
        Atom::ANY
            .with_hcp(10..=15)
            .with_eval(real_additive)
            .with_cards(CardRequirement { mask, count: 0..=2 }),
    );

    let baseline = Sampler::prepare(&baseline, Hand::FULL, Hand::EMPTY, &opts).expect("prepares");
    let with_vacuous_literal =
        Sampler::prepare(&with_vacuous_literal, Hand::FULL, Hand::EMPTY, &opts).expect("prepares");

    assert!(
        baseline.is_exact(),
        "sanity: one real additive literal alone fits the default extra_features budget"
    );
    assert!(
        with_vacuous_literal.is_exact(),
        "an always-true multi-suit card literal must not spend the additive-feature budget and \
         turn an otherwise-exact term into a rejection term (it did before Atom::normalize \
         dropped it, since classify saw two additive candidates instead of one)"
    );
    assert_eq!(baseline.count(), with_vacuous_literal.count());
}
