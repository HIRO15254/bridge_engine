//! Warnings from lenient parsing.

/// Something a lenient parser skipped or repaired.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Warning {
    /// 0-based game (PBN) or board (LIN) index.
    pub game: usize,
    /// 1-based line.
    pub line: u32,
    /// Category.
    pub kind: WarningKind,
    /// Human-readable detail.
    pub message: String,
}

/// Warning categories.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[allow(missing_docs)]
pub enum WarningKind {
    Encoding,
    MalformedTag,
    UnknownTag,
    MalformedToken,
    BadDeal,
    BadAuction,
    BadPlay,
    NoteReference,
    Truncated,
    Other,
}
