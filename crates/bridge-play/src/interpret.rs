//! `interpret_play`: history → constraints per seat.

use bridge_constraint::{DnfOptions, HandConstraint, KnownCards};
use bridge_core::{Card, Contract, PlayHistory, Seat, Strain, Suit};

use crate::{
    PlayAgreements, SignalContext, SignalEvent, SignalKind, hard_constraints, lead_constraints,
    signal_constraints,
};

/// Alternatives kept per seat after combining every event that fired (design doc §7.6, the same
/// cross-product/prune/truncate/renormalise procedure as 07-bidding.md Step B, reimplemented here
/// rather than shared: L5 does not depend on L3).
const K: usize = 8;

/// The result of [`interpret_play`].
#[derive(Clone, Debug)]
pub struct PlayInterpretation {
    /// Cards now known to belong to each seat's original hand.
    pub known: KnownCards,
    /// Hard length constraints per seat.
    pub hard: [HandConstraint; 4],
    /// Soft weighted alternatives per seat (combined from every rule that fired).
    pub soft: [Vec<(HandConstraint, f32)>; 4],
    /// Which rule fired on which card.
    pub events: Vec<PlayEvent>,
    /// Inconsistencies found.
    pub warnings: Vec<PlayWarning>,
}

impl PlayInterpretation {
    /// The spec's return type: `hard ∧ each soft branch` per seat.
    pub fn into_seats(self) -> [Vec<(HandConstraint, f32)>; 4] {
        core::array::from_fn(|i| {
            self.soft[i]
                .iter()
                .map(|(c, w)| (self.hard[i].clone().and(c.clone()), *w))
                .collect()
        })
    }
}

/// An audit-trail entry.
#[derive(Clone, PartialEq, Debug)]
pub struct PlayEvent {
    /// The seat.
    pub seat: Seat,
    /// The card.
    pub card: Card,
    /// The rule name.
    pub rule: &'static str,
}

/// An inconsistency in the record.
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum PlayWarning {
    /// The minimum lengths in `suit` exceed 13 (revoke or bad record).
    #[error("the played cards in {suit:?} add up to more than 13 across the four seats")]
    Inconsistent {
        /// The suit.
        suit: Suit,
    },
    /// A seat played a suit it had shown out of.
    #[error("seat {seat:?} played a suit it had shown out of, in trick {trick}")]
    RevokeSuspected {
        /// Trick index.
        trick: usize,
        /// The seat.
        seat: Seat,
    },
}

/// Derives hard and soft constraints from the play so far. `agreements` is indexed by seat;
/// rules apply to defenders only.
pub fn interpret_play(
    history: &PlayHistory,
    contract: &Contract,
    agreements: &[PlayAgreements; 4],
) -> PlayInterpretation {
    let (hard, known, warnings) = hard_constraints(history);
    let defenders = contract.declarer.side().other();

    let mut soft: [Vec<(HandConstraint, f32)>; 4] =
        core::array::from_fn(|_| vec![(HandConstraint::ANY, 1.0)]);
    let mut events: Vec<PlayEvent> = Vec::new();

    let fire = |seat: Seat,
                card: Card,
                rule: &'static str,
                alts: Vec<(HandConstraint, f32)>,
                soft: &mut [Vec<(HandConstraint, f32)>; 4],
                events: &mut Vec<PlayEvent>| {
        if alts.is_empty() {
            return;
        }
        let si = seat.index() as usize;
        events.push(PlayEvent { seat, card, rule });
        let existing = core::mem::take(&mut soft[si]);
        soft[si] = combine(existing, &hard[si], alts);
    };

    // Opening lead: trick 0, card 0.
    if let Some(&opening) = history.cards().first() {
        let leader = history.leader();
        if defenders.contains(leader) {
            let table = &agreements[leader.index() as usize].leads;
            let style = if contract.bid.strain() == Strain::NoTrump {
                &table.vs_nt
            } else {
                &table.vs_suit
            };
            let alts = lead_constraints(opening, contract, style);
            fire(leader, opening, "lead", alts, &mut soft, &mut events);
        }
    }

    let trump = history.trump();
    // The first two cards each defender has played in each suit, indexed `[seat][suit]`, with
    // whether that card was count-eligible (a non-winning spot following declarer's side's lead
    // of the suit). The count signal (§7.3) reads the seat's first two cards of the suit, so it
    // fires on the second card only when both of the first two were eligible: a pair of spots
    // that follows an honour, a winning card or a card played to partner's lead of the suit does
    // not start at the top of the holding, and its high-low says nothing about the parity.
    let mut suit_cards: [[Vec<(Card, bool)>; 4]; 4] = Default::default();
    // Whether a defender's first discard (§7.4) has already been used.
    let mut first_discard_used = [false; 4];

    for trick in history.tricks() {
        let Some(led) = trick.cards[0].map(|c| c.suit()) else {
            continue;
        };
        let leader_is_defender = defenders.contains(trick.leader);

        for i in 0..4u8 {
            let Some(card) = trick.cards[i as usize] else {
                break;
            };
            let seat = trick.leader.offset(i);
            if !defenders.contains(seat) {
                continue;
            }
            let si = seat.index() as usize;
            let won_trick = trick.winner == Some(seat);

            // Attitude: 3rd hand (this defender's partner led) follows with a spot, and does not
            // win the trick.
            if leader_is_defender
                && i == 2
                && trick.winner.is_some()
                && !won_trick
                && card.suit() == led
                && is_spot(card)
            {
                let event = SignalEvent {
                    seat,
                    card,
                    kind: SignalKind::Attitude,
                    context: SignalContext::None,
                };
                let alts =
                    signal_constraints(event, &agreements[si].signals, &agreements[si].discards);
                fire(seat, card, "signal:attitude", alts, &mut soft, &mut events);
            }

            // Count: the seat's second card of a suit, both of its first two cards of that suit
            // being non-winning spot follows to declarer's side's leads.
            {
                let eligible = card.suit() == led
                    && !leader_is_defender
                    && trick.winner.is_some()
                    && !won_trick
                    && is_spot(card);
                let cards = &mut suit_cards[si][card.suit().index() as usize];
                if cards.len() < 2 {
                    cards.push((card, eligible));
                    if let [(prior, true), (_, true)] = cards.as_slice() {
                        let event = SignalEvent {
                            seat,
                            card,
                            kind: SignalKind::Count,
                            context: SignalContext::Count(*prior),
                        };
                        let alts = signal_constraints(
                            event,
                            &agreements[si].signals,
                            &agreements[si].discards,
                        );
                        fire(seat, card, "signal:count", alts, &mut soft, &mut events);
                    }
                }
            }

            // First discard: neither following suit nor ruffing.
            let is_discard = card.suit() != led && Some(card.suit()) != trump.suit();
            if is_discard && !first_discard_used[si] {
                first_discard_used[si] = true;
                let event = SignalEvent {
                    seat,
                    card,
                    kind: SignalKind::FirstDiscard,
                    context: SignalContext::Discard { trump, led },
                };
                let alts =
                    signal_constraints(event, &agreements[si].signals, &agreements[si].discards);
                fire(seat, card, "signal:discard", alts, &mut soft, &mut events);
            }
        }
    }

    PlayInterpretation {
        known,
        hard,
        soft,
        events,
        warnings,
    }
}

