//! Criterion benches for `interpret` (target: under 10 µs for a 12-call auction) and
//! `sequence_log_likelihood`.
//!
//! Two systems: `bench_system` (bare HCP atoms only, so every node's `shapes()` is exactly
//! `ShapeSet::ALL`) and `bench_system_realistic` (each node also names its own suit's length).
//! `summary_satisfiable`'s `shapes == ShapeSet::ALL` shortcut means the bare-HCP system alone
//! cannot exercise the `ShapeSet::min_hcp`/`max_hcp` walk in Step B's cross-product pre-check —
//! nearly every real system node restricts at least one suit's length, so both are benched.

use std::sync::Arc;

use bridge_bidding::{
    BidContext, ImplicitPass, InterpretOptions, PolicyParams, Scoring, Table, interpret,
    sequence_log_likelihood,
};
use bridge_constraint::{Atom, HandConstraint};
use bridge_core::{Auction, Bid, Call, Deal, Hand, Seat, Strain, Suit, Vulnerability};
use bridge_system::ast::{SeatCond, VulCond};
use bridge_system::pattern::{Binding, Side};
use bridge_system::{
    Alertability, Forcing, Node, NodeFlags, NodeId, Recognition, Row, RowId, SystemIR, SystemMeta,
};
use criterion::{Criterion, criterion_group, criterion_main};

fn bid(level: u8, strain: Strain) -> Call {
    Call::Bid(Bid::new(level, strain).unwrap())
}

fn atom_hcp(lo: u8, hi: u8) -> HandConstraint {
    HandConstraint::Atom(Atom::ANY.with_hcp(lo..=hi))
}

/// `suit` has length `>=lo`, HCP in `hcp_lo..=hcp_hi`: a *realistic* node constraint (an opening
/// bid or raise always says something about its own suit), unlike `atom_hcp`'s bare-HCP atom
/// whose `shapes()` is exactly `ShapeSet::ALL` — the one case `summary_satisfiable`/`Summary::of`
/// can skip the `ShapeSet::min_hcp`/`max_hcp` walk on for free. See `bench_system_realistic`.
fn atom_suit_hcp(suit: Suit, lo: u8, hcp_lo: u8, hcp_hi: u8) -> HandConstraint {
    HandConstraint::Atom(
        Atom::ANY
            .with_suit_len(suit, lo..=13)
            .with_hcp(hcp_lo..=hcp_hi),
    )
}

/// `Strain::Clubs..=Strain::Spades` map onto `Suit` at identical indices; `bench_system`/
/// `bench_system_realistic` never bid `Strain::NoTrump`.
fn strain_to_suit(strain: Strain) -> Suit {
    match strain {
        Strain::Clubs => Suit::Clubs,
        Strain::Diamonds => Suit::Diamonds,
        Strain::Hearts => Suit::Hearts,
        Strain::Spades => Suit::Spades,
        Strain::NoTrump => unreachable!("bench auctions never bid notrump"),
    }
}

