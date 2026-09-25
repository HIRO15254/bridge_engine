//! PBN parsing (winnow, line oriented).
//!
//! 1. Bytes → text: UTF-8, falling back to ISO-8859-1; CRLF normalised.
//! 2. `%` in column 1 is a directive.
//! 3. `;` to end of line and `{…}` (non-nesting, multi-line) are comments.
//! 4. Games are separated by empty lines outside brace comments.
//! 5. Tag pairs `[Name "value"]` with `\\` and `\"` escapes, `#` and `##x` values; import mode
//!    allows several tags per line and tags spanning lines.
//! 6. `Auction`, `Play` and `*Table` tags own the following non-`[` lines as tokens; `[Note]`
//!    tags attach to the preceding section (notes are numbered per section).
//! 7. Auction tokens: bids, `Pass|P`, `X`, `XX`, `AP`, `-`, `*`, `+`, `=n=`, `$n`, suffixes.
//! 8. Play tokens: `SA`-style cards, `-`, `*`, `+`, notes, NAGs, suffixes.
//!
//! The same lexer serves [`parse_lenient`] and [`parse_strict`]: in strict mode every
//! construct that the lenient mode would repair or skip is the first error, and the
//! export-only rules (one left-justified tag pair per line, upper-case tokens, no suffix
//! annotations, no `+`, no tabs, the mandatory tag set first and in order) are enforced too.

use core::mem;

use bridge_core::{Bid, Call, Card, Rank, Strain, Suit};
use winnow::{
    ascii::digit1,
    combinator::{alt, delimited, not, preceded, terminated},
    prelude::*,
    token::{one_of, take_while},
};

use super::view::{Anchor, MTS};
use crate::{
    ParseError, Warning, WarningKind,
    pbn::{
        Comment, Directive, Game, GameView, PbnFile, Section, TagPair, TagValue, Token, ViewError,
        model::{SectionKind, is_section_tag, section_kind},
    },
    text,
};

/// Parses as much as possible; never fails and never panics.
pub fn parse_lenient(input: &[u8]) -> (PbnFile, Vec<Warning>) {
    let (text, bad_utf8) = text::decode(input);
    let mut lexer = Lexer::new(&text, false);
    if let Some(line) = bad_utf8 {
        lexer.warnings.push(Warning::new(
            0,
            line,
            WarningKind::Encoding,
            "input is not valid UTF-8; decoded as ISO-8859-1",
        ));
    }
    lexer.run();
    lexer
        .finish()
        .unwrap_or_else(|_| unreachable!("lenient parsing never fails"))
}

/// Parses an export-format file, failing on the first violation.
pub fn parse_strict(input: &str) -> Result<PbnFile, ParseError> {
    if let Some(offset) = input.find('\t') {
        return Err(ParseError {
            line: text::line_of(input, offset),
            message: "tab characters are not allowed in export format".to_string(),
        });
    }
    let text = text::normalize_newlines(input);
    let mut lexer = Lexer::new(&text, true);
    lexer.run();
    lexer.finish().map(|(file, _)| file)
}

struct Lexer<'a> {
    text: &'a str,
    pos: usize,
    line: u32,
    strict: bool,
    /// The first violation in strict mode.
    error: Option<ParseError>,
    file: PbnFile,
    warnings: Vec<Warning>,
    game: Game,
    /// Line of each section of `game`.
    section_lines: Vec<u32>,
    /// The section receiving token lines.
    open_section: Option<usize>,
    /// The section receiving `[Note]` tags.
    last_section: Option<usize>,
    /// The last successfully interpreted game, for `#` inheritance during validation.
    previous: Option<GameView>,
}

