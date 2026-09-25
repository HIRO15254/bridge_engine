//! The token vocabulary (WBF / BML abbreviations).
//!
//! Each token maps to atom literals or flags; context-dependent tokens carry no values until
//! `context.rs` resolves them. The full table (with examples from real files) is in
//! `docs/design/06-system.md` §7.4.
//!
//! `recognize` is a longest-match, order-sensitive scanner over normalised text (see
//! `normalize.rs`): it never backtracks past a chosen match, so sub-matchers are tried in an
//! order chosen to avoid one form shadowing a longer one (shapes before bare HCP ranges before
//! single keywords, longer keyword phrases before their prefixes).

use core::ops::RangeInclusive;

use bridge_core::{ShapeSet, Suit};

/// A suit reference inside a description.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SuitRef {
    /// `!s`, or a bound variable substituted at expansion.
    Fixed(Suit),
    /// `#`: the suit of the nearest variable / multi-strain call in the path.
    Hash,
    /// The suit of this row's own call.
    Own,
    /// `M`, `oM`, `m`, `om` when unbound.
    AnyMajor,
    /// Any minor.
    AnyMinor,
    /// Partner's agreed suit.
    Agreed,
    /// Opponents' suit.
    Theirs,
}

/// A recognised token.
#[derive(Clone, PartialEq, Debug)]
pub enum Token {
    /// `12+ hcp`, `15-17`, `ca 15+`.
    Hcp(RangeInclusive<u8>),
    /// `13+ points` (total points).
    Points(RangeInclusive<u8>),
    /// `5+!s`, `4=!h`, `0-3!s`, `6+ suit`, `4+#`.
    SuitLen(SuitRef, RangeInclusive<u8>),
    /// `4414`, `(54)`, `54(31)`, `(54)(xx)`, `55MM`, `5-5 minors`, `4-4 majors`.
    Shape(String),
    /// `bal`.
    Balanced,
    /// `semi-bal`.
    SemiBalanced,
    /// `unbal`.
    Unbalanced,
    /// `GF`, `INV`, `INV+`, `MIN`, `MAX`, `weak`, `STR`, `PRE`, `S/T`, `QUANT`, `NEG`, `LIM`.
    Strength(StrengthWord),
    /// `NF`, `F`, `F1`, `FG`.
    Forcing(crate::Forcing),
    /// `ART`, `(R)`, `TRF`, `PUP`, `P/C`, `S/O`, `STAY`, `SPL`, `UNT`, `Multi`, …
    Convention(String),
    /// `SOL`, `S-SOL`, `2 of top 3`, `AKQ`, `good suit`.
    Quality(SuitRef, QualityWord),
    /// `stopper`, `with stopper`.
    Stopper(SuitRef),
    /// `singleton`, `void`, `short`, `0-1!h`.
    Shortness(SuitRef, u8),
    /// `fit`, `3+ SUPP`, `support`, `raise`.
    Support(u8),
    /// `controls`, `2 controls`. The full range `0..=12` marks the bare, unconstrained form.
    Controls(RangeInclusive<u8>),
    /// `7 losers`, `LTC`. The full range `0..=24` marks the bare, unconstrained form.
    Losers(RangeInclusive<u8>),
    /// `NAT`, `natural`.
    Natural,
    /// `unlimited`, `any hand`: recognised, no constraint.
    NoBound,
}

/// Context-dependent strength words.
///
/// The `INV` family (`docs/design/06-system.md` §7.4/§7.5) is not one shape: `INV` gives both
/// bounds with no shift; `InvitationalPlus` (`INV+`) is open-ended (`start` only); `Mildly
/// invitational` shifts both bounds down by 1; `Strongly invitational` shifts both bounds up by
/// 1; `at most invitational` gives only the `end` bound (floor 0).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(missing_docs)]
pub enum StrengthWord {
    GameForcing,
    Invitational,
    InvitationalPlus,
    /// `Mildly invitational`: both bounds, shifted down by 1.
    InvitationalMild,
    /// `Strongly invitational`: both bounds, shifted up by 1.
    InvitationalStrong,
    /// `at most invitational`: `end` bound only (floor 0).
    InvitationalAtMost,
    Min,
    Max,
    Weak,
    Strong,
    Preemptive,
    SlamTry,
    Quantitative,
    Negative,
    Limit,
}

/// Suit-quality words.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(missing_docs)]
pub enum QualityWord {
    Solid,
    SemiSolid,
    TwoOfTopThree,
    ThreeOfTopFive,
    Good,
}

// ---------------------------------------------------------------------------------------------
// Small text-matching primitives (ASCII case-insensitive unless noted; ASCII-only phrases).
// ---------------------------------------------------------------------------------------------

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

/// Case-insensitive prefix match of `phrase` at the start of `s`, requiring the following byte
/// (if any) not to continue a word. Returns the number of bytes of `s` consumed (`phrase.len()`).
fn match_word(s: &str, phrase: &str) -> Option<usize> {
    if s.len() < phrase.len() || !s.is_char_boundary(phrase.len()) {
        return None;
    }
    let (head, tail) = s.split_at(phrase.len());
    if !head
        .as_bytes()
        .iter()
        .zip(phrase.as_bytes())
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
    {
        return None;
    }
    match tail.as_bytes().first() {
        None => Some(phrase.len()),
        Some(&b) if !is_word_byte(b) => Some(phrase.len()),
        _ => None,
    }
}

/// Exact-case prefix match (used only for `M`/`m`/`oM`/`om`, where case is meaningful).
fn match_word_cs(s: &str, phrase: &str) -> Option<usize> {
    if !s.as_bytes().starts_with(phrase.as_bytes()) {
        return None;
    }
    let tail = &s[phrase.len()..];
    match tail.as_bytes().first() {
        None => Some(phrase.len()),
        Some(&b) if !is_word_byte(b) => Some(phrase.len()),
        _ => None,
    }
}

/// Tries each `(phrase, value)` pair in order (the caller sorts longest-first when phrases
/// could otherwise shadow each other) and returns the first match.
fn match_table<T: Copy>(s: &str, table: &[(&str, T)]) -> Option<(T, usize)> {
    for (phrase, value) in table {
        if let Some(len) = match_word(s, phrase) {
            return Some((*value, len));
        }
    }
    None
}

