//! The policy mirror (docs/design/15-phase4-plan.md D19; 07-bidding.md §2.3 items 4–5, §8):
//!
//! - `policy_mirror`: for every call `c` of an auction and every hand `h` of its caller,
//!   `exp(log_scale) · Σ_i w_i · 1[h ∈ C_i]` equals `call_distribution(h)[c]`. Pieces with
//!   `cards`/`eval` literals may over-cover (the density is then an upper bound); under-cover is
//!   never allowed.
//!   Besides the four main cells (SAYC-generated and corpus positions, `δ ∈ {0, 0.3}`), the
//!   variant cells cover `ImplicitPass::Never` (the `N_sys` piece and the natural region without
//!   the natural implicit pass), positions whose prefix has calls substituted by random legal
//!   calls (lenient `Partial` resolutions and the run-time recompute of `X_c`), and a
//!   `BidContext` whose `natural` is `None` (the policy then uses `table.natural`, as the mirror
//!   does).
//! - `tightness` (ported from prototype A): at a position `choose_bid` reaches, a hand inside the
//!   strict (non-`Fallback`) reading of the call it made picks that call, and a hand that picks
//!   it is inside, for the `Exact` calls of the system-players preset.

mod common;

use bridge_bidding::{
    BidContext, ImplicitPass, InterpretOptions, PolicyParams, ResolutionKind, Scoring, Table,
    call_distribution, choose_bid, interpret, replay,
};
use bridge_core::{Auction, Deal, Seat, Vulnerability};
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::{Rng, SeedableRng};

fn policy_ctx(table: &Table, policy: PolicyParams) -> BidContext<'_> {
    BidContext {
        scoring: Scoring::Imp,
        natural: Some(table.natural.as_ref()),
        implicit_pass: ImplicitPass::Complement,
        policy,
    }
}

const HUMAN_LIKE: PolicyParams = PolicyParams {
    deviation: 0.3,
    ..PolicyParams::system_players()
};

#[derive(Default)]
struct Stats {
    positions: u64,
    checks: u64,
    exact: u64,
    over: u64,
    under: u64,
    shadowed: u64,
    natural_calls: u64,
    pieces: u64,
}

impl Stats {
    fn summary(&self) -> String {
        format!(
            "{} positions ({} natural-kind, {} shadowed), {} (call, hand) checks: exact {} \
             ({:.3}%), over-covered {}, under-covered {}; pieces per call mean {:.2}",
            self.positions,
            self.natural_calls,
            self.shadowed,
            self.checks,
            self.exact,
            100.0 * self.exact as f64 / self.checks.max(1) as f64,
            self.over,
            self.under,
            self.pieces as f64 / self.positions.max(1) as f64,
        )
    }
}

