//! Clause grammar.
//!
//! ```ebnf
//! description = { line } ;
//! line        = enumitem | clauses ;
//! enumitem    = ( LETTER ")" | DIGIT ")" | DIGIT "." ) clauses ;      (* items form an OR group *)
//! clauses     = orgroup { ( "," | ";" | "." ) orgroup } ;              (* "," = AND, loosest *)
//! orgroup     = andgroup { ( "or" | "/" ) andgroup } ;
//! andgroup    = fragment { ( "and" | "with" | "w/" | "+" | WS ) fragment } ;
//! fragment    = [ negation ] [ hedge ] atom ;
//! negation    = "not" | "no" | "without" | "w/o" | "denies" | "non" ;
//! hedge       = "usually" | "normally" | "may" | "might" | "rarely" | "typically" | "(?)" ;
//! ```
//!
//! `description` is normalised text (`normalize.rs`) split into physical lines. Enumeration
//! items (`a) … b) …` / `1) … 2) …`) form one `Or` group; every other line is `And`ed with that
//! group and with each other, matching real files where a header line (`one of:`) or trailing
//! common condition sits alongside the enumeration.
//!
//! Precedence, strongest first: `and`/`with`/`w/`/`+`/whitespace (implicit) > `or`/`/` >
//! `,`/`;`/`.`. Unrecognised text becomes one [`FragmentKind::Unrecognized`] fragment spanning
//! up to the next separator, so surrounding structure survives even when a phrase is not
//! understood (`docs/design/06-system.md` §7.3, risk R2).

use winnow::prelude::*;

use super::tokens;

/// A recognised or unrecognised piece of a description.
#[derive(Clone, PartialEq, Debug)]
pub struct Fragment {
    /// Byte span in the normalised text.
    pub span: (u16, u16),
    /// Negated.
    pub negated: bool,
    /// Hedged.
    pub hedged: bool,
    /// The content.
    pub kind: FragmentKind,
}

/// Fragment content (context-free where possible; see `tokens.rs` for the vocabulary).
#[derive(Clone, PartialEq, Debug)]
pub enum FragmentKind {
    /// An atom recognised by the token vocabulary.
    Token(super::tokens::Token),
    /// Text nobody recognised.
    Unrecognized(String),
}

/// The boolean structure of a description over fragment indices.
#[derive(Clone, PartialEq, Debug)]
pub enum Clause {
    /// A single fragment.
    Leaf(usize),
    /// Conjunction.
    And(Vec<Clause>),
    /// Disjunction.
    Or(Vec<Clause>),
}

// ---------------------------------------------------------------------------------------------
// Word tables for negation, hedges and connectives.
// ---------------------------------------------------------------------------------------------

const NEGATIONS: &[&str] = &["without", "denies", "not", "no", "w/o", "non"];
const HEDGES: &[&str] = &[
    "normally",
    "typically",
    "usually",
    "rarely",
    "might",
    "may",
    "(?)",
];
const AND_WORDS: &[&str] = &["and", "with", "w/"];
const OR_WORDS: &[&str] = &["or"];

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

fn strip_ci_word<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    if s.len() < word.len() || !s.is_char_boundary(word.len()) {
        return None;
    }
    let (head, tail) = s.split_at(word.len());
    if !head
        .as_bytes()
        .iter()
        .zip(word.as_bytes())
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
    {
        return None;
    }
    match tail.as_bytes().first() {
        None => Some(tail),
        Some(&b) if !is_word_byte(b) => Some(tail),
        _ => None,
    }
}

fn strip_any<'a>(s: &'a str, words: &[&str]) -> Option<&'a str> {
    for w in words {
        if let Some(rest) = strip_ci_word(s, w) {
            return Some(rest);
        }
    }
    None
}

/// What immediately follows an already-parsed andgroup member.
enum Connective {
    /// `,` `;` `.`: end of the current `orgroup`/`andgroup`, clause-level separator consumed.
    Clause(usize),
    /// `or` / `/`: end of the current `andgroup`, or-level separator consumed.
    Or(usize),
    /// `and` / `with` / `w/` / `+` / a hyphen directly between two suit-length fragments, or
    /// plain whitespace continuing the andgroup implicitly: the separator (if any) is consumed.
    And(usize),
    /// Nothing left on this line.
    End,
}

