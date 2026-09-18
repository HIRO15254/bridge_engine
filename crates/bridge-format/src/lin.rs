//! BBO LIN files.
//!
//! A LIN stream is a sequence of `tag|value|` pairs. Boards are delimited by `qx|<id>|` in
//! vugraph files (`o14` / `c14` for open and closed room), otherwise the file is one board.
//!
//! | Tag | Meaning |
//! | --- | --- |
//! | `pn` | player names, S W N E (eight names in vugraph files: open room then closed room) |
//! | `md` | `<dealer><hands>`: dealer `1`=S `2`=W `3`=N `4`=E, then the S W N E hands separated by `,`; an omitted fourth hand is computed |
//! | `sv` | vulnerability `o` none, `n` NS, `e` EW, `b` both |
//! | `mb` | a call (`p d r 1C 1N …`, case-insensitive; trailing `!` = alert) |
//! | `an` | announcement text attached to the preceding `mb` |
//! | `pc` | a card played |
//! | `mc` | claim of `n` tricks |
//! | `ah`, `st`, `rh`, `nt`, `pg`, … | stored as raw or ignored |

use bridge_core::{Call, Card, Seat, Vulnerability};

use crate::{
    Warning,
    pbn::{Game, PartialDeal},
};

/// One board from a LIN stream.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LinBoard {
    /// `qx` id, if any.
    pub id: Option<String>,
    /// Player names as given.
    pub names: Vec<String>,
    /// Board number from `ah|Board n|`, if any.
    pub board_no: Option<u16>,
    /// Dealer.
    pub dealer: Option<Seat>,
    /// Vulnerability.
    pub vul: Option<Vulnerability>,
    /// The deal (hands may be missing).
    pub deal: Option<PartialDeal>,
    /// Calls with alert flag and announcement.
    pub calls: Vec<(Call, bool, Option<String>)>,
    /// Cards played.
    pub plays: Vec<Card>,
    /// Claim.
    pub claim: Option<u8>,
    /// Unrecognised or informational pairs, verbatim.
    pub raw: Vec<(String, String)>,
}

impl LinBoard {
    /// Converts to a PBN game (tags and sections).
    pub fn to_game(&self) -> Game {
        todo!("phase 1")
    }
}

/// Parses a LIN stream leniently; unknown tags become warnings and are skipped.
pub fn parse_lenient(input: &[u8]) -> (Vec<LinBoard>, Vec<Warning>) {
    todo!("phase 1")
}
