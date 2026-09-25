//! Clause grammar.
//!
//! ```ebnf
//! description = { line } ;
//! line        = enumitem | clauses ;
//! enumitem    = ( LETTER ")" | DIGIT ")" | DIGIT "." | "(" lower ")" | "(" DIGIT+ ")" ) clauses ;
//!                                                                     (* items form an OR group *)
//! clauses     = orgroup { ( "," | ";" | "." ) [ "or" | "/" ] orgroup } ;
//!               (* "," = AND, loosest; "A, B, or C" = OR of the comma run *)
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
    /// Hedged with a *possibility* word (`may`, `might`, `possibly`, `rarely`, `occasionally`,
    /// `(?)`): the fragment states something the hand can have, not something it must have, so
    /// it contributes no literal (`docs/design/06-system.md` §7.6). Always implies `hedged`.
    pub possibility: bool,
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
/// Hedges that still describe the typical hand: the literal is kept (`usually 15-17` stays
/// 15..=17), only `soft` is recorded.
const PROBABLE_HEDGES: &[&str] = &["normally", "typically", "usually", "likely", "mostly"];
/// Hedges that describe a possibility or an exception (`may be 6!h`, `rarely 4!s`): the literal
/// is dropped (the fragment is `ANY`), `soft` is still recorded.
const POSSIBILITY_HEDGES: &[&str] = &[
    "occasionally",
    "sometimes",
    "possibly",
    "perhaps",
    "maybe",
    "rarely",
    "might",
    "may",
    "(?)",
];
/// Filler verbs a hedge is commonly followed by before the actual atom (`may be 6!h`, `might have
/// 4!s`); skipped so the hedge reaches the atom instead of hedging the filler word alone.
const HEDGE_FILLERS: &[&str] = &["be", "have", "hold", "contain", "include"];
/// Words after which a call-shaped token (`4!s`, `1NT`, `2!d-2!h-3!h`) names a call or an auction,
/// not a suit length (`TRF to 4!s`, `over 1NT`, `qualify for 1!d`).
const CALL_REF_WORDS: &[&str] = &[
    "to",
    "over",
    "after",
    "for",
    "than",
    "via",
    "opposite",
    "like",
    "see",
    "from",
    "into",
    "then",
    "by",
    "bid",
    "rebid",
    "opening",
    "open",
    "bids",
    "else",
    "otherwise",
    "instead",
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

/// The length of a call-shaped token at the start of `s` (`4♠`, `1NT`, `2N`, `3♦`), `None` when
/// `s` does not start with one. Levels are `1..=7`; the token must end at a word boundary.
fn call_token_len(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let level = *bytes.first()?;
    if !(b'1'..=b'7').contains(&level) {
        return None;
    }
    let rest = &s[1..];
    let strain_len = if let Some(ch) = rest.chars().next().filter(|c| "♣♦♥♠".contains(*c)) {
        ch.len_utf8()
    } else if rest.len() >= 2 && rest.as_bytes()[..2].eq_ignore_ascii_case(b"nt") {
        2
    } else if rest
        .as_bytes()
        .first()
        .is_some_and(|b| b.eq_ignore_ascii_case(&b'n'))
    {
        1
    } else {
        return None;
    };
    let end = 1 + strain_len;
    match s.as_bytes().get(end) {
        Some(&b) if is_word_byte(b) || b == b'+' || b == b'=' => None,
        _ => Some(end),
    }
}

/// `(level, strain rank)` of a call token, for the ascending-auction test.
fn call_token_key(tok: &str) -> (u8, u8) {
    let level = tok.as_bytes()[0] - b'0';
    let strain = match tok[1..].chars().next() {
        Some('♣') => 0,
        Some('♦') => 1,
        Some('♥') => 2,
        Some('♠') => 3,
        _ => 4,
    };
    (level, strain)
}

/// A chain of call tokens joined by `-` or `/` at the start of `s`: returns the byte length and
/// the tokens.
fn call_chain(s: &str) -> Option<(usize, Vec<&str>)> {
    let first = call_token_len(s)?;
    let mut toks = vec![&s[..first]];
    let mut i = first;
    while let Some(b'-' | b'/') = s.as_bytes().get(i) {
        let Some(l) = call_token_len(&s[i + 1..]) else {
            break;
        };
        toks.push(&s[i + 1..i + 1 + l]);
        i += 1 + l;
    }
    Some((i, toks))
}

/// A `-`-joined chain of call tokens that reads as an auction (`1♥-1♠-2♣`, `2♦-2♥-3♥`), not as
/// the `5♥-4♠` length shorthand: strictly ascending, and either three or more calls, a notrump
/// call, a level change, or a same-level pair at the one or two level (no hand holds one or two
/// cards in each of two suits as a meaningful length statement).
fn is_auction_chain(s: &str, toks: &[&str]) -> bool {
    if toks.len() < 2 || s.contains('/') {
        return false;
    }
    let keys: Vec<(u8, u8)> = toks.iter().map(|t| call_token_key(t)).collect();
    if !keys.windows(2).all(|w| w[0] < w[1]) {
        return false;
    }
    toks.len() >= 3 || keys.iter().any(|k| k.1 == 4) || keys[0].0 != keys[1].0 || keys[0].0 <= 2
}

/// The last whole word of `before` (the text preceding the current fragment, which must end in
/// whitespace), lower-cased.
fn last_word(before: &str) -> Option<String> {
    let trimmed = before.trim_end_matches(' ');
    if trimmed.len() == before.len() {
        return None;
    }
    let word_start = trimmed
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_alphabetic())
        .last()
        .map(|(i, _)| i)?;
    Some(trimmed[word_start..].to_ascii_lowercase())
}

