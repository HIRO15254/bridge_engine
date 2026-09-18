//! Text formats (`Display` / `FromStr`) for the core types.
//!
//! | Type | `Display` | `FromStr` accepts |
//! | --- | --- | --- |
//! | [`Suit`] | `C D H S` | also `♣♦♥♠`, lowercase |
//! | [`Rank`] | `2`..`9 T J Q K A` | also `10`, lowercase |
//! | [`Card`] | `SA` (suit first, PBN play style) | `SA` and `AS` (also `S10`, `10S`) |
//! | [`Holding`] | ranks descending, `AKQ`; empty for a void | any order; `-` or empty for a void; a repeated rank is harmless |
//! | [`Hand`] | PBN order spades-first `AKQ.234.AKQ.2345`; empty field for a void | `-` for a void; any rank order; partial hands allowed; duplicates rejected |
//! | [`Deal`] | `N:AKQ.234.AKQ.2345 <E> <S> <W>` clockwise from the named seat | any starting seat; validated by [`Deal::new`] |
//! | [`Shape`] | `5=4=3=1` (spades first, `=` = fixed order) | same form |
//! | [`ShapeClass`] | `5-4-3-1` | same form, any order (sorted) |
//! | [`Seat`] | `N E S W` | lowercase, full names |
//! | [`Strain`] | `C D H S NT` | `N`, lowercase, suit symbols |
//! | [`Bid`] | `1C`..`7NT` | `7N`, lowercase |
//! | [`Call`] | `Pass X XX 1C` (PBN) | `P Dbl Rdbl D R Double Redouble`, any case |
//! | [`Contract`] | `4SX` (declarer printed separately, as PBN does) | `4SXX`, `3NT`; the declarer is not part of the text and is set to North |
//! | [`Vulnerability`] | `None NS EW All` (PBN) | `Love Both - NONE`, any case |
//! | [`Auction`] | calls separated by single spaces, dealer first | no `FromStr` (use [`Auction::from_calls`]) |
//!
//! Every parser trims surrounding whitespace. Storage is clubs-first (bit layout) while text is
//! spades-first (PBN). Only this module knows about the two orders.

use core::fmt::{Display, Formatter, Result as FmtResult, Write as _};
use core::str::FromStr;

use crate::{
    Auction, Bid, Call, Card, Contract, Deal, Doubling, Hand, Holding, ParseError, Rank, Seat,
    Shape, ShapeClass, Strain, Suit, Vulnerability,
};

/// The PBN text order of the suits: spades first.
const TEXT_ORDER: [Suit; 4] = [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs];

fn suit_from_char(c: char) -> Option<Suit> {
    match c {
        'C' | 'c' | '♣' => Some(Suit::Clubs),
        'D' | 'd' | '♦' => Some(Suit::Diamonds),
        'H' | 'h' | '♥' => Some(Suit::Hearts),
        'S' | 's' | '♠' => Some(Suit::Spades),
        _ => None,
    }
}

fn rank_from_char(c: char) -> Option<Rank> {
    match c {
        '2' => Some(Rank::Two),
        '3' => Some(Rank::Three),
        '4' => Some(Rank::Four),
        '5' => Some(Rank::Five),
        '6' => Some(Rank::Six),
        '7' => Some(Rank::Seven),
        '8' => Some(Rank::Eight),
        '9' => Some(Rank::Nine),
        'T' | 't' => Some(Rank::Ten),
        'J' | 'j' => Some(Rank::Jack),
        'Q' | 'q' => Some(Rank::Queen),
        'K' | 'k' => Some(Rank::King),
        'A' | 'a' => Some(Rank::Ace),
        _ => None,
    }
}

/// Parses one suit field of a hand: ranks in any order, `-` or empty for a void.
///
/// With a known `suit`, a repeated rank is reported as [`ParseError::DuplicateCard`]; without
/// one (a bare [`Holding`]) it is simply absorbed by the set.
fn parse_holding(field: &str, suit: Option<Suit>) -> Result<Holding, ParseError> {
    let field = field.trim();
    if field == "-" {
        return Ok(Holding::EMPTY);
    }
    let mut holding = Holding::EMPTY;
    let mut chars = field.chars().peekable();
    while let Some(c) = chars.next() {
        let rank = if c == '1' && chars.peek() == Some(&'0') {
            chars.next();
            Rank::Ten
        } else {
            rank_from_char(c).ok_or(ParseError::Rank(c))?
        };
        if let (true, Some(suit)) = (holding.contains(rank), suit) {
            return Err(ParseError::DuplicateCard(Card::new(suit, rank)));
        }
        holding = holding.with(rank);
    }
    Ok(holding)
}

impl Display for Suit {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(match self {
            Suit::Clubs => "C",
            Suit::Diamonds => "D",
            Suit::Hearts => "H",
            Suit::Spades => "S",
        })
    }
}

