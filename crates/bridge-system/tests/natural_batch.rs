//! `NaturalInference::infer_batch` classifies the shared history once and skips the explanation
//! strings; its constraints, confidences and rules must equal `classify` + `infer` per call
//! (docs/design/06-system.md §8, lane-S acceptance "S, grid": identical results, <= 4 us for the
//! 31 legal calls after `1NT P 2C`).

mod common;

use bridge_constraint::{Atom, HandConstraint};
use bridge_core::{Auction, Call, Seat, Vulnerability};
use bridge_system::natural::{NaturalInference, NaturalParams, PartnerContext, classify};
use common::auction;

/// `infer` per call, the reference `infer_batch` must reproduce.
fn reference(
    engine: &NaturalInference,
    a: &Auction,
    owner: Seat,
    partner: &PartnerContext,
    call: Call,
) -> (String, f32, &'static str) {
    match a.with(call) {
        Ok(next) => {
            let mut ctx = classify(&next, a.len(), owner);
            ctx.partner_constraint = partner.partner_constraint.clone();
            ctx.forcing_situation = partner.forcing_situation;
            let inf = engine.infer(&ctx);
            (format!("{:?}", inf.constraint), inf.confidence, inf.rule)
        }
        Err(_) => ("fallback".to_string(), 0.0, "fallback"),
    }
}

/// Checks every call (legal or not) at every position of `a`, under three partner contexts.
fn check_positions(engine: &NaturalInference, a: &Auction) -> usize {
    let partners = [
        PartnerContext::default(),
        PartnerContext {
            partner_constraint: Some(HandConstraint::Atom(Atom::ANY.with_hcp(6..=9))),
            forcing_situation: false,
        },
        PartnerContext {
            partner_constraint: Some(HandConstraint::Atom(Atom::ANY.with_hcp(15..=17))),
            forcing_situation: true,
        },
    ];
    let all: Vec<Call> = (0..38).filter_map(Call::from_index).collect();
    let mut checked = 0;
    for j in 0..=a.len() {
        let prefix = Auction::from_calls(a.dealer(), a.vulnerability(), a.calls()[..j].to_vec())
            .expect("prefix of a legal auction");
        if prefix.is_complete() {
            break;
        }
        let owner = prefix.next_seat();
        for partner in &partners {
            let batch = engine.infer_batch(&prefix, owner, partner, &all);
            assert_eq!(batch.len(), all.len());
            for (got, &call) in batch.iter().zip(&all) {
                assert_eq!(got.call, call);
                let (constraint, confidence, rule) =
                    reference(engine, &prefix, owner, partner, call);
                if prefix.is_legal(call) {
                    assert_eq!(got.rule, rule, "{prefix:?} {call}");
                    assert_eq!(got.confidence, confidence, "{prefix:?} {call}");
                    assert_eq!(
                        format!("{:?}", got.constraint),
                        constraint,
                        "{prefix:?} {call}"
                    );
                } else {
                    assert!(got.is_fallback(), "illegal {call} after {prefix:?}");
                }
                checked += 1;
            }
        }
    }
    checked
}

/// A pseudo-random legal auction: mostly passes, some bids a few steps above the last, some
/// doubles/redoubles when legal (splitmix64).
fn random_auction(seed: &mut u64) -> Auction {
    let mut next = || {
        *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let dealer = Seat::ALL[(next() % 4) as usize];
    let vul = [
        Vulnerability::None,
        Vulnerability::NS,
        Vulnerability::EW,
        Vulnerability::Both,
    ][(next() % 4) as usize];
    let mut a = Auction::new(dealer, vul);
    let length = 2 + (next() % 12) as usize;
    while a.len() < length && !a.is_complete() {
        let legal: Vec<Call> = a.legal_calls().collect();
        let r = next() % 10;
        let call = if r < 4 {
            Call::Pass
        } else if r == 4 && legal.contains(&Call::Double) {
            Call::Double
        } else if r == 5 && legal.contains(&Call::Redouble) {
            Call::Redouble
        } else {
            let bids: Vec<Call> = legal.iter().copied().filter(|c| c.is_bid()).collect();
            if bids.is_empty() {
                Call::Pass
            } else {
                bids[(next() % bids.len().min(6) as u64) as usize]
            }
        };
        a.push(call).expect("legal call");
    }
    a
}

#[test]
fn infer_batch_matches_infer_on_fixed_and_random_auctions() {
    let mut params = NaturalParams::default();
    // Exercise the level floor too (the default table and a stricter one).
    let engines = [NaturalInference::new(params.clone()), {
        params.level_floor.suit = [0, 0, 20, 24, 28, 32, 36];
        NaturalInference::new(params)
    }];
    let fixed = [
        "1NT P 2C",
        "1S P",
        "1H X XX 1S P 2S P 3S",
        "P P 1D 1H X 2H 3C",
        "1C 1D 1S 3D",
        "2S P 3S P 4S",
    ];
    let mut seed = 0x0B47_C401u64;
    let mut checked = 0;
    for engine in &engines {
        for s in fixed {
            checked += check_positions(engine, &auction(Seat::North, Vulnerability::None, s));
        }
        for _ in 0..60 {
            checked += check_positions(engine, &random_auction(&mut seed));
        }
    }
    eprintln!("infer_batch == infer on {checked} (position, partner, call) triples");
}

/// Release timing of `infer_batch` for the 31 legal calls after `1NT P 2C`, against the
/// per-call `classify` + `infer` loop (`cargo test -p bridge-system --release --test
/// natural_batch -- --ignored --nocapture`).
#[test]
#[ignore = "timing; run in release with --ignored --nocapture"]
fn infer_batch_timing() {
    let engine = NaturalInference::default();
    let a = auction(Seat::North, Vulnerability::None, "1NT P 2C");
    let owner = a.next_seat();
    let calls: Vec<Call> = a.legal_calls().collect();
    assert_eq!(calls.len(), 31);
    let partner = PartnerContext {
        partner_constraint: Some(HandConstraint::Atom(Atom::ANY.with_hcp(0..=7))),
        forcing_situation: false,
    };
    let per_call = || {
        calls
            .iter()
            .map(|&c| reference(&engine, &a, owner, &partner, c).2.len())
            .sum::<usize>()
    };
    let batch = || engine.infer_batch(&a, owner, &partner, &calls).len();
    let time = |f: &dyn Fn() -> usize| {
        let mut best = f64::INFINITY;
        for _ in 0..3 {
            let reps = 20_000;
            let started = std::time::Instant::now();
            let mut sink = 0;
            for _ in 0..reps {
                sink += std::hint::black_box(f());
            }
            assert!(sink > 0);
            best = best.min(started.elapsed().as_secs_f64() * 1e6 / reps as f64);
        }
        best
    };
    let per = time(&per_call);
    let bat = time(&batch);
    let ranked = time(&|| {
        engine
            .ranked_candidates(&a, owner, &partner, bridge_system::TieBreak::RowOrder)
            .len()
    });
    eprintln!(
        "1NT P 2C, 31 calls: per-call infer {per:.2} us, infer_batch {bat:.2} us, \
         ranked_candidates {ranked:.2} us (best of 3)"
    );
}
