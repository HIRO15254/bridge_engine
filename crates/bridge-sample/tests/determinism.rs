//! `Threads::Single` and a fixed-size rayon pool must give bit-identical results (D12, D7 of
//! `09-sample.md` §7): every slot depends only on `(seed, slot index)`, never on how many
//! threads process the chunk it falls in.

#![cfg(feature = "parallel")]

use bridge_bidding::{Explanation, Interpretation, ResolutionKind};
use bridge_constraint::{Atom, HandConstraint, KnownCards, ShapeSet};
use bridge_sample::{
    ConstraintProposal, SampleContext, SampleOptions, Threads, UniformProposal, sample_deals,
};

fn empty_interpretation() -> Interpretation {
    Interpretation {
        seats: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        per_call: Vec::new(),
        divergence: None,
    }
}

#[test]
fn single_thread_and_seven_thread_pool_agree_bit_for_bit() {
    let interpretation = empty_interpretation();
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

    let single_opts = SampleOptions {
        seed: 2026,
        max_attempts_per_sample: 16,
        max_attempt_factor: 50,
        threads: Threads::Single,
    };
    let (single_deals, single_report) = sample_deals(&ctx, &UniformProposal, 200, &single_opts)
        .expect("single-threaded sampling succeeds");

    let auto_opts = SampleOptions {
        threads: Threads::Auto,
        ..single_opts
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(7)
        .build()
        .expect("building a 7-thread pool");
    let (pooled_deals, pooled_report) = pool.install(|| {
        sample_deals(&ctx, &UniformProposal, 200, &auto_opts).expect("pooled sampling succeeds")
    });

    assert_eq!(single_deals.len(), pooled_deals.len());
    for (a, b) in single_deals.iter().zip(pooled_deals.iter()) {
        assert_eq!(
            a.deal, b.deal,
            "deals differ between Single and the 7-thread pool"
        );
        assert_eq!(
            a.log_weight.to_bits(),
            b.log_weight.to_bits(),
            "log weights differ between Single and the 7-thread pool"
        );
    }

    // The report is deterministic too, except `elapsed` (wall time).
    assert_eq!(single_report.requested, pooled_report.requested);
    assert_eq!(single_report.produced, pooled_report.produced);
    assert_eq!(single_report.attempts, pooled_report.attempts);
    assert_eq!(single_report.ess.to_bits(), pooled_report.ess.to_bits());
    assert_eq!(
        single_report.ess_ratio.to_bits(),
        pooled_report.ess_ratio.to_bits()
    );
    assert_eq!(
        single_report.log_weight_max.to_bits(),
        pooled_report.log_weight_max.to_bits()
    );
    assert_eq!(single_report.warnings, pooled_report.warnings);
}

/// North must hold a balanced 15-17 HCP hand (exercised through `ConstraintProposal`'s
/// `Sampled` path); East, South and West are unconstrained (`Direct` dealing, §6.4 (a)). Every
/// `Sampler::sample` call still only consumes `rng_for(seed, slot)`, so this must be exactly as
/// deterministic across thread counts as `UniformProposal`.
fn balanced_north_interpretation() -> Interpretation {
    let strong_balanced = HandConstraint::Atom(Atom {
        shapes: ShapeSet::BALANCED,
        hcp: 15..=17,
        cards: Vec::new(),
        eval: Vec::new(),
    });
    let explanation = Explanation {
        text: "15-17 balanced".to_string(),
        node: None,
        resolution: ResolutionKind::Exact,
        parts: Vec::new(),
    };
    Interpretation {
        seats: [
            vec![(strong_balanced, 1.0, explanation)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ],
        per_call: Vec::new(),
        divergence: None,
    }
}

#[test]
fn constraint_proposal_single_thread_and_seven_thread_pool_agree_bit_for_bit() {
    let interpretation = balanced_north_interpretation();
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
    let proposal = ConstraintProposal::default();

    let single_opts = SampleOptions {
        seed: 20260925,
        max_attempts_per_sample: 32,
        max_attempt_factor: 100,
        threads: Threads::Single,
    };
    let (single_deals, single_report) = sample_deals(&ctx, &proposal, 200, &single_opts)
        .expect("single-threaded sampling succeeds");

    let auto_opts = SampleOptions {
        threads: Threads::Auto,
        ..single_opts
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(7)
        .build()
        .expect("building a 7-thread pool");
    let (pooled_deals, pooled_report) = pool.install(|| {
        sample_deals(&ctx, &proposal, 200, &auto_opts).expect("pooled sampling succeeds")
    });

    assert_eq!(single_deals.len(), pooled_deals.len());
    assert!(
        !single_deals.is_empty(),
        "the constraint is satisfiable and should produce deals"
    );
    for (a, b) in single_deals.iter().zip(pooled_deals.iter()) {
        assert_eq!(
            a.deal, b.deal,
            "deals differ between Single and the 7-thread pool"
        );
        assert_eq!(
            a.log_weight.to_bits(),
            b.log_weight.to_bits(),
            "log weights differ between Single and the 7-thread pool"
        );
    }

    assert_eq!(single_report.requested, pooled_report.requested);
    assert_eq!(single_report.produced, pooled_report.produced);
    assert_eq!(single_report.attempts, pooled_report.attempts);
    assert_eq!(single_report.ess.to_bits(), pooled_report.ess.to_bits());
    assert_eq!(
        single_report.ess_ratio.to_bits(),
        pooled_report.ess_ratio.to_bits()
    );
    assert_eq!(
        single_report.log_weight_max.to_bits(),
        pooled_report.log_weight_max.to_bits()
    );
    assert_eq!(single_report.warnings, pooled_report.warnings);
}
