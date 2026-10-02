//! `#EXACTPASS` and `#EXACTPASS FILE` (`docs/design/06-system.md` §4.8): the opponents' pass
//! right before a table's rows of ours is exact, so every other call of theirs at that position
//! that no table (and no stop) gives an edge reaches an empty `(any)` sibling and leaves the
//! system, instead of `resolve_lenient` reading it as a pass.

use bridge_core::{Auction, Call, Seat, Vulnerability};
use bridge_system::ast::SeatCond;
use bridge_system::lexer::MemLoader;
use bridge_system::trie::LookupKey;
use bridge_system::{CompileOptions, LintCode, Severity, Side, SystemIR};

fn compile_files(files: &[(&str, &str)]) -> SystemIR {
    let opts = CompileOptions {
        coverage_samples: 0,
        ..CompileOptions::default()
    };
    let loader = MemLoader {
        files: files
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect(),
    };
    let (ir, _) = bridge_system::compile(files[0].0, files[0].1, &loader, &opts);
    ir
}

fn compile(source: &str) -> SystemIR {
    compile_files(&[("root.bml", source)])
}

/// How North's system (North opens or passes first) reads `calls`, dealt by North: the number
/// of the opponents' calls `resolve_lenient` had to read as passes, whether the reading then
/// matched every call, and the calls of ours it offers there.
fn read_seat(ir: &SystemIR, calls: &str, dealer: Seat) -> (u8, bool, Vec<String>) {
    let calls: Vec<Call> = calls
        .split_whitespace()
        .map(|c| c.parse().unwrap())
        .collect();
    let auction = Auction::from_calls(dealer, Vulnerability::None, calls).unwrap();
    let key = LookupKey::for_auction(&auction, Seat::North).unwrap();
    let attempts = ir.index.resolve_lenient(&key, 2);
    let (lookup, subst) = attempts.last().unwrap();
    let children = ir
        .index
        .children(lookup.end, key.opener_pos, key.vul)
        .into_iter()
        .map(|(c, _)| c.to_string())
        .collect();
    (*subst, lookup.matched_depth == key.calls.len(), children)
}

fn read(ir: &SystemIR, calls: &str) -> (u8, bool, Vec<String>) {
    read_seat(ir, calls, Seat::North)
}

/// An off-system position: every call matched as written (none read as a pass) and the system
/// has no call of ours there, so natural inference answers.
fn off_system() -> (u8, bool, Vec<String>) {
    (0, true, Vec::new())
}

fn calls_of(list: &str) -> Vec<String> {
    list.split_whitespace().map(str::to_string).collect()
}

/// The guard nodes of the `#EXACTPASS` directive on `line` (its line is their row).
fn guards(ir: &SystemIR, line: u32) -> usize {
    ir.nodes
        .iter()
        .filter(|n| !n.is_synthesised() && ir.rows[n.row.0 as usize].span.line == line)
        .count()
}

fn count(ir: &SystemIR, code: LintCode) -> usize {
    ir.lints.iter().filter(|l| l.code == code).count()
}

/// No Error and no Warning, except `SiblingSubset` among the opponents' calls: an opponents'
/// call written without a description (any hand) contains every later sibling, the guard's
/// `(any)` included, exactly as a written `(any)` row would be (see
/// `the_guard_is_linted_like_a_written_any`).
fn no_warnings(ir: &SystemIR) {
    let bad: Vec<String> = ir
        .lints
        .iter()
        .filter(|l| l.severity != Severity::Info)
        .filter(|l| {
            l.code != LintCode::SiblingSubset
                || l.node.is_none_or(|n| ir.node(n).side != Side::Them)
        })
        .map(|l| l.to_string())
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
}

const ONE_CLUB: &str = "1C = 12--21 hcp, 3+!c

1C-
1H = 6+ hcp, 4+!h
1N = 6--10 hcp
";

#[test]
fn without_the_directive_an_unwritten_overcall_is_read_as_a_pass() {
    let ir = compile(ONE_CLUB);
    assert_eq!(read(&ir, "1C 1S"), (1, true, calls_of("1H 1NT")));
    assert_eq!(read(&ir, "1C X"), (1, true, calls_of("1H 1NT")));
}

