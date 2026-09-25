//! SAYC (`systems/sayc/*.bml`, roadmap 3.5/4.2-4.4, `docs/design/12-roadmap.md`) compiles
//! cleanly against the description-compiler vocabulary: zero `Error`-severity lints, no
//! `HandConstraint::Custom` anywhere, no `Warning`-severity `SiblingSubset` lint on any Us-side
//! node with a non-empty description (a bid that "can never be reached" because an earlier,
//! equal-priority sibling's constraint already covers it -- the exact shape of the blocker this
//! lane's review found: an unconstrained unusual-2NT row swallowing every 1-level overcall,
//! `systems/sayc/NOTES.md`), a per-file micro-averaged description-recognition ratio
//! (`Σcovered / Σtotal`, grouped by `Row.span.file`, per §7.7) of at least 0.9 for every included
//! file, and every compiled node's constraint is satisfiable
//! (`bridge_constraint::HandConstraint::is_satisfiable`, backed by the sampler, not skipped).
//!
//! A separate sanity check (not the phase-3.10 `forward_consistency`/`coverage_report.json`
//! harness, which lives in `bridge-bidding` and is out of this lane's scope) samples 10^5 random
//! hands and asks two questions of `openings-only.bml`'s opening-bid table alone: how often does
//! a hand with opening values (12+ HCP; `NOTES.md` #11 -- no rule of 20 in this file) fail to
//! satisfy any opening bid, and how often do two opening candidates tie on `{prio:N}` with no
//! row-order left to resolve them. The first is asserted near zero; the second is reported only.

mod common;

use std::collections::BTreeMap;
use std::time::Instant;

use bridge_constraint::HandConstraint;
use bridge_core::{Auction, Call, Hand, Seat, Vulnerability};
use bridge_system::{
    CompileOptions, LintCode, LookupKey, NodeId, RelVul, Severity, Side, SystemIR,
};

/// Compiles one `systems/sayc/<name>` file with `FsLoader`, panicking (not skipping) on any
/// I/O error: unlike the vendored-corpus tests, this file is checked into the repo and must
/// always be present.
fn compile_sayc(name: &str) -> (SystemIR, std::time::Duration) {
    let path = common::systems_dir().join("sayc").join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let opts = CompileOptions::default();
    let started = Instant::now();
    let (ir, _lints) = bridge_system::compile(
        &path.to_string_lossy(),
        &text,
        &bridge_system::lexer::FsLoader,
        &opts,
    );
    (ir, started.elapsed())
}

/// `true` if `c` contains a `HandConstraint::Custom` anywhere (R10: the description compiler
/// never leaves one in the compiled IR).
fn contains_custom(c: &HandConstraint) -> bool {
    match c {
        HandConstraint::Custom(_) => true,
        HandConstraint::And(terms) | HandConstraint::Or(terms) => terms.iter().any(contains_custom),
        HandConstraint::Not(inner) => contains_custom(inner),
        HandConstraint::Atom(_) => false,
    }
}

#[test]
fn sayc_compiles_with_zero_errors_and_no_custom() {
    for name in ["sayc.bml", "openings-only.bml"] {
        let (ir, elapsed) = compile_sayc(name);
        eprintln!(
            "sayc: {name} compiled in {elapsed:?} ({} rows, {} nodes, {} lints)",
            ir.rows.len(),
            ir.nodes.len(),
            ir.lints.len()
        );
        // `docs/design/11-testing.md` §1, §9's < 1s budget is a release-profile number (see
        // `tests/compile_time.rs`, which measures it under `--release` specifically); an
        // unoptimized debug build of the same compile can be an order of magnitude slower, so
        // this is only a "didn't regress into a hang or blow-up" sanity net, generous enough to
        // hold in both profiles, not a re-assertion of the release budget.
        assert!(
            elapsed.as_secs_f64() < 10.0,
            "{name}: compiling took {elapsed:?}, expected well under 10s in any profile"
        );

        let errors: Vec<_> = ir
            .lints
            .iter()
            .filter(|l| l.severity == Severity::Error)
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

        for node in &ir.nodes {
            assert!(
                !contains_custom(&node.constraint),
                "{name}: node {:?} ({}) compiled to a HandConstraint::Custom",
                node.id,
                node.description
            );
        }
    }
}

