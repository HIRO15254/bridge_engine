//! Readers and writers for the bridge record formats.
//!
//! | Format | Reads | Writes | Notes |
//! | --- | --- | --- | --- |
//! | PBN 2.1 | yes, lenient and strict | export format | tournament records; `Note` references and escapes |
//! | LIN (BBO) | yes, lenient | no | real-world files contain broken fragments; read what can be read |
//! | RBN | later | no | |
//! | Deal string | yes | yes | `N:AKQ.234.AKQ.2345 …` (also in `bridge-core`'s `FromStr`) |
//!
//! `parse_lenient` never fails and never panics: everything it cannot understand becomes a
//! [`Warning`] with a line number, and the remaining games are still returned. This crate is the
//! first milestone; nothing above it can be validated without real data.
//!
//! ```
//! use bridge_format::pbn;
//!
//! let text = b"[Event \"Test\"]\n[Dealer \"N\"]\n[Vulnerable \"None\"]\n\
//!     [Deal \"N:QJ6.K652.J85.T98 873.J97.AT764.Q4 K5.T83.KQ9.A7652 AT942.AQ4.32.KJ3\"]\n\
//!     [Auction \"N\"]\n1NT Pass 3NT AP\n";
//! let (file, warnings) = pbn::parse_lenient(text);
//! assert!(warnings.is_empty());
//! let view = file.games[0].view(None).unwrap();
//! assert!(view.deal.unwrap().complete().is_some());
//! assert_eq!(view.auction.unwrap().calls().len(), 6);
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod deal_string;
pub mod lin;
pub mod pbn;
mod text;
mod warning;

pub use pbn::{
    Game, GameView, PartialDeal, PbnFile, Section, TagPair, TagValue, Token, WriteOptions,
};
pub use warning::{Warning, WarningKind};

/// Strict parsing failed.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
#[error("line {line}: {message}")]
pub struct ParseError {
    /// 1-based line.
    pub line: u32,
    /// What went wrong.
    pub message: String,
}
