//! No natural rule may be fully shadowed by a higher-ranked one at a canonical position: under
//! the natural rank order (`NaturalInference::ranked_candidates`, docs/design/06-system.md
//! §8.6) the policy picks the first candidate a hand satisfies, so a rule whose every hand also
//! satisfies an earlier candidate is never bid and its natural exclusive region (`Y_c`,
//! 07-bidding.md §4.1) is empty.
//!
//! Regression for the phase-4.6 retune that ranked `raise` (0.45) below the shape-free 1-level
//! `resp_nt` (0.5), so after `1x P` the natural policy never raised. The only documented
//! exception is `cue` / `penalty_x` behind the unbounded `pass_default` (all at 0.3; `Pass`
//! wins the tie by call order, §8.6).

mod common;

use bridge_constraint::{HandConstraint, SampleOptions, Sampler};
use bridge_core::{Hand, Seat, Vulnerability};
use bridge_system::TieBreak;
use bridge_system::natural::{NaturalCandidate, NaturalInference, PartnerContext};
use common::auction;
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;

/// Positions (dealer North, nobody vulnerable) covering openings, responses with and without
/// interference, overcalls, advances and opener's rebids.
const POSITIONS: &[&str] = &[
    "",
    "P",
    "P P P",
    "1C P",
    "1D P",
    "1H P",
    "1S P",
    "1NT P",
    "2H P",
    "1C 1H",
    "1C 1D",
    "1D 1S",
    "1H 2C",
    "1C X",
    "1C",
    "1H",
    "2S",
    "1D 1H P",
    "1S 2C P",
    "1H X P",
    "1C P 1H P",
    "1D P 1S P",
    "1H P 1S P",
    "1H P 2C P",
    "1S P 1NT P",
    "1C P 1D P",
];

/// Rules allowed to be fully shadowed (see the module doc).
const ALLOWED: &[&str] = &["cue", "penalty_x"];

/// Hands sampled per candidate when the exclusive region cannot be counted exactly.
const SAMPLES: usize = 4000;

/// `true` when some hand satisfies `ranked[i]` and no earlier candidate: exactly when the
/// sampler can count `C_i ∧ ¬(C_0 ∨ … ∨ C_{i-1})` without rejection, otherwise by sampling
/// `C_i` and checking the earlier candidates.
fn wins_somewhere(ranked: &[NaturalCandidate], i: usize, rng: &mut Xoshiro256PlusPlus) -> bool {
    let own = &ranked[i].constraint;
    if i == 0 {
        return Sampler::prepare(own, Hand::FULL, Hand::EMPTY, &SampleOptions::default())
            .is_ok_and(|s| s.count() > 0);
    }
    let higher = ranked[..i]
        .iter()
        .map(|c| c.constraint.clone())
        .reduce(HandConstraint::or)
        .expect("i > 0");
    let exact_opts = SampleOptions {
        allow_rejection: false,
        ..SampleOptions::default()
    };
    let region = own.clone().and(higher.not());
    if let Ok(sampler) = Sampler::prepare(&region, Hand::FULL, Hand::EMPTY, &exact_opts) {
        if sampler.is_exact() {
            return sampler.count() > 0;
        }
    }
    let Ok(sampler) = Sampler::prepare(own, Hand::FULL, Hand::EMPTY, &SampleOptions::default())
    else {
        return false;
    };
    (0..SAMPLES).any(|_| {
        sampler
            .sample(rng)
            .is_some_and(|s| !ranked[..i].iter().any(|c| c.constraint.satisfies(s.hand)))
    })
}

/// The ranked natural candidates for the next call after `spec` (default partner context).
fn ranked_at(engine: &NaturalInference, spec: &str) -> Vec<NaturalCandidate> {
    let a = auction(Seat::North, Vulnerability::None, spec);
    engine.ranked_candidates(
        &a,
        a.next_seat(),
        &PartnerContext::default(),
        TieBreak::RowOrder,
    )
}

/// `(position, rule)` pairs whose every candidate at that position is fully shadowed.
fn shadowed_rules(engine: &NaturalInference) -> Vec<(String, &'static str)> {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x5ad0_0001);
    let mut out = Vec::new();
    for spec in POSITIONS {
        let ranked = ranked_at(engine, spec);
        if std::env::var_os("SHADOW_DUMP").is_some() {
            for (i, c) in ranked.iter().enumerate() {
                let w = wins_somewhere(&ranked, i, &mut rng);
                eprintln!(
                    "{spec:>12} {:>4} {:>16} {:>3} {}",
                    c.call.to_string(),
                    c.rule,
                    c.priority(),
                    if w { "" } else { "SHADOWED" }
                );
            }
        }
        let mut rules: Vec<&'static str> = ranked.iter().map(|c| c.rule).collect();
        rules.sort_unstable();
        rules.dedup();
        for rule in rules {
            let wins = (0..ranked.len())
                .filter(|&i| ranked[i].rule == rule)
                .any(|i| wins_somewhere(&ranked, i, &mut rng));
            if !wins {
                out.push((format!("[{spec}]"), rule));
            }
        }
    }
    out
}

