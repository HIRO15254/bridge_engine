//! Errors.

/// DNF normalisation failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum DnfError {
    /// The expansion would exceed the term cap and the policy is [`Overflow::Error`](crate::Overflow::Error).
    #[error("DNF would have about {estimated} terms, more than the cap of {max_terms}")]
    TooLarge {
        /// Estimated number of terms.
        estimated: usize,
        /// The configured cap.
        max_terms: usize,
    },
}

/// A sampler could not be prepared (contract violations only; an unsatisfiable constraint is
/// not an error, it yields `count() == 0`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum PrepareError {
    /// `pool` and `fixed` share a card.
    #[error("pool and fixed cards overlap")]
    Overlap,
    /// `fixed` has more than 13 cards.
    #[error("fixed part has {0} cards, more than 13")]
    TooManyFixed(u8),
    /// The constraint contains a custom predicate and the options forbid rejection sampling.
    #[error("constraint is not samplable and rejection sampling is disabled")]
    NotSamplable,
}

/// The known cards of a table are inconsistent.
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum KnownCardsError {
    /// Two seats are known to hold the same card.
    #[error("card {0} is known for two seats")]
    Duplicate(bridge_core::Card),
    /// A seat is known to hold more than 13 cards.
    #[error("{seat} has {count} known cards, more than 13")]
    TooMany {
        /// The seat.
        seat: bridge_core::Seat,
        /// Its known-card count.
        count: u8,
    },
}