impl<'a> Lexer<'a> {
    fn new(text: &'a str, strict: bool) -> Lexer<'a> {
        Lexer {
            text,
            pos: 0,
            line: 1,
            strict,
            error: None,
            file: PbnFile::default(),
            warnings: Vec::new(),
            game: Game::default(),
            section_lines: Vec::new(),
            open_section: None,
            last_section: None,
            previous: None,
        }
    }

    // --- cursor -----------------------------------------------------------------------

    fn rest(&self) -> &'a str {
        &self.text[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn at_line_start(&self) -> bool {
        self.pos == 0 || self.text.as_bytes()[self.pos - 1] == b'\n'
    }

    /// The rest of the current line, without the newline.
    fn current_line(&self) -> &'a str {
        let rest = self.rest();
        &rest[..rest.find('\n').unwrap_or(rest.len())]
    }

    fn advance(&mut self, n: usize) {
        let skipped = &self.text[self.pos..self.pos + n];
        self.line += skipped.bytes().filter(|&b| b == b'\n').count() as u32;
        self.pos += n;
    }

    fn skip_to_eol(&mut self) {
        self.pos += self.current_line().len();
    }

    fn consume_line(&mut self) {
        self.skip_to_eol();
        if self.peek() == Some('\n') {
            self.advance(1);
        }
    }

    fn skip_inline_ws(&mut self) {
        let rest = self.rest();
        let trimmed = rest.trim_start_matches([' ', '\t', '\u{0b}', '\u{0c}']);
        self.pos += rest.len() - trimmed.len();
    }

    // --- diagnostics ------------------------------------------------------------------

    /// Records a repaired or skipped construct: a warning in lenient mode, the error in
    /// strict mode.
    fn warn(&mut self, line: u32, kind: WarningKind, message: impl Into<String>) {
        if self.strict {
            if self.error.is_none() {
                self.error = Some(ParseError {
                    line,
                    message: message.into(),
                });
            }
        } else {
            self.warnings
                .push(Warning::new(self.file.games.len(), line, kind, message));
        }
    }

    /// A rule of the export format only.
    fn strict_only(&mut self, line: u32, message: impl Into<String>) {
        if self.strict {
            self.warn(line, WarningKind::Other, message);
        }
    }

    // --- main loop --------------------------------------------------------------------

    fn run(&mut self) {
        while self.pos < self.text.len() {
            if self.error.is_some() {
                return;
            }
            if self.at_line_start() {
                let line_text = self.current_line();
                if line_text.chars().all(char::is_whitespace) {
                    if !line_text.is_empty() {
                        self.strict_only(self.line, "a game separator must be an empty line");
                    }
                    self.consume_line();
                    self.game_separator();
                    continue;
                }
                if line_text.starts_with('%') {
                    self.directive();
                    continue;
                }
            }
            self.skip_inline_ws();
            match self.peek() {
                None => break,
                Some('\n') => self.advance(1),
                Some(';') => self.line_comment(),
                Some('{') => self.brace_comment(),
                Some('[') => self.tag_pair(),
                Some(_) => self.token_line(),
            }
        }
    }

    fn game_separator(&mut self) {
        if has_content(&self.game) {
            self.flush_game();
        }
    }

    fn directive(&mut self) {
        let line = self.line;
        let body = &self.current_line()[1..];
        let trimmed = body.trim();
        let directive = match trimmed.strip_prefix("PBN") {
            Some(rest) if rest.starts_with(char::is_whitespace) => match parse_version(rest) {
                Some((major, minor)) => Directive::Version(major, minor),
                None => {
                    self.warn(
                        line,
                        WarningKind::MalformedTag,
                        format!("malformed version directive {trimmed:?}; kept verbatim"),
                    );
                    Directive::Other(body.trim_end().to_string())
                }
            },
            _ if trimmed == "EXPORT" => Directive::Export,
            _ => Directive::Other(body.trim_end().to_string()),
        };
        self.file.directives.push(directive);
        self.consume_line();
    }

    fn line_comment(&mut self) {
        let line = self.line;
        let text = self.current_line()[1..].to_string();
        self.skip_to_eol();
        self.push_comment(text, line);
    }

    fn brace_comment(&mut self) {
        let line = self.line;
        let rest = self.rest();
        match rest[1..].find('}') {
            Some(i) => {
                let text = rest[1..1 + i].to_string();
                self.advance(i + 2);
                self.push_comment(text, line);
            }
            None => {
                self.warn(
                    line,
                    WarningKind::Truncated,
                    "unterminated '{' comment: the rest of the input is commentary",
                );
                let text = rest[1..].to_string();
                self.advance(rest.len());
                self.push_comment(text, line);
            }
        }
    }

    fn push_comment(&mut self, text: String, line: u32) {
        let after_tag = self.game.tags.len().checked_sub(1);
        self.game.commentary.push(Comment {
            text,
            after_tag,
            line,
        });
    }

    // --- tag pairs --------------------------------------------------------------------

    fn tag_pair(&mut self) {
        let line = self.line;
        let left_justified = self.at_line_start();
        let mut s = self.rest();
        let name = match tag_head.parse_next(&mut s) {
            Ok(name) => name.to_string(),
            Err(_) => {
                self.warn(
                    line,
                    WarningKind::MalformedTag,
                    format!(
                        "malformed tag pair skipped: {:?}",
                        self.current_line().trim()
                    ),
                );
                self.skip_to_eol();
                return;
            }
        };
        let head_len = self.rest().len() - s.len();
        if self.strict {
            if !left_justified {
                self.warn(
                    line,
                    WarningKind::Other,
                    "a tag pair must begin in column 1 on a line of its own",
                );
            }
            if self.rest()[..head_len] != format!("[{name} \"") {
                self.warn(
                    line,
                    WarningKind::Other,
                    "a tag pair must be written as [Name \"value\"] with single spaces",
                );
            }
        }
        self.advance(head_len);
        let value = self.tag_string(line);
        self.skip_inline_ws();
        if self.peek() == Some(']') {
            self.advance(1);
        } else {
            self.warn(
                line,
                WarningKind::MalformedTag,
                format!("tag {name}: missing ']' (tag kept)"),
            );
        }
        if self.strict && !self.current_line().trim().is_empty() {
            self.warn(
                self.line,
                WarningKind::Other,
                "nothing may follow a tag pair on its line",
            );
        }
        self.add_tag(name, value, line);
    }

    /// Reads a string body after its opening quote, resolving `\"` and `\\`.
    ///
    /// A backslash before any other character is kept literally (the PBN reference files
    /// write `Result\2R`). Import format lets a string span lines; the string is cut at
    /// the end of a line when the next line is blank or starts a tag or directive.
    fn tag_string(&mut self, tag_line: u32) -> String {
        let mut value = String::new();
        let rest = self.rest();
        let mut chars = rest.char_indices();
        loop {
            let Some((i, c)) = chars.next() else {
                self.warn(
                    tag_line,
                    WarningKind::MalformedTag,
                    "unterminated string at end of input",
                );
                self.advance(rest.len());
                return value;
            };
            match c {
                '"' => {
                    self.advance(i + 1);
                    return value;
                }
                '\\' => match chars.next() {
                    Some((_, '"')) => value.push('"'),
                    Some((_, '\\')) => value.push('\\'),
                    Some((_, other)) => {
                        value.push('\\');
                        value.push(other);
                    }
                    None => value.push('\\'),
                },
                '\n' => {
                    let next_line = rest[i + 1..].lines().next().unwrap_or("");
                    if self.strict
                        || next_line.trim().is_empty()
                        || next_line.starts_with(['[', '%'])
                    {
                        self.warn(
                            tag_line,
                            WarningKind::MalformedTag,
                            "unterminated string (closed at end of line)",
                        );
                        self.advance(i);
                        return value;
                    }
                    value.push('\n');
                }
                _ => value.push(c),
            }
        }
    }

    fn add_tag(&mut self, name: String, raw: String, line: u32) {
        let value = if raw == "#" {
            TagValue::Inherited
        } else if let Some(text) = raw.strip_prefix("##") {
            TagValue::Default(text.to_string())
        } else {
            TagValue::Str(raw)
        };
        if name == "Note" {
            match (self.last_section, parse_note(&value)) {
                (Some(idx), Some((n, text))) => {
                    if !(1..=32).contains(&n) {
                        self.warn(
                            line,
                            WarningKind::NoteReference,
                            format!("note index {n} is outside 1..=32"),
                        );
                    }
                    self.game.sections[idx].notes.push((n, text));
                    return;
                }
                (Some(_), None) => self.warn(
                    line,
                    WarningKind::NoteReference,
                    "Note value is not \"n:text\"; kept as an ordinary tag",
                ),
                (None, _) => self.warn(
                    line,
                    WarningKind::NoteReference,
                    "Note without a preceding section; kept as an ordinary tag",
                ),
            }
        }
        if is_section_tag(&name) {
            let arg = match &value {
                TagValue::Str(s) => s.clone(),
                TagValue::Inherited => "#".to_string(),
                TagValue::Default(t) => format!("##{t}"),
            };
            if !matches!(value, TagValue::Str(_)) {
                self.strict_only(line, format!("tag {name} may not be inherited"));
            }
            self.game.sections.push(Section {
                tag: name,
                arg,
                tokens: Vec::new(),
                notes: Vec::new(),
            });
            self.section_lines.push(line);
            let idx = self.game.sections.len() - 1;
            self.open_section = Some(idx);
            self.last_section = Some(idx);
            return;
        }
        self.open_section = None;
        if self.game.tags.iter().any(|t| t.name == name) {
            self.warn(
                line,
                WarningKind::Other,
                format!("duplicate tag {name} (both kept; `get` returns the first)"),
            );
        }
        self.game.tags.push(TagPair { name, value, line });
    }

    // --- sections ---------------------------------------------------------------------

    fn token_line(&mut self) {
        let line = self.line;
        let Some(idx) = self.open_section else {
            let text = self.current_line().trim().to_string();
            self.warn(
                line,
                WarningKind::MalformedTag,
                format!("text outside a section skipped: {text:?}"),
            );
            self.skip_to_eol();
            return;
        };
        let text = self.current_line();
        let end = text.find([';', '{', '[']).unwrap_or(text.len());
        let run = &text[..end];
        self.pos += end;
        match section_kind(&self.game.sections[idx].tag) {
            SectionKind::Table => {
                let row = run.trim();
                if !row.is_empty() {
                    self.game.sections[idx]
                        .tokens
                        .push(Token::Raw(row.to_string()));
                }
            }
            SectionKind::Auction => {
                for word in run.split_whitespace() {
                    self.word(idx, word, line, SectionKind::Auction);
                }
            }
            SectionKind::Play => {
                for word in run.split_whitespace() {
                    self.word(idx, word, line, SectionKind::Play);
                }
            }
        }
    }

    /// Tokenises one whitespace-delimited word; annotations may be attached (`1S!`, `SA=1=`).
    fn word(&mut self, idx: usize, word: &str, line: u32, kind: SectionKind) {
        let upper = word.to_ascii_uppercase();
        if self.strict && upper != word && word != "Pass" {
            self.warn(
                line,
                WarningKind::Other,
                format!("token {word:?} must be upper case"),
            );
            return;
        }
        let mut s: &str = &upper;
        while !s.is_empty() {
            let remaining = s.len();
            let lexeme = match kind {
                SectionKind::Auction => auction_token.parse_next(&mut s),
                _ => play_token.parse_next(&mut s),
            };
            match lexeme {
                Ok(Lexeme::AllPass) => self.expand_all_pass(idx),
                Ok(Lexeme::Tok(token)) => self.push_token(idx, token, line),
                Err(_) => {
                    // `to_ascii_uppercase` preserves byte offsets.
                    let raw = &word[word.len() - remaining..];
                    self.warn(
                        line,
                        WarningKind::MalformedToken,
                        format!(
                            "unrecognised token {raw:?} in {} section",
                            self.game.sections[idx].tag
                        ),
                    );
                    self.game.sections[idx]
                        .tokens
                        .push(Token::Raw(raw.to_string()));
                    return;
                }
            }
        }
    }

    fn push_token(&mut self, idx: usize, token: Token, line: u32) {
        match &token {
            Token::Suffix(s) => {
                self.strict_only(line, format!("suffix {s} must be written as a NAG"));
            }
            Token::Continuation => self.strict_only(line, "'+' is import format only"),
            _ => {}
        }
        self.game.sections[idx].tokens.push(token);
    }

    /// `AP`: as many passes as the auction needs to be complete.
    fn expand_all_pass(&mut self, idx: usize) {
        let tokens = &mut self.game.sections[idx].tokens;
        let calls: Vec<Call> = tokens
            .iter()
            .filter_map(|t| match t {
                Token::Call(c) => Some(*c),
                _ => None,
            })
            .collect();
        let trailing = calls.iter().rev().take_while(|c| **c == Call::Pass).count();
        let needed = if trailing == calls.len() {
            4usize.saturating_sub(calls.len())
        } else {
            3usize.saturating_sub(trailing)
        };
        tokens.extend(core::iter::repeat_n(Token::Call(Call::Pass), needed));
    }

    // --- games ------------------------------------------------------------------------

    fn flush_game(&mut self) {
        let game = mem::take(&mut self.game);
        let section_lines = mem::take(&mut self.section_lines);
        self.open_section = None;
        self.last_section = None;
        if self.strict {
            self.check_mandatory_tags(&game, &section_lines);
        } else {
            self.validate(&game, &section_lines);
        }
        self.file.games.push(game);
    }

    /// Interprets the game and turns everything it skipped or repaired into warnings.
    fn validate(&mut self, game: &Game, section_lines: &[u32]) {
        let index = self.file.games.len();
        let first_line = game
            .tags
            .first()
            .map(|t| t.line)
            .or(section_lines.first().copied())
            .unwrap_or(self.line);
        let line_of = |anchor: Anchor| match anchor {
            Anchor::Tag(name) => game
                .tags
                .iter()
                .find(|t| t.name == name)
                .map(|t| t.line)
                .or_else(|| {
                    game.sections
                        .iter()
                        .position(|s| s.tag == name)
                        .and_then(|i| section_lines.get(i).copied())
                })
                .unwrap_or(first_line),
            Anchor::Section(i) => section_lines.get(i).copied().unwrap_or(first_line),
        };
        let (result, notes) = game.interpret(self.previous.as_ref());
        for note in notes {
            let line = line_of(note.anchor);
            self.warnings
                .push(Warning::new(index, line, note.kind, note.message));
        }
        match result {
            Ok(view) => self.previous = Some(view),
            Err(error) => {
                let (kind, anchor) = match &error {
                    ViewError::Auction(_) => (WarningKind::BadAuction, Anchor::Tag("Auction")),
                    ViewError::Play(_) => (WarningKind::BadPlay, Anchor::Tag("Play")),
                    ViewError::BadTag { tag, .. } => (WarningKind::Other, tag_anchor(tag)),
                    ViewError::NothingToInherit(tag) => (WarningKind::Other, tag_anchor(tag)),
                };
                let line = line_of(anchor);
                self.warnings
                    .push(Warning::new(index, line, kind, error.to_string()));
            }
        }
    }

    /// Export format: the fifteen mandatory tags first, in order, before any section.
    fn check_mandatory_tags(&mut self, game: &Game, section_lines: &[u32]) {
        let last_line = game
            .tags
            .last()
            .map(|t| t.line)
            .or(section_lines.last().copied())
            .unwrap_or(self.line);
        for (k, name) in MTS.iter().enumerate() {
            match game.tags.get(k) {
                Some(tag) if tag.name == *name => {}
                Some(tag) => {
                    self.warn(
                        tag.line,
                        WarningKind::Other,
                        format!("expected mandatory tag {name}, found {}", tag.name),
                    );
                    return;
                }
                None => {
                    self.warn(
                        last_line,
                        WarningKind::Other,
                        format!("missing mandatory tag {name}"),
                    );
                    return;
                }
            }
        }
        if let (Some(&section_line), Some(tag)) = (section_lines.first(), game.tags.get(14))
            && section_line < tag.line
        {
            self.warn(
                section_line,
                WarningKind::Other,
                "a section may not precede the mandatory tags",
            );
        }
    }

    fn finish(mut self) -> Result<(PbnFile, Vec<Warning>), ParseError> {
        if has_content(&self.game) {
            self.flush_game();
        } else if !self.game.commentary.is_empty() {
            let trailing = mem::take(&mut self.game.commentary);
            match self.file.games.last_mut() {
                Some(last) => {
                    let after_tag = last.tags.len().checked_sub(1);
                    for mut comment in trailing {
                        comment.after_tag = after_tag;
                        last.commentary.push(comment);
                    }
                }
                None => {
                    self.game.commentary = trailing;
                    let game = mem::take(&mut self.game);
                    self.file.games.push(game);
                }
            }
        }
        match self.error.take() {
            Some(error) => Err(error),
            None => Ok((self.file, self.warnings)),
        }
    }
}

fn has_content(game: &Game) -> bool {
    !game.tags.is_empty() || !game.sections.is_empty()
}

/// Anchors a view error's tag name to a `'static` tag name.
fn tag_anchor(tag: &str) -> Anchor {
    for name in MTS {
        if name == tag {
            return Anchor::Tag(name);
        }
    }
    match tag {
        "Auction" => Anchor::Tag("Auction"),
        "Play" => Anchor::Tag("Play"),
        "OptimumResultTable" => Anchor::Tag("OptimumResultTable"),
        _ => Anchor::Tag("Event"),
    }
}

fn parse_version(s: &str) -> Option<(u8, u8)> {
    let (major, minor) = s.trim().split_once('.')?;
    Some((major.trim().parse().ok()?, minor.trim().parse().ok()?))
}

fn parse_note(value: &TagValue) -> Option<(u8, String)> {
    let TagValue::Str(s) = value else { return None };
    let (n, text) = s.split_once(':')?;
    Some((n.trim().parse().ok()?, text.to_string()))
}

// --- winnow grammar --------------------------------------------------------------------

/// `[` ws `Name` ws `"` → the name.
fn tag_head<'s>(i: &mut &'s str) -> ModalResult<&'s str> {
    delimited(('[', ws), tag_name, (ws, '"')).parse_next(i)
}