#[test]
fn the_implicit_pass_before_our_rows_is_exact() {
    let ir = compile(&ONE_CLUB.replace("1C-\n", "#EXACTPASS\n1C-\n"));
    no_warnings(&ir);
    assert_eq!(count(&ir, LintCode::ExactPassWithoutPass), 0);
    assert_eq!(read(&ir, "1C P"), (0, true, calls_of("1H 1NT")));
    for overcall in ["1C 1S", "1C X", "1C 2D", "1C 7NT"] {
        assert_eq!(read(&ir, overcall), off_system(), "{overcall}");
    }
    // The guard is one opponents' node with an empty description (any hand) under 1C, its
    // row the directive's line.
    let auction = Auction::from_calls(
        Seat::North,
        Vulnerability::None,
        vec!["1C".parse().unwrap()],
    )
    .unwrap()
    .with("1S".parse().unwrap())
    .unwrap();
    let key = LookupKey::for_auction(&auction, Seat::North).unwrap();
    let lookup = ir.index.resolve(&key);
    assert_eq!(lookup.via_class, 1);
    let guard = ir.node(lookup.by_depth[1].expect("the guard has a node"));
    assert_eq!(guard.side, Side::Them);
    assert_eq!(guard.description, "");
    assert!(guard.children.is_empty());
    assert!(!guard.is_synthesised());
    let opening = ir.node(lookup.by_depth[0].unwrap());
    assert!(opening.children.contains(&lookup.by_depth[1].unwrap()));
    assert_eq!(ir.rows[guard.row.0 as usize].span.line, 3);
}

#[test]
fn a_written_pass_is_exact_too() {
    let ir = compile(
        "1N = 15--17 hcp, bal

#EXACTPASS
1N-(P)-2C-(P)-
2D = no 4+ major
2H = 4+!h
2S = 4+!s
",
    );
    no_warnings(&ir);
    assert_eq!(read(&ir, "1NT P 2C P"), (0, true, calls_of("2D 2H 2S")));
    assert_eq!(read(&ir, "1NT P 2C X"), off_system());
    assert_eq!(read(&ir, "1NT P 2C 2D"), off_system());
    // Only the pass right before the table's rows of ours: the history's own `(P)` after 1N
    // is the business of the table that writes our rows there.
    assert_eq!(read(&ir, "1NT 2D"), (1, true, calls_of("2C")));
    assert_eq!(guards(&ir, 3), 1);
}

#[test]
fn the_guard_is_linted_like_a_written_any() {
    // The undescribed `(P)` (any hand) contains the guard's `(any)` (any hand), as it would
    // contain a written `1N-(P)-2C-(any)-`: `SiblingSubset` on the directive's line. A
    // description of their pass that is not "any hand" leaves an overlap (Info) instead.
    let source = "1N = 15--17 hcp, bal

#EXACTPASS
1N-(P)-2C-(P)-{DESC}
2D = no 4+ major
";
    let ir = compile(&source.replace("{DESC}", ""));
    let subsets: Vec<(Severity, u32)> = ir
        .lints
        .iter()
        .filter(|l| l.code == LintCode::SiblingSubset)
        .map(|l| (l.severity, ir.rows[l.row.unwrap().0 as usize].span.line))
        .collect();
    assert_eq!(subsets, [(Severity::Warning, 3)]);
    let written = source.replace("#EXACTPASS\n", "").replace("{DESC}", "");
    let written = compile(&format!("{written}\n1N-(P)-2C-(any)-\n"));
    assert_eq!(count(&written, LintCode::SiblingSubset), 1);
    let ir = compile(&source.replace("{DESC}", " = 0--11 hcp"));
    assert_eq!(count(&ir, LintCode::SiblingSubset), 0);
    assert_eq!(count(&ir, LintCode::SiblingOverlap), 1);
}

#[test]
fn every_pass_before_a_row_of_ours_is_guarded_and_no_other() {
    let ir = compile(
        "1C = 12--21 hcp, 3+!c

#EXACTPASS
1C-1H-
(P)
  1S = 4+!s
  1N = 12--14 hcp, bal
(1S)
  X = 3!s
  2H = 4+!h
",
    );
    no_warnings(&ir);
    // The written `(P)` row: the position after 1H. (1S) has its own edge.
    assert_eq!(read(&ir, "1C P 1H 1S"), (0, true, calls_of("X 2H")));
    assert_eq!(read(&ir, "1C P 1H X"), off_system());
    assert_eq!(read(&ir, "1C P 1H 2C"), off_system());
    // After (1S) our rows follow their bid, and no row of ours follows 1S or 1N: the pass
    // before 1S/1N is the only one guarded.
    assert_eq!(guards(&ir, 3), 1);
}

