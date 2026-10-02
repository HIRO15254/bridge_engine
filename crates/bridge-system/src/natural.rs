//! Natural inference: a generic meaning for any call, used when the system has no entry.
//!
//! The rules are a small ordered table over a [`CallContext`] (role, call kind, level, …) and
//! produce constraints from [`NaturalParams`]. [`classify`] is system-independent and testable
//! on its own. Accuracy is measured three ways (hold-out against compiled systems, reproduction
//! rate, corpus satisfaction); see `docs/design/06-system.md`.

use core::ops::RangeInclusive;

use bridge_constraint::{Atom, CardRequirement, HandConstraint};
use bridge_core::{Auction, Bid, Call, Holding, Rank, Seat, ShapeSet, Strain, Suit};

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
    /// Level-aware strength floor for natural continuation bids (see [`LevelFloor`]). Not
    /// serialised (the serialised IR bytes and `IR_FORMAT` do not depend on it): a deserialised
    /// IR carries `LevelFloor::default()`.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub level_floor: LevelFloor,
}

/// Combined HCP a partnership needs before a natural *continuation* bid at a given level is a
/// contract rather than a runaway escalation (docs/design/06-system.md §8, "level floor").
///
/// When [`NaturalInference::infer`] reads a natural bid at level `L` in strain `s` by a player
/// who, or whose partner, has already acted, and `CallContext::partner_constraint` is known, it
/// adds `own HCP >= combined(L, s) - min HCP(partner_constraint)` to the inferred constraint. An
/// opening, a first action with partner silent, and a context without a partner constraint get
/// no floor. A `0` entry means "no floor at this level".
///
/// The default is [`LevelFloor::STANDARD`] (prototype C's table, kept after the phase-4.6
/// tuning on the corpus tune split); [`LevelFloor::NONE`] restores the phase-3 behaviour. The
/// field is not serialised, so a deserialised IR gets the default table too.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LevelFloor {
    /// Combined HCP for a suit bid at levels 1..=7 (index `level - 1`).
    pub suit: [u8; 7],
    /// Combined HCP for a notrump bid at levels 1..=7 (index `level - 1`).
    pub nt: [u8; 7],
}

impl Default for LevelFloor {
    /// [`LevelFloor::STANDARD`].
    fn default() -> LevelFloor {
        LevelFloor::STANDARD
    }
}

impl LevelFloor {
    /// No floor at any level (the phase-3 behaviour).
    pub const NONE: LevelFloor = LevelFloor {
        suit: [0; 7],
        nt: [0; 7],
    };

    /// The default table: levels 1-2 need nothing extra; a suit continuation at level 3 needs
    /// 18 combined HCP (a minimum opening opposite a minimum response), 4 → 22, 5 → 26,
    /// 6 → 31, 7 → 35; notrump 3 → 24, 4 → 28, 5 → 30, 6 → 32, 7 → 36.
    pub const STANDARD: LevelFloor = LevelFloor {
        suit: [0, 0, 18, 22, 26, 31, 35],
        nt: [0, 0, 24, 28, 30, 32, 36],
    };

    /// The combined HCP required for a bid at `level` (`1..=7`) in a suit (`nt == false`) or
    /// notrump; `0` (no floor) for an out-of-range level.
    pub fn combined(&self, level: u8, nt: bool) -> u8 {
        let table = if nt { &self.nt } else { &self.suit };
        match level {
            1..=7 => table[level as usize - 1],
            _ => 0,
        }
    }
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
            level_floor: LevelFloor::default(),
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

/// The kinds of a one-level suit opener's non-pass calls after its opening, other than its bids
/// of the opened strain, each as the natural rule that reads it ([`CallContext::opener_other_calls`]).
/// All `false` ([`OpenerCalls::is_empty`]) while the opener has only opened and bid the opened
/// strain again (passes aside).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct OpenerCalls {
    /// A raise of partner's suit (rule `raise`: `rebid.raise`).
    pub raise: bool,
    /// A jump raise of partner's suit (rule `raise`: `rebid.jump_raise`).
    pub jump_raise: bool,
    /// A jump rebid of another suit opener bid before (rule `rebid_own`: `rebid.jump_rebid`).
    pub jump_rebid: bool,
    /// A reverse (rule `reverse`: `rebid.reverse`+).
    pub reverse: bool,
    /// The cheapest notrump at opener's rebid (rule `rebid_nt`: `rebid.nt_1`).
    pub nt_rebid: bool,
    /// A jump in notrump at opener's rebid (rule `rebid_nt`: `rebid.nt_2`).
    pub jump_nt_rebid: bool,
    /// A new suit that is not a reverse (rule `rebid_new_suit`).
    pub new_suit: bool,
    /// A jump shift (rule `rebid_new_suit` with a jump).
    pub jump_shift: bool,
    /// Any other call (a double, a redouble, a cue bid, a non-jump rebid of another suit, a
    /// notrump bid after the rebid): no range beyond the opening's.
    pub other: bool,
}

impl OpenerCalls {
    /// `true` when there is no such call.
    pub fn is_empty(&self) -> bool {
        *self == OpenerCalls::default()
    }
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
    /// The caller passed at an earlier turn after the auction's opening bid (the first non-pass
    /// call): East after `1H-P-2H-P-4H`, not North after `P-1H-P-4H` (a pass before it).
    pub passed_after_opening: bool,
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
    /// The suit and level of `owner`'s own first [`Call::Bid`] so far, if any. Unlike
    /// `our_suits` (side-level: opener's and responder's suits combined), this identifies
    /// specifically the suit `owner` opened, needed to check a reverse's first-suit length
    /// exactly rather than against the whole side's suits. Notrump bids are skipped: this is
    /// only ever a *suit*, so after e.g. `1NT-P-2D-P-2H` it holds `(Hearts, 2)`, the transfer
    /// completion, not the 1NT opening -- see `opener_first_bid` for the latter.
    pub opener_first_suit: Option<(Suit, u8)>,
    /// `owner`'s own first [`Call::Bid`] so far, whatever the strain (including notrump),
    /// unlike `opener_first_suit` which skips notrump. Needed by rules (`rebid_own`) that must
    /// know the level/strength of the *opening bid itself*, since after a notrump opening or a
    /// strong 2C, `opener_first_suit` instead names a later suit bid (a transfer completion, or
    /// nothing at all for 2C).
    pub opener_first_bid: Option<Bid>,
    /// `owner` has already made a non-pass call (a bid, double or redouble) earlier in the
    /// auction. Rules that describe a player's *first* action (an overcall, the limited pass of
    /// partner's opening) only apply while this is `false`.
    pub owner_acted: bool,
    /// How many non-pass calls partner has made so far.
    pub partner_actions: u8,
    /// Partner's first non-pass call, if any.
    pub partner_first_action: Option<Call>,
    /// Levels skipped by `partner_first_action` when it is a bid (0 otherwise), measured like
    /// [`CallKind::Bid::jump`] against the last bid before it.
    pub partner_first_jump: u8,
    /// `owner`'s own first non-pass call, if any.
    pub owner_first_action: Option<Call>,
    /// Levels skipped by `owner_first_action` when it is a bid (0 otherwise), like
    /// `partner_first_jump`.
    pub owner_first_jump: u8,
    /// `owner`'s only non-pass call so far is a bid made right after partner's double (partner's
    /// last call before it): the answer to partner's takeout double, `1H-X-P-1S`.
    pub owner_answered_partners_double: bool,
    /// When `owner`'s first non-pass call was a negative double (the double rule `negative_x`
    /// reads: responder's first turn over the overcall of partner's opening, `1C (1H) X`), the
    /// level of the bid it doubled. `None` otherwise.
    pub owner_first_negative_double: Option<u8>,
    /// How many bids (not passes, doubles or redoubles) the opponents have made so far. Two or
    /// more on a first entry means the opponents have exchanged bids (opener and responder).
    pub their_bids: u8,
    /// The right-hand opponent's call just before this one, if any.
    pub rho_last: Option<Call>,
    /// `owner`'s own last call so far (a pass included), if any.
    pub owner_last: Option<Call>,
    /// Strains `owner` has bid (as a [`Call::Bid`]) two or more times so far: `S` after
    /// `2S-P-2NT-P-3S`, nothing after `1H-P-3NT`.
    pub owner_repeated_strains: StrainSet,
    /// The strains of [`CallContext::owner_repeated_strains`] that `owner` bid again with a
    /// jump (measured like [`CallKind::Bid::jump`]): `S` after `1S-P-1NT-P-3S`, nothing after
    /// `1S-P-2C-P-2S`.
    pub owner_jump_rebid_strains: StrainSet,
    /// When `owner` opened one of a suit: its non-pass calls since, other than bids of the opened
    /// strain ([`OpenerCalls`]): a reverse after `1D-P-1S-P-2H`, nothing after `1S-P-2C-P-2S`.
    /// Empty for any other caller.
    pub opener_other_calls: OpenerCalls,
    /// `owner` or partner has bid 4NT or 5NT earlier in the auction (a Blackwood-style ask or a
    /// quantitative 4NT): later bids past game answer or follow up the ask.
    pub our_slam_ask: bool,
}

/// Classifies call `index` of `auction` from the point of view of its caller.
///
/// Only `auction.calls()[..index]` (the history strictly before this call) and the call at
/// `index` itself are read; anything at or after `index + 1` (a hypothetical continuation
/// appended by a caller such as [`NaturalInference::candidates`]) is ignored, so `classify` is
/// stable under `auction.with(candidate_call)`.
pub fn classify(auction: &Auction, index: usize, owner: Seat) -> CallContext {
    let history = HistoryContext::new(auction, index, owner);
    history.classify_call(auction, auction.calls()[index])
}

/// Everything [`classify`] derives from the history `auction.calls()[..index]` and `owner`
/// alone, computed once and shared by every candidate call at that position
/// ([`NaturalInference::infer_batch`]).
struct HistoryContext {
    index: usize,
    owner: Seat,
    our_suits: StrainSet,
    their_suits: StrainSet,
    owner_suits: StrainSet,
    partner_suits: StrainSet,
    role: Role,
    opener_first_suit: Option<(Suit, u8)>,
    opener_first_bid: Option<Bid>,
    position: u8,
    passed_hand: bool,
    passed_after_opening: bool,
    vul: (bool, bool),
    competitive: bool,
    partner_last: Option<Call>,
    agreed_suit: Option<Suit>,
    last_bid: Option<Bid>,
    owner_acted: bool,
    partner_actions: u8,
    partner_first: Option<(usize, Call)>,
    partner_first_jump: u8,
    owner_first: Option<(usize, Call)>,
    owner_first_jump: u8,
    owner_answered_partners_double: bool,
    owner_first_negative_double: Option<u8>,
    their_bids: u8,
    rho_last: Option<Call>,
    owner_last: Option<Call>,
    owner_repeated_strains: StrainSet,
    owner_jump_rebid_strains: StrainSet,
    opener_other_calls: OpenerCalls,
    our_slam_ask: bool,
}

impl HistoryContext {
    /// Reads only `auction.calls()[..index]` (and the dealer/vulnerability), so `auction` may be
    /// the prefix itself (`index == auction.len()`) or any continuation of it.
    fn new(auction: &Auction, index: usize, owner: Seat) -> HistoryContext {
        let history = &auction.calls()[..index];

        let our_suits = suits_bid_by(auction, history, |s| s.side() == owner.side());
        let their_suits = suits_bid_by(auction, history, |s| s.side() != owner.side());
        let owner_suits = suits_bid_by(auction, history, |s| s == owner);
        let partner_suits = suits_bid_by(auction, history, |s| s == owner.partner());

        let role = classify_role(auction, history, index, owner);
        let opener_first_suit = owner_first_suit(auction, history, owner);
        let opener_first_bid = owner_first_bid(auction, history, owner);

        let position = auction.position_of(owner);
        let passed_hand = history
            .iter()
            .enumerate()
            .any(|(i, c)| auction.seat_at(i) == owner && *c == Call::Pass);
        let passed_after_opening =
            history
                .iter()
                .position(|c| *c != Call::Pass)
                .is_some_and(|opening| {
                    history
                        .iter()
                        .enumerate()
                        .skip(opening + 1)
                        .any(|(i, c)| auction.seat_at(i) == owner && *c == Call::Pass)
                });
        let vul = (
            auction.vulnerability().is_vulnerable_side(owner.side()),
            auction
                .vulnerability()
                .is_vulnerable_side(owner.side().other()),
        );
        let competitive = our_suits.0 != 0 && their_suits.0 != 0;
        let partner_last = history
            .iter()
            .enumerate()
            .rev()
            .find(|(i, _)| auction.seat_at(*i) == owner.partner())
            .map(|(_, c)| *c);
        let owner_last = history
            .iter()
            .enumerate()
            .rev()
            .find(|(i, _)| auction.seat_at(*i) == owner)
            .map(|(_, c)| *c);
        let agreed_suit = Suit::ALL.into_iter().find(|&s| {
            let strain = Strain::from_suit(s);
            owner_suits.contains(strain) && partner_suits.contains(strain)
        });
        let last_bid = history.iter().rev().find_map(|c| c.bid());
        let owner_acted = history
            .iter()
            .enumerate()
            .any(|(i, c)| auction.seat_at(i) == owner && *c != Call::Pass);
        let mut partner_actions = 0u8;
        let mut partner_first: Option<(usize, Call)> = None;
        let mut owner_first: Option<(usize, Call)> = None;
        for (i, c) in history.iter().enumerate() {
            if *c == Call::Pass {
                continue;
            }
            if auction.seat_at(i) == owner.partner() {
                partner_actions = partner_actions.saturating_add(1);
                if partner_first.is_none() {
                    partner_first = Some((i, *c));
                }
            } else if auction.seat_at(i) == owner && owner_first.is_none() {
                owner_first = Some((i, *c));
            }
        }
        let their_bids = history
            .iter()
            .enumerate()
            .filter(|&(i, c)| auction.seat_at(i).side() != owner.side() && c.is_bid())
            .count()
            .min(u8::MAX as usize) as u8;
        let mut owner_strains = StrainSet::EMPTY;
        let mut owner_repeated_strains = StrainSet::EMPTY;
        let mut owner_jump_rebid_strains = StrainSet::EMPTY;
        let mut our_slam_ask = false;
        let mut previous_bid: Option<Bid> = None;
        for (i, c) in history.iter().enumerate() {
            let seat = auction.seat_at(i);
            let Call::Bid(b) = c else {
                continue;
            };
            if seat == owner {
                if owner_strains.contains(b.strain()) {
                    owner_repeated_strains = owner_repeated_strains.with(b.strain());
                    if b.level() > minimal_level(previous_bid, b.strain()) {
                        owner_jump_rebid_strains = owner_jump_rebid_strains.with(b.strain());
                    }
                }
                owner_strains = owner_strains.with(b.strain());
            }
            if seat.side() == owner.side()
                && b.strain() == Strain::NoTrump
                && matches!(b.level(), 4 | 5)
            {
                our_slam_ask = true;
            }
            previous_bid = Some(*b);
        }
        let first_jump = |first: Option<(usize, Call)>| match first {
            Some((i, Call::Bid(b))) => {
                let before = history[..i].iter().rev().find_map(|c| c.bid());
                b.level().saturating_sub(minimal_level(before, b.strain()))
            }
            _ => 0,
        };
        let partner_first_jump = first_jump(partner_first);
        let owner_first_jump = first_jump(owner_first);
        let owner_actions = history
            .iter()
            .enumerate()
            .filter(|&(i, c)| auction.seat_at(i) == owner && *c != Call::Pass)
            .count();
        let owner_answered_partners_double = owner_actions == 1
            && matches!(owner_first, Some((i, Call::Bid(_))) if i >= 2 && history[i - 2] == Call::Double);
        let owner_first_negative_double = match owner_first {
            Some((i, Call::Double)) if role == Role::Responder => {
                // `owner` has made no non-pass call before `i`, so this nests only once.
                let first =
                    HistoryContext::new(auction, i, owner).classify_call(auction, Call::Double);
                (first.kind == CallKind::Double(DoubleKind::Negative)
                    && responders_first_turn_over_overcall(&first))
                .then(|| first.last_bid.map_or(1, |b| b.level()))
            }
            _ => None,
        };
        let opener_other_calls = match opener_first_bid {
            Some(opening) if role == Role::Opener => {
                opener_other_calls(auction, history, owner, opening)
            }
            _ => OpenerCalls::default(),
        };

        HistoryContext {
            index,
            owner,
            our_suits,
            their_suits,
            owner_suits,
            partner_suits,
            role,
            opener_first_suit,
            opener_first_bid,
            position,
            passed_hand,
            passed_after_opening,
            vul,
            competitive,
            partner_last,
            agreed_suit,
            last_bid,
            owner_acted,
            partner_actions,
            partner_first,
            partner_first_jump,
            owner_first,
            owner_first_jump,
            owner_answered_partners_double,
            owner_first_negative_double,
            their_bids,
            rho_last: history.last().copied(),
            owner_last,
            owner_repeated_strains,
            owner_jump_rebid_strains,
            opener_other_calls,
            our_slam_ask,
        }
    }

