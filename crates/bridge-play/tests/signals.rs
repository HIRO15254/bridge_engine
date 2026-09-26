//! Table-driven tests for `signal_constraints` (design doc §7.3 / §7.4): attitude and count
//! signals (`Standard` and `UpsideDown`), and first discards (`Attitude`, `OddEven`,
//! `Lavinthal`).

use bridge_core::{Card, Hand, Seat, Strain, Suit};
use bridge_play::{
    DiscardTable, FirstDiscard, Polarity, SignalContext, SignalEvent, SignalKind, SignalTable,
    signal_constraints,
};

fn card(s: &str) -> Card {
    s.parse().unwrap()
}

fn hand(s: &str) -> Hand {
    s.parse().unwrap()
}

fn signals(attitude: Polarity, count: Polarity, confidence: f32) -> SignalTable {
    SignalTable {
        attitude,
        count,
        confidence,
    }
}

fn no_discards() -> DiscardTable {
    DiscardTable {
        first: FirstDiscard::Unknown,
        polarity: Polarity::Unknown,
    }
}

fn discards(first: FirstDiscard, polarity: Polarity) -> DiscardTable {
    DiscardTable { first, polarity }
}

fn attitude_event(c: &str) -> SignalEvent {
    SignalEvent {
        seat: Seat::East,
        card: card(c),
        kind: SignalKind::Attitude,
        context: SignalContext::None,
    }
}

fn assert_branch(alts: &[(bridge_play::HandConstraint, f32)], w: f32, good: &str, bad: &str) {
    let total: f32 = alts.iter().map(|(_, w)| w).sum();
    assert!((total - 1.0).abs() < 1e-6, "weights sum to {total}");
    assert!((alts[0].1 - w).abs() < 1e-6, "weight {}", alts[0].1);
    assert!(alts[0].0.satisfies(hand(good)), "{good} should satisfy");
    assert!(!alts[0].0.satisfies(hand(bad)), "{bad} should not satisfy");
}

#[test]
fn attitude_standard_high_encourages() {
    let s = signals(Polarity::Standard, Polarity::Unknown, 0.7);
    let alts = signal_constraints(attitude_event("S8"), &s, &no_discards());
    assert_branch(&alts, 0.7, "K973.AKQ.AKQ.AKQ", "9743.AKQ.AKQ.AKQ");
}

#[test]
fn attitude_standard_low_discourages() {
    let s = signals(Polarity::Standard, Polarity::Unknown, 0.7);
    let alts = signal_constraints(attitude_event("S3"), &s, &no_discards());
    assert_branch(&alts, 0.7, "9743.AKQ.AKQ.AKQ", "K973.AKQ.AKQ.AKQ");
}

#[test]
fn attitude_upside_down_flips() {
    let s = signals(Polarity::UpsideDown, Polarity::Unknown, 0.7);
    let alts = signal_constraints(attitude_event("S8"), &s, &no_discards());
    assert_branch(&alts, 0.7, "9743.AKQ.AKQ.AKQ", "K973.AKQ.AKQ.AKQ");
}

/// The undecided `Six` boundary splits into two 0.35 branches plus `ANY` at 0.3.
#[test]
fn attitude_mid_card_splits() {
    let s = signals(Polarity::Standard, Polarity::Unknown, 0.7);
    let alts = signal_constraints(attitude_event("S6"), &s, &no_discards());
    assert_eq!(alts.len(), 3);
    let total: f32 = alts.iter().map(|(_, w)| w).sum();
    assert!((total - 1.0).abs() < 1e-6);
    assert!((alts[0].1 - 0.35).abs() < 1e-6);
    assert!((alts[1].1 - 0.35).abs() < 1e-6);
    assert!((alts[2].1 - 0.3).abs() < 1e-6);
    let with_honor = hand("K973.AKQ.AKQ.AKQ");
    let without_honor = hand("9743.AKQ.AKQ.AKQ");
    // One branch wants the honour, the other denies it; together they cover every hand.
    assert!(alts[0].0.satisfies(with_honor) != alts[1].0.satisfies(with_honor));
    assert!(alts[0].0.satisfies(without_honor) != alts[1].0.satisfies(without_honor));
}

/// `Unknown` disables the rule.
#[test]
fn attitude_unknown_fires_nothing() {
    let s = signals(Polarity::Unknown, Polarity::Unknown, 0.7);
    assert!(signal_constraints(attitude_event("S8"), &s, &no_discards()).is_empty());
}

fn count_event(prior: &str, second: &str) -> SignalEvent {
    SignalEvent {
        seat: Seat::East,
        card: card(second),
        kind: SignalKind::Count,
        context: SignalContext::Count(card(prior)),
    }
}

#[test]
fn count_standard_high_then_low_is_even() {
    let s = signals(Polarity::Unknown, Polarity::Standard, 0.7);
    let alts = signal_constraints(count_event("S9", "S4"), &s, &no_discards());
    assert_branch(
        &alts,
        0.7,
        "AK54.AKQ.AKQ.AKQ", // 4 spades (even)
        "AK543.AKQ.AKQ.AK", // 5 spades (odd)
    );
}

