//! Forward-consistency property (07-bidding.md end of §1, and §8's `forward_consistency` row):
//! `interpret` and `choose_bid` are meant to be inverses of each other, so whatever hand a call
//! was *chosen* for should also be accepted when that same call is later *interpreted*.

mod common;

use bridge_bidding::{
    BidChoice, BidContext, ChoiceSource, Diagnostic, ImplicitPass, InterpretOptions,
    Interpretation, PolicyParams, ResolutionKind, Scoring, Table, choose_bid, interpret,
};
use bridge_core::{Auction, Call, Deal, Hand, Seat, Strain, Suit, Vulnerability};
use bridge_system::ast::{SeatCond, VulCond};
use bridge_system::natural::classify;
use bridge_system::{LookupKey as SysLookupKey, RelVul, SystemIR};
use common::*;
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;

fn table_of(sys: &Sayc) -> Table {
    Table::uniform(
        sys.sys.clone(),
        std::sync::Arc::new(bridge_system::NaturalInference::default()),
    )
}

/// The trie position (`Lookup.end`) `choose_bid` would resolve for `seat` at `auction`
/// -- the same exact-resolve-with-fallback `crate::choose::gather` computes internally (see its
/// doc comment), recomputed here from the public `SystemIR`/`AuctionTrie` API since `gather`
/// itself does not return its `Lookup`. Used to key `CoverageReport`'s gap aggregation by *trie
/// position*, per `11-testing.md` §2 point 4, instead of by the auction's full call-string (the
/// review finding: a raw-string key scatters the same system gap across every distinct auction
/// that reaches it, so a hole hit by one in a thousand deals but from a thousand distinct
/// preceding auctions never accumulates enough count to rank above a coincidental one-off).
fn trie_position(system: &SystemIR, auction: &Auction, seat: Seat) -> u32 {
    let vulnerability = auction.vulnerability();
    let vul = RelVul {
        we: vulnerability.is_vulnerable(seat),
        they: vulnerability.is_vulnerable(seat.next()),
    };
    let key = match SysLookupKey::for_auction(auction, seat) {
        Some(key) => key,
        None => SysLookupKey {
            we_opened: true,
            calls: &[],
            opener_pos: auction.position_of(seat),
            vul,
        },
    };
    system.index.resolve(&key).end.0
}

