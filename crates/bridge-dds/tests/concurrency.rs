//! Several Rust threads calling `solve_board` concurrently give the same card scores as a
//! sequential run (docs/design/10-dds.md §7.2 rule 2, §8). Also checks `info().threads` (the
//! explicit `SetResources` call of R6, docs/design/12-roadmap.md).
#![cfg(dds_vendored)]

use bridge_core::{Card, Deal, Hand, Seat, Strain};
use bridge_dds::{
    CardScore, Mode, Position, Solutions, Target, calc_dd_tables, info, solve_all_boards,
    solve_board,
};

/// Distinct, valid, full deals: a formula shuffle (no `rand` dependency here), not real hands.
fn sample_deals(count: u8) -> Vec<Deal> {
    (0..count)
        .map(|shift| {
            let mut hands = [Hand::EMPTY; 4];
            for i in 0..52u8 {
                let card = Card::from_index(i).expect("i < 52");
                let seat = ((i.wrapping_mul(7).wrapping_add(shift)) % 4) as usize;
                hands[seat] = hands[seat].with(card);
            }
            Deal::new(hands).expect("four 13-card hands covering the deck")
        })
        .collect()
}

fn positions(deals: &[Deal], strains: &[Strain]) -> Vec<(Deal, Strain, Seat)> {
    let mut out = Vec::new();
    for deal in deals {
        for &trump in strains {
            for &leader in &Seat::ALL {
                out.push((*deal, trump, leader));
            }
        }
    }
    out
}

/// `SolveBoard`'s `nodes` count is DDS's own internal search-node counter: it varies from call
/// to call even for the very same position solved sequentially, twice in a row (transposition
/// table reuse), so it is not part of what "the same answer" means here. Only the scored cards
/// are compared.
fn cards_match(a: &[CardScore], b: &[CardScore]) -> bool {
    a == b
}

fn solve_one(deal: &Deal, trump: Strain, leader: Seat) -> Vec<CardScore> {
    let pos = Position {
        deal,
        trump,
        leader,
        trick: &[],
    };
    solve_board(&pos, Target::Max, Solutions::AllRanked, Mode::Auto)
        .expect("solve_board should not fail on a valid opening-lead position")
        .cards
}