fn leading_ws_len(s: &str) -> usize {
    s.len() - s.trim_start_matches(' ').len()
}

/// Peeks at `s` to decide how the current andgroup/orgroup/clauses continues. Consumes the
/// connective's own text, including any surrounding whitespace, so the next call always starts
/// exactly at the next fragment's first character (or at the very end of `s`).
fn peek_connective(s: &str) -> Connective {
    let trimmed = s.trim_start_matches(' ');
    let ws = s.len() - trimmed.len();
    if trimmed.is_empty() {
        return Connective::End;
    }

    // Clause-level: "," ";" "." (never confused with digits since those are consumed inside a
    // fragment already).
    if trimmed.starts_with(',') || trimmed.starts_with(';') {
        return Connective::Clause(ws + 1 + leading_ws_len(&trimmed[1..]));
    }
    if let Some(after) = trimmed.strip_prefix('.') {
        // A bare `.` only ends a clause when it is not immediately followed by a digit (which
        // would make it a decimal point; not used by this vocabulary, but safe to guard).
        if !after.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            return Connective::Clause(ws + 1 + leading_ws_len(after));
        }
    }
    if let Some(after) = strip_any(trimmed, OR_WORDS) {
        let word_len = trimmed.len() - after.len();
        return Connective::Or(ws + word_len + leading_ws_len(after));
    }
    if let Some(after) = trimmed.strip_prefix('/') {
        return Connective::Or(ws + 1 + leading_ws_len(after));
    }
    if let Some(after) = strip_any(trimmed, AND_WORDS) {
        let word_len = trimmed.len() - after.len();
        return Connective::And(ws + word_len + leading_ws_len(after));
    }
    if let Some(after) = trimmed.strip_prefix('+') {
        return Connective::And(ws + 1 + leading_ws_len(after));
    }
    // A bare hyphen directly joining two suit-length fragments (`5!h-4!s`): treat as `and`. Only
    // when there was no leading whitespace (a hyphen after a space is not this shorthand).
    if ws == 0 && s.starts_with('-') {
        return Connective::And(1);
    }
    if ws > 0 {
        // Implicit `and` via whitespace only; the separator itself is just the whitespace.
        return Connective::And(ws);
    }
    Connective::End
}

/// One fragment: an optional negation, an optional hedge, then a recognised token or a run of
/// unrecognised words up to the next separator.
pub fn fragment(input: &mut &str) -> ModalResult<Fragment> {
    let start_text: &str = input;
    let mut s: &str = input;

    let mut negated = false;
    if let Some(rest) = strip_any(s, NEGATIONS) {
        negated = true;
        s = rest.trim_start_matches(' ');
    }
    let mut hedged = false;
    if let Some(rest) = strip_any(s, HEDGES) {
        hedged = true;
        s = rest.trim_start_matches(' ');
    }

    let consumed_prefix = start_text.len() - s.len();
    let kind = if let Some((token, len)) = tokens::recognize(s) {
        let end = consumed_prefix + len;
        *input = &start_text[end..];
        FragmentKind::Token(token)
    } else {
        let len = consume_unrecognized(s);
        *input = &start_text[consumed_prefix + len..];
        FragmentKind::Unrecognized(s[..len].trim_end_matches(' ').to_string())
    };

    // Span relative to the start of this call's own input slice; callers (`parse_andgroup`)
    // shift it to the fragment's absolute position within the whole normalised description.
    let span = (0u16, (start_text.len() - input.len()) as u16);
    Ok(Fragment {
        span,
        negated,
        hedged,
        kind,
    })
}

