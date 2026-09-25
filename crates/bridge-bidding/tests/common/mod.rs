//! A small, hand-built `SystemIR` for most of these integration tests, plus (`compile_sayc`)
//! the real, compiled `systems/sayc/sayc.bml` used by the phase 3.10/3.12 harnesses
//! (`tests/consistency.rs`, `tests/policy.rs`, `tests/reproduction.rs`). The hand-built system
//! predates `bridge_system::compile`/`NaturalInference` having real bodies (07-bidding.md §11's
//! original phase-3 plan); it stays in use by the smaller, targeted unit tests below, which do
//! not need a full compiled system to exercise one specific behaviour.
#![allow(dead_code)]

use std::sync::Arc;

use bridge_constraint::{Atom, HandConstraint};
use bridge_core::{
    Auction, Bid, Call, Card, Deal, Hand, Holding, Rank, Seat, ShapeClass, Strain, Suit,
    Vulnerability,
};
use bridge_system::ast::{FileId, SeatCond, Span, VulCond};
use bridge_system::pattern::{Binding, OppClass, Side};
use bridge_system::trie::Edge;
use bridge_system::{
    Alertability, AuctionTrie, Forcing, Node, NodeFlags, NodeId, Recognition, Row, RowId, SystemIR,
    SystemMeta,
};
use rand_xoshiro::rand_core::Rng;

/// `1of(strain)`.
pub fn bid(level: u8, strain: Strain) -> Call {
    Call::Bid(Bid::new(level, strain).unwrap())
}

pub const PASS: Call = Call::Pass;
pub const DBL: Call = Call::Double;
pub const RDBL: Call = Call::Redouble;

/// An HCP-only atom.
pub fn atom_hcp(lo: u8, hi: u8) -> HandConstraint {
    HandConstraint::Atom(Atom::ANY.with_hcp(lo..=hi))
}

/// `suit` has length in `lo..=hi`, HCP in `hcp_lo..=hcp_hi`.
pub fn atom_suit_hcp(suit: Suit, lo: u8, hi: u8, hcp_lo: u8, hcp_hi: u8) -> HandConstraint {
    HandConstraint::Atom(
        Atom::ANY
            .with_suit_len(suit, lo..=hi)
            .with_hcp(hcp_lo..=hcp_hi),
    )
}

/// Balanced (4333/4432/5332), HCP in `hcp_lo..=hcp_hi`.
pub fn atom_balanced(hcp_lo: u8, hcp_hi: u8) -> HandConstraint {
    let mut atom = Atom::ANY.with_hcp(hcp_lo..=hcp_hi);
    atom.shapes = bridge_constraint::ShapeSet::from_classes(&[
        ShapeClass::C4333,
        ShapeClass::C4432,
        ShapeClass::C5332,
    ]);
    HandConstraint::Atom(atom)
}

fn dummy_span() -> Span {
    Span {
        file: FileId(0),
        line: 1,
        col: 0,
        pasted_from: None,
    }
}

/// A minimal `SystemMeta` (default distribution/tie-break/etc; `NaturalParams::default()` is the
/// SAYC-like table from 06-system.md §8.1, and is *not* `todo!()` — only `infer`/`classify`/
/// `candidates` are).
pub fn meta() -> SystemMeta {
    SystemMeta::default()
}

/// Builder for a hand-built [`SystemIR`]: every [`insert`](SystemBuilder::insert) call both adds
/// a [`Node`]/[`Row`] and inserts it into the [`AuctionTrie`] (via
/// [`AuctionTrie::insert`]/[`AuctionTrie::insert_path`]), so the IR and the index never diverge.
pub struct SystemBuilder {
    pub ir: SystemIR,
}

impl SystemBuilder {
    pub fn new() -> SystemBuilder {
        SystemBuilder {
            ir: SystemIR {
                meta: meta(),
                rows: Vec::new(),
                nodes: Vec::new(),
                index: AuctionTrie::new(),
                lints: Vec::new(),
            },
        }
    }

    /// Inserts a node reached by an all-concrete-call path (the common case).
    #[allow(clippy::too_many_arguments)]
    pub fn insert(
        &mut self,
        we_opened: bool,
        calls: &[Call],
        call: Call,
        constraint: HandConstraint,
        seat: SeatCond,
        vul: VulCond,
        description: &str,
        priority: i16,
    ) -> NodeId {
        let path: Vec<Edge> = calls.iter().copied().map(Edge::Call).collect();
        self.insert_path(
            we_opened,
            &path,
            calls.to_vec(),
            call,
            constraint,
            seat,
            vul,
            description,
            priority,
        )
    }

