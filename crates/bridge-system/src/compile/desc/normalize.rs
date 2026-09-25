//! Normalisation of description text.
//!
//! Strips the leading alert marker (`!` not immediately followed by a lowercase suit letter),
//! extracts trailing `{prio:N}` / `{w:X}` annotations, maps the lowercase suit digraphs `!c !d
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
    /// Map from normalised byte offsets back to original offsets (for spans). One entry per
    /// byte of `text`, plus a trailing sentinel equal to the original text's length.
    pub offsets: Vec<u16>,
}

/// Normalises `text`.
pub fn normalize(text: &str) -> Normalized {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut offsets: Vec<u16> = Vec::with_capacity(text.len() + 1);
    let mut priority: Option<i16> = None;
    let mut weights: Vec<f32> = Vec::new();
    let mut alert = false;
    let mut started = false;
    let mut last_was_space = false;
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];

        // `{prio:N}` / `{w:X}` annotations, anywhere in the text.
        if b == b'{' {
            if let Some(rel_end) = text[i..].find('}') {
                let inner = &text[i + 1..i + rel_end];
                let mut consumed = false;
                if let Some(rest) = inner.strip_prefix("prio:") {
                    if let Ok(n) = rest.trim().parse::<i16>() {
                        priority = Some(n);
                        consumed = true;
                    }
                } else if let Some(rest) = inner.strip_prefix("w:") {
                    if let Ok(w) = rest.trim().parse::<f32>() {
                        weights.push(w);
                        consumed = true;
                    }
                }
                if consumed {
                    i += rel_end + 1;
                    continue;
                }
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
        offsets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
