//! Paragraph classification, clipboard expansion and bidding-table tree building.
//!
//! Every stage records failures as [`Lint`]s and keeps going: an unparsable call
//! token skips that row and its subtree; an indentation that matches no open ancestor attaches
//! the row to the nearest shallower one; a row containing `-` or `;` that is not the first row
//! of its table is dropped.

pub mod call;
pub mod clipboard;

use crate::{
    Lint, LintCode,
    ast::{
        BidTable, Block, BmlFile, BmlNode, CallToken, Description, RawLine, SeatCond, Span, Tri,
        VulCond,
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

    for paragraph in paragraphs {
        match classify(&paragraph) {
            ParagraphKind::Heading => blocks.push(parse_heading(&paragraph)),
            ParagraphKind::List => blocks.push(parse_list(&paragraph, false)),
            ParagraphKind::Enumeration => blocks.push(parse_list(&paragraph, true)),
            ParagraphKind::Seat => {
                seat = parse_seat(&paragraph, &mut lints);
                blocks.push(Block::Seat {
                    cond: seat,
                    span: paragraph[0].span.clone(),
                });
            }
            ParagraphKind::Vul => {
                vul = parse_vul(&paragraph, &mut lints);
                blocks.push(Block::Vul {
                    cond: vul,
                    span: paragraph[0].span.clone(),
                });
            }
            ParagraphKind::Meta => parse_meta(&paragraph, &mut lints, &mut blocks),
            ParagraphKind::BidTable | ParagraphKind::Directive => {
                if let Some(block) =
                    parse_table_paragraph(paragraph, &mut clipboard, seat, vul, &mut lints)
                {
                    blocks.push(block);
                }
            }
            ParagraphKind::Paragraph => blocks.push(parse_paragraph(&paragraph)),
        }
    }

    BmlFile {
        root: crate::lexer::ROOT,
        files: loaded.files,
        blocks,
        lints,
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
    call::calltok(&mut s).is_ok() && s.is_empty()
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
fn extract_alert(first_line: &str) -> (bool, String) {
    if let Some(rest) = first_line.strip_prefix('!') {
        let is_suit_letter = rest
            .chars()
            .next()
            .is_some_and(|c| matches!(c, 'c' | 'd' | 'h' | 's'));
        if !is_suit_letter {
            return (true, rest.to_string());
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
    if last.text.trim_start() != "#ENDCUT" {
        return None;
    }
    let indent = leading_ws(&first.text);
    let body = &paragraph[1..paragraph.len() - 1];
    let dedented: Vec<RawLine> = body
        .iter()
        .map(|l| RawLine {
            span: l.span.clone(),
            text: clipboard::dedent(&l.text, indent),
        })
        .collect();
    clipboard.blocks.push((name.to_string(), dedented));
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

        if trimmed == "#HIDE" {
            hidden = true;
            continue;
        }
        if trimmed == "#BIDTABLE" {
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
                let text_from_col = &line.text[indent as usize..];
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
                    lints.push(
                        Lint::warning(
                            LintCode::UnknownCallToken,
                            format!(
                                "cannot parse history row {token_text:?}; skipping row and its subtree"
                            ),
                        )
                        .with_span(line.span.clone()),
                    );
                    skip_indent = Some(head.indent);
                    active = ActiveDesc::None;
                    active_col = None;
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
        if let Some(root) = roots.iter().find(|r| r.indent == 0) {
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
        return None;
    }

    Some(Block::BidTable(BidTable {
        hidden,
        seat,
        vul,
        history,
        history_desc,
        rows: roots,
        span: table_span,
    }))
}