fn sentinel_suit(ch: char) -> Option<Suit> {
    match ch {
        '♣' => Some(Suit::Clubs),
        '♦' => Some(Suit::Diamonds),
        '♥' => Some(Suit::Hearts),
        '♠' => Some(Suit::Spades),
        _ => None,
    }
}

/// Parses a leading unsigned integer of at most 2 digits (HCP `0..=37`, length `0..=13`).
fn parse_number(s: &str) -> Option<(u8, usize)> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && i < 2 && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return None;
    }
    let val: u8 = s[..i].parse().ok()?;
    Some((val, i))
}

/// A suit reference following a length or as a lone fragment: a sentinel, `#`, the exact-case
/// major/minor variables, or a generic word (`suit`, `cards`, `major`, …).
fn parse_suit_ref(s: &str) -> Option<(SuitRef, usize)> {
    if let Some(ch) = s.chars().next() {
        if let Some(suit) = sentinel_suit(ch) {
            return Some((SuitRef::Fixed(suit), ch.len_utf8()));
        }
        if ch == '#' {
            return Some((SuitRef::Hash, 1));
        }
    }
    if let Some(l) = match_word_cs(s, "oM") {
        return Some((SuitRef::AnyMajor, l));
    }
    if let Some(l) = match_word_cs(s, "om") {
        return Some((SuitRef::AnyMinor, l));
    }
    if let Some(l) = match_word_cs(s, "M") {
        return Some((SuitRef::AnyMajor, l));
    }
    if let Some(l) = match_word_cs(s, "m") {
        return Some((SuitRef::AnyMinor, l));
    }
    const WORDS: &[(&str, SuitRef)] = &[
        ("card suit", SuitRef::Own),
        ("cards", SuitRef::Own),
        ("card", SuitRef::Own),
        ("suits", SuitRef::Own),
        ("suit", SuitRef::Own),
        ("majors", SuitRef::AnyMajor),
        ("major", SuitRef::AnyMajor),
        ("minors", SuitRef::AnyMinor),
        ("minor", SuitRef::AnyMinor),
    ];
    match_table(s, WORDS)
}

/// [`parse_suit_ref`], additionally trying again after skipping one leading space (`"5+ M"`,
/// `"6+ suit"`).
fn parse_suit_ref_ws(s: &str) -> Option<(SuitRef, usize)> {
    if let Some(hit) = parse_suit_ref(s) {
        return Some(hit);
    }
    let s2 = s.strip_prefix(' ')?;
    let (suitref, l) = parse_suit_ref(s2)?;
    Some((suitref, (s.len() - s2.len()) + l))
}

enum MetricKind {
    Hcp,
    Points,
    Controls,
    Losers,
}

/// An optional ` hcp` / ` points` / ` controls` / ` losers` suffix (with the leading space).
/// Returns `(kind, 0)` (defaulting to HCP) when nothing matches, so range forms can fall back to
/// the "bare range is HCP" rule.
fn match_metric_suffix(s: &str) -> (MetricKind, usize) {
    let s2 = s.strip_prefix(' ').unwrap_or(s);
    let ws = s.len() - s2.len();
    const WORDS: &[(&str, u8)] = &[
        ("total points", 1),
        ("points", 1),
        ("point", 1),
        ("tp", 1),
        ("hcp", 0),
        ("controls", 2),
        ("control", 2),
        ("losers", 3),
        ("loser", 3),
    ];
    if let Some((kind, l)) = match_table(s2, WORDS) {
        let k = match kind {
            1 => MetricKind::Points,
            2 => MetricKind::Controls,
            3 => MetricKind::Losers,
            _ => MetricKind::Hcp,
        };
        return (k, ws + l);
    }
    (MetricKind::Hcp, 0)
}

/// Words that mark a support/fit count (§7.4 row 20: `fit`, `3+ SUPP`, `support`, `raise`,
/// `trumps`). Shared by [`match_support_suffix`] (an explicit leading number, e.g. `3+ SUPP`) and
/// [`match_support`] (the bare form, defaulting to 3).
const SUPPORT_WORDS: &[&str] = &["support", "supp", "fit", "raise", "trumps"];

/// An optional ` supp` / ` support` / ` fit` / ` raise` / ` trumps` suffix (with the leading
/// space), used by [`match_length_or_metric`] to route an explicit leading number (`3+ SUPP`,
/// `4+ trumps`) into a `Token::Support` with that number, instead of letting it fall through to a
/// bare HCP/length reading and losing the written minimum.
fn match_support_suffix(s: &str) -> Option<usize> {
    let s2 = s.strip_prefix(' ').unwrap_or(s);
    let ws = s.len() - s2.len();
    for phrase in SUPPORT_WORDS {
        if let Some(l) = match_word(s2, phrase) {
            return Some(ws + l);
        }
    }
    None
}

fn strip_approx_prefix(s: &str) -> (&str, usize) {
    for word in ["ca", "about"] {
        if let Some(l) = match_word(s, word) {
            let after = &s[l..];
            if let Some(after_space) = after.strip_prefix(' ') {
                return (after_space, l + 1);
            }
        }
    }
    (s, 0)
}

fn build_token(kind_hint: MetricKindTag, range: RangeInclusive<u8>) -> Token {
    match kind_hint {
        MetricKindTag::Hcp => Token::Hcp(range),
        MetricKindTag::Points => Token::Points(range),
        MetricKindTag::Controls => Token::Controls(range),
        MetricKindTag::Losers => Token::Losers(range),
    }
}

#[derive(Clone, Copy)]
enum MetricKindTag {
    Hcp,
    Points,
    Controls,
    Losers,
}

fn tag_of(kind: &MetricKind) -> MetricKindTag {
    match kind {
        MetricKind::Hcp => MetricKindTag::Hcp,
        MetricKind::Points => MetricKindTag::Points,
        MetricKind::Controls => MetricKindTag::Controls,
        MetricKind::Losers => MetricKindTag::Losers,
    }
}

