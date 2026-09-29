//! System stops (`#STOP`, `{stop}`; `docs/design/06-system.md` §4.5): a stop behaves exactly
//! like the pasted pass chains it replaces (our pass with any hand at `{prio:-100}`, whatever the
//! opponents call, round after round), without their nodes.

use bridge_core::{Auction, Call, Seat, Vulnerability};
use bridge_system::lexer::MemLoader;
use bridge_system::trie::LookupKey;
use bridge_system::{CompileOptions, LintCode, Severity, Side, SystemIR};

/// The body shared by both systems; `{AFTER}` marks where a row ends our bidding and `{PASS}`
/// where a table's lowest-ranked call is a pass that stops the partnership.
const BODY: &str = "1C = 12--21 hcp
1N = 15--17 hcp

1C-
1H = 6+ hcp, 4+!h
1N = 6--10 hcp
{AFTER}
{PASS}

1C-1H-
2H = 12--14 hcp, 4+!h
{AFTER}
1N = 12--14 hcp, bal
{PASS}

1C-(1S)-
X = 6+ hcp
{PASS}

1C-1N-(2S)-
X = 16+ hcp

1C-1N-(any)-
3N = 18+ hcp

1C-1N-(any)-P-(2D)-
X = 8+ hcp

1N-
2C = 8+ hcp
{PASS}

1N-2C-(X)-
2D = 0+ hcp
{PASS}
";

/// Eight rounds of the phase-4 SAYC chains, written out.
const CHAINS: &str = "#CUT pass-chain
P = {prio:-100} any hand
  (any)
    P = {prio:-100} any hand
      (any)
        P = {prio:-100} any hand
          (any)
            P = {prio:-100} any hand
              (any)
                P = {prio:-100} any hand
                  (any)
                    P = {prio:-100} any hand
                      (any)
                        P = {prio:-100} any hand
                          (any)
                            P = {prio:-100} any hand
#ENDCUT

#CUT after-chain
(any)
  P = {prio:-100} any hand
    (any)
      P = {prio:-100} any hand
        (any)
          P = {prio:-100} any hand
            (any)
              P = {prio:-100} any hand
                (any)
                  P = {prio:-100} any hand
                    (any)
                      P = {prio:-100} any hand
                        (any)
                          P = {prio:-100} any hand
                            (any)
                              P = {prio:-100} any hand
#ENDCUT

";

fn compile(source: &str) -> SystemIR {
    let opts = CompileOptions {
        coverage_samples: 0,
        ..CompileOptions::default()
    };
    let (ir, _) = bridge_system::compile("inline.bml", source, &MemLoader::default(), &opts);
    ir
}

fn with_chains() -> SystemIR {
    let body = BODY
        .replace("{AFTER}", "  #PASTE after-chain")
        .replace("{PASS}", "#PASTE pass-chain");
    compile(&format!("#+TITLE: chains\n\n{CHAINS}{body}"))
}

fn with_stops() -> SystemIR {
    let body = BODY
        .replace("{AFTER}", "  #STOP")
        .replace("{PASS}", "P = {prio:-100} {stop} any hand");
    compile(&format!("#+TITLE: stops\n\n{body}"))
}

/// What a consumer can see of one position: the resolve (matched depth, wildcard edges taken, the
/// node of every matched call) and the candidates at its end.
fn view(ir: &SystemIR, auction: &Auction, owner: Seat) -> String {
    let Some(key) = LookupKey::for_auction(auction, owner) else {
        return "none".to_string();
    };
    let lookup = ir.index.resolve(&key);
    let node = |id: bridge_system::NodeId| {
        let n = ir.node(id);
        format!(
            "{:?}/{:?}/{}/{:?}",
            n.call, n.side, n.priority, n.constraint
        )
    };
    let by_depth: Vec<String> = lookup
        .by_depth
        .iter()
        .map(|d| d.map_or("-".to_string(), node))
        .collect();
    let children: Vec<String> = ir
        .index
        .children(lookup.end, key.opener_pos, key.vul)
        .into_iter()
        .map(|(c, n)| format!("{c:?}={}", node(n)))
        .collect();
    format!(
        "depth {} via {} by {:?} children {:?}",
        lookup.matched_depth, lookup.via_class, by_depth, children
    )
}

/// A deterministic splitmix64 stream.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn calls(text: &str) -> Vec<Call> {
    text.split_whitespace()
        .map(|c| c.parse().unwrap())
        .collect()
}