#[test]
fn a_stop_covering_the_position_takes_precedence() {
    let source = "1C = 12--21 hcp, 3+!c

1C-
1H = 6+ hcp, 4+!h
  #STOP

{DIRECTIVE}
1C-1H-
1S = 4+!s
";
    let plain = compile(&source.replace("{DIRECTIVE}\n", ""));
    let guarded = compile(&source.replace("{DIRECTIVE}", "#EXACTPASS"));
    no_warnings(&guarded);
    // The stop's `(any)` already takes every call of theirs after 1H: no guard is added, and
    // the stop pass is still offered over their interference.
    assert_eq!(plain.nodes.len(), guarded.nodes.len());
    assert_eq!(plain.index.len(), guarded.index.len());
    assert_eq!(read(&guarded, "1C P 1H 2C"), (0, true, calls_of("Pass")));
    for auction in ["1C P 1H P", "1C P 1H 2C", "1C P 1H X P 1S", "1C 1S"] {
        assert_eq!(read(&plain, auction), read(&guarded, auction), "{auction}");
    }
}

#[test]
fn a_written_any_takes_precedence() {
    let source = "1C = 12--21 hcp, 3+!c

{DIRECTIVE}
1C-
1H = 6+ hcp, 4+!h

1C-(any)-
X = 10+ hcp
";
    let plain = compile(&source.replace("{DIRECTIVE}\n", ""));
    let guarded = compile(&source.replace("{DIRECTIVE}", "#EXACTPASS"));
    assert_eq!(plain.nodes.len(), guarded.nodes.len());
    assert_eq!(read(&guarded, "1C 1S"), (0, true, calls_of("X")));
    assert_eq!(read(&guarded, "1C P"), (0, true, calls_of("1H")));
}

#[test]
fn the_guard_takes_only_the_calls_no_class_edge_takes() {
    // The `(bid)` table comes later in the file; the guard is still tried after it.
    let ir = compile(
        "1C = 12--21 hcp, 3+!c

#EXACTPASS
1C-
1H = 6+ hcp, 4+!h

1C-(bid)-
X = 10+ hcp

1C-(2D)-
2H = 5+!h
",
    );
    no_warnings(&ir);
    assert_eq!(read(&ir, "1C 2D"), (0, true, calls_of("2H")));
    assert_eq!(read(&ir, "1C 1S"), (0, true, calls_of("X")));
    assert_eq!(read(&ir, "1C 3NT"), (0, true, calls_of("X")));
    assert_eq!(read(&ir, "1C X"), off_system());
}

#[test]
fn every_level_wildcard_leaves_the_rest_to_the_guard() {
    // `(nX)` writes concrete edges: the bids in the strains X can take (not clubs, which 1C
    // used, and not notrump). The guard takes the double and the other bids.
    let ir = compile(
        "1C = 12--21 hcp, 3+!c

#EXACTPASS
1C-
1H = 6+ hcp, 4+!h

1C-(nX)-
D = 10+ hcp
",
    );
    assert_eq!(read(&ir, "1C 2S"), (0, true, calls_of("X")));
    assert_eq!(read(&ir, "1C 1NT"), off_system());
    assert_eq!(read(&ir, "1C 2C"), off_system());
    assert_eq!(read(&ir, "1C X"), off_system());
}

#[test]
fn variables_and_relative_levels_guard_each_expansion() {
    let ir = compile(
        "1C = 12--21 hcp, 3+!c
1D = 12--21 hcp, 4+!d
1H = 12--21 hcp, 5+!h
1S = 12--21 hcp, 5+!s

#EXACTPASS
1X-(1Y)-P-(P)-
cX = 6+X
",
    );
    no_warnings(&ir);
    for (passed, raised) in [
        ("1C 1D P P", "1C 1D P 2D"),
        ("1D 1H P P", "1D 1H P 2H"),
        ("1H 1S P P", "1H 1S P 2S"),
    ] {
        assert!(read(&ir, passed).1, "{passed}");
        assert_eq!(read(&ir, raised), off_system(), "{raised}");
    }
    // One guard per expansion with a row: C-D, C-H, C-S, D-H, D-S and H-S (1S has no 1Y
    // above it).
    assert_eq!(guards(&ir, 6), 6);
}