/// `N controls` / `N losers` / `N+ hcp` / `N-M hcp` / `N=!s` / `N!s` / `N+!s` / `N+ SUPP` / bare
/// `LTC` / `controls`, and everything else in the numeric family (§7.4 rows 1-3, 12-13, 20).
fn match_length_or_metric(s: &str) -> Option<(Token, usize)> {
    // `LTC` / `LTC 7`: reversed word-then-number order, handled first.
    if let Some(l) = match_word(s, "ltc") {
        let rest = &s[l..];
        let rest2 = rest.strip_prefix(' ').unwrap_or(rest);
        let ws = rest.len() - rest2.len();
        if let Some((n, l2)) = parse_number(rest2) {
            return Some((Token::Losers(n..=n), l + ws + l2));
        }
        return Some((Token::Losers(0..=24), l));
    }

    let (s1, prefix_len) = strip_approx_prefix(s);
    let (n1, l1) = parse_number(s1)?;
    let mut consumed = prefix_len + l1;
    let rest = &s1[l1..];

    // `N=SUIT` / `N=SUPP`: exact length, or an explicit support count.
    if let Some(rest2) = rest.strip_prefix('=') {
        if let Some((suitref, l)) = parse_suit_ref_ws(rest2) {
            return Some((Token::SuitLen(suitref, n1..=n1), consumed + 1 + l));
        }
        if let Some(l) = match_support_suffix(rest2) {
            return Some((Token::Support(n1), consumed + 1 + l));
        }
        return None;
    }

    // `N-M ...`: a range.
    if let Some(rest2) = rest.strip_prefix('-') {
        let (n2, l2) = parse_number(rest2)?;
        // A descending pair (`n1 > n2`) is not a range at all -- most often the leading half of a
        // two-suit shape shorthand missing its group word (`gjp/common/1C.bml`'s `3C = variant 2,
        // 5-4`, meant as "5 cards in one major, 4 in the other" and left implicit by the
        // surrounding prose, not a real numeric range). Accepting it as `n1..=n2` here would hand
        // `Atom::ANY`'s `hcp`/length field a `RangeInclusive` that can never be satisfied (`5..=4`
        // matches no value at all), silently turning stray prose into an always-false constraint.
        // Refusing the match here instead lets the fragment fall through to `Unrecognized` (it
        // still counts against the recognition ratio, correctly, since it *is* unrecognised).
        if n1 > n2 {
            return None;
        }
        consumed += 1 + l2;
        let after = &rest2[l2..];
        if let Some((suitref, l3)) = parse_suit_ref_ws(after) {
            return Some((Token::SuitLen(suitref, n1..=n2), consumed + l3));
        }
        if let Some(l3) = match_support_suffix(after) {
            return Some((Token::Support(n1), consumed + l3));
        }
        let (kind, l3) = match_metric_suffix(after);
        return Some((build_token(tag_of(&kind), n1..=n2), consumed + l3));
    }

    // `N+ ...`: open-ended (a suit length, a support count, or a metric).
    if let Some(rest2) = rest.strip_prefix('+') {
        consumed += 1;
        if let Some((suitref, l3)) = parse_suit_ref_ws(rest2) {
            return Some((Token::SuitLen(suitref, n1..=13), consumed + l3));
        }
        if let Some(l3) = match_support_suffix(rest2) {
            return Some((Token::Support(n1), consumed + l3));
        }
        let (kind, l3) = match_metric_suffix(rest2);
        let hi = match kind {
            MetricKind::Points => 40,
            MetricKind::Hcp => 37,
            MetricKind::Controls => 12,
            MetricKind::Losers => 24,
        };
        return Some((build_token(tag_of(&kind), n1..=hi), consumed + l3));
    }

    // Bare `N` directly followed by a suit reference: exact length.
    if let Some((suitref, l3)) = parse_suit_ref_ws(rest) {
        return Some((Token::SuitLen(suitref, n1..=n1), consumed + l3));
    }

    // Bare `N` directly followed by a support word (`4 trumps`): an explicit support count.
    if let Some(l3) = match_support_suffix(rest) {
        return Some((Token::Support(n1), consumed + l3));
    }

    // Bare `N` followed by a metric word (`hcp`, `points`, `controls`, `losers`): a single value.
    let (kind, l3) = match_metric_suffix(rest);
    if l3 > 0 {
        return Some((build_token(tag_of(&kind), n1..=n1), consumed + l3));
    }
    None
}

