//! Signal rules: attitude, count and first discard (design doc §7.3 / §7.4).
//!
//! | Event | Agreement | Constraint | w |
//! | --- | --- | --- | --- |
//! | 3rd hand follows partner's lead with a spot, does not win | attitude Standard | high (≥ 7): ≥ 1 honour; low (≤ 5): no honour; `Six`: split 0.35/0.35 | 0.7 |
//! | 2nd spot to declarer's/dummy's led suit | count Standard | 1st > 2nd: even length; else odd | 0.7 |
//!
//! | First-discard convention | Constraint | w |
//! | --- | --- | --- |
//! | Attitude | high: ≥ 1 honour in the discarded suit; low: none | 0.6 |
//! | OddEven | odd rank: ≥ 1 honour in the discarded suit; even: as `Lavinthal` | 0.6 |
//! | Lavinthal | high: ≥ 1 honour in the higher of the two suits other than the discarded suit and trump (vs NT: and the suit led); low: the lower one; upside-down swaps | 0.6 |
//!
//! `Lavinthal`/`OddEven`'s mapping to a specific suit is design doc §11 item 4, left "undecided"
//! there; this reads it as standard suit preference: the discarded suit is the one the defender
//! does not want, so the choice is between the two suits other than it and trump (against
//! notrump, other than it and the suit led), ordered by [`Suit`]'s own `Clubs < Diamonds <
//! Hearts < Spades`. When one of the two is the suit led (a suit contract with a side suit led),
//! the defender is void there, so preferring it carries no honour inference.

use bridge_constraint::{Atom, HandConstraint};
use bridge_core::{Card, Rank, Seat, Strain, Suit};

use crate::vocab::{self, Height};
use crate::{DiscardTable, FirstDiscard, Polarity, SignalTable};

/// A defender's card in a position where a signal applies.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SignalEvent {
    /// The defender.
    pub seat: Seat,
    /// The card.
    pub card: Card,
    /// What kind of signal the position calls for.
    pub kind: SignalKind,
    /// Information a single card cannot carry on its own: the earlier spot card compared against
    /// for a [`SignalKind::Count`] event, or the trump strain and suit led for a
    /// [`SignalKind::FirstDiscard`] event (needed to name the two side suits, §7.4). `None` for
    /// [`SignalKind::Attitude`], which needs only `card`.
    pub context: SignalContext,
}

/// See [`SignalEvent::context`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignalContext {
    /// No extra context needed.
    None,
    /// The earlier spot card the same seat played in the same suit.
    Count(Card),
    /// The trump strain and the suit led to the trick being discarded from.
    Discard {
        /// The trump strain.
        trump: Strain,
        /// The suit led.
        led: Suit,
    },
}

/// Signal positions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(missing_docs)]
pub enum SignalKind {
    Attitude,
    Count,
    FirstDiscard,
}

/// The weighted constraints implied by a signal.
pub fn signal_constraints(
    event: SignalEvent,
    signals: &SignalTable,
    discards: &DiscardTable,
) -> Vec<(HandConstraint, f32)> {
    match event.kind {
        SignalKind::Attitude => attitude_signal(event.card, signals),
        SignalKind::Count => count_signal(event, signals),
        SignalKind::FirstDiscard => first_discard_signal(event, discards),
    }
}

/// `[(C, w), (ANY, 1 - w)]` for a plain high/low card, or a three-way split at `Height::Mid`
/// (`w/2` each, matching §7.3's "r = 6: split 0.35/0.35 at w = 0.7").
fn height_branches(
    h: Height,
    high: HandConstraint,
    low: HandConstraint,
    w: f32,
) -> Vec<(HandConstraint, f32)> {
    match h {
        Height::High => vocab::branch(high, w),
        Height::Low => vocab::branch(low, w),
        Height::Mid => {
            let half = w / 2.0;
            vec![(high, half), (low, half), (HandConstraint::ANY, 1.0 - w)]
        }
    }
}

fn attitude_signal(card: Card, signals: &SignalTable) -> Vec<(HandConstraint, f32)> {
    if signals.attitude == Polarity::Unknown {
        return Vec::new();
    }
    let u = card.suit();
    let has_honor = vocab::atom(vec![vocab::req(vocab::honors(u), 1..=4)]);
    let no_honor = vocab::atom(vec![vocab::req(vocab::honors(u), 0..=0)]);
    let (high, low) = match signals.attitude {
        Polarity::UpsideDown => (no_honor, has_honor),
        _ => (has_honor, no_honor),
    };
    height_branches(vocab::height(card.rank()), high, low, signals.confidence)
}

