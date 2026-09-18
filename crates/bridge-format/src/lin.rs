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

use core::mem;

use bridge_core::{Auction, Call, Card, Hand, PlayHistory, Rank, Seat, Suit, Vulnerability};

use crate::{
    Warning, WarningKind, deal_string,
    pbn::{Game, PartialDeal, Section, TagPair, TagValue, Token},
    text,
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
    fn empty() -> LinBoard {
        LinBoard {
            id: None,
            names: Vec::new(),
            board_no: None,
            dealer: None,
            vul: None,
            deal: None,
            calls: Vec::new(),
            plays: Vec::new(),
            claim: None,
            raw: Vec::new(),
        }
    }

    fn has_content(&self) -> bool {
        self.id.is_some() || self.deal.is_some() || !self.calls.is_empty() || !self.plays.is_empty()
    }

    /// The room letter of the `qx` id (`o` or `c`), lower case.
    fn room(&self) -> Option<char> {
        self.id
            .as_ref()
            .and_then(|id| id.chars().next())
            .map(|c| c.to_ascii_lowercase())
    }

    /// Converts to a PBN game (tags and sections).
    ///
    /// Produces `Board`, `Dealer`, `Vulnerable`, `Deal`, the four player tags (closed-room
    /// names when the id starts with `c` and eight names were given), `Room`, an `Auction`
    /// section whose alerts and announcements become `=n=` references with `[Note]`s, a
    /// `Play` section laid out from the opening leader, `Contract`/`Declarer` from the
    /// auction, and `Result` from the claim or a complete play. Missing information simply
    /// leaves the tag out.
    pub fn to_game(&self) -> Game {
        let mut game = Game::default();
        let dealer = self
            .dealer
            .or_else(|| self.board_no.map(Seat::dealer_of_board));
        let vul = self
            .vul
            .or_else(|| self.board_no.map(Vulnerability::from_board_number));
        if let Some((_, vg)) = self.raw.iter().find(|(tag, _)| tag == "vg")
            && let Some(title) = vg.split(',').next().map(str::trim)
            && !title.is_empty()
        {
            push_tag(&mut game, "Event", title);
        }
        if let Some(n) = self.board_no {
            push_tag(&mut game, "Board", &n.to_string());
        }
        let names: &[String] = if self.room() == Some('c') && self.names.len() >= 8 {
            &self.names[4..8]
        } else {
            &self.names
        };
        for (tag, index) in [("West", 1), ("North", 2), ("East", 3), ("South", 0)] {
            if let Some(name) = names.get(index) {
                push_tag(&mut game, tag, name);
            }
        }
        if let Some(d) = dealer {
            push_tag(&mut game, "Dealer", &d.to_string());
        }
        if let Some(v) = vul {
            push_tag(&mut game, "Vulnerable", &v.to_string());
        }
        if let Some(deal) = &self.deal {
            let first = dealer.unwrap_or(Seat::North);
            push_tag(&mut game, "Deal", &deal_string::write_partial(deal, first));
        }
        match self.room() {
            Some('o') => push_tag(&mut game, "Room", "Open"),
            Some('c') => push_tag(&mut game, "Room", "Closed"),
            _ => {}
        }

        let mut auction: Option<Auction> = None;
        if let Some(dealer) = dealer
            && !self.calls.is_empty()
        {
            let mut section = Section {
                tag: "Auction".to_string(),
                arg: dealer.to_string(),
                tokens: Vec::new(),
                notes: Vec::new(),
            };
            let mut legal = Auction::new(dealer, vul.unwrap_or(Vulnerability::None));
            let mut ok = true;
            for (call, alerted, announcement) in &self.calls {
                section.tokens.push(Token::Call(*call));
                if *alerted || announcement.is_some() {
                    let n = section.notes.len() as u8 + 1;
                    if n <= 32 {
                        section.tokens.push(Token::NoteRef(n));
                        let text = announcement.clone().unwrap_or_else(|| "Alert".to_string());
                        section.notes.push((n, text));
                    }
                }
                if ok {
                    ok = legal.push(*call).is_ok();
                }
            }
            game.sections.push(section);
            if ok {
                auction = Some(legal);
            }
        }
        let contract = auction.as_ref().and_then(Auction::contract);
        if let Some(c) = contract {
            push_tag(&mut game, "Contract", &c.to_string());
            push_tag(&mut game, "Declarer", &c.declarer.to_string());
        } else if auction.as_ref().is_some_and(Auction::is_passed_out) {
            push_tag(&mut game, "Contract", "Pass");
        }

        let mut result = self.claim;
        if let Some(c) = contract
            && !self.plays.is_empty()
        {
            let leader = c.leader();
            let mut history = PlayHistory::new(c.bid.strain(), leader);
            for card in &self.plays {
                // Only the trick mechanics matter here: the view validates against the deal.
                if history.play(*card, Hand::EMPTY.with(*card)).is_err() {
                    break;
                }
            }
            let mut section = Section {
                tag: "Play".to_string(),
                arg: leader.to_string(),
                tokens: Vec::new(),
                notes: Vec::new(),
            };
            for trick in history.tricks() {
                let mut row = vec![Token::Unknown; 4];
                for (k, card) in trick.cards.iter().enumerate() {
                    if let Some(card) = card {
                        let seat = trick.leader.offset(k as u8);
                        let column = (seat.index() + 4 - leader.index()) % 4;
                        row[column as usize] = Token::Card(*card);
                    }
                }
                section.tokens.extend(row);
            }
            if history.cards().len() < 52 {
                section.tokens.push(Token::Terminator);
            }
            game.sections.push(section);
            if result.is_none() && history.cards().len() == 52 {
                result = Some(history.tricks_won(c.declarer.side()));
            }
        }
        if let Some(r) = result {
            push_tag(&mut game, "Result", &r.to_string());
        }
        game
    }
}