/// A bare `controls` / `losers` with no leading number: recognised, unconstrained.
fn match_bare_metric_word(s: &str) -> Option<(Token, usize)> {
    if let Some(l) = match_word(s, "controls") {
        return Some((Token::Controls(0..=12), l));
    }
    if let Some(l) = match_word(s, "control") {
        return Some((Token::Controls(0..=12), l));
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Shapes (§7.4 rows 6-8). `Token::Shape` stores the raw matched text; `context.rs` interprets it
// via [`scan_shape`], which this module also uses for boundary detection so the grammar lives in
// exactly one place.
// ---------------------------------------------------------------------------------------------

/// Suit positions in the order shape digits are written: S H D C.
const POSITION_SUITS: [Suit; 4] = [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs];

fn suit_range(len: u8, plus: bool) -> (u8, u8) {
    if plus { (len, 13) } else { (len, len) }
}

fn two_suit_shapeset(p: Suit, a: u8, aplus: bool, q: Suit, b: u8, bplus: bool) -> ShapeSet {
    let (alo, ahi) = suit_range(a, aplus);
    let (blo, bhi) = suit_range(b, bplus);
    let assign1 =
        ShapeSet::from_suit_len(p, alo, ahi).intersect(ShapeSet::from_suit_len(q, blo, bhi));
    let assign2 =
        ShapeSet::from_suit_len(p, blo, bhi).intersect(ShapeSet::from_suit_len(q, alo, ahi));
    assign1.union(assign2)
}

/// `["("] DIGIT [")"] ["+"]`.
fn parse_lenspec(s: &str) -> Option<(u8, bool, usize)> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    let paren = bytes.first() == Some(&b'(');
    if paren {
        i += 1;
    }
    let d = *bytes.get(i)?;
    if !d.is_ascii_digit() {
        return None;
    }
    let val = d - b'0';
    i += 1;
    if paren {
        if bytes.get(i) != Some(&b')') {
            return None;
        }
        i += 1;
    }
    let mut plus = false;
    if bytes.get(i) == Some(&b'+') {
        plus = true;
        i += 1;
    }
    Some((val, plus, i))
}

/// A named group (`majors`, `minors`, `MM`, `mm`, `red suits`, `black suits`) or an explicit
/// sentinel pair (`♦+♣`), as two suits.
fn match_group_word(s: &str) -> Option<([Suit; 2], usize)> {
    if s.as_bytes().starts_with(b"MM") {
        return Some(([Suit::Hearts, Suit::Spades], 2));
    }
    if s.as_bytes().starts_with(b"mm") {
        return Some(([Suit::Clubs, Suit::Diamonds], 2));
    }
    let s2 = s.strip_prefix(' ').unwrap_or(s);
    let ws = s.len() - s2.len();
    const WORDS: &[(&str, [Suit; 2])] = &[
        ("red suits", [Suit::Diamonds, Suit::Hearts]),
        ("black suits", [Suit::Clubs, Suit::Spades]),
        ("majors", [Suit::Hearts, Suit::Spades]),
        ("major", [Suit::Hearts, Suit::Spades]),
        ("minors", [Suit::Clubs, Suit::Diamonds]),
        ("minor", [Suit::Clubs, Suit::Diamonds]),
        ("red", [Suit::Diamonds, Suit::Hearts]),
        ("black", [Suit::Clubs, Suit::Spades]),
    ];
    if let Some((group, l)) = match_table(s2, WORDS) {
        return Some((group, ws + l));
    }
    // An explicit pair of sentinels: `♦+♣`.
    let mut chars = s.chars();
    let c1 = chars.next()?;
    let suit1 = sentinel_suit(c1)?;
    let rest = &s[c1.len_utf8()..];
    let rest2 = rest.strip_prefix('+')?;
    let c2 = rest2.chars().next()?;
    let suit2 = sentinel_suit(c2)?;
    Some(([suit1, suit2], c1.len_utf8() + 1 + c2.len_utf8()))
}

/// `<len>[-]<len> <group>`, `<len><len>MM`, `both MM`.
fn match_two_suit_shape(s: &str) -> Option<(ShapeSet, usize)> {
    if let Some(l) = match_word(s, "both") {
        let rest = s[l..].strip_prefix(' ')?;
        let (group, gl) = match_group_word(rest)?;
        let set = two_suit_shapeset(group[0], 4, true, group[1], 4, true);
        return Some((set, l + (s[l..].len() - rest.len()) + gl));
    }

    let (a, aplus, la) = parse_lenspec(s)?;
    let mut rest = &s[la..];
    let mut consumed = la;
    if rest.as_bytes().first() == Some(&b'-') {
        rest = &rest[1..];
        consumed += 1;
    }
    let (b, bplus, lb) = parse_lenspec(rest)?;
    rest = &rest[lb..];
    consumed += lb;
    let (group, gl) = match_group_word(rest)?;
    consumed += gl;
    Some((
        two_suit_shapeset(group[0], a, aplus, group[1], b, bplus),
        consumed,
    ))
}

struct ShapeSlot {
    digit: Option<u8>,
    group: Option<u32>,
}

/// A full 4-position pattern: digits, `x` wildcards and `(...)`-grouped alternatives, in S H D
/// C order (e.g. `4414`, `(54)`, `54(31)`, `(54)(xx)`).
fn match_full_shape(s: &str) -> Option<(ShapeSet, usize)> {
    let bytes = s.as_bytes();
    let mut slots: Vec<ShapeSlot> = Vec::with_capacity(4);
    let mut i = 0usize;
    let mut next_group = 0u32;

    while slots.len() < 4 {
        let b = *bytes.get(i)?;
        if b == b'(' {
            let rel_close = s[i..].find(')')?;
            let inner = &s[i + 1..i + rel_close];
            if inner.is_empty() {
                return None;
            }
            let gid = next_group;
            next_group += 1;
            for ch in inner.chars() {
                if slots.len() >= 4 {
                    return None;
                }
                if ch == 'x' || ch == 'X' {
                    slots.push(ShapeSlot {
                        digit: None,
                        group: Some(gid),
                    });
                } else if ch.is_ascii_digit() {
                    slots.push(ShapeSlot {
                        digit: Some(ch as u8 - b'0'),
                        group: Some(gid),
                    });
                } else {
                    return None;
                }
            }
            i += rel_close + 1;
        } else if b == b'x' || b == b'X' {
            slots.push(ShapeSlot {
                digit: None,
                group: None,
            });
            i += 1;
        } else if b.is_ascii_digit() {
            slots.push(ShapeSlot {
                digit: Some(b - b'0'),
                group: None,
            });
            i += 1;
        } else {
            return None;
        }
    }
    if slots.len() != 4 {
        return None;
    }
    // A fully digit-specified pattern (no `x` wildcard anywhere, grouped or not) names an exact
    // 13-card distribution; reject one whose digits do not sum to 13, so that an unrelated
    // 4-digit numeral elsewhere in the vocabulary (RKCB step-response codes like `0314`, a year
    // fragment, …) is not mistaken for a hand shape. A pattern with any `x` is a genuine partial
    // shape (the remaining cards are unspecified) and is left unvalidated as before.
    if slots.iter().all(|slot| slot.digit.is_some()) {
        let sum: u32 = slots
            .iter()
            .map(|slot| u32::from(slot.digit.unwrap()))
            .sum();
        if sum != 13 {
            return None;
        }
    }

    let mut acc = ShapeSet::ALL;
    for (idx, slot) in slots.iter().enumerate() {
        if slot.group.is_none() {
            if let Some(d) = slot.digit {
                acc = acc.intersect(ShapeSet::from_suit_len(POSITION_SUITS[idx], d, d));
            }
        }
    }
    let mut seen: Vec<u32> = Vec::new();
    for gid in slots.iter().filter_map(|slot| slot.group) {
        if seen.contains(&gid) {
            continue;
        }
        seen.push(gid);
        let positions: Vec<usize> = slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.group == Some(gid))
            .map(|(idx, _)| idx)
            .collect();
        let digit_positions: Vec<usize> = positions
            .iter()
            .copied()
            .filter(|&idx| slots[idx].digit.is_some())
            .collect();
        let digits: Vec<u8> = digit_positions
            .iter()
            .map(|&idx| slots[idx].digit.unwrap())
            .collect();
        if digits.is_empty() {
            continue;
        }
        acc = acc.intersect(permute_union(&digits, &digit_positions));
    }
    Some((acc, i))
}

