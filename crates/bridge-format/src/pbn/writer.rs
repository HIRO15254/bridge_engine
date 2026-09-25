//! PBN writer.

use bridge_core::{Call, Contract, Seat, Side, Vulnerability};

use super::view::MTS;
use crate::{
    deal_string,
    pbn::{
        Comment, Directive, Game, PbnFile, Section, TagPair, TagValue, Token,
        model::{SectionKind, is_section_tag, section_kind},
    },
};

/// Writer options.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WriteOptions {
    /// Export format: `% PBN 2.1`, `% EXPORT`, mandatory tags first in fixed order, one tag per
    /// line, CRLF, uppercase, ranks descending, `=n=` before `$n`, suffixes as NAGs.
    pub export: bool,
}

/// Serialises a file.
///
/// With `export`, the mandatory tags come first in the standard order (`?` when missing),
/// the other tags follow sorted by name with their sections, `Deal` is rewritten with the
/// dealer first and ranks descending, the last three passes of a complete auction become
/// `AP`, suffixes become NAGs, `+` and unrecognised tokens are dropped, and lines end with
/// CRLF. Otherwise tags, values and comments are written as they were read, with LF.
pub fn write(file: &PbnFile, opts: WriteOptions) -> String {
    let nl = if opts.export { "\r\n" } else { "\n" };
    let mut out = String::new();
    if opts.export {
        line(&mut out, "% PBN 2.1", nl);
        line(&mut out, "% EXPORT", nl);
        for directive in &file.directives {
            if let Directive::Other(text) = directive {
                line(&mut out, &format!("%{}", single_line(text)), nl);
            }
        }
    } else {
        for directive in &file.directives {
            let text = match directive {
                Directive::Version(major, minor) => format!("% PBN {major}.{minor}"),
                Directive::Export => "% EXPORT".to_string(),
                Directive::Other(text) => format!("%{text}"),
            };
            line(&mut out, &text, nl);
        }
    }
    for (k, game) in file.games.iter().enumerate() {
        if k > 0 {
            out.push_str(nl);
        }
        if opts.export {
            export_game(&mut out, game, nl);
        } else {
            verbatim_game(&mut out, game, nl);
        }
    }
    out
}

fn line(out: &mut String, text: &str, nl: &str) {
    out.push_str(text);
    out.push_str(nl);
}

fn single_line(text: &str) -> String {
    text.replace(['\n', '\r', '\t'], " ")
}

/// `\` → `\\`, `"` → `\"`.
fn escape(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => s.push_str("\\\\"),
            '"' => s.push_str("\\\""),
            _ => s.push(c),
        }
    }
    s
}

/// Table column specs are written with bare backslashes (`Result\2R`), as the PBN reference
/// files do; every other value gets the standard escapes.
fn escape_tag(name: &str, text: &str) -> String {
    if is_section_tag(name) && section_kind(name) == SectionKind::Table {
        text.replace('"', "\\\"")
    } else {
        escape(text)
    }
}

fn raw_value(name: &str, value: &TagValue) -> String {
    match value {
        TagValue::Str(s) => escape_tag(name, s),
        TagValue::Inherited => "#".to_string(),
        TagValue::Default(t) => format!("##{}", escape_tag(name, t)),
    }
}

fn comment(out: &mut String, c: &Comment, nl: &str) {
    if c.text.contains('}') {
        for l in c.text.lines() {
            line(out, &format!(";{l}"), nl);
        }
    } else {
        line(out, &format!("{{{}}}", c.text.replace('\n', nl)), nl);
    }
}

fn verbatim_game(out: &mut String, game: &Game, nl: &str) {
    for c in game.commentary.iter().filter(|c| c.after_tag.is_none()) {
        comment(out, c, nl);
    }
    for (i, tag) in game.tags.iter().enumerate() {
        line(
            out,
            &format!("[{} \"{}\"]", tag.name, raw_value(&tag.name, &tag.value)),
            nl,
        );
        for c in game.commentary.iter().filter(|c| c.after_tag == Some(i)) {
            comment(out, c, nl);
        }
    }
    for c in game
        .commentary
        .iter()
        .filter(|c| c.after_tag.is_some_and(|i| i >= game.tags.len()))
    {
        comment(out, c, nl);
    }
    for section in &game.sections {
        write_section(out, section, nl, false);
    }
}

fn str_value(value: &TagValue) -> Option<&str> {
    match value {
        TagValue::Str(s) | TagValue::Default(s) => Some(s.trim()),
        TagValue::Inherited => None,
    }
}