/// `seat`'s [`bridge_system::natural::Role`] at `auction` (opener/responder/overcaller/advancer/
/// balancer), the `seat_rel` half of the same aggregation key. `classify` needs a call *at*
/// `index`, but its own doc comment guarantees role determination reads only the history strictly
/// before `index`, so a throwaway `Pass` (always legal while the auction is incomplete) stands in
/// for whatever call is actually about to be chosen.
fn seat_role(auction: &Auction, seat: Seat) -> String {
    let probe = auction
        .with(Call::Pass)
        .expect("Pass is always legal while the auction is incomplete");
    let idx = auction.calls().len();
    format!("{:?}", classify(&probe, idx, seat).role).to_lowercase()
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
    let choice = choose_bid(table, hand, auction, ctx);
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

        let choice = choose_bid(&table, hand, &empty, &ctx);
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

/// Regression: `choose_bid`'s implicit-pass synthesis (`choose::gather`) used to treat candidates
/// found through `resolve_lenient` (the opponents' real call substituted by `Pass`, "system on")
/// just like an exact match, building a `Pass` from *those* siblings' complement. But
/// `interpret`'s own Step A only ever takes its implicit-pass branch on an *exact* resolve
/// (07-bidding.md §4.1.5.1) — past an off-system opponents' call, it falls through to
/// `resolve_lenient` and then `Natural`, never re-deriving the same complement. So `choose_bid`
/// could synthesise a `Pass` that `interpret` would never accept as `Exact` for the same position
/// (§5.2 step 3's bidirectional-consistency requirement).
///
/// A full round-trip through `interpret` can't be asserted here: with the resolve no longer
/// exact, South's `Pass` after the off-system overcall falls through Step A to
/// `NaturalInference` (`classify`/`infer`, still `todo!()` on this branch, would panic). The fixed
/// expectation is that `choose_bid` no longer manufactures a mismatched `Pass` in the first
/// place — with no natural fallback wired in, there is genuinely no system-backed candidate for
/// this off-system position, so it must report `NoCandidate` instead.
#[test]
fn choose_bid_does_not_synthesize_pass_from_lenient_siblings() {
    let mut b = SystemBuilder::new();
    b.insert(
        true,
        &[bid(1, Strain::Hearts)],
        bid(1, Strain::Hearts),
        atom_hcp(12, 21),
        SeatCond::Any,
        VulCond::default(),
        "opening",
        0,
    );
    b.insert(
        true,
        &[bid(1, Strain::Hearts), PASS, bid(2, Strain::Hearts)],
        bid(2, Strain::Hearts),
        atom_suit_hcp(Suit::Hearts, 3, 13, 6, 9),
        SeatCond::Any,
        VulCond::default(),
        "raise",
        0,
    );
    let sys = std::sync::Arc::new(b.build());
    let table = Table::uniform(
        sys,
        std::sync::Arc::new(bridge_system::NaturalInference::default()),
    );

    let hand = weak_hand();
    // North opens 1H, East overcalls 1S: off-system for NS (only "1H-Pass-2H" is in the trie), one
    // substitution away from `resolve_lenient` finding the `2H` response node, which `hand` (0
    // HCP) does not satisfy.
    let after_overcall = auction(
        Seat::North,
        Vulnerability::None,
        &[bid(1, Strain::Hearts), bid(1, Strain::Spades)],
    );
    let ctx = BidContext {
        scoring: Scoring::Imp,
        natural: None,
        implicit_pass: ImplicitPass::Complement,
        policy: PolicyParams::default(),
    };
    let choice = choose_bid(&table, hand, &after_overcall, &ctx);
    match choice {
        BidChoice::NoCandidate(_) => {}
        BidChoice::Chosen(c) => panic!(
            "expected NoCandidate (no natural fallback wired in for this off-system position); \
             got {:?} via {:?} instead \u{2014} the pre-fix bug synthesised a Pass from a lenient \
             sibling that `interpret` would never accept as Exact here",
            c.call, c.source
        ),
    }
}

/// Regression (review finding: 24871 `cue`-rooted violations in the 10^6 SAYC run):
/// `choose_bid`'s natural branch used to call `NaturalInference::candidates`, which builds each
/// candidate's `CallContext` with a bare `classify` and so never fills `partner_constraint` /
/// `forcing_situation`, while `interpret`'s natural step (07-bidding.md §4.1 step 6) fills both
/// from the prefix's own interpretation before calling `infer`. `rule_cue`'s `min_hcp` reads
/// `partner_constraint`, so the two computed different constraints for the same cuebid: here the
/// context-free candidate says `2S` shows 10+ hcp, `interpret` says 25+.
///
/// Position: South opens `1H` (the only system row), West overcalls `1S`, North passes, East
/// cuebids `2H`; South is to call, entirely off-system, so both sides go through natural
/// inference.
#[test]
fn choose_bid_natural_branch_matches_interpret_at_a_cuebid() {
    let mut b = SystemBuilder::new();
    b.insert(
        true,
        &[bid(1, Strain::Hearts)],
        bid(1, Strain::Hearts),
        atom_hcp(12, 21),
        SeatCond::Any,
        VulCond::default(),
        "opening",
        0,
    );
    let table = Table::uniform(
        std::sync::Arc::new(b.build()),
        std::sync::Arc::new(bridge_system::NaturalInference::default()),
    );
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
    let a = auction(
        Seat::South,
        Vulnerability::Both,
        &[
            bid(1, Strain::Hearts),
            bid(1, Strain::Spades),
            PASS,
            bid(2, Strain::Hearts),
        ],
    );
    assert_eq!(a.next_seat(), Seat::South);

    // `interpret`'s own reading of `2S` here, the constraint `choose_bid` must agree with.
    let cue = bid(2, Strain::Spades);
    let cue_interp = interpret(&table, &a.with(cue).unwrap(), &opts);
    let cue_alts = &cue_interp.per_call.last().unwrap().alternatives;

    // 15 hcp, three hearts: satisfies the context-free `2S` cue (10+) but not `interpret`'s (25+).
    // Before the fix `choose_bid` picked `2S` for it.
    let medium = hand("J73", "J73", "AK8", "KQJ3");
    assert!(
        !cue_alts.iter().any(|(c, _, _)| c.satisfies(medium)),
        "precondition: interpret's reading of the 2S cuebid rejects the 15-hcp hand"
    );
    let choice = choose_bid(&table, medium, &a, &ctx);
    assert_ne!(
        choice.call(),
        Some(cue),
        "choose_bid chose a cuebid that interpret rejects for the same hand"
    );

    // And across random hands, whatever `choose_bid` chooses here is accepted by `interpret`.
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(0xC0E_B1D);
    let mut chosen = 0;
    for _ in 0..2_000 {
        let h = random_hand13(&mut rng);
        let BidChoice::Chosen(c) = choose_bid(&table, h, &a, &ctx) else {
            continue;
        };
        chosen += 1;
        let interp = interpret(&table, &a.with(c.call).unwrap(), &opts);
        let pc = interp.per_call.last().unwrap();
        assert!(
            pc.alternatives
                .iter()
                .any(|(k, w, ex)| ex.kind != ResolutionKind::Fallback
                    && *w > 0.0
                    && k.satisfies(h)),
            "hand {h:?} was chosen to bid {} at a cuebid position, but interpret's reading of \
             that call does not accept it: {:?}",
            c.call,
            pc.alternatives
        );
    }
    assert!(chosen > 0, "no random hand had a Chosen candidate");
}

// ================================================================================================
// Phase 3.10: strict forward consistency over the *compiled* `systems/sayc/sayc.bml`, with a
// `target/coverage_report.json` gap report (11-testing.md §2, 07-bidding.md §8's
// `forward_consistency` row). Everything above this point checks the property against the small
// hand-built system from `tests/common`; this section is the real harness against the real
// system, which the roadmap's phase 3.10/3.12 tasks and the SAYC-correctness work are measured
// against.
// ================================================================================================

use std::collections::HashMap;

use serde_json::json;

/// One (trie position, `seat_rel`) pair's aggregated gap statistics (`11-testing.md` §2 point 4's
/// `gaps` entries: "`NoCandidate`と`ImplicitPass`は「局面のトライ位置(`Lookup.end`)」ごとに数え"). Keyed by
/// [`GapKey`], not the auction's raw call string: the same trie position is reached by
/// arbitrarily many distinct preceding auctions (different earlier rounds, different opening
/// seat, ...), so a string key scatters one real, frequently-hit hole across hundreds of
/// single-digit-count rows -- exactly the review finding that let deep, rarely-reached paths with
/// `rate == 1.0` fill the top-50 list ahead of shallow, frequent ones.
#[derive(Default)]
struct GapAgg {
    positions: u64,
    no_candidate: u64,
    implicit_pass: u64,
    sample_hand: Option<(Hand, u8)>,
    sample_path: Option<String>,
}

/// Aggregation key: the trie position `choose_bid` resolved (`Lookup.end`, via [`trie_position`])
/// and the acting seat's [`bridge_system::natural::Role`] (via [`seat_role`]), lower-cased. Two
/// positions with the same trie node but different roles (e.g. a balancing-seat node reached with
/// `Role::Balancer` vs. a non-balancing node that happens to share a trie id in a hand-built test
/// system) are kept separate, per `11-testing.md`'s `seat_rel` column.
#[derive(Clone, PartialEq, Eq, Hash)]
struct GapKey {
    trie: u32,
    seat_rel: String,
}

/// What kind of gap a position hit, if any (a position that got a real `Chosen` candidate from
/// the system itself hits neither).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Gap {
    None,
    NoCandidate,
    ImplicitPass,
}