fn permute_union(digits: &[u8], positions: &[usize]) -> ShapeSet {
    let mut result = ShapeSet::EMPTY;
    let mut used = vec![false; digits.len()];
    let mut current: Vec<u8> = Vec::with_capacity(digits.len());
    permute_rec(digits, &mut used, &mut current, &mut |perm| {
        let mut set = ShapeSet::ALL;
        for (i, &pos) in positions.iter().enumerate() {
            set = set.intersect(ShapeSet::from_suit_len(
                POSITION_SUITS[pos],
                perm[i],
                perm[i],
            ));
        }
        result = result.union(set);
    });
    result
}

fn permute_rec(items: &[u8], used: &mut [bool], current: &mut Vec<u8>, f: &mut impl FnMut(&[u8])) {
    if current.len() == items.len() {
        f(current);
        return;
    }
    for i in 0..items.len() {
        if used[i] {
            continue;
        }
        used[i] = true;
        current.push(items[i]);
        permute_rec(items, used, current, f);
        current.pop();
        used[i] = false;
    }
}

/// Scans a shape pattern at the start of `text`, returning its meaning and the bytes consumed.
/// Used both by [`recognize`] (boundary detection) and `context.rs` (semantic resolution of a
/// stored [`Token::Shape`]).
pub(crate) fn scan_shape(text: &str) -> Option<(ShapeSet, usize)> {
    match_two_suit_shape(text).or_else(|| match_full_shape(text))
}

// ---------------------------------------------------------------------------------------------
// Keyword vocabulary: strength, forcing, conventions, quality, stopper, shortness, support,
// natural, no-bound (§7.4 rows 9-11, 14-23).
// ---------------------------------------------------------------------------------------------

fn match_strength(s: &str) -> Option<(Token, usize)> {
    const WORDS: &[(&str, StrengthWord)] = &[
        ("forcing to game", StrengthWord::GameForcing),
        ("game forcing", StrengthWord::GameForcing),
        ("gameforcing", StrengthWord::GameForcing),
        ("game force", StrengthWord::GameForcing),
        ("any game force", StrengthWord::GameForcing),
        ("gf", StrengthWord::GameForcing),
        ("fg", StrengthWord::GameForcing),
        ("invitational plus", StrengthWord::InvitationalPlus),
        ("at most invitational", StrengthWord::InvitationalAtMost),
        ("strongly invitational", StrengthWord::InvitationalStrong),
        ("mildly invitational", StrengthWord::InvitationalMild),
        ("inv+", StrengthWord::InvitationalPlus),
        ("invitational", StrengthWord::Invitational),
        ("inv", StrengthWord::Invitational),
        ("game try", StrengthWord::Invitational),
        ("g/t", StrengthWord::Invitational),
        ("limit", StrengthWord::Limit),
        ("lim", StrengthWord::Limit),
        ("minimum", StrengthWord::Min),
        ("min/max", StrengthWord::Min),
        ("min", StrengthWord::Min),
        ("maximum", StrengthWord::Max),
        ("max", StrengthWord::Max),
        ("very light", StrengthWord::Weak),
        ("light", StrengthWord::Weak),
        ("weak", StrengthWord::Weak),
        ("wk", StrengthWord::Weak),
        ("might be strong", StrengthWord::Strong),
        ("strong", StrengthWord::Strong),
        ("str", StrengthWord::Strong),
        ("preemptive", StrengthWord::Preemptive),
        ("barrage", StrengthWord::Preemptive),
        ("pre", StrengthWord::Preemptive),
        ("slam interest", StrengthWord::SlamTry),
        ("slam try", StrengthWord::SlamTry),
        ("s/t", StrengthWord::SlamTry),
        ("quantitative", StrengthWord::Quantitative),
        ("quant", StrengthWord::Quantitative),
        ("negative", StrengthWord::Negative),
        ("neg", StrengthWord::Negative),
        ("0+ hcp", StrengthWord::Weak),
    ];
    match_table(s, WORDS).map(|(w, l)| (Token::Strength(w), l))
}

fn match_forcing(s: &str) -> Option<(Token, usize)> {
    const WORDS: &[(&str, crate::Forcing)] = &[
        ("non forcing", crate::Forcing::NonForcing),
        ("nonforcing", crate::Forcing::NonForcing),
        ("nf", crate::Forcing::NonForcing),
        ("f2nt", crate::Forcing::OneRound),
        ("f1r", crate::Forcing::OneRound),
        ("f1", crate::Forcing::OneRound),
        ("forcing", crate::Forcing::Unknown),
        ("f", crate::Forcing::Unknown),
    ];
    match_table(s, WORDS).map(|(f, l)| (Token::Forcing(f), l))
}

fn match_convention(s: &str) -> Option<(Token, usize)> {
    const WORDS: &[&str] = &[
        "artificial",
        "art",
        "relay",
        "(r)",
        "asking for",
        "asks",
        "asking",
        "ask",
        "puppet to",
        "puppet",
        "pup",
        "pass/correct",
        "pass / correct",
        "p/c",
        "t/o",
        "forced",
        "choice of games",
        "cog",
        "cuebid",
        "cue",
        "waiting",
        "transfer to",
        "transfer",
        "retransfer",
        "trf",
        "garbage stayman",
        "muppet stayman",
        "puppet stayman",
        "garbage stay",
        "muppet stay",
        "stayman",
        "stay",
        "smolen",
        "landy",
        "michaels",
        "unusual nt",
        "unusual",
        "unt",
        "gambling",
        "lebensohl",
        "ogust",
        "bw",
        "rkcb",
        "kcb",
        "k/b",
        "multi-coloured",
        "multi-colored",
        "multi",
        "mini-splinter",
        "splinter",
        "spl",
        "sign off",
        "sign-off",
        "s/o",
        "to play",
        "t/p",
    ];
    for phrase in WORDS {
        if let Some(l) = match_word(s, phrase) {
            return Some((Token::Convention(phrase.to_uppercase()), l));
        }
    }
    None
}