#[test]
fn no_natural_rule_is_fully_shadowed_at_canonical_positions() {
    let found = shadowed_rules(&NaturalInference::default());
    let unexpected: Vec<_> = found
        .iter()
        .filter(|(_, rule)| !ALLOWED.contains(rule))
        .collect();
    assert!(
        unexpected.is_empty(),
        "fully shadowed natural rules: {unexpected:?}"
    );
}

/// Standard natural calls that the natural policy must choose for some hand. Higher calls that
/// repeat a lower call's constraint (a 5-level raise after the 4-level one, an advancer's jump
/// raise, opener's 4-level jump shift) are known limitations of the rule table and are not
/// listed (docs/design/06-system.md §8.6).
const CANONICAL: &[(&str, &str)] = &[
    ("", "P 1C 1D 1H 1S 1NT 2C 2D 2H 2S 2NT 3C 3D 3H 3S"),
    ("P P P", "P 1C 1H 2S"),
    ("1C P", "P 1D 1H 1S 1NT 2C 3C 2NT"),
    ("1D P", "P 1H 1S 1NT 2C 2D 3D"),
    ("1H P", "P 1S 1NT 2C 2D 2H 3H 4H 2NT"),
    ("1S P", "P 1NT 2C 2D 2H 2S 3S 4S"),
    ("1NT P", "P 2C 2NT 3NT"),
    ("1C 1H", "P X 1S 1NT 2C 2D"),
    ("1C 1D", "P X 1H 1S 1NT 2C"),
    ("1D 1S", "P X 1NT 2D"),
    ("1H 2C", "P X 2H 2D"),
    ("1C X", "P 1D 1H 1S 1NT 2C"),
    ("1H", "P X 1S 1NT 2C 2S"),
    ("1D 1H P", "P 1S 2H"),
    ("1C P 1H P", "1S 1NT 2C 3C 2D 2H 3H"),
    ("1H P 1S P", "1NT 2C 2H 3H 2S 3S"),
    ("1H P 2C P", "2H 3C 2S"),
];

#[test]
fn canonical_natural_calls_are_chosen_for_some_hand() {
    let engine = NaturalInference::default();
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0x5ad0_0002);
    let mut missing = Vec::new();
    for &(spec, calls) in CANONICAL {
        let ranked = ranked_at(&engine, spec);
        for call in calls.split_whitespace() {
            let call: bridge_core::Call = call.parse().expect("call");
            let wins = ranked
                .iter()
                .position(|c| c.call == call)
                .is_some_and(|i| wins_somewhere(&ranked, i, &mut rng));
            if !wins {
                missing.push(format!("[{spec}] {call}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "natural calls never chosen: {missing:?}"
    );
}

/// The reviewed hands: after `1H P` / `1S P` a 6-9 point hand with three-card support raises
/// (it chose 1NT when `raise` ranked below the shape-free 1NT response).
#[test]
fn simple_raise_beats_one_notrump_with_support() {
    let engine = NaturalInference::default();
    for (spec, hand, want) in [
        ("1H P", common::hand("982", "QT763", "KJ84", "7"), "2H"),
        ("1S P", common::hand("82", "J9763", "K73", "Q84"), "2S"),
        ("1C P", common::hand("QJ964", "K82", "732", "J4"), "2C"),
    ] {
        let ranked = ranked_at(&engine, spec);
        let chosen = ranked
            .iter()
            .find(|c| c.constraint.satisfies(hand))
            .map(|c| c.call.to_string());
        assert_eq!(chosen.as_deref(), Some(want), "[{spec}] {hand:?}");
    }
    // Balanced 8 without support still responds 1NT; 10 HCP with three hearts may too.
    for (spec, hand) in [
        ("1H P", common::hand("Q84", "K7432", "J7", "Q82")),
        ("1H P", common::hand("K84", "K732", "Q76", "Q82")),
    ] {
        let ranked = ranked_at(&engine, spec);
        let chosen = ranked.iter().find(|c| c.constraint.satisfies(hand));
        assert_eq!(chosen.map(|c| c.rule), Some("resp_nt"), "[{spec}] {hand:?}");
    }
}