fn export_game(out: &mut String, game: &Game, nl: &str) {
    let dealer: Option<Seat> = game
        .get("Dealer")
        .and_then(str_value)
        .and_then(|s| s.parse().ok());
    let declarer: Option<Seat> = game
        .get("Declarer")
        .and_then(str_value)
        .and_then(|s| s.trim_start_matches('^').parse().ok());
    let context = Context { dealer, declarer };
    let comments_after = |out: &mut String, index: Option<usize>| {
        for c in game.commentary.iter().filter(|c| c.after_tag == index) {
            comment(out, c, nl);
        }
    };
    comments_after(out, None);
    let mut written: Vec<usize> = Vec::new();
    for name in MTS {
        match game.tags.iter().position(|t| t.name == name) {
            Some(i) => {
                let value = export_value(name, &game.tags[i].value, &context);
                line(out, &format!("[{name} \"{value}\"]"), nl);
                written.push(i);
                comments_after(out, Some(i));
            }
            None => line(out, &format!("[{name} \"?\"]"), nl),
        }
    }
    enum Entry<'a> {
        Tag(usize, &'a TagPair),
        Section(&'a Section),
    }
    let mut entries: Vec<(&str, Entry<'_>)> = Vec::new();
    let mut seen: Vec<&str> = MTS.to_vec();
    for (i, tag) in game.tags.iter().enumerate() {
        if tag.name == "Note" || seen.contains(&tag.name.as_str()) {
            continue;
        }
        seen.push(&tag.name);
        entries.push((&tag.name, Entry::Tag(i, tag)));
    }
    for section in &game.sections {
        entries.push((&section.tag, Entry::Section(section)));
    }
    entries.sort_by(|a, b| a.0.cmp(b.0));
    for (name, entry) in entries {
        match entry {
            Entry::Tag(i, tag) => {
                let value = export_value(name, &tag.value, &context);
                line(out, &format!("[{name} \"{value}\"]"), nl);
                written.push(i);
                comments_after(out, Some(i));
            }
            Entry::Section(section) => write_section(out, section, nl, true),
        }
    }
    // Comments anchored to tags that were not written (duplicates, orphan notes).
    for c in game
        .commentary
        .iter()
        .filter(|c| c.after_tag.is_some_and(|i| !written.contains(&i)))
    {
        comment(out, c, nl);
    }
}

struct Context {
    dealer: Option<Seat>,
    declarer: Option<Seat>,
}

/// A tag value in export form: canonical spelling where the value is understood, escaped and
/// on one line.
fn export_value(name: &str, value: &TagValue, context: &Context) -> String {
    let text = match value {
        TagValue::Inherited => return if name == "Deal" { "?" } else { "#" }.to_string(),
        TagValue::Default(t) => return format!("##{}", single_line(&escape(t))),
        TagValue::Str(s) => s.trim(),
    };
    let canonical = match name {
        "Deal" => deal_string::parse_lenient(text).ok().map(|(deal, _)| {
            let first = context
                .dealer
                .or_else(|| text.split(':').next().and_then(|s| s.parse().ok()))
                .unwrap_or(Seat::North);
            deal_string::write_partial(&deal, first)
        }),
        "Vulnerable" => text.parse::<Vulnerability>().ok().map(|v| v.to_string()),
        "Dealer" => text.parse::<Seat>().ok().map(|s| s.to_string()),
        "Declarer" => {
            let (caret, seat) = match text.strip_prefix('^') {
                Some(rest) => ("^", rest),
                None => ("", text),
            };
            seat.parse::<Seat>().ok().map(|s| format!("{caret}{s}"))
        }
        "Contract" => {
            if text.eq_ignore_ascii_case("Pass") {
                Some("Pass".to_string())
            } else {
                text.parse::<Contract>().ok().map(|c| c.to_string())
            }
        }
        "Result" => export_result(text, context.declarer),
        _ => None,
    };
    single_line(&escape(&canonical.unwrap_or_else(|| text.to_string())))
}

/// `NS n` / `EW n` → declarer's tricks when the declarer is known.
fn export_result(text: &str, declarer: Option<Seat>) -> Option<String> {
    let (caret, body) = match text.strip_prefix('^') {
        Some(rest) => ("^", rest.trim()),
        None => ("", text),
    };
    let words: Vec<&str> = body.split_whitespace().collect();
    let [side, n, ..] = words.as_slice() else {
        return None;
    };
    let n: u8 = n.parse().ok().filter(|n| *n <= 13)?;
    let side = if side.eq_ignore_ascii_case("NS") {
        Side::NS
    } else if side.eq_ignore_ascii_case("EW") {
        Side::EW
    } else {
        return None;
    };
    let declarer = declarer?;
    let tricks = if declarer.side() == side { n } else { 13 - n };
    Some(format!("{caret}{tricks}"))
}

fn write_section(out: &mut String, section: &Section, nl: &str, export: bool) {
    let kind = section_kind(&section.tag);
    let arg = match (export, kind) {
        (true, SectionKind::Auction | SectionKind::Play) => section
            .arg
            .trim()
            .parse::<Seat>()
            .map_or_else(|_| escape(&section.arg), |s| s.to_string()),
        _ => escape_tag(&section.tag, &section.arg),
    };
    line(out, &format!("[{} \"{}\"]", section.tag, arg), nl);
    match kind {
        SectionKind::Table => {
            for token in &section.tokens {
                match token {
                    Token::Raw(row) => line(out, row.trim_end(), nl),
                    other => line(out, &word_of(other, kind, export).0, nl),
                }
            }
        }
        SectionKind::Auction | SectionKind::Play => {
            write_words(out, &words(&section.tokens, kind, export), nl);
        }
    }
    for (n, text) in &section.notes {
        let text = if export {
            single_line(&escape(text))
        } else {
            escape(text)
        };
        line(out, &format!("[Note \"{n}:{text}\"]"), nl);
    }
}

/// A token as a word and whether it occupies one of the four columns of a line.
fn word_of(token: &Token, kind: SectionKind, export: bool) -> (String, bool) {
    match token {
        Token::Call(Call::Pass) => ("Pass".to_string(), true),
        Token::Call(call) => (call.to_string(), true),
        Token::Card(card) => (card.to_string(), true),
        Token::Unknown => ("-".to_string(), true),
        Token::Continuation => ("+".to_string(), true),
        Token::Terminator => ("*".to_string(), false),
        Token::NoteRef(n) => (format!("={n}="), false),
        Token::Nag(n) => (format!("${n}"), false),
        Token::Suffix(s) if export => (format!("${}", nag_of_suffix(s, kind)), false),
        Token::Suffix(s) => (s.clone(), false),
        Token::Raw(s) => (s.clone(), false),
    }
}

/// The NAG for a suffix: `$1..$6` for calls, `$7..$12` for cards.
fn nag_of_suffix(suffix: &str, kind: SectionKind) -> u8 {
    let base = match suffix {
        "!" => 1,
        "?" => 2,
        "!!" => 3,
        "??" => 4,
        "!?" => 5,
        _ => 6,
    };
    if kind == SectionKind::Play {
        base + 6
    } else {
        base
    }
}

/// The index of the first of three trailing passes that can be written as `AP`: the calls
/// before them must be a non-pass call last (so a reader expands `AP` to exactly three
/// passes) and nothing but those passes, `*`, and tokens export drops may follow.
fn ap_fold_point(tokens: &[Token]) -> Option<usize> {
    let calls: Vec<(usize, Call)> = tokens
        .iter()
        .enumerate()
        .filter_map(|(i, t)| match t {
            Token::Call(c) => Some((i, *c)),
            _ => None,
        })
        .collect();
    let n = calls.len();
    if n < 4 || calls[n - 3..].iter().any(|(_, c)| *c != Call::Pass) || calls[n - 4].1 == Call::Pass
    {
        return None;
    }
    let start = calls[n - 3].0;
    let tail = &tokens[start..];
    let clean = tail.iter().all(|t| {
        matches!(
            t,
            Token::Call(Call::Pass) | Token::Terminator | Token::Continuation | Token::Raw(_)
        )
    }) && tail.iter().filter(|t| matches!(t, Token::Call(_))).count() == 3;
    clean.then_some(start)
}

fn words(tokens: &[Token], kind: SectionKind, export: bool) -> Vec<(String, bool)> {
    let fold = if export && kind == SectionKind::Auction {
        ap_fold_point(tokens)
    } else {
        None
    };
    let mut words = Vec::with_capacity(tokens.len());
    let mut folded = false;
    for (k, token) in tokens.iter().enumerate() {
        if fold == Some(k) {
            words.push(("AP".to_string(), true));
            folded = true;
        }
        if folded && matches!(token, Token::Call(Call::Pass)) {
            continue;
        }
        if export && matches!(token, Token::Continuation | Token::Raw(_)) {
            continue;
        }
        words.push(word_of(token, kind, export));
    }
    if export {
        order_annotations(&mut words);
    }
    words
}

/// Export order after each call or card: note references first, then NAGs ascending.
fn order_annotations(words: &mut [(String, bool)]) {
    let key = |w: &str| match w.strip_prefix('$') {
        Some(n) => (1u8, n.parse::<u16>().unwrap_or(u16::MAX)),
        None => (0, 0),
    };
    let mut start = 0;
    while start < words.len() {
        if words[start].1 || words[start].0 == "*" {
            start += 1;
            continue;
        }
        let mut end = start;
        while end < words.len() && !words[end].1 && words[end].0 != "*" {
            end += 1;
        }
        words[start..end].sort_by_key(|(w, _)| key(w));
        start = end;
    }
}

fn write_words(out: &mut String, words: &[(String, bool)], nl: &str) {
    let mut current = String::new();
    let mut cells = 0;
    for (word, is_cell) in words {
        if word == "*" {
            if !current.is_empty() {
                line(out, &current, nl);
                current.clear();
            }
            line(out, "*", nl);
            cells = 0;
            continue;
        }
        if *is_cell {
            if cells == 4 {
                line(out, &current, nl);
                current.clear();
                cells = 0;
            }
            cells += 1;
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        line(out, &current, nl);
    }
}
