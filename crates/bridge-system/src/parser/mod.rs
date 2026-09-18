//! Paragraph classification, clipboard expansion and bidding-table tree building.
//!
//! Every stage records failures as [`Lint`](crate::Lint)s and keeps going: an unparsable call
//! token skips that row and its subtree; an indentation that matches no open ancestor attaches
//! the row to the nearest shallower one; a row containing `-` or `;` that is not the first row
//! of its table is dropped.

pub mod call;
pub mod clipboard;

use crate::{
    ast::{BmlFile, RawLine},
    lexer::Loaded,
};

/// Builds the AST from loaded lines.
pub fn parse(loaded: Loaded) -> BmlFile {
    todo!("phase 3")
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
    todo!("phase 3")
}
