//! `NaturalInference::candidates` returns only legal calls, each paired with the same
//! `(constraint, rule)` that `infer(classify(auction.with(call), ..))` would produce, and never a
//! `fallback`-rule call (06-system.md §8.3, last paragraph).

mod common;

use bridge_core::{Seat, Vulnerability};
use bridge_system::natural::{NaturalInference, classify};
use common::auction;

#[test]
fn candidates_are_legal_and_match_classify_infer() {
    let engine = NaturalInference::default();
    let a = auction(Seat::North, Vulnerability::None, "1S P");
    let owner = a.next_seat();
    let legal: Vec<_> = a.legal_calls().collect();

    let candidates = engine.candidates(&a, owner);
    assert!(!candidates.is_empty());
    for (call, constraint, priority) in &candidates {
        assert!(legal.contains(call), "{call} is not a legal continuation");

        let next = a.with(*call).unwrap();
        let ctx = classify(&next, a.len(), owner);
        let inf = engine.infer(&ctx);
        assert_ne!(inf.rule, "fallback");
        assert_eq!(*priority, (inf.confidence * 100.0).round() as i16);
        // Same rule fired, so the same shape of constraint: spot-check via a hand that satisfies
        // one iff it satisfies the other by comparing HCP ranges (a cheap, deterministic summary
        // that is exact for a single `Atom`, which every rule here produces).
        assert_eq!(constraint.hcp_range(), inf.constraint.hcp_range());
    }
}

#[test]
fn candidates_excludes_fallback_calls() {
    // Opener's notrump rebid matches no rule in the v1 table (see the `fallback` test in
    // `natural_rules.rs`); `candidates` must simply omit that call rather than report it with the
    // `ANY` fallback constraint.
    let engine = NaturalInference::default();
    let a = auction(Seat::North, Vulnerability::None, "1C P 1H P");
    let owner = a.next_seat();
    let candidates = engine.candidates(&a, owner);
    let nt: bridge_core::Call = "1NT".parse().unwrap();
    assert!(
        candidates.iter().all(|(call, _, _)| *call != nt),
        "1NT rebid should have been dropped as a fallback-only candidate"
    );
}

#[test]
fn candidates_cover_every_legal_call_at_least_with_fallback_in_infer() {
    // Even when `candidates` drops fallback calls, `infer` itself never panics for any legal
    // continuation: every legal call classifies and infers to *something*.
    let engine = NaturalInference::default();
    let a = auction(Seat::North, Vulnerability::None, "1S P P");
    let owner = a.next_seat();
    for call in a.legal_calls() {
        let next = a.with(call).unwrap();
        let ctx = classify(&next, a.len(), owner);
        let _ = engine.infer(&ctx);
    }
}