fn push_tag(game: &mut Game, name: &str, value: &str) {
    game.tags.push(TagPair {
        name: name.to_string(),
        value: TagValue::Str(value.to_string()),
        line: 0,
    });
}

/// Parses a LIN stream leniently; unknown tags become warnings and are skipped.
pub fn parse_lenient(input: &[u8]) -> (Vec<LinBoard>, Vec<Warning>) {
    let (text, bad_utf8) = text::decode(input);
    let mut parser = Parser {
        text: &text,
        pos: 0,
        line: 1,
        boards: Vec::new(),
        current: LinBoard::empty(),
        names: Vec::new(),
        unknown_seen: Vec::new(),
        warnings: Vec::new(),
    };
    if let Some(line) = bad_utf8 {
        parser.warn(
            line,
            WarningKind::Encoding,
            "input is not valid UTF-8; decoded as ISO-8859-1",
        );
    }
    parser.run();
    parser.finish()
}

/// Tags that are informational; they are kept verbatim without a warning.
const KNOWN_RAW: [&str; 17] = [
    "vg", "rs", "st", "rh", "ah", "nt", "pg", "bn", "bt", "tu", "cr", "cs", "hc", "lc", "mp", "pf",
    "up",
];

struct Parser<'a> {
    text: &'a str,
    pos: usize,
    line: u32,
    boards: Vec<LinBoard>,
    current: LinBoard,
    /// The most recent `pn`, copied into every new board.
    names: Vec<String>,
    /// Unknown tags already reported for the current board.
    unknown_seen: Vec<String>,
    warnings: Vec<Warning>,
}