/// Root-cause category of one forward-consistency violation, for `CoverageReport`'s
/// `violations_by_cause` counts.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ViolationCause {
    /// The root-cause call is a `Pass` the prefix generator substituted for a `NoCandidate`
    /// (`common::random_sayc_position_with_gaps`): the system has no call for that hand there, and
    /// `interpret`'s reading of the substituted `Pass` does not cover it. A coverage hole in the
    /// system definition, counted separately and closed by SAYC content, not by the engine.
    GapInduced,
    /// The root-cause call was resolved only by natural inference (`[Natural]`); carries the
    /// `natural.rs` rule name parsed from the explanation text (`"?"` if absent).
    Natural(String),
    /// The root-cause call was resolved by the system (`Exact`/`Partial` alternatives); carries
    /// the lower-cased kinds joined by `+`.
    System(String),
    /// `satisfied_by` failed although every one of the seat's calls has some satisfying
    /// alternative on its own (a Step B combination/truncation effect).
    NoSingleCall,
}

impl ViolationCause {
    fn of(
        root: Option<&(usize, Call, Vec<ResolutionKind>, Option<String>)>,
        forced_passes: &[usize],
    ) -> ViolationCause {
        let Some((index, _, kinds, rule)) = root else {
            return ViolationCause::NoSingleCall;
        };
        if forced_passes.contains(index) {
            return ViolationCause::GapInduced;
        }
        if kinds.as_slice() == [ResolutionKind::Natural] {
            return ViolationCause::Natural(rule.clone().unwrap_or_else(|| "?".to_string()));
        }
        let mut names: Vec<String> = kinds
            .iter()
            .map(|k| match k {
                ResolutionKind::Exact => "exact".to_string(),
                ResolutionKind::Partial { .. } => "partial".to_string(),
                ResolutionKind::Natural => "natural".to_string(),
                ResolutionKind::Fallback => "fallback".to_string(),
            })
            .collect();
        names.sort();
        names.dedup();
        ViolationCause::System(names.join("+"))
    }