/// A call reference at the start of `s`, given the text `before` it on the same andgroup: a
/// call-shaped token (or `-`/`/` chain of them) after a [`CALL_REF_WORDS`] word, or an ascending
/// `-` chain that reads as an auction. Returns the byte length to consume as one opaque
/// `Unrecognized` fragment.
fn call_reference_len(before: &str, s: &str) -> Option<usize> {
    let (len, toks) = call_chain(s)?;
    let after_word = last_word(before).is_some_and(|w| CALL_REF_WORDS.contains(&w.as_str()));
    (after_word || is_auction_chain(&s[..len], &toks)).then_some(len)
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
    fragment_after(input, "")
}

/// [`fragment`], given the text `before` the fragment on the same andgroup (used to tell a call
/// reference such as `TRF to 4♠` from a suit length).
fn fragment_after(input: &mut &str, before: &str) -> ModalResult<Fragment> {
    let start_text: &str = input;
    let mut s: &str = input;

    let mut negated = false;
    // A whole phrase that happens to start with a negation or hedge word (`might be strong`) is
    // tried first, before any prefix is stripped off it.
    let whole_phrase = tokens::recognize(s).is_some();
    if !whole_phrase {
        if let Some(rest) = strip_any(s, NEGATIONS) {
            negated = true;
            s = rest.trim_start_matches(' ');
            // `non-forcing` / `non-natural`: the hyphen belongs to the negation.
            if let Some(rest) = s.strip_prefix('-') {
                s = rest;
            }
        }
    }
    let mut hedged = false;
    let mut possibility = false;
    if tokens::recognize(s).is_none() {
        // `(may be 6♥ occasionally)`: a hedge inside an opening parenthesis still hedges.
        let t = match s.strip_prefix('(') {
            Some(t) if !s.starts_with("(?)") => t,
            _ => s,
        };
        let hit = if let Some(rest) = strip_any(t, PROBABLE_HEDGES) {
            Some((rest, false))
        } else {
            strip_any(t, POSSIBILITY_HEDGES).map(|rest| (rest, true))
        };
        if let Some((rest, maybe)) = hit {
            hedged = true;
            possibility = maybe;
            let mut rest = rest.trim_start_matches(' ');
            if let Some(r) = strip_any(rest, HEDGE_FILLERS) {
                rest = r.trim_start_matches(' ');
            }
            s = rest;
        }
    }

    let consumed_prefix = start_text.len() - s.len();
    let kind = if let Some(len) = call_reference_len(before, s) {
        *input = &start_text[consumed_prefix + len..];
        FragmentKind::Unrecognized(s[..len].to_string())
    } else if let Some(len) = consume_see_reference(s) {
        // A cross-reference to another auction/opening (`see 1!c-1!d-2!d-2NT`, `see the 2M
        // opening`), which `docs/design/06-system.md` §7.4 classifies as wholly `v2`/unrecognised
        // prose, not a hand description at all. Handled before the normal word-by-word
        // `consume_unrecognized` run: that one stops the instant it sees a recognisable
        // sub-fragment ahead, which is exactly wrong here -- the auction-notation shorthand after
        // "see" is built from the same suit-length tokens as a real constraint (`1!c`, `2!d`, …)
        // joined by the very same bare-hyphen "and" `peek_connective` uses for `5!h-4!s`, so
        // without this special case the reference's own step numbers get chained into a literal,
        // usually self-contradictory, `SuitLen` conjunction (two different exact lengths for the
        // same suit) instead of being ignored.
        *input = &start_text[consumed_prefix + len..];
        FragmentKind::Unrecognized(s[..len].trim_end_matches(' ').to_string())
    } else if let Some((token, len)) = tokens::recognize(s) {
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
        possibility,
        kind,
    })
}