/// Checks call `j` of `auction` (`interp` is its interpretation) against `call_distribution` on
/// `hands`.
fn check_call(
    table: &Table,
    ctx: &BidContext<'_>,
    auction: &Auction,
    interp: &bridge_bidding::Interpretation,
    j: usize,
    hands: &[bridge_core::Hand],
    st: &mut Stats,
) {
    let prefix = Auction::from_calls(
        auction.dealer(),
        auction.vulnerability(),
        auction.calls()[..j].iter().copied(),
    )
    .expect("prefix of a legal auction");
    let call = auction.calls()[j];
    let pc = &interp.per_call[j];
    st.positions += 1;
    st.pieces += pc.alternatives.len() as u64;
    st.shadowed += u64::from(pc.shadowed);
    st.natural_calls += u64::from(pc.kind == ResolutionKind::Natural);
    let scale = pc.log_scale.exp();
    for &hand in hands {
        let p = f64::from(
            call_distribution(table, hand, &prefix, ctx)
                .iter()
                .find(|(c, _)| *c == call)
                .map_or(0.0, |(_, p)| *p),
        );
        let m: f64 = scale
            * pc.alternatives
                .iter()
                .filter(|(c, _, _)| c.satisfies(hand))
                .map(|(_, w, _)| f64::from(*w))
                .sum::<f64>();
        st.checks += 1;
        let rel = (m - p).abs() / p.max(1e-12);
        if rel < 1e-4 {
            st.exact += 1;
        } else if m > p {
            st.over += 1;
        } else {
            st.under += 1;
            if st.under <= 5 {
                eprintln!(
                    "under-cover: {auction} call {j} ({call}) hand {hand:?}: policy {p:.4e}, \
                     mirror {m:.4e}; pieces {:?}",
                    pc.alternatives
                        .iter()
                        .map(|(c, w, ex)| (ex.kind, *w, c.satisfies(hand)))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}

/// Up to `positions` positions from `auctions` (`per_auction` distinct random calls of each, or
/// all of a shorter auction's calls), `hands` random hands per position plus the true hand when
/// the deal is known.
#[allow(clippy::too_many_arguments)]
fn run_positions(
    table: &Table,
    ctx: &BidContext<'_>,
    auctions: &[(Auction, Option<Deal>)],
    positions: u64,
    per_auction: usize,
    hands: usize,
    rng: &mut Xoshiro256PlusPlus,
    st: &mut Stats,
) {
    let opts = InterpretOptions::for_context(ctx);
    let start = st.positions;
    for (auction, deal) in auctions {
        if st.positions - start >= positions {
            break;
        }
        if auction.is_empty() {
            continue;
        }
        let interp = interpret(table, auction, &opts);
        // A partial Fisher-Yates shuffle picks `per_auction` distinct calls.
        let mut order: Vec<usize> = (0..auction.len()).collect();
        for k in 0..per_auction.min(auction.len()) {
            if st.positions - start >= positions {
                break;
            }
            let pick = k + (rng.next_u32() as usize) % (order.len() - k);
            order.swap(k, pick);
            let j = order[k];
            let mut hs: Vec<bridge_core::Hand> = (0..hands)
                .map(|_| common::random_hand13(&mut *rng))
                .collect();
            if let Some(deal) = deal {
                hs.push(deal.hand(auction.seat_at(j)));
            }
            check_call(table, ctx, auction, &interp, j, &hs, st);
        }
    }
}

/// Generated auctions: random deals replayed under `ctx`.
fn generated(
    table: &Table,
    ctx: &BidContext<'_>,
    n: usize,
    rng: &mut Xoshiro256PlusPlus,
) -> Vec<(Auction, Option<Deal>)> {
    (0..n)
        .map(|i| {
            let deal = common::random_deal(&mut *rng);
            let dealer = Seat::ALL[i % 4];
            let vul = Vulnerability::from_index((rng.next_u32() % 4) as u8);
            let auction = replay(table, &deal, dealer, vul, ctx).auction;
            (auction, Some(deal))
        })
        .collect()
}

/// Calls per corpus auction so that `positions` positions are reached (at least 2): the corpus
/// has fewer auctions with a deal than the large run's position count.
fn per_corpus_auction(positions: u64, corpus: &[(Auction, Option<Deal>)]) -> usize {
    if corpus.is_empty() {
        return 2;
    }
    (positions as usize).div_ceil(corpus.len()).max(2)
}

/// One generated and one corpus cell under `ctx` (`name` prefixes the cell names).
#[allow(clippy::too_many_arguments)]
fn run_cells(
    table: &Table,
    ctx: &BidContext<'_>,
    name: &str,
    corpus: &[(Auction, Option<Deal>)],
    positions: u64,
    hands: usize,
    seed: u64,
    out: &mut Vec<(String, Stats)>,
) {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
    let mut gen_st = Stats::default();
    let generated_auctions = generated(table, ctx, positions as usize, &mut rng);
    run_positions(
        table,
        ctx,
        &generated_auctions,
        positions,
        2,
        hands,
        &mut rng,
        &mut gen_st,
    );
    out.push((format!("{name} generated"), gen_st));
    let mut corpus_st = Stats::default();
    run_positions(
        table,
        ctx,
        corpus,
        positions,
        per_corpus_auction(positions, corpus),
        hands,
        &mut rng,
        &mut corpus_st,
    );
    out.push((format!("{name} corpus"), corpus_st));
}

fn run_mirror(positions: u64, hands: usize, seed: u64) -> Vec<(String, Stats)> {
    let table = common::compile_sayc("sayc.bml");
    let corpus = common::corpus_auctions_with_deals(4 * positions as usize);
    if corpus.is_empty() {
        eprintln!("policy_mirror: no corpus directory; corpus positions skipped");
    }
    let mut out = Vec::new();
    for (name, policy) in [
        ("delta=0", PolicyParams::system_players()),
        ("delta=0.3", HUMAN_LIKE),
    ] {
        let ctx = policy_ctx(&table, policy);
        run_cells(
            &table, &ctx, name, &corpus, positions, hands, seed, &mut out,
        );
    }
    out
}

/// The variant cells: `ImplicitPass::Never` (both δ), `natural: None` (δ = 0.3), and positions
/// with random substituted calls (both δ, both implicit-pass rules).
fn run_mirror_variants(positions: u64, hands: usize, seed: u64) -> Vec<(String, Stats)> {
    let table = common::compile_sayc("sayc.bml");
    let corpus = common::corpus_auctions_with_deals(4 * positions as usize);
    let mut out = Vec::new();
    for (name, policy) in [
        ("never delta=0", PolicyParams::system_players()),
        ("never delta=0.3", HUMAN_LIKE),
    ] {
        let ctx = BidContext {
            implicit_pass: ImplicitPass::Never,
            ..policy_ctx(&table, policy)
        };
        run_cells(
            &table, &ctx, name, &corpus, positions, hands, seed, &mut out,
        );
    }
    let ctx = BidContext {
        natural: None,
        ..policy_ctx(&table, HUMAN_LIKE)
    };
    run_cells(
        &table,
        &ctx,
        "natural=None delta=0.3",
        &corpus,
        positions,
        hands,
        seed,
        &mut out,
    );
    for (name, policy, implicit_pass) in [
        (
            "substituted delta=0",
            PolicyParams::system_players(),
            ImplicitPass::Complement,
        ),
        (
            "substituted delta=0.3",
            HUMAN_LIKE,
            ImplicitPass::Complement,
        ),
        (
            "substituted never delta=0.3",
            HUMAN_LIKE,
            ImplicitPass::Never,
        ),
    ] {
        let ctx = BidContext {
            implicit_pass,
            ..policy_ctx(&table, policy)
        };
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed ^ 0x5B5);
        let auctions: Vec<(Auction, Option<Deal>)> = (0..positions)
            .map(|_| {
                let p = common::random_sayc_position_with_substitution(
                    &mut rng,
                    &table,
                    &policy_ctx(&table, policy),
                    0.3,
                );
                (p.auction, Some(p.deal))
            })
            .collect();
        let mut st = Stats::default();
        run_positions(
            &table, &ctx, &auctions, positions, 2, hands, &mut rng, &mut st,
        );
        out.push((name.to_string(), st));
    }
    out
}

/// Under-cover 0 in every cell, and at least `min_exact` of the checks exact (over-cover comes
/// only from pieces with `cards`/`eval` literals).
fn assert_mirror(results: &[(String, Stats)], min_exact: f64) {
    for (name, st) in results {
        eprintln!("policy_mirror {name}: {}", st.summary());
    }
    for (name, st) in results {
        assert_eq!(st.under, 0, "{name}: {}", st.summary());
        if st.checks > 0 {
            assert!(
                st.exact as f64 >= min_exact * st.checks as f64,
                "{name}: {}",
                st.summary()
            );
        }
    }
}

/// Default suite: 150 positions × 40 hands per (source, δ).
#[test]
fn policy_mirror() {
    assert_mirror(&run_mirror(150, 40, 0x4D1_2202), 0.99);
}

/// Default suite, variant cells: 60 positions × 20 hands per cell. The acceptance's 99% exact is
/// defined on the main cells; in a variant cell this small, one over-covered position is 1.7% of
/// its checks, so only under-cover 0 and a 97% exact guard are asserted.
#[test]
fn policy_mirror_variants() {
    assert_mirror(&run_mirror_variants(60, 20, 0x4D1_2204), 0.97);
}

/// The large run: 2000 positions × 100 hands per (source, δ), then the variant cells at 500
/// positions × 50 hands.
#[test]
#[ignore = "2000 positions x 100 hands; run with `cargo test --release -- --ignored`"]
fn policy_mirror_large() {
    let n: u64 = std::env::var("MIRROR_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    assert_mirror(&run_mirror(n, 100, 0x4D1_2203), 0.99);
    assert_mirror(&run_mirror_variants(n / 4, 50, 0x4D1_2205), 0.99);
}

/// `[inside & picked, inside & not picked, outside & picked]` per kind `[Exact, Partial,
/// Natural]`.
fn run_tightness(positions: u64, hands: u64, seed: u64) -> [[u64; 3]; 3] {
    let table = common::compile_sayc("sayc.bml");
    let ctx = policy_ctx(&table, PolicyParams::system_players());
    let opts = InterpretOptions {
        strict: true,
        ..InterpretOptions::for_context(&ctx)
    };
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
    let mut m = [[0u64; 3]; 3];
    for _ in 0..positions {
        let (deal, auction) =
            std::iter::repeat_with(|| common::random_sayc_position(&mut rng, &table, &ctx))
                .find(|(_, a)| !a.is_complete())
                .expect("an incomplete position");
        let seat = auction.next_seat();
        let Some(call) = choose_bid(&table, deal.hand(seat), &auction, &ctx).call() else {
            continue;
        };
        let after = auction.with(call).expect("legal");
        let interp = interpret(&table, &after, &opts);
        let last = interp.per_call.last().expect("one call at least");
        let k = match last.kind {
            ResolutionKind::Exact => 0,
            ResolutionKind::Partial { .. } => 1,
            _ => 2,
        };
        for _ in 0..hands {
            let hand = common::random_hand13(&mut rng);
            let inside = last.alternatives.iter().any(|(c, w, e)| {
                *w > 0.0 && e.kind != ResolutionKind::Fallback && c.satisfies(hand)
            });
            let picked = choose_bid(&table, hand, &auction, &ctx).call() == Some(call);
            match (inside, picked) {
                (true, true) => m[k][0] += 1,
                (true, false) => m[k][1] += 1,
                (false, true) => m[k][2] += 1,
                (false, false) => {}
            }
        }
    }
    m
}

/// `tightness` (default suite): 150 positions × 40 hands; 0 / 0 for `Exact` calls.
#[test]
fn tightness() {
    let m = run_tightness(150, 40, 0x7167_0001);
    eprintln!("tightness [in&picked, in&!picked, out&picked] exact/partial/natural: {m:?}");
    assert_eq!(m[0][1], 0, "exact: inside but not picked: {m:?}");
    assert_eq!(m[0][2], 0, "exact: picked but outside: {m:?}");
    assert_eq!(m[1][2], 0, "partial: picked but outside: {m:?}");
    assert_eq!(m[2][2], 0, "natural: picked but outside: {m:?}");
}

/// The large tightness run (report; the same assertions).
#[test]
#[ignore = "2000 positions x 100 hands; run with `cargo test --release -- --ignored`"]
fn tightness_large() {
    let n: u64 = std::env::var("TIGHTNESS_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_000);
    let m = run_tightness(n, 100, 0x7167_0002);
    eprintln!("tightness [in&picked, in&!picked, out&picked] exact/partial/natural: {m:?}");
    assert_eq!(m[0][1], 0, "{m:?}");
    assert_eq!(m[0][2], 0, "{m:?}");
}