    fn label(&self) -> String {
        match self {
            ViolationCause::GapInduced => "gap_induced".to_string(),
            ViolationCause::Natural(rule) => format!("natural:{rule}"),
            ViolationCause::System(kinds) => format!("system:{kinds}"),
            ViolationCause::NoSingleCall => "no_single_call".to_string(),
        }
    }
}

/// Aggregates one `forward_consistency` run: violation records, `chosen`/`no_candidate`/
/// `implicit_pass` totals, per-path gap statistics, and the system-definition diagnostics
/// `choose_bid` surfaced along the way. Mirrors `11-testing.md` §2's `CoverageReport`.
struct CoverageReport {
    seed: u64,
    positions: u64,
    chosen: u64,
    no_candidate: u64,
    implicit_pass: u64,
    violations: Vec<serde_json::Value>,
    /// `violations` counted by root cause (see [`ViolationCause`]), keyed by
    /// [`ViolationCause::label`]. Every violation lands in exactly one bucket.
    violations_by_cause: std::collections::BTreeMap<String, u64>,
    /// Of `violations`, how many are [`ViolationCause::GapInduced`].
    violations_gap_induced: u64,
    /// How many checked positions had at least one forced `Pass` in their prefix.
    forced_pass_prefixes: u64,
    gaps: HashMap<GapKey, GapAgg>,
    illegal_system_call: std::collections::BTreeSet<(u32, String)>,
    unsatisfiable_node: std::collections::BTreeSet<u32>,
}

impl CoverageReport {
    fn new(seed: u64) -> CoverageReport {
        CoverageReport {
            seed,
            positions: 0,
            chosen: 0,
            no_candidate: 0,
            implicit_pass: 0,
            violations: Vec::new(),
            violations_by_cause: std::collections::BTreeMap::new(),
            violations_gap_induced: 0,
            forced_pass_prefixes: 0,
            gaps: HashMap::new(),
            illegal_system_call: std::collections::BTreeSet::new(),
            unsatisfiable_node: std::collections::BTreeSet::new(),
        }
    }