    /// Inserts a node reached by a path that may include an opponents'-interference wildcard
    /// (`Edge::Class`); `calls` is the node's own concrete-call history (used for its `Node`
    /// bookkeeping, not for the trie walk, which follows `path`).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_path(
        &mut self,
        we_opened: bool,
        path: &[Edge],
        calls: Vec<Call>,
        call: Call,
        constraint: HandConstraint,
        seat: SeatCond,
        vul: VulCond,
        description: &str,
        priority: i16,
    ) -> NodeId {
        let id = NodeId(self.ir.nodes.len() as u32);
        let row_id = RowId(self.ir.rows.len() as u32);
        self.ir.rows.push(Row {
            id: row_id,
            span: dummy_span(),
            path: Arc::from(Vec::new()),
            description_raw: description.to_string(),
            recognition: Recognition::default(),
            expansions: vec![id],
        });
        self.ir
            .index
            .insert_path(we_opened, path, seat, vul, id)
            .expect("test systems never insert the same (path, seat, vul) twice");
        self.ir.nodes.push(Node {
            id,
            row: row_id,
            side: Side::Us,
            path: Arc::from(Vec::new()),
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
            flags: NodeFlags::default(),
            description: description.to_string(),
            children: Vec::new(),
        });
        id
    }

    /// Like [`SystemBuilder::insert`], but for a node whose constraint is a top-level `Or` of
    /// `branches`, weighted by `weights` (or equally if `None`).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_or(
        &mut self,
        we_opened: bool,
        calls: &[Call],
        call: Call,
        branches: Vec<HandConstraint>,
        weights: Option<Vec<f32>>,
        seat: SeatCond,
        vul: VulCond,
        description: &str,
        priority: i16,
    ) -> NodeId {
        let id = self.insert(
            we_opened,
            calls,
            call,
            HandConstraint::Or(branches),
            seat,
            vul,
            description,
            priority,
        );
        self.ir.nodes[id.0 as usize].branch_weights = weights;
        id
    }

    /// Sets a node's forcing flag after the fact (for the `partner_constraint`/`forcing_situation`
    /// plumbing, not exercised much while `classify`/`infer` are `todo!()`).
    pub fn set_forcing(&mut self, id: NodeId, forcing: Forcing) {
        self.ir.nodes[id.0 as usize].flags.forcing = forcing;
    }

    pub fn build(self) -> SystemIR {
        self.ir
    }
}

/// A small SAYC-like system, deliberately including:
///
/// - an implicit opening pass (no `Pass` row at the root: §4.1 step 2's complement);
/// - a top-level `Or` node with `branch_weights` (`1S`'s two-range opening);
/// - a deliberate contradiction between an opening and a later rebid by the same seat
///   (`1C` 12–14 HCP vs. its own `3NT` rebid needing 25–27 HCP), for
///   `and_combination_drops_contradictions`;
/// - opponents' interference handled two ways: a concrete `Double`/`Redouble` sub-tree
///   (plain [`SystemBuilder::insert`]) and a wildcard overcall class
///   (`SystemBuilder::insert_path` with [`Edge::Class`]), demonstrating negative doubles;
/// - an implicit mid-auction pass at `1H-(X)-Pass` (no row at that depth: §4.1.5.1);
/// - an implicit pass reached *through* a deeper row (`ew_pass_2nd_over_1c`'s path structurally
///   creates East's own unrowed pass at `1C-Pass`), exercising the parent-position resolution in
///   `interpret::step_a_call`'s `d == n_k` branch.
pub struct Sayc {
    pub sys: Arc<SystemIR>,
    pub one_c: NodeId,
    pub one_d: NodeId,
    pub one_h: NodeId,
    pub one_s: NodeId,
    pub one_nt: NodeId,
    pub resp_1s: NodeId,
    pub resp_2h_raise: NodeId,
    pub resp_1d: NodeId,
    pub rebid_3nt: NodeId,
    pub dbl_redouble: NodeId,
    pub dbl_2h: NodeId,
    pub neg_dbl: NodeId,
    pub ew_over_1c: NodeId,
    pub ew_over_1h_nt: NodeId,
    pub ew_over_1h_dbl: NodeId,
    pub ew_pass_2nd_over_1c: NodeId,
}