/// R9 (review, blocker + major findings): a `Warning`-severity `SiblingSubset` lint on a Us-side
/// node whose description is non-empty means that node's bid can never be chosen by `choose_bid`
/// -- an earlier, equal-priority sibling's constraint already covers every hand that would
/// satisfy it (`docs/design/06-system.md` §9.3 point 6). This is exactly the shape of the
/// blocker this lane's review found (`(1X)- 2N = !UNT` with no shape, ranked ahead of every
/// 1-level overcall) and of several of its major findings, so it is asserted here directly rather
/// than left to a manual read of the lint list. A node with an *empty* description (a bare
/// history retrace, or a row whose own description compiled to nothing) is excluded: it carries
/// no authored intent to protect, unlike a real bid.
#[test]
fn sayc_has_no_reachability_hiding_sibling_subset_warnings() {
    for name in ["sayc.bml", "openings-only.bml"] {
        let (ir, _) = compile_sayc(name);
        let hidden: Vec<String> = ir
            .lints
            .iter()
            .filter(|l| l.code == LintCode::SiblingSubset && l.severity == Severity::Warning)
            .filter_map(|l| {
                let node_id = l.node?;
                let node = ir.node(node_id);
                (node.side == Side::Us && !node.description.is_empty()).then(|| {
                    format!(
                        "  {:?} ({:?}) at {:?}: {}",
                        node.id, node.description, l.span, l.message
                    )
                })
            })
            .collect();
        assert!(
            hidden.is_empty(),
            "{name}: {} Us-side node(s) with a non-empty description are unreachable behind an \
             earlier, equal-priority sibling (Warning-severity SiblingSubset):\n{}",
            hidden.len(),
            hidden.join("\n")
        );
    }
}

/// The `#INCLUDE` order `sayc.bml` and `openings-only.bml` both use (`systems/sayc/NOTES.md`
/// #17), so a compiled `Span.file` (a bare `FileId`) can be reported back as a filename in test
/// output. This is a test-only convenience, not a compiler API: it re-reads the root file's own
/// `#INCLUDE` lines textually, the same order `bridge_system::lexer` resolves them in, and does
/// not follow transitive includes (none of these files include a file that itself includes
/// another).
fn included_file_names(root_name: &str) -> Vec<String> {
    let root_path = common::systems_dir().join("sayc").join(root_name);
    let text = std::fs::read_to_string(&root_path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", root_path.display()));
    let mut names = vec![root_name.to_string()];
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("#INCLUDE ") {
            names.push(rest.trim().to_string());
        }
    }
    names
}

/// R9 (review, minor finding): §7.7's recognition ratio is a *micro*-average
/// (`Σcovered / Σtotal`, not a mean of per-row ratios, which over- or under-weights short and
/// long descriptions differently) and is reported *per included file* (`Row.span.file`), not
/// pooled across the whole compiled system the way a single root-level mean would: `sayc.bml`
/// pulls in nine files of very different sizes, and a mean over all of their rows together could
/// hide one weak file behind several strong ones. Rows with an empty description
/// (`recognition.total == 0`: a bare history retrace, not an authored bid) are excluded from both
/// sums, matching `check_recognition`'s own skip (`crates/bridge-system/src/lint.rs`).
#[test]
fn sayc_recognition_ratio_is_at_least_0_9_per_file() {
    for name in ["sayc.bml", "openings-only.bml"] {
        let (ir, _) = compile_sayc(name);
        let names = included_file_names(name);

        let mut by_file: BTreeMap<u16, (u64, u64)> = BTreeMap::new();
        for row in &ir.rows {
            if row.recognition.total == 0 {
                continue;
            }
            let entry = by_file.entry(row.span.file.0).or_insert((0, 0));
            entry.0 += u64::from(row.recognition.covered);
            entry.1 += u64::from(row.recognition.total);
        }

        assert!(
            !by_file.is_empty(),
            "{name}: no row had a non-empty description to measure recognition over"
        );

        for (file, (covered, total)) in &by_file {
            let ratio = *covered as f64 / *total as f64;
            let label = names
                .get(*file as usize)
                .map(String::as_str)
                .unwrap_or("<unknown file>");
            eprintln!(
                "sayc: {name}: {label} (file {file}) recognition ratio {ratio:.4} \
                 ({covered}/{total} words)"
            );
            assert!(
                ratio >= 0.9,
                "{name}: {label} (file {file}) recognition ratio {ratio:.4} \
                 ({covered}/{total} words) is below the 0.9 target"
            );
        }
    }
}

