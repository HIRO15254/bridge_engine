//! Normalisation of description text.
//!
//! Strips the leading alert marker (`!` not immediately followed by a lowercase suit letter),
//! extracts the `{prio:N}` / `{w:X}` / `{stop}` annotations, maps the lowercase suit digraphs `!c !d
//! !h !s` to suit sentinel characters (`♣ ♦ ♥ ♠`), collapses `--` to `-` (en-dash ranges), and
//! collapses runs of horizontal whitespace to a single space while keeping line breaks (each
//! physical line becomes one `clause.rs` line, used for enumerations).
//!
//! [`Normalized::offsets`] maps every byte of [`Normalized::text`] back to the byte offset it
//! came from in the original (pre-normalisation) text, with one extra trailing entry equal to
//! the original text's length. `clause.rs` spans are byte ranges into [`Normalized::text`]; a
//! caller wanting the original location looks up `offsets[start]..offsets[end]` (best-effort:
//! spans that straddle a `--` → `-` collapse or a suit-sentinel substitution resolve to the
//! first original byte of that substitution).

/// A normalised description.
#[derive(Clone, PartialEq, Debug)]
pub struct Normalized {
    /// Text with suit sentinels (`♣♦♥♠`), `-` ranges and collapsed spaces; line breaks kept.
    pub text: String,
    /// A leading `!` marker was present.
    pub alert: bool,
    /// `{prio:N}`.
    pub priority: Option<i16>,
    /// `{w:X}` values in order of appearance.
    pub weights: Vec<f32>,
    /// `{stop}`: the row is a system stop (`docs/design/06-system.md` §4.5).
    pub stop: bool,
    /// Map from normalised byte offsets back to original offsets (for spans). One entry per
    /// byte of `text`, plus a trailing sentinel equal to the original text's length.
    pub offsets: Vec<u16>,
}

/// One annotation of a description.
enum Annotation {
    /// `{prio:N}`.
    Priority(i16),
    /// `{w:X}`.
    Weight(f32),
    /// `{stop}` (spaces inside the braces allowed).
    Stop,
}

/// The annotation starting at byte `i` of `text` (a `{`) and its length in bytes, if the braces
/// hold one; any other braced text is ordinary description text.
fn annotation_at(text: &str, i: usize) -> Option<(Annotation, usize)> {
    let rel_end = text[i..].find('}')?;
    let inner = &text[i + 1..i + rel_end];
    let annotation = if let Some(rest) = inner.strip_prefix("prio:") {
        Annotation::Priority(rest.trim().parse::<i16>().ok()?)
    } else if let Some(rest) = inner.strip_prefix("w:") {
        Annotation::Weight(rest.trim().parse::<f32>().ok()?)
    } else if inner.trim() == "stop" {
        Annotation::Stop
    } else {
        return None;
    };
    Some((annotation, rel_end + 1))
}

/// `text` without its `{prio:N}` / `{w:X}` / `{stop}` annotations (what [`normalize`] extracts
/// into [`Normalized::priority`], [`Normalized::weights`] and [`Normalized::stop`]), for display.
/// Nothing else changes: suit digraphs, the alert marker and spacing inside the text are kept;
/// the spaces an annotation leaves at either end are trimmed, and so are the spaces after an
/// annotation that follows a space. Borrows `text` when it has no annotation.
pub fn strip_annotations(text: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    if !text.contains('{') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    let mut from = 0usize;
    let mut stripped = false;
    while let Some(rel) = text[from..].find('{') {
        let at = from + rel;
        let Some((_, len)) = annotation_at(text, at) else {
            from = at + 1;
            continue;
        };
        out.push_str(&text[last..at]);
        let mut end = at + len;
        if out.is_empty() || out.ends_with([' ', '\t']) {
            while text[end..].starts_with([' ', '\t']) {
                end += 1;
            }
        }
        last = end;
        from = end;
        stripped = true;
    }
    if !stripped {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[last..]);
    let trimmed = out.trim();
    if trimmed.len() == out.len() {
        Cow::Owned(out)
    } else {
        Cow::Owned(trimmed.to_string())
    }
}

