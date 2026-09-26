//! `AuctionPolicy`: the per-auction fast path of the policy likelihood (07-bidding.md §6.2).

use bridge_constraint::{HandConstraint, HcpShapeGrid};
use bridge_core::{Auction, Deal, Hand, Seat};

use crate::exclusion::{MirrorSpec, PieceRole, Reader, mirror_call};
use crate::{BidContext, ImplicitPass, PolicyParams, Scoring, Table, sequence_log_likelihood};

/// The exact membership test of one piece.
#[derive(Clone, Debug)]
enum Membership {
    /// A literal-free region: one grid lookup.
    Grid(Box<HcpShapeGrid>),
    /// Anything else: `HandConstraint::satisfies`.
    Constraint(HandConstraint),
}

impl Membership {
    fn contains(&self, hand: Hand) -> bool {
        match self {
            Membership::Grid(g) => g.contains(hand),
            Membership::Constraint(c) => c.satisfies(hand),
        }
    }
}

/// One call's term: `ln(floor + Σ_i raw_i · 1[h ∈ C_i])`.
#[derive(Clone, Debug)]
struct CallTerm {
    seat: Seat,
    /// The `ANY` piece's raw weight `ε/n` (every hand).
    floor: f64,
    /// The other pieces (pairwise disjoint), in their exact membership form.
    pieces: Vec<(Membership, f64)>,
}

/// The legacy (`legacy_temperature`) path, which the mirror does not describe: delegates to the
/// reference.
#[derive(Clone, Debug)]
struct Legacy {
    table: Table,
    scoring: Scoring,
    implicit_pass: ImplicitPass,
    natural: std::sync::Arc<bridge_system::NaturalInference>,
}

/// The policy likelihood of one fixed auction, prepared once and evaluated per deal.
///
/// `AuctionPolicy::new` builds, once per auction, each call's pieces (membership form: the exact
/// system regions of the `ExclusiveIndex`, the exact tree form of the natural regions, the
/// literal-free ones as `HcpShapeGrid`s) and raw weights under the policy of `ctx`;
/// [`AuctionPolicy::log_likelihood`] then evaluates `Σ_j ln Σ_i raw_{j,i}·1[h_{s_j} ∈ C_{j,i}]`
/// with membership tests only. It equals the reference [`sequence_log_likelihood`] with
/// `|Δ ln L| <= 1e-5` (`tests/policy.rs`). The natural engine is `ctx.natural`, or
/// `table.natural` when `ctx.natural` is `None` (as in `sequence_log_likelihood`).
///
/// Owns everything it needs, so it can live in a cache next to the interpretation of the same
/// auction ([`crate::InterpretCache::get_or_policy`]). Under
/// `PolicyParams::legacy_temperature` (which the mirror does not describe) it delegates to
/// `sequence_log_likelihood`.
#[derive(Clone, Debug)]
pub struct AuctionPolicy {
    auction: Auction,
    policy: PolicyParams,
    terms: Vec<CallTerm>,
    legacy: Option<Legacy>,
}

impl AuctionPolicy {
    /// Prepares the likelihood of `auction` under `table` and the policy of `ctx`.
    pub fn new(table: &Table, auction: &Auction, ctx: &BidContext<'_>) -> AuctionPolicy {
        if ctx.policy.legacy_temperature.is_some() {
            let natural = match ctx.natural {
                Some(engine) if !std::ptr::eq(engine, table.natural.as_ref()) => {
                    std::sync::Arc::new(engine.clone())
                }
                _ => table.natural.clone(),
            };
            return AuctionPolicy {
                auction: auction.clone(),
                policy: ctx.policy,
                terms: Vec::new(),
                legacy: Some(Legacy {
                    table: table.clone(),
                    scoring: ctx.scoring,
                    implicit_pass: ctx.implicit_pass,
                    natural,
                }),
            };
        }
        let natural = ctx.natural.unwrap_or(table.natural.as_ref());
        let spec = MirrorSpec {
            table,
            natural,
            policy: ctx.policy,
            implicit_pass: ctx.implicit_pass,
            strict: false,
            want_text: false,
        };
        let mut reader = Reader::new(table, natural, auction, ctx.implicit_pass);
        let mut prefix = Auction::new(auction.dealer(), auction.vulnerability());
        let mut terms = Vec::with_capacity(auction.len());
        for (j, &call) in auction.calls().iter().enumerate() {
            let m = mirror_call(&spec, &mut reader, &prefix, call);
            let mut floor = 0.0;
            let mut pieces = Vec::with_capacity(m.pieces.len());
            for piece in m.pieces {
                if piece.role == PieceRole::Any {
                    floor += piece.raw;
                    continue;
                }
                let membership = match piece.grid {
                    Some(g) => Membership::Grid(g),
                    None => {
                        let exact = piece.membership();
                        match HcpShapeGrid::of_exact(exact) {
                            Some(g) => Membership::Grid(Box::new(g)),
                            None => Membership::Constraint(exact.clone()),
                        }
                    }
                };
                pieces.push((membership, piece.raw));
            }
            terms.push(CallTerm {
                seat: auction.seat_at(j),
                floor,
                pieces,
            });
            prefix
                .push(call)
                .expect("call from a valid Auction is legal at its own position");
        }
        AuctionPolicy {
            auction: auction.clone(),
            policy: ctx.policy,
            terms,
            legacy: None,
        }
    }

    /// The auction this policy scores.
    pub fn auction(&self) -> &Auction {
        &self.auction
    }

    /// The policy parameters it was built with.
    pub fn policy(&self) -> PolicyParams {
        self.policy
    }

    /// `ln L(deal) = Σ_j ln p_j(calls[j] | hand of the caller of j)`: the same value as
    /// [`sequence_log_likelihood`] for this table, auction and context, up to `1e-5` (`-∞` never
    /// occurs for `ε > 0`).
    pub fn log_likelihood(&self, deal: &Deal) -> f64 {
        if let Some(legacy) = &self.legacy {
            let ctx = BidContext {
                scoring: legacy.scoring,
                natural: Some(legacy.natural.as_ref()),
                implicit_pass: legacy.implicit_pass,
                policy: self.policy,
            };
            return sequence_log_likelihood(&legacy.table, deal, &self.auction, &ctx);
        }
        let hands = [
            deal.hand(Seat::North),
            deal.hand(Seat::East),
            deal.hand(Seat::South),
            deal.hand(Seat::West),
        ];
        let mut total = 0.0f64;
        for term in &self.terms {
            let hand = hands[term.seat.index() as usize];
            let mut p = term.floor;
            // The system pieces are pairwise disjoint and so are the natural ones, but a hand can
            // be in one of each (with `δ > 0`), so every piece is tested.
            for (m, raw) in &term.pieces {
                if m.contains(hand) {
                    p += raw;
                }
            }
            total += p.ln();
        }
        total
    }
}
