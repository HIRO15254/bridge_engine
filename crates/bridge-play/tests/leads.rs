//! Table-driven tests for `lead_constraints` (design doc §7.2): one case per row of the table,
//! each checked against a satisfying and a non-satisfying candidate original hand.

use bridge_core::{Bid, Card, Contract, Doubling, Hand, Seat, Strain};
use bridge_play::{HonorLeads, LeadStyle, SpotLead, lead_constraints};

fn card(s: &str) -> Card {
    s.parse().unwrap()
}

fn hand(s: &str) -> Hand {
    s.parse().unwrap()
}

fn contract(level: u8, strain: Strain) -> Contract {
    Contract {
        bid: Bid::new(level, strain).unwrap(),
        declarer: Seat::South,
        doubling: Doubling::Undoubled,
    }
}

fn style(spot: SpotLead, honors: HonorLeads, confidence: f32) -> LeadStyle {
    LeadStyle {
        spot,
        honors,
        confidence,
    }
}

/// Every rule returns `[(C, w), (ANY, 1 - w)]`, summing to 1; `C` is checked against `good` and
/// `bad`.
fn check(led: &str, contract: &Contract, style: &LeadStyle, w: f32, good: &str, bad: &str) {
    let alts = lead_constraints(card(led), contract, style);
    assert_eq!(alts.len(), 2, "led {led}: expected [(C, w), (ANY, 1 - w)]");
    let total: f32 = alts.iter().map(|(_, w)| w).sum();
    assert!(
        (total - 1.0).abs() < 1e-6,
        "led {led}: weights sum to {total}"
    );
    assert!(
        (alts[0].1 - w).abs() < 1e-6,
        "led {led}: weight {}",
        alts[0].1
    );
    assert!(
        alts[0].0.satisfies(hand(good)),
        "led {led}: {good} should satisfy the rule"
    );
    assert!(
        !alts[0].0.satisfies(hand(bad)),
        "led {led}: {bad} should not satisfy the rule"
    );
}

#[test]
fn fourth_best() {
    let c = contract(4, Strain::Spades);
    let s = style(SpotLead::FourthBest, HonorLeads::Unknown, 0.8);
    // Spades: AKQ532 (len 6, exactly 3 cards above the 5: A K Q).
    check(
        "S5",
        &c,
        &s,
        0.8,
        "AKQ532.AKQ.43.32",
        // Same length, but 4 cards above the 5 (A K Q J).
        "AKQJ53.AKQ.43.32",
    );
}

#[test]
fn third_fifth_short_branch() {
    let c = contract(4, Strain::Hearts);
    let s = style(SpotLead::ThirdFifth, HonorLeads::Unknown, 0.8);
    // Spades: KQ6 (len 3, exactly 2 cards above the 6: K Q).
    check(
        "S6",
        &c,
        &s,
        0.8,
        "KQ6.AKQ.AKQ.AKQ4",
        // Len 3 but only 1 card above the 6.
        "K26.AKQ.AKQ.AKQ4",
    );
}

#[test]
fn third_fifth_long_branch() {
    let c = contract(4, Strain::Hearts);
    let s = style(SpotLead::ThirdFifth, HonorLeads::Unknown, 0.8);
    // Spades: AKQJ642 (len 7, exactly 4 cards above the 6: A K Q J).
    check(
        "S6",
        &c,
        &s,
        0.8,
        "AKQJ642.AK.32.32",
        // Len 7 but 5 cards above the 6 (A K Q J T).
        "AKQJT64.AK.32.32",
    );
}

#[test]
fn spot_attitude_high_denies() {
    let c = contract(3, Strain::NoTrump);
    let s = style(SpotLead::Attitude, HonorLeads::Unknown, 0.7);
    check(
        "S8",
        &c,
        &s,
        0.7,
        "8642.AKQ.AKQ.AKQ", // no honour in spades
        "K8642.AK.AKQ.AKQ", // a king in spades
    );
}

#[test]
fn spot_attitude_low_shows() {
    let c = contract(3, Strain::NoTrump);
    let s = style(SpotLead::Attitude, HonorLeads::Unknown, 0.7);
    check(
        "S3",
        &c,
        &s,
        0.7,
        "K973.AKQ.AKQ.AKQ", // a king in spades
        "9743.AKQ.AKQ.AKQ", // no honour in spades
    );
}