#[test]
fn sayc_every_node_is_satisfiable() {
    for name in ["sayc.bml", "openings-only.bml"] {
        let (ir, _) = compile_sayc(name);
        let unsatisfiable: Vec<(NodeId, String)> = ir
            .nodes
            .iter()
            .filter(|n| !n.constraint.is_satisfiable())
            .map(|n| (n.id, n.description.clone()))
            .collect();
        eprintln!(
            "sayc: {name}: {}/{} node(s) unsatisfiable",
            unsatisfiable.len(),
            ir.nodes.len()
        );
        assert!(
            unsatisfiable.is_empty(),
            "{name}: {} node(s) with an unsatisfiable constraint:\n{}",
            unsatisfiable.len(),
            unsatisfiable
                .iter()
                .map(|(id, desc)| format!("  {id:?}: {desc:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

/// R9 (review, minor finding): `competition.bml`'s balancing table is keyed on the literal
/// history `(1X)-P-(P)-`, which (per `docs/design/06-system.md` §6, matching upstream `bss.py`
/// convention: "describe every call including passes") spells out *our own* first pass as an
/// explicit call in the row's path. That makes it a genuine, row-defined `Side::Us` node with no
/// description of its own to compile -- unlike the *implicit* pass complement used everywhere
/// else a hand simply doesn't fit any of its siblings (§4.1 step 5.1 in `07-bidding.md`) -- so it
/// compiles to an unconstrained `Atom::ANY`. That is the only sound reading available (the BML
/// vocabulary has no way to write "the complement of the direct-seat actions" by hand, and this
/// row has no direct-seat siblings of its own to be a complement of), but it was previously
/// unasserted, so a future change narrowing it (or a duplicate direct-seat/balancing node mixup)
/// would have compiled silently. This pins the current, deliberate behavior down: the row exists
/// and is satisfied by every hand, weak ones included.
#[test]
fn sayc_balancing_pass_history_token_is_unconstrained() {
    let (ir, _) = compile_sayc("sayc.bml");
    let balancing_pass_nodes: Vec<_> = ir
        .nodes
        .iter()
        .filter(|n| n.side == bridge_system::Side::Us && n.call == Call::Pass && n.calls.len() == 2)
        .collect();
    assert!(
        !balancing_pass_nodes.is_empty(),
        "sayc.bml: expected at least one Us-side Pass node one call after a 1-level opening \
         (the `(1X)-P-(P)-` balancing table's own history token)"
    );
    // A hand with nothing at all still has to be able to "make" this pass: it must not have
    // picked up some accidental hcp/shape constraint from a neighboring row.
    let hopeless = common::hand("7432", "432", "432", "432");
    for node in &balancing_pass_nodes {
        assert!(
            node.constraint.satisfies(hopeless),
            "sayc.bml: node {:?} (the balancing table's own history pass) rejected a 0-hcp hand; \
             it should be unconstrained",
            node.id
        );
    }
}

/// A tiny, dependency-free, deterministic PRNG (SplitMix64) so this test needs no `rand` crate
/// and no seed ever changes between runs -- test-only, unrelated to `bridge_constraint`'s own
/// (private) sampler RNG.
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> SplitMix64 {
        SplitMix64(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform value in `0..bound` (small bias from the modulo is irrelevant for a shuffle
    /// over at most 52 elements in a test).
    fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }
}

/// A uniformly random 13-card hand, drawn by a Fisher-Yates shuffle of the 52-card deck (any
/// bijection between `Hand`'s bit positions and physical cards gives a uniform 13-subset this
/// way, so which bit is which suit/rank does not matter here).
fn random_hand(rng: &mut SplitMix64) -> Hand {
    let mut deck: [u8; 52] = core::array::from_fn(|i| i as u8);
    for i in (1..52).rev() {
        let j = rng.below((i + 1) as u64) as usize;
        deck.swap(i, j);
    }
    let mut bits: u64 = 0;
    for &card in &deck[..13] {
        bits |= 1u64 << card;
    }
    Hand::from_bits(bits).expect("13 distinct bits out of 52 is always a valid Hand")
}

/// The opening-bid candidates at the very start of the auction (dealer to call, no history): the
/// same key `bridge-bidding::choose::gather` builds for an empty auction (`LookupKey::for_auction`
/// itself returns `None` there, since it has no opener yet to key off).
fn opening_candidates(
    ir: &SystemIR,
    seat: Seat,
    vulnerability: Vulnerability,
) -> Vec<(Call, NodeId)> {
    let auction = Auction::new(seat, vulnerability);
    let key = LookupKey {
        we_opened: true,
        calls: &[],
        opener_pos: auction.position_of(seat),
        vul: RelVul {
            we: vulnerability.is_vulnerable(seat),
            they: vulnerability.is_vulnerable(seat.next()),
        },
    };
    let lookup = ir.index.resolve(&key);
    ir.index.children(lookup.end, key.opener_pos, key.vul)
}

/// Opening coverage sanity (task brief, not `docs/design/12-roadmap.md`'s own 3.10 harness): over
/// 10^5 fixed-seed random hands, how many hands SAYC opens (12+ HCP, `NOTES.md` #11: no rule of
/// 20) find no candidate opening bid, and how many hands have two candidates tied on `{prio:N}`
/// with no row-order left to resolve them (`#+TIEBREAK: row-order` only orders *rows*, not two
/// nodes of genuinely equal priority and disjoint conditions -- this is a design smell, not
/// necessarily a bug, so it is reported, not asserted).
#[test]
fn sayc_opening_coverage_sanity() {
    let (ir, _) = compile_sayc("openings-only.bml");

    let seat = Seat::North;
    let vulnerability = Vulnerability::None;
    let candidates = opening_candidates(&ir, seat, vulnerability);
    assert!(
        !candidates.is_empty(),
        "openings-only.bml: no opening candidates at all at the start of the auction"
    );
    assert!(
        candidates.iter().all(|(call, _)| *call != Call::Pass),
        "openings-only.bml now defines an explicit opening Pass; this test's \"other than Pass\" \
         filter needs updating to match"
    );

    const HANDS: u64 = 100_000;
    const SEED: u64 = 0xC0FF_EE15_5AC5_0BE1;
    let mut rng = SplitMix64::new(SEED);

    let mut no_opening_12plus = 0u64;
    let mut tied_priority = 0u64;

    for _ in 0..HANDS {
        let hand = random_hand(&mut rng);
        let hcp = bridge_eval::hcp(hand);

        let mut best_priority = i16::MIN;
        let mut best_count = 0u32;
        for &(_, node_id) in &candidates {
            let node = ir.node(node_id);
            if node.constraint.satisfies(hand) {
                match node.priority.cmp(&best_priority) {
                    std::cmp::Ordering::Greater => {
                        best_priority = node.priority;
                        best_count = 1;
                    }
                    std::cmp::Ordering::Equal => best_count += 1,
                    std::cmp::Ordering::Less => {}
                }
            }
        }

        let opened = best_count > 0;
        if !opened && hcp >= 12 {
            no_opening_12plus += 1;
        }
        if opened && best_count >= 2 {
            tied_priority += 1;
        }
    }

    eprintln!(
        "sayc opening coverage over {HANDS} random hands (seed {SEED:#x}): \
         {no_opening_12plus} hand(s) with 12+ HCP opened nothing, \
         {tied_priority} hand(s) had >=2 opening candidates tied on priority"
    );

    let gap_rate = no_opening_12plus as f64 / HANDS as f64;
    assert!(
        gap_rate < 0.0005,
        "{no_opening_12plus}/{HANDS} ({gap_rate:.6}) hands with 12+ HCP found no opening bid; \
         SAYC should open essentially every 12+ HCP hand"
    );
}