/// Recognises the start of a `see`/cross-reference aside (`see 1!c-1!d-2!d-2NT`, `see the 2M
/// opening`) and, if found, returns the byte length to consume as one opaque `Unrecognized`
/// fragment: from `see` up to the next clause-level separator (`,`/`;`/`.`) or `or`/`/`, or the
/// end of the line. Unlike [`consume_unrecognized`], this does not stop early just because a
/// sub-span happens to look like a recognisable token -- that is the whole point (see this
/// function's call site in [`fragment`]).
fn consume_see_reference(s: &str) -> Option<usize> {
    let rest = strip_ci_word(s, "see")?;
    let mut i = s.len() - rest.len();
    loop {
        let tail = &s[i..];
        if tail.is_empty()
            || matches!(
                peek_connective(tail),
                Connective::Clause(_) | Connective::Or(_)
            )
        {
            break;
        }
        i += tail.chars().next().expect("not empty").len_utf8();
    }
    Some(i)
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
        // Absorb exactly one trailing space so runs of unrecognised words merge into one
        // fragment — but only when the run actually continues into another unrecognised word.
        // If what follows the space is a clause/or separator or a recognised token, the space
        // itself must be left unconsumed: it is exactly what `parse_andgroup`'s `peek_connective`
        // needs to see as the implicit whitespace-`and` joining this Unrecognized fragment to the
        // next (recognised) one. Consuming it here would silently glue that next fragment onto
        // this run with no connective left to notice, stranding it for `finish_line` to swallow
        // whole instead of letting the andgroup loop parse it as its own fragment.
        if let Some(after) = s[i..].strip_prefix(' ') {
            let run_continues = !after.is_empty()
                && !matches!(
                    peek_connective(after),
                    Connective::Clause(_) | Connective::Or(_)
                )
                && tokens::recognize(after).is_none();
            if run_continues {
                i += 1;
            }
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
        let mut frag = fragment_after(&mut rest, &s[..pos])
            .expect("fragment() never fails: it falls back to Unrecognized");
        let consumed = before - rest.len();
        let abs_start = (base + pos) as u16;
        frag.span = (abs_start, abs_start + frag.span.1);
        // A zero-length fragment (nothing consumable before a connective, e.g. an `or` right at
        // the start of a clause) carries no text at all: never record it, since an empty
        // `Unrecognized` leaf inside an `Or` would collapse that whole group to `ANY`.
        let empty =
            consumed == 0 && matches!(&frag.kind, FragmentKind::Unrecognized(t) if t.is_empty());
        if !empty {
            frags.push(frag);
            items.push(Clause::Leaf(idx));
        }
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

/// `true` for the empty `And` an andgroup of only zero-length fragments produces.
fn is_empty_clause(c: &Clause) -> bool {
    matches!(c, Clause::And(v) if v.is_empty())
}

fn parse_orgroup(s: &str, base: usize, frags: &mut Vec<Fragment>) -> (Clause, usize) {
    let mut items = Vec::new();
    let mut pos = 0usize;
    loop {
        let (item, consumed) = parse_andgroup(&s[pos..], base + pos, frags);
        if !is_empty_clause(&item) {
            items.push(item);
        }
        pos += consumed;
        match peek_connective(&s[pos..]) {
            Connective::Or(n) => pos += n,
            _ => break,
        }
    }
    if items.is_empty() {
        return (Clause::And(Vec::new()), pos);
    }
    let clause = if items.len() == 1 {
        items.pop().expect("just checked len == 1")
    } else {
        Clause::Or(items)
    };
    (clause, pos)
}

/// `clauses` (§7.3), with the list reading of a trailing `, or`: in `A, B, or C` (and `A, or B`)
/// the `or` after a comma turns the whole comma-separated run it ends into one `Or` of its items,
/// instead of `A ∧ B ∧ (∅ ∨ C)`. A run is the items since the last `.`/`;` (sentence break) or
/// since the previous `, or` item; items of a run with no such `or` are `And`ed as before.
fn parse_clauses(s: &str, base: usize, frags: &mut Vec<Fragment>) -> (Clause, usize) {
    let mut items = Vec::new();
    let mut run: Vec<Clause> = Vec::new();
    let mut run_is_or = false;
    let flush = |run: &mut Vec<Clause>, run_is_or: &mut bool, items: &mut Vec<Clause>| {
        if *run_is_or && run.len() > 1 {
            items.push(Clause::Or(core::mem::take(run)));
        } else {
            items.append(run);
        }
        *run_is_or = false;
    };
    let mut pos = 0usize;
    // A clause cannot start with a disjunction: a stray leading `or` has nothing to join.
    if let Connective::Or(n) = peek_connective(s) {
        pos += n;
    }
    loop {
        let (item, consumed) = parse_orgroup(&s[pos..], base + pos, frags);
        if !is_empty_clause(&item) {
            run.push(item);
        }
        pos += consumed;
        match peek_connective(&s[pos..]) {
            Connective::Clause(n) => {
                let sentence_break = !s[pos..].trim_start_matches(' ').starts_with(',');
                pos += n;
                // Skip a single space after the separator (kept for symmetry; whitespace was
                // already collapsed by `normalize.rs`).
                if s[pos..].starts_with(' ') {
                    pos += 1;
                }
                if let Connective::Or(k) = peek_connective(&s[pos..]) {
                    // `A, B, or C`: the run so far and the next item are alternatives.
                    pos += k;
                    run_is_or = true;
                } else if sentence_break || run_is_or {
                    // A sentence break ends the run; so does a plain comma after the `or` item
                    // (`A, or B, 12-14 hcp` is `(A ∨ B) ∧ 12-14`).
                    flush(&mut run, &mut run_is_or, &mut items);
                }
            }
            _ => break,
        }
    }
    flush(&mut run, &mut run_is_or, &mut items);
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
        possibility: false,
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

/// Strips a leading enumeration marker (`a)`, `1)`, `1.`, `(a)`, `(1)`) from one line, if
/// present. The parenthesised letter form only takes a lower-case letter, so `(R)` (relay) and
/// `(?)` stay atoms.
fn strip_enum_marker(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    // `"(" LETTER ")"` / `"(" DIGIT+ ")"`
    if bytes[0] == b'(' {
        let mut i = 1usize;
        if bytes.get(1).is_some_and(u8::is_ascii_lowercase) {
            i = 2;
        } else {
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
        if i > 1 && bytes.get(i) == Some(&b')') {
            return Some(line[i + 1..].trim_start_matches(' '));
        }
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

    // Regression for the real-file triage (roadmap 3.2-3.4, class a): a `see <auction>`
    // cross-reference (gjp's `2D.bml`: "same meaning and development as after 2!d-2!h-3X") is
    // built from the same bare-hyphen-joined suit-length tokens (`1!c`, `2!d`, …) a real
    // constraint uses. Before `consume_see_reference`, `fragment` recognised each step as its own
    // `SuitLen` token and `peek_connective` joined them with its `5!h-4!s` "and" rule, producing a
    // literal (and usually self-contradictory: two different exact lengths for the same suit)
    // `SuitLen` conjunction instead of leaving the reference opaque.
    #[test]
    fn see_reference_is_kept_opaque_not_parsed_as_suit_lengths() {
        let (frags, clause) = fragments_of("see 1♣-1♦-2♦-2NT");
        assert_eq!(
            frags.len(),
            1,
            "the whole cross-reference must stay one fragment, not one per step"
        );
        assert!(matches!(clause, Clause::Leaf(0)));
        match &frags[0].kind {
            FragmentKind::Unrecognized(text) => assert_eq!(text, "see 1♣-1♦-2♦-2NT"),
            other => panic!("expected Unrecognized, got {other:?}"),
        }
    }

    #[test]
    fn see_reference_stops_at_the_next_clause_separator() {
        let (frags, clause) = fragments_of("see 1♣-1♦-2♦-2NT, 12-14 hcp");
        assert_eq!(frags.len(), 2);
        assert!(matches!(clause, Clause::And(ref v) if v.len() == 2));
        match &frags[0].kind {
            FragmentKind::Unrecognized(text) => assert_eq!(text, "see 1♣-1♦-2♦-2NT"),
            other => panic!("expected Unrecognized, got {other:?}"),
        }
        assert_eq!(frags[1].kind, FragmentKind::Token(Token::Hcp(12..=14)));
    }

    #[test]
    fn see_the_opening_reference_is_also_kept_opaque() {
        let (frags, clause) = fragments_of("see the 2♥ opening");
        assert_eq!(frags.len(), 1);
        assert!(matches!(clause, Clause::Leaf(0)));
        match &frags[0].kind {
            FragmentKind::Unrecognized(text) => assert_eq!(text, "see the 2♥ opening"),
            other => panic!("expected Unrecognized, got {other:?}"),
        }
    }

    fn normalize_test(s: &str) -> String {
        super::super::normalize::normalize(s).text
    }
}
