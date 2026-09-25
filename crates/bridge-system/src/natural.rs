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
}

/// Classifies call `index` of `auction` from the point of view of its caller.
///
/// Only `auction.calls()[..index]` (the history strictly before this call) and the call at
/// `index` itself are read; anything at or after `index + 1` (a hypothetical continuation
/// appended by a caller such as [`NaturalInference::candidates`]) is ignored, so `classify` is
/// stable under `auction.with(candidate_call)`.
pub fn classify(auction: &Auction, index: usize, owner: Seat) -> CallContext {
    let call = auction.calls()[index];
    let history = &auction.calls()[..index];

    let our_suits = suits_bid_by(auction, history, |s| s.side() == owner.side());
    let their_suits = suits_bid_by(auction, history, |s| s.side() != owner.side());
    let owner_suits = suits_bid_by(auction, history, |s| s == owner);
    let partner_suits = suits_bid_by(auction, history, |s| s == owner.partner());

    let role = classify_role(auction, history, index, owner);
    let opener_first_suit = owner_first_suit(auction, history, owner);
    let opener_first_bid = owner_first_bid(auction, history, owner);
    let kind = classify_kind(
        auction,
        history,
        owner,
        call,
        role,
        our_suits,
        their_suits,
        owner_suits,
        partner_suits,
        opener_first_suit,
    );

    let level = match call {
        Call::Bid(b) => b.level(),
        _ => 0,
    };
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

    CallContext {
        role,
        kind,
        call,
        level,
        position,
        passed_hand,
        vul,
        competitive,
        partner_last,
        partner_constraint: None,
        our_suits,
        their_suits,
        agreed_suit,
        last_bid,
        forcing_situation: false,
        opener_first_suit,
        opener_first_bid,
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
        )),
        Call::Bid(b) => {
            let strain = b.strain();
            let nt = strain == Strain::NoTrump;
            let new_suit = !nt && !our_suits.contains(strain) && !their_suits.contains(strain);
            let cue = !nt && their_suits.contains(strain);
            let raise = !nt && partner_suits.contains(strain);
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
fn classify_double(
    auction: &Auction,
    history: &[Call],
    owner: Seat,
    role: Role,
    our_suits: StrainSet,
    their_suits: StrainSet,
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
    // `their_suits` already includes `target.strain()`, so this is really "we have also bid the
    // suit they are doubling in" -- the classic penalty-double trigger of an agreed trump suit.
    let agreed_suit = our_suits.contains(target.strain());
    let penalty_cond = target.strain() == Strain::NoTrump || target.level() >= 4 || agreed_suit;

    let partner_last = history
        .iter()
        .enumerate()
        .rev()
        .find(|(i, _)| auction.seat_at(*i) == owner.partner())
        .map(|(_, c)| *c);

    if low_level_suit && our_suits.0 == 0 && partner_last != Some(Call::Double) {
        // The `partner_last != Double` guard keeps a double right after partner's own takeout
        // double (no suit bid by us yet either) from being read as a second takeout double
        // instead of responsive (checked below).
        DoubleKind::Takeout
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
        let p = &self.params;
        rule_open_1major(p, ctx)
            .or_else(|| rule_open_1m(p, ctx))
            .or_else(|| rule_open_nt(p, ctx))
            .or_else(|| rule_open_weak2(p, ctx))
            .or_else(|| rule_open_2c(p, ctx))
            .or_else(|| rule_open_preempt(p, ctx))
            .or_else(|| rule_open_pass(p, ctx))
            .or_else(|| rule_overcall(p, ctx))
            .or_else(|| rule_jump_overcall(p, ctx))
            .or_else(|| rule_nt_overcall(p, ctx))
            .or_else(|| rule_takeout_x(p, ctx))
            .or_else(|| rule_penalty_x(p, ctx))
            .or_else(|| rule_negative_x(p, ctx))
            .or_else(|| rule_raise(p, ctx))
            .or_else(|| rule_new_suit_resp_1(p, ctx))
            .or_else(|| rule_new_suit_resp_2(p, ctx))
            .or_else(|| rule_resp_nt(p, ctx))
            .or_else(|| rule_rebid_own(p, ctx))
            .or_else(|| rule_reverse(p, ctx))
            .or_else(|| rule_cue(p, ctx))
            .or_else(|| rule_pass_forcing(ctx))
            .or_else(|| rule_pass_default(p, ctx))
            .unwrap_or_else(rule_fallback)
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

fn rule_open_1m(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
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
            .with_suit_len(suit, p.open_1m_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.6,
        rule: "open_1m",
        explanation: format!(
            "opening {}: {}+ cards, {}-{} hcp",
            ctx.call,
            p.open_1m_len,
            p.opening_hcp.start(),
            p.opening_hcp.end()
        ),
    })
}

fn rule_open_1major(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
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
            .with_suit_len(suit, p.open_1major_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.6,
        rule: "open_1M",
        explanation: format!(
            "opening {}: {}+ cards, {}-{} hcp",
            ctx.call,
            p.open_1major_len,
            p.opening_hcp.start(),
            p.opening_hcp.end()
        ),
    })
}

fn rule_open_nt(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
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
        explanation: format!(
            "opening {}: balanced, {}-{} hcp",
            ctx.call,
            hcp.start(),
            hcp.end()
        ),
    })
}