    /// Re-targets `ctx` (built by [`HistoryContext::classify_call`] from `self`) at `call`: only
    /// the call-dependent fields change, the partner context set by the caller is kept.
    fn set_call(&self, ctx: &mut CallContext, auction: &Auction, call: Call) {
        ctx.kind = self.kind_of(auction, call);
        ctx.call = call;
        ctx.level = match call {
            Call::Bid(b) => b.level(),
            _ => 0,
        };
    }

    fn kind_of(&self, auction: &Auction, call: Call) -> CallKind {
        classify_kind(
            auction,
            &auction.calls()[..self.index],
            self.owner,
            call,
            self.role,
            self.our_suits,
            self.their_suits,
            self.owner_suits,
            self.partner_suits,
            self.opener_first_suit,
        )
    }

    /// The [`CallContext`] of `call` as the call at `self.index` (`auction` as in
    /// [`HistoryContext::new`]; `call` need not be in it).
    fn classify_call(&self, auction: &Auction, call: Call) -> CallContext {
        let kind = self.kind_of(auction, call);
        let level = match call {
            Call::Bid(b) => b.level(),
            _ => 0,
        };
        CallContext {
            role: self.role,
            kind,
            call,
            level,
            position: self.position,
            passed_hand: self.passed_hand,
            passed_after_opening: self.passed_after_opening,
            vul: self.vul,
            competitive: self.competitive,
            partner_last: self.partner_last,
            partner_constraint: None,
            our_suits: self.our_suits,
            their_suits: self.their_suits,
            agreed_suit: self.agreed_suit,
            last_bid: self.last_bid,
            forcing_situation: false,
            opener_first_suit: self.opener_first_suit,
            opener_first_bid: self.opener_first_bid,
            owner_acted: self.owner_acted,
            partner_actions: self.partner_actions,
            partner_first_action: self.partner_first.map(|(_, c)| c),
            partner_first_jump: self.partner_first_jump,
            owner_first_action: self.owner_first.map(|(_, c)| c),
            owner_first_jump: self.owner_first_jump,
            owner_answered_partners_double: self.owner_answered_partners_double,
            owner_first_negative_double: self.owner_first_negative_double,
            their_bids: self.their_bids,
            rho_last: self.rho_last,
            owner_last: self.owner_last,
            owner_repeated_strains: self.owner_repeated_strains,
            owner_jump_rebid_strains: self.owner_jump_rebid_strains,
            opener_other_calls: self.opener_other_calls,
            our_slam_ask: self.our_slam_ask,
        }
    }
}

/// Strains bid (as [`Call::Bid`]) in `history` by a seat matching `filter`.
fn suits_bid_by(auction: &Auction, history: &[Call], filter: impl Fn(Seat) -> bool) -> StrainSet {
    let mut set = StrainSet::EMPTY;
    for (i, call) in history.iter().enumerate() {
        if filter(auction.seat_at(i)) {
            if let Call::Bid(b) = call {
                set = set.with(b.strain());
            }
        }
    }
    set
}

/// `true` when `index` is the classic balancing seat: an opponent's non-pass call, then two
/// passes (partner's and the other opponent's), and now it is `owner`'s turn.
fn is_balancing_seat(auction: &Auction, index: usize, owner: Seat) -> bool {
    index >= 3
        && auction.calls()[index - 1] == Call::Pass
        && auction.calls()[index - 2] == Call::Pass
        && auction.calls()[index - 3] != Call::Pass
        && auction.seat_at(index - 3).side() != owner.side()
}

/// Determines [`Role`] from the auction history (§8.2.1 of `06-system.md`).
fn classify_role(auction: &Auction, history: &[Call], index: usize, owner: Seat) -> Role {
    match history.iter().position(|c| *c != Call::Pass) {
        // Nobody has bid yet: every seat is still a candidate opener.
        None => Role::Opener,
        Some(open_idx) => {
            let opening_seat = auction.seat_at(open_idx);
            if owner.side() == opening_seat.side() {
                if owner == opening_seat {
                    Role::Opener
                } else {
                    Role::Responder
                }
            } else if is_balancing_seat(auction, index, owner) {
                Role::Balancer
            } else {
                let entry_seat = history[(open_idx + 1)..index]
                    .iter()
                    .enumerate()
                    .map(|(rel, c)| (auction.seat_at(open_idx + 1 + rel), c))
                    .find(|(seat, c)| seat.side() == owner.side() && **c != Call::Pass)
                    .map(|(seat, _)| seat);
                match entry_seat {
                    None => Role::Overcaller,
                    Some(seat) if seat == owner => Role::Overcaller,
                    Some(_) => Role::Advancer,
                }
            }
        }
    }
}

/// The suit and level of `owner`'s first [`Call::Bid`] in `history`, if any.
fn owner_first_suit(auction: &Auction, history: &[Call], owner: Seat) -> Option<(Suit, u8)> {
    history
        .iter()
        .enumerate()
        .filter(|(i, _)| auction.seat_at(*i) == owner)
        .find_map(|(_, c)| match c {
            Call::Bid(b) => b.strain().suit().map(|s| (s, b.level())),
            _ => None,
        })
}

/// `owner`'s first [`Call::Bid`] in `history`, whatever the strain -- unlike `owner_first_suit`,
/// a notrump opening is not skipped. Needed to recover the level/strain of the actual opening
/// bid when `owner_first_suit` instead names a later suit (a transfer completion after 1NT/2NT,
/// or nothing at all after a strong 2C, since 2C itself is recorded by `owner_first_suit` too --
/// callers that need to tell a 2C opening apart from a weak two must match on `owner_first_bid`'s
/// strain, not merely its level).
fn owner_first_bid(auction: &Auction, history: &[Call], owner: Seat) -> Option<Bid> {
    history
        .iter()
        .enumerate()
        .filter(|(i, _)| auction.seat_at(*i) == owner)
        .find_map(|(_, c)| c.bid())
}

/// Index in `history` of `seat`'s first [`Call::Bid`] of `strain`, if any.
fn first_bid_of(auction: &Auction, history: &[Call], seat: Seat, strain: Strain) -> Option<usize> {
    history.iter().enumerate().position(|(i, c)| {
        auction.seat_at(i) == seat && matches!(c, Call::Bid(b) if b.strain() == strain)
    })
}

/// [`CallContext::opener_other_calls`]: `owner` opened with `opening` (its first bid); when that
/// is one of a suit, each later non-pass call of `owner` in `history` that is not a bid of the
/// opened strain, classified as the natural rules read it (the same [`classify_kind`] and the
/// same rule order: `raise`, `rebid_own`, `reverse`, `rebid_nt`, `rebid_new_suit`).
fn opener_other_calls(
    auction: &Auction,
    history: &[Call],
    owner: Seat,
    opening: Bid,
) -> OpenerCalls {
    let mut out = OpenerCalls::default();
    let Some(opening_suit) = opening.strain().suit().filter(|_| opening.level() == 1) else {
        return out;
    };
    let mut seen_opening = false;
    let mut previous: Option<Call> = None;
    for (j, &call) in history.iter().enumerate() {
        if auction.seat_at(j) != owner {
            continue;
        }
        let before = previous.replace(call);
        if !seen_opening {
            seen_opening = call == Call::Bid(opening);
            continue;
        }
        if call == Call::Pass || matches!(call, Call::Bid(b) if b.strain() == opening.strain()) {
            continue;
        }
        let prefix = &history[..j];
        let owner_suits = suits_bid_by(auction, prefix, |s| s == owner);
        let partner_suits = suits_bid_by(auction, prefix, |s| s == owner.partner());
        let their_suits = suits_bid_by(auction, prefix, |s| s.side() != owner.side());
        let our_suits = StrainSet(owner_suits.0 | partner_suits.0);
        let kind = classify_kind(
            auction,
            prefix,
            owner,
            call,
            Role::Opener,
            our_suits,
            their_suits,
            owner_suits,
            partner_suits,
            Some((opening_suit, 1)),
        );
        match kind {
            CallKind::Bid {
                raise: true, jump, ..
            } => {
                if jump >= 1 {
                    out.jump_raise = true;
                } else {
                    out.raise = true;
                }
            }
            CallKind::Bid {
                rebid_own: true,
                jump,
                ..
            } => {
                if jump >= 1 {
                    out.jump_rebid = true;
                } else {
                    out.other = true;
                }
            }
            CallKind::Bid { reverse: true, .. } => out.reverse = true,
            CallKind::Bid { nt: true, jump, .. } => {
                // `rule_rebid_nt`: opener's rebid proper (`is_openers_rebid`), at most the two
                // level, no agreed suit.
                let partner_acted = prefix
                    .iter()
                    .enumerate()
                    .any(|(i, c)| auction.seat_at(i) == owner.partner() && *c != Call::Pass);
                let agreed = Suit::ALL.into_iter().any(|s| {
                    let strain = Strain::from_suit(s);
                    owner_suits.contains(strain) && partner_suits.contains(strain)
                });
                let rebid = before == Some(Call::Bid(opening))
                    && partner_acted
                    && !their_suits.contains(Strain::NoTrump)
                    && !agreed
                    && call.bid().is_some_and(|b| b.level() <= 2);
                match (rebid, jump) {
                    (true, 0) => out.nt_rebid = true,
                    (true, 1) => out.jump_nt_rebid = true,
                    _ => out.other = true,
                }
            }
            CallKind::Bid {
                new_suit: true,
                jump,
                ..
            } => {
                if jump >= 1 {
                    out.jump_shift = true;
                } else {
                    out.new_suit = true;
                }
            }
            _ => out.other = true,
        }
    }
    out
}

/// The lowest level at which `strain` may legally be bid over `last_bid`.
fn minimal_level(last_bid: Option<Bid>, strain: Strain) -> u8 {
    match last_bid {
        None => 1,
        Some(b) if strain.index() > b.strain().index() => b.level(),
        Some(b) => b.level() + 1,
    }
}

/// Determines [`CallKind`] from the call and the strains already bid by each side (§8.2.2).
#[allow(clippy::too_many_arguments)]
fn classify_kind(
    auction: &Auction,
    history: &[Call],
    owner: Seat,
    call: Call,
    role: Role,
    our_suits: StrainSet,
    their_suits: StrainSet,
    owner_suits: StrainSet,
    partner_suits: StrainSet,
    opener_first_suit: Option<(Suit, u8)>,
) -> CallKind {
    match call {
        Call::Pass => CallKind::Pass,
        Call::Redouble => CallKind::Redouble,
        Call::Double => CallKind::Double(classify_double(
            auction,
            history,
            owner,
            role,
            our_suits,
            their_suits,
            owner_suits,
            partner_suits,
        )),
        Call::Bid(b) => {
            let strain = b.strain();
            let nt = strain == Strain::NoTrump;
            let new_suit = !nt && !our_suits.contains(strain) && !their_suits.contains(strain);
            let cue = !nt && their_suits.contains(strain);
            // A raise is a bid of a strain *partner* introduced: when `owner` bid it first and
            // partner only supported it (`1H-P-2H-P-3H`), bidding it again is `rebid_own` of an
            // agreed suit, not a raise of partner's suit.
            let raise = !nt
                && match (
                    first_bid_of(auction, history, owner.partner(), strain),
                    first_bid_of(auction, history, owner, strain),
                ) {
                    (Some(p), Some(o)) => p < o,
                    (Some(_), None) => true,
                    (None, _) => false,
                };
            let rebid_own = !nt && owner_suits.contains(strain);
            let last_bid = history.iter().rev().find_map(|c| c.bid());
            let jump = b.level().saturating_sub(minimal_level(last_bid, strain));

            // A reverse is specifically opener's rebid: a new suit at level 2 that outranks the
            // suit opener opened at level 1 (§8.2.2, "オープナーが1レベルで開いたスートより
            // 上位の新スートを2レベルで").
            let reverse = role == Role::Opener
                && b.level() == 2
                && new_suit
                && opener_first_suit.is_some_and(|(first_suit, first_level)| {
                    first_level == 1 && strain.index() > Strain::from_suit(first_suit).index()
                });

            CallKind::Bid {
                new_suit,
                raise,
                nt,
                jump,
                cue,
                reverse,
                rebid_own,
            }
        }
    }
}

/// `true` when `history[..before]` contains a [`Call::Bid`] of `strain` by `side`.
fn side_bid_strain_before(
    auction: &Auction,
    history: &[Call],
    side: bridge_core::Side,
    strain: Strain,
    before: usize,
) -> bool {
    history[..before].iter().enumerate().any(|(i, c)| {
        auction.seat_at(i).side() == side && matches!(c, Call::Bid(b) if b.strain() == strain)
    })
}