/// Consumes unrecognised text word by word, stopping as soon as a new word is recognised or a
/// separator is reached (so a following recognised token or connective is never swallowed).
fn consume_unrecognized(s: &str) -> usize {
    if s.is_empty() {
        return 0;
    }
    let mut i = 0usize;
    loop {
        // Advance past one "word" (a run of non-space, non-separator-starting characters) plus
        // any following single space, then re-check.
        let rest = &s[i..];
        if rest.is_empty() {
            break;
        }
        if matches!(
            peek_connective(rest),
            Connective::Clause(_) | Connective::Or(_)
        ) {
            break;
        }
        // An implicit/explicit `and`-connective only stops us if what follows it is itself
        // recognised (so "any game force" isn't cut at "any" then "game force" separately
        // re-tried — `tokens::recognize` already tries the longest phrase first).
        if tokens::recognize(rest).is_some() {
            break;
        }
        // Consume one word (up to the next space or separator character).
        let bytes = rest.as_bytes();
        let mut j = 0usize;
        while j < bytes.len() && !matches!(bytes[j], b' ' | b',' | b';' | b'.' | b'/' | b'+') {
            j += 1;
        }
        if j == 0 {
            // A leading separator byte with nothing consumable before it: stop (caller will
            // still have made progress via i>0, or the fragment is empty and the andgroup loop
            // treats it as End).
            break;
        }
        i += j;
        // Absorb exactly one trailing space before re-checking (matches `andgroup`'s implicit
        // whitespace-AND so runs of unrecognised words merge into one fragment).
        if s[i..].starts_with(' ') {
            i += 1;
        }
    }
    i
}

/// `base` is `s`'s absolute offset within the whole normalised description, used to convert a
/// freshly-parsed [`Fragment`]'s locally-relative span into an absolute one.
fn parse_andgroup(s: &str, base: usize, frags: &mut Vec<Fragment>) -> (Clause, usize) {
    let mut items = Vec::new();
    let mut pos = 0usize;
    loop {
        let mut rest = &s[pos..];
        let before = rest.len();
        let idx = frags.len();
        let mut frag = fragment
            .parse_next(&mut rest)
            .expect("fragment() never fails: it falls back to Unrecognized");
        let consumed = before - rest.len();
        let abs_start = (base + pos) as u16;
        frag.span = (abs_start, abs_start + frag.span.1);
        frags.push(frag);
        items.push(Clause::Leaf(idx));
        pos += consumed;

        match peek_connective(&s[pos..]) {
            Connective::And(n) => pos += n,
            _ => break,
        }
    }
    let clause = if items.len() == 1 {
        items.pop().expect("just checked len == 1")
    } else {
        Clause::And(items)
    };
    (clause, pos)
}

fn parse_orgroup(s: &str, base: usize, frags: &mut Vec<Fragment>) -> (Clause, usize) {
    let mut items = Vec::new();
    let mut pos = 0usize;
    loop {
        let (item, consumed) = parse_andgroup(&s[pos..], base + pos, frags);
        items.push(item);
        pos += consumed;
        match peek_connective(&s[pos..]) {
            Connective::Or(n) => pos += n,
            _ => break,
        }
    }
    let clause = if items.len() == 1 {
        items.pop().expect("just checked len == 1")
    } else {
        Clause::Or(items)
    };
    (clause, pos)
}

fn parse_clauses(s: &str, base: usize, frags: &mut Vec<Fragment>) -> (Clause, usize) {
    let mut items = Vec::new();
    let mut pos = 0usize;
    loop {
        let (item, consumed) = parse_orgroup(&s[pos..], base + pos, frags);
        items.push(item);
        pos += consumed;
        match peek_connective(&s[pos..]) {
            Connective::Clause(n) => {
                pos += n;
                // Skip a single space after the separator (kept for symmetry; whitespace was
                // already collapsed by `normalize.rs`).
                if s[pos..].starts_with(' ') {
                    pos += 1;
                }
            }
            _ => break,
        }
    }
    let clause = if items.len() == 1 {
        items.pop().expect("just checked len == 1")
    } else {
        Clause::And(items)
    };
    (clause, pos)
}

