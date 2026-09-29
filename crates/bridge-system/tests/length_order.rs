//! Suit-length comparisons in descriptions (`!s>=!h`, `M>oM`; `docs/design/06-system.md` §7.4
//! extension): the relative lengths that preference and "longer major" agreements state.

use bridge_core::{Auction, Call, Hand, Seat, Vulnerability};
use bridge_system::lexer::MemLoader;
use bridge_system::trie::LookupKey;
use bridge_system::{CompileOptions, LintCode, Node, SystemIR};

fn compile(source: &str) -> SystemIR {
    let opts = CompileOptions {
        coverage_samples: 0,
        ..CompileOptions::default()
    };
    let (ir, _) = bridge_system::compile("inline.bml", source, &MemLoader::default(), &opts);
    ir
}

fn node<'a>(ir: &'a SystemIR, auction: &str, call: &str) -> &'a Node {
    let calls: Vec<Call> = auction
        .split_whitespace()
        .map(|c| c.parse().unwrap())
        .collect();
    let auction = Auction::from_calls(Seat::North, Vulnerability::None, calls).unwrap();
    let key = LookupKey::for_auction(&auction, Seat::North).unwrap();
    let lookup = ir.index.resolve(&key);
    let id = ir
        .index
        .children(lookup.end, key.opener_pos, key.vul)
        .into_iter()
        .find(|(c, _)| format!("{c}") == call)
        .map(|(_, n)| n)
        .unwrap_or_else(|| panic!("{call} is not a row after {auction}"));
    ir.node(id)
}

fn hand(text: &str) -> Hand {
    text.parse().unwrap()
}

#[test]
fn a_preference_needs_at_least_as_many_cards_in_the_first_suit() {
    let ir = compile(
        "1S-1N-2H-
2S = preference, 2+!s, !s>=!h, 6--10 hcp
",
    );
    let pref = node(&ir, "1S Pass 1NT Pass 2H Pass", "2S");
    let row = &ir.rows[pref.row.0 as usize];
    assert_eq!(
        row.recognition.unrecognized.len(),
        1,
        "{:?}",
        row.recognition
    );
    // 3 spades, 3 hearts: yes. 2 spades, 3 hearts: no. 2-2: yes.
    assert!(pref.constraint.satisfies(hand("K72.Q54.K8643.92")));
    assert!(!pref.constraint.satisfies(hand("K7.Q54.K8643.972")));
    assert!(pref.constraint.satisfies(hand("K7.Q5.K86432.972")));
}

#[test]
fn variables_are_substituted_before_the_comparison_is_read() {
    // `M>oM`: the longer major, whichever `M` binds to.
    let ir = compile(
        "1C-(X)-
1M = 6+ hcp, 4+M, M>oM
",
    );
    let hearts = node(&ir, "1C X", "1H");
    let spades = node(&ir, "1C X", "1S");
    let h5s4 = hand("K742.AQ543.72.92");
    let h4s5 = hand("AQ543.K742.72.92");
    assert!(hearts.constraint.satisfies(h5s4));
    assert!(!hearts.constraint.satisfies(h4s5));
    assert!(spades.constraint.satisfies(h4s5));
    assert!(!spades.constraint.satisfies(h5s4));
    assert!(
        !ir.lints
            .iter()
            .any(|l| l.code == LintCode::UnrecognizedFragment && l.message.contains('>')),
        "{:?}",
        ir.lints
    );
}

#[test]
fn every_operator_compiles() {
    let ir = compile(
        "1C-
1D = !d>!c
1H = !h<=!c
1S = !s<!d
1N = !d = !c
",
    );
    let ge_hand = hand("K74.Q54.K643.A92"); // 3-3-4-3
    assert!(node(&ir, "1C Pass", "1D").constraint.satisfies(ge_hand));
    assert!(node(&ir, "1C Pass", "1H").constraint.satisfies(ge_hand));
    assert!(node(&ir, "1C Pass", "1S").constraint.satisfies(ge_hand));
    assert!(!node(&ir, "1C Pass", "1NT").constraint.satisfies(ge_hand));
    // 3-2-4-4: diamonds equal to clubs.
    assert!(
        node(&ir, "1C Pass", "1NT")
            .constraint
            .satisfies(hand("K74.Q5.K643.A932"))
    );
}