/// Determines [`DoubleKind`] (§8.2.2). Order matters: earlier conditions take precedence.
#[allow(clippy::too_many_arguments)]
fn classify_double(
    auction: &Auction,
    history: &[Call],
    owner: Seat,
    role: Role,
    our_suits: StrainSet,
    their_suits: StrainSet,
    owner_suits: StrainSet,
    partner_suits: StrainSet,
) -> DoubleKind {
    let Some(li) = history.iter().rposition(|c| c.is_bid()) else {
        return DoubleKind::Unknown;
    };
    let Call::Bid(target) = history[li] else {
        unreachable!("rposition matched Call::is_bid")
    };
    if auction.seat_at(li).side() == owner.side() {
        // Doubling our own side's bid is illegal; guard defensively.
        return DoubleKind::Unknown;
    }

    let low_level_suit = target.strain() != Strain::NoTrump && target.level() <= 2;
    // §8.2: "we have already agreed a suit" -- a suit bid by both `owner` and partner (the same
    // test as `CallContext::agreed_suit`), not merely "we have bid the suit they are bidding".
    let agreed_suit = Suit::ALL.into_iter().any(|s| {
        let strain = Strain::from_suit(s);
        owner_suits.contains(strain) && partner_suits.contains(strain)
    });
    let penalty_cond = target.strain() == Strain::NoTrump || target.level() >= 4 || agreed_suit;
    // Partner's most recent bid was notrump (typically the 1NT opening): SAYC doubles of an
    // overcall of it are for penalty (§8.3 `penalty_x`).
    let partner_last_bid_nt = history
        .iter()
        .enumerate()
        .rev()
        .find(|(i, c)| auction.seat_at(*i) == owner.partner() && c.is_bid())
        .is_some_and(|(_, c)| matches!(c, Call::Bid(b) if b.strain() == Strain::NoTrump));

    let partner_last = history
        .iter()
        .enumerate()
        .rev()
        .find(|(i, _)| auction.seat_at(*i) == owner.partner())
        .map(|(_, c)| *c);

    if low_level_suit && partner_suits.0 == 0 && partner_last != Some(Call::Double) {
        // §8.2: "partner has not bid yet" -- `owner`'s own earlier bids (an overcall followed by
        // a reopening double) do not stop the double from being takeout. The
        // `partner_last != Double` guard keeps a double right after partner's own takeout double
        // (no suit bid by partner yet either) from being read as a second takeout double instead
        // of responsive (checked below).
        DoubleKind::Takeout
    } else if partner_last_bid_nt {
        DoubleKind::Penalty
    } else if role == Role::Responder && low_level_suit {
        DoubleKind::Negative
    } else if penalty_cond {
        DoubleKind::Penalty
    } else if partner_last == Some(Call::Double)
        && side_bid_strain_before(auction, history, owner.side().other(), target.strain(), li)
    {
        DoubleKind::Responsive
    } else if role == Role::Opener
        && low_level_suit
        && their_suits == StrainSet::EMPTY.with(target.strain())
        && our_suits.iter().count() >= 2
        && !our_suits.contains(target.strain())
    {
        DoubleKind::Support
    } else {
        DoubleKind::Unknown
    }
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

/// Context of a natural call that [`classify`] cannot derive from the auction alone: what the
/// caller's interpretation of partner's calls says (docs/design/07-bidding.md §2.2). The default
/// is "nothing known" (`None`, `false`), which is what a bare `classify` gives.
#[derive(Clone, Debug, Default)]
pub struct PartnerContext {
    /// Fills [`CallContext::partner_constraint`].
    pub partner_constraint: Option<HandConstraint>,
    /// Fills [`CallContext::forcing_situation`].
    pub forcing_situation: bool,
}

/// One natural reading of a candidate call, without the explanation string (see
/// [`NaturalInference::infer_batch`]).
#[derive(Clone, Debug)]
pub struct NaturalCandidate {
    /// The call.
    pub call: Call,
    /// The inferred constraint.
    pub constraint: HandConstraint,
    /// Confidence in `0..=1`.
    pub confidence: f32,
    /// The rule that fired (`"fallback"` when none did).
    pub rule: &'static str,
}

impl NaturalCandidate {
    /// The rank priority `round(confidence·100)` (what `choose_bid` sorts natural candidates by).
    pub fn priority(&self) -> i16 {
        (self.confidence * 100.0).round() as i16
    }

    /// `true` when no natural rule fired (the call is not a natural candidate).
    pub fn is_fallback(&self) -> bool {
        self.rule == "fallback"
    }
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

    /// The first matching rule for `ctx` (§8.3's ordered table).
    pub fn infer(&self, ctx: &CallContext) -> Inference {
        self.infer_inner(ctx, true)
    }

    /// [`NaturalInference::infer`], building the explanation string only when `ex`.
    fn infer_inner(&self, ctx: &CallContext, ex: bool) -> Inference {
        let p = &self.params;
        rule_open_1major(p, ctx, ex)
            .or_else(|| rule_open_1m(p, ctx, ex))
            .or_else(|| rule_open_nt(p, ctx, ex))
            .or_else(|| rule_open_weak2(p, ctx, ex))
            .or_else(|| rule_open_2c(p, ctx, ex))
            .or_else(|| rule_open_preempt(p, ctx, ex))
            .or_else(|| rule_open_pass(p, ctx, ex))
            .or_else(|| rule_overcall(p, ctx, ex))
            .or_else(|| rule_jump_overcall(p, ctx, ex))
            .or_else(|| rule_nt_overcall(p, ctx, ex))
            .or_else(|| rule_takeout_x(p, ctx, ex))
            .or_else(|| rule_penalty_x(p, ctx, ex))
            .or_else(|| rule_negative_x(p, ctx, ex))
            .or_else(|| rule_competitive_x(p, ctx, ex))
            .or_else(|| rule_raise(p, ctx, ex))
            .or_else(|| rule_new_suit_resp_1(p, ctx, ex))
            .or_else(|| rule_new_suit_resp_2(p, ctx, ex))
            .or_else(|| rule_resp_nt(p, ctx, ex))
            .or_else(|| rule_rebid_own(p, ctx, ex))
            .or_else(|| rule_reverse(p, ctx, ex))
            .or_else(|| rule_rebid_nt(p, ctx, ex))
            .or_else(|| rule_rebid_new_suit(p, ctx, ex))
            .or_else(|| rule_advance_new_suit(p, ctx, ex))
            .or_else(|| rule_cue(p, ctx, ex))
            .or_else(|| rule_pass_forcing(ctx, ex))
            .or_else(|| rule_pass_default(p, ctx, ex))
            .map(|inf| apply_level_floor(&p.level_floor, ctx, inf, ex))
            .unwrap_or_else(rule_fallback)
    }

    /// [`NaturalInference::infer`] for every call of `calls` as the next call after `auction`
    /// by `owner`, with `partner`'s context filled in: one [`NaturalCandidate`] per input call,
    /// in input order. The constraint, confidence and rule of each result are identical to
    /// `infer` on `classify(&auction.with(call), auction.len(), owner)` with
    /// `partner_constraint`/`forcing_situation` taken from `partner`. A call that is not legal
    /// after `auction` yields the `fallback` result (`ANY`, rule `"fallback"`).
    ///
    /// The history-dependent part of `classify` (roles, suits bid, partner's actions, …) is
    /// computed once for all calls, the partner context is set once (not cloned per call), no
    /// continuation auction is built, and no explanation string is formatted.
    pub fn infer_batch(
        &self,
        auction: &Auction,
        owner: Seat,
        partner: &PartnerContext,
        calls: &[Call],
    ) -> Vec<NaturalCandidate> {
        let history = HistoryContext::new(auction, auction.len(), owner);
        let mut ctx = history.classify_call(auction, Call::Pass);
        ctx.partner_constraint = partner.partner_constraint.clone();
        ctx.forcing_situation = partner.forcing_situation;
        calls
            .iter()
            .map(|&call| {
                if !auction.is_legal(call) {
                    return NaturalCandidate {
                        call,
                        constraint: HandConstraint::ANY,
                        confidence: FALLBACK_CONFIDENCE,
                        rule: FALLBACK_RULE,
                    };
                }
                history.set_call(&mut ctx, auction, call);
                let inf = self.infer_inner(&ctx, false);
                NaturalCandidate {
                    call,
                    constraint: inf.constraint,
                    confidence: inf.confidence,
                    rule: inf.rule,
                }
            })
            .collect()
    }

    /// The natural candidates for `owner`'s next call after `auction` (every legal call whose
    /// inference is not the `fallback` rule), sorted in the natural rank order
    /// [`crate::exclusive::natural_rank_cmp`]: `round(confidence·100)` descending, then
    /// `tie_break` when it is `LowestCall`/`HighestCall`, then call index ascending. The natural
    /// policy picks the first candidate the hand satisfies; `choose_bid`'s natural branch and the
    /// natural exclusion of `interpret` both use this order. `partner` supplies the context that
    /// `classify` cannot derive from the auction alone (see [`NaturalInference::infer_batch`]).
    pub fn ranked_candidates(
        &self,
        auction: &Auction,
        owner: Seat,
        partner: &PartnerContext,
        tie_break: crate::TieBreak,
    ) -> Vec<NaturalCandidate> {
        let calls: Vec<Call> = auction.legal_calls().collect();
        let mut out: Vec<NaturalCandidate> = self
            .infer_batch(auction, owner, partner, &calls)
            .into_iter()
            .filter(|c| !c.is_fallback())
            .collect();
        out.sort_by(|a, b| {
            crate::exclusive::natural_rank_cmp(
                tie_break,
                (a.call, a.priority()),
                (b.call, b.priority()),
            )
        });
        out
    }

    /// The natural implicit `Pass` (docs/design/07-bidding.md §4.1 and §5.2 step 3,
    /// 06-system.md §8.6): with `ImplicitPass::Complement`, a hand that satisfies none of the
    /// natural candidates passes. `ranked` is [`NaturalInference::ranked_candidates`]'s output.
    /// The result is a candidate for `Pass` whose constraint is `¬(C_1 ∨ … ∨ C_k)` over every
    /// ranked candidate (`ANY` when there are none), with confidence `0` and rule
    /// [`IMPLICIT_PASS_RULE`]. It ranks after every natural candidate (`choose_bid` gives it
    /// priority `i16::MIN + 1`); since it is disjoint from them by construction, its region is
    /// also its exclusive region.
    ///
    /// A `Pass` that is itself a ranked candidate (a limited `pass_default`, say 0-5 HCP after
    /// `1H P`) does not suppress it: the natural region of `Pass` is then `(pass rule ∧ ¬higher)
    /// ∨ ¬(C_1 ∨ … ∨ C_k)`, so a 17-count that fits no natural call after `1H P` still passes.
    pub fn implicit_pass(ranked: &[NaturalCandidate]) -> NaturalCandidate {
        let constraint = ranked
            .iter()
            .map(|c| c.constraint.clone())
            .reduce(HandConstraint::or)
            .map_or(HandConstraint::ANY, HandConstraint::not);
        NaturalCandidate {
            call: Call::Pass,
            constraint,
            confidence: 0.0,
            rule: IMPLICIT_PASS_RULE,
        }
    }

    /// Candidate calls with their natural constraints and priorities, for `choose_bid` when the
    /// auction is off-system (§8.3, last paragraph).
    pub fn candidates(&self, auction: &Auction, owner: Seat) -> Vec<(Call, HandConstraint, i16)> {
        let index = auction.len();
        let mut out = Vec::new();
        for call in auction.legal_calls() {
            let Ok(next) = auction.with(call) else {
                continue;
            };
            let ctx = classify(&next, index, owner);
            let inf = self.infer(&ctx);
            if inf.rule == "fallback" {
                continue;
            }
            let priority = (inf.confidence * 100.0).round() as i16;
            out.push((call, inf.constraint, priority));
        }
        out
    }
}

/// The level whose combined target ([`LevelFloor::combined`]) a bid that overrides partner's
/// game needs: the six level, slam (31 combined HCP in a suit, 32 in notrump by default).
pub const SLAM_LEVEL: u8 = 6;

/// `true` when the bid overrides partner's final game choice: partner's last call placed the
/// contract at game or higher (3NT, four of a major, five of a minor, or above), the right-hand
/// opponent passed it, and the bid is none of the following.
///
/// - An answer to, or a follow-up of, a 4NT/5NT ask by either partner
///   ([`CallContext::our_slam_ask`]): `1H-P-3H-P-4NT-P-5H`, `...-4NT-P-5D-P-5H`.
/// - A bid over a forcing game-level call ([`CallContext::forcing_situation`]): partner's call
///   was not a final choice.
/// - An ordinary correction of partner's 3NT ([`corrects_partners_3nt`]): `1H-P-3NT-P-4H` with
///   six hearts, `1NT-P-2H-P-2S-P-3NT-P-4S`.
///
/// What is left is a slam move, or pulling partner's game when the bid adds nothing partner did
/// not know (`2S-P-3NT-P-4S`: the weak two promised the six spades; `2S-P-2NT-P-3S-P-3NT-P-4S`),
/// which the natural rules describe only with slam values: [`apply_level_floor`] raises its
/// combined target to the six level's.
fn overrides_partners_game(ctx: &CallContext) -> bool {
    let Some(Call::Bid(partner_bid)) = ctx.partner_last else {
        return false;
    };
    is_game_or_higher(partner_bid)
        && ctx.rho_last == Some(Call::Pass)
        && !ctx.our_slam_ask
        && !ctx.forcing_situation
        && !corrects_partners_3nt(ctx)
}

/// `true` when the bid corrects partner's 3NT (the right-hand opponent passed it) to game in a
/// suit our side has bid and the opponents have not: four of a major or five of a minor, in a
/// strain `owner` has not bid twice already ([`CallContext::owner_repeated_strains`]) unless it
/// is the suit `owner` opened at the one level ([`rebid_opened_suit`]), and did not open with
/// a weak two or a preempt ([`opened_preemptively_in`]).
///
/// Such a bid tells partner something partner did not know when choosing 3NT: a sixth card in
/// the suit opened (`1H-P-3NT-P-4H`; SAYC's 3NT response promises only two hearts), or support
/// for the suit partner offered (`1NT-P-2H-P-2S-P-3NT-P-4S`, `1S-P-2H-P-2NT-P-3NT-P-4H`). The
/// ordinary rule describes it with the ordinary level floor, read as the cheapest game bid in
/// the suit (five of a minor is not a jump rebid or a jump raise) and, in a minor, with a hand
/// unsuited to notrump ([`unsuited_to_notrump`]). A one-level opener that has rebid its suit
/// (`1S-P-2C-P-2S-P-3NT-P-4S`) corrects too: the pull shows a seventh card, with the minimum the
/// rebid showed while the rebid limited the hand, or the range of opener's strongest other call
/// (a reverse, a jump shift, a 2NT rebid) when it did not
/// ([`rule_rebid_opened_suit_over_3nt`]). A suit nobody on our
/// side has bid (`1H-P-3NT-P-5C`), the opponents' suit (a cue bid), any other suit `owner` has
/// already rebid (`2S-P-2NT-P-3S-P-3NT-P-4S`) and the suit of a weak two or a preempt
/// (`2S-P-3NT-P-4S`: the opening already promised the six cards) are not corrections.
fn corrects_partners_3nt(ctx: &CallContext) -> bool {
    let (Some(Call::Bid(partner_bid)), Some(bid)) = (ctx.partner_last, ctx.call.bid()) else {
        return false;
    };
    let strain = bid.strain();
    partner_bid.level() == 3
        && partner_bid.strain() == Strain::NoTrump
        && ctx.rho_last == Some(Call::Pass)
        && strain != Strain::NoTrump
        && bid.level() == game_level(strain)
        && ctx.our_suits.contains(strain)
        && !ctx.their_suits.contains(strain)
        && (!ctx.owner_repeated_strains.contains(strain) || rebid_opened_suit(ctx, strain))
        && !opened_preemptively_in(ctx, strain)
}

/// `true` when `owner` opened one of `strain` and has bid it again, and it is not a suit partner
/// supported ([`CallContext::agreed_suit`]): `1S-P-2C-P-2S`, `1H-P-1S-P-2H`, `1S-P-1NT-P-3S`.
fn rebid_opened_suit(ctx: &CallContext, strain: Strain) -> bool {
    ctx.role == Role::Opener
        && ctx.owner_repeated_strains.contains(strain)
        && ctx
            .opener_first_bid
            .is_some_and(|b| b.level() == 1 && b.strain() == strain)
        && ctx.agreed_suit.map(Strain::from_suit) != Some(strain)
}

/// `true` when `owner` opened a weak two or a preempt in `strain` (two or more of a suit, but
/// not a strong 2C): the opening promised the long suit already, so bidding it again over
/// partner's 3NT (`2S-P-3NT-P-4S`, `3H-P-3NT-P-4H`) adds nothing partner did not know.
fn opened_preemptively_in(ctx: &CallContext, strain: Strain) -> bool {
    ctx.role == Role::Opener
        && ctx.opener_first_bid.is_some_and(|b| {
            b.strain() == strain && b.level() >= 2 && !(b.level() == 2 && strain == Strain::Clubs)
        })
}

/// A hand unsuited to notrump: a singleton or a void. A correction of partner's 3NT to five of
/// a minor shows one (with a balanced hand the nine-trick game is the natural contract), so a
/// notrump opener never makes it.
fn unsuited_to_notrump() -> HandConstraint {
    let short = Suit::ALL.into_iter().fold(ShapeSet::EMPTY, |acc, s| {
        acc.union(suit_len_set(s, &(0..=1)))
    });
    HandConstraint::Atom(Atom {
        shapes: short,
        ..Atom::ANY
    })
}

/// The jump the natural rules read for a bid: [`CallKind::Bid::jump`], except that a correction
/// of partner's 3NT ([`corrects_partners_3nt`]) is the cheapest game bid in its suit, not a jump
/// (`1D-P-3NT-P-5D` skips four diamonds only because four is not game).
fn effective_jump(ctx: &CallContext, jump: u8) -> u8 {
    if corrects_partners_3nt(ctx) { 0 } else { jump }
}

/// `constraint`, with what a correction of partner's 3NT ([`corrects_partners_3nt`]) adds:
///
/// - in a minor, a hand unsuited to notrump ([`unsuited_to_notrump`]);
/// - with `fit` (a correction to a suit whose length the rule does not by itself make a fit:
///   a raise, a notrump opener's suit, the agreed suit), an eight-card fit with the length
///   partner has shown ([`eight_card_fit_len`]). After `1NT-P-2H-P-2S-P-3NT` partner has
///   shown five spades, so three make the fit; after Stayman (`1NT-P-2C-P-2S-P-3NT`) partner
///   has shown none, and no notrump opener has eight, so the call describes no hand.
///
/// The one-suit opener's own six-card suit (`1H-P-3NT-P-4H`) is a correction on its own.
fn with_correction_shape(
    ctx: &CallContext,
    constraint: HandConstraint,
    fit: bool,
) -> HandConstraint {
    let Some(bid) = ctx.call.bid() else {
        return constraint;
    };
    if !corrects_partners_3nt(ctx) {
        return constraint;
    }
    let mut constraint = constraint;
    if bid.strain().is_minor() {
        constraint = constraint.and(unsuited_to_notrump());
    }
    if let (true, Some(suit), Some(len)) = (fit, bid.strain().suit(), eight_card_fit_len(ctx)) {
        constraint = constraint.and(HandConstraint::Atom(Atom::ANY.with_len(suit, len..=13)));
    }
    constraint
}

/// The fewest cards in the bid suit that make an eight-card fit with partner's shown length
/// (the minimum of [`CallContext::partner_constraint`] in that suit), or `None` when partner's
/// constraint is not known or the call is not a suit bid.
fn eight_card_fit_len(ctx: &CallContext) -> Option<u8> {
    let suit = ctx.call.bid()?.strain().suit()?;
    let shown = *ctx.partner_constraint.as_ref()?.suit_len(suit).start();
    Some(8u8.saturating_sub(shown))
}

/// Applies `floor` to a natural bid's inference (see [`LevelFloor`]): a continuation bid (the
/// caller or partner has acted) at a level with a non-zero combined target, with a known
/// `partner_constraint`, also requires `own HCP >= combined - partner's minimum HCP`. A bid that
/// overrides partner's game ([`overrides_partners_game`]) uses at least the six level's target.
fn apply_level_floor(
    floor: &LevelFloor,
    ctx: &CallContext,
    mut inf: Inference,
    ex: bool,
) -> Inference {
    let Some(bid) = ctx.call.bid() else {
        return inf;
    };
    if !ctx.owner_acted && ctx.partner_actions == 0 {
        return inf;
    }
    let nt = bid.strain() == Strain::NoTrump;
    let mut combined = floor.combined(bid.level(), nt);
    if overrides_partners_game(ctx) {
        combined = combined.max(floor.combined(SLAM_LEVEL, nt));
    }
    if combined == 0 {
        return inf;
    }
    let Some(partner) = ctx.partner_constraint.as_ref() else {
        return inf;
    };
    let partner_min = *partner.hcp_range().start();
    let own_min = combined.saturating_sub(partner_min);
    if own_min <= *inf.constraint.hcp_range().start() {
        return inf;
    }
    inf.constraint = inf
        .constraint
        .and(HandConstraint::Atom(Atom::ANY.with_hcp(own_min..=37)));
    if ex {
        inf.explanation = format!(
            "{} (level {}: {own_min}+ hcp)",
            inf.explanation,
            bid.level()
        );
    }
    inf
}

/// `format!(..)` when the explanation is wanted (`$ex`), an empty (non-allocating) string
/// otherwise ([`NaturalInference::infer_batch`]).
macro_rules! expl {
    ($ex:expr, $($arg:tt)*) => {
        if $ex {
            format!($($arg)*)
        } else {
            String::new()
        }
    };
}

/// Rough combined-points target for game: the §8.3 `cue` formula's `gf_total`, and the upper
/// bound of a pass of partner's notrump (`pass_default`). Not a field of [`NaturalParams`]
/// because §8.3 only uses it as a fallback formula, not a tunable.
const GF_TOTAL: u8 = 25;

/// Standard "opening 1-of-a-minor" length and strength, adjusted for the balancing seat.
fn opener_or_balancer_hcp(
    p: &NaturalParams,
    role: Role,
    base: RangeInclusive<u8>,
) -> RangeInclusive<u8> {
    if role == Role::Balancer {
        let shifted = (*base.start() as i16 + p.balancing_shift as i16).clamp(0, 37) as u8;
        shifted..=*base.end()
    } else {
        base
    }
}

fn rule_open_1m(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.role != Role::Opener || ctx.our_suits.0 != 0 {
        return None;
    }
    let bid = ctx.call.bid()?;
    if bid.level() != 1 || !bid.strain().is_minor() {
        return None;
    }
    let suit = bid.strain().suit()?;
    let constraint = HandConstraint::Atom(
        Atom::ANY
            .with_hcp(p.opening_hcp.clone())
            .with_len(suit, p.open_1m_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.55,
        rule: "open_1m",
        explanation: expl!(
            ex,
            "opening {}: {}+ cards, {}-{} hcp",
            ctx.call,
            p.open_1m_len,
            p.opening_hcp.start(),
            p.opening_hcp.end()
        ),
    })
}

fn rule_open_1major(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.role != Role::Opener || ctx.our_suits.0 != 0 {
        return None;
    }
    let bid = ctx.call.bid()?;
    if bid.level() != 1 || !bid.strain().is_major() {
        return None;
    }
    let suit = bid.strain().suit()?;
    let constraint = HandConstraint::Atom(
        Atom::ANY
            .with_hcp(p.opening_hcp.clone())
            .with_len(suit, p.open_1major_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.6,
        rule: "open_1M",
        explanation: expl!(
            ex,
            "opening {}: {}+ cards, {}-{} hcp",
            ctx.call,
            p.open_1major_len,
            p.opening_hcp.start(),
            p.opening_hcp.end()
        ),
    })
}

fn rule_open_nt(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.role != Role::Opener || ctx.our_suits.0 != 0 {
        return None;
    }
    let CallKind::Bid { nt: true, .. } = ctx.kind else {
        return None;
    };
    let (_, hcp) = p.nt.iter().find(|(level, _)| *level == ctx.level)?;
    let constraint = HandConstraint::Atom(
        Atom {
            shapes: ShapeSet::BALANCED,
            ..Atom::ANY
        }
        .with_hcp(hcp.clone()),
    );
    Some(Inference {
        constraint,
        confidence: 0.7,
        rule: "open_nt",
        explanation: expl!(
            ex,
            "opening {}: balanced, {}-{} hcp",
            ctx.call,
            hcp.start(),
            hcp.end()
        ),
    })
}

fn rule_open_weak2(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.role != Role::Opener || ctx.our_suits.0 != 0 {
        return None;
    }
    let bid = ctx.call.bid()?;
    if bid.level() != 2 || bid.strain() == Strain::Clubs || bid.strain() == Strain::NoTrump {
        return None;
    }
    let suit = bid.strain().suit()?;
    let (min_len, hcp) = &p.weak_two;
    let constraint = HandConstraint::Atom(
        Atom::ANY
            .with_hcp(hcp.clone())
            .with_len(suit, *min_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.5,
        rule: "open_weak2",
        explanation: expl!(
            ex,
            "weak two {}: {}+ cards, {}-{} hcp",
            ctx.call,
            min_len,
            hcp.start(),
            hcp.end()
        ),
    })
}

fn rule_open_2c(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.role != Role::Opener || ctx.our_suits.0 != 0 {
        return None;
    }
    let bid = ctx.call.bid()?;
    if bid != Bid::new(2, Strain::Clubs)? {
        return None;
    }
    let constraint = HandConstraint::Atom(Atom::ANY.with_hcp(p.strong_two_c..=37));
    Some(Inference {
        constraint,
        confidence: 0.5,
        rule: "open_2c",
        explanation: expl!(ex, "strong 2C: {}+ hcp", p.strong_two_c),
    })
}

fn rule_open_preempt(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.role != Role::Opener || ctx.our_suits.0 != 0 {
        return None;
    }
    let bid = ctx.call.bid()?;
    if !(3..=5).contains(&bid.level()) || bid.strain() == Strain::NoTrump {
        return None;
    }
    let suit = bid.strain().suit()?;
    let (_, min_len, hcp) = p
        .preempt
        .iter()
        .find(|(level, _, _)| *level == bid.level())?;
    let constraint = HandConstraint::Atom(
        Atom::ANY
            .with_hcp(hcp.clone())
            .with_len(suit, *min_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.55,
        rule: "open_preempt",
        explanation: expl!(
            ex,
            "preempt {}: {}+ cards, {}-{} hcp",
            ctx.call,
            min_len,
            hcp.start(),
            hcp.end()
        ),
    })
}

fn rule_open_pass(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    // Only the pass in the opening seat, before anyone has bid (§8.3). `classify_role` keeps the
    // opener in `Role::Opener` for the whole auction, so without the `last_bid` guard every later
    // pass by the opener (of partner's raise, of 3NT, ...) would claim 0-11 HCP.
    if ctx.role != Role::Opener
        || ctx.call != Call::Pass
        || ctx.passed_hand
        || ctx.last_bid.is_some()
    {
        return None;
    }
    let hi = p.opening_hcp.start().saturating_sub(1);
    let constraint = HandConstraint::Atom(Atom::ANY.with_hcp(0..=hi));
    Some(Inference {
        constraint,
        // Below `open_weak2` (0.5): the weak-two range lies inside 0-11 HCP, and at an equal
        // priority `Pass` wins by call order, so the natural policy never opened a weak two.
        confidence: 0.45,
        rule: "open_pass",
        explanation: expl!(ex, "declines to open: 0-{hi} hcp"),
    })
}

/// `true` for the overcaller's (or balancer's) *first* action: every overcall rule describes
/// entering the auction, not the same player's later bids (which `classify_role` still labels
/// `Overcaller`).
fn is_first_overcall(ctx: &CallContext) -> bool {
    matches!(ctx.role, Role::Overcaller | Role::Balancer) && !ctx.owner_acted
}

/// The highest level of an ordinary natural first entry once the opponents have exchanged bids
/// ([`CallContext::their_bids`] >= 2).
///
/// The overcall rules describe entering over the opening, or competing for a partscore after
/// the opponents have bid and raised (the three level in the balancing seat). Above that the
/// generic ranges (five cards, 10-16 HCP, a king less in the balancing seat) are not a natural
/// action any more:
///
/// - a four-level first entry (over a raise to the three level or to four of a minor, and
///   also over their 3NT or four of a major: `(1H)-P-(4H)-4S`) needs what SAYC's own
///   competitive tables ask for there (`systems/sayc/competing.bml`): a six-card suit and
///   opening values (`four_level_entry`: 6+ cards and 12-16 HCP with the default
///   parameters), a king less (9-16) in the balancing seat below their game. Over their game
///   only a first chance is described: a player who passed at an earlier turn after their
///   opening, in the direct seat (`(1H)-P-(2H)-P-(4H)-4S`) or the pass-out seat, could have
///   overcalled the same suit then, so a six-card suit with opening values is what the pass
///   denied ([`CallContext::passed_after_opening`]);
/// - a first entry at the five level or higher (over their `4C`/`4D`, or over their game:
///   five hearts and 7-16 HCP over their `1S-3S-4S` do not bid `5H`) is a sacrifice or a
///   lead-directing gamble, not a natural overcall. No overcall rule fires, so the natural
///   policy passes.
///
/// docs/design/06-system.md §8.3 (`overcall`) and §8.6.
pub const MAX_ENTRY_LEVEL_AFTER_EXCHANGE: u8 = 3;

/// The minimum suit length of a four-level first entry (see [`MAX_ENTRY_LEVEL_AFTER_EXCHANGE`]
/// and [`MAX_JUMP_OVERCALL_LEVEL`]).
pub const FOUR_LEVEL_ENTRY_MIN_LEN: u8 = 6;

/// The highest level of a weak jump overcall: a two- or three-level preempt over their bid
/// (`overcall[2]`: a weak-two hand).
///
/// A single jump to the four level (`(2S)-4H`, `(3C)-4H`, `(1H)-P-(2NT)-4C`) is not a weak-two
/// hand: it shows the four-level entry's six cards and opening values (`four_level_entry`:
/// 6+ cards, 12-16 HCP, 9-16 in the balancing seat with the default parameters). A single
/// jump to the five level or higher (`(3S)-5C`, `(3H)-P-(4NT)-6C`) is not described.
pub const MAX_JUMP_OVERCALL_LEVEL: u8 = 3;

/// The suit length and HCP range of a four-level first entry (see
/// [`MAX_ENTRY_LEVEL_AFTER_EXCHANGE`] and [`MAX_JUMP_OVERCALL_LEVEL`]): `max(overcall[1].len,
/// FOUR_LEVEL_ENTRY_MIN_LEN)`+ cards and `max(overcall[1].start, opening.start)..=
/// overcall[1].end` HCP (6+ cards and 12-16 with the default parameters). The balancing shift
/// is applied by the caller.
fn four_level_entry(p: &NaturalParams) -> (u8, RangeInclusive<u8>) {
    let (min_len, hcp) = &p.overcall[1];
    let from = (*hcp.start()).max(*p.opening_hcp.start());
    (
        (*min_len).max(FOUR_LEVEL_ENTRY_MIN_LEN),
        from..=(*hcp.end()).max(from),
    )
}

/// The game level of `strain`: 3 in notrump, 4 in a major, 5 in a minor.
fn game_level(strain: Strain) -> u8 {
    match strain {
        Strain::NoTrump => 3,
        Strain::Hearts | Strain::Spades => 4,
        Strain::Clubs | Strain::Diamonds => 5,
    }
}

/// `true` when `bid` is a game contract or higher: 3NT, four of a major, five of a minor.
fn is_game_or_higher(bid: Bid) -> bool {
    bid.level() >= game_level(bid.strain())
}

/// How the overcall rules apply to a first entry at `ctx.level` (see
/// [`MAX_ENTRY_LEVEL_AFTER_EXCHANGE`]): `None` when no overcall rule applies (five level or
/// higher after the opponents' exchange), `Some(true)` for a four-level entry after the
/// exchange, also over their game ([`four_level_entry`]), and `Some(false)` for the ordinary
/// ranges.
fn entry_after_exchange(ctx: &CallContext) -> Option<bool> {
    if ctx.their_bids < 2 {
        return Some(false);
    }
    match ctx.level {
        0..=MAX_ENTRY_LEVEL_AFTER_EXCHANGE => Some(false),
        4 => Some(true),
        _ => None,
    }
}

fn rule_overcall(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if !is_first_overcall(ctx) {
        return None;
    }
    let four_level = entry_after_exchange(ctx)?;
    // `new_suit` excludes a cue bid of the opponents' suit (which is `rule_cue`'s).
    let CallKind::Bid {
        nt: false,
        jump: 0,
        new_suit: true,
        ..
    } = ctx.kind
    else {
        return None;
    };
    let bid = ctx.call.bid()?;
    let suit = bid.strain().suit()?;
    let (min_len, hcp) = if four_level {
        four_level_entry(p)
    } else if bid.level() == 1 {
        p.overcall[0].clone()
    } else {
        p.overcall[1].clone()
    };
    // Over their game an entry by a player who passed at an earlier turn after their opening,
    // in any seat (`(1H)-P-(2H)-P-(4H)-4S` in the direct seat, `(1H)-P-(4H)-P-(P)-4S` in the
    // pass-out seat), is not a first chance: an overcall in the same suit was available then,
    // so the six cards and opening values a first entry shows are what the pass denied. The
    // pass-out seat over their game is not a balance either (there is no partscore to contest
    // and partner's values are not trapped), and always passed earlier. No rule describes the
    // entry (MAX_ENTRY_LEVEL_AFTER_EXCHANGE).
    let over_their_game = four_level && ctx.last_bid.is_some_and(is_game_or_higher);
    if over_their_game && ctx.passed_after_opening {
        return None;
    }
    let hcp = if over_their_game {
        hcp
    } else {
        opener_or_balancer_hcp(p, ctx.role, hcp)
    };
    let constraint =
        HandConstraint::Atom(Atom::ANY.with_hcp(hcp.clone()).with_len(suit, min_len..=13));
    Some(Inference {
        constraint,
        confidence: 0.35,
        rule: "overcall",
        explanation: expl!(
            ex,
            "overcall {}: {}+ cards, {}-{} hcp",
            ctx.call,
            min_len,
            hcp.start(),
            hcp.end()
        ),
    })
}

fn rule_jump_overcall(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if !is_first_overcall(ctx) {
        return None;
    }
    let CallKind::Bid {
        nt: false,
        jump: 1,
        new_suit: true,
        ..
    } = ctx.kind
    else {
        return None;
    };
    let bid = ctx.call.bid()?;
    let suit = bid.strain().suit()?;
    // A weak jump overcall up to the three level; a single jump to the four level shows the
    // four-level entry's values; higher jumps are not described (MAX_JUMP_OVERCALL_LEVEL).
    // The four-level jump ranks like the overcall (0.35): its hands are a subset of the
    // cheaper overcall's, which the natural policy prefers (the call index breaks the tie).
    // The default Mirror interpretation therefore reads the jump itself as ANY; this range
    // reaches explanation text, the Legacy interpretation, direct `infer` callers and the
    // jumper's partner's context (docs/design/06-system.md §8.6, lane N2's N-C).
    let ((min_len, hcp), confidence) = if ctx.level <= MAX_JUMP_OVERCALL_LEVEL {
        (p.overcall[2].clone(), 0.5)
    } else if ctx.level == MAX_JUMP_OVERCALL_LEVEL + 1 {
        (four_level_entry(p), 0.35)
    } else {
        return None;
    };
    let hcp = opener_or_balancer_hcp(p, ctx.role, hcp);
    let constraint =
        HandConstraint::Atom(Atom::ANY.with_hcp(hcp.clone()).with_len(suit, min_len..=13));
    Some(Inference {
        constraint,
        confidence,
        rule: "jump_overcall",
        explanation: expl!(
            ex,
            "jump overcall {}: {}+ cards, {}-{} hcp",
            ctx.call,
            min_len,
            hcp.start(),
            hcp.end()
        ),
    })
}

fn rule_nt_overcall(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if !is_first_overcall(ctx) {
        return None;
    }
    // The natural strong notrump overcall is the cheapest notrump (1NT over a 1-level opening,
    // 2NT over a weak two); a jump to 2NT (unusual) or to 3NT is something else.
    let CallKind::Bid {
        nt: true, jump: 0, ..
    } = ctx.kind
    else {
        return None;
    };
    if ctx.level > 2 {
        return None;
    }
    let hcp = opener_or_balancer_hcp(p, ctx.role, p.nt_overcall.clone());
    let mut constraint = HandConstraint::Atom(
        Atom {
            shapes: ShapeSet::BALANCED,
            ..Atom::ANY
        }
        .with_hcp(hcp.clone()),
    );
    if let Some(their_suit) = ctx.last_bid.and_then(|b| b.strain().suit()) {
        constraint = constraint.and(stopper(their_suit));
    }
    Some(Inference {
        constraint,
        confidence: 0.6,
        rule: "nt_overcall",
        explanation: expl!(
            ex,
            "notrump overcall {}: balanced, {}-{} hcp, stopper",
            ctx.call,
            hcp.start(),
            hcp.end()
        ),
    })
}

/// Extra HCP over `takeout_double`'s minimum that a defender's second takeout double shows (a
/// takeout double by an overcaller or balancer who has already made a non-pass call): a king
/// more, the booklet's "extra values".
pub const SECOND_TAKEOUT_DOUBLE_EXTRA: u8 = 3;

/// `true` when an overcaller or a balancer makes a takeout double after an earlier non-pass
/// call of their own: `(1C)-1S-(X)-P-(2H)-X`, `(1S)-P-(P)-X-(2S)-P-(P)-X`.
///
/// The first action (the overcall, the takeout double, the balancing call) already showed a
/// minimum; with no more than that the defender passes or bids the suit again, so the second
/// double shows [`SECOND_TAKEOUT_DOUBLE_EXTRA`] points more than a takeout double: 15+, 12+ in
/// the balancing seat (with the default parameters).
///
/// Not covered, so their takeout doubles keep the ordinary minimum:
///
/// - a player whose only earlier non-pass call answered partner's takeout double
///   ([`CallContext::owner_answered_partners_double`]), in any role: the forced answer showed
///   nothing, not a minimum. In the direct seat that player is the `Advancer`
///   (`(1H)-X-(P)-1S-(P)-P-(2H)-X`: 12+), in the pass-out seat the `Balancer`
///   (`(1H)-X-(P)-1S-(2H)-P-(P)-X`: 9+);
/// - the advancer, whatever its earlier call;
/// - opener: a reopening double with a minimum opening (`1D-(1S)-P-(P)-X`) is standard
///   (docs/design/06-system.md §8.3).
fn is_defenders_second_action(ctx: &CallContext) -> bool {
    matches!(ctx.role, Role::Overcaller | Role::Balancer)
        && ctx.owner_acted
        && !ctx.owner_answered_partners_double
}

fn rule_takeout_x(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.call != Call::Double {
        return None;
    }
    let CallKind::Double(DoubleKind::Takeout) = ctx.kind else {
        return None;
    };
    let their_suit = ctx.last_bid.and_then(|b| b.strain().suit());
    let (min_hcp, their_max, unbid_min) = p.takeout_double;
    let min_hcp = if is_defenders_second_action(ctx) {
        min_hcp.saturating_add(SECOND_TAKEOUT_DOUBLE_EXTRA)
    } else {
        min_hcp
    };
    let min_hcp = opener_or_balancer_hcp(p, ctx.role, min_hcp..=37);

    let mut constraint = HandConstraint::Atom(Atom::ANY.with_hcp(min_hcp.clone()));
    if let Some(their_suit) = their_suit {
        constraint = constraint.and(HandConstraint::Atom(
            Atom::ANY.with_len(their_suit, 0..=their_max),
        ));
    }
    let unbid: Vec<Suit> = Suit::ALL
        .into_iter()
        .filter(|&s| {
            let strain = Strain::from_suit(s);
            !ctx.our_suits.contains(strain) && !ctx.their_suits.contains(strain)
        })
        .collect();
    constraint = constraint.and(at_least_two_of(&unbid, unbid_min));

    Some(Inference {
        constraint,
        confidence: 0.45,
        rule: "takeout_x",
        explanation: expl!(ex, "takeout double: {}+ hcp", min_hcp.start()),
    })
}

fn rule_penalty_x(_p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.call != Call::Double {
        return None;
    }
    let CallKind::Double(DoubleKind::Penalty) = ctx.kind else {
        return None;
    };
    let their_suit = ctx.last_bid.and_then(|b| b.strain().suit());
    let mut constraint = HandConstraint::Atom(Atom::ANY.with_hcp(10..=37));
    if let Some(their_suit) = their_suit {
        constraint = constraint.and(HandConstraint::Atom(Atom::ANY.with_len(their_suit, 4..=13)));
    }
    Some(Inference {
        constraint,
        confidence: 0.3,
        rule: "penalty_x",
        explanation: expl!(ex, "penalty double: 10+ hcp, 4+ of their suit"),
    })
}

/// `true` when responder doubles a bid of the right-hand opponent while partner's opening is
/// still partner's only non-pass call and partner's last call (`1C (1H) X`, `P P 1D (2C) X`):
/// responder's turn over the overcall of the opening.
///
/// The negative double is that call. A later double by responder (after a first response, a
/// first negative double, or a first pass: `1C (1H) X (2D) P (P) X`, `1H (1S) P (2S) P (P) X`,
/// `2D (P) P (2H) P (P) X`) is classified `Negative` by the §8.2 order but is not a negative
/// double, whose range (6+, an unbid major) it does not have:
///
/// - after responder's own unlimited first call (a negative double or a new suit), it is
///   `competitive_x` ([`rule_competitive_x`]): the first call showed a minimum, so the double
///   shows [`LATER_RESPONDER_DOUBLE_EXTRA`] points more than a negative double at that level;
/// - after a limited first response (a raise of partner's suit or notrump,
///   [`responders_first_call_limited`]) it is competitive within that response's range; no rule
///   describes it;
/// - after responder's first pass (a weak hand, a trap pass, or a pass of partner's weak two)
///   it can be a balancing takeout double or a penalty double with the hand the pass hid; no
///   rule describes it, so the natural policy passes (docs/design/06-system.md §8.3).
fn responders_first_turn_over_overcall(ctx: &CallContext) -> bool {
    ctx.role == Role::Responder
        && ctx.partner_actions == 1
        && matches!(ctx.partner_last, Some(Call::Bid(_)))
        && ctx.partner_last == ctx.partner_first_action
        && matches!(ctx.rho_last, Some(Call::Bid(_)))
}

/// `true` when `owner`'s first non-pass call was a limited response: notrump, or a bid of the
/// strain of partner's first call (a raise of the opening, a jump raise included). A new suit,
/// a cue bid of the opponents' suit and a negative double are not limited.
fn responders_first_call_limited(ctx: &CallContext) -> bool {
    let Some(Call::Bid(first)) = ctx.owner_first_action else {
        return false;
    };
    first.strain() == Strain::NoTrump
        || ctx
            .partner_first_action
            .and_then(|c| c.bid())
            .is_some_and(|opening| opening.strain() == first.strain())
}

/// The minimum HCP of a negative double of a bid at the level of `ctx.last_bid`
/// ([`negative_double_min_at`]).
fn negative_double_min_hcp(p: &NaturalParams, ctx: &CallContext) -> u8 {
    negative_double_min_at(p, ctx.last_bid.map(|b| b.level()).unwrap_or(1))
}

/// The minimum HCP of a negative double of a bid at `their_level`:
/// `response.new_suit_1.1 + 2 × (level − 1)` (6 at the one level, 8 at the two level).
fn negative_double_min_at(p: &NaturalParams, their_level: u8) -> u8 {
    p.response
        .new_suit_1
        .1
        .saturating_add(2u8.saturating_mul(their_level.saturating_sub(1)))
}

/// Extra HCP over the negative double's minimum at the same level
/// (`response.new_suit_1.1 + 2 × (level − 1)`) that responder's later double shows after
/// responder's own non-pass call (rule `competitive_x`): a king more, like
/// [`SECOND_TAKEOUT_DOUBLE_EXTRA`] for a defender. docs/design/06-system.md §8.3.
pub const LATER_RESPONDER_DOUBLE_EXTRA: u8 = 3;

/// Responder's later double after responder's own unlimited first call (a first negative
/// double or a new suit): `1C (1H) X (2D) P (P) X`, `1D (1H) 1S (2H) P (P) X`. The §8.2 order
/// classifies it `Negative`, but it is not a negative double
/// ([`responders_first_turn_over_overcall`]). Responder has shown a minimum already, and with
/// no more than that passes or bids again; the double is competitive and shows the values
/// the first call could not: [`LATER_RESPONDER_DOUBLE_EXTRA`] more than a negative double at
/// that level (9+ over a one-level bid, 11+ over a two-level bid). Strength only: the double
/// may be for takeout or for penalty, which the auction alone does not tell.
///
/// After a limited first response ([`responders_first_call_limited`]: `1H (1S) 2H (2S) P (P)
/// X`, `1D (1S) 1NT (2S) P (P) X`) the rule does not apply: with more than that response's
/// range responder would have bid differently, so the double is competitive within the range
/// (a maximum). No rule describes it: the top of the range was measured and dropped
/// (docs/design/06-system.md §8.6; for a raise, the natural rules rank the competitive raise
/// to the three level above the double for the same hands anyway).
fn rule_competitive_x(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.call != Call::Double
        || ctx.role != Role::Responder
        || !ctx.owner_acted
        || responders_first_turn_over_overcall(ctx)
        || responders_first_call_limited(ctx)
    {
        return None;
    }
    let CallKind::Double(DoubleKind::Negative) = ctx.kind else {
        return None;
    };
    let min_hcp = negative_double_min_hcp(p, ctx).saturating_add(LATER_RESPONDER_DOUBLE_EXTRA);
    Some(Inference {
        constraint: HandConstraint::Atom(Atom::ANY.with_hcp(min_hcp..=37)),
        confidence: 0.4,
        rule: "competitive_x",
        explanation: expl!(ex, "responder's competitive double: {min_hcp}+ hcp"),
    })
}

fn rule_negative_x(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.call != Call::Double {
        return None;
    }
    let CallKind::Double(DoubleKind::Negative) = ctx.kind else {
        return None;
    };
    if !responders_first_turn_over_overcall(ctx) {
        return None;
    }
    let min_hcp = negative_double_min_hcp(p, ctx);
    // Only the major(s) neither side has bid yet count: holding length in the suit the
    // opponents just showed (their overcall) says nothing about an unbid major. Where the new
    // suits are still available at the 1 level the double is what they are not (SAYC):
    //
    // - both majors unbid (`1C (1D) X`): both majors, since `1H`/`1S` show one four-card major;
    //   ranked with them (0.5, and the double wins the tie by call order) so a 4-4 hand doubles;
    // - one major unbid (`1C (1H) X`): exactly four, since `1S` shows five.
    //
    // Otherwise (a 2-level overcall, `1D (2C) X`) it is four cards in either unbid major.
    let unbid: Vec<Suit> = [Suit::Hearts, Suit::Spades]
        .into_iter()
        .filter(|&s| {
            let strain = Strain::from_suit(s);
            !ctx.our_suits.contains(strain) && !ctx.their_suits.contains(strain)
        })
        .collect();
    let one_level = |s: Suit| {
        ctx.last_bid
            .is_some_and(|b| b.level() == 1 && Strain::from_suit(s) > b.strain())
    };
    let hcp = HandConstraint::Atom(Atom::ANY.with_hcp(min_hcp..=37));
    let (majors, confidence) = match *unbid.as_slice() {
        [h, s] if one_level(h) && one_level(s) => (
            HandConstraint::Atom(Atom::ANY.with_len(h, 4..=13).with_len(s, 4..=13)),
            0.5,
        ),
        [m] if one_level(m) => (
            HandConstraint::Atom(Atom::ANY.with_len(m, 4..=MIN_NEW_SUIT_OVER_OVERCALL - 1)),
            0.4,
        ),
        _ => (
            unbid
                .iter()
                .map(|&s| HandConstraint::Atom(Atom::ANY.with_len(s, 4..=13)))
                .reduce(HandConstraint::or)
                .unwrap_or(HandConstraint::ANY),
            0.4,
        ),
    };
    let constraint = majors.and(hcp);
    Some(Inference {
        constraint,
        confidence,
        rule: "negative_x",
        explanation: expl!(ex, "negative double: {min_hcp}+ hcp, 4+ card unbid major"),
    })
}

fn rule_raise(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    let CallKind::Bid {
        raise: true, jump, ..
    } = ctx.kind
    else {
        return None;
    };
    let bid = ctx.call.bid()?;
    let suit = bid.strain().suit()?;
    let jump = effective_jump(ctx, jump);
    let (min_len, hcp) = match ctx.role {
        // A choice of game over opener's 3NT, not a game raise; no rule when responder's
        // first call has no range (`responders_first_call_hcp`).
        Role::Responder if corrects_partners_3nt(ctx) => {
            (p.response.raise.0, responders_first_call_hcp(p, ctx)?)
        }
        Role::Responder if bid.level() >= 4 => (p.response.raise.0, 13..=37),
        Role::Responder if jump >= 1 => (p.response.jump_raise.0, p.response.jump_raise.1.clone()),
        Role::Responder => (p.response.raise.0, p.response.raise.1.clone()),
        Role::Opener if jump >= 1 => (p.response.raise.0, p.rebid.jump_raise.clone()),
        Role::Opener => (p.response.raise.0, p.rebid.raise.clone()),
        _ => (p.advance.raise.0, p.advance.raise.1.clone()),
    };
    let constraint = with_correction_shape(
        ctx,
        HandConstraint::Atom(Atom::ANY.with_hcp(hcp.clone()).with_len(suit, min_len..=13)),
        true,
    );
    Some(Inference {
        constraint,
        confidence: 0.45,
        rule: "raise",
        explanation: expl!(
            ex,
            "raise {}: {}+ support, {}-{} hcp",
            ctx.call,
            min_len,
            hcp.start(),
            hcp.end()
        ),
    })
}

/// The minimum HCP of responder's redouble after partner's opening was doubled (`1H (X) XX`):
/// SAYC's 10+.
const RESPONDER_REDOUBLE_MIN_HCP: u8 = 10;

/// The HCP range responder's first non-pass call showed, for a later bid that only chooses the
/// game (a correction of opener's 3NT, [`corrects_partners_3nt`]). Each limited call keeps its
/// range and each unlimited one its minimum, open-ended:
///
/// - a one-level new suit: `response.new_suit_1.1`+; a non-jump new suit at the two or three
///   level: `response.new_suit_2.1`+ (the natural rules describe only the two-level one, and the
///   forcing three-level new suit in competition, `1S (2H) 3C`, is no weaker); a jump shift:
///   `response.jump_shift`+;
/// - notrump: `response.nt[L]`;
/// - a raise of partner's opening: `response.raise.1`, a jump raise `response.jump_raise.1`; a
///   cue bid of the opponents' suit: `response.jump_raise.1.start`+ (a limit raise or better);
/// - a negative double ([`CallContext::owner_first_negative_double`]): its own minimum at the
///   level it doubled (`response.new_suit_1.1 + 2 × (level − 1)`)+; a redouble: 10+.
///
/// `None` for anything else (another double, a notrump level with no range, or no non-pass call
/// before the correction): no rule describes the correction.
fn responders_first_call_hcp(p: &NaturalParams, ctx: &CallContext) -> Option<RangeInclusive<u8>> {
    let first = match ctx.owner_first_action {
        Some(Call::Bid(first)) => first,
        Some(Call::Redouble) => return Some(RESPONDER_REDOUBLE_MIN_HCP..=37),
        Some(Call::Double) => {
            return ctx
                .owner_first_negative_double
                .map(|level| negative_double_min_at(p, level)..=37);
        }
        _ => return None,
    };
    let jump = ctx.owner_first_jump;
    if first.strain() == Strain::NoTrump {
        return p
            .response
            .nt
            .iter()
            .find(|(level, _)| *level == first.level())
            .map(|(_, hcp)| hcp.clone());
    }
    let opening = ctx.partner_first_action.and_then(|c| c.bid());
    if opening.is_some_and(|o| o.strain() == first.strain()) {
        return Some(if jump >= 1 {
            p.response.jump_raise.1.clone()
        } else {
            p.response.raise.1.clone()
        });
    }
    if ctx.their_suits.contains(first.strain()) {
        return Some(*p.response.jump_raise.1.start()..=37);
    }
    Some(match (first.level(), jump) {
        (1, _) => p.response.new_suit_1.1..=37,
        (_, 0) => p.response.new_suit_2.1..=37,
        _ => p.response.jump_shift..=37,
    })
}

/// Responder's own suit over opener's 3NT, a correction ([`corrects_partners_3nt`]):
/// `1C-P-1S-P-3NT-P-4S`. Six cards or more (the first call showed four or five), the range of
/// responder's first call ([`responders_first_call_hcp`]; no rule when it has none) and, in a
/// minor, a hand unsuited to notrump ([`with_correction_shape`]); the ordinary level floor.
fn rule_responder_corrects_to_own_suit(
    p: &NaturalParams,
    ctx: &CallContext,
    suit: Suit,
    ex: bool,
) -> Option<Inference> {
    let hcp = responders_first_call_hcp(p, ctx)?;
    let constraint = with_correction_shape(
        ctx,
        HandConstraint::Atom(Atom::ANY.with_hcp(hcp.clone()).with_len(suit, 6..=13)),
        false,
    );
    Some(Inference {
        constraint,
        confidence: 0.5,
        rule: "rebid_own",
        explanation: expl!(
            ex,
            "corrects 3NT to own suit {}: 6+ cards, {}-{} hcp",
            ctx.call,
            hcp.start(),
            hcp.end()
        ),
    })
}

fn rule_new_suit_resp_1(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.role != Role::Responder {
        return None;
    }
    let CallKind::Bid { new_suit: true, .. } = ctx.kind else {
        return None;
    };
    if ctx.level != 1 {
        return None;
    }
    let bid = ctx.call.bid()?;
    let suit = bid.strain().suit()?;
    let (min_len, min_hcp) = p.response.new_suit_1;
    // After a 1-level overcall in a major (`1C (1H) 1S`), the new major shows five: with exactly
    // four responder makes the negative double, which this rule, ranked above it, would
    // otherwise shadow completely. Over a minor overcall (`1C (1D) 1H`) it stays 4+ (SAYC; the
    // double then shows both majors).
    let over_major = [Strain::Hearts, Strain::Spades]
        .into_iter()
        .any(|m| ctx.their_suits.contains(m));
    let min_len = if ctx.competitive && over_major {
        min_len.max(MIN_NEW_SUIT_OVER_OVERCALL)
    } else {
        min_len
    };
    let constraint = HandConstraint::Atom(
        Atom::ANY
            .with_hcp(min_hcp..=37)
            .with_len(suit, min_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.5,
        rule: "new_suit_resp_1",
        explanation: expl!(
            ex,
            "1-level new suit {}: {min_len}+ cards, {min_hcp}+ hcp",
            ctx.call
        ),
    })
}

fn rule_new_suit_resp_2(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.role != Role::Responder {
        return None;
    }
    let CallKind::Bid {
        new_suit: true,
        jump,
        ..
    } = ctx.kind
    else {
        return None;
    };
    if ctx.level != 2 {
        return None;
    }
    let bid = ctx.call.bid()?;
    let suit = bid.strain().suit()?;
    let (min_len, _) = p.response.new_suit_2;
    let min_hcp = if jump >= 1 {
        p.response.jump_shift
    } else {
        p.response.new_suit_2.1
    };
    let constraint = HandConstraint::Atom(
        Atom::ANY
            .with_hcp(min_hcp..=37)
            .with_len(suit, min_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.6,
        rule: "new_suit_resp_2",
        explanation: expl!(
            ex,
            "2-level new suit {}: {min_len}+ cards, {min_hcp}+ hcp",
            ctx.call
        ),
    })
}

fn rule_resp_nt(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.role != Role::Responder {
        return None;
    }
    let CallKind::Bid { nt: true, .. } = ctx.kind else {
        return None;
    };
    let (_, hcp) = p
        .response
        .nt
        .iter()
        .find(|(level, _)| *level == ctx.level)?;
    let mut atom = Atom::ANY.with_hcp(hcp.clone());
    if ctx.level >= 2 {
        atom.shapes = ShapeSet::BALANCED;
    }
    let constraint = if ctx.level == 1 && !ctx.owner_acted {
        one_nt_response(p, ctx, atom)
    } else {
        HandConstraint::Atom(atom)
    };
    Some(Inference {
        constraint,
        confidence: 0.5,
        rule: "resp_nt",
        explanation: expl!(
            ex,
            "notrump response {}: {}-{} hcp",
            ctx.call,
            hcp.start(),
            hcp.end()
        ),
    })
}

/// Minimum length of a 1-level new major after an overcall in the other major (`1C (1H) 1S`):
/// with exactly four, responder makes the negative double (`negative_x`).
const MIN_NEW_SUIT_OVER_OVERCALL: u8 = 5;

/// Minimum support in partner's minor that a natural raise shows in place of a 1NT response
/// (SAYC `1m-2m`: 5+). A major raise needs only `response.raise.0` (3).
const MINOR_RAISE_SUPPORT: u8 = 5;

/// Responder's first-response 1NT (`resp_nt` at the 1 level; `atom` carries its HCP range)
/// denies a simple raise of partner's suit: support (`response.raise.0` in a major,
/// [`MINOR_RAISE_SUPPORT`] in a minor) together with the raise's HCP range, as SAYC writes it
/// (`1H-1NT`: 6-9, not 3+ hearts; `1C-1NT`: not 5+ clubs).
///
/// Without it the shape-free 1NT covers every simple-raise hand, so ranking it above `raise`
/// (phase 4.6: 0.5 against 0.45) left the raise with an empty natural region: after `1H P` the
/// natural policy never bid `2H`. A hand with support and more strength than a simple raise (10
/// HCP and 3 hearts after `1H`) may still respond 1NT. A 4-card major biddable at the 1 level is
/// not denied here: `new_suit_resp_1` ties with 1NT and wins by call order, so the natural
/// exclusive region already leaves it out, and the raw constraint stays the node's own (as the
/// SAYC tables write it; §8.5 measurement 1 compares raw constraints).
fn one_nt_response(p: &NaturalParams, ctx: &CallContext, atom: Atom) -> HandConstraint {
    let Some(Call::Bid(opening)) = ctx.partner_first_action else {
        return HandConstraint::Atom(atom);
    };
    let Some(suit) = opening.strain().suit() else {
        return HandConstraint::Atom(atom);
    };
    let support = if matches!(suit, Suit::Hearts | Suit::Spades) {
        p.response.raise.0
    } else {
        MINOR_RAISE_SUPPORT
    };
    let (lo, hi) = (*atom.hcp.start(), *atom.hcp.end());
    let raise_hi = *p.response.raise.1.end();
    let short = atom.clone().with_len(suit, 0..=support.saturating_sub(1));
    // The part of the 1NT range above the simple raise keeps any support.
    let stronger = raise_hi.checked_add(1).filter(|&from| from.max(lo) <= hi);
    match stronger {
        Some(from) => {
            HandConstraint::Atom(short).or(HandConstraint::Atom(atom.with_hcp(from.max(lo)..=hi)))
        }
        None => HandConstraint::Atom(short),
    }
}

/// The HCP range implied by the *original* opening bid, for rules (`rebid_own`) that need to
/// know what kind of opening is being rebid rather than always assuming the 1-level range: a
/// weak two, a preempt or a strong 2C is a fundamentally different hand.
///
/// Takes `opener_first_bid` (the owner's first [`Call::Bid`] of any strain), not
/// `opener_first_suit`: the latter skips notrump, so after `1NT-P-2D-P-2H` (a transfer
/// completion) it names `(Hearts, 2)`, which is not the opening at all. Matching on that level
/// alone would misjudge a later rebid of the transfer suit as a weak two.
///
/// Found by the hold-out measurement (D8 measurement 1, `crates/bridge-bidding/tests/
/// natural_metrics.rs`): `rule_rebid_own` used to compare every own-suit rebid against
/// `opening_hcp` (12..=21) unconditionally, even when the opening itself was a weak two or a
/// preempt. `weak_two` and `opening_hcp` are disjoint ranges, so every weak-two rebid-own node
/// measured recall and precision of exactly 0, not merely low -- the constraint the generic
/// engine inferred could never be satisfied by the same hand that satisfied the real, weak
/// range at all. See `rebid_own_after_weak_two_uses_weak_two_hcp` below.
fn opening_level_hcp(p: &NaturalParams, opener_first_bid: Option<Bid>) -> RangeInclusive<u8> {
    match opener_first_bid {
        // A strong 2C opening: checked before the generic level-2 suit case below, since 2C is
        // itself a suit bid and would otherwise match it.
        Some(bid) if bid.level() == 2 && bid.strain() == Strain::Clubs => p.strong_two_c..=37,
        // A weak two (2D/2H/2S): any other suit at level 2.
        Some(bid) if bid.level() == 2 && bid.strain().suit().is_some() => p.weak_two.1.clone(),
        // A suit preempt at the 3-5 level.
        Some(bid) if (3..=5).contains(&bid.level()) && bid.strain().suit().is_some() => p
            .preempt
            .iter()
            .find(|(lvl, _, _)| *lvl == bid.level())
            .map(|(_, _, hcp)| hcp.clone())
            .unwrap_or_else(|| p.opening_hcp.clone()),
        // A 1-level suit opening, or anything else this table does not special-case: the
        // 1-level range is still the closest available default. (`rule_rebid_own` answers a
        // notrump opening with its own range before it gets here: `notrump_opening_hcp`.)
        _ => p.opening_hcp.clone(),
    }
}

fn rule_rebid_own(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    let CallKind::Bid {
        rebid_own: true,
        jump,
        ..
    } = ctx.kind
    else {
        return None;
    };
    let bid = ctx.call.bid()?;
    let suit = bid.strain().suit()?;
    if ctx.role == Role::Responder && ctx.agreed_suit != Some(suit) && corrects_partners_3nt(ctx) {
        return rule_responder_corrects_to_own_suit(p, ctx, suit, ex);
    }
    if ctx.role != Role::Opener {
        return None;
    }
    if ctx.agreed_suit == Some(suit) && opened_one_of_a_suit(ctx) {
        return Some(rule_reraise_agreed(p, ctx, suit, jump, ex));
    }
    if let Some(hcp) = notrump_opening_hcp(p, ctx.opener_first_bid) {
        return Some(rule_rebid_own_after_notrump(ctx, suit, hcp, ex));
    }
    if rebid_opened_suit(ctx, bid.strain()) && corrects_partners_3nt(ctx) {
        return Some(rule_rebid_opened_suit_over_3nt(p, ctx, suit, ex));
    }
    let jump = effective_jump(ctx, jump);
    let hcp = if jump >= 1 {
        p.rebid.jump_rebid.clone()
    } else {
        opening_level_hcp(p, ctx.opener_first_bid)
    };
    let constraint = with_correction_shape(
        ctx,
        HandConstraint::Atom(Atom::ANY.with_hcp(hcp.clone()).with_len(suit, 6..=13)),
        false,
    );
    Some(Inference {
        constraint,
        // A jump rebid (16-18) lies inside the plain rebid's range (12-21): at an equal
        // priority the cheaper call would always win and the jump would never be made.
        confidence: if jump >= 1 { 0.55 } else { 0.5 },
        rule: "rebid_own",
        explanation: expl!(
            ex,
            "rebids own suit {}: 6+ cards, {}-{} hcp",
            ctx.call,
            hcp.start(),
            hcp.end()
        ),
    })
}

/// The suit length a one-level opener shows by pulling partner's 3NT to its suit after rebidding
/// it (`1S-P-2C-P-2S-P-3NT-P-4S`): a seventh card, since the rebid already showed six
/// (docs/design/06-system.md §8.6).
pub const REBID_SUIT_PULL_LEN: u8 = 7;

/// A one-level opener that has rebid its suit pulls partner's 3NT to game in it
/// (`1S-P-2C-P-2S-P-3NT-P-4S`, `1H-P-1S-P-2H-P-3NT-P-4H`): a choice of game, not a slam move
/// (partner's 3NT chose the contract).
///
/// The rebid showed six cards (rule `rebid_own`), so partner chose 3NT knowing them, and the
/// news is a seventh: [`REBID_SUIT_PULL_LEN`]+ cards (in a minor also a singleton or a void,
/// [`with_correction_shape`]). Six cards with a short suit are not enough (the corpus check is
/// in docs/design/06-system.md §8.6). The strength:
///
/// - while the rebid limited the hand, that is, opener's only non-pass calls are the opening and
///   bids of the opened suit ([`CallContext::opener_other_calls`] is empty), what the rebid
///   showed: after a non-jump rebid a minimum (`opening_hcp.start..rebid.jump_rebid.start`,
///   12-15 with the default parameters), after a jump rebid
///   ([`CallContext::owner_jump_rebid_strains`]) `rebid.jump_rebid` (16-18);
/// - otherwise the rebid did not limit it: a later non-jump rebid after a reverse
///   (`1D-P-1S-P-2H-P-2NT-P-3D-P-3NT-P-5D`), a jump shift or a 2NT rebid is not a minimum. The
///   range of opener's strongest earlier call ([`openers_strongest_call_hcp`]): 17-21 after a
///   reverse, 19-21 after a jump shift, 18-19 after a 2NT rebid.
///
/// The ordinary level floor applies ([`corrects_partners_3nt`]).
fn rule_rebid_opened_suit_over_3nt(
    p: &NaturalParams,
    ctx: &CallContext,
    suit: Suit,
    ex: bool,
) -> Inference {
    let jump_rebid = ctx
        .owner_jump_rebid_strains
        .contains(Strain::from_suit(suit));
    let hcp = if !ctx.opener_other_calls.is_empty() {
        openers_strongest_call_hcp(p, ctx.opener_other_calls, jump_rebid)
    } else if jump_rebid {
        p.rebid.jump_rebid.clone()
    } else {
        let top = p.rebid.jump_rebid.start().saturating_sub(1);
        *p.opening_hcp.start()..=top.max(*p.opening_hcp.start())
    };
    let constraint = HandConstraint::Atom(
        Atom::ANY
            .with_hcp(hcp.clone())
            .with_len(suit, REBID_SUIT_PULL_LEN..=13),
    );
    Inference {
        constraint: with_correction_shape(ctx, constraint, false),
        confidence: 0.5,
        rule: "rebid_own",
        explanation: expl!(
            ex,
            "pulls 3NT to the rebid suit {}: {}+ cards, {}-{} hcp",
            ctx.call,
            REBID_SUIT_PULL_LEN,
            hcp.start(),
            hcp.end()
        ),
    }
}

/// The HCP range of a one-level suit opener's strongest earlier call besides its opening and its
/// bids of the opened suit ([`CallContext::opener_other_calls`]), or of a jump rebid of the
/// opened suit (`jump_rebid`): each call's range as its natural rule reads it, the one with the
/// highest minimum (the narrowest among equal minimums), within `opening_hcp`. With the default
/// parameters: a reverse 17-21, a jump shift 19-21, a 2NT rebid 18-19, a jump raise or a jump
/// rebid 16-18, a raise 12-15, a 1NT rebid 12-14, a new suit 12-18, anything else 12-21.
fn openers_strongest_call_hcp(
    p: &NaturalParams,
    calls: OpenerCalls,
    jump_rebid: bool,
) -> RangeInclusive<u8> {
    let opening = &p.opening_hcp;
    let ranges = [
        (calls.raise, p.rebid.raise.clone()),
        (calls.jump_raise, p.rebid.jump_raise.clone()),
        (calls.jump_rebid || jump_rebid, p.rebid.jump_rebid.clone()),
        (calls.reverse, p.rebid.reverse..=37),
        (calls.nt_rebid, p.rebid.nt_1.clone()),
        (calls.jump_nt_rebid, p.rebid.nt_2.clone()),
        (calls.new_suit, opener_new_suit_hcp(p, 0)),
        (calls.jump_shift, opener_new_suit_hcp(p, 1)),
        (calls.other, opening.clone()),
    ];
    let strongest = ranges
        .into_iter()
        .filter(|(made, _)| *made)
        .map(|(_, hcp)| hcp)
        .max_by(|a, b| a.start().cmp(b.start()).then(b.end().cmp(a.end())))
        .unwrap_or_else(|| opening.clone());
    let from = (*strongest.start()).max(*opening.start());
    from..=(*strongest.end()).min(*opening.end()).max(from)
}

/// The minimum length a notrump opener shows by bidding again a suit it bid after the opening
/// (rule `rebid_own`, docs/design/06-system.md §8.3).
pub const NT_OPENER_SUIT_MIN_LEN: u8 = 3;

/// The HCP range of `owner`'s notrump opening (`nt[L]` for the opening's level `L`), or `None`
/// when the opening was not notrump or its level has no range.
fn notrump_opening_hcp(
    p: &NaturalParams,
    opener_first_bid: Option<Bid>,
) -> Option<RangeInclusive<u8>> {
    let bid = opener_first_bid.filter(|b| b.strain() == Strain::NoTrump)?;
    p.nt.iter()
        .find(|(level, _)| *level == bid.level())
        .map(|(_, hcp)| hcp.clone())
}

/// A notrump opener bids again a suit it bid after the opening (a transfer completion or an
/// answer to Stayman): `1NT-P-2H-P-2S-P-3NT-P-4S`, `1NT-P-2C-P-2H-P-3H-P-4H`. The opening
/// already showed a balanced hand of the notrump range, so the bid shows support for partner or
/// a fourth card, not a long suit: balanced, `nt[L]`, [`NT_OPENER_SUIT_MIN_LEN`]+ cards. The
/// six-card rebid of a suit opening would be a hand the notrump opening excludes.
fn rule_rebid_own_after_notrump(
    ctx: &CallContext,
    suit: Suit,
    hcp: RangeInclusive<u8>,
    ex: bool,
) -> Inference {
    Inference {
        constraint: with_correction_shape(
            ctx,
            HandConstraint::Atom(
                Atom {
                    shapes: ShapeSet::BALANCED,
                    ..Atom::ANY
                }
                .with_hcp(hcp.clone())
                .with_len(suit, NT_OPENER_SUIT_MIN_LEN..=13),
            ),
            true,
        ),
        confidence: 0.5,
        rule: "rebid_own",
        explanation: expl!(
            ex,
            "notrump opener bids {} again: balanced, {}+ cards, {}-{} hcp",
            ctx.call,
            NT_OPENER_SUIT_MIN_LEN,
            hcp.start(),
            hcp.end()
        ),
    }
}

/// `owner` opened one of a suit (the rebid rules below describe rebids after such an opening,
/// not after a notrump, strong 2C or preempt opening).
fn opened_one_of_a_suit(ctx: &CallContext) -> bool {
    ctx.opener_first_bid
        .is_some_and(|b| b.level() == 1 && b.strain() != Strain::NoTrump)
}

/// Opener bids again a suit it bid first and partner then supported (`1H-P-2H-P-3H`): not a
/// raise of partner's suit, and not the 6-card rebid of an unsupported suit either. A non-jump
/// re-raise without competition is a game try (`rebid.jump_rebid`); in competition it is
/// merely competitive (`opening_hcp`); a jump (to game) shows `rebid.jump_rebid`'s minimum or
/// more; a correction of partner's 3NT to game in the suit ([`corrects_partners_3nt`]) is a
/// choice of game, any opening (`opening_hcp`). Length is the opening's own minimum for the
/// opened suit, 4 for a second suit.
fn rule_reraise_agreed(
    p: &NaturalParams,
    ctx: &CallContext,
    suit: Suit,
    jump: u8,
    ex: bool,
) -> Inference {
    let min_len = match ctx.opener_first_bid {
        Some(b) if b.strain() == Strain::from_suit(suit) => {
            if b.strain().is_major() {
                p.open_1major_len
            } else {
                p.open_1m_len
            }
        }
        _ => 4,
    };
    let hcp = if corrects_partners_3nt(ctx) {
        p.opening_hcp.clone()
    } else if jump >= 1 {
        *p.rebid.jump_rebid.start()..=*p.opening_hcp.end()
    } else if ctx.competitive {
        p.opening_hcp.clone()
    } else {
        p.rebid.jump_rebid.clone()
    };
    Inference {
        constraint: with_correction_shape(
            ctx,
            HandConstraint::Atom(Atom::ANY.with_hcp(hcp.clone()).with_len(suit, min_len..=13)),
            true,
        ),
        confidence: 0.5,
        rule: "rebid_own",
        explanation: expl!(
            ex,
            "re-raises agreed suit {}: {}+ cards, {}-{} hcp",
            ctx.call,
            min_len,
            hcp.start(),
            hcp.end()
        ),
    }
}

fn rule_reverse(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    let CallKind::Bid { reverse: true, .. } = ctx.kind else {
        return None;
    };
    let bid = ctx.call.bid()?;
    let second_suit = bid.strain().suit()?;
    let second = HandConstraint::Atom(Atom::ANY.with_len(second_suit, 4..=13));
    // The suit opener actually opened (not the side-level `our_suits`, which by now also
    // contains responder's suit(s)): a reverse requires 5+ of *that* suit specifically.
    let (first_suit, _) = ctx.opener_first_suit?;
    let first = HandConstraint::Atom(Atom::ANY.with_len(first_suit, 5..=13));
    let constraint = HandConstraint::Atom(Atom::ANY.with_hcp(p.rebid.reverse..=37))
        .and(first)
        .and(second);
    Some(Inference {
        constraint,
        confidence: 0.55,
        rule: "reverse",
        explanation: expl!(
            ex,
            "reverse into {}: {}+ hcp, 5+/4+ shape",
            ctx.call,
            p.rebid.reverse
        ),
    })
}

/// `true` when this is opener's rebid proper: opener's last call is still the opening (opener
/// has not called since, not even a pass), partner has answered it with a non-pass call (a
/// response or a negative double), and the opponents have not bid notrump.
///
/// The notrump ranges of `rule_rebid_nt` describe that one call. Opener's notrump at a later
/// turn (`1D-P-1S-P-1NT-P-2S-P-2NT`, after the balanced range is already shown), after an
/// earlier pass (`1C-P-1D-(1S)-P-(2S)-P-(P)-2NT`), when partner has not responded
/// (`1C-(1D)-P-(1S)-1NT`, a reopening that SAYC does not make with a minimum), or over the
/// opponents' own notrump (`1C-(1NT)-2H-(P)-2NT`) is not a range rebid, and no rule fires
/// there (docs/design/06-system.md §8.3).
fn is_openers_rebid(ctx: &CallContext) -> bool {
    ctx.opener_first_bid.is_some()
        && ctx.owner_last == ctx.opener_first_bid.map(Call::Bid)
        && ctx.partner_actions >= 1
        && !ctx.their_suits.contains(Strain::NoTrump)
}

/// Opener's notrump rebid after a 1-of-a-suit opening: the cheapest notrump shows
/// `rebid.nt_1`, a jump `rebid.nt_2`, both balanced. Higher notrump rebids, and notrump once a
/// suit is agreed (`1H-P-2H-P-2NT` is a game try, not a range rebid), are left to `fallback`;
/// so is notrump at any turn but the rebid itself ([`is_openers_rebid`]).
fn rule_rebid_nt(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.role != Role::Opener
        || !opened_one_of_a_suit(ctx)
        || ctx.level > 2
        || ctx.agreed_suit.is_some()
        || !is_openers_rebid(ctx)
    {
        return None;
    }
    let CallKind::Bid { nt: true, jump, .. } = ctx.kind else {
        return None;
    };
    let hcp = match jump {
        0 => p.rebid.nt_1.clone(),
        1 => p.rebid.nt_2.clone(),
        _ => return None,
    };
    Some(Inference {
        constraint: HandConstraint::Atom(
            Atom {
                shapes: ShapeSet::BALANCED,
                ..Atom::ANY
            }
            .with_hcp(hcp.clone()),
        ),
        confidence: 0.45,
        rule: "rebid_nt",
        explanation: expl!(
            ex,
            "notrump rebid {}: balanced, {}-{} hcp",
            ctx.call,
            hcp.start(),
            hcp.end()
        ),
    })
}

/// The HCP range of opener's new suit (`rule_rebid_new_suit`): from the opening minimum up to
/// `rebid.jump_raise`'s maximum; a jump shift (`jump >= 1`) shows more than `rebid.jump_rebid`.
fn opener_new_suit_hcp(p: &NaturalParams, jump: u8) -> RangeInclusive<u8> {
    if jump >= 1 {
        p.rebid.jump_rebid.end().saturating_add(1)..=37
    } else {
        *p.opening_hcp.start()..=*p.rebid.jump_raise.end()
    }
}

/// Opener's new suit that is not a reverse (`rule_reverse` runs first), after a 1-of-a-suit
/// opening: 4+ cards, [`opener_new_suit_hcp`].
fn rule_rebid_new_suit(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.role != Role::Opener || !opened_one_of_a_suit(ctx) {
        return None;
    }
    let CallKind::Bid {
        new_suit: true,
        jump,
        ..
    } = ctx.kind
    else {
        return None;
    };
    let suit = ctx.call.bid()?.strain().suit()?;
    let hcp = opener_new_suit_hcp(p, jump);
    Some(Inference {
        constraint: HandConstraint::Atom(Atom::ANY.with_hcp(hcp.clone()).with_len(suit, 4..=13)),
        confidence: 0.35,
        rule: "rebid_new_suit",
        explanation: expl!(
            ex,
            "new suit rebid {}: 4+ cards, {}-{} hcp",
            ctx.call,
            hcp.start(),
            hcp.end()
        ),
    })
}

/// The advancer's first action in a new suit (not a jump) over partner's overcall:
/// `advance.new_suit`. An answer to partner's takeout double is a different animal (it can be a
/// forced bid with nothing) and is left to `fallback`.
fn rule_advance_new_suit(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.role != Role::Advancer
        || ctx.owner_acted
        || !matches!(ctx.partner_first_action, Some(Call::Bid(_)))
    {
        return None;
    }
    let CallKind::Bid {
        new_suit: true,
        jump: 0,
        ..
    } = ctx.kind
    else {
        return None;
    };
    let suit = ctx.call.bid()?.strain().suit()?;
    let (min_len, min_hcp) = p.advance.new_suit;
    Some(Inference {
        constraint: HandConstraint::Atom(
            Atom::ANY
                .with_hcp(min_hcp..=37)
                .with_len(suit, min_len..=13),
        ),
        confidence: 0.5,
        rule: "advance_new_suit",
        explanation: expl!(
            ex,
            "new suit advance {}: {min_len}+ cards, {min_hcp}+ hcp",
            ctx.call
        ),
    })
}

fn rule_cue(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    let CallKind::Bid { cue: true, .. } = ctx.kind else {
        return None;
    };
    // The game-forcing formula only describes a cue bid by the opening side (opener or
    // responder cue-bidding the opponents' suit). An advancer's cue is the standard "limit raise
    // or better" (`advance.cue`) whatever partner's overcall showed -- `25 - partner_min` would
    // turn it into 17+ -- and an overcaller's or balancer's cue (Michaels-style) has no partner
    // bid to build on at all.
    let min_hcp = match (&ctx.partner_constraint, ctx.role) {
        (Some(c), Role::Opener | Role::Responder) => {
            GF_TOTAL.saturating_sub(*c.hcp_range().start())
        }
        _ => p.advance.cue,
    };
    Some(Inference {
        constraint: HandConstraint::Atom(Atom::ANY.with_hcp(min_hcp..=37)),
        confidence: 0.3,
        rule: "cue",
        explanation: expl!(ex, "cue bid {}: {min_hcp}+ hcp, artificial", ctx.call),
    })
}

fn rule_pass_forcing(ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.call != Call::Pass || !ctx.forcing_situation {
        return None;
    }
    Some(Inference {
        constraint: HandConstraint::Atom(Atom::ANY.with_hcp(0..=0)),
        confidence: 0.1,
        rule: "pass_forcing",
        explanation: expl!(
            ex,
            "pass over a forcing call: contradictory, near-unsatisfiable"
        ),
    })
}

fn rule_pass_default(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.call != Call::Pass {
        return None;
    }
    // An unbounded pass is satisfied by every hand, so it shadows every rule it ties or
    // outranks (`choose_bid` breaks equal priorities by call order, where `Pass` comes first).
    // At 0.3 (phase 4.6, tuned on the corpus tune split; 06-system.md §8.6) it ranks below every
    // bid rule except `cue` and `penalty_x`, which it ties and therefore shadows: the natural
    // policy passes rather than cue-bids or doubles for penalty when nothing else fits. A
    // limited pass is disjoint from the bids it declines, so it keeps 0.4.
    let (constraint, confidence) = match pass_default_max_hcp(p, ctx) {
        Some(hi) => (HandConstraint::Atom(Atom::ANY.with_hcp(0..=hi)), 0.4),
        None => (HandConstraint::ANY, 0.3),
    };
    Some(Inference {
        constraint,
        confidence,
        rule: "pass_default",
        explanation: expl!(ex, "pass: below the threshold to act"),
    })
}

/// The HCP ceiling of a limited pass (§8.3 `pass_default`), or `None` for an unbounded one.
///
/// Only the responder's or advancer's *first* action over partner's opening or overcall (their
/// only non-pass call so far) is limited: a later pass by a player who has already bid, or a
/// pass after partner has bid again, says nothing about strength. The ceiling depends on what
/// partner bid:
///
/// - a 1-level suit opening: `response.new_suit_1.1 − 1` (below a 1-level response);
/// - a 1NT/2NT opening: `GF_TOTAL − nt[L].end − 1` (no game opposite a maximum);
/// - a simple (non-jump) suit overcall at the 1 or 2 level: `advance.raise.1.start − 1`;
/// - a notrump overcall: `GF_TOTAL − nt_overcall.end − 1`;
/// - anything else (weak two, preempt, strong 2C, a double, a jump overcall): unbounded.
fn pass_default_max_hcp(p: &NaturalParams, ctx: &CallContext) -> Option<u8> {
    if ctx.owner_acted || ctx.partner_actions != 1 {
        return None;
    }
    let Some(Call::Bid(partner_bid)) = ctx.partner_first_action else {
        return None;
    };
    let level = partner_bid.level();
    let nt = partner_bid.strain() == Strain::NoTrump;
    match ctx.role {
        Role::Responder if !nt && level == 1 => Some(p.response.new_suit_1.1.saturating_sub(1)),
        Role::Responder if nt && level <= 2 => {
            p.nt.iter()
                .find(|(l, _)| *l == level)
                .map(|(_, hcp)| GF_TOTAL.saturating_sub(hcp.end().saturating_add(1)))
        }
        Role::Advancer if ctx.partner_first_jump == 0 && level <= 2 => {
            if nt {
                Some(GF_TOTAL.saturating_sub(p.nt_overcall.end().saturating_add(1)))
            } else {
                Some(p.advance.raise.1.start().saturating_sub(1))
            }
        }
        _ => None,
    }
}

/// The rule name of the "no rule matched" result.
pub const FALLBACK_RULE: &str = "fallback";

/// The rule name of the natural implicit `Pass` ([`NaturalInference::implicit_pass`]).
pub const IMPLICIT_PASS_RULE: &str = "implicit_pass";

/// The explanation of the natural implicit `Pass`.
pub const IMPLICIT_PASS_EXPLANATION: &str = "pass: no natural call fits this hand";
/// The confidence of the "no rule matched" result.
const FALLBACK_CONFIDENCE: f32 = 0.05;

fn rule_fallback() -> Inference {
    Inference {
        constraint: HandConstraint::ANY,
        confidence: FALLBACK_CONFIDENCE,
        rule: FALLBACK_RULE,
        explanation: "no rule matched".to_string(),
    }
}

/// Shapes by exact length per suit (`[suit][len]`), built once: a suit-length range is then a
/// union of at most 14 precomputed sets instead of a scan over all 560 shapes.
fn exact_len_sets() -> &'static [[ShapeSet; 14]; 4] {
    static SETS: std::sync::OnceLock<[[ShapeSet; 14]; 4]> = std::sync::OnceLock::new();
    SETS.get_or_init(|| {
        let mut sets = [[ShapeSet::EMPTY; 14]; 4];
        for suit in Suit::ALL {
            for len in 0..14u8 {
                sets[suit as usize][len as usize] = ShapeSet::from_suit_len(suit, len, len);
            }
        }
        sets
    })
}