/// Guards against any byte of a line surviving past `parse_clauses` unaccounted for (a
/// connective this grammar does not recognise, sitting directly against a fragment with no
/// intervening whitespace). Rather than silently drop it — which would corrupt both the
/// constraint and the recognition ratio — it becomes one trailing `Unrecognized` fragment,
/// `And`ed onto the line's clause. In practice `peek_connective`'s implicit-whitespace fallback
/// means this only fires for pathological input; it never fires for text produced by
/// `normalize.rs` followed by a grammar-covered separator.
fn finish_line(
    s: &str,
    base: usize,
    pos: usize,
    clause: Clause,
    frags: &mut Vec<Fragment>,
) -> Clause {
    if pos >= s.len() {
        return clause;
    }
    let leftover = &s[pos..];
    if leftover.trim_matches(' ').is_empty() {
        return clause;
    }
    let idx = frags.len();
    let trimmed = leftover.trim_start_matches(' ');
    let lead_ws = leftover.len() - trimmed.len();
    let trimmed = trimmed.trim_end_matches(' ');
    let start = (base + pos + lead_ws) as u16;
    frags.push(Fragment {
        span: (start, start + trimmed.len() as u16),
        negated: false,
        hedged: false,
        kind: FragmentKind::Unrecognized(trimmed.to_string()),
    });
    match clause {
        Clause::And(mut v) => {
            v.push(Clause::Leaf(idx));
            Clause::And(v)
        }
        other => Clause::And(vec![other, Clause::Leaf(idx)]),
    }
}

/// Strips a leading enumeration marker (`a)`, `1)`, `1.`) from one line, if present.
fn strip_enum_marker(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    // `LETTER ")"`
    if bytes[0].is_ascii_alphabetic() && bytes.get(1) == Some(&b')') {
        return Some(line[2..].trim_start_matches(' '));
    }
    // `DIGIT+ ")"` or `DIGIT+ "."`
    let mut i = 0usize;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && (bytes.get(i) == Some(&b')') || bytes.get(i) == Some(&b'.')) {
        return Some(line[i + 1..].trim_start_matches(' '));
    }
    None
}

