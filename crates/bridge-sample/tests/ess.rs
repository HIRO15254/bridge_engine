//! `ConstraintProposal` vs `UniformProposal`: effective sample size over a batch of hand-built
//! interpretations modelled on real auctions (§9 of `09-sample.md`, table row `uniform_vs_
//! constraint_ess`). `#[ignore]`d: it is a statistical benchmark, not a correctness check, and
//! is run on demand (`cargo test -p bridge-sample --all-features -- --ignored uniform_vs`).
//!
//! With a hand-built [`Interpretation`] and `ctx.bidding = None`, the target density is `Σ_s ln
//! interpretation.likelihood(s, h_s)` (`ln_likelihood` in `lib.rs`), so `SampleReport::ess` (from
//! `sample_deals` itself, not recomputed here) measures exactly how well each proposal's `π`
//! fits that interpretation's own mixture — which is what this table row is about.

use std::fs;
use std::path::PathBuf;

use bridge_bidding::{
    CallExplanation, CallInterpretation, Explanation, Interpretation, ResolutionKind,
};
use bridge_constraint::{Atom, HandConstraint, KnownCards, ShapeSet};
use bridge_core::{Call, Seat};
use bridge_sample::{
    ConstraintProposal, SampleContext, SampleOptions, Threads, UniformProposal, sample_deals,
};

/// One constrained seat's contribution to a scenario: a primary constraint with weight `1 −
/// eps`, plus the ε-mixture defensive `ANY` branch every real interpretation carries (07-
/// bidding.md §4.2, D15).
struct SeatCall {
    seat: Seat,
    primary: HandConstraint,
    eps: f32,
}

struct Scenario {
    name: &'static str,
    calls: Vec<SeatCall>,
}

fn balanced(hcp: core::ops::RangeInclusive<u8>) -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: ShapeSet::BALANCED,
        hcp,
        cards: Vec::new(),
        eval: Vec::new(),
    })
}

fn suit_len(
    suit: bridge_core::Suit,
    lo: u8,
    hi: u8,
    hcp: core::ops::RangeInclusive<u8>,
) -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: ShapeSet::from_suit_len(suit, lo, hi),
        hcp,
        cards: Vec::new(),
        eval: Vec::new(),
    })
}

fn strong_any_shape(hcp: core::ops::RangeInclusive<u8>) -> HandConstraint {
    HandConstraint::Atom(Atom {
        shapes: ShapeSet::ALL,
        hcp,
        cards: Vec::new(),
        eval: Vec::new(),
    })
}

