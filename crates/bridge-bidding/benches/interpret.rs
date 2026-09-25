//! Criterion benches for `interpret` (target: under 10 µs for a 12-call auction) and
//! `sequence_log_likelihood`.

use std::sync::Arc;

use bridge_bidding::{
    BidContext, ImplicitPass, InterpretOptions, PolicyParams, Scoring, Table, interpret,
    sequence_log_likelihood,
};
use bridge_constraint::{Atom, HandConstraint};
use bridge_core::{Auction, Bid, Call, Deal, Hand, Seat, Strain, Vulnerability};
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

fn bench_sequence_log_likelihood(c: &mut Criterion) {
    let system = Arc::new(bench_system());
    let natural = Arc::new(bridge_system::NaturalInference::default());
    let table = Table::uniform(system, natural);
    let auction = bench_auction();

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
    let deal = Deal::new(hands).unwrap();
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

criterion_group!(benches, bench_interpret, bench_sequence_log_likelihood);
criterion_main!(benches);
