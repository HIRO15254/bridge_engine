//! Hard constraints from the play.
//!
//! 1. Walk the tricks; `seat_at` gives each card's player.
//! 2. Maintain `played[s]` and `shown_out[s][suit]` (a seat that did not follow the led suit).
//! 3. `hard[s]` = shapes whose length in a shown-out suit equals the number of that suit's cards
//!    the seat has played, and whose length in every other suit is at least the number played.
//! 4. Consistency: per suit, the minimum lengths must sum to at most 13; otherwise a warning
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

    // Consistency: the four seats' minimum lengths in one suit cannot add up to more than the
    // 13 cards of that suit (a revoke, or a bad record). Every seat's own total across suits is
    // its own played-card count, which `PlayHistory` already bounds at 13 (at most 13 completed
    // tricks), so no analogous per-seat check is needed.
    //
    // `PlayWarning::Inconsistent` is defensive: `PlayHistory::play` (via `check`) rejects any
    // already-played card unconditionally, so every suit's 13 physical cards can be distributed
    // across the 4 seats' play counts at most once each, and `sum` above can never exceed 13 for
    // a history built through the safe `PlayHistory` API. The branch is kept for untrusted or
    // parsed input that builds a `PlayHistory` outside that API's checks (e.g. a future BML
    // record loader that replays a possibly-corrupt log), where two seats' recorded plays could
    // disagree about who held a card. No test can trigger it today without an unchecked
    // `PlayHistory` constructor, which `bridge-core` does not currently expose.
    for suit in Suit::ALL {
        let ui = suit.index() as usize;
        let sum: u16 = Seat::ALL
            .iter()
            .map(|s| min_len[s.index() as usize][ui] as u16)
            .sum();
        if sum > 13 {
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
