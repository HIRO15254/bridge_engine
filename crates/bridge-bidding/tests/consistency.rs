//! Forward-consistency property (07-bidding.md end of §1, and §8's `forward_consistency` row):
//! `interpret` and `choose_bid` are meant to be inverses of each other, so whatever hand a call
//! was *chosen* for should also be accepted when that same call is later *interpreted*.

mod common;

use bridge_bidding::{
    BidChoice, BidContext, ImplicitPass, InterpretOptions, PolicyParams, ResolutionKind, Scoring,
    Table, choose_bid, interpret,
};
use bridge_core::{Auction, Deal, Seat, Vulnerability};
use common::*;
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;

fn table_of(sys: &Sayc) -> Table {
    Table::uniform(
        sys.sys.clone(),
        std::sync::Arc::new(bridge_system::NaturalInference::default()),
    )
}

/// Checks one call's worth of forward-consistency and returns the extended auction: whatever
/// `choose_bid` picked for `hand` must, once appended, be accepted by `interpret`'s own reading of
/// that same call (a non-`Fallback` alternative on its `per_call` entry that `hand` satisfies).
/// Checked via `per_call` directly, never `Interpretation::satisfied_by` (owned by a parallel lane
/// and still `todo!()` on this branch).
fn check_one_call(
    table: &Table,
    auction: &Auction,
    hand: bridge_core::Hand,
    ctx: &BidContext<'_>,
    opts: &InterpretOptions,
) -> Auction {
    let seat = auction.next_seat();
    let system = &table.systems[seat.index() as usize];
    let choice = choose_bid(system, hand, auction, ctx);
    let BidChoice::Chosen(chosen) = choice else {
        panic!("ImplicitPass::Complement guarantees a Chosen candidate whenever Pass is legal");
    };
    let extended = auction
        .with(chosen.call)
        .expect("choose_bid returns a legal call");
    let interp = interpret(table, &extended, opts);
    let pc = interp
        .per_call
        .last()
        .expect("the auction just grew by one call");
    assert_eq!(pc.seat, seat);
    assert_eq!(pc.call, chosen.call);
    assert!(
        pc.alternatives
            .iter()
            .any(|(c, w, ex)| ex.kind != ResolutionKind::Fallback && *w > 0.0 && c.satisfies(hand)),
        "seat {seat:?} hand {hand:?} was chosen to bid {:?}, but interpret's own reading of that \
         call does not accept the hand",
        chosen.call
    );
    extended
}

/// Same property as [`forward_consistency_opening_only`], but walked several calls deep into a
/// full, randomly-dealt auction instead of stopping after the opening.
///
/// `#[ignore]`d because, past the first round or two, a hand-built system as small as
/// `sayc_system()` inevitably runs off its own covered sequences (07-bidding.md §11's phase-3
/// scope only requires the rows listed in `tests/common`), and both `choose_bid` (via
/// `NaturalInference::candidates`) and `interpret` (via `classify`+`infer`) then fall through to
/// natural inference, which is still `todo!()` on this branch (owned by a parallel lane). Once
/// phase 4 supplies a real `NaturalInference`, this can be un-ignored as-is; the shallow
/// `forward_consistency_opening_only` test below covers the same property unconditionally for the
/// one round that never needs it.
#[test]
#[ignore = "relies on bridge_system::natural::classify/infer past the first round or two, still todo!() on this branch"]
fn forward_consistency() {
    let sys = sayc_system();
    let table = table_of(&sys);
    // `Some(&table.natural)`, not `None`: with `None`, a seat that runs off the hand-built
    // system's covered sequences simply yields `NoCandidate` (no natural fallback to try), which
    // would make this test pass vacuously without ever reaching the very `todo!()`s it exists to
    // document. Wiring in the real (still-`todo!()`) `NaturalInference` is what actually reaches
    // them, which is the whole reason this test stays `#[ignore]`d until phase 4.
    let ctx = BidContext {
        scoring: Scoring::Imp,
        natural: Some(table.natural.as_ref()),
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::default(),
    };
    let opts = InterpretOptions {
        strict: true,
        ..InterpretOptions::default()
    };

    let mut rng = Xoshiro256PlusPlus::seed_from_u64(7);
    for _ in 0..20 {
        let deal: Deal = random_deal(&mut rng);
        let mut auction = Auction::new(Seat::North, Vulnerability::None);
        for _ in 0..6 {
            if auction.is_complete() {
                break;
            }
            let seat = auction.next_seat();
            auction = check_one_call(&table, &auction, deal.hand(seat), &ctx, &opts);
        }
    }
}

/// A weak variant of the same property that does not need Natural fallback at all: the empty
/// auction's opening decision only ever needs the hand-built system's own opening rows (every
/// hand is either strong enough to open something, or the `Pass`-complement synthesises `Pass`),
/// so this direction is exercised unconditionally.
#[test]
fn forward_consistency_opening_only() {
    let sys = sayc_system();
    let table = table_of(&sys);
    let ctx = BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::default(),
    };
    let opts = InterpretOptions {
        strict: true,
        ..InterpretOptions::default()
    };

    let mut rng = Xoshiro256PlusPlus::seed_from_u64(42);
    for _ in 0..200 {
        let hand = random_hand13(&mut rng);
        let empty = Auction::new(Seat::North, Vulnerability::None);
        let system = &table.systems[Seat::North.index() as usize];

        let choice = choose_bid(system, hand, &empty, &ctx);
        let BidChoice::Chosen(chosen) = choice else {
            panic!("ImplicitPass::Complement guarantees a Chosen candidate at the opening");
        };
        let extended = empty.with(chosen.call).unwrap();
        let interp = interpret(&table, &extended, &opts);
        let pc = &interp.per_call[0];
        assert_eq!(pc.call, chosen.call);
        assert!(
            pc.alternatives
                .iter()
                .any(|(c, w, ex)| ex.kind != ResolutionKind::Fallback
                    && *w > 0.0
                    && c.satisfies(hand)),
            "hand {hand:?} was chosen to open {:?}, but interpret's own reading of that call does \
             not accept the hand",
            chosen.call
        );
    }
    let _ = Vulnerability::None;
}
