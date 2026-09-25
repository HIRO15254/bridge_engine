//! Property: `parse_strict(write(g, export))` equals a normalised `g`, writing is idempotent,
//! and the views agree.

mod common;

use bridge_core::{Auction, Call, Seat, Vulnerability};
use bridge_format::{
    PbnFile, WarningKind, WriteOptions, deal_string,
    pbn::{self, Comment, Directive, Game, Section, TagPair, TagValue, Token},
};
use common::{arb_auction, arb_deal, arb_seat, play_cards};
use proptest::prelude::*;
use proptest::sample::Index;

const MTS: [&str; 15] = [
    "Event",
    "Site",
    "Date",
    "Board",
    "West",
    "North",
    "East",
    "South",
    "Dealer",
    "Vulnerable",
    "Deal",
    "Scoring",
    "Declarer",
    "Contract",
    "Result",
];

/// Tags the generator may add, several times (duplicates are dropped by the writer).
const OPTIONAL: [&str; 10] = [
    "Event", "Site", "Date", "West", "North", "East", "South", "Scoring", "Room", "Score",
];

const WORDS: [&str; 12] = [
    "Test",
    "Cup 2019",
    "a \"quoted\" name",
    "back\\slash",
    "Café",
    "Møller-Åström",
    "?",
    "",
    "The Hague [NL]",
    "semi;colon",
    "brace {x}",
    "#",
];

fn spell_vulnerability(v: Vulnerability, variant: usize) -> &'static str {
    match (v, variant % 3) {
        (Vulnerability::None, 0) => "None",
        (Vulnerability::None, 1) => "Love",
        (Vulnerability::None, _) => "-",
        (Vulnerability::NS, 0) => "NS",
        (Vulnerability::NS, _) => "ns",
        (Vulnerability::EW, 0) => "EW",
        (Vulnerability::EW, _) => "ew",
        (Vulnerability::Both, 0) => "All",
        (Vulnerability::Both, 1) => "Both",
        (Vulnerability::Both, _) => "all",
    }
}