impl Parser<'_> {
    fn warn(&mut self, line: u32, kind: WarningKind, message: impl Into<String>) {
        self.warnings
            .push(Warning::new(self.boards.len(), line, kind, message));
    }

    fn advance(&mut self, n: usize) {
        let skipped = &self.text[self.pos..self.pos + n];
        self.line += skipped.bytes().filter(|&b| b == b'\n').count() as u32;
        self.pos += n;
    }

    fn run(&mut self) {
        loop {
            let rest = &self.text[self.pos..];
            let trimmed = rest.trim_start();
            self.advance(rest.len() - trimmed.len());
            if trimmed.is_empty() {
                break;
            }
            let line = self.line;
            let Some(bar) = trimmed.find('|') else {
                self.warn(
                    line,
                    WarningKind::MalformedToken,
                    format!("dangling fragment {:?} skipped", trimmed.trim()),
                );
                break;
            };
            let tag = trimmed[..bar].trim();
            if tag.len() != 2 || !tag.chars().all(|c| c.is_ascii_alphanumeric()) {
                self.warn(
                    line,
                    WarningKind::MalformedToken,
                    format!("fragment {tag:?} is not a tag; skipped"),
                );
                self.advance(bar + 1);
                continue;
            }
            let after = &trimmed[bar + 1..];
            let Some(end) = after.find('|') else {
                self.warn(
                    line,
                    WarningKind::MalformedToken,
                    format!("tag {tag} has no closing '|'; skipped"),
                );
                break;
            };
            let tag = tag.to_ascii_lowercase();
            let value = &after[..end];
            self.advance(bar + 1 + end + 1);
            self.pair(&tag, value, line);
        }
    }

    fn pair(&mut self, tag: &str, value: &str, line: u32) {
        match tag {
            "qx" => self.start_board(value),
            "pn" => {
                let names: Vec<String> = value.split(',').map(|s| s.trim().to_string()).collect();
                self.names.clone_from(&names);
                self.current.names = names;
            }
            "md" => self.deal(value, line),
            "sv" => match vulnerability(value) {
                Ok(v) => {
                    if v.is_some() {
                        self.current.vul = v;
                    }
                }
                Err(()) => self.warn(
                    line,
                    WarningKind::MalformedToken,
                    format!("sv value {value:?} is not a vulnerability"),
                ),
            },
            "mb" => match parse_call(value) {
                Some((call, alerted)) => self.current.calls.push((call, alerted, None)),
                None => self.warn(
                    line,
                    WarningKind::MalformedToken,
                    format!("mb value {value:?} is not a call"),
                ),
            },
            "an" => {
                let text = value.trim();
                match self.current.calls.last_mut() {
                    Some(last) => {
                        if !text.is_empty() {
                            last.2 = Some(text.to_string());
                        }
                    }
                    None => self.warn(
                        line,
                        WarningKind::MalformedToken,
                        "announcement without a preceding call",
                    ),
                }
            }
            "pc" => match value.trim().parse::<Card>() {
                Ok(card) => self.current.plays.push(card),
                Err(_) => self.warn(
                    line,
                    WarningKind::MalformedToken,
                    format!("pc value {value:?} is not a card"),
                ),
            },
            "mc" => match value.trim().parse::<u8>() {
                Ok(n) if n <= 13 => self.current.claim = Some(n),
                _ => self.warn(
                    line,
                    WarningKind::MalformedToken,
                    format!("mc value {value:?} is not a trick count"),
                ),
            },
            _ => {
                if tag == "ah"
                    && let Some(n) = number_in(value)
                {
                    self.current.board_no = Some(n);
                }
                if !KNOWN_RAW.contains(&tag) && !self.unknown_seen.iter().any(|t| t == tag) {
                    self.unknown_seen.push(tag.to_string());
                    self.warn(
                        line,
                        WarningKind::UnknownTag,
                        format!("unknown tag {tag:?} kept as raw"),
                    );
                }
                self.current.raw.push((tag.to_string(), value.to_string()));
            }
        }
    }

    fn start_board(&mut self, id: &str) {
        if self.current.has_content() {
            let board = mem::replace(&mut self.current, LinBoard::empty());
            self.boards.push(board);
            self.unknown_seen.clear();
            self.current.names.clone_from(&self.names);
        }
        // Otherwise the pairs so far are the file header (vg, rs, pn): they stay with this
        // first board.
        let id = id.trim();
        self.current.id = Some(id.to_string());
        if self.current.board_no.is_none() {
            self.current.board_no = number_in(id);
        }
    }

    fn deal(&mut self, value: &str, line: u32) {
        let value = value.trim();
        let mut chars = value.chars();
        let hands_text = match chars.next() {
            Some(d @ '0'..='4') => {
                self.current.dealer = match d {
                    '1' => Some(Seat::South),
                    '2' => Some(Seat::West),
                    '3' => Some(Seat::North),
                    '4' => Some(Seat::East),
                    _ => self.current.board_no.map(Seat::dealer_of_board),
                };
                chars.as_str()
            }
            _ => {
                self.warn(
                    line,
                    WarningKind::MalformedToken,
                    "md value has no dealer digit",
                );
                value
            }
        };
        let fields: Vec<&str> = hands_text.split(',').collect();
        if fields.len() > 4 {
            self.warn(
                line,
                WarningKind::BadDeal,
                format!("md has {} hands; extra hands ignored", fields.len()),
            );
        }
        let mut hands = [None; 4];
        let mut seen = Hand::EMPTY;
        // Seats whose hand was left out (as opposed to dropped for being wrong).
        let mut omitted: Vec<usize> = Vec::new();
        for (k, seat) in [Seat::South, Seat::West, Seat::North, Seat::East]
            .into_iter()
            .enumerate()
        {
            let field = fields.get(k).map(|f| f.trim()).unwrap_or("");
            if field.is_empty() {
                if k < 3 {
                    self.warn(line, WarningKind::BadDeal, format!("{seat}: hand missing"));
                }
                omitted.push(seat.index() as usize);
                continue;
            }
            let hand = match parse_hand(field) {
                Ok(hand) => hand,
                Err(message) => {
                    self.warn(
                        line,
                        WarningKind::BadDeal,
                        format!("{seat}: {message}; hand dropped"),
                    );
                    continue;
                }
            };
            if hand.len() != 13 {
                self.warn(
                    line,
                    WarningKind::BadDeal,
                    format!(
                        "{seat} holds {} cards, expected 13; hand dropped",
                        hand.len()
                    ),
                );
                continue;
            }
            if let Some(card) = seen.intersect(hand).cards().next() {
                self.warn(
                    line,
                    WarningKind::BadDeal,
                    format!("{seat}: card {card} is held by two seats; hand dropped"),
                );
                continue;
            }
            seen = seen.union(hand);
            hands[seat.index() as usize] = Some(hand);
        }
        // An omitted hand is the remainder of the deck when the other three are complete.
        if let [only] = omitted.as_slice()
            && hands.iter().filter(|h| h.is_none()).count() == 1
            && seen.len() == 39
        {
            hands[*only] = Some(seen.complement());
        }
        self.current.deal = Some(PartialDeal { hands });
    }

    fn finish(mut self) -> (Vec<LinBoard>, Vec<Warning>) {
        let keep = self.current.has_content()
            || (self.boards.is_empty()
                && (!self.current.raw.is_empty() || !self.current.names.is_empty()));
        if keep {
            self.boards.push(self.current);
        }
        (self.boards, self.warnings)
    }
}