fn ws(i: &mut &str) -> ModalResult<()> {
    take_while(0.., [' ', '\t', '\n']).void().parse_next(i)
}

fn tag_name<'s>(i: &mut &'s str) -> ModalResult<&'s str> {
    (
        one_of(|c: char| c.is_ascii_alphabetic()),
        take_while(0.., |c: char| c.is_ascii_alphanumeric() || c == '_'),
    )
        .take()
        .parse_next(i)
}

#[derive(Clone, Debug)]
enum Lexeme {
    Tok(Token),
    AllPass,
}

fn note_ref(i: &mut &str) -> ModalResult<Token> {
    delimited('=', digit1, '=')
        .verify_map(|d: &str| d.parse::<u8>().ok().map(Token::NoteRef))
        .parse_next(i)
}

fn nag(i: &mut &str) -> ModalResult<Token> {
    preceded('$', digit1)
        .verify_map(|d: &str| d.parse::<u8>().ok().map(Token::Nag))
        .parse_next(i)
}

fn suffix(i: &mut &str) -> ModalResult<Token> {
    alt(("!!", "??", "!?", "?!", "!", "?"))
        .map(|s: &str| Token::Suffix(s.to_string()))
        .parse_next(i)
}

fn marker(i: &mut &str) -> ModalResult<Token> {
    alt((
        '-'.value(Token::Unknown),
        '*'.value(Token::Terminator),
        '+'.value(Token::Continuation),
    ))
    .parse_next(i)
}