/// Parses a normalised description into fragments and their boolean structure. Fragment spans
/// are absolute byte offsets into `text`.
pub fn parse(text: &str) -> (Vec<Fragment>, Clause) {
    let mut frags: Vec<Fragment> = Vec::new();
    if text.is_empty() {
        return (frags, Clause::And(Vec::new()));
    }

    let mut plain: Vec<Clause> = Vec::new();
    let mut enum_items: Vec<Clause> = Vec::new();
    let mut line_start = 0usize;

    for line in text.split('\n') {
        if line.is_empty() {
            line_start += 1; // the '\n' this empty line consumed
            continue;
        }
        if let Some(rest) = strip_enum_marker(line) {
            let marker_len = line.len() - rest.len();
            let base = line_start + marker_len;
            let (clause, pos) = parse_clauses(rest, base, &mut frags);
            enum_items.push(finish_line(rest, base, pos, clause, &mut frags));
        } else {
            let (clause, pos) = parse_clauses(line, line_start, &mut frags);
            plain.push(finish_line(line, line_start, pos, clause, &mut frags));
        }
        line_start += line.len() + 1; // + 1 for the '\n' separator
    }

    if !enum_items.is_empty() {
        plain.push(Clause::Or(enum_items));
    }

    let top = if plain.len() == 1 {
        plain.pop().expect("just checked len == 1")
    } else {
        Clause::And(plain)
    };
    (frags, top)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::desc::tokens::{StrengthWord, SuitRef, Token};

    fn fragments_of(text: &str) -> (Vec<Fragment>, Clause) {
        parse(text)
    }

    #[test]
    fn simple_and() {
        let (frags, clause) = fragments_of("15-17 hcp and balanced");
        assert_eq!(frags.len(), 2);
        assert!(matches!(clause, Clause::And(ref v) if v.len() == 2));
        assert_eq!(frags[0].kind, FragmentKind::Token(Token::Hcp(15..=17)));
        assert_eq!(frags[1].kind, FragmentKind::Token(Token::Balanced));
    }

    #[test]
    fn implicit_and_via_whitespace() {
        let (frags, clause) = fragments_of("GF SPL");
        assert_eq!(frags.len(), 2);
        assert!(matches!(clause, Clause::And(ref v) if v.len() == 2));
    }

    #[test]
    fn comma_is_and() {
        // `parse()` takes already-normalised text (`normalize.rs` turns `!c` into `♣`); go
        // through `normalize` here so the fixture reads like a real description.
        let n = normalize_test("5+!c, 12-14 hcp");
        let (frags, clause) = fragments_of(&n);
        assert_eq!(frags.len(), 2);
        assert!(matches!(clause, Clause::And(ref v) if v.len() == 2));
    }

    #[test]
    fn or_word_and_slash() {
        let (_frags, clause) = fragments_of("weak or GF");
        assert!(matches!(clause, Clause::Or(ref v) if v.len() == 2));
        let (_frags, clause) = fragments_of("longer suit/shorter suit");
        assert!(matches!(clause, Clause::Or(ref v) if v.len() == 2));
    }

    #[test]
    fn precedence_and_over_or_over_comma() {
        // "and" binds tighter than "or"/"/", which binds tighter than ",". `or` and `/` are
        // both or-level separators, so "6+♣ or 5♣ and 4♥/4♠" splits into three andgroups: the
        // middle one ("5♣ and 4♥") is itself a two-fragment `And`.
        let text = "6+♣ or 5♣ and 4♥/4♠, 11-15 hcp";
        let (_frags, clause) = fragments_of(text);
        match clause {
            Clause::And(top) => {
                assert_eq!(top.len(), 2);
                match &top[0] {
                    Clause::Or(branches) => {
                        assert_eq!(branches.len(), 3);
                        assert!(matches!(branches[1], Clause::And(ref v) if v.len() == 2));
                    }
                    other => panic!("expected Or, got {other:?}"),
                }
            }
            other => panic!("expected top-level And, got {other:?}"),
        }
    }

    #[test]
    fn negation() {
        let (frags, clause) = fragments_of("not 4333");
        assert_eq!(frags.len(), 1);
        assert!(frags[0].negated);
        assert!(matches!(clause, Clause::Leaf(0)));
    }

    #[test]
    fn hedge() {
        let (frags, _clause) = fragments_of("usually 5+♣");
        assert_eq!(frags.len(), 1);
        assert!(frags[0].hedged);
        assert!(!frags[0].negated);
    }

    #[test]
    fn enumeration_forms_or_group() {
        let text = "one of:\n1) weak-two in a major\n2) 22-24 NT\n3) FG in ♦";
        let (frags, clause) = fragments_of(text);
        assert!(!frags.is_empty());
        match clause {
            Clause::And(top) => {
                assert_eq!(top.len(), 2, "header line AND the enumeration Or group");
                match top.last().unwrap() {
                    Clause::Or(items) => assert_eq!(items.len(), 3),
                    other => panic!("expected Or, got {other:?}"),
                }
            }
            other => panic!("expected top-level And, got {other:?}"),
        }
    }

    #[test]
    fn unrecognized_text_kept() {
        let (frags, clause) = fragments_of("values in the bid suits");
        assert_eq!(frags.len(), 1);
        assert!(matches!(clause, Clause::Leaf(0)));
        match &frags[0].kind {
            FragmentKind::Unrecognized(text) => assert_eq!(text, "values in the bid suits"),
            other => panic!("expected Unrecognized, got {other:?}"),
        }
    }

    #[test]
    fn mixed_recognized_and_unrecognized() {
        let (frags, clause) = fragments_of("15-17 hcp, longer major");
        assert_eq!(frags.len(), 2);
        assert!(matches!(clause, Clause::And(ref v) if v.len() == 2));
        assert_eq!(frags[0].kind, FragmentKind::Token(Token::Hcp(15..=17)));
        assert!(matches!(frags[1].kind, FragmentKind::Unrecognized(_)));
    }

    #[test]
    fn context_word_with_synthetic_fragment() {
        let (frags, _clause) = fragments_of("GF");
        assert_eq!(
            frags[0].kind,
            FragmentKind::Token(Token::Strength(StrengthWord::GameForcing))
        );
    }

    #[test]
    fn hash_and_own_suit_refs() {
        let (frags, _clause) = fragments_of("5+#");
        assert_eq!(
            frags[0].kind,
            FragmentKind::Token(Token::SuitLen(SuitRef::Hash, 5..=13))
        );
    }

    fn normalize_test(s: &str) -> String {
        super::super::normalize::normalize(s).text
    }
}
