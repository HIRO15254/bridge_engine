//! Exercises the `calc_dd_tables`/`solve_all_boards` chunk boundaries directly (39/40/41 for
//! `MAXNOOFTABLES`, 199/200/201 for `MAXNOOFBOARDS`; docs/design/12-roadmap.md 5.5), without
//! needing the corpus: each batched call is checked against the same positions solved one at a
//! time with `calc_dd_table`/`solve_board`, at counts just below, at, and just above the chunk
//! size, so an off-by-one in the `.chunks()` split or in reassembling chunk results in order
//! would show up here even when `list100_matches_upstream` (which only checks a couple of
//! batch sizes) does not happen to cross it.
#![cfg(dds_vendored)]

use bridge_core::{Card, Deal, Hand, Seat, Strain};
use bridge_dds::{
    Mode, Position, Solutions, Target, calc_dd_table, calc_dd_tables, solve_all_boards,
    solve_board, sys,
};

/// `count` distinct, valid, full deals (a formula shuffle, not real hands).
fn sample_deals(count: usize) -> Vec<Deal> {
    (0..count)
        .map(|shift| {
            let mut hands = [Hand::EMPTY; 4];
            for i in 0..52u8 {
                let card = Card::from_index(i).expect("i < 52");
                let seat = ((i as usize).wrapping_mul(7).wrapping_add(shift)) % 4;
                hands[seat] = hands[seat].with(card);
            }
            Deal::new(hands).expect("four 13-card hands covering the deck")
        })
        .collect()
}

fn check_calc_dd_tables_at(count: usize) {
    let deals = sample_deals(count);
    let batched = calc_dd_tables(&deals).expect("calc_dd_tables should not fail");
    assert_eq!(batched.len(), count);
    for (deal, table) in deals.iter().zip(&batched) {
        let single = calc_dd_table(deal).expect("calc_dd_table should not fail");
        assert_eq!(
            single.as_array(),
            table.as_array(),
            "calc_dd_tables[{}] diverged from calc_dd_table for the same deal (count={count})",
            deals.iter().position(|d| d == deal).unwrap()
        );
    }
}

#[test]
fn calc_dd_tables_below_the_chunk_boundary() {
    check_calc_dd_tables_at(sys::MAXNOOFTABLES - 1);
}

#[test]
fn calc_dd_tables_at_the_chunk_boundary() {
    check_calc_dd_tables_at(sys::MAXNOOFTABLES);
}

#[test]
fn calc_dd_tables_above_the_chunk_boundary() {
    check_calc_dd_tables_at(sys::MAXNOOFTABLES + 1);
}

fn check_solve_all_boards_at(count: usize) {
    let deals = sample_deals(count);
    let positions: Vec<_> = deals
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
    let batched = solve_all_boards(&positions).expect("solve_all_boards should not fail");
    assert_eq!(batched.len(), count);
    for (i, (deal, ft)) in deals.iter().zip(&batched).enumerate() {
        let pos = Position {
            deal,
            trump: Strain::NoTrump,
            leader: Seat::North,
            trick: &[],
        };
        let single = solve_board(&pos, Target::Max, Solutions::AllRanked, Mode::Auto)
            .expect("solve_board should not fail");
        assert_eq!(
            single.cards, ft.cards,
            "solve_all_boards[{i}] diverged from solve_board for the same deal (count={count})"
        );
    }
}

#[test]
fn solve_all_boards_below_the_chunk_boundary() {
    check_solve_all_boards_at(sys::MAXNOOFBOARDS - 1);
}

#[test]
fn solve_all_boards_at_the_chunk_boundary() {
    check_solve_all_boards_at(sys::MAXNOOFBOARDS);
}

#[test]
fn solve_all_boards_above_the_chunk_boundary() {
    check_solve_all_boards_at(sys::MAXNOOFBOARDS + 1);
}

#[test]
fn calc_dd_tables_and_solve_all_boards_reject_empty_input_by_returning_empty() {
    // Not an error path: an empty slice is a degenerate but valid batch (zero chunks).
    assert_eq!(calc_dd_tables(&[]).expect("empty input is valid").len(), 0);
    assert_eq!(
        solve_all_boards(&[]).expect("empty input is valid").len(),
        0
    );
}