/// Builds a minimal node (most fields are irrelevant to `interpret`'s hot path).
///
/// `we_opened` selects which partnership's trie root this call is attached under (07-bidding.md
/// §3.2 / `AuctionTrie::root_id`): `LookupKey::for_auction` computes it as "does the calling
/// seat's side match the auction's opener's side", so with `Table::uniform` sharing one
/// `SystemIR` across all four seats, a 12-call auction where all four seats make real (non-Pass)
/// calls needs both roots populated at the positions where each side actually calls — never just
/// `true` for every node, or the opposing side's own calls have no trie coverage at all and fall
/// through to `NaturalInference` (still `todo!()` on this branch).
#[allow(clippy::too_many_arguments)]
fn make_node(
    ir: &mut SystemIR,
    we_opened: bool,
    calls: Vec<Call>,
    call: Call,
    constraint: HandConstraint,
    seat: SeatCond,
    vul: VulCond,
    description: &str,
    priority: i16,
) -> NodeId {
    let id = NodeId(ir.nodes.len() as u32);
    let row_id = RowId(ir.rows.len() as u32);
    ir.rows.push(Row {
        id: row_id,
        span: bridge_system::ast::Span {
            file: bridge_system::ast::FileId(0),
            line: 1,
            col: 0,
            pasted_from: None,
        },
        path: Arc::from(vec![]),
        description_raw: description.to_string(),
        recognition: Recognition::default(),
        expansions: vec![id],
    });
    ir.index.insert(we_opened, &calls, seat, vul, id).ok();
    ir.nodes.push(Node {
        id,
        row: row_id,
        side: Side::Us,
        path: Arc::from(vec![]),
        calls,
        call,
        binding: Binding::default(),
        seat,
        vul,
        constraint,
        branch_weights: None,
        priority,
        volume_log2: 0,
        alertable: Alertability::Unspecified,
        flags: NodeFlags {
            forcing: Forcing::Unknown,
            ..NodeFlags::default()
        },
        description: description.to_string(),
        children: Vec::new(),
    });
    id
}

/// A tiny hand-built SAYC-like system deep enough for a 12-call auction: 1C opening, overcalls
/// and rebids through 1C-1D-1H-1S-2C-2D-2H-2S-3C-3D-3H-3S (each level just re-describes "more of
/// the same" so the auction stays legal and in-system throughout).
fn bench_system() -> SystemIR {
    let mut ir = SystemIR {
        meta: SystemMeta::default(),
        rows: Vec::new(),
        nodes: Vec::new(),
        index: bridge_system::AuctionTrie::new(),
        lints: Vec::new(),
    };

    let strains = [
        Strain::Clubs,
        Strain::Diamonds,
        Strain::Hearts,
        Strain::Spades,
    ];
    let mut path = Vec::new();
    let mut j = 0usize; // Flat call index into the 12-call auction (North deals, so seat parity
    // alternates N,E,S,W,...; even j = North/South's own call, odd j = East/West's).
    for level in 1..=3u8 {
        for &strain in &strains {
            let call = bid(level, strain);
            let mut calls = path.clone();
            calls.push(call);
            make_node(
                &mut ir,
                j % 2 == 0,
                calls.clone(),
                call,
                atom_hcp(10, 24),
                SeatCond::Any,
                VulCond::default(),
                "12 calls of natural bidding",
                0,
            );
            path = calls;
            j += 1;
        }
    }
    ir
}

/// Same shape as `bench_system`, but every node's constraint names its own suit (`>=4` cards,
/// `10-24` HCP) instead of a bare HCP atom — the realistic case `summary_satisfiable`'s
/// `shapes == ShapeSet::ALL` shortcut cannot take, and so the case `Summary`'s incremental
/// `ShapeSet::min_hcp`/`max_hcp` bounds (only recomputed when the shape set actually narrows) are
/// for. See the module doc for why the plain `bench_system` alone hid this cost.
fn bench_system_realistic() -> SystemIR {
    let mut ir = SystemIR {
        meta: SystemMeta::default(),
        rows: Vec::new(),
        nodes: Vec::new(),
        index: bridge_system::AuctionTrie::new(),
        lints: Vec::new(),
    };

    let strains = [
        Strain::Clubs,
        Strain::Diamonds,
        Strain::Hearts,
        Strain::Spades,
    ];
    let mut path = Vec::new();
    let mut j = 0usize;
    for level in 1..=3u8 {
        for &strain in &strains {
            let call = bid(level, strain);
            let mut calls = path.clone();
            calls.push(call);
            make_node(
                &mut ir,
                j % 2 == 0,
                calls.clone(),
                call,
                atom_suit_hcp(strain_to_suit(strain), 4, 10, 24),
                SeatCond::Any,
                VulCond::default(),
                "12 calls of natural bidding, own-suit length required",
                0,
            );
            path = calls;
            j += 1;
        }
    }
    ir
}

