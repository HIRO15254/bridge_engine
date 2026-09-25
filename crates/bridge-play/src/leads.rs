//! Opening-lead rules (design doc §7.2).
//!
//! | Event | Agreement | Constraint on the leader's original hand (suit `u`, led rank `r`) | w |
//! | --- | --- | --- | --- |
//! | spot ≤ 9 | 4th best | `len[u] ≥ 4 ∧ exactly 3 cards above r` | 0.8 |
//! | spot | 3rd/5th | `(len 3 ∧ 2 above) ∨ (len ≥ 5 ∧ 4 above)` | 0.8 |
//! | spot | attitude | high (≥ 7): no honour in `u`; low (≤ 6): ≥ 1 honour | 0.7 |
//! | K vs suit | standard | `AK ∨ KQ` | 0.9 |
//! | K vs NT | standard | `KQ ∧ (J ∨ T)` (or AKJT branch) | 0.85 |
//! | K/Q/J/T/9 | Rusinow | holds the next higher honour | 0.9 |
//! | J | jack denies | no A, K, Q; T: `J ∧ (A ∨ K ∨ Q)`, or `9` denying all four | 0.9 |
//!
//! Trick 0's opening lead only (§7.2 item 2 of the "undecided" list: whether the same tables
//! apply to a later lead is left open in v1). The ace and queen leads have no rule under
//! `Standard`/`JackDenies` (§7.5 item 4); `Rusinow` does cover the queen.

use bridge_constraint::HandConstraint;
use bridge_core::{Card, Contract, Rank, Strain, Suit};

use crate::vocab;
use crate::{HonorLeads, LeadStyle, SpotLead};

/// The ranks the active honour-lead convention treats as an honour-lead event, in leader's-hand
/// order. Only these ranks reach [`honor_lead`]; every other led rank is a spot-card event.
pub(crate) fn honor_ranks(style: HonorLeads) -> &'static [Rank] {
    match style {
        HonorLeads::Standard => &[Rank::King],
        HonorLeads::Rusinow => &[Rank::King, Rank::Queen, Rank::Jack, Rank::Ten, Rank::Nine],
        HonorLeads::JackDenies => &[Rank::Jack, Rank::Ten],
        HonorLeads::Unknown => &[],
    }
}

/// The weighted constraints implied by an opening lead of `card` against `contract`.
pub fn lead_constraints(
    card: Card,
    contract: &Contract,
    style: &LeadStyle,
) -> Vec<(HandConstraint, f32)> {
    let u = card.suit();
    let r = card.rank();

    if honor_ranks(style.honors).contains(&r) {
        return honor_lead(u, r, contract, style);
    }
    if vocab::is_spot(r) {
        return spot_lead(u, r, style);
    }
    Vec::new()
}

fn spot_lead(u: Suit, r: Rank, style: &LeadStyle) -> Vec<(HandConstraint, f32)> {
    match style.spot {
        SpotLead::Unknown => Vec::new(),
        SpotLead::FourthBest => vocab::branch(
            vocab::shaped_atom(
                vocab::len(u, 4, 13),
                vec![vocab::req(vocab::above(u, r), 3..=3)],
            ),
            style.confidence,
        ),
        SpotLead::ThirdFifth => {
            let three = vocab::shaped_atom(
                vocab::len(u, 3, 3),
                vec![vocab::req(vocab::above(u, r), 2..=2)],
            );
            let five = vocab::shaped_atom(
                vocab::len(u, 5, 13),
                vec![vocab::req(vocab::above(u, r), 4..=4)],
            );
            vocab::branch(three.or(five), style.confidence)
        }
        SpotLead::Attitude => {
            // The table's own resolution of the r = 6 boundary (unlike the signal attitude
            // table, this one folds it into "low" rather than splitting it): high (>= 7) denies
            // an honour, low (<= 6) shows one.
            let req = if vocab::numeric(r) >= 7 {
                vocab::req(vocab::honors(u), 0..=0)
            } else {
                vocab::req(vocab::honors(u), 1..=4)
            };
            vocab::branch(vocab::atom(vec![req]), style.confidence)
        }
    }
}

fn honor_lead(
    u: Suit,
    r: Rank,
    contract: &Contract,
    style: &LeadStyle,
) -> Vec<(HandConstraint, f32)> {
    match style.honors {
        HonorLeads::Unknown => Vec::new(),
        HonorLeads::Standard => standard_honor_lead(u, r, contract, style.confidence),
        HonorLeads::Rusinow => {
            // K -> A, Q -> K, J -> Q, T -> J, 9 -> T: the next rank up, which stays in range for
            // every rank `honor_ranks(Rusinow)` lists (King's successor is Ace, the top rank).
            let above = Rank::from_index(r.index() + 1);
            vocab::branch(
                vocab::atom(vec![vocab::req(vocab::one(u, above), 1..=1)]),
                style.confidence,
            )
        }
        HonorLeads::JackDenies => jack_denies_lead(u, r, style.confidence),
    }
}

fn standard_honor_lead(
    u: Suit,
    r: Rank,
    contract: &Contract,
    w: f32,
) -> Vec<(HandConstraint, f32)> {
    if r != Rank::King {
        // The ace lead has no rule under `Standard` (§7.5 item 4); `honor_ranks` never sends
        // anything but a king here in practice, but this keeps the function total.
        return Vec::new();
    }
    let atom = if contract.bid.strain() == Strain::NoTrump {
        // KQJ or KQT.
        let kq_plus = vocab::atom(vec![
            vocab::req(vocab::one(u, Rank::Queen), 1..=1),
            vocab::req(
                vocab::one(u, Rank::Jack).union(vocab::one(u, Rank::Ten)),
                1..=2,
            ),
        ]);
        // AKJT.
        let akjt = vocab::atom(vec![vocab::req(
            vocab::one(u, Rank::Ace)
                .union(vocab::one(u, Rank::Jack))
                .union(vocab::one(u, Rank::Ten)),
            3..=3,
        )]);
        kq_plus.or(akjt)
    } else {
        let ak = vocab::atom(vec![vocab::req(vocab::one(u, Rank::Ace), 1..=1)]);
        let kq = vocab::atom(vec![vocab::req(vocab::one(u, Rank::Queen), 1..=1)]);
        ak.or(kq)
    };
    vocab::branch(atom, w)
}

fn jack_denies_lead(u: Suit, r: Rank, w: f32) -> Vec<(HandConstraint, f32)> {
    let akq = vocab::one(u, Rank::Ace)
        .union(vocab::one(u, Rank::King))
        .union(vocab::one(u, Rank::Queen));
    let atom = match r {
        Rank::Jack => vocab::atom(vec![vocab::req(akq, 0..=0)]),
        Rank::Ten => {
            let j_and_higher = vocab::atom(vec![
                vocab::req(vocab::one(u, Rank::Jack), 1..=1),
                vocab::req(akq, 1..=3),
            ]);
            let nine_denies_all = vocab::atom(vec![
                vocab::req(vocab::one(u, Rank::Nine), 1..=1),
                vocab::req(akq.union(vocab::one(u, Rank::Jack)), 0..=0),
            ]);
            j_and_higher.or(nine_denies_all)
        }
        // The queen lead has no rule under `JackDenies` (§7.5 item 4); unreachable via
        // `honor_ranks(JackDenies)`, which only lists jack and ten.
        _ => return Vec::new(),
    };
    vocab::branch(atom, w)
}
