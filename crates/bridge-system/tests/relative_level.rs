//! Relative levels (`cS`, `jY`; `docs/design/06-system.md` §4.6): a call at the cheapest
//! sufficient level, or one level above it, after the path's last bid.

use bridge_core::{Auction, Call, Seat, Vulnerability};
use bridge_system::lexer::MemLoader;
use bridge_system::trie::LookupKey;
use bridge_system::{CompileOptions, LintCode, Severity, SystemIR};

fn compile(source: &str) -> SystemIR {
    let opts = CompileOptions {
        coverage_samples: 0,
        ..CompileOptions::default()
    };
    let (ir, _) = bridge_system::compile("inline.bml", source, &MemLoader::default(), &opts);
    ir
}

fn calls(text: &str) -> Vec<Call> {
    text.split_whitespace()
        .map(|c| c.parse().unwrap())
        .collect()
}

/// The calls our side can make after `auction` (North owns the system), as text.
fn children(ir: &SystemIR, auction: &str, owner: Seat) -> Vec<String> {
    let auction = Auction::from_calls(Seat::North, Vulnerability::None, calls(auction)).unwrap();
    let key = LookupKey::for_auction(&auction, owner).unwrap();
    let lookup = ir.index.resolve(&key);
    assert_eq!(
        lookup.matched_depth,
        key.calls.len(),
        "{auction} is not in the trie"
    );
    let mut out: Vec<String> = ir
        .index
        .children(lookup.end, key.opener_pos, key.vul)
        .into_iter()
        .map(|(c, _)| format!("{c}"))
        .collect();
    out.sort();
    out
}

fn has(ir: &SystemIR, code: LintCode) -> bool {
    ir.lints.iter().any(|l| l.code == code)
}

#[test]
fn cheapest_follows_the_last_bid_of_either_side() {
    let ir = compile(
        "1H = 12+ hcp

1H-
cS = 6+ hcp, 4+!s

1H-(2H)-
cS = 6+ hcp, 5+!s

1H-(2S)-
cS = 10+ hcp, 5+!s
cN = 10+ hcp, stopper
",
    );
    assert!(
        !ir.lints.iter().any(|l| l.severity == Severity::Error),
        "{:?}",
        ir.lints
    );
    // East responds for North's side: dealer North opens 1H, East passes, South is to call.
    assert_eq!(children(&ir, "1H Pass", Seat::North), vec!["1S"]);
    assert_eq!(children(&ir, "1H 2H", Seat::North), vec!["2S"]);
    assert_eq!(children(&ir, "1H 2S", Seat::North), vec!["2NT", "3S"]);
}

#[test]
fn a_bound_variable_takes_the_cheapest_level_for_its_strain() {
    // Their raise at the two level; our overcall in each unbid major / minor at its cheapest
    // level: over (1H)-P-(2H) that is 2S, 3C and 3D; over (1S)-P-(2S) it is 3C, 3D and 3H.
    let ir = compile(
        "(1X)-P-(2X)-
cM = 12+ hcp, 5+M
cm = 12+ hcp, 5+m
jM = 11+ hcp, 6+M
",
    );
    assert!(
        !ir.lints.iter().any(|l| l.severity == Severity::Error),
        "{:?}",
        ir.lints
    );
    // East opens for the opponents: North owns the system and is to call fourth.
    assert_eq!(
        children(&ir, "Pass 1H Pass 2H", Seat::North),
        vec!["2S", "3C", "3D", "3S"]
    );
    assert_eq!(
        children(&ir, "Pass 1S Pass 2S", Seat::North),
        vec!["3C", "3D", "3H", "4H"]
    );
    assert_eq!(
        children(&ir, "Pass 1C Pass 2C", Seat::North),
        vec!["2D", "2H", "2S", "3H", "3S"]
    );
    // The description is substituted per binding: the 2S row wants spades, not hearts.
    let auction =
        Auction::from_calls(Seat::North, Vulnerability::None, calls("Pass 1H Pass 2H")).unwrap();
    let key = LookupKey::for_auction(&auction, Seat::North).unwrap();
    let lookup = ir.index.resolve(&key);
    let spades = ir
        .index
        .children(lookup.end, key.opener_pos, key.vul)
        .into_iter()
        .find(|(c, _)| format!("{c}") == "2S")
        .map(|(_, n)| ir.node(n))
        .unwrap();
    assert_eq!(spades.description, "12+ hcp, 5+!s");
}

#[test]
fn a_jump_past_seven_has_no_candidate() {
    let ir = compile(
        "7H-
jS = 37+ hcp
cS = 20+ hcp
",
    );
    assert!(has(&ir, LintCode::NoSufficientLevel));
    assert_eq!(children(&ir, "7H Pass", Seat::North), vec!["7S"]);
}

#[test]
fn below_a_wildcard_the_level_needs_a_known_last_bid() {
    let ir = compile(
        "1C-(any)-
cS = 6+ hcp

1C-(any)-1H-(P)-
cS = 12+ hcp, 4+!s
",
    );
    // Directly below `(any)` the last bid is unknown: the row is skipped with an error.
    let unknown: Vec<_> = ir
        .lints
        .iter()
        .filter(|l| l.code == LintCode::LevelWithoutAnchor)
        .collect();
    assert_eq!(unknown.len(), 1, "{:?}", ir.lints);
    assert_eq!(unknown[0].severity, Severity::Error);
    assert_eq!(unknown[0].span.as_ref().unwrap().line, 2);
    // After a concrete bid of ours below the wildcard the level is known again.
    assert_eq!(children(&ir, "1C 1D 1H Pass", Seat::North), vec!["1S"]);
}

#[test]
fn a_paragraph_starting_with_a_relative_level_is_not_a_table() {
    let ir = compile("cS is how this file writes the cheapest spade bid.\n\n1C = 12+ hcp\n");
    assert!(!has(&ir, LintCode::UnknownCallToken), "{:?}", ir.lints);
    assert_eq!(ir.rows.len(), 1);
    // In a table they are rows, flagged as non-standard tokens.
    let ir = compile("1C-\ncS = 6+ hcp\n");
    assert!(
        ir.lints
            .iter()
            .any(|l| l.code == LintCode::NonStandardToken && l.message.contains("relative level"))
    );
}