    fn record_diagnostics(&mut self, diagnostics: &[Diagnostic]) {
        for d in diagnostics {
            match *d {
                Diagnostic::IllegalSystemCall { node, call } => {
                    self.illegal_system_call.insert((node.0, format!("{call}")));
                }
                Diagnostic::UnsatisfiableNode { node } => {
                    self.unsatisfiable_node.insert(node.0);
                }
                Diagnostic::DuplicateCandidate { .. } => {}
            }
        }
    }

    /// Records one checked position (whatever its outcome) against its `(trie, seat_rel)` key
    /// (`11-testing.md` §2 point 4), so a gap's rank reflects how often *that system position* is
    /// actually reached, not how often one particular preceding auction happened to recur.
    fn record_position(&mut self, auction: &Auction, hand: Hand, gap: Gap, key: GapKey) {
        self.positions += 1;
        match gap {
            Gap::None => self.chosen += 1,
            Gap::NoCandidate => self.no_candidate += 1,
            Gap::ImplicitPass => self.implicit_pass += 1,
        }
        let entry = self.gaps.entry(key).or_default();
        entry.positions += 1;
        match gap {
            Gap::None => {}
            Gap::NoCandidate => entry.no_candidate += 1,
            Gap::ImplicitPass => entry.implicit_pass += 1,
        }
        if entry.sample_hand.is_none() && !matches!(gap, Gap::None) {
            entry.sample_hand = Some((hand, bridge_eval::hcp(hand)));
            entry.sample_path = Some(format!("{auction}"));
        }
    }

    /// Records one `satisfied_by` failure, tagged with its [`ViolationCause`]: the root cause is
    /// the earliest of `seat`'s calls (which can be earlier than `call` itself, since
    /// `satisfied_by` ANDs over every call the seat has made) that has no satisfying,
    /// non-`Fallback`, positive-weight alternative (see [`root_cause`]).
    ///
    /// Nothing is excused here: every violation is kept in `violations` and counted in exactly
    /// one `violations_by_cause` bucket. The gate ([`Self::violations_not_gap_induced`]) sets
    /// aside only [`ViolationCause::GapInduced`], a separately counted category.
    #[allow(clippy::too_many_arguments)]
    fn record_violation(
        &mut self,
        index: u64,
        auction: &Auction,
        seat: Seat,
        hand: Hand,
        call: Call,
        root: Option<(usize, Call, Vec<ResolutionKind>, Option<String>)>,
        forced_passes: &[usize],
    ) {
        let cause = ViolationCause::of(root.as_ref(), forced_passes);
        if cause == ViolationCause::GapInduced {
            self.violations_gap_induced += 1;
        }
        *self.violations_by_cause.entry(cause.label()).or_default() += 1;
        self.violations.push(json!({
            "index": index,
            "path": format!("{auction}"),
            "seat": format!("{seat:?}"),
            "call": format!("{call}"),
            "hand": format!("{hand:?}"),
            "cause": cause.label(),
            "forced_passes": forced_passes,
            "root_cause_call_index": root.as_ref().map(|(i, ..)| *i),
            "root_cause_call": root.as_ref().map(|(_, c, ..)| format!("{c}")),
            "root_cause_kinds": root.as_ref().map(|(_, _, k, _)| {
                k.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>()
            }),
            "root_cause_natural_rule": root.as_ref().and_then(|(_, _, _, r)| r.clone()),
        }));
    }

    /// Violations that are *not* [`ViolationCause::GapInduced`]: genuine disagreements between
    /// `choose_bid` and `interpret` (a system-definition contradiction or an engine bug). The
    /// strict gate is `== 0` on this count.
    fn violations_not_gap_induced(&self) -> u64 {
        self.violations.len() as u64 - self.violations_gap_induced
    }

