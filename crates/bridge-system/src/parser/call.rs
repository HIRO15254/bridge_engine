//! The call-token grammar (winnow).
//!
//! ```ebnf
//! calltok    = "(" callcore ")" | callcore ;
//! callcore   = "P" | "D" | "R" | "X" | "XX"
//!            | level strainspec
//!            | level ( "step" | "steps" )
//!            | callcore "/" ( callcore | strainspec ) ;      (* 2S/3H, 4D/H *)
//! level      = "1".."7" | "n" ;
//! strainspec = literal | variable | "red" | "black" ;
//! literal    = "NT" | "N" | ( "C" | "D" | "H" | "S" ) { "C" | "D" | "H" | "S" } ;
//! variable   = "M" | "m" | "oM" | "om" | "X" | "Y" | "Z" | "x" | "y" | "z" ;
//! ```

use bridge_core::{Bid, Call, Strain};
use winnow::{
    ascii::digit1,
    combinator::{alt, delimited, opt, repeat},
    prelude::*,
    token::take,
};

use crate::pattern::{CallPattern, Level, OppClass, Side, StrainSet, Var};

/// The two kinds of `strainspec`: a fixed set of strains (no binding), or a variable that binds
/// on first use.
enum StrainSpec {
    Literal(StrainSet),
    Variable(Var),
}

fn strain_of(c: char) -> Strain {
    match c {
        'C' => Strain::Clubs,
        'D' => Strain::Diamonds,
        'H' => Strain::Hearts,
        'S' => Strain::Spades,
        _ => unreachable!("only called for C/D/H/S"),
    }
}

fn literal_strains(input: &mut &str) -> ModalResult<StrainSet> {
    alt((
        "NT".value(StrainSet::EMPTY.with(Strain::NoTrump)),
        "N".value(StrainSet::EMPTY.with(Strain::NoTrump)),
        repeat(1.., winnow::token::one_of(['C', 'D', 'H', 'S'])).map(|cs: Vec<char>| {
            cs.into_iter()
                .fold(StrainSet::EMPTY, |acc, c| acc.with(strain_of(c)))
        }),
    ))
    .parse_next(input)
}

/// `"red"` / `"black"`, case-insensitive (BML 2).
fn red_black(input: &mut &str) -> ModalResult<StrainSet> {
    alt((
        take(5usize)
            .verify_map(|s: &str| s.eq_ignore_ascii_case("black").then_some(StrainSet::BLACK)),
        take(3usize).verify_map(|s: &str| s.eq_ignore_ascii_case("red").then_some(StrainSet::RED)),
    ))
    .parse_next(input)
}

fn variable(input: &mut &str) -> ModalResult<Var> {
    alt((
        "oM".value(Var::OtherMajor),
        "om".value(Var::OtherMinor),
        "M".value(Var::Major),
        "m".value(Var::Minor),
        winnow::token::one_of(['X', 'x']).value(Var::X),
        winnow::token::one_of(['Y', 'y']).value(Var::Y),
        winnow::token::one_of(['Z', 'z']).value(Var::Z),
    ))
    .parse_next(input)
}

fn strainspec(input: &mut &str) -> ModalResult<StrainSpec> {
    alt((
        red_black.map(StrainSpec::Literal),
        variable.map(StrainSpec::Variable),
        literal_strains.map(StrainSpec::Literal),
    ))
    .parse_next(input)
}

fn level(input: &mut &str) -> ModalResult<Level> {
    alt((
        winnow::token::one_of('1'..='7').map(|c: char| Level::At(c as u8 - b'0')),
        'n'.value(Level::Any),
    ))
    .parse_next(input)
}

/// `"step"` / `"steps"`, case-insensitive.
fn step_word(input: &mut &str) -> ModalResult<()> {
    alt((
        take(5usize).verify_map(|s: &str| s.eq_ignore_ascii_case("steps").then_some(())),
        take(4usize).verify_map(|s: &str| s.eq_ignore_ascii_case("step").then_some(())),
    ))
    .parse_next(input)
}

