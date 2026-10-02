//! Phase 5 completion criterion (docs/design/12-roadmap.md 5.x): "DDS FFI works and returns
//! analysis for sampled deals" -- the full pipeline end to end, through the facade only.
//!
//! `systems/sayc/sayc.bml` is compiled, a real auction is interpreted against it,
//! [`sample_deals`] draws deals from [`ConstraintProposal`] weighted by the auction's own bidding
//! likelihood (`SampleContext::bidding`), and every sampled deal is solved double-dummy through
//! `bridge::dd::dds()`.
//!
//! Gracefully does nothing when DDS was not vendored (`cargo xtask dds vendor`), matching
//! `dds.rs`'s own `dds()`-returns-`None` handling, so `cargo test -p bridge --features dds`
//! still passes without the vendored sources.
#![cfg(all(feature = "dds", not(target_arch = "wasm32")))]

use std::path::PathBuf;
use std::sync::Arc;

use bridge::bidding::{
    BidContext, ImplicitPass, InterpretOptions, PolicyParams, Scoring, Table, interpret,
};
use bridge::constraint::HandConstraint;
use bridge::dd::dds;
use bridge::sample::{
    BiddingLikelihood, ConstraintProposal, KnownCards, SampleContext, SampleOptions, Threads,
    sample_deals,
};
use bridge::system::{self, NaturalInference, Severity, SystemIR};
use bridge::{Auction, Bid, Call, Deal, Seat, Strain, Vulnerability};

/// `<crate>/../../systems`, matching `bridge-system`'s own test helper.
fn systems_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../systems")
}

/// Compiles the checked-in `systems/sayc/sayc.bml` (not vendored, so this must always succeed)
/// and requires zero `Error`-severity lints, matching `bridge-system/tests/sayc.rs`.
fn compile_sayc() -> SystemIR {
    let path = systems_dir().join("sayc").join("sayc.bml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let opts = system::CompileOptions::default();
    let (ir, lints) = system::compile(
        &path.to_string_lossy(),
        &text,
        &system::lexer::FsLoader,
        &opts,
    );
    let errors = lints
        .iter()
        .filter(|l| l.severity == Severity::Error)
        .count();
    assert_eq!(
        errors, 0,
        "sayc.bml compiled with {errors} Error-severity lint(s)"
    );
    ir
}

fn sayc_table() -> Table {
    Table::uniform(
        Arc::new(compile_sayc()),
        Arc::new(NaturalInference::default()),
    )
}

/// `natural: None` falls back to `table.natural` inside `sequence_log_likelihood` itself (see
/// that function's own rustdoc), so this is equivalent to `Some(&NaturalInference::default())`
/// here, just without a separate borrow to keep alive.
fn bid_ctx() -> BidContext<'static> {
    BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::default(),
    }
}

fn bid(level: u8, strain: Strain) -> Call {
    Call::Bid(Bid::new(level, strain).expect("1..=7"))
}

/// Draws `n` deals for `auction` under `table`, weighted by the auction's real bidding
/// likelihood, and returns them alongside the sampling report.
fn sample_for_auction(
    table: &Table,
    auction: &Auction,
    n: usize,
    seed: u64,
) -> (
    Vec<bridge::sample::WeightedDeal>,
    bridge::sample::SampleReport,
) {
    let interp = interpret(table, auction, &InterpretOptions::default());
    let bctx = bid_ctx();
    let ctx = SampleContext {
        known: KnownCards::EMPTY,
        interpretation: &interp,
        play_constraints: &[HandConstraint::ANY; 4],
        play_soft: None,
        bidding: Some(BiddingLikelihood {
            table,
            auction,
            ctx: &bctx,
        }),
    };
    let proposal = ConstraintProposal::default();
    let opts = SampleOptions {
        seed,
        threads: Threads::Single,
        ..SampleOptions::default()
    };
    sample_deals(&ctx, &proposal, n, &opts)
        .expect("sample_deals should succeed for a legal, interpretable auction")
}

/// Every seat's known cards (here, none are fixed: `KnownCards::EMPTY`) are a subset of what the
/// sampled deal actually holds for that seat -- vacuously true with no known cards fixed, but the
/// same assertion protects a future test that does fix a viewer's hand.
fn assert_consistent_with_known(deal: &Deal, known: &KnownCards) {
    for seat in Seat::ALL {
        let fixed = known.known[seat.index() as usize];
        assert!(
            deal.hand(seat).intersect(fixed) == fixed,
            "{seat}'s sampled hand does not contain its known cards"
        );
    }
}