/// A call or card must not be immediately followed by another letter or digit.
fn boundary(i: &mut &str) -> ModalResult<()> {
    not(one_of(|c: char| c.is_ascii_alphanumeric())).parse_next(i)
}

fn bid(i: &mut &str) -> ModalResult<Bid> {
    (
        one_of('1'..='7'),
        alt((
            "NT".value(Strain::NoTrump),
            'C'.value(Strain::Clubs),
            'D'.value(Strain::Diamonds),
            'H'.value(Strain::Hearts),
            'S'.value(Strain::Spades),
        )),
    )
        .verify_map(|(level, strain): (char, Strain)| Bid::new(level as u8 - b'0', strain))
        .parse_next(i)
}

fn call(i: &mut &str) -> ModalResult<Lexeme> {
    terminated(
        alt((
            "AP".value(Lexeme::AllPass),
            "PASS".value(Lexeme::Tok(Token::Call(Call::Pass))),
            "P".value(Lexeme::Tok(Token::Call(Call::Pass))),
            "XX".value(Lexeme::Tok(Token::Call(Call::Redouble))),
            "X".value(Lexeme::Tok(Token::Call(Call::Double))),
            bid.map(|b| Lexeme::Tok(Token::Call(Call::Bid(b)))),
        )),
        boundary,
    )
    .parse_next(i)
}