    fn summary(&self) -> String {
        let by_cause = self
            .violations_by_cause
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{} violation(s) over {} position(s) (seed {:#x}): chosen={}, no_candidate={}, \
             implicit_pass={}; {} gap-induced (root cause is a forced Pass the prefix generator \
             substituted for a NoCandidate), {} not gap-induced; by cause: [{by_cause}]",
            self.violations.len(),
            self.positions,
            self.seed,
            self.chosen,
            self.no_candidate,
            self.implicit_pass,
            self.violations_gap_induced,
            self.violations_not_gap_induced(),
        )
    }

    /// Writes `<workspace>/target/coverage_report.json` (11-testing.md §2's shape).
    fn write_json(&self, system: &str, meta: &bridge_system::SystemMeta) {
        // Ranked by how often the gap is hit (`no_candidate + implicit_pass`), 11-testing.md §2
        // point 4's "頻度上位 50 件": a rarely reached position with `rate == 1.0` must not push a
        // frequent hole out of the list. Ties are broken by key for a deterministic report.
        let mut gaps: Vec<(&GapKey, &GapAgg)> = self
            .gaps
            .iter()
            .filter(|(_, g)| g.no_candidate + g.implicit_pass > 0)
            .collect();
        gaps.sort_by(|a, b| {
            (b.1.no_candidate + b.1.implicit_pass)
                .cmp(&(a.1.no_candidate + a.1.implicit_pass))
                .then_with(|| a.0.trie.cmp(&b.0.trie))
                .then_with(|| a.0.seat_rel.cmp(&b.0.seat_rel))
        });
        gaps.truncate(50);
        let gaps_json: Vec<serde_json::Value> = gaps
            .into_iter()
            .map(|(key, g)| {
                let rate = (g.no_candidate + g.implicit_pass) as f64 / g.positions as f64;
                json!({
                    "trie": key.trie,
                    "path": g.sample_path,
                    "seat_rel": key.seat_rel,
                    "no_candidate": g.no_candidate,
                    "implicit_pass": g.implicit_pass,
                    "positions": g.positions,
                    "rate": rate,
                    "sample_hand": g.sample_hand.map(|(h, _)| format!("{h:?}")),
                    "sample_hcp": g.sample_hand.map(|(_, hcp)| hcp),
                })
            })
            .collect();
        let system_hash = meta
            .source_hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();

        let report = json!({
            "system": system,
            "system_hash": format!("blake3:{system_hash}"),
            "compiler_version": meta.compiler_version,
            "seed": self.seed,
            "positions": self.positions,
            "random_call_rate": 0.0,
            "violations": self.violations,
            "violations_by_cause": self.violations_by_cause,
            "violations_gap_induced": self.violations_gap_induced,
            "violations_not_gap_induced": self.violations_not_gap_induced(),
            "forced_pass_prefixes": self.forced_pass_prefixes,
            "counts": {
                "chosen": self.chosen,
                "no_candidate": self.no_candidate,
                "implicit_pass": self.implicit_pass,
            },
            "gaps": gaps_json,
            "diagnostics": {
                "illegal_system_call": self.illegal_system_call.iter().map(|(n, c)| json!({"node": n, "call": c})).collect::<Vec<_>>(),
                "unsatisfiable_node": self.unsatisfiable_node.iter().map(|n| json!({"node": n})).collect::<Vec<_>>(),
            },
        });

        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir
            .parent()
            .and_then(std::path::Path::parent)
            .expect("crates/bridge-bidding is two levels under the workspace root");
        let target_dir = workspace_root.join("target");
        std::fs::create_dir_all(&target_dir).expect("create target/ directory");
        std::fs::write(
            target_dir.join("coverage_report.json"),
            serde_json::to_string_pretty(&report).expect("report serializes"),
        )
        .expect("write target/coverage_report.json");
    }
}

/// Finds the root cause of a `satisfied_by(seat, hand) == false` result: one of `seat`'s calls in
/// `interp` that has no satisfying, non-`Fallback`, positive-weight alternative (this can be an
/// *earlier* call than the one just chosen, since `satisfied_by` ANDs over every call the seat has
/// made so far). The earliest failing call that is *not* a forced pass (`forced_passes`) wins, so
/// a failure of a real `choose_bid` choice is never hidden behind an earlier gap; only when every
/// failing call is a forced pass is the earliest of those returned (a gap-induced violation).
/// Returns the call's index, the call itself, the `ResolutionKind`s among its alternatives, and --
/// when every alternative is `Natural` -- the natural-inference rule name, parsed out of
/// `CallExplanation::text`'s trailing `"... (rule)"` (the format `natural_alternative` in
/// `bridge_bidding::interpret` and `choose_bid`'s own explanation-building both use).
fn root_cause(
    interp: &Interpretation,
    seat: Seat,
    hand: Hand,
    forced_passes: &[usize],
) -> Option<(usize, Call, Vec<ResolutionKind>, Option<String>)> {
    let failing: Vec<_> = interp
        .per_call
        .iter()
        .filter(|pc| pc.seat == seat)
        .filter(|pc| {
            !pc.alternatives.iter().any(|(cons, w, ex)| {
                ex.kind != ResolutionKind::Fallback && *w > 0.0 && cons.satisfies(hand)
            })
        })
        .collect();
    failing
        .iter()
        .find(|pc| !forced_passes.contains(&pc.call_index))
        .or_else(|| failing.first())
        .map(|pc| {
            let kinds: Vec<ResolutionKind> =
                pc.alternatives.iter().map(|(_, _, ex)| ex.kind).collect();
            let rule = if kinds == [ResolutionKind::Natural] {
                pc.alternatives.first().and_then(|(_, _, ex)| {
                    let text = ex.text.trim_end();
                    text.rsplit_once('(')
                        .and_then(|(_, tail)| tail.strip_suffix(')'))
                        .map(str::to_string)
                })
            } else {
                None
            };
            (pc.call_index, pc.call, kinds, rule)
        })
}

/// Where [`run_forward_consistency`] writes its report, if anywhere. Only the 10^3 and 10^6 runs
/// write `target/coverage_report.json`; other callers (e.g. the diagnostics check, which runs in
/// parallel with the 10^3 test) must not race on the same file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReportOutput {
    CoverageJson,
    None,
}