/// Every cell of a double-dummy table is a valid trick count.
fn assert_table_filled(table: &bridge::DdTable) {
    for strain in Strain::ALL {
        for seat in Seat::ALL {
            assert!(table.tricks(strain, seat) <= 13);
        }
    }
}

/// The light, always-run version: a handful of deals through the full compile -> interpret ->
/// sample -> solve pipeline, fast enough for `cargo test` in debug (well under the 20s budget).
#[test]
fn dds_over_sampled_deals_is_sane() {
    let Some(backend) = dds() else {
        eprintln!("DDS not vendored in this build (cargo xtask dds vendor); skipping");
        return;
    };

    let table = sayc_table();

    // 1NT - P - 3NT - P - P - P: North opens a natural 15-17 balanced 1NT, South raises straight
    // to game, everyone passes. North declares 3NT.
    let auction_1nt = Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        [
            bid(1, Strain::NoTrump),
            Call::Pass,
            bid(3, Strain::NoTrump),
            Call::Pass,
            Call::Pass,
            Call::Pass,
        ],
    )
    .expect("legal auction");

    // 1S - 2H - P - P - P: a competitive auction where East's simple overcall gets passed out.
    let auction_competitive = Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        [
            bid(1, Strain::Spades),
            bid(2, Strain::Hearts),
            Call::Pass,
            Call::Pass,
            Call::Pass,
        ],
    )
    .expect("legal auction");

    for auction in [&auction_1nt, &auction_competitive] {
        let (deals, report) = sample_for_auction(&table, auction, 6, 1);
        assert_eq!(report.produced, deals.len());
        assert!(!deals.is_empty(), "expected at least one sampled deal");
        for weighted in &deals {
            assert_consistent_with_known(&weighted.deal, &KnownCards::EMPTY);
            let dd_table = backend
                .dd_table(&weighted.deal)
                .expect("dd_table should succeed on a sampled deal");
            assert_table_filled(&dd_table);
        }
    }
}

/// The heavy version (`docs/design/12-roadmap.md` phase 5 completion criterion): 200 deals for
/// 1NT-P-3NT-P-P-P, every one solved double-dummy, and NS's 3NT (declared by North, the 1NT
/// opener) makes on a clear majority of them -- North's 15-17 and South's game raise combine to
/// a hand that is double-dummy makeable far more often than not.
#[test]
#[ignore = "83+ double-dummy solves over sampled deals; slow in debug, run with `cargo test --release -p bridge --features dds -- --ignored`"]
fn dds_1nt_3nt_makes_on_a_clear_majority_of_sampled_deals() {
    let Some(backend) = dds() else {
        eprintln!("DDS not vendored in this build (cargo xtask dds vendor); skipping");
        return;
    };

    let table = sayc_table();
    let auction = Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        [
            bid(1, Strain::NoTrump),
            Call::Pass,
            bid(3, Strain::NoTrump),
            Call::Pass,
            Call::Pass,
            Call::Pass,
        ],
    )
    .expect("legal auction");

    let (deals, report) = sample_for_auction(&table, &auction, 200, 7);
    assert_eq!(report.produced, 200);
    assert_eq!(deals.len(), 200);

    // `sample_deals` draws from a proposal that is only broadly *shaped* by the auction (§6 of
    // `09-sample.md`); the auction's actual posterior is the *importance-weighted* distribution
    // (`WeightedDeal::log_weight`), not a naive count of the raw draws -- a low `ess_ratio` (as
    // reported here) means many draws come from the proposal's wide eps-fallback tail rather
    // than the true 15-17-balanced/game-values region, and an unweighted majority count is
    // biased by exactly that tail.
    let weights = bridge::sample::WeightedDeal::normalized_weights(&deals);
    let mut makes_mass = 0.0f64;
    let mut raw_makes = 0usize;
    for (weighted, &w) in deals.iter().zip(&weights) {
        assert_consistent_with_known(&weighted.deal, &KnownCards::EMPTY);
        let dd_table = backend
            .dd_table(&weighted.deal)
            .expect("dd_table should succeed on a sampled deal");
        assert_table_filled(&dd_table);
        if dd_table.tricks(Strain::NoTrump, Seat::North) >= 9 {
            makes_mass += w;
            raw_makes += 1;
        }
    }

    eprintln!(
        "3NT by North makes on {raw_makes}/200 raw draws, {:.1}% of the importance-weighted \
         mass (report: {report:?})",
        makes_mass * 100.0
    );
    assert!(
        makes_mass > 0.5,
        "expected 3NT to make on a clear majority of the weighted deals, got {:.1}%",
        makes_mass * 100.0
    );
}
