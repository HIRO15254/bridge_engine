//! Constraints inferred from the play of the cards.
//!
//! Independent of `bridge-system` on purpose: lead and signal agreements are written by
//! different people on a different schedule from the bidding system.
//!
//! | Tier | Content | v1 |
//! | --- | --- | --- |
//! | Hard constraints | a seat that failed to follow suit is void (its original length in that suit equals the cards it has played); played cards become known cards | yes |
//! | Declared soft information | lead conventions (4th best, 3rd/5th, attitude; honour leads), signals (attitude, count, discards) as rule tables | yes |
//! | Learned policy | "a good defender would not have played that" | no (a `Proposal` implementation from outside) |
//!
//! Lead and signal agreements are disclosed information (they are on the convention card), so
//! they can be rules rather than learned models.
#![forbid(unsafe_code)]
#![warn(missing_docs)]
// Phase 0 skeleton: the public surface is final, bodies are `todo!()`.
#![allow(dead_code, unused_variables)]

mod agreements;
mod hard;
mod interpret;
mod leads;
mod signals;

pub use agreements::{
    DiscardTable, FirstDiscard, HonorLeads, LeadStyle, LeadTable, PlayAgreements, Polarity,
    SignalTable, SpotLead,
};
pub use bridge_constraint::{HandConstraint, KnownCards};
pub use hard::hard_constraints;
pub use interpret::{PlayEvent, PlayInterpretation, PlayWarning, interpret_play};