/// Runs `cases` through `worker_count` threads (round-robin) and returns results in the
/// original order.
fn solve_concurrently(cases: &[(Deal, Strain, Seat)], worker_count: usize) -> Vec<Vec<CardScore>> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..worker_count)
            .map(|worker| {
                scope.spawn(move || {
                    cases
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| i % worker_count == worker)
                        .map(|(i, (deal, trump, leader))| (i, solve_one(deal, *trump, *leader)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut results: Vec<(usize, Vec<CardScore>)> = handles
            .into_iter()
            .flat_map(|h| h.join().expect("worker thread should not panic"))
            .collect();
        results.sort_by_key(|(i, _)| *i);
        results.into_iter().map(|(_, cards)| cards).collect()
    })
}

fn check(cases: &[(Deal, Strain, Seat)], worker_count: usize) {
    let sequential: Vec<Vec<CardScore>> = cases
        .iter()
        .map(|(deal, trump, leader)| solve_one(deal, *trump, *leader))
        .collect();
    let concurrent = solve_concurrently(cases, worker_count);

    assert_eq!(sequential.len(), concurrent.len());
    for (i, (seq, conc)) in sequential.iter().zip(concurrent.iter()).enumerate() {
        assert!(
            cards_match(seq, conc),
            "case {i}: concurrent solve_board diverged from sequential\n  sequential: {seq:?}\n  concurrent: {conc:?}"
        );
    }
}

/// Small enough to run in well under 10s in a debug build.
#[test]
fn concurrent_solve_board_matches_sequential() {
    let deals = sample_deals(2);
    let cases = positions(&deals, &[Strain::NoTrump]);
    check(&cases, 8);
}

/// The full scale from docs/design/10-dds.md §8 ("8 threads x 100 positions"); slow in debug.
/// Run with `cargo test --release -p bridge-dds -- --ignored`.
#[test]
#[ignore = "slow in debug; run with `cargo test --release -p bridge-dds -- --ignored`"]
fn concurrent_solve_board_matches_sequential_at_scale() {
    let deals = sample_deals(5);
    let cases = positions(
        &deals,
        &[
            Strain::Clubs,
            Strain::Diamonds,
            Strain::Hearts,
            Strain::Spades,
            Strain::NoTrump,
        ],
    );
    assert!(
        cases.len() >= 100,
        "expected >= 100 positions, got {}",
        cases.len()
    );
    check(&cases, 8);
}

/// Regression test for f3db40e: mixing a bulk call (`calc_dd_tables`/`solve_all_boards`, which
/// also drives DDS's own per-thread-index state internally) concurrently with slot-holding
/// `solve_board` calls from other Rust threads used to corrupt that shared state (observed as
/// `Moves::GetTrickData`'s `"Sum N is not four"` abort or an `ABsearch.cpp` assertion) before
/// the bulk call took every slot for its duration. Runs a batch of bulk calls on one thread
/// racing a stream of `solve_board` calls on several others and checks every result is still
/// self-consistent (a bulk-call result matches the same position solved singly); the original
/// bug crashed the process outright rather than returning a wrong answer, so surviving to the
/// end of this test is itself most of the regression coverage.
#[test]
fn concurrent_bulk_and_slot_calls_do_not_corrupt_each_other() {
    let deals = sample_deals(6);
    let bulk_positions: Vec<_> = deals
        .iter()
        .map(|deal| {
            (
                Position {
                    deal,
                    trump: Strain::NoTrump,
                    leader: Seat::North,
                    trick: &[],
                },
                Target::Max,
                Solutions::AllRanked,
                Mode::Auto,
            )
        })
        .collect();
    let solo_cases = positions(&deals, &[Strain::Spades, Strain::Hearts, Strain::NoTrump]);

    std::thread::scope(|scope| {
        // One thread repeatedly runs both bulk entry points.
        let bulk = scope.spawn(|| {
            for _ in 0..8 {
                let tables = calc_dd_tables(&deals).expect("calc_dd_tables should not fail");
                assert_eq!(tables.len(), deals.len());
                let futs =
                    solve_all_boards(&bulk_positions).expect("solve_all_boards should not fail");
                assert_eq!(futs.len(), bulk_positions.len());
                for ((deal, trump, leader, _), ft) in bulk_positions
                    .iter()
                    .map(|(pos, t, s, m)| (pos.deal, pos.trump, pos.leader, (t, s, m)))
                    .zip(&futs)
                {
                    let solo = solve_one(deal, trump, leader);
                    assert_eq!(
                        solo, ft.cards,
                        "solve_all_boards result diverged from solve_board for the same position \
                         while racing calc_dd_tables/solve_all_boards on another thread"
                    );
                }
            }
        });

        // Several threads hammer solve_board (slot-holding) at the same time.
        let solvers: Vec<_> = (0..4)
            .map(|worker| {
                let cases = &solo_cases;
                scope.spawn(move || {
                    for _ in 0..6 {
                        for (i, (deal, trump, leader)) in cases.iter().enumerate() {
                            if i % 4 != worker {
                                continue;
                            }
                            let a = solve_one(deal, *trump, *leader);
                            let b = solve_one(deal, *trump, *leader);
                            assert_eq!(a, b, "solve_board is not deterministic under mixed load");
                        }
                    }
                })
            })
            .collect();

        bulk.join().expect("bulk thread should not panic");
        for h in solvers {
            h.join().expect("solver thread should not panic");
        }
    });
}

#[test]
fn info_reports_at_least_one_thread() {
    let i = info().expect("DDS is vendored in this build");
    assert!(
        i.threads >= 1,
        "SetResources should configure at least one thread: {i:?}"
    );
}