#[test]
fn a_stop_resolves_exactly_like_the_pasted_chains() {
    let chains = with_chains();
    let stops = with_stops();
    let starts = [
        "1C P 1N",
        "1C P 1H P 2H",
        "1C P 1H P 1NT",
        "1C 1S",
        "1C P 1N 2S",
        "1C P 1N 2D",
        "1NT P",
        "1NT P 2C X",
        "1C P",
    ];
    let pool = calls("P P P P P X XX 2D 2H 2S 3C 3N 4S 5D 6H 7NT");
    let mut rng = Rng(0x5709);
    let mut positions = 0usize;
    let mut stopped = 0usize;
    for round in 0..3000 {
        let start = starts[round % starts.len()];
        let mut auction =
            Auction::from_calls(Seat::North, Vulnerability::None, calls(start)).unwrap();
        // Our side is North-South: after the start, at most eight rounds of ours (the chains'
        // depth), so both systems must agree on every prefix.
        for _ in 0..14 {
            for owner in [Seat::North, Seat::East] {
                let seen = view(&stops, &auction, owner);
                assert_eq!(
                    view(&chains, &auction, owner),
                    seen,
                    "{auction} for {owner:?}"
                );
                positions += 1;
                stopped += usize::from(seen.contains("=Pass/Us/-100/"));
            }
            if auction.is_complete() {
                break;
            }
            let legal: Vec<Call> = pool
                .iter()
                .copied()
                .filter(|&c| auction.is_legal(c))
                .collect();
            let call = legal[(rng.next() % legal.len() as u64) as usize];
            auction.push(call).unwrap();
        }
    }
    eprintln!("{positions} positions, {stopped} offering the stop pass");
    assert!(
        positions > 20_000 && stopped > 5_000,
        "{positions} {stopped}"
    );
    assert!(stops.nodes.len() < chains.nodes.len() / 3);
}

#[test]
fn the_stop_pass_is_offered_after_any_call_of_theirs_and_loops() {
    let ir = with_stops();
    let mut auction =
        Auction::from_calls(Seat::North, Vulnerability::None, calls("1C P 1N 2H P 3H")).unwrap();
    // The opponents bid on alone for 20 rounds of ours: the chains stopped after six (eight in
    // this file), the stop never does.
    let bids = calls("3S 3N 4C 4D 4H 4S 4N 5C 5D 5H 5S 5N 6C 6D 6H 6S 6N 7C 7D 7H");
    for (round, &bid) in bids.iter().enumerate() {
        let owner = auction.next_seat();
        let key = LookupKey::for_auction(&auction, owner).unwrap();
        let lookup = ir.index.resolve(&key);
        assert_eq!(lookup.matched_depth, key.calls.len(), "round {round}");
        let children = ir.index.children(lookup.end, key.opener_pos, key.vul);
        assert_eq!(children.len(), 1, "round {round}");
        let (call, node) = children[0];
        assert_eq!(call, Call::Pass);
        let node = ir.node(node);
        assert!(node.is_synthesised() && node.flags.stop);
        assert_eq!((node.side, node.priority), (Side::Us, -100));
        assert_eq!(
            format!("{:?}", node.constraint),
            format!("{:?}", bridge_constraint::HandConstraint::ANY)
        );
        auction.push(Call::Pass).unwrap();
        auction.push(bid).unwrap();
    }
}

#[test]
fn an_exact_call_of_theirs_after_the_stop_keeps_its_table() {
    let ir = with_stops();
    // `1C-1N-(2S)-` has its own table: opener's double, and no pass row.
    let auction =
        Auction::from_calls(Seat::North, Vulnerability::None, calls("1C P 1N 2S")).unwrap();
    let key = LookupKey::for_auction(&auction, Seat::North).unwrap();
    let lookup = ir.index.resolve(&key);
    assert_eq!(lookup.matched_depth, 4);
    let children: Vec<Call> = ir
        .index
        .children(lookup.end, key.opener_pos, key.vul)
        .into_iter()
        .map(|(c, _)| c)
        .collect();
    assert_eq!(children, vec![Call::Double]);
    // After any other call, the `(any)` table's 3NT joins the stop pass.
    let auction =
        Auction::from_calls(Seat::North, Vulnerability::None, calls("1C P 1N 3D")).unwrap();
    let key = LookupKey::for_auction(&auction, Seat::North).unwrap();
    let lookup = ir.index.resolve(&key);
    let mut children: Vec<(Call, i16)> = ir
        .index
        .children(lookup.end, key.opener_pos, key.vul)
        .into_iter()
        .map(|(c, n)| (c, ir.node(n).priority))
        .collect();
    children.sort_by_key(|&(c, _)| c.index());
    assert_eq!(children, vec![(Call::Pass, -100), (calls("3N")[0], 0)]);
}