fn rule_open_weak2(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
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
            .with_suit_len(suit, *min_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.5,
        rule: "open_weak2",
        explanation: format!(
            "weak two {}: {}+ cards, {}-{} hcp",
            ctx.call,
            min_len,
            hcp.start(),
            hcp.end()
        ),
    })
}

fn rule_open_2c(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
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
        explanation: format!("strong 2C: {}+ hcp", p.strong_two_c),
    })
}

fn rule_open_preempt(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
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
            .with_suit_len(suit, *min_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.5,
        rule: "open_preempt",
        explanation: format!(
            "preempt {}: {}+ cards, {}-{} hcp",
            ctx.call,
            min_len,
            hcp.start(),
            hcp.end()
        ),
    })
}

fn rule_open_pass(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
    if ctx.role != Role::Opener || ctx.call != Call::Pass || ctx.passed_hand {
        return None;
    }
    let hi = p.opening_hcp.start().saturating_sub(1);
    let constraint = HandConstraint::Atom(Atom::ANY.with_hcp(0..=hi));
    Some(Inference {
        constraint,
        confidence: 0.5,
        rule: "open_pass",
        explanation: format!("declines to open: 0-{hi} hcp"),
    })
}

fn rule_overcall(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
    if !matches!(ctx.role, Role::Overcaller | Role::Balancer) {
        return None;
    }
    let CallKind::Bid {
        nt: false, jump: 0, ..
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
            .with_suit_len(suit, *min_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.5,
        rule: "overcall",
        explanation: format!(
            "overcall {}: {}+ cards, {}-{} hcp",
            ctx.call,
            min_len,
            hcp.start(),
            hcp.end()
        ),
    })
}

fn rule_jump_overcall(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
    if !matches!(ctx.role, Role::Overcaller | Role::Balancer) {
        return None;
    }
    let CallKind::Bid {
        nt: false, jump: 1, ..
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
            .with_suit_len(suit, *min_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.4,
        rule: "jump_overcall",
        explanation: format!(
            "jump overcall {}: {}+ cards, {}-{} hcp",
            ctx.call,
            min_len,
            hcp.start(),
            hcp.end()
        ),
    })
}

fn rule_nt_overcall(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
    if !matches!(ctx.role, Role::Overcaller | Role::Balancer) {
        return None;
    }
    let CallKind::Bid { nt: true, .. } = ctx.kind else {
        return None;
    };
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
        explanation: format!(
            "notrump overcall {}: balanced, {}-{} hcp, stopper",
            ctx.call,
            hcp.start(),
            hcp.end()
        ),
    })
}

fn rule_takeout_x(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
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
            Atom::ANY.with_suit_len(their_suit, 0..=their_max),
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
        confidence: 0.5,
        rule: "takeout_x",
        explanation: format!("takeout double: {}+ hcp", min_hcp.start()),
    })
}

