//! Recognition ratio.
//!
//! Words are whitespace-separated after normalisation, punctuation trimmed, suit sentinels kept
//! attached (`5+♠` is one word). Stopwords (`a an the and or with w/ in of at hand suit suits
//! cards points hcp`) are excluded from the denominator unless a fragment covers them.
//! `ratio = covered / (total − uncovered stopwords)`; an empty description has ratio 1.0.

use crate::{Recognition, compile::desc::clause::Fragment};

/// Computes the statistics for a normalised description and its fragments.
pub fn compute(text: &str, fragments: &[Fragment]) -> Recognition {
    todo!("phase 3")
}

/// Words excluded from the denominator.
pub const STOPWORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "with", "w/", "in", "of", "at", "hand", "suit", "suits",
    "cards", "points", "hcp",
];