fn tag(name: &str, value: &str) -> TagPair {
    TagPair {
        name: name.to_string(),
        value: TagValue::Str(value.to_string()),
        line: 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_game(
    deal: bridge_core::Deal,
    auction: Auction,
    play_picks: Vec<Index>,
    first: Seat,
    lowercase_deal: bool,
    board: Option<u16>,
    optional: Vec<(usize, usize)>,
    annotations: Vec<(Index, u8)>,
    comments: Vec<usize>,
    vul_variant: usize,
    order: Vec<u32>,
) -> Game {
    let dealer = auction.dealer();
    let contract = auction.contract();
    let mut tags = vec![
        tag(
            "Dealer",
            &if vul_variant % 2 == 0 {
                dealer.to_string()
            } else {
                dealer.to_string().to_ascii_lowercase()
            },
        ),
        tag(
            "Vulnerable",
            spell_vulnerability(auction.vulnerability(), vul_variant),
        ),
    ];
    let mut deal_text = deal_string::write(&deal, first);
    if lowercase_deal {
        deal_text = deal_text.to_ascii_lowercase();
    }
    tags.push(tag("Deal", &deal_text));
    if let Some(b) = board {
        tags.push(tag("Board", &b.to_string()));
    }
    for (name, word) in optional {
        tags.push(tag(OPTIONAL[name], WORDS[word]));
    }
    match contract {
        Some(c) => {
            tags.push(tag("Contract", &c.to_string()));
            tags.push(tag("Declarer", &c.declarer.to_string()));
        }
        None => tags.push(tag("Contract", "Pass")),
    }

    let mut sections = Vec::new();
    let mut auction_section = Section {
        tag: "Auction".to_string(),
        arg: dealer.to_string(),
        tokens: Vec::new(),
        notes: Vec::new(),
    };
    let calls = auction.calls();
    for (i, call) in calls.iter().enumerate() {
        auction_section.tokens.push(Token::Call(*call));
        for (index, kind) in &annotations {
            if index.index(calls.len()) != i {
                continue;
            }
            match kind {
                0 => {
                    let n = auction_section.notes.len() as u8 + 1;
                    if n <= 32 {
                        auction_section.tokens.push(Token::NoteRef(n));
                        auction_section
                            .notes
                            .push((n, WORDS[(i + n as usize) % WORDS.len()].to_string()));
                    }
                }
                1 => auction_section.tokens.push(Token::Nag(1 + i as u8 % 13)),
                2 => auction_section.tokens.push(Token::Suffix(
                    ["!", "?", "!!", "??", "!?", "?!"][i % 6].to_string(),
                )),
                _ => {
                    auction_section.tokens.push(Token::Nag(9));
                    auction_section.tokens.push(Token::Nag(2));
                    let n = auction_section.notes.len() as u8 + 1;
                    if n <= 32 {
                        auction_section.tokens.push(Token::NoteRef(n));
                        auction_section.notes.push((n, "note".to_string()));
                    }
                }
            }
        }
    }
    sections.push(auction_section);

    if let Some(c) = contract
        && !play_picks.is_empty()
    {
        let leader = c.leader();
        let history = play_cards(&deal, c.bid.strain(), leader, &play_picks);
        let mut section = Section {
            tag: "Play".to_string(),
            arg: leader.to_string(),
            tokens: Vec::new(),
            notes: Vec::new(),
        };
        for trick in history.tricks() {
            let mut row = vec![Token::Unknown; 4];
            for (k, card) in trick.cards.iter().enumerate() {
                if let Some(card) = card {
                    let seat = trick.leader.offset(k as u8);
                    row[((seat.index() + 4 - leader.index()) % 4) as usize] = Token::Card(*card);
                }
            }
            section.tokens.extend(row);
        }
        if !annotations.is_empty() {
            section.tokens.insert(1, Token::Suffix("?!".to_string()));
            section.tokens.insert(1, Token::NoteRef(1));
            section.notes.push((1, "lead".to_string()));
        }
        if history.cards().len() < 52 {
            section.tokens.push(Token::Terminator);
        } else {
            tags.push(tag(
                "Result",
                &history.tricks_won(c.declarer.side()).to_string(),
            ));
        }
        sections.push(section);
    }

    // Random tag order.
    let mut keyed: Vec<(u32, TagPair)> = tags
        .into_iter()
        .enumerate()
        .map(|(i, t)| (order[i % order.len()], t))
        .collect();
    keyed.sort_by_key(|(k, _)| *k);
    let tags = keyed.into_iter().map(|(_, t)| t).collect();

    let commentary = comments
        .into_iter()
        .map(|i| Comment {
            text: WORDS[i].replace('}', ")"),
            after_tag: None,
            line: 0,
        })
        .collect();
    Game {
        tags,
        sections,
        commentary,
    }
}

fn arb_game() -> impl Strategy<Value = Game> {
    (
        arb_deal(),
        arb_auction(),
        proptest::collection::vec(any::<Index>(), 0..60),
        arb_seat(),
        any::<bool>(),
        proptest::option::of(1..=99u16),
        proptest::collection::vec((0..OPTIONAL.len(), 0..WORDS.len()), 0..8),
        proptest::collection::vec((any::<Index>(), 0..4u8), 0..5),
        proptest::collection::vec(0..WORDS.len(), 0..3),
        0..6usize,
        proptest::collection::vec(any::<u32>(), 8..16),
    )
        .prop_map(|(d, a, p, f, l, b, o, an, c, v, ord)| {
            build_game(d, a, p, f, l, b, o, an, c, v, ord)
        })
}

/// What the strict reader must give back for the export of `game`.
fn normalize(game: &Game) -> Game {
    let value_of = |name: &str| match game.get(name) {
        Some(TagValue::Str(s)) => Some(s.trim().to_string()),
        _ => None,
    };
    let dealer: Option<Seat> = value_of("Dealer").and_then(|s| s.parse().ok());
    let canon = |name: &str, value: &TagValue| -> TagValue {
        match value {
            TagValue::Inherited => TagValue::Inherited,
            TagValue::Default(t) => TagValue::Default(t.clone()),
            TagValue::Str(s) => {
                let s = s.trim();
                if s == "#" {
                    return TagValue::Inherited;
                }
                if let Some(rest) = s.strip_prefix("##") {
                    return TagValue::Default(rest.to_string());
                }
                let c = match name {
                    "Deal" => {
                        let deal = deal_string::parse(s).unwrap().complete().unwrap();
                        let first = dealer.unwrap_or(first_seat(s));
                        deal_string::write(&deal, first)
                    }
                    "Vulnerable" => s.parse::<Vulnerability>().unwrap().to_string(),
                    "Dealer" | "Declarer" => s.parse::<Seat>().unwrap().to_string(),
                    _ => s.to_string(),
                };
                TagValue::Str(c)
            }
        }
    };
    let mut tags = Vec::new();
    for name in MTS {
        match game.tags.iter().find(|t| t.name == name) {
            Some(t) => tags.push(TagPair {
                name: name.to_string(),
                value: canon(name, &t.value),
                line: 0,
            }),
            None => tags.push(tag(name, "?")),
        }
    }
    let mut extra: Vec<&TagPair> = Vec::new();
    for t in &game.tags {
        if MTS.contains(&t.name.as_str()) || extra.iter().any(|e| e.name == t.name) {
            continue;
        }
        extra.push(t);
    }
    let mut sections: Vec<Section> = game.sections.clone();
    let mut entries: Vec<(String, Result<TagPair, Section>)> = extra
        .into_iter()
        .map(|t| {
            (
                t.name.clone(),
                Ok(TagPair {
                    name: t.name.clone(),
                    value: canon(&t.name, &t.value),
                    line: 0,
                }),
            )
        })
        .collect();
    for section in sections.drain(..) {
        entries.push((section.tag.clone(), Err(section)));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, entry) in entries {
        match entry {
            Ok(t) => tags.push(t),
            Err(s) => sections.push(normalize_section(s)),
        }
    }
    let commentary = game
        .commentary
        .iter()
        .map(|c| Comment {
            text: c.text.clone(),
            after_tag: c.after_tag,
            line: 0,
        })
        .collect();
    Game {
        tags,
        sections,
        commentary,
    }
}

fn first_seat(deal: &str) -> Seat {
    deal[..1].parse().unwrap()
}

fn normalize_section(mut section: Section) -> Section {
    let play = section.tag == "Play";
    if section.tag == "Auction" || play {
        section.arg = section.arg.parse::<Seat>().unwrap().to_string();
    }
    let mut out = Vec::new();
    let mut run: Vec<Token> = Vec::new();
    let flush = |run: &mut Vec<Token>, out: &mut Vec<Token>| {
        run.sort_by_key(|t| match t {
            Token::Nag(n) => (1u8, *n),
            _ => (0, 0),
        });
        out.append(run);
    };
    for token in section.tokens {
        match token {
            Token::NoteRef(_) | Token::Nag(_) => run.push(token),
            Token::Suffix(s) => {
                let base = match s.as_str() {
                    "!" => 1,
                    "?" => 2,
                    "!!" => 3,
                    "??" => 4,
                    "!?" => 5,
                    _ => 6,
                };
                run.push(Token::Nag(if play { base + 6 } else { base }));
            }
            Token::Continuation | Token::Raw(_) => {}
            other => {
                flush(&mut run, &mut out);
                out.push(other);
            }
        }
    }
    flush(&mut run, &mut out);
    section.tokens = out;
    section
}

fn strip_lines(mut game: Game) -> Game {
    for t in &mut game.tags {
        t.line = 0;
    }
    for c in &mut game.commentary {
        c.line = 0;
    }
    game
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1500))]

    #[test]
    fn export_round_trip(game in arb_game()) {
        let file = PbnFile { games: vec![game.clone()], directives: Vec::new() };
        let text = pbn::write(&file, WriteOptions { export: true });
        let parsed = match pbn::parse_strict(&text) {
            Ok(p) => p,
            Err(e) => {
                let line = text.lines().nth(e.line as usize - 1).unwrap_or("");
                prop_assert!(false, "{}: {:?}\n{}", e, line, text);
                unreachable!()
            }
        };
        prop_assert_eq!(&parsed.directives, &vec![Directive::Version(2, 1), Directive::Export]);
        prop_assert_eq!(parsed.games.len(), 1);
        prop_assert_eq!(strip_lines(parsed.games[0].clone()), normalize(&game), "\n{}", text);
        // Idempotent.
        prop_assert_eq!(pbn::write(&parsed, WriteOptions { export: true }), text.clone());
        // Same view.
        prop_assert_eq!(parsed.games[0].view(None), game.view(None));
        prop_assert!(game.view(None).is_ok());
        // The lenient reader agrees and only reports the `-` cells of an unfinished trick.
        let (lenient, warnings) = pbn::parse_lenient(text.as_bytes());
        prop_assert!(warnings.iter().all(|w| w.kind == WarningKind::Truncated), "{:?}", warnings);
        prop_assert_eq!(strip_lines(lenient.games[0].clone()), normalize(&game));
        // Verbatim writing and re-reading gives the same view as well.
        let verbatim = pbn::write(&file, WriteOptions::default());
        let (again, _) = pbn::parse_lenient(verbatim.as_bytes());
        prop_assert_eq!(again.games[0].view(None), game.view(None));
        // The auction of the view is the generated one.
        let view = game.view(None).unwrap();
        let expected_calls = game_calls(&game);
        prop_assert_eq!(view.auction.as_ref().map(Auction::calls), Some(expected_calls.as_slice()));
    }
}

fn game_calls(game: &Game) -> Vec<Call> {
    game.sections[0]
        .tokens
        .iter()
        .filter_map(|t| match t {
            Token::Call(c) => Some(*c),
            _ => None,
        })
        .collect()
}