/// Runs the strict forward-consistency property over `n` positions on the real, compiled SAYC
/// system, per `11-testing.md` §2 / `07-bidding.md` §8's `forward_consistency` row. Positions come
/// from `common::random_sayc_position_with_gaps` (11-testing.md §2 point 1: the prefix is advanced
/// like `replay`, `NoCandidate` becoming `Pass`, minus the `random_call_rate` off-system
/// substitution scoped to phase 3.11); its forced-pass indices classify gap-induced violations.
fn run_forward_consistency(
    system: &'static str,
    n: u64,
    seed: u64,
    output: ReportOutput,
) -> CoverageReport {
    let table = common::compile_sayc(system);
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
    let mut report = CoverageReport::new(seed);
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);

    for i in 0..n {
        // A prefix that happened to complete the auction has no next call to check; redraw a
        // fresh position instead of silently under-counting.
        let (deal, auction, forced_passes) = std::iter::repeat_with(|| {
            common::random_sayc_position_with_gaps(&mut rng, &table, &ctx)
        })
        .find(|(_, auction, _)| !auction.is_complete())
        .expect("random_sayc_position_with_gaps eventually yields an incomplete auction");
        let seat = auction.next_seat();
        let hand = deal.hand(seat);
        if !forced_passes.is_empty() {
            report.forced_pass_prefixes += 1;
        }
        let key = GapKey {
            trie: trie_position(&table.systems[seat.index() as usize], &auction, seat),
            seat_rel: seat_role(&auction, seat),
        };

        match choose_bid(&table, hand, &auction, &ctx) {
            BidChoice::Chosen(c) => {
                report.record_diagnostics(&c.diagnostics);
                let after = auction
                    .with(c.call)
                    .expect("choose_bid returns a legal call");
                let interp = interpret(&table, &after, &opts);
                let gap = if c.source == ChoiceSource::ImplicitPass {
                    Gap::ImplicitPass
                } else {
                    Gap::None
                };
                if !interp.satisfied_by(seat, hand) {
                    let root = root_cause(&interp, seat, hand, &forced_passes);
                    report.record_violation(i, &auction, seat, hand, c.call, root, &forced_passes);
                }
                report.record_position(&auction, hand, gap, key);
            }
            BidChoice::NoCandidate(nc) => {
                report.record_diagnostics(&nc.diagnostics);
                report.record_position(&auction, hand, Gap::NoCandidate, key);
            }
        }
    }

    if output == ReportOutput::CoverageJson {
        report.write_json(system, &table.systems[0].meta);
    }
    report
}