fn match_quality(s: &str) -> Option<(Token, usize)> {
    // Explicit honour runs, e.g. `AKQ`, `AKQxx`, `KQJ109x`: a run of `A K Q J T` letters
    // (optionally followed by `x`/`9..2` filler), at least 2 honours.
    let bytes = s.as_bytes();
    let mut honours = 0usize;
    let mut i = 0usize;
    while i < bytes.len() && matches!(bytes[i], b'A' | b'K' | b'Q' | b'J' | b'T') {
        honours += 1;
        i += 1;
    }
    if honours >= 2 {
        let mut j = i;
        while j < bytes.len() && matches!(bytes[j], b'x' | b'X' | b'2'..=b'9') {
            j += 1;
        }
        // A literal honour run (`AKQ`, `AKQxx`, `KQJ109x`) is approximated as "2+ of the top
        // 3" (§7.4's `TwoOfTopThree`); the exact-count reading (`Solid`'s card part without its
        // length requirement) is not separately representable in `QualityWord`.
        return Some((Token::Quality(SuitRef::Own, QualityWord::TwoOfTopThree), j));
    }

    const WORDS: &[(&str, QualityWord)] = &[
        ("semi-solid", QualityWord::SemiSolid),
        ("semi solid", QualityWord::SemiSolid),
        ("s-sol", QualityWord::SemiSolid),
        ("solid suit", QualityWord::Solid),
        ("solid", QualityWord::Solid),
        ("sol", QualityWord::Solid),
        ("3 of top 5", QualityWord::ThreeOfTopFive),
        ("three of top five", QualityWord::ThreeOfTopFive),
        ("2 of top 3", QualityWord::TwoOfTopThree),
        ("2 of 3 top", QualityWord::TwoOfTopThree),
        ("2/3 top", QualityWord::TwoOfTopThree),
        ("two of top three", QualityWord::TwoOfTopThree),
        ("decent suit", QualityWord::Good),
        ("reasonable suit", QualityWord::Good),
        ("quality suit", QualityWord::Good),
        ("good suit", QualityWord::Good),
    ];
    match_table(s, WORDS).map(|(w, l)| (Token::Quality(SuitRef::Own, w), l))
}

fn match_stopper(s: &str) -> Option<(Token, usize)> {
    const WORDS: &[&str] = &["with stopper", "stopper in", "stoppers", "stopper", "stop"];
    for phrase in WORDS {
        if let Some(l) = match_word(s, phrase) {
            if let Some((suitref, l2)) = parse_suit_ref_ws(&s[l..]) {
                return Some((Token::Stopper(suitref), l + l2));
            }
            return Some((Token::Stopper(SuitRef::Theirs), l));
        }
    }
    None
}

fn match_shortness(s: &str) -> Option<(Token, usize)> {
    const WORDS: &[(&str, u8)] = &[("void", 0), ("singleton", 1), ("short", 2), ("s/s", 2)];
    if let Some((max, l)) = match_table(s, WORDS) {
        if let Some((suitref, l2)) = parse_suit_ref_ws(&s[l..]) {
            return Some((Token::Shortness(suitref, max), l + l2));
        }
        return Some((Token::Shortness(SuitRef::Own, max), l));
    }
    None
}

fn match_support(s: &str) -> Option<(Token, usize)> {
    for phrase in SUPPORT_WORDS {
        if let Some(l) = match_word(s, phrase) {
            return Some((Token::Support(3), l));
        }
    }
    None
}

fn match_natural(s: &str) -> Option<(Token, usize)> {
    if let Some(l) = match_word(s, "natural") {
        return Some((Token::Natural, l));
    }
    if let Some(l) = match_word(s, "nat") {
        return Some((Token::Natural, l));
    }
    None
}

fn match_balanced(s: &str) -> Option<(Token, usize)> {
    const WORDS: &[(&str, u8)] = &[
        ("semi-balanced", 1),
        ("semi balanced", 1),
        ("semi-bal", 1),
        ("semibal", 1),
        ("unbalanced", 2),
        ("unbal", 2),
        ("balanced", 0),
        ("bal", 0),
    ];
    match_table(s, WORDS).map(|(kind, l)| {
        let token = match kind {
            1 => Token::SemiBalanced,
            2 => Token::Unbalanced,
            _ => Token::Balanced,
        };
        (token, l)
    })
}

fn match_no_bound(s: &str) -> Option<(Token, usize)> {
    const WORDS: &[&str] = &[
        "any distribution",
        "any strength",
        "any hand",
        "wide ranged",
        "wide range",
        "unlimited",
        "any",
    ];
    for phrase in WORDS {
        if let Some(l) = match_word(s, phrase) {
            return Some((Token::NoBound, l));
        }
    }
    None
}