#[test]
fn a_call_of_ours_after_the_stop_leaves_the_system() {
    let ir = with_stops();
    let auction =
        Auction::from_calls(Seat::North, Vulnerability::None, calls("1C P 1N 2H 3C P")).unwrap();
    let key = LookupKey::for_auction(&auction, Seat::North).unwrap();
    let lookup = ir.index.resolve(&key);
    // 3C is not a row after the stop: the walk ends before it, exactly as with the chains.
    assert_eq!(lookup.matched_depth, 4);
}

#[test]
fn stop_markers_parse_and_compile_cleanly() {
    let ir = with_stops();
    assert!(
        !ir.lints
            .iter()
            .any(|l| l.severity == Severity::Error || l.code == LintCode::UnknownDirective),
        "{:?}",
        ir.lints
    );
    // The `1C-1N-(any)-P-(2D)-` header's empty pass placeholder took the stop pass's content.
    let auction =
        Auction::from_calls(Seat::North, Vulnerability::None, calls("1C P 1N 3C P")).unwrap();
    let key = LookupKey::for_auction(&auction, Seat::North).unwrap();
    let lookup = ir.index.resolve(&key);
    let pass = lookup.by_depth[4].expect("the placeholder");
    assert_eq!(ir.node(pass).priority, -100);
    assert!(ir.node(pass).flags.stop);
    // It took the stop pass's content, but it is still the header's own node.
    assert!(!ir.node(pass).is_synthesised() && !ir.node(pass).path.is_empty());
    // Two synthesised nodes, both without a position.
    assert_eq!(ir.nodes.iter().filter(|n| n.is_synthesised()).count(), 2);
    // The nodes of the stop rows are flagged.
    let flagged = ir
        .nodes
        .iter()
        .filter(|n| n.flags.stop && !n.is_synthesised())
        .count();
    assert!(flagged >= 7, "{flagged}");
}

#[test]
fn a_stop_at_the_top_of_a_table_without_history_is_reported() {
    let ir = compile("1C = 12+ hcp\n#STOP\n");
    assert!(
        ir.lints
            .iter()
            .any(|l| l.code == LintCode::UnknownDirective)
    );
    assert!(!ir.nodes.iter().any(|n| n.is_synthesised()));
}

/// Compiles `body` twice, with `{AFTER}`/`{PASS}` as the pasted chains and as stops, and checks
/// that both resolve alike on every prefix of random continuations of `starts` (both
/// partnerships, every dealer and vulnerability given) as far as the chains reach.
fn assert_chain_equivalent(body: &str, starts: &[&str], seed: u64) -> usize {
    let chains = compile(&format!(
        "#+TITLE: chains\n\n{CHAINS}{}",
        body.replace("{AFTER}", "#PASTE after-chain")
            .replace("{PASS}", "#PASTE pass-chain")
    ));
    let stops = compile(&format!(
        "#+TITLE: stops\n\n{}",
        body.replace("{AFTER}", "#STOP")
            .replace("{PASS}", "P = {prio:-100} {stop} any hand")
    ));
    let pool = calls("P P P P P X XX 2D 2H 2S 3C 3N 4S 5D 6H 7NT");
    let mut rng = Rng(seed);
    let mut stopped = 0usize;
    for round in 0..400 {
        let start = starts[round % starts.len()];
        let dealer = Seat::ALL[(round / starts.len()) % 4];
        let vul = [
            Vulnerability::None,
            Vulnerability::NS,
            Vulnerability::EW,
            Vulnerability::Both,
        ][(round / (4 * starts.len())) % 4];
        let Ok(mut auction) = Auction::from_calls(dealer, vul, calls(start)) else {
            continue;
        };
        for _ in 0..12 {
            for owner in [Seat::North, Seat::East] {
                let seen = view(&stops, &auction, owner);
                assert_eq!(
                    view(&chains, &auction, owner),
                    seen,
                    "{auction} (dealer {dealer:?}, {vul:?}) for {owner:?}"
                );
                stopped += usize::from(seen.contains("=Pass/Us/-100/"));
            }
            if auction.is_complete() {
                break;
            }
            let legal: Vec<Call> = pool
                .iter()
                .copied()
                .filter(|&c| auction.is_legal(c))
                .collect();
            let call = legal[(rng.next() % legal.len() as u64) as usize];
            auction.push(call).unwrap();
        }
    }
    stopped
}

