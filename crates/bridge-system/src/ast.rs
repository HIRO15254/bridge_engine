//! The BML abstract syntax tree.

use std::sync::Arc;

use crate::{
    Lint,
    pattern::{CallPattern, Side},
};

/// Index of a source file inside a [`BmlFile`] (0 = the root file; `#INCLUDE`d files follow).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FileId(pub u16);

/// A source location.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Span {
    /// The file.
    pub file: FileId,
    /// 1-based line.
    pub line: u32,
    /// 0-based column.
    pub col: u16,
    /// For lines produced by `#PASTE`: the clipboard name and the line inside it.
    pub pasted_from: Option<(Arc<str>, u32)>,
}

/// One physical line after include resolution.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RawLine {
    /// Where it came from.
    pub span: Span,
    /// The text without the line terminator.
    pub text: String,
}

/// A parsed BML file (with includes resolved).
#[derive(Clone, Debug)]
pub struct BmlFile {
    /// The root file.
    pub root: FileId,
    /// `(path, text)` for every file, indexed by [`FileId`]; used for spans and hashing.
    pub files: Vec<(Arc<str>, Arc<str>)>,
    /// Top-level blocks in order.
    pub blocks: Vec<Block>,
    /// Parse-stage diagnostics.
    pub lints: Vec<Lint>,
}

/// A top-level block.
#[derive(Clone, Debug)]
pub enum Block {
    /// `#+KEY: value`.
    Meta {
        /// Key.
        key: String,
        /// Value.
        value: String,
        /// Location.
        span: Span,
    },
    /// `#SEAT n`.
    Seat {
        /// The condition.
        cond: SeatCond,
        /// Location.
        span: Span,
    },
    /// `#VUL we them`.
    Vul {
        /// The condition.
        cond: VulCond,
        /// Location.
        span: Span,
    },
    /// `* Heading`.
    Heading {
        /// Number of stars.
        level: u8,
        /// Text.
        text: String,
        /// Location.
        span: Span,
    },
    /// Free text.
    Paragraph {
        /// Text.
        text: String,
        /// Location.
        span: Span,
    },
    /// `-` or `1.` list.
    List {
        /// Items.
        items: Vec<String>,
        /// Numbered.
        ordered: bool,
        /// Location.
        span: Span,
    },
    /// A bidding table.
    BidTable(BidTable),
    /// A top-level `#CUT` block (clipboard only).
    Clipboard {
        /// Name.
        name: String,
        /// Lines.
        lines: Vec<RawLine>,
    },
}

/// A bidding table.
#[derive(Clone, Debug)]
pub struct BidTable {
    /// `#HIDE` was given (ignored by the compiler; kept for tooling).
    pub hidden: bool,
    /// Seat condition in force.
    pub seat: SeatCond,
    /// Vulnerability condition in force.
    pub vul: VulCond,
    /// The history row (`1N-2C;`), empty for a table of openings.
    pub history: Vec<CallToken>,
    /// Description written on the history row, if any (belongs to its last call).
    pub history_desc: Option<Description>,
    /// Top-level rows.
    pub rows: Vec<BmlNode>,
    /// A `#STOP` directive at the table's top level: the position the history row names is a
    /// system stop (see [`BmlNode::stop`]).
    pub stop: bool,
    /// An `#ANYORDER` directive anywhere in the table: its fresh `X`/`Y`/`Z` bindings ignore the
    /// `X < Y < Z` strain order (they stay distinct and unused; `docs/design/06-system.md` §4.7).
    pub any_order: bool,
    /// The `#EXACTPASS` directive in force for the table, by its location: `#EXACTPASS` anywhere
    /// in the table, or `#EXACTPASS FILE` in a paragraph of its own earlier in the same file
    /// (the file scope wins when both apply). The opponents' pass right before each row of ours
    /// is then exact: every other call of theirs at that position that has no trie edge of its
    /// own reaches an empty `(any)` sibling, so it is off-system instead of being read as a pass
    /// (`docs/design/06-system.md` §4.8).
    pub exact_pass: Option<Span>,
    /// Location.
    pub span: Span,
}

/// One row of a bidding table with its sub-rows.
#[derive(Clone, Debug)]
pub struct BmlNode {
    /// The call(s) of this row (one, except that `2S/3H` alternatives are kept as one token).
    pub calls: Vec<CallToken>,
    /// The description.
    pub description: Description,
    /// Sub-rows.
    pub children: Vec<BmlNode>,
    /// A `#STOP` directive among the sub-rows: the partnership has stopped at this row's
    /// position, so from here on it passes with any hand whatever the opponents call
    /// (`docs/design/06-system.md` §4.5). The `{stop}` description annotation means the same
    /// and is read from the description itself.
    pub stop: bool,
    /// Indentation in columns.
    pub indent: u16,
    /// Location.
    pub span: Span,
}

/// A row description.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Description {
    /// Lines joined with `\n`, without the alert marker.
    pub text: String,
    /// A leading `!` (not followed by a suit letter) marked the row as alertable.
    pub alert: bool,
    /// Column where the description starts (continuation lines align to it).
    pub col: u16,
}

/// A call token as written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CallToken {
    /// Ours or theirs (parenthesised).
    pub side: Side,
    /// The pattern.
    pub pattern: CallPattern,
    /// The raw text.
    pub raw: String,
    /// Location.
    pub span: Span,
}

/// `#SEAT` condition: the position in which the opening bid is made.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SeatCond {
    /// `#SEAT 0`.
    #[default]
    Any,
    /// `#SEAT 1`.
    First,
    /// `#SEAT 2`.
    Second,
    /// `#SEAT 3`.
    Third,
    /// `#SEAT 4`.
    Fourth,
    /// `#SEAT 12`.
    FirstOrSecond,
    /// `#SEAT 34`.
    ThirdOrFourth,
}

impl SeatCond {
    /// Whether an opener in `position` (`1..=4`) satisfies the condition.
    pub const fn matches(self, position: u8) -> bool {
        match self {
            SeatCond::Any => true,
            SeatCond::First => position == 1,
            SeatCond::Second => position == 2,
            SeatCond::Third => position == 3,
            SeatCond::Fourth => position == 4,
            SeatCond::FirstOrSecond => position <= 2,
            SeatCond::ThirdOrFourth => position >= 3,
        }
    }

    /// How specific the condition is (higher wins at lookup).
    pub const fn specificity(self) -> u8 {
        match self {
            SeatCond::Any => 0,
            SeatCond::FirstOrSecond | SeatCond::ThirdOrFourth => 1,
            _ => 2,
        }
    }
}

/// A yes / no / don't-care value.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Tri {
    /// `Y`.
    Yes,
    /// `N`.
    No,
    /// `0`.
    #[default]
    Any,
}

impl Tri {
    /// Whether the actual value `v` satisfies this condition.
    pub const fn matches(self, v: bool) -> bool {
        match self {
            Tri::Yes => v,
            Tri::No => !v,
            Tri::Any => true,
        }
    }
}

/// `#VUL we them` condition, relative to the system owner's partnership.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VulCond {
    /// Our vulnerability.
    pub we: Tri,
    /// Their vulnerability.
    pub they: Tri,
}

impl VulCond {
    /// Whether the actual vulnerability satisfies the condition.
    pub const fn matches(self, we: bool, they: bool) -> bool {
        self.we.matches(we) && self.they.matches(they)
    }

    /// How specific the condition is (0, 1 or 2 fixed sides).
    pub const fn specificity(self) -> u8 {
        (!matches!(self.we, Tri::Any)) as u8 + (!matches!(self.they, Tri::Any)) as u8
    }
}