/// `<n>step[s]`: `n` is a plain digit sequence (not restricted to `1..=7`; real files use
/// `9steps`), counting steps above the auction's last bid rather than a contract level.
fn step_pattern(input: &mut &str) -> ModalResult<CallPattern> {
    let n = digit1
        .verify_map(|s: &str| s.parse::<u16>().ok())
        .parse_next(input)?;
    step_word.parse_next(input)?;
    Ok(CallPattern::Step(u8::try_from(n).unwrap_or(u8::MAX)))
}

fn build_level_strain(level: Level, spec: StrainSpec) -> CallPattern {
    match spec {
        StrainSpec::Variable(var) => CallPattern::Var { level, var },
        StrainSpec::Literal(strains) => {
            if let Level::At(n) = level {
                let mut iter = strains.iter();
                if let (Some(strain), None) = (iter.next(), iter.next()) {
                    if let Some(bid) = Bid::new(n, strain) {
                        return CallPattern::Exact(Call::Bid(bid));
                    }
                }
            }
            CallPattern::Strains { level, strains }
        }
    }
}

fn level_strain(input: &mut &str) -> ModalResult<CallPattern> {
    let lvl = level.parse_next(input)?;
    let spec = strainspec.parse_next(input)?;
    Ok(build_level_strain(lvl, spec))
}

/// Opponents' interference classes: `(any)`, `(bid)`, `(suit)` (extension; always wrapped in
/// parens since they only ever describe the opponents' calls).
fn opp_class(input: &mut &str) -> ModalResult<CallPattern> {
    alt((
        "any".value(CallPattern::Class(OppClass::AnyCall)),
        "suit".value(CallPattern::Class(OppClass::AnySuitBid)),
        "bid".value(CallPattern::Class(OppClass::AnyBid)),
    ))
    .parse_next(input)
}

fn callcore_atom(input: &mut &str) -> ModalResult<CallPattern> {
    alt((
        "XX".value(CallPattern::Exact(Call::Redouble)),
        "X".value(CallPattern::Exact(Call::Double)),
        "P".value(CallPattern::Exact(Call::Pass)),
        "D".value(CallPattern::Exact(Call::Double)),
        "R".value(CallPattern::Exact(Call::Redouble)),
        opp_class,
        step_pattern,
        level_strain,
    ))
    .parse_next(input)
}

/// The level of an already-built pattern, used so that a bare `strainspec` after `/` (as in
/// `4D/H`) can reuse the level established by the chain so far.
fn level_of(pattern: &CallPattern) -> Option<Level> {
    match pattern {
        CallPattern::Exact(Call::Bid(b)) => Some(Level::At(b.level())),
        CallPattern::Strains { level, .. } | CallPattern::Var { level, .. } => Some(*level),
        _ => None,
    }
}

/// `callcore = callcore_atom { "/" ( callcore_atom | strainspec ) }` (ext: `2S/3H`, `4D/H`).
fn callcore(input: &mut &str) -> ModalResult<CallPattern> {
    let first = callcore_atom.parse_next(input)?;
    let mut last_level = level_of(&first);
    let mut parts = vec![first];
    loop {
        let checkpoint = *input;
        if opt('/').parse_next(input)?.is_none() {
            break;
        }
        if let Ok(atom) = callcore_atom.parse_next(input) {
            last_level = level_of(&atom).or(last_level);
            parts.push(atom);
            continue;
        }
        if let Some(lvl) = last_level {
            if let Ok(spec) = strainspec.parse_next(input) {
                parts.push(build_level_strain(lvl, spec));
                continue;
            }
        }
        *input = checkpoint;
        break;
    }
    Ok(if parts.len() == 1 {
        parts.pop().expect("just checked len == 1")
    } else {
        CallPattern::AnyOf(parts)
    })
}

/// Parses one call token (with optional parentheses for the opponents' calls).
pub fn calltok(input: &mut &str) -> ModalResult<(Side, CallPattern)> {
    alt((
        delimited('(', callcore, ')').map(|p| (Side::Them, p)),
        callcore.map(|p| (Side::Us, p)),
    ))
    .parse_next(input)
}

