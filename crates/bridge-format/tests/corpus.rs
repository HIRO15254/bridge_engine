//! Corpus tests (`cargo test -p bridge-format --release -- --ignored --nocapture`).
//!
//! They read `BRIDGE_CORPUS_DIR` (default `<workspace>/corpus/data`) and skip when it is absent.

mod common;

use bridge_core::Contract;
use bridge_format::{
    GameView, WriteOptions, deal_string, lin,
    pbn::{self, ViewError},
};
use common::{corpus_dir, files_with_extension};

/// Why a game does not count as parsed.
fn game_problem(game: &pbn::Game, view: &Result<GameView, ViewError>) -> Option<String> {
    let view = match view {
        Ok(v) => v,
        Err(e) => return Some(format!("view error: {e}")),
    };
    if view.deal.is_none() {
        return Some("no Deal".to_string());
    }
    if game.sections.iter().any(|s| s.tag == "Auction") {
        let Some(auction) = &view.auction else {
            return Some("Auction section not interpreted".to_string());
        };
        if let Some(contract) = view.contract {
            match auction.contract() {
                Some(c) if c == contract => {}
                other => {
                    return Some(format!("auction gives {other:?}, tags say {contract:?}"));
                }
            }
        } else if game.get("Contract").is_some() && !auction.is_passed_out() {
            return Some("Contract tag is Pass but the auction is not passed out".to_string());
        }
    }
    if game.sections.iter().any(|s| s.tag == "Play") && view.play.is_none() {
        return Some("Play section not interpreted".to_string());
    }
    None
}

#[test]
#[ignore = "needs the corpus"]
fn pbn_parse_rate() {
    let Some(dir) = corpus_dir() else { return };
    let files = files_with_extension(&dir.join("pbn"), "pbn");
    assert!(!files.is_empty(), "no PBN files under {}", dir.display());
    let mut total = 0usize;
    let mut parsed = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut warnings_total = 0usize;
    let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        let (file, warnings) = pbn::parse_lenient(&bytes);
        warnings_total += warnings.len();
        for w in &warnings {
            *kinds.entry(format!("{:?}", w.kind)).or_default() += 1;
        }
        let mut file_parsed = 0;
        let mut previous: Option<GameView> = None;
        for (k, game) in file.games.iter().enumerate() {
            let view = game.view(previous.as_ref());
            match game_problem(game, &view) {
                None => file_parsed += 1,
                Some(reason) => failures.push(format!("{}: game {k}: {reason}", path.display())),
            }
            if let Ok(v) = view {
                previous = Some(v);
            }
        }
        println!(
            "{:<60} games {:>4} parsed {:>4} warnings {:>3}",
            path.strip_prefix(&dir).unwrap_or(path).display(),
            file.games.len(),
            file_parsed,
            warnings.len()
        );
        total += file.games.len();
        parsed += file_parsed;
    }
    let rate = parsed as f64 / total as f64;
    println!(
        "PBN: {parsed}/{total} games parsed ({:.2}%), {warnings_total} warnings, {} failures",
        rate * 100.0,
        failures.len()
    );
    println!("  warning kinds: {kinds:?}");
    for failure in failures.iter().take(10) {
        println!("  {failure}");
    }
    assert!(rate >= 0.99, "PBN parse rate {rate:.4} below 99%");
}

fn view_summary(v: &Result<GameView, ViewError>) -> String {
    match v {
        Ok(v) => format!(
            "{:?} {:?} {:?} {:?} {:?} {:?}",
            v.deal,
            v.auction,
            v.play.as_ref().map(|p| p.cards().to_vec()),
            v.contract
                .map(|c: Contract| (c.bid, c.declarer, c.doubling)),
            v.result,
            v.dd_table
        ),
        Err(e) => format!("error: {e}"),
    }
}