impl FromStr for Suit {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Suit, ParseError> {
        let mut chars = s.trim().chars();
        let first = chars.next().ok_or(ParseError::Empty)?;
        match (suit_from_char(first), chars.next()) {
            (Some(suit), None) => Ok(suit),
            (Some(_), Some(extra)) => Err(ParseError::Suit(extra)),
            (None, _) => Err(ParseError::Suit(first)),
        }
    }
}

impl Display for Rank {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_char(self.to_char())
    }
}

impl FromStr for Rank {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Rank, ParseError> {
        let s = s.trim();
        if s == "10" {
            return Ok(Rank::Ten);
        }
        let mut chars = s.chars();
        let first = chars.next().ok_or(ParseError::Empty)?;
        match (rank_from_char(first), chars.next()) {
            (Some(rank), None) => Ok(rank),
            (Some(_), Some(extra)) => Err(ParseError::Rank(extra)),
            (None, _) => Err(ParseError::Rank(first)),
        }
    }
}

impl Display for Card {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}{}", self.suit().letter(), self.rank().to_char())
    }
}

impl FromStr for Card {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Card, ParseError> {
        let s = s.trim();
        let first = s.chars().next().ok_or(ParseError::Empty)?;
        if let Some(suit) = suit_from_char(first) {
            let rank: Rank = s[first.len_utf8()..].parse()?;
            return Ok(Card::new(suit, rank));
        }
        let last = s.chars().next_back().unwrap_or(first);
        match suit_from_char(last) {
            Some(suit) => {
                let rank: Rank = s[..s.len() - last.len_utf8()].parse()?;
                Ok(Card::new(suit, rank))
            }
            None => Err(ParseError::Suit(first)),
        }
    }
}

impl Display for Holding {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        for rank in self.ranks() {
            f.write_char(rank.to_char())?;
        }
        Ok(())
    }
}

impl FromStr for Holding {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Holding, ParseError> {
        parse_holding(s, None)
    }
}

impl Display for Hand {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        for (k, suit) in TEXT_ORDER.into_iter().enumerate() {
            if k > 0 {
                f.write_char('.')?;
            }
            Display::fmt(&self.holding(suit), f)?;
        }
        Ok(())
    }
}

impl FromStr for Hand {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Hand, ParseError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(ParseError::Empty);
        }
        let mut fields = s.split('.');
        let mut hand = Hand::EMPTY;
        for suit in TEXT_ORDER {
            let field = fields
                .next()
                .ok_or_else(|| ParseError::SuitCount(s.split('.').count()))?;
            hand = hand.with_holding(suit, parse_holding(field, Some(suit))?);
        }
        if fields.next().is_some() {
            return Err(ParseError::SuitCount(s.split('.').count()));
        }
        Ok(hand)
    }
}

impl Display for Deal {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(
            f,
            "N:{} {} {} {}",
            self.hand(Seat::North),
            self.hand(Seat::East),
            self.hand(Seat::South),
            self.hand(Seat::West)
        )
    }
}

impl FromStr for Deal {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Deal, ParseError> {
        let s = s.trim();
        let first_char = s.chars().next().ok_or(ParseError::Empty)?;
        let (seat_part, rest) = s.split_once(':').ok_or(ParseError::Seat(first_char))?;
        let first: Seat = seat_part.parse()?;
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if fields.len() != 4 {
            return Err(ParseError::HandCount(fields.len()));
        }
        let mut hands = [Hand::EMPTY; 4];
        let mut seen = Hand::EMPTY;
        for (k, field) in fields.iter().enumerate() {
            let hand: Hand = field.parse()?;
            if let Some(card) = seen.intersect(hand).cards().next() {
                return Err(ParseError::DuplicateCard(card));
            }
            seen = seen.union(hand);
            hands[first.offset(k as u8).index() as usize] = hand;
        }
        Ok(Deal::new(hands)?)
    }
}

/// Parses four lengths separated by `sep`, each at most 13, in the order written.
fn parse_four_lens(s: &str, sep: char) -> Option<[u8; 4]> {
    let mut parts = s.trim().split(sep);
    let mut lens = [0u8; 4];
    for slot in &mut lens {
        let n: u8 = parts.next()?.trim().parse().ok()?;
        if n > 13 {
            return None;
        }
        *slot = n;
    }
    parts.next().is_none().then_some(lens)
}

impl Display for Shape {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let l = self.lens();
        write!(f, "{}={}={}={}", l[3], l[2], l[1], l[0])
    }
}

impl FromStr for Shape {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Shape, ParseError> {
        let text = parse_four_lens(s, '=').ok_or_else(|| ParseError::Shape(s.to_string()))?;
        // Text is spades-first; storage is clubs-first.
        Ok(Shape::new(text[3], text[2], text[1], text[0]))
    }
}