fn is_spot(card: Card) -> bool {
    card.rank().index() <= bridge_core::Rank::Nine.index()
}

/// Combines the alternatives of one more event into a seat's running soft constraint (§7.6,
/// same procedure as 07-bidding.md Step B): cross product with `and`, drop combinations that
/// contradict `hard` or are otherwise unsatisfiable (checked via [`HandConstraint::to_dnf`] and
/// [`bridge_constraint::Atom::is_trivially_unsat`] rather than the still-provisional
/// `is_satisfiable`, per the phase-5 notes), truncate to [`K`] by weight (the `K - 1` heaviest
/// plus the dropped weight folded into an `ANY` entry, so every hand satisfying `hard` keeps
/// positive soft mass) and renormalise. Falls
/// back to `[(ANY, 1.0)]` with a `tracing::warn!` if every combination turns out unsatisfiable.
fn combine(
    existing: Vec<(HandConstraint, f32)>,
    hard: &HandConstraint,
    new_alts: Vec<(HandConstraint, f32)>,
) -> Vec<(HandConstraint, f32)> {
    let mut combos: Vec<(HandConstraint, f32)> =
        Vec::with_capacity(existing.len() * new_alts.len());
    for (c1, w1) in &existing {
        for (c2, w2) in &new_alts {
            let combined = and_skipping_any(c1, c2);
            if !possibly_satisfiable(&hard.clone().and(combined.clone())) {
                continue;
            }
            combos.push((combined, w1 * w2));
        }
    }

    if combos.is_empty() {
        tracing::warn!("play soft constraints became unsatisfiable; falling back to ANY");
        return vec![(HandConstraint::ANY, 1.0)];
    }

    combos.sort_by(|a, b| b.1.total_cmp(&a.1));
    if combos.len() > K {
        // Keep the remainder (§7.1: it always goes to `ANY`). Dropping the tail outright would
        // drop the all-`ANY` product too, and a legal hand matching none of the surviving
        // branches would get soft mass 0, turning a soft signal into a hard exclusion.
        let dropped: f32 = combos[K - 1..].iter().map(|(_, w)| w).sum();
        combos.truncate(K - 1);
        match combos.iter_mut().find(|(c, _)| is_any(c)) {
            Some((_, w)) => *w += dropped,
            None => combos.push((HandConstraint::ANY, dropped)),
        }
        // The remainder can outweigh kept branches; keep the list sorted by weight.
        combos.sort_by(|a, b| b.1.total_cmp(&a.1));
    }
    let total: f32 = combos.iter().map(|(_, w)| w).sum();
    if total > 0.0 {
        for (_, w) in &mut combos {
            *w /= total;
        }
    }
    combos
}