/// Non-`#[ignore]`d, debug-friendly version: 0 violations other than the separately counted
/// gap-induced ones (see [`ViolationCause::GapInduced`]), whose count is reported in the summary.
#[test]
fn sayc_forward_consistency_1e3() {
    let report =
        run_forward_consistency("sayc.bml", 1_000, 0x5A1C_0001, ReportOutput::CoverageJson);
    eprintln!("sayc_forward_consistency_1e3: {}", report.summary());
    assert_eq!(
        report.violations_not_gap_induced(),
        0,
        "{}",
        report.summary()
    );
}

/// The full 10^6-position release harness (`11-testing.md` §2). Run with
/// `cargo test --release -p bridge-bidding --test consistency -- --ignored
/// sayc_forward_consistency_1e6`.
///
/// `SAYC_CONSISTENCY_N` sets the position count (default 1_000_000) and
/// `SAYC_CONSISTENCY_SEED_OFFSET` is added to the base seed (default 0), so the full run can be
/// split into several shorter chunks -- e.g. four chunks of 250_000 with offsets 0/1/2/3 -- each
/// drawing an independent random stream, with their counts summed for the reported total.
#[test]
#[ignore = "10^6 positions; run with `cargo test --release -- --ignored`"]
fn sayc_forward_consistency_1e6() {
    let started = std::time::Instant::now();
    let n: u64 = match std::env::var("SAYC_CONSISTENCY_N") {
        Ok(v) => v.parse().expect("SAYC_CONSISTENCY_N is a valid u64"),
        Err(_) => 1_000_000,
    };
    let seed_offset: u64 = match std::env::var("SAYC_CONSISTENCY_SEED_OFFSET") {
        Ok(v) => v
            .parse()
            .expect("SAYC_CONSISTENCY_SEED_OFFSET is a valid u64"),
        Err(_) => 0,
    };
    let report = run_forward_consistency(
        "sayc.bml",
        n,
        0x5A1C_0002u64.wrapping_add(seed_offset),
        ReportOutput::CoverageJson,
    );
    eprintln!(
        "sayc_forward_consistency_1e6: {} in {:?}",
        report.summary(),
        started.elapsed()
    );
    assert_eq!(
        report.violations_not_gap_induced(),
        0,
        "{}",
        report.summary()
    );
}

/// A cheap, non-`#[ignore]`d check that `choose_bid` never raises `IllegalSystemCall` or
/// `UnsatisfiableNode` against the real, compiled `sayc.bml` (these are counted, not asserted,
/// inside `run_forward_consistency` itself, since a hand-built test system might legitimately
/// exercise them; the real system should not).
#[test]
fn sayc_forward_consistency_diagnostics_are_empty_on_current_sayc() {
    let report = run_forward_consistency("sayc.bml", 200, 0x5A1C_0003, ReportOutput::None);
    assert!(
        report.illegal_system_call.is_empty(),
        "sayc.bml: unexpected IllegalSystemCall diagnostics: {:?}",
        report.illegal_system_call
    );
    assert!(
        report.unsatisfiable_node.is_empty(),
        "sayc.bml: unexpected UnsatisfiableNode diagnostics: {:?}",
        report.unsatisfiable_node
    );
}
