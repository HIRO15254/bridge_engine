//! Typed view of a game.

use bridge_core::{
    Auction, Card, Contract, DdTable, Deal, Hand, PlayHistory, Seat, Strain, Vulnerability,
};

use crate::{
    WarningKind, deal_string,
    pbn::{Game, Section, TagValue, Token},
};

/// A deal in which some hands may be unknown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PartialDeal {
    /// Known hands indexed by [`Seat`]; `None` for `-`.
    pub hands: [Option<Hand>; 4],
}

impl PartialDeal {
    /// The full deal, if all four hands are known and valid.
    pub fn complete(&self) -> Option<bridge_core::Deal> {
        let [n, e, s, w] = self.hands;
        Deal::new([n?, e?, s?, w?]).ok()
    }
}

/// The interpreted content of a game: typed values derived from the tags and sections.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct GameView {
    /// `Board`.
    pub board: Option<u16>,
    /// `Dealer`.
    pub dealer: Option<Seat>,
    /// `Vulnerable`.
    pub vulnerable: Option<Vulnerability>,
    /// `Deal`.
    pub deal: Option<PartialDeal>,
    /// The `Auction` section as a validated auction.
    pub auction: Option<Auction>,
    /// The `Play` section rotated into trick order.
    pub play: Option<PlayHistory>,
    /// `Contract` (with `Declarer`).
    pub contract: Option<Contract>,
    /// `Declarer`.
    pub declarer: Option<Seat>,
    /// `Result` (tricks taken by declarer).
    pub result: Option<u8>,
    /// `OptimumResultTable`.
    pub dd_table: Option<DdTable>,
}

impl Game {
    /// Interprets the tags and sections. `previous` supplies values for `#` inheritance.
    ///
    /// Missing tags give `None`; only a tag value that cannot be interpreted is an error. A
    /// `-` in the auction leaves `auction` as `None`; a `-` in the play stops the play there.
    pub fn view(&self, previous: Option<&GameView>) -> Result<GameView, ViewError> {
        self.interpret(previous).0
    }

    /// [`Game::view`] plus the constructs it skipped or repaired, for the lenient parser.
    pub(crate) fn interpret(
        &self,
        previous: Option<&GameView>,
    ) -> (Result<GameView, ViewError>, Vec<Note>) {
        let mut notes = Vec::new();
        let result = Interp {
            game: self,
            previous,
            notes: &mut notes,
        }
        .run();
        (result, notes)
    }
}

/// A game could not be interpreted.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum ViewError {
    /// A tag value has the wrong form.
    #[error("tag {tag}: {message}")]
    BadTag {
        /// Tag name.
        tag: String,
        /// Detail.
        message: String,
    },
    /// The auction section is not a legal auction.
    #[error("auction: {0}")]
    Auction(#[from] bridge_core::AuctionError),
    /// The play section is not a legal play.
    #[error("play: {0}")]
    Play(#[from] bridge_core::PlayError),
    /// A `#` value has nothing to inherit from.
    #[error("tag {0} inherits from a previous game but there is none")]
    NothingToInherit(String),
}

/// What a [`Note`] refers to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Anchor {
    /// A tag pair, by name.
    Tag(&'static str),
    /// A section, by index into [`Game::sections`].
    Section(usize),
}

/// Something the interpretation skipped or repaired (a warning without a line yet).
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Note {
    pub kind: WarningKind,
    pub anchor: Anchor,
    pub message: String,
}

/// The mandatory tag set, in export order.
pub(crate) const MTS: [&str; 15] = [
    "Event",
    "Site",
    "Date",
    "Board",
    "West",
    "North",
    "East",
    "South",
    "Dealer",
    "Vulnerable",
    "Deal",
    "Scoring",
    "Declarer",
    "Contract",
    "Result",
];

enum Resolved<'a> {
    Missing,
    Inherit,
    Text(&'a str),
}

struct Interp<'a> {
    game: &'a Game,
    previous: Option<&'a GameView>,
    notes: &'a mut Vec<Note>,
}

fn bad(tag: &str, message: impl Into<String>) -> ViewError {
    ViewError::BadTag {
        tag: tag.to_string(),
        message: message.into(),
    }
}

impl<'a> Interp<'a> {
    fn note(&mut self, kind: WarningKind, anchor: Anchor, message: impl Into<String>) {
        self.notes.push(Note {
            kind,
            anchor,
            message: message.into(),
        });
    }

