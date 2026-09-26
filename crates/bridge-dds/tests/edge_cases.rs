//! Error paths and DDS return-value quirks the typed wrapper has to absorb (docs/design/10-dds.md
//! §7): out-of-range targets and out-of-turn trick cards are rejected in Rust *before* DDS sees
//! them (DDS's own checks write a `dump.txt` into the process working directory on every such
//! error), and `SolveBoard`'s `score == -2` "only one legal card, not searched" sentinel never
//! leaks out as a bogus trick count.

#![cfg(dds_vendored)]

use std::path::Path;

use bridge_core::{Card, Deal, Seat, Strain};
use bridge_dds::{DdsError, Mode, Position, Solutions, Target, solve_board, sys};

fn deal(pbn: &str) -> Deal {
    pbn.parse().expect("valid deal")
}

fn card(s: &str) -> Card {
    s.parse().expect("valid card")
}

/// East holds a singleton spade and 12 top winners; North leads a spade. With East to play
/// there is exactly one legal card, which is where DDS's `mode == 0` shortcut kicks in.
const FORCED: &str = "N:AKQ2.T98.T98.T98 3.AKQJ.AKQJ.AKQJ JT98.765.765.765 7654.432.432.432";

fn dump_file() -> &'static Path {
    Path::new("dump.txt")
}

fn assert_code(result: Result<bridge_dds::FutureTricks, DdsError>, code: i32) {
    match result {
        Err(DdsError::Code { code: got, .. }) => assert_eq!(got, code),
        other => panic!("expected DDS error {code}, got {other:?}"),
    }
}

#[test]
fn a_forced_card_gets_its_real_score_in_every_mode() {
    let d = deal(FORCED);
    let trick = [card("S2")];
    let pos = Position {
        deal: &d,
        trump: Strain::NoTrump,
        leader: Seat::North,
        trick: &trick,
    };
    for mode in [Mode::Auto, Mode::Search, Mode::ReuseTable] {
        for solutions in [Solutions::One, Solutions::AllOptimal, Solutions::AllRanked] {
            let ft = solve_board(&pos, Target::Max, solutions, mode).expect("valid position");
            assert_eq!(ft.cards.len(), 1, "{mode:?}/{solutions:?}: {ft:?}");
            assert_eq!(ft.cards[0].card, card("S3"));
            // NS cash four spades (East discards), then East takes the remaining nine.
            assert_eq!(ft.cards[0].score, 9, "{mode:?}/{solutions:?}: {ft:?}");
        }
    }
}

#[test]
fn out_of_range_target_is_rejected_without_calling_dds() {
    let d = deal(FORCED);
    let pos = Position {
        deal: &d,
        trump: Strain::Spades,
        leader: Seat::West,
        trick: &[],
    };
    let _ = std::fs::remove_file(dump_file());
    assert_code(
        solve_board(&pos, Target::Tricks(14), Solutions::One, Mode::Auto),
        sys::RETURN_TARGET_WRONG_HI,
    );
    assert!(
        !dump_file().exists(),
        "DDS itself saw the bad target and wrote dump.txt"
    );
}

#[test]
fn trick_card_played_out_of_turn_is_rejected_without_calling_dds() {
    let d = deal(FORCED);
    // East's S3 as if East had led, while North is the leader.
    let trick = [card("S3")];
    let pos = Position {
        deal: &d,
        trump: Strain::NoTrump,
        leader: Seat::North,
        trick: &trick,
    };
    let _ = std::fs::remove_file(dump_file());
    assert_code(
        solve_board(&pos, Target::Max, Solutions::One, Mode::Auto),
        sys::RETURN_CARD_COUNT,
    );
    assert!(
        !dump_file().exists(),
        "DDS itself saw the bad trick and wrote dump.txt"
    );
}
