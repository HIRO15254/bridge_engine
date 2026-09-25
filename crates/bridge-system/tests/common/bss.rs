//! Encoding/decoding of the `.bss` oracle format, ported directly from the reference
//! `gpaulissen/bml/src/bml/bss.py` (`VUL_DICT`, `SEAT_DICT`, `Sequence`, `systemdata_to_bss`).
//!
//! `bss.py`'s per-line "flags" (the `N`, `YYYYYY`, `0` and optional `08` fields written by
//! `systemdata_to_bss`) are hardcoded constants in the reference implementation itself -- they
//! are never actually computed from the system -- so they carry no information and this parser
//! does not attempt to interpret them beyond skipping over them to find the description. The
//! real oracle content is: whether we or they made the first call (`*` prefix), the seat and
//! vulnerability condition, the concrete call sequence (implicit passes included, written as a
//! plain concatenation of tokens with no separator), and the description text.
#![allow(dead_code)]

use bridge_core::{Call, Strain};
use bridge_system::ast::{SeatCond, Tri, VulCond};

/// `SEAT_DICT` values: the bss digit for a `#SEAT` condition.
pub fn bss_seat_char(seat: SeatCond) -> char {
    match seat {
        SeatCond::Any => '0',
        SeatCond::First => '1',
        SeatCond::Second => '2',
        SeatCond::Third => '3',
        SeatCond::Fourth => '4',
        SeatCond::FirstOrSecond => '5',
        SeatCond::ThirdOrFourth => '6',
    }
}

fn tri_char(t: Tri) -> char {
    match t {
        Tri::Any => '0',
        Tri::No => 'N',
        Tri::Yes => 'Y',
    }
}

/// `VUL_DICT` values: the bss digit for a `#VUL` condition (keyed by `we` then `they`, exactly
/// as the reference's literal table -- there is no arithmetic relationship between the two
/// characters and the digit, so this mirrors the table verbatim).
pub fn bss_vul_char(vul: VulCond) -> char {
    match [tri_char(vul.we), tri_char(vul.they)] {
        ['0', '0'] => '0',
        ['N', 'N'] => '1',
        ['Y', 'N'] => '2',
        ['N', 'Y'] => '3',
        ['Y', 'Y'] => '4',
        ['N', '0'] => '5',
        ['Y', '0'] => '6',
        ['0', 'N'] => '7',
        ['0', 'Y'] => '8',
        other => unreachable!(
            "VulCond {{we: {:?}, they: {:?}}} -> {other:?} is not one of bss.py's 9 VUL_DICT keys",
            vul.we, vul.they
        ),
    }
}

fn bss_strain_letter(s: Strain) -> char {
    match s {
        Strain::Clubs => 'C',
        Strain::Diamonds => 'D',
        Strain::Hearts => 'H',
        Strain::Spades => 'S',
        Strain::NoTrump => 'N',
    }
}

/// The bss token for one call (`bml.py`'s own vocabulary: `P`, `D`, `R`, `<level><strain>`;
/// note this is unrelated to `Call`'s own `Display`, which spells double/redouble `X`/`XX`).
pub fn bss_token(call: Call) -> String {
    match call {
        Call::Pass => "P".to_string(),
        Call::Double => "D".to_string(),
        Call::Redouble => "R".to_string(),
        Call::Bid(b) => format!("{}{}", b.level(), bss_strain_letter(b.strain())),
    }
}

/// The bss sequence string for a full concrete call list (implicit passes included): a plain
/// concatenation of each call's token, no separator (`Sequence.__str__`, after its parens are
/// stripped -- our `Call` list never carries the "whose call" parens in the first place).
pub fn bss_sequence(calls: &[Call]) -> String {
    calls.iter().map(|&c| bss_token(c)).collect()
}

/// One data line of a `.bss` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BssEntry {
    /// Whether the partnership the system describes made the first call of the sequence.
    pub we_open: bool,
    /// bss seat digit.
    pub seat: char,
    /// bss vulnerability digit.
    pub vul: char,
    /// The call sequence, e.g. `"1CP1DP1HP2C"`.
    pub sequence: String,
    /// The description, verbatim (already variable-substituted by the compiler).
    pub desc: String,
}

/// A parsed `.bss` file.
#[derive(Clone, Debug, Default)]
pub struct BssFile {
    /// `#+TITLE`, from the file's first line.
    pub title: String,
    /// `#+DESCRIPTION`, from the file's first line.
    pub description: String,
    /// One entry per non-root line.
    pub entries: Vec<BssEntry>,
}

/// Parses `.bss` text (`systemdata_to_bss`'s output format).
pub fn parse_bss(text: &str) -> BssFile {
    let mut file = BssFile::default();

    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let (we_open, rest) = match line.strip_prefix('*') {
            Some(r) => (false, r),
            None => (true, line),
        };

        // The one meta line: literally `*00{TITLE}=NYYYYYY` + DESCRIPTION (systemdata_to_bss's
        // first `f.write`, unconditional and unrelated to any real sequence).
        if let Some(braced) = rest.strip_prefix("00{") {
            if let Some(close) = braced.find('}') {
                file.title = braced[..close].to_string();
                let after = &braced[close + 1..];
                if let Some(after_eq) = after.strip_prefix('=') {
                    file.description = after_eq
                        .strip_prefix("NYYYYYY")
                        .unwrap_or(after_eq)
                        .to_string();
                }
            }
            continue;
        }

        if rest.len() < 2 {
            continue;
        }
        let seat = rest.as_bytes()[0] as char;
        let vul = rest.as_bytes()[1] as char;
        let after_seatvul = &rest[2..];
        let Some(eq) = after_seatvul.find('=') else {
            continue;
        };
        let sequence = after_seatvul[..eq].to_string();
        let tail = &after_seatvul[eq + 1..];

        // `N` (artificial, hardcoded) + `YYYYYY` (result flags, hardcoded) + `0` (characteristics,
        // hardcoded) are always present; a suit bid (level 1-7 + C/D/H/S) additionally gets a
        // hardcoded `08` (systemdata_to_bss's card-count placeholder) before the description.
        let Some(tail) = tail.strip_prefix("NYYYYYY0") else {
            continue;
        };
        let last_two = &sequence[sequence.len().saturating_sub(2)..];
        let is_suit_bid = last_two.len() == 2
            && matches!(last_two.as_bytes()[0], b'1'..=b'7')
            && matches!(last_two.as_bytes()[1], b'C' | b'D' | b'H' | b'S');
        let desc = if is_suit_bid {
            tail.strip_prefix("08").unwrap_or(tail)
        } else {
            tail
        };

        file.entries.push(BssEntry {
            we_open,
            seat,
            vul,
            sequence,
            desc: desc.to_string(),
        });
    }

    file
}

/// Merges `override_file`'s entries into `base` (same key = replace, else append), per
/// `06-system.md` §1.4's `*.bss.override` mechanism for the documented intentional differences.
pub fn apply_override(base: &mut BssFile, override_file: &BssFile) {
    for entry in &override_file.entries {
        base.entries.retain(|e| {
            !(e.we_open == entry.we_open
                && e.seat == entry.seat
                && e.vul == entry.vul
                && e.sequence == entry.sequence)
        });
        base.entries.push(entry.clone());
    }
}