fn rule_penalty_x(_p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
    if ctx.call != Call::Double {
        return None;
    }
    let CallKind::Double(DoubleKind::Penalty) = ctx.kind else {
        return None;
    };
    let their_suit = ctx.last_bid.and_then(|b| b.strain().suit());
    let mut constraint = HandConstraint::Atom(Atom::ANY.with_hcp(10..=37));
    if let Some(their_suit) = their_suit {
        constraint = constraint.and(HandConstraint::Atom(
            Atom::ANY.with_suit_len(their_suit, 4..=13),
        ));
    }
    Some(Inference {
        constraint,
        confidence: 0.3,
        rule: "penalty_x",
        explanation: "penalty double: 10+ hcp, 4+ of their suit".to_string(),
    })
}

fn rule_negative_x(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
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
    // opponents just showed (their overcall) says nothing about an unbid major.
    let majors = [Suit::Hearts, Suit::Spades]
        .into_iter()
        .filter(|&s| {
            let strain = Strain::from_suit(s);
            !ctx.our_suits.contains(strain) && !ctx.their_suits.contains(strain)
        })
        .map(|s| HandConstraint::Atom(Atom::ANY.with_suit_len(s, 4..=13)))
        .reduce(HandConstraint::or)
        .unwrap_or(HandConstraint::ANY);
    let constraint = majors.and(HandConstraint::Atom(Atom::ANY.with_hcp(min_hcp..=37)));
    Some(Inference {
        constraint,
        confidence: 0.5,
        rule: "negative_x",
        explanation: format!("negative double: {min_hcp}+ hcp, 4+ card unbid major"),
    })
}

fn rule_raise(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
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
    let constraint = HandConstraint::Atom(
        Atom::ANY
            .with_hcp(hcp.clone())
            .with_suit_len(suit, min_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.6,
        rule: "raise",
        explanation: format!(
            "raise {}: {}+ support, {}-{} hcp",
            ctx.call,
            min_len,
            hcp.start(),
            hcp.end()
        ),
    })
}

fn rule_new_suit_resp_1(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
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
    let constraint = HandConstraint::Atom(
        Atom::ANY
            .with_hcp(min_hcp..=37)
            .with_suit_len(suit, min_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.5,
        rule: "new_suit_resp_1",
        explanation: format!(
            "1-level new suit {}: {min_len}+ cards, {min_hcp}+ hcp",
            ctx.call
        ),
    })
}

fn rule_new_suit_resp_2(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
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
            .with_suit_len(suit, min_len..=13),
    );
    Some(Inference {
        constraint,
        confidence: 0.5,
        rule: "new_suit_resp_2",
        explanation: format!(
            "2-level new suit {}: {min_len}+ cards, {min_hcp}+ hcp",
            ctx.call
        ),
    })
}

