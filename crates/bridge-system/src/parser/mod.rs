//! Paragraph classification, clipboard expansion and bidding-table tree building.
//!
//! Every stage records failures as [`Lint`]s and keeps going: an unparsable call
//! token skips that row and its subtree; an indentation that matches no open ancestor attaches
//! the row to the nearest shallower one; a row containing `-` or `;` that is not the first row
//! of its table is dropped.

pub mod call;
pub mod clipboard;

use crate::{
    CallPattern, Lint, LintCode, Side, Var,
    ast::{
        BidTable, Block, BmlFile, BmlNode, CallToken, Description, FileId, RawLine, SeatCond, Span,
        Tri, VulCond,
    },
    lexer::Loaded,
};
use clipboard::Clipboard;

/// Builds the AST from loaded lines.
pub fn parse(loaded: Loaded) -> BmlFile {
    let mut lints = loaded.lints;
    let paragraphs = crate::lexer::paragraphs(&loaded.lines);
    let mut blocks = Vec::new();
    let mut clipboard = Clipboard::default();
    let mut seat = SeatCond::default();
    let mut vul = VulCond::default();
    let mut exact_pass_files = ExactPassFiles::default();

    for paragraph in paragraphs {
        match classify(&paragraph) {
            ParagraphKind::Heading => blocks.push(parse_heading(&paragraph)),
            ParagraphKind::List => blocks.push(parse_list(&paragraph, false)),
            ParagraphKind::Enumeration => blocks.push(parse_list(&paragraph, true)),
            ParagraphKind::Seat => {
                seat = parse_seat(&paragraph, &mut lints);
                ignored_exact_pass_file(&paragraph, "#SEAT", &mut lints);
                blocks.push(Block::Seat {
                    cond: seat,
                    span: paragraph[0].span.clone(),
                });
            }
            ParagraphKind::Vul => {
                vul = parse_vul(&paragraph, &mut lints);
                ignored_exact_pass_file(&paragraph, "#VUL", &mut lints);
                blocks.push(Block::Vul {
                    cond: vul,
                    span: paragraph[0].span.clone(),
                });
            }
            ParagraphKind::Meta => parse_meta(&paragraph, &mut lints, &mut blocks),
            ParagraphKind::BidTable | ParagraphKind::Directive => {
                if let Some(block) = parse_table_paragraph(
                    paragraph,
                    &mut clipboard,
                    seat,
                    vul,
                    &mut exact_pass_files,
                    &mut lints,
                ) {
                    blocks.push(block);
                }
            }
            ParagraphKind::Paragraph => {
                relative_row_as_prose(&paragraph, &mut lints);
                broken_row_as_prose(&paragraph, &mut lints);
                blocks.push(parse_paragraph(&paragraph));
            }
        }
    }

    exact_pass_files.report_unused(&mut lints);

    BmlFile {
        root: crate::lexer::ROOT,
        files: loaded.files,
        blocks,
        lints,
    }
}

/// The `#EXACTPASS FILE` directives met so far: each puts every later table of its own file in
/// scope (`docs/design/06-system.md` §4.8). A file is a [`FileId`], one per inclusion, so the
/// scope ends with the file and never reaches a file it includes or the file that includes it.
#[derive(Default)]
struct ExactPassFiles {
    /// `(file, the directive's location, whether a table in its scope has a row of ours right
    /// after an opponents' pass)`; the first directive of a file is the one in force.
    files: Vec<(FileId, Span, bool)>,
}

impl ExactPassFiles {
    /// Opens the scope of the directive at `span` for the rest of its file.
    fn open(&mut self, span: Span) {
        if !self.files.iter().any(|(f, _, _)| *f == span.file) {
            self.files.push((span.file, span, false));
        }
    }

    /// The directive whose scope covers a table of `file`, if any, noting whether the table has
    /// a row for it to act on.
    fn scope_of(&mut self, file: FileId, acts: bool) -> Option<Span> {
        let (_, span, used) = self.files.iter_mut().find(|(f, _, _)| *f == file)?;
        *used |= acts;
        Some(span.clone())
    }

    /// `ExactPassWithoutPass` for each directive none of whose tables it could act on.
    fn report_unused(&self, lints: &mut Vec<Lint>) {
        for (_, span, used) in &self.files {
            if !used {
                lints.push(
                    Lint::info(
                        LintCode::ExactPassWithoutPass,
                        "#EXACTPASS FILE: no table after it in this file has a row of ours right \
                         after an opponents' pass, so it has no effect",
                    )
                    .with_span(span.clone()),
                );
            }
        }
    }
}

/// Kind of a paragraph, decided in the same order as the reference implementation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(missing_docs)]
pub enum ParagraphKind {
    Heading,
    List,
    Enumeration,
    Seat,
    Vul,
    BidTable,
    Meta,
    Directive,
    Paragraph,
}

/// Classifies a paragraph.
pub fn classify(paragraph: &[RawLine]) -> ParagraphKind {
    let Some(first) = paragraph.first() else {
        return ParagraphKind::Paragraph;
    };
    let trimmed = first.text.trim_start();
    if trimmed.starts_with('*') {
        return ParagraphKind::Heading;
    }
    if trimmed.starts_with('-') {
        return ParagraphKind::List;
    }
    if trimmed.starts_with("#VUL") {
        return ParagraphKind::Vul;
    }
    if trimmed.starts_with("#SEAT") {
        return ParagraphKind::Seat;
    }
    if is_enum_start(trimmed) {
        return ParagraphKind::Enumeration;
    }
    let first_word = trimmed.split_whitespace().next().unwrap_or("");
    if is_bidtable_start(first_word) {
        return ParagraphKind::BidTable;
    }
    if is_meta_start(trimmed) {
        return ParagraphKind::Meta;
    }
    if trimmed.starts_with('#') {
        return ParagraphKind::Directive;
    }
    ParagraphKind::Paragraph
}

