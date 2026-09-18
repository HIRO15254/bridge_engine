//! Context resolution (pass 2).
//!
//! | Word | Resolution |
//! | --- | --- |
//! | `GF` | `own_min = gf_total − partner_min` |
//! | `INV` | `[inv.start − partner_min, inv.end − partner_min]`; `INV+` lower bound only |
//! | `MIN` / `MAX` | lower / upper half of the same player's previous range |
//! | `weak` | responder `[0, weak_max]`; opener at level 2 `weak_two`; level 3/4 `preempt`; overcaller `weak_jump` |
//! | `S/T` | `own_min = slam_total − partner_max` |
//! | `QUANT` | `[gf_total − partner_max + 1, slam_total − partner_min]` |
//! | `NAT` | `NaturalInference` for this call in this context, intersected with explicit fragments |
//! | `#`, `Own`, `Agreed`, `Theirs` | concrete suits from the path |
//!
//! Unknown partner ranges default to `opening_min..=21` (opener) or `0..=37` and are flagged
//! `assumed`.

use bridge_constraint::Atom;
use bridge_core::{Bid, Call, Side as TableSide, Suit};

use crate::{Node, SystemMeta, natural::Role, pattern::Binding};

/// Where a resolved value came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// Written in the description.
    Explicit,
    /// Derived from the path context.
    Context,
    /// Taken from natural-inference defaults.
    NaturalDefault,
}

/// Provenance of a resolved fragment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Provenance {
    /// Byte span of the token.
    pub span: (u16, u16),
    /// A default was assumed because the context did not state the value.
    pub assumed: bool,
    /// Source.
    pub source: Source,
}

/// Everything known about a row when its description is compiled (ancestors are done first).
#[derive(Clone, Debug)]
pub struct RowContext<'a> {
    /// This row's call.
    pub call: Call,
    /// Whose call (as a table side).
    pub side: TableSide,
    /// Bid level (0 for pass/double/redouble).
    pub level: u8,
    /// The call skipped at least one level.
    pub is_jump: bool,
    /// Variable binding.
    pub binding: &'a Binding,
    /// The suit `#` refers to.
    pub hash_suit: Option<Suit>,
    /// The same player's previous node on this path.
    pub own_prev: Option<&'a Node>,
    /// Partner's last node on this path.
    pub partner_last: Option<&'a Node>,
    /// Opponents' last bid.
    pub their_last_bid: Option<Bid>,
    /// Agreed suit, if any.
    pub agreed_suit: Option<Suit>,
    /// Role in the auction.
    pub role: Role,
}

/// Resolves the context-dependent tokens of a clause into atom literals.
pub fn resolve(
    tokens: &[super::tokens::Token],
    ctx: &RowContext<'_>,
    meta: &SystemMeta,
) -> (Vec<Atom>, Vec<Provenance>) {
    todo!("phase 3")
}