/// `Ok(None)` for an empty value (no information), `Err` for an unknown letter.
fn vulnerability(value: &str) -> Result<Option<Vulnerability>, ()> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" => Ok(None),
        "o" | "0" | "-" | "none" => Ok(Some(Vulnerability::None)),
        "n" | "ns" => Ok(Some(Vulnerability::NS)),
        "e" | "ew" => Ok(Some(Vulnerability::EW)),
        "b" | "both" | "all" => Ok(Some(Vulnerability::Both)),
        _ => Err(()),
    }
}

/// `p`, `d`, `r`, `1C`, `1N`, `1NT`, any case; a trailing `!` marks an alert.
fn parse_call(value: &str) -> Option<(Call, bool)> {
    let value = value.trim();
    let (text, alerted) = match value.strip_suffix('!') {
        Some(rest) => (rest.trim(), true),
        None => (value, false),
    };
    text.parse::<Call>().ok().map(|call| (call, alerted))
}

/// `SAKQHJT9D8765C432`: a suit letter introduces the ranks that follow it.
fn parse_hand(field: &str) -> Result<Hand, String> {
    let mut hand = Hand::EMPTY;
    let mut suit = None;
    let mut chars = field.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(s) = suit_of(c) {
            suit = Some(s);
            continue;
        }
        let Some(suit) = suit else {
            return Err(format!("rank {c:?} before any suit letter"));
        };
        let rank = if c == '1' && chars.peek() == Some(&'0') {
            chars.next();
            Rank::Ten
        } else {
            rank_of(c).ok_or_else(|| format!("{c:?} is not a rank"))?
        };
        let card = Card::new(suit, rank);
        if hand.contains(card) {
            return Err(format!("duplicate card {card}"));
        }
        hand = hand.with(card);
    }
    Ok(hand)
}

fn suit_of(c: char) -> Option<Suit> {
    Suit::ALL
        .into_iter()
        .find(|s| s.letter() == c.to_ascii_uppercase())
}

fn rank_of(c: char) -> Option<Rank> {
    Rank::ALL
        .into_iter()
        .find(|r| r.to_char() == c.to_ascii_uppercase())
}

/// The first run of digits in `text`.
fn number_in(text: &str) -> Option<u16> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let digits: String = text[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hands_and_calls() {
        let hand = parse_hand("SA87HQT65DJ97C932").unwrap();
        assert_eq!(hand.to_string(), "A87.QT65.J97.932");
        assert_eq!(parse_hand("s10h2").unwrap().to_string(), "T.2..");
        assert!(parse_hand("SAA").is_err());
        assert!(parse_hand("A").is_err());
        assert_eq!(parse_call("p"), Some((Call::Pass, false)));
        assert_eq!(parse_call("D"), Some((Call::Double, false)));
        assert_eq!(parse_call("r"), Some((Call::Redouble, false)));
        assert_eq!(parse_call("2S!"), Some(("2S".parse().unwrap(), true)));
        assert_eq!(parse_call("1n"), Some(("1NT".parse().unwrap(), false)));
        assert_eq!(parse_call("1NT"), Some(("1NT".parse().unwrap(), false)));
        assert_eq!(parse_call("8C"), None);
        assert_eq!(vulnerability("E"), Ok(Some(Vulnerability::EW)));
        assert_eq!(vulnerability("0"), Ok(Some(Vulnerability::None)));
        assert_eq!(vulnerability(""), Ok(None));
        assert_eq!(vulnerability("x"), Err(()));
        assert_eq!(number_in("Board 12"), Some(12));
        assert_eq!(number_in("o25"), Some(25));
        assert_eq!(number_in("st"), None);
    }
}