#[test]
fn count_standard_low_then_high_is_odd() {
    let s = signals(Polarity::Unknown, Polarity::Standard, 0.7);
    let alts = signal_constraints(count_event("S4", "S9"), &s, &no_discards());
    assert_branch(
        &alts,
        0.7,
        "AK543.AKQ.AKQ.AK", // 5 spades (odd)
        "AK54.AKQ.AKQ.AKQ", // 4 spades (even)
    );
}

#[test]
fn count_upside_down_flips() {
    let s = signals(Polarity::Unknown, Polarity::UpsideDown, 0.7);
    let alts = signal_constraints(count_event("S9", "S4"), &s, &no_discards());
    assert_branch(
        &alts,
        0.7,
        "AK543.AKQ.AKQ.AK", // 5 spades (odd, since upside-down flips the meaning)
        "AK54.AKQ.AKQ.AKQ", // 4 spades (even)
    );
}

fn discard_event(trump: Strain, led: Suit, c: &str) -> SignalEvent {
    SignalEvent {
        seat: Seat::East,
        card: card(c),
        kind: SignalKind::FirstDiscard,
        context: SignalContext::Discard { trump, led },
    }
}

#[test]
fn discard_attitude_high_shows_honor() {
    let d = discards(FirstDiscard::Attitude, Polarity::Standard);
    let alts = signal_constraints(
        discard_event(Strain::Spades, Suit::Hearts, "D8"),
        &SignalTable::default(),
        &d,
    );
    assert_branch(&alts, 0.6, "AKQ.32.K973.AKQJ", "AKQ.32.9743.AKQJ");
}

#[test]
fn discard_attitude_low_denies_honor() {
    let d = discards(FirstDiscard::Attitude, Polarity::Standard);
    let alts = signal_constraints(
        discard_event(Strain::Spades, Suit::Hearts, "D3"),
        &SignalTable::default(),
        &d,
    );
    assert_branch(&alts, 0.6, "AKQ.32.9743.AKQJ", "AKQ.32.K973.AKQJ");
}

/// `OddEven`'s odd-rank rule fires unconditionally (design doc §7.4 states no polarity
/// dependency for it, unlike `Attitude` discards): `Standard`, `UpsideDown` and even `Unknown`
/// polarity all show honour on an odd-rank discard.
#[test]
fn discard_odd_even_odd_shows_honor_regardless_of_polarity() {
    for polarity in [Polarity::Standard, Polarity::UpsideDown, Polarity::Unknown] {
        let d = discards(FirstDiscard::OddEven, polarity);
        let alts = signal_constraints(
            discard_event(Strain::Spades, Suit::Hearts, "D7"), // odd rank
            &SignalTable::default(),
            &d,
        );
        assert_branch(&alts, 0.6, "AKQ.32.K973.AKQJ", "AKQ.32.9743.AKQJ");
    }
}

/// Notrump, hearts led, a diamond discarded: the two suits a suit-preference discard chooses
/// between are clubs (low) and spades (high), never the discarded suit itself (the full table,
/// suit contracts included, is in `tests/review.rs`).
#[test]
fn discard_lavinthal_high_card_prefers_the_higher_side_suit() {
    let d = discards(FirstDiscard::Lavinthal, Polarity::Standard);
    let alts = signal_constraints(
        discard_event(Strain::NoTrump, Suit::Hearts, "D8"),
        &SignalTable::default(),
        &d,
    );
    assert_branch(&alts, 0.6, "K973.32.9743.T98", "9873.32.KQJ3.T98");
}

#[test]
fn discard_lavinthal_low_card_prefers_the_lower_side_suit() {
    let d = discards(FirstDiscard::Lavinthal, Polarity::Standard);
    let alts = signal_constraints(
        discard_event(Strain::NoTrump, Suit::Hearts, "D3"),
        &SignalTable::default(),
        &d,
    );
    assert_branch(&alts, 0.6, "9873.32.9743.AT9", "K973.32.KQJ3.T98");
}

/// `OddEven`'s even branch defers to the same suit-preference mapping as `Lavinthal`.
#[test]
fn discard_odd_even_even_defers_to_lavinthal() {
    let d = discards(FirstDiscard::OddEven, Polarity::Standard);
    let alts = signal_constraints(
        discard_event(Strain::NoTrump, Suit::Hearts, "D8"), // even, high
        &SignalTable::default(),
        &d,
    );
    assert_branch(&alts, 0.6, "K973.32.9743.T98", "9873.32.KQJ3.T98");
}

#[test]
fn first_discard_unknown_fires_nothing() {
    let d = no_discards();
    let alts = signal_constraints(
        discard_event(Strain::Spades, Suit::Hearts, "D8"),
        &SignalTable::default(),
        &d,
    );
    assert!(alts.is_empty());
}
