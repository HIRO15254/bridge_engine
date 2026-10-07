//! `#COPY` / `#CUT` / `#PASTE` (textual, in the reference implementation's order).

use std::sync::Arc;

use crate::{Lint, LintCode, ast::RawLine};

/// Named blocks available to `#PASTE` (global and order-dependent across the file).
#[derive(Clone, Debug, Default)]
pub struct Clipboard {
    /// `name → lines`.
    pub blocks: Vec<(String, Vec<RawLine>)>,
}

impl Clipboard {
    /// Stores `body` under `name`. Like `bml.py`'s `content.clipboard[name] = value` (a dict), a
    /// redefinition replaces the earlier body in place, so every later `#PASTE` gets the latest
    /// one.
    pub fn insert(&mut self, name: &str, body: Vec<RawLine>) {
        match self.blocks.iter_mut().find(|(n, _)| n == name) {
            Some((_, lines)) => *lines = body,
            None => self.blocks.push((name.to_string(), body)),
        }
    }

    fn find(&self, name: &str) -> Option<&Vec<RawLine>> {
        self.blocks
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, lines)| lines)
    }
}

/// Leading `' '` count.
fn indent_of(line: &RawLine) -> usize {
    line.text.len() - line.text.trim_start_matches(' ').len()
}

/// Removes up to `n` leading spaces.
pub(crate) fn dedent(text: &str, n: usize) -> String {
    let mut removed = 0;
    let mut chars = text.chars();
    while removed < n {
        match chars.clone().next() {
            Some(' ') => {
                chars.next();
                removed += 1;
            }
            _ => break,
        }
    }
    chars.collect()
}

/// The first whitespace-separated word of `text`, and the rest (trimmed of one leading space).
fn first_word(text: &str) -> (&str, &str) {
    let trimmed = text.trim_start();
    match trimmed.find(char::is_whitespace) {
        Some(i) => (&trimmed[..i], trimmed[i..].trim_start()),
        None => (trimmed, ""),
    }
}

/// Extracts every `#START name … #END` block, storing the dedented body under `name`.
/// `keep_body`: `#COPY` leaves the body in the output paragraph, `#CUT` removes it entirely.
fn extract_blocks(
    lines: Vec<RawLine>,
    start_kw: &str,
    end_kw: &str,
    keep_body: bool,
    clipboard: &mut Clipboard,
    lints: &mut Vec<Lint>,
) -> Vec<RawLine> {
    let mut out = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        let trimmed = line.text.trim_start();
        if trimmed.trim_end() == start_kw || trimmed.starts_with(&format!("{start_kw} ")) {
            let (_, name) = first_word(trimmed);
            let name = name.trim();
            if name.is_empty() {
                out.push(line.clone());
                i += 1;
                continue;
            }
            let indent = indent_of(line);
            // Find the matching #END line.
            let mut end = None;
            for (j, candidate) in lines.iter().enumerate().skip(i + 1) {
                // `bml.py` accepts `#ENDCUT[ ]*`: trailing whitespace does not matter.
                if candidate.text.trim() == end_kw {
                    end = Some(j);
                    break;
                }
            }
            match end {
                Some(end_idx) => {
                    let body: Vec<RawLine> = lines[i + 1..end_idx]
                        .iter()
                        .map(|l| RawLine {
                            span: l.span.clone(),
                            text: dedent(&l.text, indent),
                        })
                        .collect();
                    if keep_body {
                        out.extend(lines[i + 1..end_idx].iter().cloned());
                    }
                    clipboard.insert(name, body);
                    i = end_idx + 1;
                }
                None => {
                    lints.push(
                        Lint::warning(
                            LintCode::UnknownDirective,
                            format!("unterminated {start_kw} {name} (no matching {end_kw})"),
                        )
                        .with_span(line.span.clone()),
                    );
                    i += 1;
                }
            }
            continue;
        }
        out.push(line.clone());
        i += 1;
    }
    out
}

/// Applies the `tgt=rep …` substitutions given after the clipboard name, in order.
fn apply_replacements(text: &str, replacements: &[(String, String)]) -> String {
    let mut result = text.to_string();
    for (tgt, rep) in replacements {
        result = result.replace(tgt.as_str(), rep.as_str());
    }
    result
}

/// The clipboard name and the `tgt=rep` substitutions after it. An argument with no `=`, or
/// with an empty target (`=x`, which would insert `x` between every character), is dropped with
/// a Warning.
fn parse_paste_args(
    rest: &str,
    span: &crate::ast::Span,
    lints: &mut Vec<Lint>,
) -> (String, Vec<(String, String)>) {
    let mut tokens = rest.split_whitespace();
    let name = tokens.next().unwrap_or("").to_string();
    let mut replacements = Vec::new();
    for tok in tokens {
        match tok.split_once('=') {
            Some((tgt, rep)) if !tgt.is_empty() => {
                replacements.push((tgt.to_string(), rep.to_string()));
            }
            _ => lints.push(
                Lint::warning(
                    LintCode::UnknownDirective,
                    format!(
                        "#PASTE {name}: argument {tok:?} is not a `target=replacement` \
                         substitution with a non-empty target; ignored"
                    ),
                )
                .with_span(span.clone()),
            ),
        }
    }
    (name, replacements)
}

/// Nested `#PASTE` (a pasted body that itself pastes another block) is expanded at most this
/// many levels deep; a deeper chain can only come from a block that (indirectly) pastes itself.
const MAX_PASTE_DEPTH: usize = 16;

fn is_paste_line(line: &RawLine) -> bool {
    let trimmed = line.text.trim_start();
    trimmed.trim_end() == "#PASTE" || trimmed.starts_with("#PASTE ")
}

