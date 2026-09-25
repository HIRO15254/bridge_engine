//! Natural inference: a generic meaning for any call, used when the system has no entry.
//!
//! The rules are a small ordered table over a [`CallContext`] (role, call kind, level, …) and
//! produce constraints from [`NaturalParams`]. [`classify`] is system-independent and testable
//! on its own. Accuracy is measured three ways (hold-out against compiled systems, reproduction
//! rate, corpus satisfaction); see `docs/design/06-system.md`.

use core::ops::RangeInclusive;

use bridge_constraint::HandConstraint;
use bridge_core::{Auction, Bid, Call, Seat, Suit};

use crate::pattern::StrainSet;

/// Parameters of the natural rules (`#+NATURAL:` or a TOML file).
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NaturalParams {
    /// Opening strength.
    pub opening_hcp: RangeInclusive<u8>,
    /// Minimum length of a 1-of-a-minor opening.
    pub open_1m_len: u8,
    /// Minimum length of a 1-of-a-major opening.
    pub open_1major_len: u8,
    /// Notrump openings by level: `(level, hcp)`.
    pub nt: Vec<(u8, RangeInclusive<u8>)>,
    /// Weak two: `(min length, hcp)`.
    pub weak_two: (u8, RangeInclusive<u8>),
    /// Preempts by level (3, 4, 5): `(min length, hcp)`.
    pub preempt: Vec<(u8, u8, RangeInclusive<u8>)>,
    /// Minimum HCP of a strong 2♣.
    pub strong_two_c: u8,
    /// Overcalls: `(min length, hcp)` for 1-level, 2-level, jump.
    pub overcall: [(u8, RangeInclusive<u8>); 3],
    /// 1NT overcall.
    pub nt_overcall: RangeInclusive<u8>,
    /// Takeout double: `(min hcp, max in their suit, min in unbid suits)`.
    pub takeout_double: (u8, u8, u8),
    /// Responses.
    pub response: ResponseParams,
    /// Opener's rebids.
    pub rebid: RebidParams,
    /// Advances of an overcall.
    pub advance: AdvanceParams,
    /// HCP shift in the balancing seat (default −3).
    pub balancing_shift: i8,
    /// A raise of partner's suit implies support even without text.
    pub implicit_raise_support: bool,
}

/// Response parameters.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResponseParams {
    /// New suit at the 1-level: `(min length, min hcp)`.
    pub new_suit_1: (u8, u8),
    /// New suit at the 2-level.
    pub new_suit_2: (u8, u8),
    /// Simple raise: `(min support, hcp)`.
    pub raise: (u8, RangeInclusive<u8>),
    /// Jump raise.
    pub jump_raise: (u8, RangeInclusive<u8>),
    /// Notrump responses by level.
    pub nt: Vec<(u8, RangeInclusive<u8>)>,
    /// Minimum hcp of a jump shift.
    pub jump_shift: u8,
}

/// Opener's rebid parameters.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RebidParams {
    /// Minimum hcp of a reverse.
    pub reverse: u8,
    /// Jump rebid of own suit.
    pub jump_rebid: RangeInclusive<u8>,
    /// 1NT rebid.
    pub nt_1: RangeInclusive<u8>,
    /// 2NT rebid.
    pub nt_2: RangeInclusive<u8>,
    /// Simple raise of responder's suit.
    pub raise: RangeInclusive<u8>,
    /// Jump raise of responder's suit.
    pub jump_raise: RangeInclusive<u8>,
}

/// Advancer parameters.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AdvanceParams {
    /// Simple raise of the overcall: `(min support, hcp)`.
    pub raise: (u8, RangeInclusive<u8>),
    /// New suit: `(min length, min hcp)`.
    pub new_suit: (u8, u8),
    /// Minimum hcp of a cue bid.
    pub cue: u8,
}

impl Default for NaturalParams {
    /// SAYC-like defaults (`docs/design/06-system.md` §8.1).
    fn default() -> NaturalParams {
        NaturalParams {
            opening_hcp: 12..=21,
            open_1m_len: 3,
            open_1major_len: 5,
            nt: vec![(1, 15..=17), (2, 20..=21), (3, 25..=27)],
            weak_two: (6, 5..=10),
            preempt: vec![(3, 7, 5..=9), (4, 8, 5..=10), (5, 8, 5..=11)],
            strong_two_c: 22,
            overcall: [(5, 8..=16), (5, 10..=16), (6, 5..=10)],
            nt_overcall: 15..=18,
            takeout_double: (12, 2, 3),
            response: ResponseParams::default(),
            rebid: RebidParams::default(),
            advance: AdvanceParams::default(),
            balancing_shift: -3,
            implicit_raise_support: true,
        }
    }
}

impl Default for ResponseParams {
    fn default() -> ResponseParams {
        ResponseParams {
            new_suit_1: (4, 6),
            new_suit_2: (5, 10),
            raise: (3, 6..=9),
            jump_raise: (4, 10..=12),
            nt: vec![(1, 6..=10), (2, 11..=12), (3, 13..=15)],
            jump_shift: 17,
        }
    }
}

