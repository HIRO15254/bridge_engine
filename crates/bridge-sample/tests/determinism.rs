//! `Threads::Single` and a fixed-size rayon pool must give bit-identical results (D12, D7 of
//! `09-sample.md` §7): every slot depends only on `(seed, slot index)`, never on how many
//! threads process the chunk it falls in.

#![cfg(feature = "parallel")]

use bridge_bidding::Interpretation;
use bridge_constraint::{Atom, HandConstraint, KnownCards, ShapeSet};
use bridge_sample::{
    SampleContext, SampleOptions, SampleWarning, Threads, UniformProposal, sample_deals,
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
    assert_eq!(
        single_report.acceptance_rate.to_bits(),
        pooled_report.acceptance_rate.to_bits()
    );
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

/// The trivial case above never rejects a single attempt (`UniformProposal` with `HandConstraint::
/// ANY` everywhere always succeeds on the first try), so it never exercises a slot's within-slot
/// retries (`max_attempts_per_sample > 1`), more than one chunk, or a `Truncated` warning. A rare
/// hard play constraint on one seat forces all of that: most attempts violate it (retried, or the
/// slot fails outright), most chunks fail to reach `n`, and the attempt budget is exhausted before
/// `n` deals are produced.
#[test]
fn single_thread_and_seven_thread_pool_agree_bit_for_bit_with_a_rejecting_play_constraint() {
    let interpretation = empty_interpretation();
    // Balanced and 20-21 HCP: rare enough that a small attempt budget reliably truncates.
    let rare = HandConstraint::Atom(Atom {
        shapes: ShapeSet::BALANCED,
        hcp: 20..=21,
        cards: Vec::new(),
        eval: Vec::new(),
    });
    let play_constraints = [
        rare,
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

    let n = 30;
    let single_opts = SampleOptions {
        seed: 4104,
        max_attempts_per_sample: 8,
        max_attempt_factor: 20,
        threads: Threads::Single,
    };
    let (single_deals, single_report) = sample_deals(&ctx, &UniformProposal, n, &single_opts)
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
        sample_deals(&ctx, &UniformProposal, n, &auto_opts).expect("pooled sampling succeeds")
    });

    // Sanity: this constraint is actually rare enough, within this attempt budget, to need more
    // than one chunk (attempts > n) and to leave the sample truncated - otherwise this test would
    // not exercise anything the trivial case above does not already cover.
    assert!(
        single_report.attempts > n as u64,
        "sanity: expected more than one chunk of attempts, got {}",
        single_report.attempts
    );
    assert!(
        single_report.warnings.contains(&SampleWarning::Truncated {
            produced: single_report.produced
        }),
        "sanity: expected a Truncated warning, got {:?}",
        single_report.warnings
    );

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

    assert_eq!(single_report.requested, pooled_report.requested);
    assert_eq!(single_report.produced, pooled_report.produced);
    assert_eq!(single_report.attempts, pooled_report.attempts);
    assert_eq!(
        single_report.acceptance_rate.to_bits(),
        pooled_report.acceptance_rate.to_bits()
    );
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