fn is_enum_start(s: &str) -> bool {
    let after_digits = s.trim_start_matches(|c: char| c.is_ascii_digit());
    if after_digits.len() == s.len() {
        return false; // no leading digits at all
    }
    match after_digits.strip_prefix('.') {
        Some(rest) => rest.is_empty() || rest.starts_with(char::is_whitespace),
        None => false,
    }
}

/// A paragraph's first word is a bidding-table row exactly when it parses fully as a call token
/// or a history sequence: this reuses the grammar instead of approximating it with a regex, so
/// it also recognises tables that open on a bare `P`/`D`/`R`/`X`/`XX` row.
fn is_bidtable_start(word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    if word.contains('-') || word.contains(';') {
        let mut s = word;
        return call::history(&mut s).is_ok() && s.is_empty();
    }
    let mut s = word;
    match call::calltok(&mut s) {
        // A relative level (`cS`, `jY`) names no call without a bid before it, so it never
        // starts a table: a prose paragraph that happens to begin with such a word stays prose.
        Ok((_, pattern)) => s.is_empty() && !has_relative_level(&pattern),
        Err(_) => false,
    }
}

/// A prose paragraph whose first line has the shape of a bidding-table row led by a relative
/// level (`cS = 5+!s`): a relative level never starts a table (`is_bidtable_start`), so the
/// paragraph, and every ordinary row under it, is read as prose. Say so rather than drop the
/// rows without a trace (Warning `UnknownCallToken`; `docs/design/06-system.md` §3.2).
fn relative_row_as_prose(paragraph: &[RawLine], lints: &mut Vec<Lint>) {
    let Some(first) = paragraph.first() else {
        return;
    };
    let mut words = first.text.split_whitespace();
    let (Some(word), Some(next)) = (words.next(), words.next()) else {
        return;
    };
    // `cS = …` is flagged. Without the `=`, `cS is how this file writes …` may be ordinary
    // prose that happens to start with a relative-level word, so it is flagged only when a
    // later line of the paragraph is itself a row (`1S = …`): that paragraph was a table.
    if next != "=" && !paragraph[1..].iter().any(|line| is_row_line(&line.text)) {
        return;
    }
    let mut s = word;
    let relative =
        matches!(call::calltok(&mut s), Ok((_, ref p)) if s.is_empty() && has_relative_level(p));
    if relative {
        lints.push(
            Lint::warning(
                LintCode::UnknownCallToken,
                format!(
                    "{word}: a relative level cannot open a bidding table (no bid before it); \
                     the paragraph ({} line(s)) is read as prose",
                    paragraph.len()
                ),
            )
            .with_span(first.span.clone()),
        );
    }
}

/// Whether a line has the shape of an explicit bidding-table row: a call token (or
/// history) followed by `=`.
fn is_row_line(text: &str) -> bool {
    let mut words = text.split_whitespace();
    matches!((words.next(), words.next()), (Some(word), Some("=")) if is_bidtable_start(word))
}

/// A prose paragraph whose first line looks like a bidding-table row with a typo in its call
/// token (`1N--2C- = …`, `1N-2Q-`, `1Nx = …`): the first word starts with a level and a strain
/// letter (after an optional `(`), has no `!`, and is a sequence or is followed by `=`, but does
/// not parse, so the paragraph
/// and every row under it is read as prose. Say so rather than drop the whole table without a
/// trace (Warning `UnknownCallToken`).
fn broken_row_as_prose(paragraph: &[RawLine], lints: &mut Vec<Lint>) {
    let Some(first) = paragraph.first() else {
        return;
    };
    let mut words = first.text.split_whitespace();
    let Some(word) = words.next() else {
        return;
    };
    // A level and a strain letter: `1N…`, `(2S)…`; not `2-suited`.
    let starts_with_level = matches!(
        word.strip_prefix('(').unwrap_or(word).as_bytes(),
        [b'1'..=b'7', b'C' | b'D' | b'H' | b'S' | b'N', ..]
    );
    let row_shaped = word.contains('-') || word.contains(';') || words.next() == Some("=");
    // Prose written with suit symbols (`1!d-(2!c)-3!d is preemptive …`) is prose on purpose:
    // `!` never appears in a call token.
    if !starts_with_level || !row_shaped || word.contains('!') || is_bidtable_start(word) {
        return;
    }
    lints.push(
        Lint::warning(
            LintCode::UnknownCallToken,
            format!(
                "{word}: the paragraph looks like a bidding table, but its first row cannot be \
                 parsed; the paragraph ({} line(s)) is read as prose",
                paragraph.len()
            ),
        )
        .with_span(first.span.clone()),
    );
}

/// How many distinct variables among `X`, `Y`, `Z` a table's history and rows use (the ones
/// whose strain order `#ANYORDER` lifts).
fn xyz_variables(history: &[CallToken], rows: &[BmlNode]) -> usize {
    fn pattern(p: &CallPattern, seen: &mut [bool; 3]) {
        match p {
            CallPattern::Var { var, .. } => match var {
                Var::X => seen[0] = true,
                Var::Y => seen[1] = true,
                Var::Z => seen[2] = true,
                _ => {}
            },
            CallPattern::AnyOf(alts) => alts.iter().for_each(|a| pattern(a, seen)),
            _ => {}
        }
    }
    fn node(n: &BmlNode, seen: &mut [bool; 3]) {
        n.calls.iter().for_each(|t| pattern(&t.pattern, seen));
        n.children.iter().for_each(|c| node(c, seen));
    }
    let mut seen = [false; 3];
    history.iter().for_each(|t| pattern(&t.pattern, &mut seen));
    rows.iter().for_each(|n| node(n, &mut seen));
    seen.iter().filter(|&&b| b).count()
}

