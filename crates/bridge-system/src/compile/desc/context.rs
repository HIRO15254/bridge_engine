//! Context resolution (pass 2).
//!
//! | Word | Resolution |
//! | --- | --- |
//! | `GF` | `own_min = gf_total − partner_min` |
//! | `INV` | `[inv.start − partner_min, inv.end − partner_min]`; `INV+` lower bound only; mildly/strongly shift both bounds by ∓1; "at most" upper bound only |
//! | `MIN` / `MAX` | lower / upper half of the same player's range so far; with none, the opener's `[opening_min, opening_min+2]` / `[opening_min+3, 21]` (assumed), unconstrained (assumed) for anyone else |
//! | `weak` | responder `[0, weak_max]`; opener at level 2 `weak_two`; level 3/4 `preempt`; overcaller `weak_jump` |
//! | `S/T` | `own_min = slam_total − partner_max` |
//! | `QUANT` | `[small − partner_max, small − partner_min − 1]` with `small = slam_total + 2` (small-slam total) |
//! | `NAT` | `NaturalInference` for this call in this context, intersected with explicit fragments |
//! | `#`, `Own`, `Agreed`, `Theirs` | concrete suits from the path |
//!
//! A player's range is the intersection of every HCP range that player has shown on the path
//! (`RowContext::partner_hcp` / `own_hcp`), not just their last call's, which is often an
//! artificial call stating no HCP at all. A range that is still the full `0..=hcp_max` is
//! unknown: an unknown partner range defaults to `opening_min..=21` and is flagged `assumed`.

use core::ops::RangeInclusive;

use bridge_constraint::{Atom, CardRequirement, EvalRequirement, HandConstraint, Metric};
use bridge_core::{Bid, Call, Holding, Rank, ShapeSet, Side as TableSide, Suit};
use bridge_eval::LtcMethod;

use super::tokens::{QualityWord, StrengthWord, SuitRef, Token};
use crate::{Node, SystemMeta, natural::Role, pattern::Binding};

/// Where a resolved value came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// Written in the description.
    Explicit,
    /// Derived from the path context.
    Context,
    /// Taken from natural-inference defaults.
    NaturalDefault,
}

/// Provenance of a resolved fragment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Provenance {
    /// Byte span of the token.
    pub span: (u16, u16),
    /// A default was assumed because the context did not state the value.
    pub assumed: bool,
    /// Source.
    pub source: Source,
}

/// Everything known about a row when its description is compiled (ancestors are done first).
#[derive(Clone, Debug)]
pub struct RowContext<'a> {
    /// This row's call.
    pub call: Call,
    /// Whose call (as a table side).
    pub side: TableSide,
    /// Bid level (0 for pass/double/redouble).
    pub level: u8,
    /// The call skipped at least one level.
    pub is_jump: bool,
    /// Variable binding.
    pub binding: &'a Binding,
    /// The suit `#` refers to.
    pub hash_suit: Option<Suit>,
    /// The same player's previous node on this path.
    pub own_prev: Option<&'a Node>,
    /// Partner's last node on this path.
    pub partner_last: Option<&'a Node>,
    /// Opponents' last bid.
    pub their_last_bid: Option<Bid>,
    /// Agreed suit, if any.
    pub agreed_suit: Option<Suit>,
    /// Role in the auction.
    pub role: Role,
    /// Partner's HCP range so far: the intersection of the HCP ranges of every node partner made
    /// on this path (`None` when not tracked; then `partner_last`'s own range is used).
    pub partner_hcp: Option<RangeInclusive<u8>>,
    /// This player's own HCP range so far, the same way (`None`: fall back to `own_prev`).
    pub own_hcp: Option<RangeInclusive<u8>>,
}

/// Resolves the context-dependent tokens of a clause into constraint literals.
///
/// One literal (an [`Atom`], or for a stopper the §7.4 `Or` of honour-and-length atoms) and one
/// [`Provenance`] are produced per input token, in the same order, so a caller holding the
/// original [`super::clause::Fragment`]s can zip them back together by index. `stated_with(k,
/// pred)` says whether a token matching `pred` is stated alongside token `k` (conjoined with it,
/// neither negated nor a possibility), so a splinter can see what the author stated explicitly
/// next to it.
/// `Provenance::span` is left as `(0, 0)` here: this function is not given fragment spans (only
/// bare tokens), so `compile_description` (which does have them) overwrites each entry's `span`
/// with the originating fragment's span before the provenance is used for lints or tracing.
pub fn resolve(
    tokens: &[Token],
    ctx: &RowContext<'_>,
    meta: &SystemMeta,
    stated_with: &StatedWith<'_>,
) -> (Vec<HandConstraint>, Vec<Provenance>) {
    let mut literals = Vec::with_capacity(tokens.len());
    let mut provs = Vec::with_capacity(tokens.len());
    for (k, token) in tokens.iter().enumerate() {
        let (literal, prov) = match token {
            Token::Stopper(suitref) => match resolve_single_suit(*suitref, ctx) {
                Some(suit) => (stopper_constraint(suit), explicit()),
                None => (HandConstraint::ANY, context_prov(true)),
            },
            Token::Splinter(short, mini) => {
                let facts = ExplicitFacts::of(&|pred| stated_with(k, pred), ctx);
                let (atom, prov) = resolve_splinter(*short, *mini, &facts, ctx, meta);
                (HandConstraint::Atom(atom), prov)
            }
            _ => {
                let (atom, prov) = resolve_one(token, ctx, meta);
                (HandConstraint::Atom(atom), prov)
            }
        };
        literals.push(literal);
        provs.push(prov);
    }
    (literals, provs)
}

/// A predicate over tokens.
pub type TokenPred<'a> = dyn Fn(&Token) -> bool + 'a;
/// Whether a token matching the predicate is stated alongside one fixed token.
pub type Stated<'a> = dyn Fn(&TokenPred<'_>) -> bool + 'a;
/// Whether a token matching the predicate is stated alongside the token at the given index
/// (see `compile_description`'s `stated_alongside`).
pub type StatedWith<'a> = dyn Fn(usize, &TokenPred<'_>) -> bool + 'a;