#[test]
fn any_order_expansions_are_guarded_too() {
    let source = "1C = 12--21 hcp, 3+!c
1D = 12--21 hcp, 4+!d
1H = 12--21 hcp, 5+!h
1S = 12--21 hcp, 5+!s

#EXACTPASS
{ANYORDER}
1X-(2Y)-P-(P)-
D = 15+ hcp, 0--2Y
";
    let ordered = compile(&source.replace("{ANYORDER}\n", ""));
    let any = compile(&source.replace("{ANYORDER}", "#ANYORDER"));
    no_warnings(&any);
    assert_eq!(read(&ordered, "1H 2S P 3S"), off_system());
    assert!(!read(&ordered, "1H 2C P").1);
    assert_eq!(read(&any, "1H 2C P 3C"), off_system());
    assert_eq!(read(&any, "1H 2C P P"), (0, true, calls_of("X")));
}

#[test]
fn a_row_that_expands_to_nothing_guards_nothing() {
    // Under `(any)` the relative level has no anchor: the row is dropped, so is the guard.
    let ir = compile(
        "1C = 12--21 hcp, 3+!c

#EXACTPASS
1C-(any)-P-(P)-
cH = 4+!h
",
    );
    assert_eq!(count(&ir, LintCode::LevelWithoutAnchor), 1);
    assert_eq!(read(&ir, "1C 1S P 2D").0, 1);
}

#[test]
fn the_guard_carries_the_table_seat_and_vul_but_its_edge_does_not() {
    let ir = compile(
        "1C = 12--21 hcp, 3+!c

#SEAT 34

#EXACTPASS
1C-
1H = 6+ hcp, 4+!h
",
    );
    let auction = |dealer: Seat| {
        let calls: Vec<Call> = match dealer {
            Seat::North => vec!["1C".parse().unwrap(), "1S".parse().unwrap()],
            _ => vec![
                Call::Pass,
                Call::Pass,
                "1C".parse().unwrap(),
                "1S".parse().unwrap(),
            ],
        };
        Auction::from_calls(dealer, Vulnerability::None, calls).unwrap()
    };
    // Third seat: North opens after two passes (dealer South).
    let third = auction(Seat::South);
    let key = LookupKey::for_auction(&third, Seat::North).unwrap();
    assert_eq!(key.opener_pos, 3);
    let lookup = ir.index.resolve(&key);
    let guard = lookup.by_depth[1].expect("an entry for the third seat");
    assert_eq!(ir.node(guard).seat, SeatCond::ThirdOrFourth);
    assert_eq!(read_seat(&ir, "P P 1C 1S", Seat::South), off_system());
    // First seat: the edge is the trie's, whatever the seat (like a written `(any)`), so 1S
    // still reaches the guard, which has no entry there.
    let first = auction(Seat::North);
    let key = LookupKey::for_auction(&first, Seat::North).unwrap();
    let lookup = ir.index.resolve(&key);
    assert_eq!(lookup.matched_depth, 2);
    assert_eq!(lookup.by_depth[1], None);
}

/// The seat condition of the guard's node for `1C (1S)` with North opening in first seat
/// (dealer North) and in third seat (dealer South), `None` when it has no entry there.
fn guard_seats(ir: &SystemIR) -> [Option<SeatCond>; 2] {
    [
        (Seat::North, vec!["1C", "1S"]),
        (Seat::South, vec!["P", "P", "1C", "1S"]),
    ]
    .map(|(dealer, calls)| {
        let calls = calls.into_iter().map(|c| c.parse().unwrap());
        let auction = Auction::from_calls(dealer, Vulnerability::None, calls).unwrap();
        let key = LookupKey::for_auction(&auction, Seat::North).unwrap();
        let lookup = ir.index.resolve(&key);
        assert_eq!(lookup.matched_depth, 2);
        lookup.by_depth[1].map(|n| ir.node(n).seat)
    })
}

