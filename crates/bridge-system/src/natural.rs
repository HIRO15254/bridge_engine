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
    vul: (bool, bool),
    competitive: bool,
    partner_last: Option<Call>,
    agreed_suit: Option<Suit>,
    last_bid: Option<Bid>,
    owner_acted: bool,
    partner_actions: u8,
    partner_first: Option<(usize, Call)>,
    partner_first_jump: u8,
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
        for (i, c) in history.iter().enumerate() {
            if auction.seat_at(i) == owner.partner() && *c != Call::Pass {
                partner_actions = partner_actions.saturating_add(1);
                if partner_first.is_none() {
                    partner_first = Some((i, *c));
                }
            }
        }
        let partner_first_jump = match partner_first {
            Some((i, Call::Bid(b))) => {
                let before = history[..i].iter().rev().find_map(|c| c.bid());
                b.level().saturating_sub(minimal_level(before, b.strain()))
            }
            _ => 0,
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
            vul,
            competitive,
            partner_last,
            agreed_suit,
            last_bid,
            owner_acted,
            partner_actions,
            partner_first,
            partner_first_jump,
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

/// Applies `floor` to a natural bid's inference (see [`LevelFloor`]): a continuation bid (the
/// caller or partner has acted) at a level with a non-zero combined target, with a known
/// `partner_constraint`, also requires `own HCP >= combined - partner's minimum HCP`.
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
    let combined = floor.combined(bid.level(), bid.strain() == Strain::NoTrump);
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

fn rule_overcall(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if !is_first_overcall(ctx) {
        return None;
    }
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
    let (min_len, hcp) = if bid.level() == 1 {
        &p.overcall[0]
    } else {
        &p.overcall[1]
    };
    let hcp = opener_or_balancer_hcp(p, ctx.role, hcp.clone());
    let constraint = HandConstraint::Atom(
        Atom::ANY
            .with_hcp(hcp.clone())
            .with_len(suit, *min_len..=13),
    );
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
    let (min_len, hcp) = &p.overcall[2];
    let hcp = opener_or_balancer_hcp(p, ctx.role, hcp.clone());
    let constraint = HandConstraint::Atom(
        Atom::ANY
            .with_hcp(hcp.clone())
            .with_len(suit, *min_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.5,
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

fn rule_takeout_x(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.call != Call::Double {
        return None;
    }
    let CallKind::Double(DoubleKind::Takeout) = ctx.kind else {
        return None;
    };
    let their_suit = ctx.last_bid.and_then(|b| b.strain().suit());
    let (min_hcp, their_max, unbid_min) = p.takeout_double;
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

fn rule_negative_x(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.call != Call::Double {
        return None;
    }
    let CallKind::Double(DoubleKind::Negative) = ctx.kind else {
        return None;
    };
    let their_level = ctx.last_bid.map(|b| b.level()).unwrap_or(1);
    let min_hcp = p
        .response
        .new_suit_1
        .1
        .saturating_add(2u8.saturating_mul(their_level.saturating_sub(1)));
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
    let (min_len, hcp) = match ctx.role {
        Role::Responder if bid.level() >= 4 => (p.response.raise.0, 13..=37),
        Role::Responder if jump >= 1 => (p.response.jump_raise.0, p.response.jump_raise.1.clone()),
        Role::Responder => (p.response.raise.0, p.response.raise.1.clone()),
        Role::Opener if jump >= 1 => (p.response.raise.0, p.rebid.jump_raise.clone()),
        Role::Opener => (p.response.raise.0, p.rebid.raise.clone()),
        _ => (p.advance.raise.0, p.advance.raise.1.clone()),
    };
    let constraint =
        HandConstraint::Atom(Atom::ANY.with_hcp(hcp.clone()).with_len(suit, min_len..=13));
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
        // A 1-level opening (suit or NT), a notrump opening at any level, or anything else this
        // table does not special-case: the 1-level range is still the closest available default.
        _ => p.opening_hcp.clone(),
    }
}

fn rule_rebid_own(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.role != Role::Opener {
        return None;
    }
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
    if ctx.agreed_suit == Some(suit) && opened_one_of_a_suit(ctx) {
        return Some(rule_reraise_agreed(p, ctx, suit, jump, ex));
    }
    let hcp = if jump >= 1 {
        p.rebid.jump_rebid.clone()
    } else {
        opening_level_hcp(p, ctx.opener_first_bid)
    };
    let constraint = HandConstraint::Atom(Atom::ANY.with_hcp(hcp.clone()).with_len(suit, 6..=13));
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
/// more. Length is the opening's own minimum for the opened suit, 4 for a second suit.
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
    let hcp = if jump >= 1 {
        *p.rebid.jump_rebid.start()..=*p.opening_hcp.end()
    } else if ctx.competitive {
        p.opening_hcp.clone()
    } else {
        p.rebid.jump_rebid.clone()
    };
    Inference {
        constraint: HandConstraint::Atom(
            Atom::ANY.with_hcp(hcp.clone()).with_len(suit, min_len..=13),
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

/// Opener's notrump rebid after a 1-of-a-suit opening: the cheapest notrump shows
/// `rebid.nt_1`, a jump `rebid.nt_2`, both balanced. Higher notrump rebids, and notrump once a
/// suit is agreed (`1H-P-2H-P-2NT` is a game try, not a range rebid), are left to `fallback`.
fn rule_rebid_nt(p: &NaturalParams, ctx: &CallContext, ex: bool) -> Option<Inference> {
    if ctx.role != Role::Opener
        || !opened_one_of_a_suit(ctx)
        || ctx.level > 2
        || ctx.agreed_suit.is_some()
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

/// Opener's new suit that is not a reverse (`rule_reverse` runs first), after a 1-of-a-suit
/// opening: 4+ cards, from the opening minimum up to `rebid.jump_raise`'s maximum; a jump shift
/// shows more than `rebid.jump_rebid`.
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
    let hcp = if jump >= 1 {
        p.rebid.jump_rebid.end().saturating_add(1)..=37
    } else {
        *p.opening_hcp.start()..=*p.rebid.jump_raise.end()
    };
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
    /// against the 1NT opening's own strength (`opening_hcp`, since the notrump tiers are not in
    /// this table), not against `weak_two` -- `opener_first_suit` names the completion (`2H` at
    /// level 2), which used to be mistaken for a weak-two opening by level alone (see
    /// `opening_level_hcp`'s doc comment).
    #[test]
    fn rebid_own_after_nt_transfer_completion_uses_opening_hcp() {
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
        assert_eq!(inf.constraint.hcp_range(), params.opening_hcp);
        assert_ne!(inf.constraint.hcp_range(), params.weak_two.1);
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