/// The 13 scenarios (task 5.3 / 09-sample.md §9): opening bids, a raise, a limit raise, a weak
/// two in each major, a strong artificial opening, a rebid sequence, an overcall, Stayman, a
/// game raise and a preempt. Every seat not mentioned is left completely unconstrained (`ANY`,
/// direct-dealt, §6.4 (a)).
///
/// A real bidding sequence's constraint on a hand is usually not razor-thin (07-bidding.md's
/// Step B ANDs a seat's own calls together, and its own natural-inference / partial-match
/// fallbacks tend to widen rather than narrow whenever a system match is inexact) — most of
/// these are deliberately generous for that reason. A handful (weak 2♠/2♥, the strong 2♣, the
/// preempt) are kept genuinely narrow on purpose: real conventions sometimes *are* that narrow,
/// and the point of reporting `uniform_ess_ratio` alongside is to show both proposals under the
/// same hard case, not to hide it.
fn scenarios() -> Vec<Scenario> {
    use bridge_core::Suit::{Diamonds, Hearts, Spades};

    vec![
        Scenario {
            name: "1NT-ish opener (balanced 12-17)",
            calls: vec![SeatCall {
                seat: Seat::North,
                primary: balanced(12..=17),
                eps: 0.02,
            }],
        },
        Scenario {
            name: "1C - 1H - 1NT rebid (balanced 8-17)",
            calls: vec![SeatCall {
                seat: Seat::North,
                // The combined effect of opening 1C and rebidding 1NT (Step B of 07-bidding.md
                // §4.4 would AND the two calls' own constraints together; a hand-built scenario
                // states the combined result directly).
                primary: balanced(8..=17),
                eps: 0.02,
            }],
        },
        Scenario {
            name: "weak 2S",
            calls: vec![SeatCall {
                seat: Seat::North,
                primary: suit_len(Spades, 6, 13, 5..=11),
                eps: 0.15,
            }],
        },
        Scenario {
            name: "weak 2H",
            calls: vec![SeatCall {
                seat: Seat::North,
                primary: suit_len(Hearts, 6, 13, 5..=11),
                eps: 0.15,
            }],
        },
        Scenario {
            name: "2C strong artificial",
            calls: vec![SeatCall {
                seat: Seat::North,
                primary: strong_any_shape(20..=37),
                eps: 0.02,
            }],
        },
        Scenario {
            name: "preemptive 3S",
            calls: vec![SeatCall {
                seat: Seat::North,
                primary: suit_len(Spades, 7, 13, 4..=10),
                eps: 0.15,
            }],
        },
        Scenario {
            name: "opponents' 1H overcall",
            calls: vec![SeatCall {
                seat: Seat::East,
                primary: suit_len(Hearts, 5, 13, 8..=21),
                eps: 0.02,
            }],
        },
        Scenario {
            name: "1D opener (3+, opening values)",
            calls: vec![SeatCall {
                seat: Seat::North,
                primary: suit_len(Diamonds, 3, 13, 6..=21),
                eps: 0.02,
            }],
        },
        Scenario {
            name: "semi-balanced opener (10-17)",
            calls: vec![SeatCall {
                seat: Seat::North,
                primary: HandConstraint::Atom(Atom {
                    shapes: ShapeSet::SEMI_BALANCED,
                    hcp: 10..=17,
                    cards: Vec::new(),
                    eval: Vec::new(),
                }),
                eps: 0.02,
            }],
        },
        Scenario {
            name: "1H opener - 2H raise",
            calls: vec![
                SeatCall {
                    seat: Seat::North,
                    primary: suit_len(Hearts, 5, 13, 8..=21),
                    eps: 0.02,
                },
                SeatCall {
                    seat: Seat::South,
                    primary: suit_len(Hearts, 3, 13, 5..=11),
                    eps: 0.02,
                },
            ],
        },
        Scenario {
            name: "1S opener - 3S limit raise",
            calls: vec![
                SeatCall {
                    seat: Seat::North,
                    primary: suit_len(Spades, 4, 13, 6..=21),
                    eps: 0.02,
                },
                SeatCall {
                    seat: Seat::South,
                    primary: suit_len(Spades, 4, 13, 6..=21),
                    eps: 0.02,
                },
            ],
        },
        Scenario {
            name: "1NT - 2C Stayman",
            calls: vec![
                SeatCall {
                    seat: Seat::North,
                    primary: balanced(12..=17),
                    eps: 0.02,
                },
                SeatCall {
                    seat: Seat::South,
                    // Asks for a 4-card major; either one qualifies.
                    primary: HandConstraint::Or(vec![
                        suit_len(Hearts, 4, 13, 8..=37),
                        suit_len(Spades, 4, 13, 8..=37),
                    ]),
                    eps: 0.02,
                },
            ],
        },
        Scenario {
            name: "1NT - 3NT game raise",
            calls: vec![
                SeatCall {
                    seat: Seat::North,
                    primary: balanced(12..=17),
                    eps: 0.02,
                },
                SeatCall {
                    seat: Seat::South,
                    primary: strong_any_shape(10..=14),
                    eps: 0.02,
                },
            ],
        },
    ]
}

/// Builds the `Interpretation` for a scenario: each mentioned seat gets one call whose
/// alternatives are `[(primary, 1 - eps), (ANY, eps)]` (the ε-mixture every real auction call
/// carries), in both `seats` (what `ConstraintProposal` builds alternatives from) and `per_call`
/// (what `Interpretation::likelihood` sums over — the target density this test measures ESS
/// against).
fn build_interpretation(scenario: &Scenario) -> Interpretation {
    let mut seats: [Vec<(HandConstraint, f32, Explanation)>; 4] =
        [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let mut per_call = Vec::new();

    for call in &scenario.calls {
        let idx = call.seat.index() as usize;
        let weighted: Vec<(HandConstraint, f32)> = vec![
            (call.primary.clone(), 1.0 - call.eps),
            (HandConstraint::ANY, call.eps),
        ];
        seats[idx] = weighted
            .iter()
            .map(|(c, w)| {
                (
                    c.clone(),
                    *w,
                    Explanation {
                        text: String::new(),
                        node: None,
                        resolution: ResolutionKind::Exact,
                        parts: Vec::new(),
                    },
                )
            })
            .collect();
        let alternatives = weighted
            .into_iter()
            .map(|(c, w)| {
                (
                    c,
                    w,
                    CallExplanation {
                        call_index: 0,
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
            seat: call.seat,
            call: Call::Pass,
            kind: ResolutionKind::Exact,
            alternatives,
        });
    }

    Interpretation {
        seats,
        per_call,
        divergence: None,
    }
}

fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.partial_cmp(b).expect("no NaNs among ESS ratios"));
    let n = xs.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        xs[n / 2]
    } else {
        (xs[n / 2 - 1] + xs[n / 2]) / 2.0
    }
}