#[test]
fn tables_under_different_seat_conditions_each_guard_the_position() {
    // A `#SEAT 34` table and an unconditioned one guard the same position, in both orders.
    // Each guard behaves like a `1C-(any)-` written under its table's `#SEAT` after every
    // table, whatever the file order: the unconditioned table's guard has an entry in first
    // seat too.
    let opening = "1C = 12--21 hcp, 3+!c\n";
    let third = "#SEAT 34\n\n#EXACTPASS\n1C-\n1H = 6+ hcp, 4+!h\n";
    let any = "#SEAT 0\n\n#EXACTPASS\n1C-\n1N = 6--10 hcp\n";
    let written = |table: &str| {
        let seat = table.lines().next().unwrap();
        format!("{seat}\n\n1C-(any)-\n")
    };
    for (a, b, expected) in [
        (
            third,
            any,
            [Some(SeatCond::Any), Some(SeatCond::ThirdOrFourth)],
        ),
        // The third-seat guard's condition is covered by the unconditioned entry before it,
        // so it reuses that node (an empty description, like any history token).
        (any, third, [Some(SeatCond::Any), Some(SeatCond::Any)]),
    ] {
        let ir = compile(&format!("{opening}\n{a}\n{b}"));
        no_warnings(&ir);
        assert_eq!(guard_seats(&ir), expected, "{a}{b}");
        for dealer_calls in [("1C 1S", Seat::North), ("P P 1C 1S", Seat::South)] {
            assert_eq!(
                read_seat(&ir, dealer_calls.0, dealer_calls.1),
                off_system(),
                "{a}{b}{dealer_calls:?}"
            );
        }
        let by_hand = compile(&format!(
            "{opening}\n{}\n{}\n{}\n{}",
            a.replace("#EXACTPASS\n", ""),
            b.replace("#EXACTPASS\n", ""),
            written(a),
            written(b)
        ));
        assert_eq!(guard_seats(&by_hand), expected, "{a}{b}");
        assert_eq!(ir.nodes.len(), by_hand.nodes.len(), "{a}{b}");
        assert_eq!(ir.index.len(), by_hand.index.len(), "{a}{b}");
    }
}

#[test]
fn the_file_form_covers_the_later_tables_of_its_own_file_only() {
    let ir = compile_files(&[
        (
            "root.bml",
            "1C = 12--21 hcp, 3+!c
1D = 12--21 hcp, 4+!d
1H = 12--21 hcp, 5+!h
1S = 12--21 hcp, 5+!s

1C-
1H = 6+ hcp, 4+!h

#EXACTPASS FILE

1D-
1H = 6+ hcp, 4+!h

#INCLUDE part.bml

1H-
2H = 6--10 hcp, 3+!h
",
        ),
        (
            "part.bml",
            "1S-
2S = 6--10 hcp, 3+!s
",
        ),
    ]);
    no_warnings(&ir);
    assert_eq!(count(&ir, LintCode::ExactPassWithoutPass), 0);
    // Before the directive: not covered.
    assert_eq!(read(&ir, "1C 1S").0, 1);
    // After it, in the same file (also after the include): covered.
    assert_eq!(read(&ir, "1D 1S"), off_system());
    assert_eq!(read(&ir, "1H 1S"), off_system());
    // The included file is a file of its own: not covered.
    assert_eq!(read(&ir, "1S 2C").0, 1);
}

#[test]
fn the_file_form_in_an_included_file_stays_there() {
    let ir = compile_files(&[
        (
            "root.bml",
            "1C = 12--21 hcp, 3+!c
1D = 12--21 hcp, 4+!d

#INCLUDE part.bml

1D-
1H = 6+ hcp, 4+!h
",
        ),
        (
            "part.bml",
            "#EXACTPASS FILE

1C-
1H = 6+ hcp, 4+!h
",
        ),
    ]);
    no_warnings(&ir);
    assert_eq!(read(&ir, "1C 1S"), off_system());
    assert_eq!(read(&ir, "1D 1S").0, 1);
}