/// Parses a history row: `1N-2C;`, `(1NT)---`, `1C-(1D)-`.
pub fn history(input: &mut &str) -> ModalResult<Vec<(Side, CallPattern)>> {
    let mut out = vec![calltok.parse_next(input)?];
    loop {
        if input.is_empty() {
            break;
        }
        let checkpoint = *input;
        if winnow::token::one_of::<_, _, winnow::error::ContextError>(['-', ';'])
            .parse_next(input)
            .is_err()
        {
            break;
        }
        match calltok.parse_next(input) {
            Ok(tok) => out.push(tok),
            Err(_) => {
                // Trailing marks only (no more calls follow): consume the rest of them.
                while winnow::token::one_of::<_, _, winnow::error::ContextError>(['-', ';'])
                    .parse_next(input)
                    .is_ok()
                {}
                let _ = checkpoint;
                break;
            }
        }
    }
    Ok(out)
}

/// Why a successfully parsed raw call token counts as a BML 2 extension (`NonStandardToken`).
pub(crate) fn nonstandard_reasons(raw: &str) -> Vec<&'static str> {
    let inner = raw.trim_start_matches('(').trim_end_matches(')');
    let mut reasons = Vec::new();
    if inner == "X" {
        reasons.push("bare X (= D)");
    } else if inner == "XX" {
        reasons.push("bare XX (= R)");
    }
    if inner.contains('/') {
        reasons.push("alternative calls (a/b)");
    }
    if inner.starts_with('n') && inner.len() > 1 {
        reasons.push("any-level wildcard (n)");
    }
    if inner.chars().any(|c| matches!(c, 'x' | 'y' | 'z')) {
        reasons.push("lowercase variable (x/y/z)");
    }
    if matches!(inner, "any" | "bid" | "suit") {
        reasons.push("opponent-class wildcard");
    }
    reasons
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_core::{Bid, Strain};

    fn parse(input: &str) -> (Side, CallPattern) {
        let mut s = input;
        let r = calltok(&mut s).unwrap_or_else(|e| panic!("{input}: {e}"));
        assert!(s.is_empty(), "leftover after {input:?}: {s:?}");
        r
    }

    #[test]
    fn exact_calls() {
        assert_eq!(parse("P"), (Side::Us, CallPattern::Exact(Call::Pass)));
        assert_eq!(parse("D"), (Side::Us, CallPattern::Exact(Call::Double)));
        assert_eq!(parse("R"), (Side::Us, CallPattern::Exact(Call::Redouble)));
        assert_eq!(
            parse("1C"),
            (
                Side::Us,
                CallPattern::Exact(Call::Bid(Bid::new(1, Strain::Clubs).unwrap()))
            )
        );
        assert_eq!(
            parse("7N"),
            (
                Side::Us,
                CallPattern::Exact(Call::Bid(Bid::new(7, Strain::NoTrump).unwrap()))
            )
        );
        assert_eq!(
            parse("1NT"),
            (
                Side::Us,
                CallPattern::Exact(Call::Bid(Bid::new(1, Strain::NoTrump).unwrap()))
            )
        );
    }

    #[test]
    fn opponents_are_parenthesised() {
        assert_eq!(parse("(P)"), (Side::Them, CallPattern::Exact(Call::Pass)));
        assert_eq!(
            parse("(1N)"),
            (
                Side::Them,
                CallPattern::Exact(Call::Bid(Bid::new(1, Strain::NoTrump).unwrap()))
            )
        );
    }

    #[test]
    fn multi_strain_literal() {
        let (side, pat) = parse("2HS");
        assert_eq!(side, Side::Us);
        assert_eq!(
            pat,
            CallPattern::Strains {
                level: Level::At(2),
                strains: StrainSet::EMPTY.with(Strain::Hearts).with(Strain::Spades),
            }
        );
    }

    #[test]
    fn red_black_literal() {
        let (_, pat) = parse("2red");
        assert_eq!(
            pat,
            CallPattern::Strains {
                level: Level::At(2),
                strains: StrainSet::RED,
            }
        );
        let (_, pat) = parse("3black");
        assert_eq!(
            pat,
            CallPattern::Strains {
                level: Level::At(3),
                strains: StrainSet::BLACK,
            }
        );
    }

    #[test]
    fn variables() {
        assert_eq!(
            parse("1M"),
            (
                Side::Us,
                CallPattern::Var {
                    level: Level::At(1),
                    var: Var::Major
                }
            )
        );
        assert_eq!(
            parse("2m"),
            (
                Side::Us,
                CallPattern::Var {
                    level: Level::At(2),
                    var: Var::Minor
                }
            )
        );
        assert_eq!(
            parse("1oM"),
            (
                Side::Us,
                CallPattern::Var {
                    level: Level::At(1),
                    var: Var::OtherMajor
                }
            )
        );
        assert_eq!(
            parse("2om"),
            (
                Side::Us,
                CallPattern::Var {
                    level: Level::At(2),
                    var: Var::OtherMinor
                }
            )
        );
        assert_eq!(
            parse("1X"),
            (
                Side::Us,
                CallPattern::Var {
                    level: Level::At(1),
                    var: Var::X
                }
            )
        );
        assert_eq!(
            parse("1Y"),
            (
                Side::Us,
                CallPattern::Var {
                    level: Level::At(1),
                    var: Var::Y
                }
            )
        );
        assert_eq!(
            parse("1Z"),
            (
                Side::Us,
                CallPattern::Var {
                    level: Level::At(1),
                    var: Var::Z
                }
            )
        );
        // lowercase extension
        assert_eq!(
            parse("1x"),
            (
                Side::Us,
                CallPattern::Var {
                    level: Level::At(1),
                    var: Var::X
                }
            )
        );
    }

    #[test]
    fn step_patterns() {
        assert_eq!(parse("1step"), (Side::Us, CallPattern::Step(1)));
        assert_eq!(parse("2steps"), (Side::Us, CallPattern::Step(2)));
        assert_eq!(parse("9steps"), (Side::Us, CallPattern::Step(9)));
    }

    #[test]
    fn any_level_wildcard() {
        assert_eq!(
            parse("nX"),
            (
                Side::Us,
                CallPattern::Var {
                    level: Level::Any,
                    var: Var::X
                }
            )
        );
    }

    #[test]
    fn bare_double_redouble_extension() {
        assert_eq!(parse("X"), (Side::Us, CallPattern::Exact(Call::Double)));
        assert_eq!(parse("XX"), (Side::Us, CallPattern::Exact(Call::Redouble)));
    }

    #[test]
    fn alternative_calls() {
        let (_, pat) = parse("2S/3H");
        assert_eq!(
            pat,
            CallPattern::AnyOf(vec![
                CallPattern::Exact(Call::Bid(Bid::new(2, Strain::Spades).unwrap())),
                CallPattern::Exact(Call::Bid(Bid::new(3, Strain::Hearts).unwrap())),
            ])
        );
        let (_, pat) = parse("4D/H");
        assert_eq!(
            pat,
            CallPattern::AnyOf(vec![
                CallPattern::Exact(Call::Bid(Bid::new(4, Strain::Diamonds).unwrap())),
                CallPattern::Exact(Call::Bid(Bid::new(4, Strain::Hearts).unwrap())),
            ])
        );
    }

    #[test]
    fn opponent_classes() {
        assert_eq!(
            parse("(any)"),
            (Side::Them, CallPattern::Class(OppClass::AnyCall))
        );
        assert_eq!(
            parse("(bid)"),
            (Side::Them, CallPattern::Class(OppClass::AnyBid))
        );
        assert_eq!(
            parse("(suit)"),
            (Side::Them, CallPattern::Class(OppClass::AnySuitBid))
        );
    }

    #[test]
    fn history_sequences() {
        let mut s = "1N-2C;";
        let toks = history(&mut s).unwrap();
        assert!(s.is_empty());
        assert_eq!(toks.len(), 2);

        let mut s = "(1NT)---";
        let toks = history(&mut s).unwrap();
        assert!(s.is_empty());
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].0, Side::Them);

        let mut s = "1C-(1D)-";
        let toks = history(&mut s).unwrap();
        assert!(s.is_empty());
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].0, Side::Us);
        assert_eq!(toks[1].0, Side::Them);

        let mut s = "1N-2C";
        let toks = history(&mut s).unwrap();
        assert!(s.is_empty());
        assert_eq!(toks.len(), 2);

        let mut s = "(1N)-P-(P)---";
        let toks = history(&mut s).unwrap();
        assert!(s.is_empty());
        assert_eq!(toks.len(), 3);
    }

    #[test]
    fn garbage_is_rejected() {
        let mut s = "hello";
        assert!(calltok(&mut s).is_err());
    }
}
