//! Hard constraints from the play.
//!
//! 1. Walk the tricks; `seat_at` gives each card's player.
//! 2. Maintain `played[s]` and `shown_out[s][suit]` (a seat that did not follow the led suit).
//! 3. `hard[s]` = shapes whose length in a shown-out suit equals the number of that suit's cards
//!    the seat has played, and whose length in every other suit is at least the number played.
//! 4. Consistency: per suit, the length intervals of the four seats must admit a total of
//!    exactly 13, and per seat, its four intervals must admit 13 cards; otherwise a warning
//!    (revoke or bad record). The constraints are still returned.
//!
//! Only lengths go into the constraints; card identities go into [`KnownCards`] (the sampler's
//! `fixed`), which is cheaper than card requirements.

use bridge_constraint::{Atom, HandConstraint, KnownCards};
use bridge_core::{PlayHistory, Seat, ShapeSet, Suit};

use crate::PlayWarning;

/// Length constraints and known cards implied by the play so far.
///
/// The caller is responsible for combining the result with `contract` (`hard_constraints` does
/// not take one: it trusts that `history.leader()` is `contract.leader()`, checked by whoever
/// assembles the [`bridge_core::PlayHistory`]) and for adding the viewer's own hand and the
/// exposed dummy to the returned [`KnownCards`] (design doc §5).
pub fn hard_constraints(
    history: &PlayHistory,
) -> ([HandConstraint; 4], KnownCards, Vec<PlayWarning>) {
    // `played[s]`: every card seat `s` has played. `shown_out[s][u]`: `s` failed to follow suit
    // `u` when it was led (so `u`'s original length in `s`'s hand is exactly what `s` has played
    // of it).
    let mut played = [bridge_core::Hand::EMPTY; 4];
    let mut shown_out = [[false; 4]; 4];
    let mut warnings = Vec::new();

    for (t, trick) in history.tricks().enumerate() {
        let Some(led) = trick.cards[0].map(|c| c.suit()) else {
            continue;
        };
        for i in 0..4u8 {
            let Some(card) = trick.cards[i as usize] else {
                break;
            };
            let seat = trick.leader.offset(i);
            let si = seat.index() as usize;
            let ui = card.suit().index() as usize;

            // A card of a suit this seat has already shown out of: the record contradicts
            // itself (the seat claimed to hold none of `card.suit()` in an earlier trick).
            if shown_out[si][ui] {
                warnings.push(PlayWarning::RevokeSuspected { trick: t, seat });
            }

            played[si] = played[si].with(card);

            if i > 0 && card.suit() != led {
                shown_out[si][led.index() as usize] = true;
            }
        }
    }

    // `min_len[s][u]`: the number of `u` cards `s` has played, a lower bound on `s`'s original
    // holding in `u` (exact when `shown_out[s][u]`).
    let mut min_len = [[0u8; 4]; 4];
    for seat in Seat::ALL {
        let si = seat.index() as usize;
        for suit in Suit::ALL {
            min_len[si][suit.index() as usize] = played[si].holding(suit).len();
        }
    }

    // Consistency (design doc §6 item 7). Each seat's original length in suit `u` lies in
    // `min_len[s][u]..=max_len[s][u]`, where `max_len` is `min_len` for a shown-out suit and 13
    // otherwise. The record is inconsistent when those intervals cannot hold a real deal:
    //
    // - per suit, the four seats' lengths must add up to exactly 13, so `Σ_s min_len ≤ 13 ≤
    //   Σ_s max_len`. The lower-bound half is defensive (`PlayHistory::play` rejects a card
    //   played twice, so the played cards of one suit never exceed 13); the upper-bound half is
    //   reachable: when all four seats have shown out of a suit, their exact lengths must still
    //   add up to 13, which a revoke (or a bad record) breaks.
    // - per seat, the lengths must add up to exactly 13, so `Σ_u max_len ≥ 13`. A seat that has
    //   shown out of every suit is held to exactly the cards it has played, fewer than 13 before
    //   the last trick. (`Σ_u min_len ≤ 13` always holds: a seat plays at most 13 cards.)
    //
    // Either failure leaves some seat's `ShapeSet` (or the deal as a whole) empty; the warning
    // names the suit, and the constraints are still returned (the caller treats them as
    // `EmptySupport`).
    let max_len = |si: usize, ui: usize| -> u8 {
        if shown_out[si][ui] {
            min_len[si][ui]
        } else {
            13
        }
    };
    let mut inconsistent = [false; 4];
    for suit in Suit::ALL {
        let ui = suit.index() as usize;
        let lo: u16 = (0..4).map(|si| u16::from(min_len[si][ui])).sum();
        let hi: u16 = (0..4).map(|si| u16::from(max_len(si, ui))).sum();
        if lo > 13 || hi < 13 {
            inconsistent[ui] = true;
        }
    }
    for si in 0..4 {
        let hi: u16 = (0..4).map(|ui| u16::from(max_len(si, ui))).sum();
        if hi < 13 {
            // Every suit of this seat is capped (it has shown out of all four): flag each.
            inconsistent.fill(true);
        }
    }
    for suit in Suit::ALL {
        if inconsistent[suit.index() as usize] {
            warnings.push(PlayWarning::Inconsistent { suit });
        }
    }

    let hard = Seat::ALL.map(|seat| {
        let si = seat.index() as usize;
        let mut lens = [(0u8, 13u8); 4];
        for suit in Suit::ALL {
            let ui = suit.index() as usize;
            let n = min_len[si][ui];
            lens[ui] = if shown_out[si][ui] { (n, n) } else { (n, 13) };
        }
        HandConstraint::Atom(Atom {
            shapes: ShapeSet::from_suit_lens(lens),
            ..Atom::ANY
        })
    });

    let known = KnownCards::EMPTY.with_play(history);

    (hard, known, warnings)
}