/// What the description states explicitly alongside a compound word (`SPL`), for the
/// "衝突は明示が勝つ" rule. `stated` answers whether a fragment matching a predicate is stated
/// alongside the word (conjoined with it, neither negated nor a possibility; see
/// `compile_description`'s `stated_alongside`), so a negated, hedged or other-branch fragment
/// never switches a part of the word off.
struct ExplicitFacts {
    /// An explicit `Hcp`/`Points` fragment.
    strength: bool,
    /// An explicit shortness (`Shortness`, or a suit length of at most 1).
    shortness: bool,
    /// Suits whose length an explicit `SuitLen` fragment pins.
    lengths: Vec<Suit>,
}

impl ExplicitFacts {
    fn of(stated: &Stated<'_>, ctx: &RowContext<'_>) -> ExplicitFacts {
        let strength = stated(&|t| matches!(t, Token::Hcp(_) | Token::Points(_)));
        let shortness = stated(&|t| match t {
            Token::Shortness(..) => true,
            Token::SuitLen(_, r) => *r.end() <= 1,
            _ => false,
        });
        let lengths = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades]
            .into_iter()
            .filter(|&suit| {
                stated(&|t| matches!(t, Token::SuitLen(..)) && suit_len_pins(t, suit, ctx))
            })
            .collect();
        ExplicitFacts {
            strength,
            shortness,
            lengths,
        }
    }
}

/// A stopper in `suit` (`docs/design/06-system.md` §7.4): `A`, or `K` with 2+ cards, or `Q` with
/// 3+ cards, or `J` with 4+ cards.
pub(super) fn stopper_constraint(suit: Suit) -> HandConstraint {
    let branch = |rank: Rank, min_len: u8| {
        HandConstraint::Atom(Atom {
            shapes: ShapeSet::from_suit_len(suit, min_len, 13),
            ..Atom::ANY.with_cards(CardRequirement::in_suit(
                suit,
                Holding::EMPTY.with(rank),
                1..=1,
            ))
        })
    };
    HandConstraint::Or(vec![
        branch(Rank::Ace, 1),
        branch(Rank::King, 2),
        branch(Rank::Queen, 3),
        branch(Rank::Jack, 4),
    ])
}

fn explicit() -> Provenance {
    Provenance {
        span: (0, 0),
        assumed: false,
        source: Source::Explicit,
    }
}

fn context_prov(assumed: bool) -> Provenance {
    Provenance {
        span: (0, 0),
        assumed,
        source: Source::Context,
    }
}

fn natural_default() -> Provenance {
    Provenance {
        span: (0, 0),
        assumed: true,
        source: Source::NaturalDefault,
    }
}

/// The suit of this row's own call, if it is a suit bid (`None` for notrump/pass/double/redouble).
pub(super) fn own_suit(ctx: &RowContext<'_>) -> Option<Suit> {
    match ctx.call {
        Call::Bid(bid) => bid.strain().suit(),
        _ => None,
    }
}

/// Whether `token` is an explicit fragment that already pins the length of `suit`: a `SuitLen`
/// naming it (possibly through a [`SuitRef`] that resolves to it) or a full `Shape` (which pins
/// every suit's length at once). Used by `compile_description`'s NAT explicit-wins rule
/// (`docs/design/06-system.md` §7.5's "NAT" row: "衝突は明示が勝つ" -- NAT's own assumed minimum
/// length must give way to a length the author wrote out explicitly for the same suit).
pub(super) fn suit_len_pins(token: &Token, suit: Suit, ctx: &RowContext<'_>) -> bool {
    match token {
        Token::SuitLen(suitref, _) => resolve_single_suit(*suitref, ctx) == Some(suit),
        Token::Shape(_) => true,
        _ => false,
    }
}

/// The suit of the opponents' last bid, if it named one.
fn their_suit(ctx: &RowContext<'_>) -> Option<Suit> {
    ctx.their_last_bid.and_then(|bid| bid.strain().suit())
}

/// The trump suit a support/fit/splinter fragment agrees on: `ctx.agreed_suit` when the
/// expansion loop (`compile::expand`) already tracked one (an earlier support/fit call, or this
/// call repeating partner's own suit), or else partner's most recent *natural* suit bid
/// (`docs/design/06-system.md` §7.5's `support`/`fit`/`SUPP`/raise row: "`agreed = agreed_suit`
/// かパートナーの最後のスートビッド" -- with nothing else established, a raise or splinter
/// defaults to agreeing whatever suit partner bid last). Without this fallback, a splinter made
/// right after partner's *first* suit call -- the common case, since a splinter is itself usually
/// the very next thing said about trumps -- would find `ctx.agreed_suit` still `None` and treat
/// its own call's suit as both the shown shortness *and*, wrongly, the only length reference
/// available, with no `suit_len[agreed]` conjunct to keep the two apart.
///
/// Partner's last call only counts when it is not [`crate::NodeFlags::artificial`]: an artificial
/// relay/ask (a puppet, a forcing "(R)" step-response call, …) names no suit its bidder actually
/// holds, so treating it as the agreed trump would fabricate a length requirement in a suit the
/// description may separately, and correctly, describe as short (`jdh8/common/2NT-UNT.bml`'s `2N-
/// 3H- 3N = !SPL, 0--1!h`, where `3H` is a bare relay: without this guard, `SPL` would agree
/// hearts from `3H` and then contradict the row's own explicit `0--1!h`).
///
/// This module does not also fall back to this player's *own* previous suit (e.g. the opener's
/// own suit after a Jacoby-style `1M-2N-` raise, where partner's `2N` names no suit itself): the
/// trie shares one [`crate::Node`] per concrete auction position across every table that retraces
/// it (`compile::expand`'s module doc), so `own_prev`/`partner_last` reflect whichever table's
/// traversal happened to compile that position *first* -- a different, unrelated table can
/// retrace the identical call sequence with a same-suit `own_prev` that is not artificial (its own
/// history has a genuine natural meaning there), and folding that in here would agree the wrong
/// suit for a row that only makes sense under the first table's own context. Left as a known gap
/// (`docs/design/06-system.md`'s open issues): `SPL`/`Support` under a partner call that names no
/// suit (an NT raise) stay `assumed` rather than resolved.
fn agreed_suit_or_partner_last(ctx: &RowContext<'_>) -> Option<Suit> {
    ctx.agreed_suit.or_else(|| {
        ctx.partner_last
            .filter(|node| !node.flags.artificial)
            .and_then(|node| {
                let suit = node.call.bid().and_then(|bid| bid.strain().suit())?;
                // As in `compile::expand`'s `partner_bid_this_suit`: a non-artificial call still
                // only agrees the suit it names when its own constraint shows length there. A call
                // that mentions the strain while actually showing shortness or the other hand's
                // suits (e.g. `1C-1D-2S = 16+, 5+!c and 4+!h`) is not a spade agreement.
                (*node.constraint.suit_len(suit).start() >= 3).then_some(suit)
            })
    })
}

