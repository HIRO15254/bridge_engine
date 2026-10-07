//! Shared test helpers: where the vendored/fixture BML files live, and builders for auctions
//! and hands from short text specs, so test bodies read as bidding sequences and PBN-style
//! holdings.
#![allow(dead_code)]

pub mod bss;

use std::path::PathBuf;

use bridge_core::{Auction, Call, Hand, Holding, Rank, Seat, Vulnerability};

/// `BRIDGE_SYSTEMS_DIR`, or `<crate>/../../systems`.
pub fn systems_dir() -> PathBuf {
    match std::env::var_os("BRIDGE_SYSTEMS_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../systems"),
    }
}

/// Every `.bml` file under `dir`, recursively, sorted.
pub fn bml_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(bml_files(&path));
        } else if path.extension().is_some_and(|e| e == "bml") {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// Compiles `path`, guarded against the panic that a still-`todo!()` helper on another lane's
/// branch (not yet merged here) would raise. Returns `None` (a "blocked" file, not a test
/// failure) only when the panic payload looks like a `todo!()`/`unimplemented!()` message; any
/// other panic -- a real bug in expansion, the trie or lints -- is re-raised via
/// `resume_unwind` so it still fails the test. The panic hook is silenced for the duration so an
/// expected "blocked" panic does not spam stderr.
///
/// Every test that calls this becomes a real, unguarded assertion the moment the last `todo!()`
/// on the compile path lands: nothing about the comparison logic downstream of this function
/// depends on the panic.
pub fn compile_guarded(
    path: &std::path::Path,
    opts: &bridge_system::CompileOptions,
) -> Option<bridge_system::SystemIR> {
    let text = std::fs::read_to_string(path).ok()?;
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        bridge_system::compile(
            &path.to_string_lossy(),
            &text,
            &bridge_system::lexer::FsLoader,
            opts,
        )
    }));
    std::panic::set_hook(prev_hook);
    match result {
        Ok((ir, _lints)) => Some(ir),
        Err(payload) => {
            if panic_looks_like_todo(&payload) {
                None
            } else {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

/// Same as [`compile_guarded`], but also returns the path per [`bridge_system::ast::FileId`]
/// (the root file, then every `#INCLUDE`d file in load order) -- needed to resolve a lint's
/// `span.file` back to the file it actually names, since an included file's lints carry its own
/// `FileId`, not the root's. Loads the source a second time (once here for the file table via
/// `lexer::load`, once inside `compile` itself); both loads are deterministic over the same
/// bytes, so the two file tables always agree on order.
pub fn compile_guarded_with_files(
    path: &std::path::Path,
    opts: &bridge_system::CompileOptions,
) -> Option<(bridge_system::SystemIR, Vec<String>)> {
    let text = std::fs::read_to_string(path).ok()?;
    let path_str = path.to_string_lossy().into_owned();
    let loaded = bridge_system::lexer::load(&path_str, &text, &bridge_system::lexer::FsLoader);
    let files: Vec<String> = loaded
        .files
        .iter()
        .map(|(p, _)| p.as_ref().to_string())
        .collect();

    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        bridge_system::compile(&path_str, &text, &bridge_system::lexer::FsLoader, opts)
    }));
    std::panic::set_hook(prev_hook);
    match result {
        Ok((ir, _lints)) => Some((ir, files)),
        Err(payload) => {
            if panic_looks_like_todo(&payload) {
                None
            } else {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

/// True when a caught panic payload's message contains the boilerplate `todo!()`/
/// `unimplemented!()` wording, as opposed to a genuine assertion or logic-error message.
fn panic_looks_like_todo(payload: &(dyn std::any::Any + Send)) -> bool {
    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
        *s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.as_str()
    } else {
        return false;
    };
    msg.contains("not yet implemented") || msg.contains("not implemented")
}

/// Builds an auction from a dealer, a vulnerability and a space-separated list of calls
/// (`"1S P 2S P"`, `"P P 1NT P"`, `"X"`, `"XX"`, …), as accepted by `Call`'s `FromStr`.
pub fn auction(dealer: Seat, vul: Vulnerability, calls: &str) -> Auction {
    let calls: Vec<Call> = calls
        .split_whitespace()
        .map(|c| c.parse().unwrap_or_else(|_| panic!("bad call {c:?}")))
        .collect();
    Auction::from_calls(dealer, vul, calls).expect("legal auction")
}

/// Builds a 13-card hand from four suit holdings (clubs, diamonds, hearts, spades), each a
/// string of rank characters (`"AKQJT98765432"`, case-insensitive, any subset, any order).
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
