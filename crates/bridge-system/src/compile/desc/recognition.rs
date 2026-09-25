//! Recognition ratio.
//!
//! Words are whitespace-separated after normalisation, punctuation trimmed, suit sentinels kept
//! attached (`5+♠` is one word). Stopwords (`a an the and or with w/ in of at hand suit suits
//! cards points hcp`) are excluded from the denominator unless a fragment covers them.
//! `ratio = covered / (total − uncovered stopwords)`; an empty description has ratio 1.0.

use crate::{
    Recognition,
    compile::desc::{
        clause::{Fragment, FragmentKind},
        tokens::Token,
    },
};

/// Computes the statistics for a normalised description and its fragments.
///
/// `fragments` is Pass 1's output (before context resolution), so `assumed` — which needs the
/// resolved [`super::context::Provenance`] list — is always `0` here; `compile_description`
/// fills it in afterwards by counting `Provenance::assumed` entries. Everything else is
/// determined by `text` and `fragments` alone.
pub fn compute(text: &str, fragments: &[Fragment]) -> Recognition {
    if text.is_empty() {
        return Recognition {
            covered: 0,
            total: 0,
            ratio: 1.0,
            unrecognized: Vec::new(),
            constraint_bearing: false,
            assumed: 0,
            soft: 0,
        };
    }

    let mut total: u16 = 0;
    let mut covered: u16 = 0;
    let mut uncovered_stopwords: u16 = 0;

    for (start, end) in word_spans(text) {
        total += 1;
        let word_covered = fragments.iter().any(|f| {
            matches!(f.kind, FragmentKind::Token(_))
                && (start as usize) < f.span.1 as usize
                && (f.span.0 as usize) < (end as usize)
        });
        if word_covered {
            covered += 1;
        } else if is_stopword(&text[start as usize..end as usize]) {
            uncovered_stopwords += 1;
        }
    }

    let denom = total.saturating_sub(uncovered_stopwords);
    let ratio = if denom == 0 {
        1.0
    } else {
        f32::from(covered) / f32::from(denom)
    };

    let unrecognized = fragments
        .iter()
        .filter(|f| matches!(f.kind, FragmentKind::Unrecognized(_)))
        .map(|f| f.span)
        .collect();

    let soft = u8::try_from(fragments.iter().filter(|f| f.hedged).count()).unwrap_or(u8::MAX);

    // A description with nothing but bare forcing/convention/no-bound markers (which never
    // resolve to a literal; see `context::resolve`) contributes no constraint at all.
    let constraint_bearing = fragments.iter().any(|f| match &f.kind {
        FragmentKind::Token(t) => {
            !matches!(t, Token::Forcing(_) | Token::Convention(_) | Token::NoBound)
        }
        FragmentKind::Unrecognized(_) => false,
    });

    Recognition {
        covered,
        total,
        ratio,
        unrecognized,
        constraint_bearing,
        assumed: 0,
        soft,
    }
}

/// Byte spans of whitespace-separated words in `text` (suit sentinels stay attached to their
/// word; only ASCII whitespace splits).
fn word_spans(text: &str) -> impl Iterator<Item = (u16, u16)> + '_ {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    core::iter::from_fn(move || {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        Some((start as u16, i as u16))
    })
}

/// Whether `word` (sentence punctuation trimmed, but not `/` or `+`/`-`, which are meaningful in
/// this vocabulary: `w/`, `5+`, `w/o`) is one of [`STOPWORDS`].
fn is_stopword(word: &str) -> bool {
    let trimmed = word.trim_matches(|c: char| {
        matches!(
            c,
            ',' | ';' | '.' | '(' | ')' | ':' | '!' | '?' | '"' | '\''
        )
    });
    STOPWORDS.iter().any(|w| w.eq_ignore_ascii_case(trimmed))
}

/// Words excluded from the denominator.
pub const STOPWORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "with", "w/", "in", "of", "at", "hand", "suit", "suits",
    "cards", "points", "hcp",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::desc::{clause, normalize};

    fn compile(text: &str) -> (String, Recognition) {
        let normalized = normalize::normalize(text);
        let (fragments, _clause) = clause::parse(&normalized.text);
        let recognition = compute(&normalized.text, &fragments);
        (normalized.text, recognition)
    }

    #[test]
    fn empty_description_is_fully_recognized() {
        let (_, r) = compile("");
        assert_eq!(r.total, 0);
        assert_eq!(r.covered, 0);
        assert_eq!(r.ratio, 1.0);
        assert!(!r.constraint_bearing);
    }

    #[test]
    fn fully_recognized_description_has_ratio_one() {
        let (_, r) = compile("15-17 hcp and balanced");
        assert_eq!(r.ratio, 1.0);
        assert!(r.unrecognized.is_empty());
        assert!(r.constraint_bearing);
    }

    #[test]
    fn stopwords_excluded_unless_covered() {
        // "hcp" is a stopword but sits inside the recognized "15-17 hcp" fragment, so it counts
        // covered; "the" is a genuinely unrecognized stopword and is excluded from the
        // denominator entirely, so it does not drag the ratio down.
        let (_, r) = compile("15-17 hcp, forcing to game with the fourth suit");
        assert!(r.ratio < 1.0);
        assert!(!r.unrecognized.is_empty());
    }

    #[test]
    fn unrecognized_prose_lowers_ratio_and_is_reported() {
        let (text, r) = compile("some completely unrecognizable prose");
        assert_eq!(r.covered, 0);
        assert_eq!(r.ratio, 0.0);
        assert_eq!(r.unrecognized.len(), 1);
        let (start, end) = r.unrecognized[0];
        assert_eq!(
            &text[start as usize..end as usize],
            "some completely unrecognizable prose"
        );
    }

    #[test]
    fn hedge_marks_soft() {
        let (_, r) = compile("usually 5+♣");
        assert_eq!(r.soft, 1);
    }

    #[test]
    fn bare_convention_is_not_constraint_bearing() {
        let (_, r) = compile("SPL");
        assert!(!r.constraint_bearing);
    }
}