pub fn sayc_system() -> Sayc {
    let mut b = SystemBuilder::new();

    let one_c = b.insert(
        true,
        &[bid(1, Strain::Clubs)],
        bid(1, Strain::Clubs),
        atom_suit_hcp(Suit::Clubs, 3, 13, 12, 14),
        SeatCond::Any,
        VulCond::default(),
        "12-14, 3+ clubs, no 5-card major, not balanced",
        0,
    );
    let one_d = b.insert(
        true,
        &[bid(1, Strain::Diamonds)],
        bid(1, Strain::Diamonds),
        atom_suit_hcp(Suit::Diamonds, 3, 13, 12, 14),
        SeatCond::Any,
        VulCond::default(),
        "12-14, 3+ diamonds, no 5-card major, not balanced",
        0,
    );
    let one_h = b.insert(
        true,
        &[bid(1, Strain::Hearts)],
        bid(1, Strain::Hearts),
        atom_suit_hcp(Suit::Hearts, 5, 13, 12, 21),
        SeatCond::Any,
        VulCond::default(),
        "12-21, 5+ hearts",
        0,
    );
    let one_s = b.insert_or(
        true,
        &[bid(1, Strain::Spades)],
        bid(1, Strain::Spades),
        vec![
            atom_suit_hcp(Suit::Spades, 5, 13, 12, 14),
            atom_suit_hcp(Suit::Spades, 5, 13, 18, 21),
        ],
        Some(vec![0.6, 0.4]),
        SeatCond::Any,
        VulCond::default(),
        "5+ spades, 12-14 or 18-21 (skipping the middle)",
        0,
    );
    let one_nt = b.insert(
        true,
        &[bid(1, Strain::NoTrump)],
        bid(1, Strain::NoTrump),
        atom_balanced(15, 17),
        SeatCond::Any,
        VulCond::default(),
        "15-17 balanced",
        0,
    );

    // Responses to 1H (opener's partner's turn; the trie stores our own calls only, opponents'
    // Pass in between needs no row).
    let resp_1s = b.insert(
        true,
        &[bid(1, Strain::Hearts), PASS, bid(1, Strain::Spades)],
        bid(1, Strain::Spades),
        atom_suit_hcp(Suit::Spades, 4, 13, 6, 10),
        SeatCond::Any,
        VulCond::default(),
        "4+ spades, 6-10, new suit",
        0,
    );
    let resp_2h_raise = b.insert(
        true,
        &[bid(1, Strain::Hearts), PASS, bid(2, Strain::Hearts)],
        bid(2, Strain::Hearts),
        atom_suit_hcp(Suit::Hearts, 3, 13, 6, 9),
        SeatCond::Any,
        VulCond::default(),
        "3+ hearts, 6-9, simple raise",
        0,
    );

    // Response to 1C (its own entry, distinct from the deeper rebid_3nt path below, so that
    // interpreting "1D" on its own does not fall through Step A's defensive path).
    let resp_1d = b.insert(
        true,
        &[bid(1, Strain::Clubs), PASS, bid(1, Strain::Diamonds)],
        bid(1, Strain::Diamonds),
        atom_suit_hcp(Suit::Diamonds, 4, 13, 6, 10),
        SeatCond::Any,
        VulCond::default(),
        "4+ diamonds, 6-10, new suit",
        0,
    );

    // Opener's rebid over 1H-P-1S-P: a deliberate contradiction with the 1C opening (never
    // reached in the same auction as 1C, but AND-combined by Step B when both calls belong to
    // 1C's own follow-up auction "1C-P-1D-P-3NT").
    let rebid_3nt = b.insert(
        true,
        &[
            bid(1, Strain::Clubs),
            PASS,
            bid(1, Strain::Diamonds),
            PASS,
            bid(3, Strain::NoTrump),
        ],
        bid(3, Strain::NoTrump),
        atom_hcp(25, 27),
        SeatCond::Any,
        VulCond::default(),
        "25-27, jump rebid (contradicts the 1C opening's 12-14)",
        0,
    );

    // Opponents double our 1H: redouble (10+, SOS) or a raise (support, 6-9). The implicit pass
    // at "1H-(X)-Pass" is deliberately *not* inserted.
    let dbl_redouble = b.insert(
        true,
        &[bid(1, Strain::Hearts), DBL, RDBL],
        RDBL,
        atom_hcp(10, 21),
        SeatCond::Any,
        VulCond::default(),
        "10+, redouble for penalty/SOS",
        0,
    );
    let dbl_2h = b.insert(
        true,
        &[bid(1, Strain::Hearts), DBL, bid(2, Strain::Hearts)],
        bid(2, Strain::Hearts),
        atom_suit_hcp(Suit::Hearts, 3, 13, 6, 9),
        SeatCond::Any,
        VulCond::default(),
        "3+ hearts, 6-9, raise over the double",
        0,
    );

    // Opponents overcall in any suit over our 1H: negative double shows the other major.
    let neg_dbl = b.insert_path(
        true,
        &[
            Edge::Call(bid(1, Strain::Hearts)),
            Edge::Class(OppClass::AnySuitBid),
            Edge::Call(DBL),
        ],
        vec![bid(1, Strain::Hearts), DBL],
        DBL,
        atom_suit_hcp(Suit::Spades, 4, 13, 6, 21),
        SeatCond::Any,
        VulCond::default(),
        "4+ spades, negative double",
        0,
    );

    // The "they opened" arena (`we_opened = false`): with `Table::uniform`, every seat's own
    // calls are interpreted through *this same* system, so East/West's own actions after North
    // (or South) opens must resolve here too, or Step A falls through to natural inference
    // (`classify`/`infer`, still `todo!()` on this branch, see the module doc).
    let ew_over_1c = b.insert(
        false,
        &[bid(1, Strain::Clubs), bid(1, Strain::NoTrump)],
        bid(1, Strain::NoTrump),
        atom_balanced(15, 18),
        SeatCond::Any,
        VulCond::default(),
        "1NT overcall, 15-18 balanced",
        0,
    );
    let ew_over_1h_nt = b.insert(
        false,
        &[bid(1, Strain::Hearts), bid(1, Strain::NoTrump)],
        bid(1, Strain::NoTrump),
        atom_balanced(15, 18),
        SeatCond::Any,
        VulCond::default(),
        "1NT overcall, 15-18 balanced",
        0,
    );
    let ew_over_1h_dbl = b.insert(
        false,
        &[bid(1, Strain::Hearts), DBL],
        DBL,
        atom_hcp(12, 21),
        SeatCond::Any,
        VulCond::default(),
        "takeout double, 12+",
        0,
    );

    // West's second pass over "1C-P-1D-P" (`we_opened: false`): a genuine row, not a per-level
    // workaround. Inserting it (via `insert`, all-concrete-call edges) structurally creates every
    // intermediate trie node along its path (06-system.md §4.3), including East's own *first*
    // pass at `[1C, Pass]` — which gets no row of its own. That is exactly the
    // implicit-pass-through-a-deeper-row shape `interpret::step_a_call`'s `d == n_k` branch
    // resolves (07-bidding.md §4.1.5.1): East's pass is picked up as the complement of `1C`'s
    // real children (`ew_over_1c`) at the *parent* position, not treated as off-system. A real
    // BML-compiled system would have deeper EW continuations everywhere and never need a row
    // spelled out by hand just for this; this one row is what keeps
    // `and_combination_drops_contradictions`'s auction (which walks through both EW passes to
    // reach North's own rebid) resolvable without ever reaching `NaturalInference`, still
    // `todo!()` on this branch — and, as a side effect, is exactly the shape
    // `implicit_pass_through_deeper_row` below exercises directly.
    let ew_pass_2nd_over_1c = b.insert(
        false,
        &[bid(1, Strain::Clubs), PASS, bid(1, Strain::Diamonds), PASS],
        PASS,
        HandConstraint::ANY,
        SeatCond::Any,
        VulCond::default(),
        "pass, insufficient to act",
        0,
    );

    Sayc {
        sys: Arc::new(b.build()),
        one_c,
        one_d,
        one_h,
        one_s,
        one_nt,
        resp_1s,
        resp_2h_raise,
        resp_1d,
        rebid_3nt,
        dbl_redouble,
        dbl_2h,
        neg_dbl,
        ew_over_1c,
        ew_over_1h_nt,
        ew_over_1h_dbl,
        ew_pass_2nd_over_1c,
    }
}