#[test]
fn misuse_is_reported() {
    // A table with no row of ours after a pass of theirs.
    let ir = compile(
        "1C = 12--21 hcp, 3+!c

#EXACTPASS
1C-(1S)-
X = 6+ hcp, 4+!h
",
    );
    assert_eq!(count(&ir, LintCode::ExactPassWithoutPass), 1);
    // A table of openings: there is no pass before an opening in the trie.
    let ir = compile("#EXACTPASS\n1C = 12--21 hcp, 3+!c\n");
    assert_eq!(count(&ir, LintCode::ExactPassWithoutPass), 1);
    // The file form with no such table after it.
    let ir = compile("1C-\n1H = 6+ hcp\n\n#EXACTPASS FILE\n\n1C = 12--21 hcp\n");
    assert_eq!(count(&ir, LintCode::ExactPassWithoutPass), 1);
    // The table form in a paragraph of its own names no table.
    let ir = compile("1C = 12--21 hcp\n\n#EXACTPASS\n\n1C-\n1H = 6+ hcp\n");
    assert!(
        ir.lints
            .iter()
            .any(|l| l.code == LintCode::UnknownDirective && l.message.contains("#EXACTPASS FILE")),
        "{:?}",
        ir.lints
    );
    assert_eq!(read(&ir, "1C 1S").0, 1);
    // The file form inside a table is ignored.
    let ir = compile("1C = 12--21 hcp\n\n1C-\n#EXACTPASS FILE\n1H = 6+ hcp\n");
    assert_eq!(count(&ir, LintCode::UnknownDirective), 1);
    assert_eq!(read(&ir, "1C 1S").0, 1);
}

/// The three examples of `docs/design/16-extended-bml.md` §4.8, with what the text says of
/// them (`bml_reference.rs` only checks that they compile cleanly).
#[test]
fn the_reference_examples_behave_as_documented() {
    let stayman = "1N = 15--17 hcp, bal

1N-
2C = !STAY, 8+ hcp, 4+ major
P = {prio:-100} {stop} 0--7 hcp

{DIRECTIVE}
1N-2C-
2D = no 4+ major
2H = 4+!h
2S = 4+!s, 0--3!h
";
    let ir = compile(&stayman.replace("{DIRECTIVE}", "#EXACTPASS"));
    no_warnings(&ir);
    assert_eq!(read(&ir, "1NT P 2C P"), (0, true, calls_of("2D 2H 2S")));
    assert_eq!(read(&ir, "1NT P 2C X"), off_system());
    assert_eq!(read(&ir, "1NT P 2C 2D"), off_system());
    let ir = compile(&stayman.replace("{DIRECTIVE}\n", ""));
    assert_eq!(read(&ir, "1NT P 2C X"), (1, true, calls_of("2D 2H 2S")));
    assert_eq!(read(&ir, "1NT P 2C 2D"), (1, true, calls_of("2D 2H 2S")));

    let ir = compile(
        "1S = 12--21 hcp, 5+!s

#EXACTPASS
1S-
2S = 6--10 hcp, 3+!s
4S = 0--9 hcp, 5+!s

1S-(bid)- = 8+ hcp
D = 10+ hcp, 0--2!s
",
    );
    no_warnings(&ir);
    assert_eq!(read(&ir, "1S 2H"), (0, true, calls_of("X")));
    assert_eq!(read(&ir, "1S X"), off_system());

    let ir = compile(
        "1C = 12--21 hcp, 3+!c
1D = 12--21 hcp, 3+!d

1C-
1H = 6+ hcp, 4+!h

#EXACTPASS FILE

1D-
1H = 6+ hcp, 4+!h

1D-1H-
1S = 4+!s
1N = 12--14 hcp, bal
",
    );
    no_warnings(&ir);
    assert_eq!(read(&ir, "1D 1S"), off_system());
    assert_eq!(read(&ir, "1D P 1H 2C"), off_system());
    assert_eq!(read(&ir, "1C 1S"), (1, true, calls_of("1H")));
}

#[test]
fn the_pass_of_an_alternative_is_guarded() {
    let ir = compile(
        "1C = 12--21 hcp, 3+!c

1C-
1H = 6+ hcp, 4+!h

#EXACTPASS
1C-1H-
(P/1S)
  1N = 12--14 hcp, bal
",
    );
    no_warnings(&ir);
    assert_eq!(read(&ir, "1C P 1H P"), (0, true, calls_of("1NT")));
    assert_eq!(read(&ir, "1C P 1H 1S"), (0, true, calls_of("1NT")));
    assert_eq!(read(&ir, "1C P 1H X"), off_system());
    assert_eq!(read(&ir, "1C P 1H 2C"), off_system());
    assert_eq!(guards(&ir, 6), 1);
}