fn rule_resp_nt(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
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
    Some(Inference {
        constraint: HandConstraint::Atom(atom),
        confidence: 0.5,
        rule: "resp_nt",
        explanation: format!(
            "notrump response {}: {}-{} hcp",
            ctx.call,
            hcp.start(),
            hcp.end()
        ),
    })
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

fn rule_rebid_own(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
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
    let hcp = if jump >= 1 {
        p.rebid.jump_rebid.clone()
    } else {
        opening_level_hcp(p, ctx.opener_first_bid)
    };
    let constraint =
        HandConstraint::Atom(Atom::ANY.with_hcp(hcp.clone()).with_suit_len(suit, 6..=13));
    Some(Inference {
        constraint,
        confidence: 0.5,
        rule: "rebid_own",
        explanation: format!(
            "rebids own suit {}: 6+ cards, {}-{} hcp",
            ctx.call,
            hcp.start(),
            hcp.end()
        ),
    })
}

fn rule_reverse(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
    let CallKind::Bid { reverse: true, .. } = ctx.kind else {
        return None;
    };
    let bid = ctx.call.bid()?;
    let second_suit = bid.strain().suit()?;
    let second = HandConstraint::Atom(Atom::ANY.with_suit_len(second_suit, 4..=13));
    // The suit opener actually opened (not the side-level `our_suits`, which by now also
    // contains responder's suit(s)): a reverse requires 5+ of *that* suit specifically.
    let (first_suit, _) = ctx.opener_first_suit?;
    let first = HandConstraint::Atom(Atom::ANY.with_suit_len(first_suit, 5..=13));
    let constraint = HandConstraint::Atom(Atom::ANY.with_hcp(p.rebid.reverse..=37))
        .and(first)
        .and(second);
    Some(Inference {
        constraint,
        confidence: 0.4,
        rule: "reverse",
        explanation: format!(
            "reverse into {}: {}+ hcp, 5+/4+ shape",
            ctx.call, p.rebid.reverse
        ),
    })
}

fn rule_cue(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
    let CallKind::Bid { cue: true, .. } = ctx.kind else {
        return None;
    };
    /// Rough combined-points target for a game-forcing sequence; not a field of
    /// [`NaturalParams`] because §8.3 only specifies it as a fallback formula, not a tunable.
    const GF_TOTAL: u8 = 25;
    let min_hcp = match &ctx.partner_constraint {
        Some(c) => GF_TOTAL.saturating_sub(*c.hcp_range().start()),
        None => p.advance.cue,
    };
    Some(Inference {
        constraint: HandConstraint::Atom(Atom::ANY.with_hcp(min_hcp..=37)),
        confidence: 0.3,
        rule: "cue",
        explanation: format!("cue bid {}: {min_hcp}+ hcp, artificial", ctx.call),
    })
}

fn rule_pass_forcing(ctx: &CallContext) -> Option<Inference> {
    if ctx.call != Call::Pass || !ctx.forcing_situation {
        return None;
    }
    Some(Inference {
        constraint: HandConstraint::Atom(Atom::ANY.with_hcp(0..=0)),
        confidence: 0.1,
        rule: "pass_forcing",
        explanation: "pass over a forcing call: contradictory, near-unsatisfiable".to_string(),
    })
}

fn rule_pass_default(p: &NaturalParams, ctx: &CallContext) -> Option<Inference> {
    if ctx.call != Call::Pass {
        return None;
    }
    let constraint = match ctx.role {
        Role::Responder => {
            let hi = p.response.new_suit_1.1.saturating_sub(1);
            HandConstraint::Atom(Atom::ANY.with_hcp(0..=hi))
        }
        Role::Advancer => {
            let hi = p.advance.raise.1.start().saturating_sub(1);
            HandConstraint::Atom(Atom::ANY.with_hcp(0..=hi))
        }
        _ => HandConstraint::ANY,
    };
    Some(Inference {
        constraint,
        confidence: 0.4,
        rule: "pass_default",
        explanation: "pass: below the threshold to act".to_string(),
    })
}

fn rule_fallback() -> Inference {
    Inference {
        constraint: HandConstraint::ANY,
        confidence: 0.05,
        rule: "fallback",
        explanation: "no rule matched".to_string(),
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
            .with_suit_len(suit, 2..=13),
    );
    let queen = HandConstraint::Atom(
        Atom::ANY
            .with_cards(CardRequirement::in_suit(
                suit,
                Holding::EMPTY.with(Rank::Queen),
                1..=1,
            ))
            .with_suit_len(suit, 3..=13),
    );
    ace.or(king).or(queen)
}

/// `suits` each meeting `min_len`, relaxed to "at least two of them" once there are three or
/// more (§8.3, `takeout_x`).
fn at_least_two_of(suits: &[Suit], min_len: u8) -> HandConstraint {
    if suits.len() <= 2 {
        return suits
            .iter()
            .map(|&s| HandConstraint::Atom(Atom::ANY.with_suit_len(s, min_len..=13)))
            .reduce(HandConstraint::and)
            .unwrap_or(HandConstraint::ANY);
    }
    let mut combos = Vec::new();
    for i in 0..suits.len() {
        for j in (i + 1)..suits.len() {
            let both = HandConstraint::Atom(Atom::ANY.with_suit_len(suits[i], min_len..=13)).and(
                HandConstraint::Atom(Atom::ANY.with_suit_len(suits[j], min_len..=13)),
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