/// Builds an auction, panicking on illegal input (test convenience).
pub fn auction(dealer: Seat, vul: Vulnerability, calls: &[Call]) -> Auction {
    Auction::from_calls(dealer, vul, calls.iter().copied()).expect("test auction must be legal")
}

/// Builds a 13-card hand from four suit holdings (clubs, diamonds, hearts, spades), each a string
/// of rank characters (`"AKQJT98765432"`, case-insensitive, any subset, any order). Mirrors
/// `bridge-system`'s own test helper of the same name/signature.
pub fn hand(clubs: &str, diamonds: &str, hearts: &str, spades: &str) -> Hand {
    Hand::from_holdings(
        holding(clubs),
        holding(diamonds),
        holding(hearts),
        holding(spades),
    )
}

/// Parses one suit's ranks (see [`hand`]).
pub fn holding(ranks: &str) -> Holding {
    ranks.chars().fold(Holding::EMPTY, |h, c| h.with(rank(c)))
}

fn rank(c: char) -> Rank {
    match c.to_ascii_uppercase() {
        'A' => Rank::Ace,
        'K' => Rank::King,
        'Q' => Rank::Queen,
        'J' => Rank::Jack,
        'T' => Rank::Ten,
        '9' => Rank::Nine,
        '8' => Rank::Eight,
        '7' => Rank::Seven,
        '6' => Rank::Six,
        '5' => Rank::Five,
        '4' => Rank::Four,
        '3' => Rank::Three,
        '2' => Rank::Two,
        other => panic!("not a rank: {other:?}"),
    }
}