fn bench_auction() -> Auction {
    let strains = [
        Strain::Clubs,
        Strain::Diamonds,
        Strain::Hearts,
        Strain::Spades,
    ];
    let mut calls = Vec::new();
    for level in 1..=3u8 {
        for &strain in &strains {
            calls.push(bid(level, strain));
        }
    }
    Auction::from_calls(Seat::North, Vulnerability::None, calls).unwrap()
}

fn bench_interpret(c: &mut Criterion) {
    let system = Arc::new(bench_system());
    let natural = Arc::new(bridge_system::NaturalInference::default());
    let table = Table::uniform(system, natural);
    let auction = bench_auction();
    let opts = InterpretOptions::default();

    c.bench_function("interpret/12-call-auction", |b| {
        b.iter(|| std::hint::black_box(interpret(&table, &auction, &opts)))
    });
}

/// An arbitrary, fixed 4-way deal (a fresh full-deck deal in index order), used only to give
/// `sequence_log_likelihood`'s benches a `Deal` to score; the hands' actual content is irrelevant
/// to the timing.
fn bench_deal() -> Deal {
    let deck = Hand::FULL;
    let cards: Vec<_> = deck.cards().collect();
    let hands = [
        Hand::from_bits(
            cards[0..13]
                .iter()
                .fold(0u64, |acc, c| acc | (1u64 << c.index())),
        ),
        Hand::from_bits(
            cards[13..26]
                .iter()
                .fold(0u64, |acc, c| acc | (1u64 << c.index())),
        ),
        Hand::from_bits(
            cards[26..39]
                .iter()
                .fold(0u64, |acc, c| acc | (1u64 << c.index())),
        ),
        Hand::from_bits(
            cards[39..52]
                .iter()
                .fold(0u64, |acc, c| acc | (1u64 << c.index())),
        ),
    ]
    .map(|h| h.unwrap());
    Deal::new(hands).unwrap()
}

fn bench_sequence_log_likelihood(c: &mut Criterion) {
    let system = Arc::new(bench_system());
    let natural = Arc::new(bridge_system::NaturalInference::default());
    let table = Table::uniform(system, natural);
    let auction = bench_auction();
    let deal = bench_deal();
    let ctx = BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Never,
        policy: PolicyParams::default(),
    };

    c.bench_function("sequence_log_likelihood/12-call-auction", |b| {
        b.iter(|| std::hint::black_box(sequence_log_likelihood(&table, &deal, &auction, &ctx)))
    });
}

/// Same as `bench_interpret`, but with `bench_system_realistic` (07-bidding.md §4.4.2's cross
/// product now has real per-suit summaries to intersect, not the all-shapes case).
fn bench_interpret_realistic(c: &mut Criterion) {
    let system = Arc::new(bench_system_realistic());
    let natural = Arc::new(bridge_system::NaturalInference::default());
    let table = Table::uniform(system, natural);
    let auction = bench_auction();
    let opts = InterpretOptions::default();

    c.bench_function("interpret/12-call-auction-realistic", |b| {
        b.iter(|| std::hint::black_box(interpret(&table, &auction, &opts)))
    });
}

/// Same as `bench_sequence_log_likelihood`, but with `bench_system_realistic`.
fn bench_sequence_log_likelihood_realistic(c: &mut Criterion) {
    let system = Arc::new(bench_system_realistic());
    let natural = Arc::new(bridge_system::NaturalInference::default());
    let table = Table::uniform(system, natural);
    let auction = bench_auction();
    let deal = bench_deal();
    let ctx = BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Never,
        policy: PolicyParams::default(),
    };

    c.bench_function("sequence_log_likelihood/12-call-auction-realistic", |b| {
        b.iter(|| std::hint::black_box(sequence_log_likelihood(&table, &deal, &auction, &ctx)))
    });
}

criterion_group!(
    benches,
    bench_interpret,
    bench_sequence_log_likelihood,
    bench_interpret_realistic,
    bench_sequence_log_likelihood_realistic
);
criterion_main!(benches);