/// Tries to recognise one fragment of normalised text. Returns the token and the number of
/// bytes consumed.
///
/// Order matters: shapes are tried before bare numeric ranges (a run of exactly 4 digits is a
/// shape, not an HCP number), and numeric/metric forms before the plain keyword table (so `S/T`
/// the strength word is not confused with a length-like prefix, and a `5+!s` length is not
/// swallowed by `match_support`'s `supp`/`fit`).
pub fn recognize(text: &str) -> Option<(Token, usize)> {
    if let Some((set, len)) = scan_shape(text) {
        let _ = set; // interpreted again, from the stored string, by `context::resolve`.
        return Some((Token::Shape(text[..len].to_string()), len));
    }
    if let Some(hit) = match_length_or_metric(text) {
        return Some(hit);
    }
    if let Some(hit) = match_bare_metric_word(text) {
        return Some(hit);
    }
    if let Some(hit) = match_balanced(text) {
        return Some(hit);
    }
    if let Some(hit) = match_strength(text) {
        return Some(hit);
    }
    if let Some(hit) = match_forcing(text) {
        return Some(hit);
    }
    if let Some(hit) = match_convention(text) {
        return Some(hit);
    }
    if let Some(hit) = match_quality(text) {
        return Some(hit);
    }
    if let Some(hit) = match_stopper(text) {
        return Some(hit);
    }
    if let Some(hit) = match_shortness(text) {
        return Some(hit);
    }
    if let Some(hit) = match_natural(text) {
        return Some(hit);
    }
    if let Some(hit) = match_no_bound(text) {
        return Some(hit);
    }
    if let Some(hit) = match_support(text) {
        return Some(hit);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(s: &str) -> Token {
        recognize(s)
            .unwrap_or_else(|| panic!("expected a token in {s:?}"))
            .0
    }

    #[test]
    fn hcp_range() {
        assert_eq!(rec("15-17"), Token::Hcp(15..=17));
        assert_eq!(rec("12+ hcp"), Token::Hcp(12..=37));
        assert_eq!(rec("9+hcp"), Token::Hcp(9..=37));
    }

    #[test]
    fn points_range() {
        assert_eq!(rec("13+ points"), Token::Points(13..=40));
        assert_eq!(rec("9-17 points"), Token::Points(9..=17));
    }

    #[test]
    fn suit_len_sentinel() {
        assert_eq!(
            rec("5+♠"),
            Token::SuitLen(SuitRef::Fixed(Suit::Spades), 5..=13)
        );
        assert_eq!(
            rec("4=♥"),
            Token::SuitLen(SuitRef::Fixed(Suit::Hearts), 4..=4)
        );
        assert_eq!(
            rec("0-3♠"),
            Token::SuitLen(SuitRef::Fixed(Suit::Spades), 0..=3)
        );
        assert_eq!(rec("5+#"), Token::SuitLen(SuitRef::Hash, 5..=13));
    }

    #[test]
    fn suit_len_own_word() {
        assert_eq!(rec("6+ suit"), Token::SuitLen(SuitRef::Own, 6..=13));
        assert_eq!(rec("5 card suit"), Token::SuitLen(SuitRef::Own, 5..=5));
    }

    #[test]
    fn suit_len_major_minor_word() {
        assert_eq!(rec("5+ M"), Token::SuitLen(SuitRef::AnyMajor, 5..=13));
        assert_eq!(rec("4+m"), Token::SuitLen(SuitRef::AnyMinor, 4..=13));
        assert_eq!(rec("4M"), Token::SuitLen(SuitRef::AnyMajor, 4..=4));
    }

    // Regression for the real-file triage (roadmap 3.2-3.4, class a): gjp's `3C = variant 2,
    // 5-4` means "5 in one major, 4 in the other" (a two-suit shape shorthand missing its group
    // word), not a numeric range -- `n1 > n2` can never be satisfied as `n1..=n2`. Before this
    // guard it matched as `Token::SuitLen`/a length metric with an always-empty range, silently
    // turning stray prose into an always-false constraint instead of falling through to
    // `Unrecognized` (still counted against the recognition ratio, correctly, since it *is*
    // unrecognised).
    #[test]
    fn descending_numeric_range_is_not_a_valid_range() {
        assert!(match_length_or_metric("5-4").is_none());
        assert!(match_length_or_metric("5-4 hcp").is_none());
        assert!(match_length_or_metric("5-4♣").is_none());
        // A genuine ascending range still matches, and an equal pair still matches too (as an
        // exact 1-value "range").
        assert!(match_length_or_metric("4-5 hcp").is_some());
        assert!(match_length_or_metric("5-5 hcp").is_some());
    }

    // Regression for the real-file triage (roadmap 3.2-3.4, class a): a fully digit-specified
    // pattern (no `x` wildcard) names an exact 13-card distribution. Before this guard, an
    // unrelated 4-digit numeral elsewhere in the vocabulary (an RKCB step-response code, a year
    // fragment, …) whose digits do not sum to 13 was still accepted as a `Shape`, contradicting
    // the row's own explicit suit-length fragments once resolved.
    #[test]
    fn full_shape_digits_must_sum_to_thirteen() {
        assert!(
            match_full_shape("4432").is_some(),
            "4+4+3+2 = 13: a real shape"
        );
        assert!(
            match_full_shape("0314").is_none(),
            "an RKCB step-response code (sums to 8), not a shape"
        );
        assert!(match_full_shape("9999").is_none(), "sums to 36");
        // A pattern with any `x` wildcard is a genuine partial shape and stays unvalidated.
        assert!(match_full_shape("44xx").is_some());
    }

    #[test]
    fn full_shape() {
        assert_eq!(rec("4414"), Token::Shape("4414".to_string()));
        let (_, len) = recognize("(54)(xx)").unwrap();
        assert_eq!(len, 8);
        let (_, len) = recognize("54(31)").unwrap();
        assert_eq!(len, 6);
    }

    #[test]
    fn shape_semantics_full() {
        let (set, _) = scan_shape("4414").unwrap();
        let shape = bridge_core::Shape::from_lens([4, 1, 4, 4]); // C D H S
        assert!(set.contains(shape));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn shape_semantics_parenthesised() {
        let (set, len) = scan_shape("54(31)").unwrap();
        assert_eq!(len, 6);
        // S=5 H=4, {D,C} any permutation of {3,1}.
        let a = bridge_core::Shape::from_lens([1, 3, 4, 5]);
        let b = bridge_core::Shape::from_lens([3, 1, 4, 5]);
        assert!(set.contains(a));
        assert!(set.contains(b));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn shape_two_suit_majors() {
        let (set, len) = scan_shape("5-5 minors").unwrap();
        assert_eq!(len, 10);
        let a = bridge_core::Shape::from_lens([5, 5, 0, 3]); // C=5 D=5 H=0 S=3
        assert!(set.contains(a));
        let b = bridge_core::Shape::from_lens([0, 0, 5, 8]); // not both minors long
        assert!(!set.contains(b));
    }

    #[test]
    fn shape_compact_mm() {
        let (set, len) = scan_shape("44MM").unwrap();
        assert_eq!(len, 4);
        let a = bridge_core::Shape::from_lens([2, 3, 4, 4]);
        assert!(set.contains(a));
    }

    #[test]
    fn balanced_words() {
        assert_eq!(rec("bal"), Token::Balanced);
        assert_eq!(rec("semi-bal"), Token::SemiBalanced);
        assert_eq!(rec("unbal"), Token::Unbalanced);
    }

    #[test]
    fn strength_words() {
        assert_eq!(rec("GF"), Token::Strength(StrengthWord::GameForcing));
        assert_eq!(rec("INV"), Token::Strength(StrengthWord::Invitational));
        assert_eq!(rec("INV+"), Token::Strength(StrengthWord::InvitationalPlus));
        assert_eq!(
            rec("Mildly invitational"),
            Token::Strength(StrengthWord::InvitationalMild)
        );
        assert_eq!(
            rec("Strongly invitational"),
            Token::Strength(StrengthWord::InvitationalStrong)
        );
        assert_eq!(
            rec("at most invitational"),
            Token::Strength(StrengthWord::InvitationalAtMost)
        );
        assert_eq!(rec("MIN"), Token::Strength(StrengthWord::Min));
        assert_eq!(rec("MAX"), Token::Strength(StrengthWord::Max));
        assert_eq!(rec("weak"), Token::Strength(StrengthWord::Weak));
        assert_eq!(rec("STR"), Token::Strength(StrengthWord::Strong));
        assert_eq!(rec("PRE"), Token::Strength(StrengthWord::Preemptive));
        assert_eq!(rec("S/T"), Token::Strength(StrengthWord::SlamTry));
        assert_eq!(rec("QUANT"), Token::Strength(StrengthWord::Quantitative));
        assert_eq!(rec("NEG"), Token::Strength(StrengthWord::Negative));
        assert_eq!(rec("LIM"), Token::Strength(StrengthWord::Limit));
    }

    #[test]
    fn forcing_words() {
        assert_eq!(rec("NF"), Token::Forcing(crate::Forcing::NonForcing));
        assert_eq!(rec("F"), Token::Forcing(crate::Forcing::Unknown));
        assert_eq!(rec("F1"), Token::Forcing(crate::Forcing::OneRound));
        assert_eq!(rec("F2NT"), Token::Forcing(crate::Forcing::OneRound));
    }

    #[test]
    fn convention_words() {
        assert_eq!(rec("ART"), Token::Convention("ART".to_string()));
        assert_eq!(rec("(R)"), Token::Convention("(R)".to_string()));
        assert_eq!(rec("TRF"), Token::Convention("TRF".to_string()));
        assert_eq!(rec("STAY"), Token::Convention("STAY".to_string()));
        assert_eq!(rec("SPL"), Token::Convention("SPL".to_string()));
        assert_eq!(rec("UNT"), Token::Convention("UNT".to_string()));
        assert_eq!(rec("P/C"), Token::Convention("P/C".to_string()));
        assert_eq!(rec("S/O"), Token::Convention("S/O".to_string()));
    }

    #[test]
    fn quality_words() {
        assert_eq!(rec("SOL"), Token::Quality(SuitRef::Own, QualityWord::Solid));
        assert_eq!(
            rec("AKQ"),
            Token::Quality(SuitRef::Own, QualityWord::TwoOfTopThree)
        );
        assert_eq!(
            rec("good suit"),
            Token::Quality(SuitRef::Own, QualityWord::Good)
        );
    }

    #[test]
    fn stopper_word() {
        assert_eq!(rec("stopper"), Token::Stopper(SuitRef::Theirs));
    }

    #[test]
    fn shortness_words() {
        assert_eq!(rec("singleton"), Token::Shortness(SuitRef::Own, 1));
        assert_eq!(rec("void"), Token::Shortness(SuitRef::Own, 0));
        assert_eq!(
            rec("short ♥"),
            Token::Shortness(SuitRef::Fixed(Suit::Hearts), 2)
        );
        assert_eq!(
            rec("0-1♥"),
            Token::SuitLen(SuitRef::Fixed(Suit::Hearts), 0..=1)
        );
    }

    #[test]
    fn support_words() {
        assert_eq!(rec("fit"), Token::Support(3));
        assert_eq!(rec("support"), Token::Support(3));
    }

    #[test]
    fn support_words_with_explicit_length() {
        // The design table's own examples (§7.4 row 20): the written minimum must survive, not
        // get discarded in favour of the bare-form default of 3.
        let (tok, len) = recognize("3+ SUPP").unwrap();
        assert_eq!(tok, Token::Support(3));
        assert_eq!(len, "3+ SUPP".len());

        let (tok, len) = recognize("4+ trumps").unwrap();
        assert_eq!(tok, Token::Support(4));
        assert_eq!(len, "4+ trumps".len());

        assert_eq!(rec("5+ fit"), Token::Support(5));
        assert_eq!(rec("4=support"), Token::Support(4));
    }

    #[test]
    fn controls_and_losers() {
        assert_eq!(rec("3+ controls"), Token::Controls(3..=12));
        assert_eq!(rec("2 controls"), Token::Controls(2..=2));
        assert_eq!(rec("controls"), Token::Controls(0..=12));
        assert_eq!(rec("7 losers"), Token::Losers(7..=7));
        assert_eq!(rec("6-7 losers"), Token::Losers(6..=7));
        assert_eq!(rec("LTC 7"), Token::Losers(7..=7));
        assert_eq!(rec("LTC"), Token::Losers(0..=24));
    }

    #[test]
    fn natural_word() {
        assert_eq!(rec("NAT"), Token::Natural);
        assert_eq!(rec("natural"), Token::Natural);
    }

    #[test]
    fn no_bound_words() {
        assert_eq!(rec("unlimited"), Token::NoBound);
        assert_eq!(rec("any hand"), Token::NoBound);
        assert_eq!(rec("any distribution"), Token::NoBound);
    }

    #[test]
    fn unrecognized_prose_returns_none() {
        assert!(recognize("values in the bid suits").is_none());
        assert!(recognize("longer major").is_none());
    }
}