/// Whether a table has a row of ours right after an opponents' pass: after a `(P)` (in the
/// history, as a row, or as an alternative such as `(P/1S)`), or after a call of ours, with the
/// opponents' implicit pass between the two (the positions `#EXACTPASS` acts on).
fn has_row_after_their_pass(history: &[CallToken], rows: &[BmlNode]) -> bool {
    fn is_pass(p: &CallPattern) -> bool {
        match p {
            CallPattern::Exact(call) => *call == bridge_core::Call::Pass,
            CallPattern::AnyOf(alts) => alts.iter().any(is_pass),
            _ => false,
        }
    }
    /// `before`: the side and pattern of the call the rows follow (`None` for openings).
    fn walk(before: Option<(Side, &CallPattern)>, rows: &[BmlNode]) -> bool {
        let after_their_pass = match before {
            Some((Side::Us, _)) => true,
            Some((Side::Them, p)) => is_pass(p),
            None => false,
        };
        rows.iter().any(|r| {
            let tok = &r.calls[0];
            (tok.side == Side::Us && after_their_pass)
                || walk(Some((tok.side, &tok.pattern)), &r.children)
        })
    }
    walk(history.last().map(|t| (t.side, &t.pattern)), rows)
}

/// `#EXACTPASS FILE` (the words separated by any whitespace).
fn is_exact_pass_file(trimmed: &str) -> bool {
    let mut words = trimmed.split_whitespace();
    words.next() == Some("#EXACTPASS") && words.next() == Some("FILE") && words.next().is_none()
}

/// Reports a `#EXACTPASS FILE` line after the first line of a `#SEAT` / `#VUL` paragraph
/// (`head` names the directive). Those paragraphs read only their first line, so the line has
/// no effect; it opens a scope only in a directive or table paragraph (a `#+` meta paragraph
/// already reports every line that is not `#+KEY: value`).
fn ignored_exact_pass_file(paragraph: &[RawLine], head: &str, lints: &mut Vec<Lint>) {
    for line in paragraph.iter().skip(1) {
        if is_exact_pass_file(line.text.trim_start()) {
            lints.push(
                Lint::warning(
                    LintCode::UnknownDirective,
                    format!(
                        "#EXACTPASS FILE inside a {head} paragraph: write it as a paragraph of \
                         its own before the tables it covers; ignored"
                    ),
                )
                .with_span(line.span.clone()),
            );
        }
    }
}

/// Whether `pattern` (or one of its alternatives) uses a relative level (`c`, `j`).
fn has_relative_level(pattern: &CallPattern) -> bool {
    match pattern {
        CallPattern::Strains { level, .. } | CallPattern::Var { level, .. } => level.is_relative(),
        CallPattern::AnyOf(alts) => alts.iter().any(has_relative_level),
        _ => false,
    }
}

fn is_meta_start(s: &str) -> bool {
    s.strip_prefix("#+")
        .and_then(|rest| rest.chars().next())
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
}

fn parse_seat(paragraph: &[RawLine], lints: &mut Vec<Lint>) -> SeatCond {
    let line = &paragraph[0];
    let arg = line.text.split_whitespace().nth(1).unwrap_or("");
    match arg {
        "0" => SeatCond::Any,
        "1" => SeatCond::First,
        "2" => SeatCond::Second,
        "3" => SeatCond::Third,
        "4" => SeatCond::Fourth,
        "12" => SeatCond::FirstOrSecond,
        "34" => SeatCond::ThirdOrFourth,
        other => {
            lints.push(
                Lint::warning(
                    LintCode::UnknownDirective,
                    format!("#SEAT {other}: expected one of 0 1 2 3 4 12 34"),
                )
                .with_span(line.span.clone()),
            );
            SeatCond::Any
        }
    }
}

fn parse_tri(s: &str) -> Option<Tri> {
    match s {
        "Y" => Some(Tri::Yes),
        "N" => Some(Tri::No),
        "0" => Some(Tri::Any),
        _ => None,
    }
}

fn parse_vul(paragraph: &[RawLine], lints: &mut Vec<Lint>) -> VulCond {
    let line = &paragraph[0];
    let arg = line.text.split_whitespace().nth(1).unwrap_or("");
    if arg.len() == 2 {
        let (a, b) = arg.split_at(1);
        if let Some(we) = parse_tri(a) {
            if let Some(they) = parse_tri(b) {
                return VulCond { we, they };
            }
        }
    }
    lints.push(
        Lint::warning(
            LintCode::UnknownDirective,
            format!("#VUL {arg}: expected two of Y/N/0"),
        )
        .with_span(line.span.clone()),
    );
    VulCond::default()
}

fn parse_meta(paragraph: &[RawLine], lints: &mut Vec<Lint>, blocks: &mut Vec<Block>) {
    for line in paragraph {
        let trimmed = line.text.trim_start();
        if let Some(rest) = trimmed.strip_prefix("#+") {
            if let Some((key, value)) = rest.split_once(':') {
                blocks.push(Block::Meta {
                    key: key.trim().to_string(),
                    value: value.trim().to_string(),
                    span: line.span.clone(),
                });
                continue;
            }
        }
        lints.push(
            Lint::warning(
                LintCode::UnknownDirective,
                format!("malformed meta line: {trimmed:?}"),
            )
            .with_span(line.span.clone()),
        );
    }
}

fn parse_heading(paragraph: &[RawLine]) -> Block {
    let line = &paragraph[0];
    let trimmed = line.text.trim_start();
    let level = trimmed.chars().take_while(|&c| c == '*').count() as u8;
    let text = trimmed[level as usize..].trim().to_string();
    Block::Heading {
        level,
        text,
        span: line.span.clone(),
    }
}

fn parse_list(paragraph: &[RawLine], ordered: bool) -> Block {
    let span = paragraph[0].span.clone();
    let mut items: Vec<String> = Vec::new();
    for line in paragraph {
        let trimmed = line.text.trim_start();
        let bullet = if ordered {
            let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
            if digits > 0 && trimmed[digits..].starts_with('.') {
                Some(trimmed[digits + 1..].trim())
            } else {
                None
            }
        } else {
            trimmed.strip_prefix('-').map(str::trim)
        };
        match bullet {
            Some(text) => items.push(text.to_string()),
            None => {
                if let Some(last) = items.last_mut() {
                    last.push(' ');
                    last.push_str(trimmed.trim());
                }
            }
        }
    }
    Block::List {
        items,
        ordered,
        span,
    }
}

