//! Parses `#+KEY: value` blocks into [`SystemMeta`] (`docs/design/06-system.md` §5.2).
//!
//! First definition wins (BML's general rule): a key seen twice keeps its first value. Unknown
//! keys are preserved verbatim in [`SystemMeta::extra`]; a known key whose value fails to parse
//! keeps the default and reports [`LintCode::UnknownDirective`].

use std::collections::HashSet;

use bridge_core::ShapeClass;
use bridge_eval::DistMethod;

use crate::{
    Lint, LintCode, SystemMeta, TieBreak,
    ast::{Block, Span},
};

/// Builds [`SystemMeta`] from the blocks of a parsed file. `default_name` is used for
/// `meta.name` when no `#+TITLE:` is given (the spec's "file name" default).
pub(crate) fn parse_meta(
    blocks: &[Block],
    default_name: &str,
    lints: &mut Vec<Lint>,
) -> SystemMeta {
    let mut meta = SystemMeta::default();
    let mut seen: HashSet<String> = HashSet::new();

    for block in blocks {
        let Block::Meta { key, value, span } = block else {
            continue;
        };
        let upper = key.to_ascii_uppercase();
        if !seen.insert(upper.clone()) {
            continue; // first definition wins
        }
        apply_key(&mut meta, &upper, key, value, span, lints);
    }

    if meta.name.is_empty() {
        meta.name = default_name.to_string();
    }
    meta
}

fn apply_key(
    meta: &mut SystemMeta,
    upper: &str,
    raw_key: &str,
    value: &str,
    span: &Span,
    lints: &mut Vec<Lint>,
) {
    match upper {
        "TITLE" => meta.name = value.to_string(),
        "DESCRIPTION" => meta.description = value.to_string(),
        "AUTHOR" => meta.authors = split_authors(value),
        "DATE" => meta.date = Some(value.to_string()),
        "VERSION" => meta.version = value.to_string(),
        "STRENGTH" => parse_strength(value, meta, span, lints),
        "NATURAL" => parse_natural(value, meta, span, lints),
        "DISTPOINTS" => parse_dist(value, meta, span, lints),
        "TIEBREAK" => parse_tiebreak(value, meta, span, lints),
        "BALANCED" => parse_balanced(value, meta, span, lints),
        "CONVENTION" => parse_convention(value, meta, span, lints),
        "RECOGNITION" => parse_recognition(value, meta, span, lints),
        _ => {
            meta.extra.insert(raw_key.to_string(), value.to_string());
        }
    }
}