    fn resolve(&self, name: &str) -> Resolved<'a> {
        match self.game.get(name) {
            None => Resolved::Missing,
            Some(TagValue::Inherited) => Resolved::Inherit,
            Some(TagValue::Default(t) | TagValue::Str(t)) => {
                let t = t.trim();
                if t.is_empty() || t == "?" {
                    Resolved::Missing
                } else {
                    Resolved::Text(t)
                }
            }
        }
    }

    fn inherited<T>(&self, name: &str, field: impl FnOnce(&GameView) -> T) -> Result<T, ViewError> {
        self.previous
            .map(field)
            .ok_or_else(|| ViewError::NothingToInherit(name.to_string()))
    }

    /// A typed tag: missing → `None`, `#` → the previous game's value, otherwise parsed.
    fn typed<T: Copy>(
        &self,
        name: &'static str,
        field: impl FnOnce(&GameView) -> Option<T>,
        parse: impl FnOnce(&str) -> Result<T, String>,
    ) -> Result<Option<T>, ViewError> {
        match self.resolve(name) {
            Resolved::Missing => Ok(None),
            Resolved::Inherit => self.inherited(name, field),
            Resolved::Text(t) => parse(t).map(Some).map_err(|m| bad(name, m)),
        }
    }

    fn section(&self, name: &str) -> Option<(usize, &'a Section)> {
        self.game
            .sections
            .iter()
            .enumerate()
            .find(|(_, s)| s.tag == name)
    }

    fn run(mut self) -> Result<GameView, ViewError> {
        let mut v = GameView::default();
        v.board = self.typed(
            "Board",
            |p| p.board,
            |t| {
                t.parse::<u16>()
                    .map_err(|_| format!("{t:?} is not a board number"))
            },
        )?;
        v.dealer = self.typed(
            "Dealer",
            |p| p.dealer,
            |t| t.parse::<Seat>().map_err(|e| e.to_string()),
        )?;
        v.vulnerable = self.typed(
            "Vulnerable",
            |p| p.vulnerable,
            |t| t.parse::<Vulnerability>().map_err(|e| e.to_string()),
        )?;
        v.deal = self.deal()?;
        v.declarer = self.typed(
            "Declarer",
            |p| p.declarer,
            |t| {
                t.trim_start_matches('^')
                    .parse::<Seat>()
                    .map_err(|e| e.to_string())
            },
        )?;
        v.auction = self.auction(&v)?;
        v.contract = self.contract(&v)?;
        v.result = self.result(&v)?;
        v.play = self.play(&v)?;
        v.dd_table = self.dd_table()?;
        Ok(v)
    }

    fn deal(&mut self) -> Result<Option<PartialDeal>, ViewError> {
        match self.resolve("Deal") {
            Resolved::Missing => Ok(None),
            Resolved::Inherit => self.inherited("Deal", |p| p.deal),
            Resolved::Text(t) => {
                let (deal, problems) =
                    deal_string::parse_lenient(t).map_err(|e| bad("Deal", e.message))?;
                for problem in problems {
                    self.note(
                        WarningKind::BadDeal,
                        Anchor::Tag("Deal"),
                        format!("{problem}; hand dropped"),
                    );
                }
                Ok(Some(deal))
            }
        }
    }

    fn contract(&mut self, v: &GameView) -> Result<Option<Contract>, ViewError> {
        match self.resolve("Contract") {
            Resolved::Missing => Ok(None),
            Resolved::Inherit => self.inherited("Contract", |p| p.contract),
            Resolved::Text(t) => {
                if t.eq_ignore_ascii_case("Pass") {
                    return Ok(None);
                }
                let mut contract: Contract =
                    t.parse().map_err(|e| bad("Contract", format!("{e}")))?;
                let declarer = v.declarer.or_else(|| {
                    v.auction
                        .as_ref()
                        .and_then(Auction::contract)
                        .map(|c| c.declarer)
                });
                match declarer {
                    Some(declarer) => {
                        contract.declarer = declarer;
                        Ok(Some(contract))
                    }
                    None => {
                        self.note(
                            WarningKind::Other,
                            Anchor::Tag("Contract"),
                            "no Declarer and no complete auction: contract dropped",
                        );
                        Ok(None)
                    }
                }
            }
        }
    }

    fn result(&mut self, v: &GameView) -> Result<Option<u8>, ViewError> {
        match self.resolve("Result") {
            Resolved::Missing => Ok(None),
            Resolved::Inherit => self.inherited("Result", |p| p.result),
            Resolved::Text(t) => {
                let t = t.trim_start_matches('^').trim();
                let tricks = |s: &str| {
                    s.parse::<u8>()
                        .ok()
                        .filter(|n| *n <= 13)
                        .ok_or_else(|| bad("Result", format!("{s:?} is not a trick count")))
                };
                let words: Vec<&str> = t.split_whitespace().collect();
                match words.as_slice() {
                    [n] => tricks(n).map(Some),
                    [side, n, ..]
                        if side.eq_ignore_ascii_case("NS") || side.eq_ignore_ascii_case("EW") =>
                    {
                        let n = tricks(n)?;
                        let declarer = v.contract.map(|c| c.declarer).or(v.declarer);
                        let Some(declarer) = declarer else {
                            self.note(
                                WarningKind::Other,
                                Anchor::Tag("Result"),
                                "side result without a declarer: result dropped",
                            );
                            return Ok(None);
                        };
                        let ns = side.eq_ignore_ascii_case("NS");
                        let declarer_ns = declarer.side() == bridge_core::Side::NS;
                        Ok(Some(if ns == declarer_ns { n } else { 13 - n }))
                    }
                    _ => Err(bad("Result", format!("{t:?} is not a result"))),
                }
            }
        }
    }

    fn auction(&mut self, v: &GameView) -> Result<Option<Auction>, ViewError> {
        let Some((idx, section)) = self.section("Auction") else {
            return Ok(None);
        };
        let dealer = match section.arg.trim().parse::<Seat>() {
            Ok(seat) => seat,
            Err(_) => v.dealer.ok_or_else(|| {
                bad(
                    "Auction",
                    format!("{:?} is not a seat and there is no Dealer", section.arg),
                )
            })?,
        };
        let mut auction = Auction::new(dealer, v.vulnerable.unwrap_or(Vulnerability::None));
        for token in &section.tokens {
            match token {
                Token::Call(call) => {
                    if auction.is_complete() {
                        self.note(
                            WarningKind::BadAuction,
                            Anchor::Section(idx),
                            format!("call {call} after the auction is complete is ignored"),
                        );
                        break;
                    }
                    auction.push(*call)?;
                }
                Token::Terminator | Token::Continuation => break,
                Token::NoteRef(_) | Token::Nag(_) | Token::Suffix(_) => {}
                Token::Unknown => {
                    self.note(
                        WarningKind::Truncated,
                        Anchor::Section(idx),
                        "unknown call (-): the auction is not interpreted",
                    );
                    return Ok(None);
                }
                Token::Card(_) | Token::Raw(_) => {
                    self.note(
                        WarningKind::Truncated,
                        Anchor::Section(idx),
                        format!("unexpected token {token:?}: the auction is not interpreted"),
                    );
                    return Ok(None);
                }
            }
        }
        Ok(Some(auction))
    }

    fn play(&mut self, v: &GameView) -> Result<Option<PlayHistory>, ViewError> {
        let Some((idx, section)) = self.section("Play") else {
            return Ok(None);
        };
        let contract = v
            .contract
            .or_else(|| v.auction.as_ref().and_then(Auction::contract));
        let Some(contract) = contract else {
            self.note(
                WarningKind::Other,
                Anchor::Section(idx),
                "play without a contract: not interpreted",
            );
            return Ok(None);
        };
        let Some(deal) = v.deal.as_ref().and_then(PartialDeal::complete) else {
            self.note(
                WarningKind::Other,
                Anchor::Section(idx),
                "play without a complete deal: not interpreted",
            );
            return Ok(None);
        };
        let first: Seat = section
            .arg
            .trim()
            .parse()
            .map_err(|_| bad("Play", format!("{:?} is not a seat", section.arg)))?;
        let leader = contract.leader();
        if first != leader {
            return Err(bad(
                "Play",
                format!("first seat {first} is not the opening leader {leader}"),
            ));
        }
        let mut history = PlayHistory::new(contract.bid.strain(), leader);
        let mut cells: Vec<Option<Card>> = Vec::with_capacity(4);
        for token in &section.tokens {
            match token {
                Token::Card(card) => cells.push(Some(*card)),
                Token::Unknown | Token::Continuation => cells.push(None),
                Token::Terminator => break,
                Token::NoteRef(_) | Token::Nag(_) | Token::Suffix(_) => continue,
                Token::Call(_) | Token::Raw(_) => {
                    self.note(
                        WarningKind::Truncated,
                        Anchor::Section(idx),
                        format!("unexpected token {token:?}: the play stops there"),
                    );
                    break;
                }
            }
            if cells.len() == 4 {
                if !self.play_row(&mut history, &deal, first, &cells, idx)? {
                    return Ok(Some(history));
                }
                cells.clear();
            }
        }
        if !cells.is_empty() {
            cells.resize(4, None);
            self.play_row(&mut history, &deal, first, &cells, idx)?;
        }
        Ok(Some(history))
    }

    /// Plays one row (one trick) in play order. `false` when an unknown card cut it short.
    fn play_row(
        &mut self,
        history: &mut PlayHistory,
        deal: &Deal,
        first: Seat,
        cells: &[Option<Card>],
        idx: usize,
    ) -> Result<bool, ViewError> {
        let leader = history.next_to_play();
        let trick = history.cards().len() / 4 + 1;
        for k in 0..4u8 {
            let seat = leader.offset(k);
            let column = (seat.index() + 4 - first.index()) % 4;
            match cells[column as usize] {
                Some(card) => history.play(card, deal.hand(seat))?,
                None => {
                    self.note(
                        WarningKind::Truncated,
                        Anchor::Section(idx),
                        format!("unknown card for {seat} in trick {trick}: the play stops there"),
                    );
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn dd_table(&mut self) -> Result<Option<DdTable>, ViewError> {
        const TAG: &str = "OptimumResultTable";
        let Some((_, section)) = self.section(TAG) else {
            return Ok(None);
        };
        let columns: Vec<String> = section.arg.split(';').map(column_name).collect();
        let column = |name: &str| columns.iter().position(|c| c.eq_ignore_ascii_case(name));
        let (Some(ci_declarer), Some(ci_denomination), Some(ci_result)) =
            (column("Declarer"), column("Denomination"), column("Result"))
        else {
            return Err(bad(
                TAG,
                format!(
                    "columns {:?} lack Declarer, Denomination or Result",
                    section.arg
                ),
            ));
        };
        let mut tricks = [[None::<u8>; 4]; 5];
        let mut unknown = false;
        for token in &section.tokens {
            let Token::Raw(row) = token else { continue };
            let cells: Vec<&str> = row.split_whitespace().collect();
            let cell = |i: usize| {
                cells
                    .get(i)
                    .copied()
                    .ok_or_else(|| bad(TAG, format!("row {row:?} is short")))
            };
            let declarer: Seat = cell(ci_declarer)?
                .parse()
                .map_err(|_| bad(TAG, format!("bad declarer in row {row:?}")))?;
            let strain: Strain = cell(ci_denomination)?
                .parse()
                .map_err(|_| bad(TAG, format!("bad denomination in row {row:?}")))?;
            let result = cell(ci_result)?;
            if result == "?" || result == "-" {
                unknown = true;
                continue;
            }
            let n: u8 = result
                .parse()
                .ok()
                .filter(|n| *n <= 13)
                .ok_or_else(|| bad(TAG, format!("bad result in row {row:?}")))?;
            tricks[strain.index() as usize][declarer.index() as usize] = Some(n);
        }
        if unknown {
            return Ok(None);
        }
        let mut table = [[0u8; 4]; 5];
        for (strain, row) in table.iter_mut().enumerate() {
            for (declarer, slot) in row.iter_mut().enumerate() {
                *slot = tricks[strain][declarer].ok_or_else(|| {
                    bad(
                        TAG,
                        format!(
                            "no row for {} {}",
                            Seat::from_index(declarer as u8),
                            Strain::from_index(strain as u8)
                        ),
                    )
                })?;
            }
        }
        Ok(Some(DdTable::new(table)))
    }
}

/// The name of a table column spec such as `Result\2R` or `+Score`.
fn column_name(spec: &str) -> String {
    let spec = spec.trim().trim_start_matches(['+', '-']);
    spec.split('\\').next().unwrap_or("").trim().to_string()
}