/// The candidates of `owner` after `text` (dealer North).
fn children_after(ir: &SystemIR, vul: Vulnerability, text: &str, owner: Seat) -> Vec<Call> {
    let auction = Auction::from_calls(Seat::North, vul, calls(text)).unwrap();
    let key = LookupKey::for_auction(&auction, owner).unwrap();
    let lookup = ir.index.resolve(&key);
    assert_eq!(
        lookup.matched_depth,
        key.calls.len(),
        "{text} for {owner:?}"
    );
    ir.index
        .children(lookup.end, key.opener_pos, key.vul)
        .into_iter()
        .map(|(c, _)| c)
        .collect()
}

#[test]
fn stops_under_different_seat_conditions_at_one_position_each_keep_the_stop_pass() {
    let body = "#SEAT 34\n\n1N = 15--17 hcp\n\n1N-\n3N = 10+ hcp\n  {AFTER}\n\n\
                #SEAT 12\n\n1N = 12--14 hcp\n\n1N-\n3N = 12+ hcp\n  {AFTER}\n";
    let stopped = assert_chain_equivalent(body, &["1N P 3N", "P P 1N P 3N", "P 1N P 3N"], 0x5e47);
    assert!(stopped > 100, "{stopped}");
    // The second condition's stop, met where the first one's loop was already linked.
    let ir = compile(&format!(
        "#+TITLE: stops\n\n{}",
        body.replace("{AFTER}", "#STOP")
    ));
    assert_eq!(
        children_after(&ir, Vulnerability::None, "1N P 3N X", Seat::North),
        vec![Call::Pass]
    );
    assert_eq!(
        children_after(&ir, Vulnerability::None, "1N P 3N X P 4C", Seat::South),
        vec![Call::Pass]
    );
    assert_eq!(
        children_after(&ir, Vulnerability::None, "P P 1N P 3N X P 4C", Seat::South),
        vec![Call::Pass]
    );
}

#[test]
fn a_general_stop_after_a_specific_one_at_one_position_keeps_the_stop_pass() {
    let body = "#VUL Y0\n\n1S-\n4S = 8+ hcp, 4+!s\n  {AFTER}\n\n\
                #VUL 00\n\n1S = 12+ hcp\n\n1S-\n4S = 5+ hcp, 5+!s\n  {AFTER}\n";
    let stopped = assert_chain_equivalent(body, &["1S P 4S", "P 1S P 4S"], 0x5e48);
    assert!(stopped > 100, "{stopped}");
    let ir = compile(&format!(
        "#+TITLE: stops\n\n{}",
        body.replace("{AFTER}", "#STOP")
    ));
    for vul in [Vulnerability::None, Vulnerability::NS] {
        assert_eq!(
            children_after(&ir, vul, "1S P 4S 5C", Seat::North),
            vec![Call::Pass],
            "{vul:?}"
        );
        assert_eq!(
            children_after(&ir, vul, "1S P 4S 5C P 5D", Seat::South),
            vec![Call::Pass],
            "{vul:?}"
        );
    }
}

#[test]
fn a_stop_inside_another_stops_continuation_under_another_condition() {
    let body = "1N = 15--17 hcp\n\n1N-\n3N = 10+ hcp\n  {AFTER}\n\n\
                #SEAT 34\n\n1N-3N-(X)-\nXX = 4+!c\n  {AFTER}\nP = any hand\n  {AFTER}\n\n\
                #SEAT 12\n\n1N-3N-(X)-\nP = 0+ hcp\n  {AFTER}\n";
    let stopped = assert_chain_equivalent(
        body,
        &["1N P 3N X", "P P 1N P 3N X", "1N P 3N 4C", "P P 1N P 3N 4D"],
        0x5e49,
    );
    assert!(stopped > 100, "{stopped}");
}

#[test]
fn a_spaced_stop_annotation_is_a_stop() {
    for annotation in ["{stop}", "{ stop }", "{stop }"] {
        let ir = compile(&format!(
            "#+TITLE: t\n\n1C = 12+ hcp\n\n1C-1H-\nP = {{prio:-100}} {annotation} any hand\n"
        ));
        assert_eq!(
            ir.nodes.iter().filter(|n| n.is_synthesised()).count(),
            2,
            "{annotation}"
        );
        assert_eq!(
            children_after(&ir, Vulnerability::None, "1C P 1H P P 2S", Seat::South),
            vec![Call::Pass],
            "{annotation}"
        );
    }
}