/// Resolves a [`SuitRef`] and a length range into a [`ShapeSet`] literal, with the provenance
/// that reflects whether resolving the reference needed path context and whether that context
/// was actually available (`AnyMajor`/`AnyMinor` fold both candidate suits into one `ShapeSet`
/// via `union`, so the ambiguity never needs a top-level `Or`).
fn suit_len_shapes(
    suitref: SuitRef,
    lo: u8,
    hi: u8,
    ctx: &RowContext<'_>,
) -> (ShapeSet, Provenance) {
    match suitref {
        SuitRef::Fixed(suit) => (ShapeSet::from_suit_len(suit, lo, hi), explicit()),
        SuitRef::Hash => match ctx.hash_suit {
            Some(suit) => (ShapeSet::from_suit_len(suit, lo, hi), context_prov(false)),
            None => (ShapeSet::ALL, context_prov(true)),
        },
        SuitRef::Own => match own_suit(ctx) {
            Some(suit) => (ShapeSet::from_suit_len(suit, lo, hi), context_prov(false)),
            None => (ShapeSet::ALL, context_prov(true)),
        },
        SuitRef::AnyMajor => (
            ShapeSet::from_suit_len(Suit::Hearts, lo, hi).union(ShapeSet::from_suit_len(
                Suit::Spades,
                lo,
                hi,
            )),
            explicit(),
        ),
        SuitRef::AnyMinor => (
            ShapeSet::from_suit_len(Suit::Clubs, lo, hi).union(ShapeSet::from_suit_len(
                Suit::Diamonds,
                lo,
                hi,
            )),
            explicit(),
        ),
        SuitRef::Agreed => match ctx.agreed_suit {
            Some(suit) => (ShapeSet::from_suit_len(suit, lo, hi), context_prov(false)),
            None => (ShapeSet::ALL, context_prov(true)),
        },
        SuitRef::Theirs => match their_suit(ctx) {
            Some(suit) => (ShapeSet::from_suit_len(suit, lo, hi), context_prov(false)),
            None => (ShapeSet::ALL, context_prov(true)),
        },
    }
}

/// Resolves a [`SuitRef`] to one concrete suit, for literals (card quality, stoppers) that need
/// a single suit and cannot fold an `AnyMajor`/`AnyMinor` ambiguity into a `ShapeSet` union.
/// Returns `None` for that ambiguous case (the caller falls back to `Atom::ANY`); see
/// `open_issues` in the compiler's final report.
fn resolve_single_suit(suitref: SuitRef, ctx: &RowContext<'_>) -> Option<Suit> {
    match suitref {
        SuitRef::Fixed(suit) => Some(suit),
        SuitRef::Hash => ctx.hash_suit,
        SuitRef::Own => own_suit(ctx),
        SuitRef::Agreed => ctx.agreed_suit,
        SuitRef::Theirs => their_suit(ctx),
        SuitRef::AnyMajor | SuitRef::AnyMinor => None,
    }
}

/// A card requirement approximating a suit-quality word (`docs/design/06-system.md` §7.4): the
/// exact honour count each word names, read off the top of the suit. `Good` is a loose, single
/// top honour (the vaguest word in the table).
fn quality_requirement(suit: Suit, word: QualityWord) -> CardRequirement {
    match word {
        QualityWord::Solid => CardRequirement::in_suit(suit, Holding::top_ranks(3), 3..=3),
        QualityWord::SemiSolid => CardRequirement::in_suit(suit, Holding::top_ranks(3), 2..=3),
        QualityWord::TwoOfTopThree => CardRequirement::in_suit(suit, Holding::top_ranks(3), 2..=3),
        QualityWord::ThreeOfTopFive => CardRequirement::in_suit(suit, Holding::top_ranks(5), 3..=5),
        QualityWord::Good => CardRequirement::in_suit(suit, Holding::top_ranks(3), 1..=3),
    }
}

/// A player's known HCP range: the tracked running range, else their last node's own range;
/// `None` when neither states anything (the full `0..=hcp_max` is "unknown", not "any").
fn known_range(
    tracked: Option<&RangeInclusive<u8>>,
    last: Option<&Node>,
    meta: &SystemMeta,
) -> Option<RangeInclusive<u8>> {
    let full = 0..=meta.strength.hcp_max;
    let informative = |r: &RangeInclusive<u8>| *r.start() > 0 || *r.end() < *full.end();
    tracked
        .filter(|r| informative(r))
        .cloned()
        .or_else(|| {
            last.map(|node| node.constraint.hcp_range())
                .filter(|r| informative(r))
        })
        .filter(|r| r.start() <= r.end())
}

/// Partner's HCP range so far on this path, or an assumed opening-range default when nothing
/// partner said states one (`docs/design/06-system.md` §7.5).
fn partner_hcp_range(ctx: &RowContext<'_>, meta: &SystemMeta) -> (RangeInclusive<u8>, bool) {
    match known_range(ctx.partner_hcp.as_ref(), ctx.partner_last, meta) {
        Some(r) => (r, false),
        None => (meta.strength.opening_min..=21, true),
    }
}

/// `weak`'s meaning depends on who is bidding and at what level (`docs/design/06-system.md`
/// §7.5): a responder's simple weak range, an opener's weak-two/preempt table, or an overcaller's
/// weak jump overcall (approximated by the jump entry of `NaturalParams::overcall`).
fn weak_hcp(ctx: &RowContext<'_>, meta: &SystemMeta) -> (RangeInclusive<u8>, bool) {
    match ctx.role {
        Role::Responder => (0..=meta.strength.weak_max, false),
        Role::Opener if ctx.level == 2 => (meta.natural.weak_two.1.clone(), false),
        Role::Opener if ctx.level == 3 || ctx.level == 4 => preempt_hcp(ctx, meta),
        Role::Overcaller => (meta.natural.overcall[2].1.clone(), false),
        _ => (0..=meta.strength.weak_max, true),
    }
}