#[test]
fn a_directive_in_a_cut_applies_to_the_table_it_is_pasted_into() {
    let source = "1C = 12--21 hcp, 3+!c

#CUT responses
{DIRECTIVE}
1H = 6+ hcp, 4+!h
#ENDCUT

1C-
#PASTE responses
1N = 6--10 hcp
";
    let plain = compile(&source.replace("{DIRECTIVE}\n", ""));
    assert_eq!(read(&plain, "1C 1S"), (1, true, calls_of("1H 1NT")));
    let ir = compile(&source.replace("{DIRECTIVE}", "#EXACTPASS"));
    no_warnings(&ir);
    assert_eq!(count(&ir, LintCode::ExactPassWithoutPass), 0);
    assert_eq!(read(&ir, "1C P"), (0, true, calls_of("1H 1NT")));
    assert_eq!(read(&ir, "1C 1S"), off_system());
    assert_eq!(read(&ir, "1C X"), off_system());
}

#[test]
fn a_pasted_table_belongs_to_the_file_it_is_pasted_in() {
    // The clip is cut in `part.bml` and pasted in `root.bml`: the pasted lines take the
    // `#PASTE` line's file, so the scope of `#EXACTPASS FILE` is the paste site's.
    let clip = "#CUT table
1C-
1H = 6+ hcp, 4+!h
#ENDCUT
";
    let guarded = compile_files(&[
        (
            "root.bml",
            "1C = 12--21 hcp, 3+!c

#INCLUDE part.bml

#EXACTPASS FILE

#PASTE table
",
        ),
        ("part.bml", clip),
    ]);
    no_warnings(&guarded);
    assert_eq!(count(&guarded, LintCode::ExactPassWithoutPass), 0);
    assert_eq!(read(&guarded, "1C 1S"), off_system());
    assert_eq!(read(&guarded, "1C P"), (0, true, calls_of("1H")));

    // The reverse: cut after `#EXACTPASS FILE` in `part.bml`, pasted in `root.bml`, which
    // has none. The table is not guarded, and the file form in `part.bml` covers no table.
    let unguarded = compile_files(&[
        (
            "root.bml",
            "1C = 12--21 hcp, 3+!c

#INCLUDE part.bml

#PASTE table
",
        ),
        ("part.bml", &format!("#EXACTPASS FILE\n\n{clip}")),
    ]);
    assert_eq!(count(&unguarded, LintCode::ExactPassWithoutPass), 1);
    assert_eq!(read(&unguarded, "1C 1S"), (1, true, calls_of("1H")));
}

#[test]
fn a_file_included_twice_has_two_scopes() {
    // Each inclusion is a file of its own: the 1D table before the directive is outside the
    // scope in both copies (the first copy's scope ends with that copy), and the 1C table
    // after it is inside in both.
    let ir = compile_files(&[
        (
            "root.bml",
            "1C = 12--21 hcp, 3+!c
1D = 12--21 hcp, 4+!d

#INCLUDE part.bml

#INCLUDE part.bml
",
        ),
        (
            "part.bml",
            "1D-
1H = 6+ hcp, 4+!h

#EXACTPASS FILE

1C-
1H = 6+ hcp, 4+!h
",
        ),
    ]);
    assert_eq!(count(&ir, LintCode::ExactPassWithoutPass), 0);
    assert_eq!(read(&ir, "1C 1S"), off_system());
    assert_eq!(read(&ir, "1D 1S"), (1, true, calls_of("1H")));
}

#[test]
fn a_written_pass_after_their_own_call_is_guarded_from_the_call_before_it() {
    // `(P)` follows their 1S, with our implicit pass between them: the guard is the `(any)`
    // sibling of that `(P)`, expanded from 1S like a row written after it (`1C-(1S)-P-(any)-`).
    let ir = compile(
        "1C = 12--21 hcp, 3+!c

#EXACTPASS
1C-(1S)-
(P)
  1N = 12--14 hcp, bal
",
    );
    no_warnings(&ir);
    assert_eq!(count(&ir, LintCode::ExactPassWithoutPass), 0);
    assert_eq!(read(&ir, "1C 1S P P"), (0, true, calls_of("1NT")));
    assert_eq!(read(&ir, "1C 1S P 2D"), off_system());
    assert_eq!(read(&ir, "1C 1S P 2S"), off_system());
    assert_eq!(guards(&ir, 3), 1);
}