/// A minimal hand-rolled JSON writer (no new dependency): every value here is a plain string or
/// finite `f64`, so a full serializer would be overkill.
fn write_report(path: &PathBuf, per_scenario: &[(String, f64, f64)], n: usize) {
    let mut json = String::new();
    json.push_str("{\n");
    json.push_str(&format!("  \"n\": {n},\n"));
    json.push_str("  \"scenarios\": [\n");
    for (i, (name, constraint_ratio, uniform_ratio)) in per_scenario.iter().enumerate() {
        let comma = if i + 1 < per_scenario.len() { "," } else { "" };
        let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
        json.push_str(&format!(
            "    {{ \"name\": \"{escaped}\", \"constraint_ess_ratio\": {constraint_ratio:.6}, \
             \"uniform_ess_ratio\": {uniform_ratio:.6} }}{comma}\n"
        ));
    }
    json.push_str("  ],\n");
    let constraint_median = median(per_scenario.iter().map(|(_, c, _)| *c).collect());
    let uniform_median = median(per_scenario.iter().map(|(_, _, u)| *u).collect());
    json.push_str(&format!(
        "  \"median_constraint_ess_ratio\": {constraint_median:.6},\n"
    ));
    json.push_str(&format!(
        "  \"median_uniform_ess_ratio\": {uniform_median:.6}\n"
    ));
    json.push_str("}\n");

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(path, json).expect("writing target/ess_report.json");
}

#[test]
#[ignore = "statistical benchmark: n = 1000 per scenario across 12+ scenarios, run on demand"]
fn uniform_vs_constraint_ess() {
    let scenarios = scenarios();
    assert!(
        scenarios.len() >= 12,
        "the design calls for at least 12 hand-built scenarios, got {}",
        scenarios.len()
    );

    let n = 1000usize;
    let opts = SampleOptions {
        seed: 20260925,
        max_attempts_per_sample: 32,
        max_attempt_factor: 200,
        threads: Threads::Single,
    };
    let play_constraints = [
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
    ];

    let mut per_scenario = Vec::new();
    for scenario in &scenarios {
        let interpretation = build_interpretation(scenario);
        let ctx = SampleContext {
            known: KnownCards::EMPTY,
            interpretation: &interpretation,
            play_constraints: &play_constraints,
            play_soft: None,
            bidding: None,
        };

        let (_, constraint_report) = sample_deals(&ctx, &ConstraintProposal::default(), n, &opts)
            .unwrap_or_else(|e| panic!("{}: ConstraintProposal failed: {e}", scenario.name));
        let (_, uniform_report) = sample_deals(&ctx, &UniformProposal, n, &opts)
            .unwrap_or_else(|e| panic!("{}: UniformProposal failed: {e}", scenario.name));

        per_scenario.push((
            scenario.name.to_string(),
            constraint_report.ess_ratio,
            uniform_report.ess_ratio,
        ));
    }

    let report_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ess_report.json");
    write_report(&report_path, &per_scenario, n);

    let constraint_median = median(per_scenario.iter().map(|(_, c, _)| *c).collect());
    let uniform_median = median(per_scenario.iter().map(|(_, _, u)| *u).collect());
    assert!(
        constraint_median >= 0.5,
        "ConstraintProposal's median ESS ({}) is below the 0.5n target ({}); per-scenario: {:?}",
        constraint_median * n as f64,
        0.5 * n as f64,
        per_scenario
    );
    // Reported, not asserted on: this is what the comparison is for (D-row of §9's table).
    let _ = uniform_median;
}