fn auction_token(i: &mut &str) -> ModalResult<Lexeme> {
    alt((
        note_ref.map(Lexeme::Tok),
        nag.map(Lexeme::Tok),
        suffix.map(Lexeme::Tok),
        marker.map(Lexeme::Tok),
        call,
    ))
    .parse_next(i)
}

fn suit_of(c: char) -> Option<Suit> {
    Suit::ALL.into_iter().find(|s| s.letter() == c)
}

fn rank_of(c: char) -> Option<Rank> {
    Rank::ALL.into_iter().find(|r| r.to_char() == c)
}

fn card(i: &mut &str) -> ModalResult<Token> {
    terminated(
        (
            one_of(['S', 'H', 'D', 'C']).verify_map(suit_of),
            alt((
                "10".value(Rank::Ten),
                one_of(|c: char| rank_of(c).is_some()).verify_map(rank_of),
            )),
        ),
        boundary,
    )
    .map(|(suit, rank)| Token::Card(Card::new(suit, rank)))
    .parse_next(i)
}

fn play_token(i: &mut &str) -> ModalResult<Lexeme> {
    alt((note_ref, nag, suffix, marker, card))
        .map(Lexeme::Tok)
        .parse_next(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(kind: SectionKind, text: &str) -> Vec<Token> {
        let head = match kind {
            SectionKind::Auction => "[Auction \"N\"]\n",
            SectionKind::Play => "[Play \"W\"]\n",
            SectionKind::Table => "[XTable \"A;B\"]\n",
        };
        let (file, _) = parse_lenient(format!("{head}{text}\n").as_bytes());
        file.games[0].sections[0].tokens.clone()
    }

    #[test]
    fn auction_tokens() {
        let bid = |s: &str| Token::Call(s.parse().unwrap());
        assert_eq!(
            tokens(SectionKind::Auction, "1S=1= pass X! XX $3 - * + 2nt"),
            vec![
                bid("1S"),
                Token::NoteRef(1),
                bid("Pass"),
                bid("X"),
                Token::Suffix("!".into()),
                bid("XX"),
                Token::Nag(3),
                Token::Unknown,
                Token::Terminator,
                Token::Continuation,
                bid("2NT"),
            ]
        );
        // `1N` is not a bid; `PX` is not a call.
        assert_eq!(
            tokens(SectionKind::Auction, "1N PX"),
            vec![Token::Raw("1N".into()), Token::Raw("PX".into())]
        );
    }

    #[test]
    fn all_pass_expands_to_completion() {
        let pass = Token::Call(Call::Pass);
        assert_eq!(tokens(SectionKind::Auction, "AP"), vec![pass.clone(); 4]);
        let t = tokens(SectionKind::Auction, "1S Pass AP");
        assert_eq!(t.len(), 4);
        assert_eq!(&t[2..], &[pass.clone(), pass.clone()]);
        let t = tokens(SectionKind::Auction, "Pass 1S X AP");
        assert_eq!(t.len(), 6);
        assert_eq!(tokens(SectionKind::Auction, "Pass Pass Pass AP").len(), 4);
    }

    #[test]
    fn play_tokens() {
        let card = |s: &str| Token::Card(s.parse().unwrap());
        assert_eq!(
            tokens(SectionKind::Play, "SA h10 -  C2=2= *"),
            vec![
                card("SA"),
                card("HT"),
                Token::Unknown,
                card("C2"),
                Token::NoteRef(2),
                Token::Terminator,
            ]
        );
        assert_eq!(
            tokens(SectionKind::Play, "SAK"),
            vec![Token::Raw("SAK".into())]
        );
    }

    #[test]
    fn table_rows_are_raw() {
        assert_eq!(
            tokens(SectionKind::Table, "N  C  9\nE NT 10"),
            vec![Token::Raw("N  C  9".into()), Token::Raw("E NT 10".into())]
        );
    }
}