/// `c1 ∧ c2`, returning the other side unchanged when one side is the `ANY` atom. Every rule's
/// `(ANY, 1 − w)` remainder would otherwise pile up as `And([ANY, ANY, ..])` nodes, one per event,
/// in the constraints handed to the sampler.
fn and_skipping_any(c1: &HandConstraint, c2: &HandConstraint) -> HandConstraint {
    if is_any(c1) {
        c2.clone()
    } else if is_any(c2) {
        c1.clone()
    } else {
        c1.clone().and(c2.clone())
    }
}

fn is_any(c: &HandConstraint) -> bool {
    matches!(c, HandConstraint::Atom(a) if *a == bridge_constraint::Atom::ANY)
}

/// A summary satisfiability check built from primitives that are already exact (DNF expansion
/// and [`bridge_constraint::Atom::is_trivially_unsat`]), rather than through
/// [`HandConstraint::is_satisfiable`], whose own doc comment calls it provisional pending the
/// exact sampler (bridge-constraint 2.4). A term surviving `to_dnf` (which already drops every
/// trivially-unsatisfiable term) is not a guarantee the atom is satisfiable in full generality
/// (card-requirement interactions can still be empty), but it is the same guarantee
/// `is_satisfiable` currently offers, obtained without depending on that name.
fn possibly_satisfiable(c: &HandConstraint) -> bool {
    match c.to_dnf(&DnfOptions::default()) {
        Ok(dnf) => !dnf.terms.is_empty(),
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_constraint::Atom;

    fn atom_hcp(lo: u8, hi: u8) -> HandConstraint {
        HandConstraint::Atom(Atom {
            hcp: lo..=hi,
            ..Atom::ANY
        })
    }

    /// Combining 3 events of 2 branches each stays within `K = 8` and sums to 1 (design doc
    /// §7.6 / the roadmap's `combine_caps_at_k`).
    #[test]
    fn combine_caps_at_k_and_sums_to_one() {
        let hard = HandConstraint::ANY;
        let mut combos = vec![(HandConstraint::ANY, 1.0)];
        for _ in 0..3 {
            let alts = vec![(atom_hcp(10, 20), 0.5), (HandConstraint::ANY, 0.5)];
            combos = combine(combos, &hard, alts);
        }
        assert_eq!(combos.len(), 8);
        assert!(combos.len() <= K);
        let total: f32 = combos.iter().map(|(_, w)| w).sum();
        assert!((total - 1.0).abs() < 1e-5, "total = {total}");
    }

    /// Regression: truncation used to drop the lowest-weight products, including the all-`ANY`
    /// one, so a legal hand matching none of the surviving branches got soft mass 0 (a soft
    /// signal acting as a hard exclusion). After truncation every hand satisfying `hard` must
    /// keep positive mass.
    #[test]
    fn combine_truncation_keeps_the_any_remainder() {
        let hard = HandConstraint::ANY;
        let events = [
            vec![
                (atom_hcp(0, 5), 0.5),
                (atom_hcp(6, 10), 0.3),
                (HandConstraint::ANY, 0.2),
            ],
            vec![(atom_hcp(0, 8), 0.7), (HandConstraint::ANY, 0.3)],
            vec![(atom_hcp(3, 12), 0.6), (HandConstraint::ANY, 0.4)],
        ];
        let mut combos = vec![(HandConstraint::ANY, 1.0)];
        for alts in events {
            combos = combine(combos, &hard, alts);
        }
        assert!(combos.len() <= K);
        let total: f32 = combos.iter().map(|(_, w)| w).sum();
        assert!((total - 1.0).abs() < 1e-5, "total = {total}");
        // A 20-HCP hand matches only the all-ANY product (every other branch caps HCP at 12).
        let strong: bridge_core::Hand = "AKQJ.AKQ.432.432".parse().unwrap();
        let mass: f32 = combos
            .iter()
            .filter(|(c, _)| c.satisfies(strong))
            .map(|(_, w)| w)
            .sum();
        assert!(mass > 0.0, "{combos:?}");
        // The remainder carries every dropped product's weight: the kept branches other than
        // ANY sum to exactly what they weighed before truncation (renormalisation is a no-op
        // because the full product already sums to 1).
        assert!(
            combos
                .iter()
                .any(|(c, w)| is_any(c) && *w >= 0.2 * 0.3 * 0.4)
        );
    }

    /// When every combination contradicts `hard`, `combine` falls back to `[(ANY, 1.0)]`.
    #[test]
    fn combine_falls_back_to_any_when_every_combination_is_unsatisfiable() {
        let hard = atom_hcp(0, 5);
        let existing = vec![(HandConstraint::ANY, 1.0)];
        let alts = vec![(atom_hcp(10, 20), 1.0)];
        let combos = combine(existing, &hard, alts);
        assert_eq!(combos.len(), 1);
        assert_eq!(combos[0].1, 1.0);
        match &combos[0].0 {
            HandConstraint::Atom(a) => assert_eq!(*a, Atom::ANY),
            other => panic!("expected the ANY fallback, got {other:?}"),
        }
    }
}