#[test]
fn standard_king_vs_suit() {
    let c = contract(4, Strain::Hearts);
    let s = style(SpotLead::Unknown, HonorLeads::Standard, 0.9);
    check(
        "SK",
        &c,
        &s,
        0.9,
        "AK543.AKQ.AKQ.AK", // holds the ace
        "K5432.AKQ.AKQ.AK", // neither ace nor queen
    );
}

#[test]
fn standard_king_vs_suit_queen_branch() {
    let c = contract(4, Strain::Hearts);
    let s = style(SpotLead::Unknown, HonorLeads::Standard, 0.9);
    assert!(
        lead_constraints(card("SK"), &c, &s)[0]
            .0
            .satisfies(hand("KQ543.AKQ.AKQ.AK"))
    );
}

#[test]
fn standard_king_vs_nt_kqj_branch() {
    let c = contract(3, Strain::NoTrump);
    let s = style(SpotLead::Unknown, HonorLeads::Standard, 0.85);
    check(
        "SK",
        &c,
        &s,
        0.85,
        "KQJ54.AKQ.AKQ.AK", // K Q J
        "KJT54.AKQ.AKQ.AK", // J and T but no queen, and not A-J-T either
    );
}

#[test]
fn standard_king_vs_nt_akjt_branch() {
    let c = contract(3, Strain::NoTrump);
    let s = style(SpotLead::Unknown, HonorLeads::Standard, 0.85);
    assert!(
        lead_constraints(card("SK"), &c, &s)[0]
            .0
            .satisfies(hand("AKJT5.AKQ.AKQ.AK"))
    );
}

#[test]
fn rusinow_king_shows_ace() {
    let c = contract(4, Strain::Spades);
    let s = style(SpotLead::Unknown, HonorLeads::Rusinow, 0.9);
    check(
        "SK",
        &c,
        &s,
        0.9,
        "AK543.AKQ.AKQ.AK",
        "K5432.AKQ.AKQ.AK", // no ace
    );
}

#[test]
fn rusinow_nine_shows_ten() {
    let c = contract(4, Strain::Spades);
    let s = style(SpotLead::Unknown, HonorLeads::Rusinow, 0.9);
    check(
        "S9",
        &c,
        &s,
        0.9,
        "T9543.AKQ.AKQ.A2",
        "985432.AKQ.AKQ.2", // no ten
    );
}

#[test]
fn jack_denies_jack() {
    let c = contract(4, Strain::Spades);
    let s = style(SpotLead::Unknown, HonorLeads::JackDenies, 0.9);
    check(
        "SJ",
        &c,
        &s,
        0.9,
        "J543.AKQ.AKQ.AK2", // no A, K or Q
        "AJ54.AKQ.AKQ.AK2", // holds the ace
    );
}

#[test]
fn jack_denies_ten_jack_branch() {
    let c = contract(4, Strain::Spades);
    let s = style(SpotLead::Unknown, HonorLeads::JackDenies, 0.9);
    check(
        "ST",
        &c,
        &s,
        0.9,
        "AJT54.AKQ.AKQ.AK", // jack plus one of A K Q
        "T854.AKQ.AKQ.AK2", // no jack and no nine
    );
}

#[test]
fn jack_denies_ten_nine_branch() {
    let c = contract(4, Strain::Spades);
    let s = style(SpotLead::Unknown, HonorLeads::JackDenies, 0.9);
    assert!(
        lead_constraints(card("ST"), &c, &s)[0]
            .0
            .satisfies(hand("T954.AKQ.AKQ.AK2")) // nine, and none of A K Q J
    );
}

/// A rule that is not held for the active convention (undecided in v1, §7.5 item 4) fires no
/// event at all.
#[test]
fn ace_lead_has_no_rule_under_standard() {
    let c = contract(4, Strain::Hearts);
    let s = style(SpotLead::Unknown, HonorLeads::Standard, 0.9);
    assert!(lead_constraints(card("SA"), &c, &s).is_empty());
}

/// `Unknown` disables the rule entirely.
#[test]
fn unknown_style_fires_nothing() {
    let c = contract(4, Strain::Hearts);
    let s = LeadStyle {
        spot: SpotLead::Unknown,
        honors: HonorLeads::Unknown,
        confidence: 0.8,
    };
    assert!(lead_constraints(card("S5"), &c, &s).is_empty());
    assert!(lead_constraints(card("SK"), &c, &s).is_empty());
}
