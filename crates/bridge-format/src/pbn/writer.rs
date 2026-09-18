//! PBN writer.

use crate::pbn::PbnFile;

/// Writer options.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WriteOptions {
    /// Export format: `% PBN 2.1`, `% EXPORT`, mandatory tags first in fixed order, one tag per
    /// line, CRLF, uppercase, ranks descending, `=n=` before `$n`, suffixes as NAGs.
    pub export: bool,
}

/// Serialises a file.
pub fn write(file: &PbnFile, opts: WriteOptions) -> String {
    todo!("phase 1")
}
