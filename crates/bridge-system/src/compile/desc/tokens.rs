//! The token vocabulary (WBF / BML abbreviations).
//!
//! Each token maps to atom literals or flags; context-dependent tokens carry no values until
//! `context.rs` resolves them. The full table (with examples from real files) is in
//! `docs/design/06-system.md`.

use core::ops::RangeInclusive;

use bridge_core::Suit;

/// A suit reference inside a description.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SuitRef {
    /// `!s`, or a bound variable substituted at expansion.
    Fixed(Suit),
    /// `#`: the suit of the nearest variable / multi-strain call in the path.
    Hash,
    /// The suit of this row's own call.
    Own,
    /// `M`, `oM`, `m`, `om` when unbound.
    AnyMajor,
    /// Any minor.
    AnyMinor,
    /// Partner's agreed suit.
    Agreed,
    /// Opponents' suit.
    Theirs,
}

/// A recognised token.
#[derive(Clone, PartialEq, Debug)]
pub enum Token {
    /// `12+ hcp`, `15-17`, `ca 15+`.
    Hcp(RangeInclusive<u8>),
    /// `13+ points` (total points).
    Points(RangeInclusive<u8>),
    /// `5+!s`, `4=!h`, `0-3!s`, `6+ suit`, `4+#`.
    SuitLen(SuitRef, RangeInclusive<u8>),
    /// `4414`, `(54)`, `54(31)`, `(54)(xx)`, `55MM`, `5-5 minors`, `4-4 majors`.
    Shape(String),
    /// `bal`.
    Balanced,
    /// `semi-bal`.
    SemiBalanced,
    /// `unbal`.
    Unbalanced,
    /// `GF`, `INV`, `INV+`, `MIN`, `MAX`, `weak`, `STR`, `PRE`, `S/T`, `QUANT`, `NEG`, `LIM`.
    Strength(StrengthWord),
    /// `NF`, `F`, `F1`, `FG`.
    Forcing(crate::Forcing),
    /// `ART`, `(R)`, `TRF`, `PUP`, `P/C`, `S/O`, `STAY`, `SPL`, `UNT`, `Multi`, …
    Convention(String),
    /// `SOL`, `S-SOL`, `2 of top 3`, `AKQ`, `good suit`.
    Quality(SuitRef, QualityWord),
    /// `stopper`, `with stopper`.
    Stopper(SuitRef),
    /// `singleton`, `void`, `short`, `0-1!h`.
    Shortness(SuitRef, u8),
    /// `fit`, `3+ SUPP`, `support`, `raise`.
    Support(u8),
    /// `controls`, `2 controls`.
    Controls(RangeInclusive<u8>),
    /// `7 losers`, `LTC`.
    Losers(RangeInclusive<u8>),
    /// `NAT`, `natural`.
    Natural,
    /// `unlimited`, `any hand`: recognised, no constraint.
    NoBound,
}

/// Context-dependent strength words.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(missing_docs)]
pub enum StrengthWord {
    GameForcing,
    Invitational,
    InvitationalPlus,
    Min,
    Max,
    Weak,
    Strong,
    Preemptive,
    SlamTry,
    Quantitative,
    Negative,
    Limit,
}

/// Suit-quality words.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(missing_docs)]
pub enum QualityWord {
    Solid,
    SemiSolid,
    TwoOfTopThree,
    ThreeOfTopFive,
    Good,
}

/// Tries to recognise one fragment of normalised text. Returns the token and the number of
/// bytes consumed.
pub fn recognize(text: &str) -> Option<(Token, usize)> {
    todo!("phase 3")
}