/// `PRE`'s range from `NaturalParams::preempt`, matched by this call's level; falls back to the
/// generic weak range (assumed) when the level has no preempt entry.
fn preempt_hcp(ctx: &RowContext<'_>, meta: &SystemMeta) -> (RangeInclusive<u8>, bool) {
    meta.natural
        .preempt
        .iter()
        .find(|(level, _, _)| *level == ctx.level)
        .map(|(_, _, range)| (range.clone(), false))
        .unwrap_or((0..=meta.strength.weak_max, true))
}

/// Resolves one context-dependent `Strength` word into an HCP range, per the table in this
/// module's doc comment.
fn strength_hcp(
    word: StrengthWord,
    ctx: &RowContext<'_>,
    meta: &SystemMeta,
) -> (RangeInclusive<u8>, bool) {
    let sv = &meta.strength;
    let max = sv.hcp_max;
    let (partner, partner_assumed) = partner_hcp_range(ctx, meta);
    let partner_min = *partner.start();
    let partner_max = *partner.end();
    match word {
        StrengthWord::GameForcing => (
            sv.gf_total.saturating_sub(partner_min)..=max,
            partner_assumed,
        ),
        StrengthWord::Invitational | StrengthWord::Limit => {
            let lo = sv.inv_total.start().saturating_sub(partner_min);
            let hi = sv.inv_total.end().saturating_sub(partner_min).min(max);
            (lo..=hi.max(lo), partner_assumed)
        }
        StrengthWord::InvitationalPlus => (
            sv.inv_total.start().saturating_sub(partner_min)..=max,
            partner_assumed,
        ),
        StrengthWord::InvitationalMild => {
            // Both bounds, shifted down by 1 (`docs/design/06-system.md` §7.4/§7.5's "mild は
            // −1 ... のシフト").
            let lo = sv
                .inv_total
                .start()
                .saturating_sub(1)
                .saturating_sub(partner_min);
            let hi = sv
                .inv_total
                .end()
                .saturating_sub(1)
                .saturating_sub(partner_min)
                .min(max);
            (lo..=hi.max(lo), partner_assumed)
        }
        StrengthWord::InvitationalStrong => {
            // Both bounds, shifted up by 1 ("strong は +1 のシフト"), still a narrow range (not
            // the open-ended `InvitationalPlus` reading).
            let lo = sv
                .inv_total
                .start()
                .saturating_add(1)
                .saturating_sub(partner_min);
            let hi = sv
                .inv_total
                .end()
                .saturating_add(1)
                .saturating_sub(partner_min)
                .min(max);
            (lo..=hi.max(lo), partner_assumed)
        }
        StrengthWord::InvitationalAtMost => {
            // `end` bound only ("at most は end のみ"): the floor is unconstrained (0).
            let hi = sv.inv_total.end().saturating_sub(partner_min).min(max);
            (0..=hi, partner_assumed)
        }
        StrengthWord::Min | StrengthWord::Max => {
            let is_min = matches!(word, StrengthWord::Min);
            match known_range(ctx.own_hcp.as_ref(), ctx.own_prev, meta) {
                Some(prev) => {
                    let lo = *prev.start();
                    let hi = *prev.end();
                    let mid = lo + hi.saturating_sub(lo) / 2;
                    if is_min {
                        (lo..=mid, false)
                    } else {
                        ((mid + 1).min(hi).max(lo)..=hi, false)
                    }
                }
                // §7.5: with no range of its own yet, the opener's minimum / maximum opening; for
                // anyone else there is no base range to take a half of, so the word stays
                // unconstrained (still flagged assumed).
                None if ctx.role == Role::Opener => {
                    let open = sv.opening_min;
                    if is_min {
                        (open..=open.saturating_add(2).min(max), true)
                    } else {
                        (open.saturating_add(3).min(21)..=21.min(max), true)
                    }
                }
                None => (0..=max, true),
            }
        }
        StrengthWord::Weak => weak_hcp(ctx, meta),
        StrengthWord::Strong => (sv.strong_min..=max, false),
        StrengthWord::Preemptive => preempt_hcp(ctx, meta),
        StrengthWord::SlamTry => (
            sv.slam_total.saturating_sub(partner_max)..=max,
            partner_assumed,
        ),
        StrengthWord::Quantitative => {
            // An invitation to a small slam: enough for 6NT opposite partner's maximum but not
            // opposite the minimum (`small` = small-slam total, `slam_total` + 2, i.e. 33 with the
            // default 31 slam-interest threshold): 16..=17 over a 15-17 notrump.
            let small = sv.slam_total.saturating_add(2);
            let lo = small.saturating_sub(partner_max).min(max);
            let hi = small.saturating_sub(partner_min).saturating_sub(1).min(max);
            (lo..=hi.max(lo), partner_assumed)
        }
        StrengthWord::Negative => (0..=sv.neg_max, false),
    }
}

/// `NAT`: a minimal, direct natural meaning for this row's own call, per `meta.natural` and the
/// role/level in `ctx` — a suit bid's length per the role's natural-length parameter, notrump's
/// balanced shape, and nothing for pass/double/redouble. This does not go through
/// [`crate::natural::NaturalInference`] (its `classify`/`infer` are out of scope here; see
/// `open_issues` in the compiler's final report for routing `NAT` through it later).
fn resolve_natural(ctx: &RowContext<'_>, meta: &SystemMeta) -> (Atom, Provenance) {
    let prov = natural_default();
    match ctx.call {
        Call::Pass | Call::Double | Call::Redouble => (Atom::ANY, prov),
        Call::Bid(bid) => match bid.strain() {
            bridge_core::Strain::NoTrump => (
                Atom {
                    shapes: ShapeSet::from_classes(&meta.balanced.balanced),
                    ..Atom::ANY
                },
                prov,
            ),
            strain => {
                let suit = strain
                    .suit()
                    .expect("a non-notrump strain always names a suit");
                let min_len = natural_suit_length(suit, ctx, meta);
                (
                    Atom {
                        shapes: ShapeSet::from_suit_len(suit, min_len, 13),
                        ..Atom::ANY
                    },
                    prov,
                )
            }
        },
    }
}

