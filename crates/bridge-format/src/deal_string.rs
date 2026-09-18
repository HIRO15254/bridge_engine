//! The deal string: `N:AKQ.234.AKQ.2345 <E> <S> <W>`.
//!
//! Hands are listed clockwise from the named seat; each hand is spades-first with `.` between
//! suits, `T` for ten, and `-` for an unknown hand (which yields a [`PartialDeal`]).
//!
//! Reading accepts ranks in any order and `10` for a ten. A hand that is not exactly thirteen
//! distinct cards, or that shares a card with another hand, is an error in [`parse`]; the
//! lenient PBN and LIN readers report the same conditions as warnings instead.

use core::fmt::Write as _;

use bridge_core::{Deal, Hand, Seat};

use crate::{ParseError, pbn::PartialDeal};

/// Parses a deal string, allowing unknown (`-`) hands.
pub fn parse(input: &str) -> Result<PartialDeal, ParseError> {
    let (deal, problems) = parse_lenient(input)?;
    match problems.into_iter().next() {
        Some(message) => Err(ParseError { line: 1, message }),
        None => Ok(deal),
    }
}

/// Parses a deal string, dropping hands that are not thirteen distinct, unshared cards.
///
/// The `Err` case is an unrecoverable form (no `<seat>:` prefix, not four hands); the returned
/// strings describe every hand that was dropped.
pub(crate) fn parse_lenient(input: &str) -> Result<(PartialDeal, Vec<String>), ParseError> {
    let s = input.trim();
    let err = |message: String| ParseError { line: 1, message };
    let Some((seat_part, rest)) = s.split_once(':') else {
        return Err(err(format!("deal {s:?} has no '<seat>:' prefix")));
    };
    let first: Seat = seat_part
        .parse()
        .map_err(|_| err(format!("unknown first seat {seat_part:?}")))?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    if fields.len() != 4 {
        return Err(err(format!("expected 4 hands, found {}", fields.len())));
    }
    let mut hands = [None; 4];
    let mut problems = Vec::new();
    let mut seen = Hand::EMPTY;
    for (k, field) in fields.iter().enumerate() {
        let seat = first.offset(k as u8);
        if *field == "-" {
            continue;
        }
        let hand: Hand = match field.parse() {
            Ok(hand) => hand,
            Err(e) => {
                problems.push(format!("{seat}: {e}"));
                continue;
            }
        };
        if hand.len() != 13 {
            problems.push(format!("{seat} holds {} cards, expected 13", hand.len()));
            continue;
        }
        if let Some(card) = seen.intersect(hand).cards().next() {
            problems.push(format!("{seat}: card {card} is held by two seats"));
            continue;
        }
        seen = seen.union(hand);
        hands[seat.index() as usize] = Some(hand);
    }
    Ok((PartialDeal { hands }, problems))
}

/// Writes a deal string starting from `first`, ranks descending.
pub fn write(deal: &Deal, first: Seat) -> String {
    let mut out = String::with_capacity(72);
    let _ = write!(out, "{first}:");
    for k in 0..4 {
        if k > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{}", deal.hand(first.offset(k)));
    }
    out
}

/// Writes a partial deal, `-` for unknown hands.
pub(crate) fn write_partial(deal: &PartialDeal, first: Seat) -> String {
    let mut out = String::with_capacity(72);
    let _ = write!(out, "{first}:");
    for k in 0..4 {
        if k > 0 {
            out.push(' ');
        }
        match deal.hands[first.offset(k).index() as usize] {
            Some(hand) => {
                let _ = write!(out, "{hand}");
            }
            None => out.push('-'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = "N:QJ6.K652.J85.T98 873.J97.AT764.Q4 K5.T83.KQ9.A7652 AT942.AQ4.32.KJ3";

    #[test]
    fn parses_and_writes_a_full_deal() {
        let partial = parse(FULL).unwrap();
        let deal = partial.complete().unwrap();
        assert_eq!(write(&deal, Seat::North), FULL);
        // Any starting seat, lowercase and `10` are accepted; ranks may be in any order.
        let rotated =
            parse("e:873.J97.AT764.Q4 K5.T83.KQ9.A7652 AT942.AQ4.32.KJ3 qj6.k652.j85.1098")
                .unwrap()
                .complete()
                .unwrap();
        assert_eq!(rotated, deal);
        assert_eq!(
            write(&deal, Seat::West),
            "W:AT942.AQ4.32.KJ3 QJ6.K652.J85.T98 873.J97.AT764.Q4 K5.T83.KQ9.A7652"
        );
    }

    #[test]
    fn unknown_hands_and_errors() {
        let partial = parse("N:QJ6.K652.J85.T98 - K5.T83.KQ9.A7652 -").unwrap();
        assert!(partial.hands[Seat::North.index() as usize].is_some());
        assert!(partial.hands[Seat::East.index() as usize].is_none());
        assert!(partial.complete().is_none());
        assert_eq!(
            write_partial(&partial, Seat::South),
            "S:K5.T83.KQ9.A7652 - QJ6.K652.J85.T98 -"
        );

        assert!(
            parse("QJ6.K652.J85.T98 873.J97.AT764.Q4 K5.T83.KQ9.A7652 AT942.AQ4.32.KJ3").is_err()
        );
        assert!(
            parse("X:QJ6.K652.J85.T98 873.J97.AT764.Q4 K5.T83.KQ9.A7652 AT942.AQ4.32.KJ3").is_err()
        );
        assert!(parse("N:QJ6.K652.J85.T98 873.J97.AT764.Q4 K5.T83.KQ9.A7652").is_err());
        // twelve cards
        assert!(
            parse("N:QJ6.K652.J85.T9 873.J97.AT764.Q4 K5.T83.KQ9.A7652 AT942.AQ4.32.KJ3").is_err()
        );
        // duplicate within a suit and across hands
        assert!(
            parse("N:QQ6.K652.J85.T98 873.J97.AT764.Q4 K5.T83.KQ9.A7652 AT942.AQ4.32.KJ3").is_err()
        );
        assert!(
            parse("N:QJ6.K652.J85.T98 873.J97.AT764.Q4 K5.T83.KQ9.A7652 AT942.AQ4.32.KJ6").is_err()
        );
        assert!(parse("").is_err());
    }
}
