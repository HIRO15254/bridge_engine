//! A [`DdTable`] is plain data: it can be read from a PBN `OptimumResultTable` with no solver
//! compiled in at all (docs/design/12-roadmap.md 5.7), and, when DDS *is* available, the table
//! a PBN file carries can be checked against a fresh solve through the same type.
#![cfg(feature = "format")]

use bridge::dd::{DdTable, dds};
use bridge::format::pbn;
use bridge::{Seat, Strain};

/// A hand-made PBN game whose table is DDS's output for its deal (checked by
/// `pbn_table_agrees_with_dds_when_available`).
const GAME: &str = "[Board \"1\"]\n[Dealer \"N\"]\n[Vulnerable \"-\"]\n\
[Deal \"N:4.KJ32.842.AQ743 JT987.Q876.AK5.2 AK532.T.JT6.T985 Q6.A954.Q973.KJ6\"]\n\
[OptimumResultTable \"Declarer;Denomination\\\\2R;Result\\\\2R\"]\n\
N  C  9\nN  D  6\nN  H  5\nN  S  6\nN NT  6\n\
E  C  3\nE  D  7\nE  H  8\nE  S  7\nE NT  6\n\
S  C  9\nS  D  6\nS  H  5\nS  S  6\nS NT  6\n\
W  C  3\nW  D  7\nW  H  8\nW  S  7\nW NT  7\n";

fn table_from_pbn() -> (bridge::Deal, DdTable) {
    let (file, warnings) = pbn::parse_lenient(GAME.as_bytes());
    assert!(warnings.is_empty(), "{warnings:?}");
    let view = file.games[0].view(None).expect("valid game");
    (
        view.deal
            .and_then(|d| d.complete())
            .expect("the game has a full deal"),
        view.dd_table
            .expect("the game has a complete OptimumResultTable"),
    )
}

#[test]
fn dd_table_is_readable_from_pbn_without_a_solver() {
    let (_, table) = table_from_pbn();
    assert_eq!(table.tricks(Strain::Clubs, Seat::North), 9);
    assert_eq!(table.tricks(Strain::Hearts, Seat::East), 8);
    assert_eq!(table.tricks(Strain::NoTrump, Seat::West), 7);
    assert_eq!(table.tricks(Strain::Spades, Seat::South), 6);
}

#[test]
fn pbn_table_agrees_with_dds_when_available() {
    let Some(solver) = dds() else {
        eprintln!("no double-dummy solver in this build; only the PBN side is checked");
        return;
    };
    let (deal, from_pbn) = table_from_pbn();
    let solved = solver.dd_table(&deal).expect("valid deal");
    assert_eq!(solved, from_pbn);
}