/// The minimum length `NAT` implies for a suit bid, by role (`docs/design/06-system.md` §8.1's
/// `NaturalParams` table). Levels/roles the table does not single out fall back to a plain
/// 5-card suit, the most common minimum across the vocabulary.
fn natural_suit_length(suit: Suit, ctx: &RowContext<'_>, meta: &SystemMeta) -> u8 {
    let n = &meta.natural;
    match ctx.role {
        Role::Opener if ctx.level == 1 => {
            if matches!(suit, Suit::Hearts | Suit::Spades) {
                n.open_1major_len
            } else {
                n.open_1m_len
            }
        }
        Role::Opener => 5,
        Role::Responder if ctx.level <= 1 => n.response.new_suit_1.0,
        Role::Responder => n.response.new_suit_2.0,
        Role::Overcaller if ctx.level <= 1 => n.overcall[0].0,
        Role::Overcaller if ctx.is_jump => n.overcall[2].0,
        Role::Overcaller => n.overcall[1].0,
        Role::Advancer => n.advance.new_suit.0,
        Role::Balancer => n.overcall[1].0,
    }
}

/// `SPL`/`splinter` (and its mini-splinter variant): a compound atom of shortness in the own
/// call's suit, support in the agreed suit, and a game-forcing (or, for the mini variant,
/// invitational) strength range (`docs/design/06-system.md` §7.4/§7.5's `SPL` row: `suit_len[short]
/// = 0..=1` ∧ `suit_len[agreed] ≥ conventions.splinter_support` ∧ the `GF`/`INV` formula).
///
/// The short suit is the one written after the word (`SPL ♣`, `SPL m`), else the row's own call
/// (`SPL in the other major` is not parsed). Explicit statements elsewhere in the description
/// win over each part (§7.5's "衝突は明示が勝つ"): an explicit HCP/points range drops the
/// strength part, an explicit shortness drops the own-suit shortness, and an explicit length in
/// the agreed suit drops the support part. The support part is also dropped when the agreed suit
/// is the short suit itself (a splinter in, or over a transfer into, that suit), which would
/// otherwise demand at most one and at least four cards in the same suit.
fn resolve_splinter(
    short: Option<SuitRef>,
    mini: bool,
    explicit: &ExplicitFacts,
    ctx: &RowContext<'_>,
    meta: &SystemMeta,
) -> (Atom, Provenance) {
    let mut atom = Atom::ANY;
    let mut assumed = false;

    let short_suit = match short {
        Some(suitref) => {
            let (shapes, prov) = suit_len_shapes(suitref, 0, 1, ctx);
            atom = atom.intersect(&Atom {
                shapes,
                ..Atom::ANY
            });
            assumed |= prov.assumed;
            resolve_single_suit(suitref, ctx)
        }
        None if explicit.shortness => None,
        None => match own_suit(ctx) {
            Some(suit) => {
                atom = atom.intersect(&Atom {
                    shapes: ShapeSet::from_suit_len(suit, 0, 1),
                    ..Atom::ANY
                });
                Some(suit)
            }
            None => {
                assumed = true;
                None
            }
        },
    };

    match agreed_suit_or_partner_last(ctx) {
        Some(agreed) if Some(agreed) == short_suit || explicit.lengths.contains(&agreed) => {}
        Some(agreed) => {
            atom = atom.intersect(&Atom {
                shapes: ShapeSet::from_suit_len(agreed, meta.conventions.splinter_support, 13),
                ..Atom::ANY
            });
        }
        None => assumed = true,
    }

    if !explicit.strength {
        let word = if mini {
            StrengthWord::Invitational
        } else {
            StrengthWord::GameForcing
        };
        let (hcp, strength_assumed) = strength_hcp(word, ctx, meta);
        atom = atom.intersect(&Atom { hcp, ..Atom::ANY });
        assumed |= strength_assumed;
    }

    (atom, context_prov(assumed))
}

/// A literal honour run (`KQJ109x`): every listed honour held, and at least as many cards in
/// the suit as were written (§7.4).
fn honour_run_atom(suit: Suit, honours: &str, cards: u8) -> Atom {
    let mut mask = Holding::EMPTY;
    for ch in honours.chars() {
        let rank = match ch {
            'A' => Rank::Ace,
            'K' => Rank::King,
            'Q' => Rank::Queen,
            'J' => Rank::Jack,
            _ => Rank::Ten,
        };
        mask = mask.with(rank);
    }
    let n = u8::try_from(honours.len()).unwrap_or(u8::MAX);
    Atom {
        shapes: ShapeSet::from_suit_len(suit, cards, 13),
        ..Atom::ANY.with_cards(CardRequirement::in_suit(suit, mask, n..=n))
    }
}