impl Default for RebidParams {
    fn default() -> RebidParams {
        RebidParams {
            reverse: 17,
            jump_rebid: 16..=18,
            nt_1: 12..=14,
            nt_2: 18..=19,
            raise: 12..=15,
            jump_raise: 16..=18,
        }
    }
}

impl Default for AdvanceParams {
    fn default() -> AdvanceParams {
        AdvanceParams {
            raise: (3, 6..=9),
            new_suit: (5, 8),
            cue: 10,
        }
    }
}

/// A player's role in the auction.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[allow(missing_docs)]
pub enum Role {
    Opener,
    Responder,
    Overcaller,
    Advancer,
    Balancer,
}

/// Kinds of double.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[allow(missing_docs)]
pub enum DoubleKind {
    Takeout,
    Penalty,
    Negative,
    Responsive,
    Support,
    Unknown,
}

/// What kind of call was made.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CallKind {
    /// Pass.
    Pass,
    /// Double of a given kind.
    Double(DoubleKind),
    /// Redouble.
    Redouble,
    /// A bid.
    Bid {
        /// A suit not bid before by this side.
        new_suit: bool,
        /// Partner's suit.
        raise: bool,
        /// Notrump.
        nt: bool,
        /// Levels skipped.
        jump: u8,
        /// Opponents' suit.
        cue: bool,
        /// Opener's reverse.
        reverse: bool,
        /// Own suit again.
        rebid_own: bool,
    },
}

/// The context of one call, derived from the auction alone.
#[derive(Clone, Debug)]
pub struct CallContext {
    /// Role.
    pub role: Role,
    /// Kind.
    pub kind: CallKind,
    /// The call.
    pub call: Call,
    /// Level (0 for pass/double/redouble).
    pub level: u8,
    /// Opener position `1..=4`.
    pub position: u8,
    /// The caller passed earlier.
    pub passed_hand: bool,
    /// We / they vulnerable.
    pub vul: (bool, bool),
    /// Both sides have bid.
    pub competitive: bool,
    /// Partner's last call.
    pub partner_last: Option<Call>,
    /// Partner's constraint so far, if an interpretation is available.
    pub partner_constraint: Option<HandConstraint>,
    /// Strains bid by our side.
    pub our_suits: StrainSet,
    /// Strains bid by their side.
    pub their_suits: StrainSet,
    /// Agreed suit.
    pub agreed_suit: Option<Suit>,
    /// Last bid in the auction.
    pub last_bid: Option<Bid>,
    /// Partner's last call was forcing.
    pub forcing_situation: bool,
}

/// Classifies call `index` of `auction` from the point of view of its caller.
pub fn classify(auction: &Auction, index: usize, owner: Seat) -> CallContext {
    todo!("phase 3")
}

/// The result of natural inference.
#[derive(Clone, Debug)]
pub struct Inference {
    /// The constraint.
    pub constraint: HandConstraint,
    /// Confidence in `0..=1`.
    pub confidence: f32,
    /// The rule that fired.
    pub rule: &'static str,
    /// Human-readable explanation.
    pub explanation: String,
}

/// The natural-inference engine.
#[derive(Clone, Debug)]
pub struct NaturalInference {
    params: NaturalParams,
}

impl NaturalInference {
    /// Builds an engine from parameters.
    pub fn new(params: NaturalParams) -> NaturalInference {
        NaturalInference { params }
    }

    /// The parameters.
    pub fn params(&self) -> &NaturalParams {
        &self.params
    }

    /// The first matching rule for `ctx`.
    pub fn infer(&self, ctx: &CallContext) -> Inference {
        todo!("phase 3")
    }

    /// Candidate calls with their natural constraints and priorities, for `choose_bid` when the
    /// auction is off-system.
    pub fn candidates(&self, auction: &Auction, owner: Seat) -> Vec<(Call, HandConstraint, i16)> {
        todo!("phase 3")
    }
}

impl Default for NaturalInference {
    fn default() -> NaturalInference {
        NaturalInference::new(NaturalParams::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `NaturalParams::default()` matches the SAYC-like values in `docs/design/06-system.md`
    /// §8.1.
    #[test]
    fn natural_params_default_is_sayc() {
        let params = NaturalParams::default();
        assert_eq!(params.opening_hcp, 12..=21);
        assert_eq!(params.open_1m_len, 3);
        assert_eq!(params.open_1major_len, 5);
        assert_eq!(params.nt, vec![(1, 15..=17), (2, 20..=21), (3, 25..=27)]);
        assert_eq!(params.weak_two, (6, 5..=10));
        assert_eq!(params.strong_two_c, 22);
        assert_eq!(params.takeout_double, (12, 2, 3));
        assert_eq!(params.balancing_shift, -3);
        assert!(params.implicit_raise_support);

        assert_eq!(params.response.new_suit_1, (4, 6));
        assert_eq!(params.response.jump_shift, 17);
        assert_eq!(params.rebid.reverse, 17);
        assert_eq!(params.advance.cue, 10);
    }
}