/// `ShapeSet::from_suit_len(suit, lo, hi)` from the cached per-length sets.
fn suit_len_set(suit: Suit, range: &RangeInclusive<u8>) -> ShapeSet {
    let sets = &exact_len_sets()[suit as usize];
    let hi = (*range.end()).min(13);
    (*range.start()..=hi).fold(ShapeSet::EMPTY, |acc, len| acc.union(sets[len as usize]))
}

/// [`Atom::with_suit_len`] through the cached per-length sets (same result, no 560-shape scan).
trait WithLen {
    fn with_len(self, suit: Suit, range: RangeInclusive<u8>) -> Atom;
}

impl WithLen for Atom {
    fn with_len(mut self, suit: Suit, range: RangeInclusive<u8>) -> Atom {
        self.shapes = self.shapes.intersect(suit_len_set(suit, &range));
        self
    }
}

/// A crude stopper heuristic for `suit`: the ace alone, the king with one low card, or the queen
/// with two low cards. Used only by `nt_overcall`; `bridge-constraint` has no dedicated "stopper"
/// primitive yet.
fn stopper(suit: Suit) -> HandConstraint {
    let ace = HandConstraint::Atom(Atom::ANY.with_cards(CardRequirement::in_suit(
        suit,
        Holding::EMPTY.with(Rank::Ace),
        1..=1,
    )));
    let king = HandConstraint::Atom(
        Atom::ANY
            .with_cards(CardRequirement::in_suit(
                suit,
                Holding::EMPTY.with(Rank::King),
                1..=1,
            ))
            .with_len(suit, 2..=13),
    );
    let queen = HandConstraint::Atom(
        Atom::ANY
            .with_cards(CardRequirement::in_suit(
                suit,
                Holding::EMPTY.with(Rank::Queen),
                1..=1,
            ))
            .with_len(suit, 3..=13),
    );
    ace.or(king).or(queen)
}