/// Resolves one token, per `docs/design/06-system.md` §7.4/§7.5. Named conventions
/// ([`Token::Convention`]) and bare forcing/no-bound markers carry no atom by default (the
/// design's "既定では Atom なし" rule): they only ever contribute [`crate::NodeFlags`], which
/// `compile_description` derives separately from the same token list.
fn resolve_one(token: &Token, ctx: &RowContext<'_>, meta: &SystemMeta) -> (Atom, Provenance) {
    match token {
        Token::Hcp(range) => (
            Atom {
                hcp: range.clone(),
                ..Atom::ANY
            },
            explicit(),
        ),
        Token::Points(range) => (
            Atom::ANY.with_eval(EvalRequirement {
                metric: Metric::TotalPoints(meta.dist_method),
                range: range.clone(),
            }),
            explicit(),
        ),
        Token::SuitLen(suitref, range) => {
            let (shapes, prov) = suit_len_shapes(*suitref, *range.start(), *range.end(), ctx);
            (
                Atom {
                    shapes,
                    ..Atom::ANY
                },
                prov,
            )
        }
        Token::Shape(raw) => {
            let shapes = super::tokens::scan_shape(raw)
                .map(|(set, _)| set)
                .unwrap_or(ShapeSet::ALL);
            (
                Atom {
                    shapes,
                    ..Atom::ANY
                },
                explicit(),
            )
        }
        Token::Balanced => (
            Atom {
                shapes: ShapeSet::from_classes(&meta.balanced.balanced),
                ..Atom::ANY
            },
            explicit(),
        ),
        Token::SemiBalanced => {
            let shapes = ShapeSet::from_classes(&meta.balanced.balanced)
                .union(ShapeSet::from_classes(&meta.balanced.semi_balanced));
            (
                Atom {
                    shapes,
                    ..Atom::ANY
                },
                explicit(),
            )
        }
        Token::Unbalanced => {
            let shapes = ShapeSet::from_classes(&meta.balanced.balanced)
                .union(ShapeSet::from_classes(&meta.balanced.semi_balanced))
                .complement();
            (
                Atom {
                    shapes,
                    ..Atom::ANY
                },
                explicit(),
            )
        }
        Token::Strength(word) => {
            let (range, assumed) = strength_hcp(*word, ctx, meta);
            (
                Atom {
                    hcp: range,
                    ..Atom::ANY
                },
                context_prov(assumed),
            )
        }
        Token::Forcing(_) | Token::NoBound | Token::Convention(_) => (Atom::ANY, explicit()),
        Token::LengthOrder(a, cmp, b) => (
            Atom {
                shapes: super::tokens::length_order_shapes(*a, *cmp, *b),
                ..Atom::ANY
            },
            explicit(),
        ),
        // Both need the whole description; `resolve` handles them before calling this.
        Token::Splinter(..) | Token::Stopper(_) => (Atom::ANY, context_prov(true)),
        Token::HonourRun(suitref, honours, cards) => match resolve_single_suit(*suitref, ctx) {
            Some(suit) => (honour_run_atom(suit, honours, *cards), explicit()),
            None => (Atom::ANY, context_prov(true)),
        },
        Token::Quality(suitref, word) => match resolve_single_suit(*suitref, ctx) {
            Some(suit) => (
                Atom::ANY.with_cards(quality_requirement(suit, *word)),
                explicit(),
            ),
            None => (Atom::ANY, context_prov(true)),
        },
        Token::Shortness(suitref, max_len) => {
            let (shapes, prov) = suit_len_shapes(*suitref, 0, *max_len, ctx);
            (
                Atom {
                    shapes,
                    ..Atom::ANY
                },
                prov,
            )
        }
        Token::Support(min_len) => match agreed_suit_or_partner_last(ctx) {
            Some(suit) => (
                Atom {
                    shapes: ShapeSet::from_suit_len(suit, *min_len, 13),
                    ..Atom::ANY
                },
                context_prov(false),
            ),
            None => (Atom::ANY, context_prov(true)),
        },
        Token::Controls(range) => (
            Atom::ANY.with_eval(EvalRequirement {
                metric: Metric::Controls,
                range: range.clone(),
            }),
            explicit(),
        ),
        // The description counts whole losers (`LTC 7`, `6-7 losers`); `Metric::Losers` is
        // evaluated in half-losers (`losers_with(..).halves()`, `0..=24`), so both ends are doubled.
        Token::Losers(range) => (
            Atom::ANY.with_eval(EvalRequirement {
                metric: Metric::Losers(LtcMethod::Classic),
                range: range.start().saturating_mul(2).min(24)
                    ..=range.end().saturating_mul(2).min(24),
            }),
            explicit(),
        ),
        Token::Natural => resolve_natural(ctx, meta),
    }
}

#[cfg(test)]
mod tests {
    use core::ops::RangeInclusive;
    use std::sync::Arc;

    use bridge_constraint::HandConstraint;
    use bridge_core::{Bid, Call, Strain, Suit};

    use super::*;
    use crate::{
        Alertability, NodeFlags, NodeId, RowId,
        ast::{SeatCond, VulCond},
    };

    fn node_with_hcp(range: RangeInclusive<u8>) -> Node {
        Node {
            id: NodeId(0),
            row: RowId(0),
            side: bridge_system_pattern_side(),
            path: Arc::from(Vec::new()),
            calls: Vec::new(),
            call: Call::Pass,
            binding: Binding::default(),
            seat: SeatCond::default(),
            vul: VulCond::default(),
            constraint: HandConstraint::Atom(Atom {
                hcp: range,
                ..Atom::ANY
            }),
            branch_weights: None,
            priority: 0,
            volume_log2: 0,
            alertable: Alertability::default(),
            flags: NodeFlags::default(),
            description: String::new(),
            children: Vec::new(),
        }
    }

    // Small indirection so the test module doesn't need to import `pattern::Side` under a name
    // that collides with `bridge_core::Side` (aliased `TableSide` in this module).
    fn bridge_system_pattern_side() -> crate::pattern::Side {
        crate::pattern::Side::Us
    }

