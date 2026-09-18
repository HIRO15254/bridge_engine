//! PBN parsing (winnow, line oriented).
//!
//! 1. Bytes → text: UTF-8, falling back to ISO-8859-1; CRLF normalised.
//! 2. `%` in column 1 is a directive.
//! 3. `;` to end of line and `{…}` (non-nesting, multi-line) are comments.
//! 4. Games are separated by empty lines outside brace comments.
//! 5. Tag pairs `[Name "value"]` with `\\` and `\"` escapes, `#` and `##x` values; import mode
//!    allows several tags per line and tags spanning lines.
//! 6. `Auction`, `Play` and `*Table` tags own the following non-`[` lines as tokens; `[Note]`
//!    tags attach to the preceding section (notes are numbered per section).
//! 7. Auction tokens: bids, `Pass|P`, `X`, `XX`, `AP`, `-`, `*`, `+`, `=n=`, `$n`, suffixes.
//! 8. Play tokens: `SA`-style cards, `-`, `*`, `+`, notes, NAGs, suffixes.

use crate::{ParseError, Warning, pbn::PbnFile};

/// Parses as much as possible; never fails and never panics.
pub fn parse_lenient(input: &[u8]) -> (PbnFile, Vec<Warning>) {
    todo!("phase 1")
}

/// Parses an export-format file, failing on the first violation.
pub fn parse_strict(input: &str) -> Result<PbnFile, ParseError> {
    todo!("phase 1")
}