/// `suits` each meeting `min_len`, relaxed to "at least two of them" once there are three or
/// more (§8.3, `takeout_x`).
fn at_least_two_of(suits: &[Suit], min_len: u8) -> HandConstraint {
    if suits.len() <= 2 {
        return suits
            .iter()
            .map(|&s| HandConstraint::Atom(Atom::ANY.with_len(s, min_len..=13)))
            .reduce(HandConstraint::and)
            .unwrap_or(HandConstraint::ANY);
    }
    let mut combos = Vec::new();
    for i in 0..suits.len() {
        for j in (i + 1)..suits.len() {
            let both = HandConstraint::Atom(Atom::ANY.with_len(suits[i], min_len..=13)).and(
                HandConstraint::Atom(Atom::ANY.with_len(suits[j], min_len..=13)),
            );
            combos.push(both);
        }
    }
    combos
        .into_iter()
        .reduce(HandConstraint::or)
        .unwrap_or(HandConstraint::ANY)
}

impl Default for NaturalInference {
    fn default() -> NaturalInference {
        NaturalInference::new(NaturalParams::default())
    }
}

#[cfg(test)]
mod tests {
    use bridge_core::Vulnerability;

    use super::*;

    #[test]
    fn infer_batch_and_ranked_candidates_agree_with_infer() {
        let engine = NaturalInference::default();
        let auction = Auction::from_calls(
            Seat::North,
            Vulnerability::None,
            [Call::Bid(Bid::new(1, Strain::Hearts).unwrap()), Call::Pass],
        )
        .unwrap();
        let owner = auction.next_seat();
        let partner = PartnerContext::default();
        let calls: Vec<Call> = auction.legal_calls().collect();
        let batch = engine.infer_batch(&auction, owner, &partner, &calls);
        assert_eq!(batch.len(), calls.len());
        for (call, got) in calls.iter().zip(&batch) {
            let ctx = classify(&auction.with(*call).unwrap(), auction.len(), owner);
            let want = engine.infer(&ctx);
            assert_eq!(got.call, *call);
            assert_eq!(got.rule, want.rule);
            assert_eq!(got.confidence, want.confidence);
            assert_eq!(
                format!("{:?}", got.constraint),
                format!("{:?}", want.constraint)
            );
        }
        let ranked = engine.ranked_candidates(&auction, owner, &partner, crate::TieBreak::RowOrder);
        let mut expected = engine.candidates(&auction, owner);
        // `candidates` lists legal calls in call-index order; a stable sort by priority
        // descending is the natural rank order under `RowOrder`.
        expected.sort_by_key(|c| std::cmp::Reverse(c.2));
        assert_eq!(
            ranked
                .iter()
                .map(|c| (c.call, c.priority()))
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|(c, _, p)| (*c, *p))
                .collect::<Vec<_>>()
        );
        assert!(!ranked.is_empty());
    }

    #[test]
    fn implicit_pass_is_the_complement_of_the_ranked_candidates() {
        let engine = NaturalInference::default();
        // 1S P: responder has natural candidates, a limited Pass (pass_default, 0-5) among them.
        let a = Auction::from_calls(
            Seat::North,
            Vulnerability::None,
            [Call::Bid(Bid::new(1, Strain::Spades).unwrap()), Call::Pass],
        )
        .unwrap();
        let ranked = engine.ranked_candidates(
            &a,
            Seat::South,
            &PartnerContext::default(),
            crate::TieBreak::RowOrder,
        );
        assert!(ranked.iter().any(|c| c.call == Call::Pass));
        let without_pass: Vec<NaturalCandidate> = ranked
            .iter()
            .filter(|c| c.call != Call::Pass)
            .cloned()
            .collect();
        for set in [&ranked, &without_pass] {
            let pass = NaturalInference::implicit_pass(set);
            assert_eq!(pass.call, Call::Pass);
            assert_eq!(pass.rule, IMPLICIT_PASS_RULE);
            assert!(!pass.is_fallback());
            let mut seed = 0x1a55u64;
            for _ in 0..2000 {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let hand = test_hand(seed);
                let any = set.iter().any(|c| c.constraint.satisfies(hand));
                assert_eq!(pass.constraint.satisfies(hand), !any);
            }
        }
        // The listed limited Pass does not suppress the implicit one: after 1H P the 17-count
        // AK2.32.AQ32.KJ32 fits no natural call (pass_default is 0-5 there), and passes.
        let a = Auction::from_calls(
            Seat::North,
            Vulnerability::None,
            [Call::Bid(Bid::new(1, Strain::Hearts).unwrap()), Call::Pass],
        )
        .unwrap();
        let ranked = engine.ranked_candidates(
            &a,
            Seat::South,
            &PartnerContext::default(),
            crate::TieBreak::RowOrder,
        );
        assert!(ranked.iter().any(|c| c.call == Call::Pass));
        let strong = bridge_core::Hand::from_holdings(
            "KJ32".parse().unwrap(),
            "AQ32".parse().unwrap(),
            "32".parse().unwrap(),
            "AK2".parse().unwrap(),
        );
        assert!(!ranked.iter().any(|c| c.constraint.satisfies(strong)));
        assert!(
            NaturalInference::implicit_pass(&ranked)
                .constraint
                .satisfies(strong)
        );
        let none = NaturalInference::implicit_pass(&[]);
        assert!(matches!(none.constraint, HandConstraint::Atom(ref a) if *a == Atom::ANY));
    }

    /// A pseudo-random 13-card hand from `seed` (partial Fisher-Yates on a splitmix stream).
    fn test_hand(seed: u64) -> bridge_core::Hand {
        let mut z = seed;
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        let mut cards: Vec<u8> = (0..52).collect();
        let mut hand = bridge_core::Hand::EMPTY;
        for i in 0..13 {
            let j = i + (next() % (52 - i as u64)) as usize;
            cards.swap(i, j);
            hand = hand.with(bridge_core::Card::from_index(cards[i]).unwrap());
        }
        hand
    }

    #[test]
    fn level_floor_is_standard_by_default_and_none_disables_it() {
        assert_eq!(LevelFloor::default(), LevelFloor::STANDARD);
        assert_eq!(NaturalParams::default().level_floor, LevelFloor::STANDARD);
        let floor = LevelFloor::STANDARD;
        assert_eq!(floor.combined(4, false), 22);
        assert_eq!(floor.combined(3, true), 24);
        assert_eq!(floor.combined(8, true), 0);
        // 1H P 2H P 3H: opener invites after a raise, a level-3 continuation.
        let calls = [
            Call::Bid(Bid::new(1, Strain::Hearts).unwrap()),
            Call::Pass,
            Call::Bid(Bid::new(2, Strain::Hearts).unwrap()),
            Call::Pass,
            Call::Bid(Bid::new(3, Strain::Hearts).unwrap()),
        ];
        let auction = Auction::from_calls(Seat::North, Vulnerability::None, calls).unwrap();
        let mut ctx = classify(&auction, 4, Seat::North);
        ctx.partner_constraint = Some(HandConstraint::Atom(Atom::ANY.with_hcp(0..=9)));
        let plain = NaturalInference::new(NaturalParams {
            level_floor: LevelFloor::NONE,
            ..NaturalParams::default()
        })
        .infer(&ctx);
        let floored = NaturalInference::default().infer(&ctx);
        assert_eq!(plain.rule, floored.rule);
        assert_ne!(plain.rule, "fallback");
        // 18 combined minus partner's 0: at least 18 of our own.
        assert!(*floored.constraint.hcp_range().start() >= 18);
        assert!(*plain.constraint.hcp_range().start() < 18);
    }

    /// Regression for the hold-out measurement's finding (see `opening_level_hcp`'s doc comment):
    /// rebidding one's own suit after a *weak two* opening must be judged against `weak_two`'s
    /// HCP range, not `opening_hcp` (12..=21), which is disjoint from it.
    #[test]
    fn rebid_own_after_weak_two_uses_weak_two_hcp() {
        let engine = NaturalInference::default();
        // 2H (weak two) - P - 2NT (feature ask) - P - 3H (opener rebids own suit, no jump).
        let auction = Auction::from_calls(
            Seat::North,
            Vulnerability::None,
            [
                Call::Bid(Bid::new(2, Strain::Hearts).unwrap()),
                Call::Pass,
                Call::Bid(Bid::new(2, Strain::NoTrump).unwrap()),
                Call::Pass,
                Call::Bid(Bid::new(3, Strain::Hearts).unwrap()),
            ],
        )
        .unwrap();
        let ctx = classify(&auction, 4, Seat::North);
        assert_eq!(ctx.role, Role::Opener);
        assert!(matches!(
            ctx.kind,
            CallKind::Bid {
                rebid_own: true,
                jump: 0,
                ..
            }
        ));

        let inf = engine.infer(&ctx);
        assert_eq!(inf.rule, "rebid_own");
        let params = NaturalParams::default();
        assert_eq!(inf.constraint.hcp_range(), params.weak_two.1);
        assert_ne!(inf.constraint.hcp_range(), params.opening_hcp);
    }

    /// Same shape, but after a preempt: the 3-level table's hcp applies, not `opening_hcp`
    /// either.
    #[test]
    fn rebid_own_after_preempt_uses_preempt_hcp() {
        let engine = NaturalInference::default();
        // 3H (preempt) - P - 3NT (asking) - P - 4H (opener rebids own suit, no jump).
        let auction = Auction::from_calls(
            Seat::North,
            Vulnerability::None,
            [
                Call::Bid(Bid::new(3, Strain::Hearts).unwrap()),
                Call::Pass,
                Call::Bid(Bid::new(3, Strain::NoTrump).unwrap()),
                Call::Pass,
                Call::Bid(Bid::new(4, Strain::Hearts).unwrap()),
            ],
        )
        .unwrap();
        let ctx = classify(&auction, 4, Seat::North);
        let inf = engine.infer(&ctx);
        assert_eq!(inf.rule, "rebid_own");
        let params = NaturalParams::default();
        let (_, _, expected_hcp) = params
            .preempt
            .iter()
            .find(|(level, _, _)| *level == 3)
            .expect("NaturalParams::default() has a 3-level preempt entry");
        assert_eq!(inf.constraint.hcp_range(), *expected_hcp);
    }

    /// Regression: a rebid of the suit reached via a Jacoby transfer completion must be judged
    /// against the 1NT opening's own strength, not against `weak_two` -- `opener_first_suit`
    /// names the completion (`2H` at level 2), which used to be mistaken for a weak-two opening
    /// by level alone (see `opening_level_hcp`'s doc comment). It was first judged against
    /// `opening_hcp`; since phase 4 lane N it is the notrump range itself, balanced, with three
    /// cards or more (`rule_rebid_own_after_notrump`).
    #[test]
    fn rebid_own_after_nt_transfer_completion_uses_the_notrump_range() {
        let engine = NaturalInference::default();
        // 1NT - P - 2D (transfer) - P - 2H (completion) - P - 3NT (asking) - P - 4H (opener
        // rebids the transfer suit, no jump).
        let auction = Auction::from_calls(
            Seat::North,
            Vulnerability::None,
            [
                Call::Bid(Bid::new(1, Strain::NoTrump).unwrap()),
                Call::Pass,
                Call::Bid(Bid::new(2, Strain::Diamonds).unwrap()),
                Call::Pass,
                Call::Bid(Bid::new(2, Strain::Hearts).unwrap()),
                Call::Pass,
                Call::Bid(Bid::new(3, Strain::NoTrump).unwrap()),
                Call::Pass,
                Call::Bid(Bid::new(4, Strain::Hearts).unwrap()),
            ],
        )
        .unwrap();
        let ctx = classify(&auction, 8, Seat::North);
        assert_eq!(ctx.role, Role::Opener);
        assert_eq!(ctx.opener_first_suit, Some((Suit::Hearts, 2)));
        assert!(matches!(
            ctx.kind,
            CallKind::Bid {
                rebid_own: true,
                jump: 0,
                ..
            }
        ));

        let inf = engine.infer(&ctx);
        assert_eq!(inf.rule, "rebid_own");
        let params = NaturalParams::default();
        assert_eq!(inf.constraint.hcp_range(), params.nt[0].1);
        assert_ne!(inf.constraint.hcp_range(), params.weak_two.1);
        assert_eq!(
            inf.constraint.suit_len(Suit::Hearts),
            NT_OPENER_SUIT_MIN_LEN..=5
        );
    }

    /// Regression: a rebid of opener's own suit after a strong, artificial 2C opening must be
    /// judged against `strong_two_c`, not `weak_two` -- `owner_first_suit` records `(Clubs, 2)`
    /// for the 2C opening itself (it is a suit bid), which used to be mistaken for a weak-two
    /// opening by level alone (see `opening_level_hcp`'s doc comment).
    #[test]
    fn rebid_own_after_strong_2c_uses_strong_two_c_hcp() {
        let engine = NaturalInference::default();
        // 2C (strong) - P - 2D (waiting) - P - 3C (opener rebids own suit, no jump).
        let auction = Auction::from_calls(
            Seat::North,
            Vulnerability::None,
            [
                Call::Bid(Bid::new(2, Strain::Clubs).unwrap()),
                Call::Pass,
                Call::Bid(Bid::new(2, Strain::Diamonds).unwrap()),
                Call::Pass,
                Call::Bid(Bid::new(3, Strain::Clubs).unwrap()),
            ],
        )
        .unwrap();
        let ctx = classify(&auction, 4, Seat::North);
        assert_eq!(ctx.role, Role::Opener);
        assert_eq!(ctx.opener_first_suit, Some((Suit::Clubs, 2)));
        assert!(matches!(
            ctx.kind,
            CallKind::Bid {
                rebid_own: true,
                jump: 0,
                ..
            }
        ));

        let inf = engine.infer(&ctx);
        assert_eq!(inf.rule, "rebid_own");
        let params = NaturalParams::default();
        assert_eq!(inf.constraint.hcp_range(), params.strong_two_c..=37);
        assert_ne!(inf.constraint.hcp_range(), params.weak_two.1);
    }

    /// The slam floor of a bid that overrides partner's game is the six level's combined target
    /// in the bid's own denomination: 31 in a suit, 32 in notrump (no default rule describes a
    /// notrump bid past game, so this checks `apply_level_floor` directly).
    #[test]
    fn slam_floor_uses_the_six_level_target_of_the_denomination() {
        let calls = |s: &str| -> Vec<Call> {
            s.split_whitespace()
                .map(|c| c.parse().expect("a call"))
                .collect()
        };
        let partner = HandConstraint::Atom(Atom::ANY.with_hcp(15..=17));
        let any = || Inference {
            constraint: HandConstraint::ANY,
            confidence: 0.5,
            rule: "test",
            explanation: String::new(),
        };
        for (auction, want) in [
            ("1H P 3NT P 4NT", 32 - 15),
            ("1H P 3NT P 4C", 31 - 15),
            // A suit nobody on our side has bid is not a correction of partner's 3NT.
            ("1H P 3NT P 5C", 31 - 15),
            // Opener's own suit at game is (the ordinary four-level target, 22).
            ("1H P 3NT P 4H", 22 - 15),
            // An ask answered or followed up: the ordinary target (5 level: 26, 6 level NT: 32).
            ("1H P 3NT P 4NT P 5D P 5H", 26 - 15),
            ("1H P 3NT P 4NT P 5D P 6NT", 32 - 15),
        ] {
            let a = Auction::from_calls(Seat::North, Vulnerability::None, calls(auction)).unwrap();
            let index = a.len() - 1;
            let mut ctx = classify(&a, index, a.seat_at(index));
            ctx.partner_constraint = Some(partner.clone());
            let inf = apply_level_floor(&LevelFloor::STANDARD, &ctx, any(), false);
            assert_eq!(*inf.constraint.hcp_range().start(), want, "{auction}");
        }
    }

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