/// Normalises `text`.
pub fn normalize(text: &str) -> Normalized {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut offsets: Vec<u16> = Vec::with_capacity(text.len() + 1);
    let mut priority: Option<i16> = None;
    let mut weights: Vec<f32> = Vec::new();
    let mut stop = false;
    let mut alert = false;
    let mut started = false;
    let mut last_was_space = false;
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];

        // `{prio:N}` / `{w:X}` / `{stop}` annotations, anywhere in the text.
        if b == b'{' {
            if let Some((annotation, len)) = annotation_at(text, i) {
                match annotation {
                    Annotation::Priority(n) => priority = Some(n),
                    Annotation::Weight(w) => weights.push(w),
                    Annotation::Stop => stop = true,
                }
                i += len;
                continue;
            }
        }

        // Leading alert marker: a `!` at the very start of the (non-whitespace) text that is not
        // immediately followed by a lowercase suit letter.
        if !started && b == b'!' && !matches!(bytes.get(i + 1), Some(b'c' | b'd' | b'h' | b's')) {
            alert = true;
            i += 1;
            continue;
        }

        // Suit sentinel digraphs `!c !d !h !s` (lowercase only).
        if b == b'!' {
            let sentinel = match bytes.get(i + 1) {
                Some(b'c') => Some('♣'),
                Some(b'd') => Some('♦'),
                Some(b'h') => Some('♥'),
                Some(b's') => Some('♠'),
                _ => None,
            };
            if let Some(sentinel) = sentinel {
                for _ in 0..sentinel.len_utf8() {
                    offsets.push(i as u16);
                }
                out.push(sentinel);
                i += 2;
                started = true;
                last_was_space = false;
                continue;
            }
        }

        // `--` (en-dash range) → `-`.
        if b == b'-' && bytes.get(i + 1) == Some(&b'-') {
            out.push('-');
            offsets.push(i as u16);
            i += 2;
            started = true;
            last_was_space = false;
            continue;
        }

        // Horizontal whitespace: collapse runs to a single space, drop leading whitespace.
        if b == b' ' || b == b'\t' || b == b'\r' {
            if !started || last_was_space {
                i += 1;
                continue;
            }
            out.push(' ');
            offsets.push(i as u16);
            last_was_space = true;
            i += 1;
            continue;
        }

        // Line break: trim trailing space before it, keep it, treat as a space for collapsing.
        if b == b'\n' {
            if out.ends_with(' ') {
                out.pop();
                offsets.pop();
            }
            out.push('\n');
            offsets.push(i as u16);
            last_was_space = true;
            started = true;
            i += 1;
            continue;
        }

        // Default: copy the character (may be multi-byte UTF-8).
        let ch = text[i..].chars().next().expect("i is a char boundary");
        let len = ch.len_utf8();
        for _ in 0..len {
            offsets.push(i as u16);
        }
        out.push(ch);
        i += len;
        started = true;
        last_was_space = false;
    }

    // Trim trailing whitespace collapsed above.
    while out.ends_with(' ') || out.ends_with('\n') {
        out.pop();
        offsets.pop();
    }

    offsets.push(text.len().min(u16::MAX as usize) as u16);

    Normalized {
        text: out,
        alert,
        priority,
        weights,
        stop,
        offsets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_annotations_keeps_the_text() {
        for (text, want) in [
            ("{prio:-100} {stop} any hand", "any hand"),
            ("{prio:-100} any hand", "any hand"),
            ("any hand {prio:-100}", "any hand"),
            ("5+!s {w:0.5} or 6+!h {w:0.5}", "5+!s or 6+!h"),
            ("{ stop } 0+ hcp", "0+ hcp"),
            ("6--9 hcp{prio:5}, 4+!h", "6--9 hcp, 4+!h"),
            ("!GF, {x} braces stay", "!GF, {x} braces stay"),
            (
                "{prio:abc} is not an annotation",
                "{prio:abc} is not an annotation",
            ),
            ("{prio:3}", ""),
        ] {
            assert_eq!(strip_annotations(text), want, "{text:?}");
        }
        assert!(matches!(
            strip_annotations("12+ hcp"),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn alert_marker_stripped() {
        let n = normalize("!FG, 5+!c");
        assert!(n.alert);
        assert_eq!(n.text, "FG, 5+♣");
    }

    #[test]
    fn suit_sentinel_not_alert() {
        let n = normalize("!c!d!h!s");
        assert!(!n.alert);
        assert_eq!(n.text, "♣♦♥♠");
    }

    #[test]
    fn en_dash_collapsed() {
        let n = normalize("12--14 HCP");
        assert_eq!(n.text, "12-14 HCP");
    }

    #[test]
    fn whitespace_collapsed_newline_kept() {
        let n = normalize("a)  weak-two   in a major\n  b) 22-24 NT");
        assert_eq!(n.text, "a) weak-two in a major\nb) 22-24 NT");
    }

    #[test]
    fn priority_and_weight_extracted() {
        let s = normalize("{prio:-100} {stop} any hand");
        assert!(s.stop);
        assert_eq!(s.priority, Some(-100));
        assert_eq!(s.text, "any hand");
        assert!(!normalize("any hand {stopper}").stop);
        let n = normalize("5+!c or 4+!h {w:0.6} {prio:3}");
        assert_eq!(n.priority, Some(3));
        assert_eq!(n.weights, vec![0.6]);
        assert_eq!(n.text, "5+♣ or 4+♥");
    }

    #[test]
    fn empty_description() {
        let n = normalize("");
        assert_eq!(n.text, "");
        assert!(!n.alert);
        assert_eq!(*n.offsets.last().unwrap(), 0);
    }

    #[test]
    fn offsets_len_matches_text_plus_one() {
        let n = normalize("!FG, 5+!c and 6+!d");
        assert_eq!(n.offsets.len(), n.text.len() + 1);
    }
}