    fn base_ctx<'a>(binding: &'a Binding, call: Call, role: Role) -> RowContext<'a> {
        RowContext {
            call,
            side: TableSide::NS,
            level: match call {
                Call::Bid(bid) => bid.level(),
                _ => 0,
            },
            is_jump: false,
            binding,
            hash_suit: None,
            own_prev: None,
            partner_last: None,
            their_last_bid: None,
            agreed_suit: None,
            role,
            partner_hcp: None,
            own_hcp: None,
        }
    }

    /// [`resolve`], for tests whose literals are all plain atoms.
    fn resolve_atoms(
        tokens: &[Token],
        ctx: &RowContext<'_>,
        meta: &SystemMeta,
    ) -> (Vec<Atom>, Vec<Provenance>) {
        let (literals, provs) = resolve(tokens, ctx, meta, &|_, pred| tokens.iter().any(pred));
        let atoms = literals
            .into_iter()
            .map(|l| match l {
                HandConstraint::Atom(a) => a,
                other => panic!("expected an atom literal, got {other:?}"),
            })
            .collect();
        (atoms, provs)
    }

    fn recognize_one(text: &str) -> Token {
        let (tok, len) = super::super::tokens::recognize(text).expect("token recognized");
        assert_eq!(len, text.len(), "expected the whole fixture to match");
        tok
    }

    #[test]
    fn hcp_is_explicit() {
        let binding = Binding::default();
        let ctx = base_ctx(&binding, Call::Pass, Role::Opener);
        let meta = SystemMeta::default();
        let token = recognize_one("15-17 hcp");
        let (atoms, provs) = resolve_atoms(&[token], &ctx, &meta);
        assert_eq!(atoms[0].hcp, 15..=17);
        assert_eq!(provs[0].source, Source::Explicit);
        assert!(!provs[0].assumed);
    }

    #[test]
    fn game_forcing_uses_partner_min() {
        let binding = Binding::default();
        let mut ctx = base_ctx(&binding, Call::Pass, Role::Responder);
        let partner = node_with_hcp(12..=14);
        ctx.partner_last = Some(&partner);
        let meta = SystemMeta::default();
        let token = recognize_one("GF");
        let (atoms, provs) = resolve_atoms(&[token], &ctx, &meta);
        // gf_total (25) - partner_min (12) = 13.
        assert_eq!(atoms[0].hcp, 13..=37);
        assert_eq!(provs[0].source, Source::Context);
        assert!(!provs[0].assumed);
    }

    #[test]
    fn game_forcing_assumes_opening_partner_when_unknown() {
        let binding = Binding::default();
        let ctx = base_ctx(&binding, Call::Pass, Role::Responder);
        let meta = SystemMeta::default();
        let token = recognize_one("GF");
        let (atoms, provs) = resolve_atoms(&[token], &ctx, &meta);
        // gf_total (25) - opening_min (12) = 13, same numeric answer here, but flagged assumed.
        assert_eq!(atoms[0].hcp, 13..=37);
        assert!(provs[0].assumed);
    }

    #[test]
    fn min_max_split_previous_range() {
        let binding = Binding::default();
        let prev = node_with_hcp(12..=16);
        let mut ctx = base_ctx(&binding, Call::Pass, Role::Opener);
        ctx.own_prev = Some(&prev);
        let meta = SystemMeta::default();

        let min_token = recognize_one("MIN");
        let (atoms, _) = resolve_atoms(&[min_token], &ctx, &meta);
        assert_eq!(atoms[0].hcp, 12..=14);

        let max_token = recognize_one("MAX");
        let (atoms, _) = resolve_atoms(&[max_token], &ctx, &meta);
        assert_eq!(atoms[0].hcp, 15..=16);
    }

    #[test]
    fn weak_two_opener_uses_natural_params() {
        let binding = Binding::default();
        let call = Call::Bid(Bid::new(2, Strain::Hearts).unwrap());
        let ctx = base_ctx(&binding, call, Role::Opener);
        let meta = SystemMeta::default();
        let token = recognize_one("weak");
        let (atoms, _) = resolve_atoms(&[token], &ctx, &meta);
        assert_eq!(atoms[0].hcp, meta.natural.weak_two.1);
    }

    #[test]
    fn suit_len_hash_suit_resolves_from_context() {
        let binding = Binding::default();
        let mut ctx = base_ctx(&binding, Call::Pass, Role::Responder);
        ctx.hash_suit = Some(Suit::Hearts);
        let meta = SystemMeta::default();
        let token = recognize_one("5+#");
        let (atoms, provs) = resolve_atoms(&[token], &ctx, &meta);
        assert_eq!(
            atoms[0].shapes,
            ShapeSet::from_suit_len(Suit::Hearts, 5, 13)
        );
        assert_eq!(provs[0].source, Source::Context);
        assert!(!provs[0].assumed);
    }

    #[test]
    fn suit_len_hash_suit_assumed_when_absent() {
        let binding = Binding::default();
        let ctx = base_ctx(&binding, Call::Pass, Role::Responder);
        let meta = SystemMeta::default();
        let token = recognize_one("4+#");
        let (atoms, provs) = resolve_atoms(&[token], &ctx, &meta);
        assert_eq!(atoms[0].shapes, ShapeSet::ALL);
        assert!(provs[0].assumed);
    }

    #[test]
    fn natural_suit_bid_opener_one_major() {
        let binding = Binding::default();
        let call = Call::Bid(Bid::new(1, Strain::Spades).unwrap());
        let ctx = base_ctx(&binding, call, Role::Opener);
        let meta = SystemMeta::default();
        let token = recognize_one("NAT");
        let (atoms, provs) = resolve_atoms(&[token], &ctx, &meta);
        assert_eq!(
            atoms[0].shapes,
            ShapeSet::from_suit_len(Suit::Spades, meta.natural.open_1major_len, 13)
        );
        assert_eq!(provs[0].source, Source::NaturalDefault);
    }

    #[test]
    fn natural_notrump_is_balanced() {
        let binding = Binding::default();
        let call = Call::Bid(Bid::new(1, Strain::NoTrump).unwrap());
        let ctx = base_ctx(&binding, call, Role::Opener);
        let meta = SystemMeta::default();
        let token = recognize_one("NAT");
        let (atoms, _) = resolve_atoms(&[token], &ctx, &meta);
        assert_eq!(
            atoms[0].shapes,
            ShapeSet::from_classes(&meta.balanced.balanced)
        );
    }

    #[test]
    fn natural_pass_carries_nothing() {
        let binding = Binding::default();
        let ctx = base_ctx(&binding, Call::Pass, Role::Responder);
        let meta = SystemMeta::default();
        let token = recognize_one("NAT");
        let (atoms, _) = resolve_atoms(&[token], &ctx, &meta);
        assert_eq!(atoms[0], Atom::ANY);
    }

    #[test]
    fn balanced_uses_meta_classes() {
        let binding = Binding::default();
        let ctx = base_ctx(&binding, Call::Pass, Role::Opener);
        let meta = SystemMeta::default();
        let token = recognize_one("bal");
        let (atoms, _) = resolve_atoms(&[token], &ctx, &meta);
        assert_eq!(
            atoms[0].shapes,
            ShapeSet::from_classes(&meta.balanced.balanced)
        );
    }

    #[test]
    fn quality_word_builds_card_requirement() {
        let binding = Binding::default();
        let call = Call::Bid(Bid::new(1, Strain::Spades).unwrap());
        let ctx = base_ctx(&binding, call, Role::Opener);
        let meta = SystemMeta::default();
        let token = recognize_one("AKQ");
        let (atoms, provs) = resolve_atoms(&[token], &ctx, &meta);
        assert_eq!(atoms[0].cards.len(), 1);
        assert_eq!(provs[0].source, Source::Explicit);
    }

    #[test]
    fn convention_and_forcing_are_atom_any() {
        // `SPL` is deliberately excluded: unlike a plain named convention, it resolves to a
        // compound shortness/support/strength atom (see `splinter_*` tests below).
        let binding = Binding::default();
        let ctx = base_ctx(&binding, Call::Pass, Role::Opener);
        let meta = SystemMeta::default();
        for text in ["STAY", "F1", "unlimited"] {
            let token = recognize_one(text);
            let (atoms, _) = resolve_atoms(&[token], &ctx, &meta);
            assert_eq!(atoms[0], Atom::ANY, "{text} should carry no atom");
        }
    }

    #[test]
    fn splinter_builds_shortness_support_and_strength() {
        let binding = Binding::default();
        let call = Call::Bid(Bid::new(4, Strain::Clubs).unwrap());
        let mut ctx = base_ctx(&binding, call, Role::Opener);
        ctx.agreed_suit = Some(Suit::Hearts);
        let meta = SystemMeta::default();
        let token = recognize_one("SPL");
        let (atoms, provs) = resolve_atoms(&[token], &ctx, &meta);
        assert_eq!(
            atoms[0].shapes,
            ShapeSet::from_suit_len(Suit::Clubs, 0, 1).intersect(ShapeSet::from_suit_len(
                Suit::Hearts,
                meta.conventions.splinter_support,
                13
            ))
        );
        // gf_total (25) - assumed opening partner_min (12) = 13.
        assert_eq!(atoms[0].hcp, 13..=37);
        assert_eq!(provs[0].source, Source::Context);
    }

    #[test]
    fn mini_splinter_uses_invitational_strength() {
        let binding = Binding::default();
        let call = Call::Bid(Bid::new(3, Strain::Clubs).unwrap());
        let mut ctx = base_ctx(&binding, call, Role::Opener);
        ctx.agreed_suit = Some(Suit::Hearts);
        let mut partner = node_with_hcp(12..=14);
        partner.id = NodeId(1);
        ctx.partner_last = Some(&partner);
        let meta = SystemMeta::default();
        let token = recognize_one("mini-splinter");
        let (atoms, _) = resolve_atoms(&[token], &ctx, &meta);
        // inv_total 22..=24, partner_min 12: [10, 12] (same formula as bare `INV`).
        assert_eq!(atoms[0].hcp, 10..=12);
    }

    // Regression for the real-file triage (roadmap 3.2-3.4, class a): a splinter made right after
    // partner's own first suit call (the common case) used to be `assumed`-only for the support
    // suit, since `ctx.agreed_suit` is only set once the expansion loop has already tracked an
    // earlier support/fit call. `agreed_suit_or_partner_last` falls back to partner's last natural
    // suit bid so this ordinary case resolves like a real support requirement instead.
    #[test]
    fn splinter_falls_back_to_partners_last_natural_suit_when_no_agreed_suit() {
        let binding = Binding::default();
        let call = Call::Bid(Bid::new(4, Strain::Clubs).unwrap());
        let mut ctx = base_ctx(&binding, call, Role::Opener);
        let mut partner = node_with_hcp(12..=14);
        partner.call = Call::Bid(Bid::new(1, Strain::Hearts).unwrap());
        // The guard added for `example3.bml`'s 1C-1D-2S contradiction requires partner's
        // constraint to actually show length in the named suit, not just the same-strain call:
        // give this fixture a genuine 5+!h so the fallback still applies here.
        partner.constraint = HandConstraint::Atom(Atom {
            hcp: 12..=14,
            shapes: ShapeSet::from_suit_len(Suit::Hearts, 5, 13),
            ..Atom::ANY
        });
        ctx.partner_last = Some(&partner);
        // `ctx.agreed_suit` deliberately left `None`: nothing upstream has agreed a trump yet.
        let meta = SystemMeta::default();
        let token = recognize_one("SPL");
        let (atoms, provs) = resolve_atoms(&[token], &ctx, &meta);
        assert_eq!(
            atoms[0].shapes,
            ShapeSet::from_suit_len(Suit::Clubs, 0, 1).intersect(ShapeSet::from_suit_len(
                Suit::Hearts,
                meta.conventions.splinter_support,
                13
            ))
        );
        assert_eq!(provs[0].source, Source::Context);
    }

    // Same family: an artificial call (a puppet, a forcing relay) names no suit its bidder
    // actually holds, so it must not be treated as agreeing that suit -- a row that separately,
    // and correctly, describes shortness there (jdh8's `2N-3H-3N = !SPL, 0--1!h`) would otherwise
    // be forced to also support the very suit it denies.
    #[test]
    fn splinter_ignores_partners_artificial_last_call() {
        let binding = Binding::default();
        let call = Call::Bid(Bid::new(4, Strain::Clubs).unwrap());
        let mut ctx = base_ctx(&binding, call, Role::Opener);
        let mut partner = node_with_hcp(12..=14);
        partner.call = Call::Bid(Bid::new(3, Strain::Hearts).unwrap());
        partner.flags.artificial = true;
        ctx.partner_last = Some(&partner);
        let meta = SystemMeta::default();
        let token = recognize_one("SPL");
        let (atoms, provs) = resolve_atoms(&[token], &ctx, &meta);
        // No `suit_len[agreed]` conjunct at all (just the own-suit shortness): the artificial 3H
        // is not agreeing hearts.
        assert_eq!(atoms[0].shapes, ShapeSet::from_suit_len(Suit::Clubs, 0, 1));
        assert!(provs[0].assumed);
    }

    #[test]
    fn shape_token_reparses_raw_text() {
        let binding = Binding::default();
        let ctx = base_ctx(&binding, Call::Pass, Role::Opener);
        let meta = SystemMeta::default();
        let token = recognize_one("4414");
        let (atoms, _) = resolve_atoms(&[token], &ctx, &meta);
        // "4414" reads spades-hearts-diamonds-clubs (S4 H4 D1 C4); `Shape::new` takes C D H S.
        assert!(
            atoms[0]
                .shapes
                .contains(bridge_core::Shape::new(4, 1, 4, 4))
        );
        assert!(
            !atoms[0]
                .shapes
                .contains(bridge_core::Shape::new(3, 3, 3, 4))
        );
    }
}