fn parse_paragraph(paragraph: &[RawLine]) -> Block {
    let span = paragraph[0].span.clone();
    let text = paragraph
        .iter()
        .map(|l| l.text.trim())
        .collect::<Vec<_>>()
        .join("\n");
    Block::Paragraph { text, span }
}

/// Leading `' '` count.
fn leading_ws(s: &str) -> usize {
    s.len() - s.trim_start_matches(' ').len()
}

struct RowHead<'a> {
    indent: u16,
    token_text: &'a str,
    desc_col: u16,
    first_desc: &'a str,
    /// Whether at least one separator character (whitespace or `=`) followed the token: a token
    /// that consumes the whole line has no real "description column" to continue from, so such
    /// a row's children may legitimately start at whatever column follows the token.
    has_separator: bool,
}

/// Splits a candidate row's raw text into its call token and description, per
/// `row = WS0, calltok, [WS, ["=", WS0], description]`.
fn split_row(text: &str) -> Option<RowHead<'_>> {
    let indent = leading_ws(text);
    let rest = &text[indent..];
    if rest.is_empty() {
        return None;
    }
    let token_len = rest.find(' ').unwrap_or(rest.len());
    if token_len == 0 {
        return None;
    }
    let token_text = &rest[..token_len];
    let mut after = &rest[token_len..];
    let mut col = indent + token_len;
    let ws1 = leading_ws(after);
    after = &after[ws1..];
    col += ws1;
    if let Some(stripped) = after.strip_prefix('=') {
        after = stripped;
        col += 1;
        let ws2 = leading_ws(after);
        after = &after[ws2..];
        col += ws2;
    }
    Some(RowHead {
        indent: indent as u16,
        token_text,
        desc_col: col as u16,
        first_desc: after,
        has_separator: col > indent + token_len,
    })
}

/// A leading `!` not followed by a lowercase suit letter is the alert marker (stripped from the
/// stored text); `!c`/`!d`/`!h`/`!s` at the very start is the suit-symbol notation instead.
/// Leading annotations (`{prio:5} !Foo`) are skipped first, the same way the description
/// normaliser skips them, and are kept in the stored text.
fn extract_alert(first_line: &str) -> (bool, String) {
    let lead = crate::compile::desc::normalize::leading_annotations_len(first_line);
    let (prefix, rest) = first_line.split_at(lead);
    if let Some(after) = rest.strip_prefix('!') {
        let is_suit_letter = after
            .chars()
            .next()
            .is_some_and(|c| matches!(c, 'c' | 'd' | 'h' | 's'));
        if !is_suit_letter {
            return (true, format!("{prefix}{after}"));
        }
    }
    (false, first_line.to_string())
}

fn row_span(line: &RawLine, col: u16) -> Span {
    Span {
        file: line.span.file,
        line: line.span.line,
        col,
        pasted_from: line.span.pasted_from.clone(),
    }
}

fn emit_nonstandard(lints: &mut Vec<Lint>, raw: &str, span: &Span) {
    for reason in call::nonstandard_reasons(raw) {
        lints.push(
            Lint::info(LintCode::NonStandardToken, format!("{raw}: {reason}"))
                .with_span(span.clone()),
        );
    }
}

struct Frame {
    indent: u16,
    node: BmlNode,
}

fn attach(stack: &mut [Frame], roots: &mut Vec<BmlNode>, node: BmlNode) {
    match stack.last_mut() {
        Some(parent) => parent.node.children.push(node),
        None => roots.push(node),
    }
}

/// Whether the whole paragraph is a single top-level `#CUT name … #ENDCUT` block (clipboard
/// only, no table). Still registers the block in `clipboard` for later `#PASTE`s.
fn try_whole_cut_block(paragraph: &[RawLine], clipboard: &mut Clipboard) -> Option<Block> {
    if paragraph.len() < 2 {
        return None;
    }
    let first = &paragraph[0];
    let trimmed_first = first.text.trim_start();
    let name = trimmed_first.strip_prefix("#CUT")?.trim();
    if name.is_empty() {
        return None;
    }
    let last = paragraph.last()?;
    if last.text.trim() != "#ENDCUT" {
        return None;
    }
    let indent = leading_ws(&first.text);
    let body = &paragraph[1..paragraph.len() - 1];
    // Several `#CUT … #ENDCUT` blocks in one paragraph: the first `#ENDCUT` ends the first
    // block, so this is not one whole block. `clipboard::expand` registers each of them.
    if body.iter().any(|l| l.text.trim() == "#ENDCUT") {
        return None;
    }
    let dedented: Vec<RawLine> = body
        .iter()
        .map(|l| RawLine {
            span: l.span.clone(),
            text: clipboard::dedent(&l.text, indent),
        })
        .collect();
    clipboard.insert(name, dedented);
    Some(Block::Clipboard {
        name: name.to_string(),
        lines: body.to_vec(),
    })
}

/// Where a continuation's text is currently being appended to.
enum ActiveDesc {
    None,
    History,
    Node,
}