#[test]
#[ignore = "needs the corpus"]
fn pbn_round_trip() {
    let Some(dir) = corpus_dir() else { return };
    let files = files_with_extension(&dir.join("pbn"), "pbn");
    let mut games = 0usize;
    let mut mismatches: Vec<String> = Vec::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        let (file, _) = pbn::parse_lenient(&bytes);
        let text = pbn::write(&file, WriteOptions { export: true });
        let reparsed = match pbn::parse_strict(&text) {
            Ok(f) => f,
            Err(e) => {
                let line = text.lines().nth(e.line as usize - 1).unwrap_or("");
                mismatches.push(format!(
                    "{}: strict parse failed: {e}: {line:?}",
                    path.display()
                ));
                continue;
            }
        };
        assert_eq!(reparsed.games.len(), file.games.len(), "{}", path.display());
        let mut prev_a: Option<GameView> = None;
        let mut prev_b: Option<GameView> = None;
        for (k, (a, b)) in file.games.iter().zip(&reparsed.games).enumerate() {
            games += 1;
            let va = a.view(prev_a.as_ref());
            let vb = b.view(prev_b.as_ref());
            let (sa, sb) = (view_summary(&va), view_summary(&vb));
            if sa != sb {
                mismatches.push(format!(
                    "{}: game {k}:\n  before: {sa}\n  after:  {sb}",
                    path.display()
                ));
            }
            prev_a = va.ok();
            prev_b = vb.ok();
        }
    }
    println!(
        "PBN round trip: {games} games, {} mismatches",
        mismatches.len()
    );
    for m in mismatches.iter().take(5) {
        println!("  {m}");
    }
    assert!(mismatches.is_empty());
}

#[test]
#[ignore = "needs the corpus"]
fn lin_parse_rate() {
    let Some(dir) = corpus_dir() else { return };
    let files = files_with_extension(&dir.join("lin"), "lin");
    assert!(!files.is_empty(), "no LIN files under {}", dir.display());
    let mut total = 0usize;
    let mut ok = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        let (boards, warnings) = lin::parse_lenient(&bytes);
        let mut file_ok = 0;
        for (k, board) in boards.iter().enumerate() {
            let game = board.to_game();
            let view = game.view(None);
            let good = match &view {
                Ok(v) => v.deal.is_some_and(|d| d.complete().is_some()) && v.auction.is_some(),
                Err(_) => false,
            };
            if good {
                file_ok += 1;
            } else {
                failures.push(format!(
                    "{}: board {k} ({:?}): {}",
                    path.display(),
                    board.id,
                    match &view {
                        Ok(v) => format!(
                            "deal {:?} auction {:?}",
                            v.deal.is_some(),
                            v.auction.is_some()
                        ),
                        Err(e) => e.to_string(),
                    }
                ));
            }
        }
        println!(
            "{:<40} boards {:>3} ok {:>3} warnings {:>3}",
            path.strip_prefix(&dir).unwrap_or(path).display(),
            boards.len(),
            file_ok,
            warnings.len()
        );
        for w in warnings.iter().take(3) {
            println!(
                "    warning: board {} line {} {:?} {}",
                w.game, w.line, w.kind, w.message
            );
        }
        total += boards.len();
        ok += file_ok;
    }
    let rate = ok as f64 / total as f64;
    println!(
        "LIN: {ok}/{total} boards ({:.2}%), {} failures",
        rate * 100.0,
        failures.len()
    );
    for f in failures.iter().take(10) {
        println!("  {f}");
    }
    assert!(rate >= 0.95, "LIN rate {rate:.4} below 95%");
}

#[test]
#[ignore = "needs the corpus"]
fn dds_deal_strings() {
    let Some(dir) = corpus_dir() else { return };
    let path = dir.join("dds/list100.txt");
    if !path.is_file() {
        eprintln!("{} not found; skipping", path.display());
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap();
    let mut count = 0;
    for line in text.lines() {
        // `PBN <trump> <first> <?> <?> "N:… … … …"`
        let Some(rest) = line.strip_prefix("PBN ") else {
            continue;
        };
        let Some(start) = rest.find('"') else {
            continue;
        };
        let Some(end) = rest[start + 1..].find('"') else {
            continue;
        };
        let deal = &rest[start + 1..start + 1 + end];
        let parsed = deal_string::parse(deal).unwrap_or_else(|e| panic!("{deal}: {e}"));
        let full = parsed
            .complete()
            .unwrap_or_else(|| panic!("{deal}: incomplete"));
        let first: bridge_core::Seat = deal[..1].parse().unwrap();
        assert_eq!(deal_string::write(&full, first), deal);
        count += 1;
    }
    println!("DDS: {count} deal strings parsed");
    assert!(count >= 100);
}