impl Display for ShapeClass {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let l = self.lens();
        write!(f, "{}-{}-{}-{}", l[0], l[1], l[2], l[3])
    }
}

impl FromStr for ShapeClass {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<ShapeClass, ParseError> {
        parse_four_lens(s, '-')
            .map(ShapeClass::new)
            .ok_or_else(|| ParseError::Shape(s.to_string()))
    }
}

impl Display for Seat {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_char(self.letter())
    }
}

impl FromStr for Seat {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Seat, ParseError> {
        let s = s.trim();
        let first = s.chars().next().ok_or(ParseError::Empty)?;
        let names: [(&str, &str, Seat); 4] = [
            ("N", "NORTH", Seat::North),
            ("E", "EAST", Seat::East),
            ("S", "SOUTH", Seat::South),
            ("W", "WEST", Seat::West),
        ];
        names
            .iter()
            .find(|(short, long, _)| s.eq_ignore_ascii_case(short) || s.eq_ignore_ascii_case(long))
            .map(|(_, _, seat)| *seat)
            .ok_or(ParseError::Seat(first))
    }
}

impl Display for Strain {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(match self {
            Strain::Clubs => "C",
            Strain::Diamonds => "D",
            Strain::Hearts => "H",
            Strain::Spades => "S",
            Strain::NoTrump => "NT",
        })
    }
}

impl FromStr for Strain {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Strain, ParseError> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("NT") || s.eq_ignore_ascii_case("N") {
            return Ok(Strain::NoTrump);
        }
        s.parse::<Suit>().map(Strain::from_suit)
    }
}

impl Display for Bid {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}{}", self.level(), self.strain())
    }
}

impl FromStr for Bid {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Bid, ParseError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(ParseError::Empty);
        }
        let invalid = || ParseError::Bid(s.to_string());
        let mut chars = trimmed.chars();
        let level = chars
            .next()
            .and_then(|c| c.to_digit(10))
            .ok_or_else(invalid)? as u8;
        let strain: Strain = chars.as_str().parse().map_err(|_| invalid())?;
        Bid::new(level, strain).ok_or_else(invalid)
    }
}

impl Display for Call {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Call::Pass => f.write_str("Pass"),
            Call::Double => f.write_str("X"),
            Call::Redouble => f.write_str("XX"),
            Call::Bid(b) => Display::fmt(b, f),
        }
    }
}

impl FromStr for Call {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Call, ParseError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(ParseError::Empty);
        }
        match trimmed.to_ascii_uppercase().as_str() {
            "P" | "PASS" => Ok(Call::Pass),
            "X" | "D" | "DBL" | "DOUBLE" => Ok(Call::Double),
            "XX" | "R" | "RDBL" | "REDOUBLE" => Ok(Call::Redouble),
            _ => trimmed
                .parse::<Bid>()
                .map(Call::Bid)
                .map_err(|_| ParseError::Call(s.to_string())),
        }
    }
}

impl Display for Doubling {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(match self {
            Doubling::Undoubled => "",
            Doubling::Doubled => "X",
            Doubling::Redoubled => "XX",
        })
    }
}

impl Display for Contract {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}{}", self.bid, self.doubling)
    }
}

impl FromStr for Contract {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Contract, ParseError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(ParseError::Empty);
        }
        let bid_part = trimmed.trim_end_matches(['X', 'x']);
        let doubling = match trimmed.len() - bid_part.len() {
            0 => Doubling::Undoubled,
            1 => Doubling::Doubled,
            2 => Doubling::Redoubled,
            _ => return Err(ParseError::Call(trimmed[bid_part.len()..].to_string())),
        };
        let bid: Bid = bid_part
            .parse()
            .map_err(|_| ParseError::Bid(bid_part.to_string()))?;
        Ok(Contract {
            bid,
            declarer: Seat::North,
            doubling,
        })
    }
}

impl Display for Vulnerability {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(match self {
            Vulnerability::None => "None",
            Vulnerability::NS => "NS",
            Vulnerability::EW => "EW",
            Vulnerability::Both => "All",
        })
    }
}

impl FromStr for Vulnerability {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Vulnerability, ParseError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(ParseError::Empty);
        }
        match trimmed.to_ascii_uppercase().as_str() {
            "NONE" | "LOVE" | "-" => Ok(Vulnerability::None),
            "NS" => Ok(Vulnerability::NS),
            "EW" => Ok(Vulnerability::EW),
            "ALL" | "BOTH" => Ok(Vulnerability::Both),
            _ => Err(ParseError::Vulnerability(s.to_string())),
        }
    }
}

impl Display for Auction {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        for (i, call) in self.calls().iter().enumerate() {
            if i > 0 {
                f.write_char(' ')?;
            }
            Display::fmt(call, f)?;
        }
        Ok(())
    }
}
