//! Bytes to text, shared by the PBN and LIN readers.

/// Decodes as UTF-8, falling back to ISO-8859-1, and normalises line endings to LF. The
/// second value is the line of the first invalid byte when the fallback was used.
pub(crate) fn decode(input: &[u8]) -> (String, Option<u32>) {
    let input = input.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(input);
    match core::str::from_utf8(input) {
        Ok(s) => (normalize_newlines(s), None),
        Err(e) => {
            let line = input[..e.valid_up_to()]
                .iter()
                .filter(|&&b| b == b'\n')
                .count() as u32
                + 1;
            let latin1: String = input.iter().map(|&b| char::from(b)).collect();
            (normalize_newlines(&latin1), Some(line))
        }
    }
}

/// CRLF and lone CR become LF; a leading byte-order mark is dropped.
pub(crate) fn normalize_newlines(s: &str) -> String {
    let s = s.strip_prefix('\u{FEFF}').unwrap_or(s);
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// 1-based line of a byte offset.
pub(crate) fn line_of(text: &str, offset: usize) -> u32 {
    text.as_bytes()[..offset.min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count() as u32
        + 1
}