/// Expands every `#PASTE`, then rescans the result, the way `bml.py`'s `while True:
/// re.search('#PASTE ...')` loop over the whole paragraph does: a `#PASTE` inside a pasted body
/// is expanded too, its indentation accumulating on top of the outer paste's. Capped at
/// [`MAX_PASTE_DEPTH`] rounds (a self-referencing block would otherwise never end); any
/// `#PASTE` still left then is dropped with a Warning.
fn expand_pastes(
    mut lines: Vec<RawLine>,
    clipboard: &Clipboard,
    lints: &mut Vec<Lint>,
) -> Vec<RawLine> {
    for _ in 0..MAX_PASTE_DEPTH {
        if !lines.iter().any(is_paste_line) {
            return lines;
        }
        lines = expand_pastes_once(lines, clipboard, lints);
    }
    lines
        .into_iter()
        .filter(|line| {
            if !is_paste_line(line) {
                return true;
            }
            lints.push(
                Lint::warning(
                    LintCode::UnknownDirective,
                    format!(
                        "{}: nested #PASTE deeper than {MAX_PASTE_DEPTH} levels (a block that \
                         pastes itself?); dropped",
                        line.text.trim()
                    ),
                )
                .with_span(line.span.clone()),
            );
            false
        })
        .collect()
}

fn expand_pastes_once(
    lines: Vec<RawLine>,
    clipboard: &Clipboard,
    lints: &mut Vec<Lint>,
) -> Vec<RawLine> {
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let trimmed = line.text.trim_start();
        if is_paste_line(&line) {
            let (_, rest) = first_word(trimmed);
            let (name, replacements) = parse_paste_args(rest, &line.span, lints);
            let indent = " ".repeat(indent_of(&line));
            match clipboard.find(&name) {
                Some(body) => {
                    for src in body.iter() {
                        let text =
                            format!("{indent}{}", apply_replacements(&src.text, &replacements));
                        out.push(RawLine {
                            span: crate::ast::Span {
                                file: line.span.file,
                                line: line.span.line,
                                col: 0,
                                pasted_from: Some((Arc::from(name.as_str()), src.span.line)),
                            },
                            text,
                        });
                    }
                }
                None => {
                    lints.push(
                        Lint::warning(
                            LintCode::PasteUnknownName,
                            format!("#PASTE {name}: no such clipboard block"),
                        )
                        .with_span(line.span.clone()),
                    );
                }
            }
            continue;
        }
        out.push(line);
    }
    out
}

/// Expands clipboard directives inside one table paragraph: extract `#CUT` blocks, then `#COPY`
/// blocks (keeping their bodies), then expand every `#PASTE name tgt=rep …` by prepending the
/// paste line's indentation and applying the replacements in order.
pub fn expand(paragraph: Vec<RawLine>, clipboard: &mut Clipboard) -> (Vec<RawLine>, Vec<Lint>) {
    let mut lints = Vec::new();
    let after_cut = extract_blocks(paragraph, "#CUT", "#ENDCUT", false, clipboard, &mut lints);
    let after_copy = extract_blocks(after_cut, "#COPY", "#ENDCOPY", true, clipboard, &mut lints);
    let result = expand_pastes(after_copy, clipboard, &mut lints);
    (result, lints)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Span;

    fn line(text: &str) -> RawLine {
        RawLine {
            span: Span {
                file: crate::lexer::ROOT,
                line: 1,
                col: 0,
                pasted_from: None,
            },
            text: text.to_string(),
        }
    }

    #[test]
    fn cut_is_removed_and_stored() {
        let lines = vec![
            line("#CUT foo"),
            line("2C Stayman"),
            line("#ENDCUT"),
            line("1N-"),
        ];
        let mut clip = Clipboard::default();
        let (out, lints) = expand(lines, &mut clip);
        assert!(lints.is_empty());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "1N-");
        assert_eq!(clip.blocks.len(), 1);
        assert_eq!(clip.blocks[0].0, "foo");
        assert_eq!(clip.blocks[0].1[0].text, "2C Stayman");
    }

    #[test]
    fn copy_keeps_body_and_stores() {
        let lines = vec![line("#COPY foo"), line("2C Stayman"), line("#ENDCOPY")];
        let mut clip = Clipboard::default();
        let (out, lints) = expand(lines, &mut clip);
        assert!(lints.is_empty());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "2C Stayman");
        assert_eq!(clip.blocks.len(), 1);
    }

    #[test]
    fn paste_reindents_and_substitutes() {
        let mut clip = Clipboard::default();
        clip.blocks.push((
            "transfer".to_string(),
            vec![line("2\\R Transfer"), line("  2\\M Transfer accept")],
        ));
        let lines = vec![line("#PASTE transfer \\R=D \\M=H")];
        let (out, lints) = expand(lines, &mut clip);
        assert!(lints.is_empty());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "2D Transfer");
        assert_eq!(out[1].text, "  2H Transfer accept");
    }

    #[test]
    fn paste_unknown_name_is_warning() {
        let mut clip = Clipboard::default();
        let lines = vec![line("#PASTE nope")];
        let (out, lints) = expand(lines, &mut clip);
        assert!(out.is_empty());
        assert_eq!(lints.len(), 1);
        assert_eq!(lints[0].code, LintCode::PasteUnknownName);
    }

    #[test]
    fn paste_with_indentation_prefixes_body() {
        let mut clip = Clipboard::default();
        clip.blocks
            .push(("foo".to_string(), vec![line("2C Stayman")]));
        let lines = vec![line("  #PASTE foo")];
        let (out, _) = expand(lines, &mut clip);
        assert_eq!(out[0].text, "  2C Stayman");
    }
}