/// A fixed 0-HCP, short-hearts 13-card hand (the 13 lowest non-honour cards): useful whenever a
/// test wants a hand that is certain to fail every "12+ HCP" / "6-9 HCP" style constraint.
pub fn weak_hand() -> Hand {
    let mut hand = Hand::EMPTY;
    let mut count = 0usize;
    'outer: for suit in Suit::ALL {
        for rank in 0..9u8 {
            // Two..=Ten: no honours (Jack..Ace start at 9).
            if count >= 13 {
                break 'outer;
            }
            let card = Card::from_index(suit.index() * 13 + rank).expect("valid card index");
            hand = hand.with(card);
            count += 1;
        }
    }
    hand
}

/// A uniformly random 13-card hand, via a partial Fisher-Yates shuffle of the deck (no dependency
/// on `bridge_constraint::Sampler`).
pub fn random_hand13(rng: &mut impl Rng) -> Hand {
    let mut deck: Vec<u8> = (0..52).collect();
    for i in 0..13 {
        let j = i + (rng.next_u32() as usize) % (52 - i);
        deck.swap(i, j);
    }
    let mut hand = Hand::EMPTY;
    for &c in &deck[..13] {
        hand = hand.with(Card::from_index(c).expect("index < 52"));
    }
    hand
}

/// A random deal: shuffle the deck and deal it out 13 cards at a time.
pub fn random_deal(rng: &mut impl Rng) -> Deal {
    let mut deck: Vec<u8> = (0..52).collect();
    for i in 0..52 {
        let j = i + (rng.next_u32() as usize) % (52 - i);
        deck.swap(i, j);
    }
    let hands = std::array::from_fn(|seat| {
        let mut hand = Hand::EMPTY;
        for &c in &deck[seat * 13..seat * 13 + 13] {
            hand = hand.with(Card::from_index(c).expect("index < 52"));
        }
        hand
    });
    Deal::new(hands).expect("a full-deck shuffle is always a valid deal")
}

/// `BRIDGE_CORPUS_DIR`, or `<workspace>/corpus/data` (mirrors `bridge-format`'s own test helper of
/// the same name in `tests/common/mod.rs`); `None` when neither exists, so a corpus-dependent
/// `#[ignore]`d test can return early instead of panicking.
pub fn corpus_dir() -> Option<std::path::PathBuf> {
    let dir = match std::env::var_os("BRIDGE_CORPUS_DIR") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/data"),
    };
    if dir.is_dir() { Some(dir) } else { None }
}