/// `,` or ` and ` separated author list.
fn split_authors(value: &str) -> Vec<String> {
    value
        .split(" and ")
        .flat_map(|s| s.split(','))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn warn_bad_value(kind: &str, value: &str, span: &Span, lints: &mut Vec<Lint>) {
    lints.push(
        Lint::warning(
            LintCode::UnknownDirective,
            format!("#+{kind}: {value:?} could not be parsed; using the default"),
        )
        .with_span(span.clone()),
    );
}

/// Parses `a-b` or `a--b` into `a..=b`.
fn parse_range(s: &str) -> Option<std::ops::RangeInclusive<u8>> {
    let s = s.replace("--", "-");
    let (lo, hi) = s.split_once('-')?;
    Some(lo.trim().parse().ok()?..=hi.trim().parse().ok()?)
}

/// `gf=25 inv=22-24 slam=31 strong=16 weak=9 opening=12 neg=7`.
fn parse_strength(value: &str, meta: &mut SystemMeta, span: &Span, lints: &mut Vec<Lint>) {
    for token in value.split_whitespace() {
        let Some((key, v)) = token.split_once('=') else {
            warn_bad_value("STRENGTH", token, span, lints);
            continue;
        };
        let ok = match key {
            "gf" => v.parse().ok().map(|n| meta.strength.gf_total = n),
            "inv" => parse_range(v).map(|r| meta.strength.inv_total = r),
            "slam" => v.parse().ok().map(|n| meta.strength.slam_total = n),
            "strong" => v.parse().ok().map(|n| meta.strength.strong_min = n),
            "weak" => v.parse().ok().map(|n| meta.strength.weak_max = n),
            "neg" => v.parse().ok().map(|n| meta.strength.neg_max = n),
            "opening" => v.parse().ok().map(|n| meta.strength.opening_min = n),
            "max" => v.parse().ok().map(|n| meta.strength.hcp_max = n),
            _ => None,
        };
        if ok.is_none() {
            warn_bad_value("STRENGTH", token, span, lints);
        }
    }
}

/// `1M=5 1m=3 1N=15-17 2N=20-21 weak2=6 overcall=5`. A best-effort subset of the length/HCP
/// overrides `NaturalParams` exposes; other subkeys are reported and ignored.
fn parse_natural(value: &str, meta: &mut SystemMeta, span: &Span, lints: &mut Vec<Lint>) {
    for token in value.split_whitespace() {
        let Some((key, v)) = token.split_once('=') else {
            warn_bad_value("NATURAL", token, span, lints);
            continue;
        };
        let ok = match key {
            "1M" => v.parse().ok().map(|n| meta.natural.open_1major_len = n),
            "1m" => v.parse().ok().map(|n| meta.natural.open_1m_len = n),
            "weak2" => v.parse().ok().map(|n| meta.natural.weak_two.0 = n),
            "overcall" => v.parse().ok().map(|n| meta.natural.overcall[0].0 = n),
            "strong2c" => v.parse().ok().map(|n| meta.natural.strong_two_c = n),
            key if key.ends_with('N') && key.len() <= 2 => {
                let level: Option<u8> = key.trim_end_matches('N').parse().ok();
                let range = parse_range(v);
                match (level, range) {
                    (Some(level), Some(range)) => {
                        match meta.natural.nt.iter_mut().find(|(l, _)| *l == level) {
                            Some(entry) => entry.1 = range,
                            None => meta.natural.nt.push((level, range)),
                        }
                        Some(())
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if ok.is_none() {
            warn_bad_value("NATURAL", token, span, lints);
        }
    }
}

/// `321` | `bergen` | `none`.
fn parse_dist(value: &str, meta: &mut SystemMeta, span: &Span, lints: &mut Vec<Lint>) {
    match value.trim().to_ascii_lowercase().as_str() {
        "321" => meta.dist_method = DistMethod::GOREN_321,
        "bergen" => meta.dist_method = DistMethod::BergenStarting,
        "none" => {
            meta.dist_method = DistMethod::ShortSuit {
                void: 0,
                singleton: 0,
                doubleton: 0,
            }
        }
        _ => warn_bad_value("DISTPOINTS", value, span, lints),
    }
}

/// `row-order` | `narrowest` | `lowest-call` | `highest-call`.
fn parse_tiebreak(value: &str, meta: &mut SystemMeta, span: &Span, lints: &mut Vec<Lint>) {
    match value.trim().to_ascii_lowercase().as_str() {
        "row-order" | "row_order" => meta.tie_break = TieBreak::RowOrder,
        "narrowest" => meta.tie_break = TieBreak::Narrowest,
        "lowest-call" | "lowest_call" => meta.tie_break = TieBreak::LowestCall,
        "highest-call" | "highest_call" => meta.tie_break = TieBreak::HighestCall,
        _ => warn_bad_value("TIEBREAK", value, span, lints),
    }
}

/// A space-separated list of 4-digit shape tokens (`4333 4432 5332 5422`), replacing
/// `meta.balanced.balanced` (the "semi" additions keep their default: the spec's meta table gives
/// no separate syntax for them).
fn parse_balanced(value: &str, meta: &mut SystemMeta, span: &Span, lints: &mut Vec<Lint>) {
    let mut classes = Vec::new();
    for token in value.split_whitespace() {
        match parse_shape_digits(token) {
            Some(class) => classes.push(class),
            None => warn_bad_value("BALANCED", token, span, lints),
        }
    }
    if !classes.is_empty() {
        meta.balanced.balanced = classes;
    }
}

/// Four ASCII digits summing to 13, in any order (`ShapeClass::new` sorts them).
fn parse_shape_digits(token: &str) -> Option<ShapeClass> {
    if token.len() != 4 {
        return None;
    }
    let mut lens = [0u8; 4];
    for (i, c) in token.chars().enumerate() {
        lens[i] = c.to_digit(10)? as u8;
    }
    if lens.iter().map(|&n| n as u16).sum::<u16>() != 13 {
        return None;
    }
    Some(ShapeClass::new(lens))
}

/// `transfer=5 stayman=4M splinter=4` (`stayman=any` clears `stayman_major`).
fn parse_convention(value: &str, meta: &mut SystemMeta, span: &Span, lints: &mut Vec<Lint>) {
    for token in value.split_whitespace() {
        let Some((key, v)) = token.split_once('=') else {
            warn_bad_value("CONVENTION", token, span, lints);
            continue;
        };
        let ok = match key {
            "transfer" => v.parse().ok().map(|n| meta.conventions.transfer_len = n),
            "splinter" => v
                .parse()
                .ok()
                .map(|n| meta.conventions.splinter_support = n),
            "stayman" => {
                if v.eq_ignore_ascii_case("any") {
                    meta.conventions.stayman_major = false;
                    Some(())
                } else if v.ends_with('M') || v.ends_with('m') {
                    meta.conventions.stayman_major = true;
                    Some(())
                } else {
                    None
                }
            }
            _ => None,
        };
        if ok.is_none() {
            warn_bad_value("CONVENTION", token, span, lints);
        }
    }
}

fn parse_recognition(value: &str, meta: &mut SystemMeta, span: &Span, lints: &mut Vec<Lint>) {
    match value.trim().parse::<f32>() {
        Ok(n) if (0.0..=1.0).contains(&n) => meta.recognition_threshold = n,
        _ => warn_bad_value("RECOGNITION", value, span, lints),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::FileId;

    fn span() -> Span {
        Span {
            file: FileId(0),
            line: 1,
            col: 0,
            pasted_from: None,
        }
    }

    fn meta_block(key: &str, value: &str) -> Block {
        Block::Meta {
            key: key.to_string(),
            value: value.to_string(),
            span: span(),
        }
    }

    #[test]
    fn title_and_description_and_author() {
        let blocks = vec![
            meta_block("TITLE", "Strawberry Club"),
            meta_block("DESCRIPTION", "A Polish Club variant"),
            meta_block("AUTHOR", "Alice, Bob and Carol"),
        ];
        let mut lints = Vec::new();
        let meta = parse_meta(&blocks, "fallback.bml", &mut lints);
        assert_eq!(meta.name, "Strawberry Club");
        assert_eq!(meta.description, "A Polish Club variant");
        assert_eq!(meta.authors, vec!["Alice", "Bob", "Carol"]);
        assert!(lints.is_empty());
    }

    #[test]
    fn missing_title_falls_back_to_the_file_name() {
        let meta = parse_meta(&[], "sayc.bml", &mut Vec::new());
        assert_eq!(meta.name, "sayc.bml");
    }

    #[test]
    fn first_definition_of_a_key_wins() {
        let blocks = vec![meta_block("TITLE", "First"), meta_block("TITLE", "Second")];
        let meta = parse_meta(&blocks, "x.bml", &mut Vec::new());
        assert_eq!(meta.name, "First");
    }

    #[test]
    fn unknown_key_is_preserved_in_extra() {
        let blocks = vec![meta_block("CUSTOMKEY", "some value")];
        let meta = parse_meta(&blocks, "x.bml", &mut Vec::new());
        assert_eq!(meta.extra.get("CUSTOMKEY"), Some(&"some value".to_string()));
    }

    #[test]
    fn strength_parses_all_subkeys() {
        let blocks = vec![meta_block(
            "STRENGTH",
            "gf=26 inv=21-23 slam=32 strong=15 weak=8 neg=6 opening=13 max=40",
        )];
        let meta = parse_meta(&blocks, "x.bml", &mut Vec::new());
        assert_eq!(meta.strength.gf_total, 26);
        assert_eq!(meta.strength.inv_total, 21..=23);
        assert_eq!(meta.strength.slam_total, 32);
        assert_eq!(meta.strength.strong_min, 15);
        assert_eq!(meta.strength.weak_max, 8);
        assert_eq!(meta.strength.neg_max, 6);
        assert_eq!(meta.strength.opening_min, 13);
        assert_eq!(meta.strength.hcp_max, 40);
    }

    #[test]
    fn dist_points_variants() {
        for value in ["321", "bergen", "none"] {
            let blocks = vec![meta_block("DISTPOINTS", value)];
            let mut lints = Vec::new();
            parse_meta(&blocks, "x.bml", &mut lints);
            assert!(lints.is_empty(), "{value}");
        }

        let blocks = vec![meta_block("DISTPOINTS", "none")];
        let meta = parse_meta(&blocks, "x.bml", &mut Vec::new());
        assert_eq!(
            meta.dist_method,
            DistMethod::ShortSuit {
                void: 0,
                singleton: 0,
                doubleton: 0
            }
        );

        let blocks = vec![meta_block("DISTPOINTS", "bergen")];
        let meta = parse_meta(&blocks, "x.bml", &mut Vec::new());
        assert_eq!(meta.dist_method, DistMethod::BergenStarting);
    }

    #[test]
    fn dist_points_unknown_value_warns_and_keeps_default() {
        let blocks = vec![meta_block("DISTPOINTS", "garbage")];
        let mut lints = Vec::new();
        let meta = parse_meta(&blocks, "x.bml", &mut lints);
        assert_eq!(meta.dist_method, DistMethod::GOREN_321);
        assert_eq!(lints.len(), 1);
        assert_eq!(lints[0].code, LintCode::UnknownDirective);
    }

    #[test]
    fn tiebreak_variants() {
        let blocks = vec![meta_block("TIEBREAK", "narrowest")];
        let meta = parse_meta(&blocks, "x.bml", &mut Vec::new());
        assert_eq!(meta.tie_break, TieBreak::Narrowest);
    }

    #[test]
    fn balanced_parses_shape_tokens_regardless_of_digit_order() {
        let blocks = vec![meta_block("BALANCED", "3334 4432 2335")];
        let meta = parse_meta(&blocks, "x.bml", &mut Vec::new());
        assert_eq!(
            meta.balanced.balanced,
            vec![ShapeClass::C4333, ShapeClass::C4432, ShapeClass::C5332]
        );
    }

    #[test]
    fn convention_parses_transfer_stayman_splinter() {
        let blocks = vec![meta_block(
            "CONVENTION",
            "transfer=6 stayman=any splinter=3",
        )];
        let meta = parse_meta(&blocks, "x.bml", &mut Vec::new());
        assert_eq!(meta.conventions.transfer_len, 6);
        assert!(!meta.conventions.stayman_major);
        assert_eq!(meta.conventions.splinter_support, 3);
    }

    #[test]
    fn recognition_threshold_out_of_range_is_rejected() {
        let blocks = vec![meta_block("RECOGNITION", "1.5")];
        let mut lints = Vec::new();
        let meta = parse_meta(&blocks, "x.bml", &mut lints);
        assert_eq!(meta.recognition_threshold, 0.5);
        assert_eq!(lints.len(), 1);
    }

    #[test]
    fn recognition_threshold_in_range_is_applied() {
        let blocks = vec![meta_block("RECOGNITION", "0.75")];
        let meta = parse_meta(&blocks, "x.bml", &mut Vec::new());
        assert_eq!(meta.recognition_threshold, 0.75);
    }
}