fn count_signal(event: SignalEvent, signals: &SignalTable) -> Vec<(HandConstraint, f32)> {
    if signals.count == Polarity::Unknown {
        return Vec::new();
    }
    let SignalContext::Count(prior) = event.context else {
        return Vec::new();
    };
    let u = event.card.suit();
    let first_higher = prior.rank() > event.card.rank();
    let is_even = match signals.count {
        Polarity::UpsideDown => !first_higher,
        _ => first_higher,
    };
    let even = HandConstraint::Atom(Atom {
        shapes: vocab::lens(u, &[2, 4, 6, 8, 10, 12]),
        ..Atom::ANY
    });
    let odd = HandConstraint::Atom(Atom {
        shapes: vocab::lens(u, &[3, 5, 7, 9, 11, 13]),
        ..Atom::ANY
    });
    vocab::branch(if is_even { even } else { odd }, signals.confidence)
}

fn first_discard_signal(event: SignalEvent, discards: &DiscardTable) -> Vec<(HandConstraint, f32)> {
    let SignalContext::Discard { trump, led } = event.context else {
        return Vec::new();
    };
    let u = event.card.suit();
    let r = event.card.rank();
    let w = 0.6;
    match discards.first {
        FirstDiscard::Unknown => Vec::new(),
        FirstDiscard::Attitude => attitude_discard(u, r, discards.polarity, w),
        FirstDiscard::OddEven => {
            if vocab::numeric(r) % 2 == 1 {
                // Unlike `Attitude` discards, the odd-rank rule has no polarity dependency in
                // the design (08-play.md §7.4): it fires unconditionally, independent of
                // `discards.polarity` (including `Polarity::Unknown`).
                vocab::branch(vocab::atom(vec![vocab::req(vocab::honors(u), 1..=4)]), w)
            } else {
                lavinthal_discard(u, r, trump, led, discards.polarity, w)
            }
        }
        FirstDiscard::Lavinthal => lavinthal_discard(u, r, trump, led, discards.polarity, w),
    }
}

fn attitude_discard(u: Suit, r: Rank, polarity: Polarity, w: f32) -> Vec<(HandConstraint, f32)> {
    if polarity == Polarity::Unknown {
        return Vec::new();
    }
    let has_honor = vocab::atom(vec![vocab::req(vocab::honors(u), 1..=4)]);
    let no_honor = vocab::atom(vec![vocab::req(vocab::honors(u), 0..=0)]);
    let (high, low) = match polarity {
        Polarity::UpsideDown => (no_honor, has_honor),
        _ => (has_honor, no_honor),
    };
    height_branches(vocab::height(r), high, low, w)
}

/// The two suits a suit-preference discard of suit `discarded` chooses between (design doc §7.4,
/// §11 item 4), ascending by [`Suit`]'s own order: the suits other than the discarded suit and
/// trumps. Against notrump that leaves three, and the suit led (which the discarder has just
/// shown out of) is dropped as well.
fn preference_suits(discarded: Suit, trump: Strain, led: Suit) -> Vec<Suit> {
    let mut suits: Vec<Suit> = Suit::ALL
        .into_iter()
        .filter(|&s| s != discarded && Some(s) != trump.suit())
        .collect();
    if suits.len() > 2 {
        suits.retain(|&s| s != led);
    }
    suits.sort();
    suits
}

/// Suit preference: a high card asks for the higher of [`preference_suits`], a low card for the
/// lower, and upside-down polarity swaps the two (§7.1; `Unknown` polarity reads as standard,
/// since the convention itself already fixes a meaning). The preferred suit is read as holding an
/// honour there, except when it is the suit led, which the discarder has just shown out of (a
/// suit contract with a side suit led): that branch carries no honour inference and is `ANY`.
fn lavinthal_discard(
    u: Suit,
    r: Rank,
    trump: Strain,
    led: Suit,
    polarity: Polarity,
    w: f32,
) -> Vec<(HandConstraint, f32)> {
    let suits = preference_suits(u, trump, led);
    let (Some(&low_suit), Some(&high_suit)) = (suits.first(), suits.last()) else {
        return Vec::new();
    };
    let prefers = |suit: Suit| {
        if suit == led {
            HandConstraint::ANY
        } else {
            vocab::atom(vec![vocab::req(vocab::honors(suit), 1..=4)])
        }
    };
    let (high, low) = match polarity {
        Polarity::UpsideDown => (prefers(low_suit), prefers(high_suit)),
        _ => (prefers(high_suit), prefers(low_suit)),
    };
    height_branches(vocab::height(r), high, low, w)
}
