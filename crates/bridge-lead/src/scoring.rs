//! A minimal standard duplicate-scoring table, used only to rank leads under
//! [`crate::LeadScoring::Score`] (`14-lead.md` §3.1).
//!
//! `bridge-core` has no scoring module yet (`14-lead.md` §5 item 2); this is deliberately not a
//! public, general-purpose scorer (no partial-score carryover, no slam bonuses beyond the flat
//! small/grand-slam bonus, no honours). It exists only to give [`crate::LeadScoring::Score`] a
//! declarer score to rank by, from the declaring side's point of view.

use bridge_core::{Contract, Doubling, Strain};

/// The declarer's duplicate score for making or defeating `contract` by `tricks` (the number of
/// tricks the declaring side actually took, `0..=13`), `vulnerable` from the declaring side's
/// point of view. Positive if made, negative if defeated.
pub(crate) fn declarer_score(contract: Contract, vulnerable: bool, tricks: u8) -> i32 {
    let required = contract.bid.tricks_required();
    if tricks >= required {
        let overtricks = tricks - required;
        made_score(contract, vulnerable, overtricks)
    } else {
        let undertricks = required - tricks;
        -penalty(contract.doubling, vulnerable, undertricks)
    }
}

/// Score for making the contract with `overtricks` beyond the required number of tricks.
fn made_score(contract: Contract, vulnerable: bool, overtricks: u8) -> i32 {
    let level = i32::from(contract.bid.level());
    let strain = contract.bid.strain();
    let trick_value = trick_value(strain);

    let base_trick_score = match contract.doubling {
        Doubling::Undoubled => level * trick_value + first_trick_bonus(strain),
        Doubling::Doubled => 2 * (level * trick_value + first_trick_bonus(strain)),
        Doubling::Redoubled => 4 * (level * trick_value + first_trick_bonus(strain)),
    };

    let game_bonus = if base_trick_score >= 100 {
        if vulnerable { 500 } else { 300 }
    } else {
        50
    };

    let slam_bonus = match level {
        6 => {
            if vulnerable {
                750
            } else {
                500
            }
        }
        7 => {
            if vulnerable {
                1500
            } else {
                1000
            }
        }
        _ => 0,
    };

    let insult = match contract.doubling {
        Doubling::Undoubled => 0,
        Doubling::Doubled => 50,
        Doubling::Redoubled => 100,
    };

    let overtrick_score = match contract.doubling {
        Doubling::Undoubled => i32::from(overtricks) * trick_value,
        Doubling::Doubled => i32::from(overtricks) * if vulnerable { 200 } else { 100 },
        Doubling::Redoubled => i32::from(overtricks) * if vulnerable { 400 } else { 200 },
    };

    base_trick_score + game_bonus + slam_bonus + insult + overtrick_score
}

/// Points per trick bid in `strain` (minor 20, major 30; declarer's first notrump trick is
/// scored separately by [`first_trick_bonus`]).
fn trick_value(strain: Strain) -> i32 {
    match strain {
        Strain::Clubs | Strain::Diamonds => 20,
        Strain::Hearts | Strain::Spades => 30,
        Strain::NoTrump => 30,
    }
}

/// Notrump's first trick is worth 40, not 30; every other strain (and every other notrump
/// trick) uses [`trick_value`] alone.
fn first_trick_bonus(strain: Strain) -> i32 {
    if strain == Strain::NoTrump { 10 } else { 0 }
}

/// Penalty for going down `undertricks` (`1..=13`), doubled state and vulnerability, from the
/// standard duplicate table.
fn penalty(doubling: Doubling, vulnerable: bool, undertricks: u8) -> i32 {
    match doubling {
        Doubling::Undoubled => i32::from(undertricks) * if vulnerable { 100 } else { 50 },
        Doubling::Doubled | Doubling::Redoubled => {
            let multiplier = if doubling == Doubling::Redoubled {
                2
            } else {
                1
            };
            let mut total = 0i32;
            for i in 1..=i32::from(undertricks) {
                let step = if !vulnerable {
                    match i {
                        1 => 100,
                        2 | 3 => 200,
                        _ => 300,
                    }
                } else {
                    match i {
                        1 => 200,
                        _ => 300,
                    }
                };
                total += step;
            }
            total * multiplier
        }
    }
}

#[cfg(test)]
mod tests {
    use bridge_core::{Bid, Seat};

    use super::*;

    fn contract(level: u8, strain: Strain, doubling: Doubling) -> Contract {
        Contract {
            bid: Bid::new(level, strain).unwrap(),
            declarer: Seat::North,
            doubling,
        }
    }

    #[test]
    fn three_nt_making_exactly_is_400_non_vulnerable() {
        let c = contract(3, Strain::NoTrump, Doubling::Undoubled);
        // 40 (first NT trick) + 2*30 + 50 (part/game threshold: 100 base -> game bonus 300) = 100
        assert_eq!(declarer_score(c, false, 9), 400);
    }

    #[test]
    fn three_nt_making_exactly_is_600_vulnerable() {
        let c = contract(3, Strain::NoTrump, Doubling::Undoubled);
        assert_eq!(declarer_score(c, true, 9), 600);
    }

    #[test]
    fn one_club_making_exactly_is_partscore() {
        let c = contract(1, Strain::Clubs, Doubling::Undoubled);
        // trick score 20, below 100 -> partscore bonus 50.
        assert_eq!(declarer_score(c, false, 7), 70);
    }

    #[test]
    fn four_spades_down_one_non_vulnerable_undoubled() {
        let c = contract(4, Strain::Spades, Doubling::Undoubled);
        assert_eq!(declarer_score(c, false, 9), -50);
    }

    #[test]
    fn four_spades_down_one_vulnerable_undoubled() {
        let c = contract(4, Strain::Spades, Doubling::Undoubled);
        assert_eq!(declarer_score(c, true, 9), -100);
    }

    #[test]
    fn doubled_down_three_non_vulnerable() {
        let c = contract(3, Strain::NoTrump, Doubling::Doubled);
        // -(100 + 200 + 200) = -500
        assert_eq!(declarer_score(c, false, 6), -500);
    }

    #[test]
    fn small_slam_making_vulnerable() {
        let c = contract(6, Strain::Spades, Doubling::Undoubled);
        // trick score 180 (>=100 -> game bonus 500) + slam bonus 750
        assert_eq!(declarer_score(c, true, 12), 180 + 500 + 750);
    }
}