/// Parses one bidding-table paragraph (possibly carrying embedded clipboard directives) into a
/// [`Block::BidTable`], or a [`Block::Clipboard`] when the whole paragraph is a top-level `#CUT`.
fn parse_table_paragraph(
    paragraph: Vec<RawLine>,
    clipboard: &mut Clipboard,
    seat: SeatCond,
    vul: VulCond,
    exact_pass_files: &mut ExactPassFiles,
    lints: &mut Vec<Lint>,
) -> Option<Block> {
    if let Some(block) = try_whole_cut_block(&paragraph, clipboard) {
        return Some(block);
    }

    let (expanded, clip_lints) = clipboard::expand(paragraph, clipboard);
    lints.extend(clip_lints);
    if expanded.is_empty() {
        return None;
    }
    let table_span = row_span(&expanded[0], leading_ws(&expanded[0].text) as u16);

    let mut hidden = false;
    let mut roots: Vec<BmlNode> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut history: Vec<CallToken> = Vec::new();
    let mut history_desc: Option<Description> = None;
    let mut active = ActiveDesc::None;
    let mut active_col: Option<u16> = None;
    let mut is_first_row = true;
    let mut table_unit: Option<u16> = None;
    let mut skip_indent: Option<u16> = None;
    let mut history_no_trailing = false;
    let mut table_stop = false;
    let mut any_order = false;
    let mut exact_pass: Option<Span> = None;
    let mut exact_pass_file: Option<Span> = None;
    let mut bidtable = false;

    for line in &expanded {
        let indent = leading_ws(&line.text) as u16;
        let trimmed = line.text.trim_start();

        if let Some(threshold) = skip_indent {
            if indent > threshold {
                continue;
            }
            skip_indent = None;
        }

        if trimmed.is_empty() {
            continue;
        }

        // Trailing whitespace after a directive keyword does not matter (as for `#ENDCUT`).
        if trimmed.trim_end() == "#HIDE" {
            hidden = true;
            continue;
        }
        if trimmed.trim_end() == "#BIDTABLE" {
            bidtable = true;
            continue;
        }
        if trimmed.trim_end() == "#ANYORDER" {
            // Table-scoped wherever it is written (a sub-row's indentation does not narrow it).
            any_order = true;
            continue;
        }
        if trimmed.trim_end() == "#EXACTPASS" {
            // Table-scoped wherever it is written, like `#ANYORDER`.
            exact_pass.get_or_insert_with(|| line.span.clone());
            continue;
        }
        if is_exact_pass_file(trimmed) {
            // Valid only as a paragraph of its own (checked once the paragraph is read).
            exact_pass_file.get_or_insert_with(|| line.span.clone());
            continue;
        }
        if trimmed.trim_end() == "#STOP" {
            // A system stop for the enclosing row, i.e. the nearest open row indented less than
            // the directive (exactly where a row at this indentation would be attached), or for
            // the history row's position at the table's top level.
            while let Some(top) = stack.last() {
                if top.indent >= indent {
                    let popped = stack.pop().expect("just checked non-empty");
                    attach(&mut stack, &mut roots, popped.node);
                } else {
                    break;
                }
            }
            match stack.last_mut() {
                Some(parent) => parent.node.stop = true,
                None => table_stop = true,
            }
            active = ActiveDesc::None;
            active_col = None;
            continue;
        }
        if trimmed.starts_with('#') {
            lints.push(
                Lint::warning(
                    LintCode::UnknownDirective,
                    format!("unknown directive: {trimmed}"),
                )
                .with_span(line.span.clone()),
            );
            continue;
        }

        if let Some(col) = active_col {
            if indent == col {
                // `bml.py` appends each continuation row with `row.strip()` (both `bml.py`'s
                // `lastnode.desc += '\n' + row.strip()` and the history-description equivalent):
                // trailing whitespace on a continuation line (e.g. an indented lone `.` row
                // followed by stray spaces) must not survive into the stored description, or a
                // later `\n.`-suffix check (dropping a trailing blank line) fails to match.
                let text_from_col = line.text[indent as usize..].trim_end();
                match active {
                    ActiveDesc::History => {
                        let d = history_desc.get_or_insert_with(|| Description {
                            text: String::new(),
                            alert: false,
                            col,
                        });
                        d.text.push('\n');
                        d.text.push_str(text_from_col);
                    }
                    ActiveDesc::Node => {
                        if let Some(frame) = stack.last_mut() {
                            frame.node.description.text.push('\n');
                            frame.node.description.text.push_str(text_from_col);
                        }
                    }
                    ActiveDesc::None => {}
                }
                continue;
            }
        }

        let Some(head) = split_row(&line.text) else {
            continue;
        };
        let token_text = head.token_text;
        let treat_as_history =
            is_first_row && (token_text.contains('-') || token_text.contains(';'));
        is_first_row = false;

        if treat_as_history {
            let mut s = token_text;
            match call::history(&mut s) {
                Ok(toks) if s.is_empty() => {
                    emit_nonstandard(lints, token_text, &line.span);
                    let span = row_span(line, head.indent);
                    history = toks
                        .into_iter()
                        .map(|(side, pattern)| CallToken {
                            side,
                            pattern,
                            raw: token_text.to_string(),
                            span: span.clone(),
                        })
                        .collect();
                    history_no_trailing = !(token_text.ends_with('-') || token_text.ends_with(';'));
                    if !head.first_desc.is_empty() {
                        let (alert, text) = extract_alert(head.first_desc);
                        history_desc = Some(Description {
                            text,
                            alert,
                            col: head.desc_col,
                        });
                    }
                    active = ActiveDesc::History;
                    active_col = head.has_separator.then_some(head.desc_col);
                }
                _ => {
                    // Every row of the table hangs off the history, usually at the same column,
                    // so the whole table goes: its rows must not be re-rooted as openings.
                    lints.push(
                        Lint::warning(
                            LintCode::UnknownCallToken,
                            format!(
                                "cannot parse history row {token_text:?}; skipping the whole table"
                            ),
                        )
                        .with_span(line.span.clone()),
                    );
                    return None;
                }
            }
            continue;
        }

        if token_text.contains('-') || token_text.contains(';') {
            lints.push(
                Lint::error(
                    LintCode::SequenceNotFirst,
                    format!("{token_text:?} is a sequence but is not the table's first row"),
                )
                .with_span(line.span.clone()),
            );
            skip_indent = Some(head.indent);
            active = ActiveDesc::None;
            active_col = None;
            continue;
        }

        let mut s = token_text;
        match call::calltok(&mut s) {
            Ok((side, pattern)) if s.is_empty() => {
                emit_nonstandard(lints, token_text, &line.span);
                let (alert, text) = extract_alert(head.first_desc);
                let span = row_span(line, head.indent);
                let node = BmlNode {
                    calls: vec![CallToken {
                        side,
                        pattern,
                        raw: token_text.to_string(),
                        span: span.clone(),
                    }],
                    description: Description {
                        text,
                        alert,
                        col: head.desc_col,
                    },
                    children: Vec::new(),
                    stop: false,
                    indent: head.indent,
                    span,
                };
                while let Some(top) = stack.last() {
                    if top.indent >= head.indent {
                        let popped = stack.pop().expect("just checked non-empty");
                        attach(&mut stack, &mut roots, popped.node);
                    } else {
                        break;
                    }
                }
                if let Some(parent) = stack.last() {
                    let diff = head.indent.saturating_sub(parent.indent);
                    match table_unit {
                        None => table_unit = Some(diff),
                        Some(u) if u != diff => {
                            lints.push(
                                Lint::warning(
                                    LintCode::IndentationMismatch,
                                    format!(
                                        "indent {} does not match the table's unit ({u})",
                                        head.indent
                                    ),
                                )
                                .with_span(line.span.clone()),
                            );
                        }
                        _ => {}
                    }
                }
                stack.push(Frame {
                    indent: head.indent,
                    node,
                });
                active = ActiveDesc::Node;
                active_col = head.has_separator.then_some(head.desc_col);
            }
            _ => {
                lints.push(
                    Lint::warning(
                        LintCode::UnknownCallToken,
                        format!(
                            "cannot parse call token {token_text:?}; skipping row and its subtree"
                        ),
                    )
                    .with_span(line.span.clone()),
                );
                skip_indent = Some(head.indent);
                active = ActiveDesc::None;
                active_col = None;
            }
        }
    }

    while let Some(frame) = stack.pop() {
        attach(&mut stack, &mut roots, frame.node);
    }

    if history_no_trailing {
        for root in roots.iter().filter(|r| r.indent == 0) {
            lints.push(
                Lint::info(
                    LintCode::ColumnZeroContinuation,
                    "column-0 row after a marker-less history row treated as its child",
                )
                .with_span(root.span.clone()),
            );
        }
    }

    if history.is_empty() && roots.is_empty() {
        // `#EXACTPASS FILE` is written this way: a paragraph of its own, opening its scope.
        if let Some(span) = exact_pass_file {
            exact_pass_files.open(span);
        }
        // A paragraph of table directives alone (`#ANYORDER`, `#EXACTPASS`, `#STOP`, `#HIDE` or
        // `#BIDTABLE` followed by a blank line, the way `#SEAT`/`#VUL` are written) names no
        // table: the directives would otherwise vanish without a trace while the table below
        // stays ordered / unguarded / unstopped.
        let mut orphans = Vec::new();
        if any_order {
            orphans.push("#ANYORDER");
        }
        if exact_pass.is_some() {
            orphans.push("#EXACTPASS");
        }
        if table_stop {
            orphans.push("#STOP");
        }
        if hidden {
            orphans.push("#HIDE");
        }
        if bidtable {
            orphans.push("#BIDTABLE");
        }
        if !orphans.is_empty() {
            lints.push(
                Lint::warning(
                    LintCode::UnknownDirective,
                    format!(
                        "{} outside a bidding table has no effect (write it inside the table's \
                         paragraph, with no blank line before the table{})",
                        orphans.join(" and "),
                        if exact_pass.is_some() {
                            "; `#EXACTPASS FILE` covers every later table of the file"
                        } else {
                            ""
                        }
                    ),
                )
                .with_span(table_span),
            );
        }
        return None;
    }
    if let Some(span) = &exact_pass_file {
        lints.push(
            Lint::warning(
                LintCode::UnknownDirective,
                "#EXACTPASS FILE inside a bidding table: write it as a paragraph of its own \
                 before the tables it covers (for this table alone, write #EXACTPASS); ignored",
            )
            .with_span(span.clone()),
        );
    }
    let after_their_pass = has_row_after_their_pass(&history, &roots);
    if let (Some(span), false) = (&exact_pass, after_their_pass) {
        lints.push(
            Lint::info(
                LintCode::ExactPassWithoutPass,
                "#EXACTPASS in a table with no row of ours right after an opponents' pass has no \
                 effect",
            )
            .with_span(span.clone()),
        );
    }
    let exact_pass = exact_pass_files
        .scope_of(table_span.file, after_their_pass)
        .or(exact_pass);
    if any_order && xyz_variables(&history, &roots) < 2 {
        lints.push(
            Lint::info(
                LintCode::AnyOrderWithoutVariables,
                "#ANYORDER in a table with fewer than two of the variables X, Y, Z has no effect",
            )
            .with_span(table_span.clone()),
        );
    }

    Some(Block::BidTable(BidTable {
        hidden,
        seat,
        vul,
        history,
        history_desc,
        rows: roots,
        stop: table_stop,
        any_order,
        exact_pass,
        span: table_span,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::{MemLoader, load};

    fn parse_str(text: &str) -> BmlFile {
        parse(load("root.bml", text, &MemLoader::default()))
    }

    /// Every bidding table as `(history raw, [row tree rendered as "call desc" with indent])`.
    fn tables(file: &BmlFile) -> Vec<(String, Vec<String>)> {
        fn walk(node: &BmlNode, depth: usize, out: &mut Vec<String>) {
            out.push(format!(
                "{}{} {}",
                "  ".repeat(depth),
                node.calls[0].raw,
                node.description.text
            ));
            for child in &node.children {
                walk(child, depth + 1, out);
            }
        }
        file.blocks
            .iter()
            .filter_map(|b| match b {
                Block::BidTable(t) => {
                    let history: Vec<&str> = t.history.iter().map(|c| c.raw.as_str()).collect();
                    let mut rows = Vec::new();
                    for row in &t.rows {
                        walk(row, 0, &mut rows);
                    }
                    Some((history.join(" "), rows))
                }
                _ => None,
            })
            .collect()
    }

    // Regression (integration review #2): `bml.py` stores the clipboard in a dict, so a later
    // `#CUT`/`#COPY` of the same name replaces the earlier body for every later `#PASTE`.
    #[test]
    fn redefined_clipboard_name_pastes_the_latest_body() {
        let file = parse_str(
            "#CUT resp\n2C Stayman\n#ENDCUT\n\n1N-\n#PASTE resp\n\n\
             #CUT resp\n2C Puppet Stayman\n#ENDCUT\n\n2N-\n#PASTE resp\n",
        );
        assert!(file.lints.is_empty(), "{:?}", file.lints);
        let t = tables(&file);
        assert_eq!(t.len(), 2, "{t:?}");
        assert_eq!(t[0].1, ["2C Stayman"]);
        assert_eq!(t[1].1, ["2C Puppet Stayman"]);
    }

    #[test]
    fn redefined_embedded_cut_replaces_the_earlier_body() {
        let file = parse_str(
            "1N-\n#CUT resp\n2C Stayman\n#ENDCUT\n#CUT resp\n2C Puppet\n#ENDCUT\n#PASTE resp\n",
        );
        assert!(file.lints.is_empty(), "{:?}", file.lints);
        assert_eq!(tables(&file)[0].1, ["2C Puppet"]);
    }

    // Regression (integration review #3): `bml.py` rescans the paragraph after every paste, so
    // a `#PASTE` inside a pasted body is expanded too (with the indentation accumulating).
    #[test]
    fn paste_inside_a_pasted_body_is_expanded() {
        let file = parse_str(
            "#CUT inner\n3C inner\n#ENDCUT\n\n\
             #CUT outer\n2D outer\n  #PASTE inner\n#ENDCUT\n\n\
             1N-\n#PASTE outer\n",
        );
        assert!(file.lints.is_empty(), "{:?}", file.lints);
        let t = tables(&file);
        assert_eq!(t.len(), 1, "{t:?}");
        assert_eq!(t[0].1, ["2D outer", "  3C inner"]);
    }

    #[test]
    fn self_referencing_paste_is_capped_with_a_warning() {
        let file = parse_str("#CUT loop\n2C again\n  #PASTE loop\n#ENDCUT\n\n1N-\n#PASTE loop\n");
        assert!(
            file.lints
                .iter()
                .any(|l| l.code == LintCode::UnknownDirective && l.message.contains("nested")),
            "{:?}",
            file.lints
        );
        assert!(!tables(&file).is_empty());
    }

    // Regression (integration review #4): `bml.py` accepts `#ENDCUT[ ]*` / `#ENDCOPY[ ]*`, so
    // trailing spaces after an end marker (or after `#HIDE`/`#BIDTABLE`) must not matter.
    #[test]
    fn end_markers_with_trailing_spaces_are_recognised() {
        let file =
            parse_str("#CUT resp\n2C  Stayman\n2D  Transfer\n#ENDCUT \n\n1N-\n#PASTE resp\n");
        assert!(file.lints.is_empty(), "{:?}", file.lints);
        let t = tables(&file);
        assert_eq!(t.len(), 1, "{t:?}");
        assert_eq!(t[0].0, "1N-");
        assert_eq!(t[0].1, ["2C Stayman", "2D Transfer"]);

        let file = parse_str(
            "1N-\n#COPY resp  \n2C  Stayman\n#ENDCOPY  \n\n2N-\n#HIDE \n#BIDTABLE \n#PASTE resp\n",
        );
        assert!(file.lints.is_empty(), "{:?}", file.lints);
        let t = tables(&file);
        assert_eq!(t[0].1, ["2C Stayman"]);
        assert_eq!(t[1].1, ["2C Stayman"]);
        assert!(matches!(&file.blocks[1], Block::BidTable(b) if b.hidden));
    }

    #[test]
    fn unterminated_cut_lint_has_a_location() {
        let file = parse_str("1N-\n#CUT resp\n2C  Stayman\n");
        let lint = file
            .lints
            .iter()
            .find(|l| l.message.contains("unterminated"))
            .expect("unterminated lint");
        assert_eq!(lint.span.as_ref().map(|s| s.line), Some(2));
    }

    #[test]
    fn any_order_is_a_table_directive() {
        let file = parse_str("#ANYORDER \n1X-(2Y)-\nD  10+ hcp\n\n1X-(3Y)-\nD  12+ hcp\n");
        assert!(file.lints.is_empty(), "{:?}", file.lints);
        let flags: Vec<bool> = file
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::BidTable(t) => Some(t.any_order),
                _ => None,
            })
            .collect();
        assert_eq!(flags, [true, false]);
        // Written under a row it still marks the table, and the row tree is unchanged.
        let file = parse_str("1X-(2Y)-\nD  10+ hcp\n  #ANYORDER\n  P  any\n");
        assert!(matches!(&file.blocks[0], Block::BidTable(b) if b.any_order));
        assert_eq!(tables(&file)[0].1, ["D 10+ hcp", "  P any"]);
    }

    /// The line of each table's `#EXACTPASS` in force (0 for none).
    fn exact_pass_lines(file: &BmlFile) -> Vec<u32> {
        file.blocks
            .iter()
            .filter_map(|b| match b {
                Block::BidTable(t) => Some(t.exact_pass.as_ref().map_or(0, |s| s.line)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn exact_pass_is_a_table_directive() {
        let file = parse_str("#EXACTPASS \n1C-\n1H  6+ hcp\n\n1D-\n1H  6+ hcp\n");
        assert!(file.lints.is_empty(), "{:?}", file.lints);
        assert_eq!(exact_pass_lines(&file), [1, 0]);
        // Written under a row it still marks the table, and the row tree is unchanged.
        let file = parse_str("1C-\n1H  6+ hcp\n  #EXACTPASS\n  1S  4+!s\n");
        assert!(file.lints.is_empty(), "{:?}", file.lints);
        assert_eq!(exact_pass_lines(&file), [3]);
        assert_eq!(tables(&file)[0].1, ["1H 6+ hcp", "  1S 4+!s"]);
    }

    #[test]
    fn exact_pass_file_covers_the_later_tables_of_its_file() {
        let file = parse_str(
            "1C-\n1H  6+ hcp\n\n#EXACTPASS   FILE\n\n1D-\n1H  6+ hcp\n\n\
             #EXACTPASS\n1H-\n1S  6+ hcp\n\n#EXACTPASS FILE\n\n1S-\n2S  raise\n",
        );
        assert!(file.lints.is_empty(), "{:?}", file.lints);
        // Not the table before it; the file scope wins over a table's own directive; a
        // second file directive changes nothing.
        assert_eq!(exact_pass_lines(&file), [0, 4, 4, 4]);
    }

    #[test]
    fn a_row_of_ours_after_their_pass_is_found() {
        let has = |text: &str| {
            let file = parse_str(text);
            let Some(Block::BidTable(t)) = file.blocks.first() else {
                panic!("{text}");
            };
            has_row_after_their_pass(&t.history, &t.rows)
        };
        assert!(has("1C-\n1H  x\n"));
        assert!(has("1C-(P)-\n1H  x\n"));
        assert!(has("1C-(1S)-\nX  x\n  2C  x\n"));
        assert!(has("1C-1H-\n(P/1S)\n  1N  x\n"));
        assert!(!has("1C  x\n"));
        assert!(!has("1C-(1S)-\nX  x\n"));
        assert!(!has("1C-(any)-\nX  x\n"));
        assert!(!has("1C-\n(1S)\n  X  x\n"));
    }

    #[test]
    fn exact_pass_misuse_is_reported() {
        let codes = |text: &str| -> Vec<LintCode> {
            parse_str(text).lints.iter().map(|l| l.code).collect()
        };
        // No row of ours after a pass of theirs.
        assert_eq!(
            codes("#EXACTPASS\n1C-(1S)-\nD  x\n"),
            [LintCode::ExactPassWithoutPass]
        );
        assert_eq!(
            codes("#EXACTPASS\n1C  x\n"),
            [LintCode::ExactPassWithoutPass]
        );
        assert_eq!(
            codes("#EXACTPASS FILE\n\n1C  x\n\n1C-(1S)-\nD  x\n"),
            [LintCode::ExactPassWithoutPass]
        );
        assert_eq!(codes("#EXACTPASS FILE\n"), [LintCode::ExactPassWithoutPass]);
        // The table form alone in its paragraph names no table, and the file is not covered.
        let file = parse_str("#EXACTPASS\n\n1C-\n1H  x\n");
        assert_eq!(
            file.lints.iter().map(|l| l.code).collect::<Vec<_>>(),
            [LintCode::UnknownDirective]
        );
        assert!(file.lints[0].message.contains("#EXACTPASS FILE"));
        assert_eq!(exact_pass_lines(&file), [0]);
        // The file form inside a table is ignored.
        let file = parse_str("1C-\n#EXACTPASS FILE\n1H  x\n");
        assert_eq!(
            file.lints.iter().map(|l| l.code).collect::<Vec<_>>(),
            [LintCode::UnknownDirective]
        );
        assert_eq!(exact_pass_lines(&file), [0]);
        // Other words are not the directive.
        assert_eq!(
            codes("1C-\n#EXACTPASS ALL\n1H  x\n"),
            [LintCode::UnknownDirective]
        );
    }

    #[test]
    fn exact_pass_file_in_a_seat_vul_or_meta_paragraph_opens_no_scope() {
        let table = "\n\n1C-\n1H  x\n";
        for (paragraph, head) in [
            ("#SEAT 3\n#EXACTPASS FILE", "#SEAT"),
            ("#SEAT 3\n  #EXACTPASS   FILE  ", "#SEAT"),
            ("#VUL YN\n#EXACTPASS FILE", "#VUL"),
        ] {
            let file = parse_str(&format!("{paragraph}{table}"));
            let lints: Vec<(LintCode, u32)> = file
                .lints
                .iter()
                .map(|l| (l.code, l.span.as_ref().map_or(0, |s| s.line)))
                .collect();
            assert_eq!(lints, [(LintCode::UnknownDirective, 2)], "{paragraph:?}");
            assert!(file.lints[0].message.contains(head), "{paragraph:?}");
            assert_eq!(exact_pass_lines(&file), [0], "{paragraph:?}");
        }
        // The first line still sets the condition, and a `#SEAT` / `#VUL` paragraph without
        // the line, or with other lines (dropped silently), has no lint.
        let file = parse_str("#SEAT 3\n1C-\n1H  x\n\n1D-\n1H  x\n");
        assert!(file.lints.is_empty(), "{:?}", file.lints);
        let file = parse_str("#SEAT 3\n#EXACTPASS FILE\n\n1C-\n1H  x\n");
        assert!(matches!(
            file.blocks[0],
            Block::Seat {
                cond: SeatCond::Third,
                ..
            }
        ));
        // A meta paragraph reports every line that is not `#+KEY: value`, this one included.
        let file = parse_str(&format!("#+TITLE: x\n#EXACTPASS FILE{table}"));
        assert_eq!(
            file.lints.iter().map(|l| l.code).collect::<Vec<_>>(),
            [LintCode::UnknownDirective]
        );
        assert!(file.lints[0].message.contains("malformed meta line"));
        assert_eq!(exact_pass_lines(&file), [0]);
        // In a directive paragraph of its own, anywhere in it, the line opens the scope.
        let file = parse_str(&format!("#HIDE\n#EXACTPASS FILE{table}"));
        assert_eq!(exact_pass_lines(&file), [2]);
    }

    #[test]
    fn any_order_without_two_variables_is_reported() {
        let file = parse_str("#ANYORDER\n1X-\n2X  raise\n");
        let codes: Vec<LintCode> = file.lints.iter().map(|l| l.code).collect();
        assert_eq!(codes, [LintCode::AnyOrderWithoutVariables]);
        let file = parse_str("#ANYORDER\n1X-\n2Y  new suit\n");
        assert!(file.lints.is_empty(), "{:?}", file.lints);
    }
}