/// `BRIDGE_SYSTEMS_DIR`, or `<workspace>/systems` (mirrors `bridge-system`'s own test helper of
/// the same name).
pub fn systems_dir() -> std::path::PathBuf {
    match std::env::var_os("BRIDGE_SYSTEMS_DIR") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../systems"),
    }
}

/// Compiles `systems/sayc/<name>` (checked into the repo, so this panics rather than skips on
/// any I/O error) and wraps it in a [`Table::uniform`](bridge_bidding::Table::uniform) with the
/// real [`bridge_system::NaturalInference::default`] fallback -- the phase 3.10/3.12 harnesses'
/// system under test, as opposed to [`sayc_system`]'s small hand-built stand-in.
pub fn compile_sayc(name: &str) -> bridge_bidding::Table {
    let path = systems_dir().join("sayc").join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let opts = bridge_system::CompileOptions::default();
    let (ir, lints) = bridge_system::compile(
        &path.to_string_lossy(),
        &text,
        &bridge_system::lexer::FsLoader,
        &opts,
    );
    let errors: Vec<_> = lints
        .iter()
        .filter(|l| l.severity == bridge_system::Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "{name}: {} Error-severity lint(s):\n{}",
        errors.len(),
        errors
            .iter()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    bridge_bidding::Table::uniform(
        Arc::new(ir),
        Arc::new(bridge_system::NaturalInference::default()),
    )
}

/// Builds one random position against a compiled SAYC [`Table`](bridge_bidding::Table): a random
/// deal, dealer and vulnerability, then a random-depth prefix (0..12 calls, stopping early if the
/// auction completes) advanced with the same procedure as [`bridge_bidding::replay`]
/// (`11-testing.md` §2 step 1): each seat's call is [`choose_bid`](bridge_bidding::choose_bid)'s
/// choice for its own hand, and a `NoCandidate` becomes `Pass`. Shared by the phase 3.10/3.12
/// harnesses (`tests/consistency.rs`, `tests/policy.rs`), which all need the exact same generator
/// to compare against.
///
/// See [`random_sayc_position_with_gaps`] for the variant that also reports which prefix calls
/// were such forced passes.
pub fn random_sayc_position(
    rng: &mut impl Rng,
    table: &bridge_bidding::Table,
    ctx: &bridge_bidding::BidContext<'_>,
) -> (Deal, Auction) {
    let (deal, auction, _) = random_sayc_position_with_gaps(rng, table, ctx);
    (deal, auction)
}

/// [`random_sayc_position`], also returning the indices (into `auction.calls()`) of every prefix
/// call that was a forced `Pass` substituted for a `NoCandidate` -- the same `gaps` list
/// [`bridge_bidding::replay`] records. A forced `Pass` is not a system-chosen call: `interpret`
/// reads it with whatever the system says a `Pass` shows at that position, which the hand that
/// had no candidate may well not satisfy. A later consistency failure whose root cause is one of
/// these indices is therefore *gap-induced* (a coverage hole in the system, to be closed by SAYC
/// content), and the harness counts it separately from genuine `choose_bid`/`interpret`
/// disagreements rather than excusing it silently.
///
/// Consumes the RNG exactly like [`random_sayc_position`], so both draw the same positions for the
/// same seed.
pub fn random_sayc_position_with_gaps(
    rng: &mut impl Rng,
    table: &bridge_bidding::Table,
    ctx: &bridge_bidding::BidContext<'_>,
) -> (Deal, Auction, Vec<usize>) {
    use bridge_bidding::{BidChoice, choose_bid};

    let deal = random_deal(rng);
    let dealer = Seat::from_index((rng.next_u32() % 4) as u8);
    let vul = Vulnerability::from_index((rng.next_u32() % 4) as u8);
    let mut auction = Auction::new(dealer, vul);
    let mut forced_passes = Vec::new();
    let depth = rng.next_u32() % 12;
    for _ in 0..depth {
        if auction.is_complete() {
            break;
        }
        let seat = auction.next_seat();
        let hand = deal.hand(seat);
        let call = match choose_bid(table, hand, &auction, ctx) {
            BidChoice::Chosen(c) => c.call,
            BidChoice::NoCandidate(_) => {
                forced_passes.push(auction.len());
                Call::Pass
            }
        };
        auction = auction.with(call).expect("choose_bid returns a legal call");
    }
    (deal, auction, forced_passes)
}
