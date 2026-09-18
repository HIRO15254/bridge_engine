//! Hand-written PBN cases: escapes, inheritance, `AP`, `-`, comments, notes, tables,
//! encodings, the writer and strict mode.

use bridge_core::{Call, Seat, Side, Strain, Vulnerability};
use bridge_format::{
    PbnFile, TagValue, Token, Warning, WarningKind, WriteOptions,
    pbn::{self, Comment, Directive, Game, ViewError},
};

/// The PBN 2.1 `OptimumPlayTable` example without its table (LF line endings).
const GAME: &str = "[Event \"12th Cap Gemini World Top Tournament 1998\"]\n\
[Site \"The Hague\"]\n\
[Date \"1998.01.15\"]\n\
[Board \"16\"]\n\
[West \"Blakset\"]\n\
[North \"Sun\"]\n\
[East \"Christiansen\"]\n\
[South \"Wang\"]\n\
[Dealer \"W\"]\n\
[Vulnerable \"EW\"]\n\
[Deal \"W:AJ9754.J62.K62.6 32..A954.AQJ9875 KQ86.KQT843.QJ.K T.A975.T873.T432\"]\n\
[Scoring \"IMP\"]\n\
[Declarer \"N\"]\n\
[Contract \"5CX\"]\n\
[Result \"NS 11\"]\n\
[Auction \"W\"]\n\
2D 3C 4H 5C\n\
Pass Pass X Pass\n\
Pass Pass\n\
[Play \"E\"]\n\
SK ST S4 S2\n\
HK H5 H2 C7\n\
S6 C2 S5 S3\n\
H3 HA H6 D4\n\
*\n";

fn lenient(text: &str) -> (PbnFile, Vec<Warning>) {
    pbn::parse_lenient(text.as_bytes())
}

fn kinds(warnings: &[Warning]) -> Vec<WarningKind> {
    warnings.iter().map(|w| w.kind).collect()
}

fn text(game: &Game, name: &str) -> String {
    match game.get(name) {
        Some(TagValue::Str(s)) => s.clone(),
        other => panic!("{name}: {other:?}"),
    }
}

#[test]
fn reference_game_is_read_and_viewed() {
    let (file, warnings) = lenient(GAME);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(file.games.len(), 1);
    let game = &file.games[0];
    assert_eq!(game.tags.len(), 15);
    assert_eq!(game.tags[0].line, 1);
    assert_eq!(game.tags[14].line, 15);
    assert_eq!(game.sections.len(), 2);
    assert_eq!(game.sections[0].tokens.len(), 10);
    assert_eq!(game.sections[1].tokens.len(), 17);

    let view = game.view(None).unwrap();
    assert_eq!(view.board, Some(16));
    assert_eq!(view.dealer, Some(Seat::West));
    assert_eq!(view.vulnerable, Some(Vulnerability::EW));
    let deal = view.deal.unwrap().complete().unwrap();
    assert_eq!(deal.hand(Seat::North).to_string(), "32..A954.AQJ9875");
    let auction = view.auction.as_ref().unwrap();
    assert!(auction.is_complete());
    assert_eq!(auction.dealer(), Seat::West);
    let contract = view.contract.unwrap();
    assert_eq!(contract.to_string(), "5CX");
    assert_eq!(contract.declarer, Seat::North);
    assert_eq!(auction.contract(), Some(contract));
    assert_eq!(view.declarer, Some(Seat::North));
    assert_eq!(view.result, Some(11));
    let play = view.play.as_ref().unwrap();
    assert_eq!(play.leader(), Seat::East);
    assert_eq!(play.trump(), Strain::Clubs);
    assert_eq!(play.cards().len(), 16);
    // E wins the first trick, N ruffs the second, S wins the third and fourth.
    assert_eq!(play.trick_winner(0), Some(Seat::East));
    assert_eq!(play.trick_winner(1), Some(Seat::North));
    assert_eq!(play.trick_winner(2), Some(Seat::South));
    assert_eq!(play.trick_winner(3), Some(Seat::South));
    assert_eq!(play.tricks_won(Side::NS), 3);
    assert_eq!(play.cards()[8].to_string(), "S3"); // North led the third trick
    assert_eq!(view.dd_table, None);

    // Verbatim writing reproduces the input exactly.
    assert_eq!(pbn::write(&file, WriteOptions::default()), GAME);
}

#[test]
fn escapes_several_tags_per_line_and_directives() {
    let (file, warnings) = lenient(
        "% PBN 2.1\n%BoardsPerPage 4\n[Event \"a \\\"quoted\\\" \\\\ back\"] [Site \"x\"]\n\
         [Spec \"Result\\2R\"]\n[Multi \"one\n two\"]\n% EXPORT\n",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        file.directives,
        vec![
            Directive::Version(2, 1),
            Directive::Other("BoardsPerPage 4".to_string()),
            Directive::Export
        ]
    );
    let game = &file.games[0];
    assert_eq!(text(game, "Event"), "a \"quoted\" \\ back");
    assert_eq!(text(game, "Site"), "x");
    assert_eq!(game.tags[1].line, 3);
    // A backslash before an ordinary character is literal (PBN table specs do this).
    assert_eq!(text(game, "Spec"), "Result\\2R");
    // Import format lets a string span lines.
    assert_eq!(text(game, "Multi"), "one\n two");
    assert_eq!(game.tags.len(), 4);

    // The writer escapes both characters again.
    let written = pbn::write(&file, WriteOptions::default());
    assert!(written.contains("[Event \"a \\\"quoted\\\" \\\\ back\"]"));
    assert!(written.contains("[Spec \"Result\\\\2R\"]"));
    let (again, _) = pbn::parse_lenient(written.as_bytes());
    assert_eq!(text(&again.games[0], "Spec"), "Result\\2R");
}

#[test]
fn inheritance() {
    let (file, warnings) = lenient(
        "[Event \"E1\"]\n[Board \"1\"]\n[Dealer \"N\"]\n[Vulnerable \"None\"]\n\n\
         [Event \"#\"]\n[Board \"#\"]\n[Dealer \"#\"]\n[Vulnerable \"##NS\"]\n",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(file.games.len(), 2);
    let second = &file.games[1];
    assert_eq!(second.get("Event"), Some(&TagValue::Inherited));
    assert_eq!(second.get("Board"), Some(&TagValue::Inherited));
    assert_eq!(
        second.get("Vulnerable"),
        Some(&TagValue::Default("NS".to_string()))
    );
    assert_eq!(
        second.view(None),
        Err(ViewError::NothingToInherit("Board".to_string()))
    );
    let first = file.games[0].view(None).unwrap();
    let view = second.view(Some(&first)).unwrap();
    assert_eq!(view.board, Some(1));
    assert_eq!(view.dealer, Some(Seat::North));
    assert_eq!(view.vulnerable, Some(Vulnerability::NS));
}

#[test]
fn all_pass_notes_suffixes_and_nags() {
    let (file, warnings) = lenient(
        "[Dealer \"N\"]\n[Note \"9:orphan\"]\n[Auction \"N\"]\n1S =1= Pass 2S! $3\nAP\n\
         [Note \"1:five spades\"]\n[Note \"2:bad\"]\n",
    );
    assert_eq!(kinds(&warnings), vec![WarningKind::NoteReference]);
    let game = &file.games[0];
    // The orphan note stays an ordinary tag.
    assert_eq!(text(game, "Note"), "9:orphan");
    let section = &game.sections[0];
    let call = |s: &str| Token::Call(s.parse().unwrap());
    assert_eq!(
        section.tokens,
        vec![
            call("1S"),
            Token::NoteRef(1),
            call("Pass"),
            call("2S"),
            Token::Suffix("!".to_string()),
            Token::Nag(3),
            call("Pass"),
            call("Pass"),
            call("Pass"),
        ]
    );
    assert_eq!(
        section.notes,
        vec![(1, "five spades".to_string()), (2, "bad".to_string())]
    );
    let view = game.view(None).unwrap();
    let auction = view.auction.unwrap();
    assert!(auction.is_complete());
    let contract = auction.contract().unwrap();
    assert_eq!(contract.to_string(), "2S");
    assert_eq!(contract.declarer, Seat::North);
    // Without a Contract tag the view has no contract of its own.
    assert_eq!(view.contract, None);
}

#[test]
fn unknown_hands_and_cards() {
    let (file, warnings) = lenient(
        "[Deal \"N:AJ9754.J62.K62.6 - KQ86.KQT843.QJ.K -\"]\n\n\
         [Deal \"N:AJ9754.J62.K62.6 32..A954.AQJ987 KQ86.KQT843.QJ.K T.A975.T873.T432\"]\n",
    );
    assert_eq!(kinds(&warnings), vec![WarningKind::BadDeal]);
    assert_eq!(warnings[0].game, 1);
    assert_eq!(warnings[0].line, 3);
    let deal = file.games[0].view(None).unwrap().deal.unwrap();
    assert!(deal.hands[Seat::North.index() as usize].is_some());
    assert!(deal.hands[Seat::East.index() as usize].is_none());
    assert!(deal.hands[Seat::South.index() as usize].is_some());
    assert!(deal.hands[Seat::West.index() as usize].is_none());
    assert!(deal.complete().is_none());
    // The twelve-card hand is dropped, the others kept.
    let deal = file.games[1].view(None).unwrap().deal.unwrap();
    assert!(deal.hands[Seat::East.index() as usize].is_none());
    assert!(deal.hands[Seat::West.index() as usize].is_some());

    // `-` in a play row: the trick is played up to the unknown card and the play stops.
    let partial = GAME.replace("H3 HA H6 D4", "- HA H6 D4");
    let (file, warnings) = lenient(&partial);
    assert_eq!(kinds(&warnings), vec![WarningKind::Truncated]);
    assert_eq!(warnings[0].line, 20);
    let view = file.games[0].view(None).unwrap();
    let play = view.play.unwrap();
    assert_eq!(play.cards().len(), 15);
    assert_eq!(play.cards()[12].to_string(), "HA");
    assert_eq!(play.next_to_play(), Seat::East);

    // An unknown call leaves the auction uninterpreted; the deal is still there.
    let unknown = GAME.replace("Pass Pass X Pass", "Pass - X Pass");
    let (file, warnings) = lenient(&unknown);
    assert!(kinds(&warnings).contains(&WarningKind::Truncated));
    let view = file.games[0].view(None).unwrap();
    assert_eq!(view.auction, None);
    assert!(view.deal.is_some());
    assert!(view.contract.is_some());
    assert!(view.play.is_some());

    // An unrecognised token is kept raw with a warning.
    let junk = GAME.replace("2D 3C 4H 5C", "2D 3C 4H 5C 1N");
    let (file, warnings) = lenient(&junk);
    assert!(kinds(&warnings).contains(&WarningKind::MalformedToken));
    assert_eq!(
        file.games[0].sections[0].tokens[4],
        Token::Raw("1N".to_string())
    );
}

#[test]
fn comments_and_game_separation() {
    let (file, warnings) = lenient(
        "{leading\ncomment}\n[Event \"x\"]\n{after\nevent} ; rest\n[Site \"y\"]\n\n\n\
         {control 2}\n[Event \"z\"]\n\n{trailing}\n",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(file.games.len(), 2);
    let first = &file.games[0];
    assert_eq!(
        first.commentary,
        vec![
            Comment {
                text: "leading\ncomment".to_string(),
                after_tag: None,
                line: 1
            },
            Comment {
                text: "after\nevent".to_string(),
                after_tag: Some(0),
                line: 4
            },
            Comment {
                text: " rest".to_string(),
                after_tag: Some(0),
                line: 5
            },
        ]
    );
    assert_eq!(first.tags[1].line, 6);
    let second = &file.games[1];
    assert_eq!(text(second, "Event"), "z");
    assert_eq!(second.tags[0].line, 10);
    assert_eq!(second.commentary.len(), 2);
    assert_eq!(second.commentary[0].after_tag, None);
    assert_eq!(second.commentary[1].text, "trailing");
    assert_eq!(second.commentary[1].after_tag, Some(0));

    // Blank lines inside a brace comment do not separate games.
    let (file, _) = lenient("[Event \"a\"]\n{one\n\ntwo}\n[Site \"b\"]\n");
    assert_eq!(file.games.len(), 1);
    assert_eq!(file.games[0].tags.len(), 2);

    // An unterminated brace comment swallows the rest with a warning.
    let (file, warnings) = lenient("[Event \"a\"]\n{never closed\n[Site \"b\"]\n");
    assert_eq!(kinds(&warnings), vec![WarningKind::Truncated]);
    assert_eq!(file.games[0].tags.len(), 1);
}

#[test]
fn notes_are_numbered_per_section() {
    let with_notes = GAME
        .replace(
            "Pass Pass\n[Play",
            "Pass Pass\n[Note \"1:auction note\"]\n[Play",
        )
        .replace("*\n", "*\n[Note \"1:play note\"]\n");
    let (file, warnings) = lenient(&with_notes);
    assert!(warnings.is_empty(), "{warnings:?}");
    let game = &file.games[0];
    assert_eq!(
        game.sections[0].notes,
        vec![(1, "auction note".to_string())]
    );
    assert_eq!(game.sections[1].notes, vec![(1, "play note".to_string())]);
    assert!(game.get("Note").is_none());
}

const TABLE: &str = "[Board \"1\"]\n[Dealer \"N\"]\n[Vulnerable \"-\"]\n\
[Deal \"N:4.KJ32.842.AQ743 JT987.Q876.AK5.2 AK532.T.JT6.T985 Q6.A954.Q973.KJ6\"]\n\
[OptimumResultTable \"Declarer;Denomination\\2R;Result\\2R\"]\n\
N  C  9\nN  D  6\nN  H  5\nN  S  6\nN NT  6\n\
E  C  3\nE  D  7\nE  H  8\nE  S  7\nE NT  6\n\
S  C  9\nS  D  6\nS  H  5\nS  S  6\nS NT  6\n\
W  C  3\nW  D  7\nW  H  8\nW  S  7\nW NT  7\n";

#[test]
fn table_section() {
    let (file, warnings) = lenient(TABLE);
    assert!(warnings.is_empty(), "{warnings:?}");
    let game = &file.games[0];
    let section = &game.sections[0];
    assert_eq!(section.tag, "OptimumResultTable");
    assert_eq!(section.arg, "Declarer;Denomination\\2R;Result\\2R");
    assert_eq!(section.tokens.len(), 20);
    assert_eq!(section.tokens[0], Token::Raw("N  C  9".to_string()));
    let view = game.view(None).unwrap();
    assert_eq!(view.vulnerable, Some(Vulnerability::None));
    let table = view.dd_table.unwrap();
    assert_eq!(table.tricks(Strain::Clubs, Seat::North), 9);
    assert_eq!(table.tricks(Strain::NoTrump, Seat::West), 7);
    assert_eq!(table.tricks(Strain::Hearts, Seat::East), 8);

    // The writer keeps the column spec with a bare backslash and the rows verbatim.
    let written = pbn::write(&file, WriteOptions::default());
    assert_eq!(written, TABLE);
    let export = pbn::write(&file, WriteOptions { export: true });
    assert!(
        export.contains(
            "[OptimumResultTable \"Declarer;Denomination\\2R;Result\\2R\"]\r\nN  C  9\r\n"
        )
    );
    let strict = pbn::parse_strict(&export).unwrap();
    assert_eq!(strict.games[0].view(None).unwrap().dd_table, Some(table));

    // Nineteen rows: an error; unknown results: no table.
    let short = TABLE.replace("W NT  7\n", "");
    let (file, warnings) = lenient(&short);
    assert_eq!(kinds(&warnings), vec![WarningKind::Other]);
    assert!(matches!(
        file.games[0].view(None),
        Err(ViewError::BadTag { .. })
    ));
    let unknown = TABLE.replace("W NT  7\n", "W NT  ?\n");
    let (file, _) = lenient(&unknown);
    assert_eq!(file.games[0].view(None).unwrap().dd_table, None);
}

#[test]
fn crlf_latin1_and_bom() {
    let mut bytes =
        b"\xEF\xBB\xBF[Event \"x\"]\r\n[Site \"Caf\xE9\"]\r\n\r\n[Event \"y\"]\r\n".to_vec();
    let (file, warnings) = pbn::parse_lenient(&bytes);
    assert_eq!(kinds(&warnings), vec![WarningKind::Encoding]);
    assert_eq!(file.games.len(), 2);
    assert_eq!(text(&file.games[0], "Site"), "Café");
    assert_eq!(file.games[0].tags[1].line, 2);
    assert_eq!(file.games[1].tags[0].line, 4);
    // Valid UTF-8 needs no warning.
    bytes = "\u{FEFF}[Event \"x\"]\r\n[Site \"Café\"]\r\n"
        .as_bytes()
        .to_vec();
    let (file, warnings) = pbn::parse_lenient(&bytes);
    assert!(warnings.is_empty());
    assert_eq!(text(&file.games[0], "Site"), "Café");
    // A lone CR is a line break too.
    let (file, _) = lenient("[Event \"x\"]\r[Site \"y\"]\r");
    assert_eq!(file.games[0].tags[1].line, 2);
}

#[test]
fn tag_value_conventions() {
    let (file, _) = lenient(
        "[Vulnerable \"Love\"]\n[Declarer \"^S\"]\n[Contract \"4sx\"]\n[Result \"^NS 11\"]\n\n\
         [Vulnerable \"Both\"]\n[Declarer \"W\"]\n[Contract \"3NT\"]\n[Result \"NS 4\"]\n\n\
         [Vulnerable \"?\"]\n[Contract \"Pass\"]\n[Result \"EW 5\"]\n[Declarer \"N\"]\n\n\
         [Contract \"5X\"]\n\n[Board \"x\"]\n",
    );
    let v = file.games[0].view(None).unwrap();
    assert_eq!(v.vulnerable, Some(Vulnerability::None));
    assert_eq!(v.declarer, Some(Seat::South));
    assert_eq!(v.contract.unwrap().to_string(), "4SX");
    assert_eq!(v.contract.unwrap().declarer, Seat::South);
    assert_eq!(v.result, Some(11));
    let v = file.games[1].view(None).unwrap();
    assert_eq!(v.vulnerable, Some(Vulnerability::Both));
    assert_eq!(v.result, Some(9));
    let v = file.games[2].view(None).unwrap();
    assert_eq!(v.vulnerable, None);
    assert_eq!(v.contract, None);
    assert_eq!(v.result, Some(8));
    assert!(
        matches!(file.games[3].view(None), Err(ViewError::BadTag { tag, .. }) if tag == "Contract")
    );
    assert!(
        matches!(file.games[4].view(None), Err(ViewError::BadTag { tag, .. }) if tag == "Board")
    );
}

#[test]
fn lenient_warnings_and_recovery() {
    let (file, warnings) = lenient(
        "[Event \"a\"]\n[Event \"b\"]\nstray text\n[Site \"unterminated\n[Date \"d\"]\n[Bad x]\n[Round \"1\"\n",
    );
    assert_eq!(
        kinds(&warnings),
        vec![
            WarningKind::Other,        // duplicate Event
            WarningKind::MalformedTag, // stray text
            WarningKind::MalformedTag, // unterminated string
            WarningKind::MalformedTag, // missing ] after it
            WarningKind::MalformedTag, // [Bad x]
            WarningKind::MalformedTag, // missing ]
        ]
    );
    let game = &file.games[0];
    assert_eq!(text(game, "Event"), "a");
    assert_eq!(game.tags.len(), 5);
    assert_eq!(text(game, "Site"), "unterminated");
    assert_eq!(text(game, "Date"), "d");
    assert_eq!(text(game, "Round"), "1");
    assert_eq!(warnings[1].line, 3);
    assert_eq!(warnings[4].line, 6);

    // An illegal call is an error of the view and a warning of the parser; an extra pass
    // after the end of the auction is only a warning.
    let illegal = GAME.replace("2D 3C 4H 5C", "2D 3C 2H 5C");
    let (file, warnings) = lenient(&illegal);
    assert_eq!(kinds(&warnings), vec![WarningKind::BadAuction]);
    assert_eq!(warnings[0].line, 16);
    assert!(matches!(
        file.games[0].view(None),
        Err(ViewError::Auction(_))
    ));
    let extra = GAME.replace("Pass Pass\n[Play", "Pass Pass Pass\n[Play");
    let (file, warnings) = lenient(&extra);
    assert_eq!(kinds(&warnings), vec![WarningKind::BadAuction]);
    assert_eq!(file.games[0].view(None).unwrap().auction.unwrap().len(), 10);

    // A revoke is an error of the view.
    let revoke = GAME.replace("HK H5 H2 C7", "HK H5 C7 H2");
    let (file, warnings) = lenient(&revoke);
    assert_eq!(kinds(&warnings), vec![WarningKind::BadPlay]);
    assert!(matches!(file.games[0].view(None), Err(ViewError::Play(_))));
    // The play must start with the opening leader.
    let wrong_leader = GAME.replace("[Play \"E\"]", "[Play \"W\"]");
    let (file, _) = lenient(&wrong_leader);
    assert!(
        matches!(file.games[0].view(None), Err(ViewError::BadTag { tag, .. }) if tag == "Play")
    );

    // Empty input and whitespace-only input give no games and no panic.
    assert!(lenient("").0.games.is_empty());
    assert!(lenient("\n\n  \n").0.games.is_empty());
}

#[test]
fn export_writer() {
    let (file, _) = lenient(
        "{intro}\n[Score \"NS 590\"]\n[Contract \"4sx\"]\n[Dealer \"e\"]\n[Vulnerable \"Love\"]\n\
         [Declarer \"n\"]\n[Result \"NS 10\"]\n\
         [Deal \"S:k932.kj75.aq65.t 764.QT83.K87.Q43 QJT85.9.J.J87652 A.A642.T9432.AK9\"]\n\
         [Auction \"E\"]\nPass 2S X! 4S ; end of line comment\nPass Pass X $2 =1= Pass\nPass Pass +\n[Note \"1:\\\"penalty\\\"\"]\n\
         [Play \"E\"]\nCA CT C3 C6\nDT DA D8 DJ\n*\n",
    );
    let export = pbn::write(&file, WriteOptions { export: true });
    let expected = "% PBN 2.1\r\n% EXPORT\r\n{intro}\r\n\
[Event \"?\"]\r\n[Site \"?\"]\r\n[Date \"?\"]\r\n[Board \"?\"]\r\n[West \"?\"]\r\n[North \"?\"]\r\n\
[East \"?\"]\r\n[South \"?\"]\r\n[Dealer \"E\"]\r\n[Vulnerable \"None\"]\r\n\
[Deal \"E:A.A642.T9432.AK9 K932.KJ75.AQ65.T 764.QT83.K87.Q43 QJT85.9.J.J87652\"]\r\n\
{ end of line comment}\r\n\
[Scoring \"?\"]\r\n[Declarer \"N\"]\r\n[Contract \"4SX\"]\r\n[Result \"10\"]\r\n\
[Auction \"E\"]\r\nPass 2S X $1 4S\r\nPass Pass X =1= $2 AP\r\n[Note \"1:\\\"penalty\\\"\"]\r\n\
[Play \"E\"]\r\nCA CT C3 C6\r\nDT DA D8 DJ\r\n*\r\n[Score \"NS 590\"]\r\n";
    assert_eq!(export, expected);
    let strict = pbn::parse_strict(&export).unwrap();
    assert_eq!(
        strict.directives,
        vec![Directive::Version(2, 1), Directive::Export]
    );
    let game = &strict.games[0];
    assert_eq!(game.tags.len(), 16);
    assert_eq!(game.commentary.len(), 2);
    assert_eq!(game.commentary[0].after_tag, None);
    assert_eq!(game.commentary[1].after_tag, Some(10)); // after Deal
    let view = game.view(None).unwrap();
    assert_eq!(view.contract.unwrap().to_string(), "4SX");
    assert_eq!(view.auction.as_ref().unwrap().len(), 10);
    assert_eq!(view.play.as_ref().unwrap().cards().len(), 8);
    assert_eq!(view.result, Some(10));
    // Writing again is stable.
    assert_eq!(pbn::write(&strict, WriteOptions { export: true }), export);
    // The original and the export view the same.
    let original = file.games[0].view(None).unwrap();
    assert_eq!(original, view);

    // A passed-out auction keeps its four passes, and `#` values survive except for Deal.
    let (file, _) = lenient("[Event \"#\"]\n[Deal \"#\"]\n[Auction \"N\"]\nPass Pass Pass Pass\n");
    let export = pbn::write(&file, WriteOptions { export: true });
    assert!(export.contains("[Event \"#\"]\r\n"));
    assert!(export.contains("[Deal \"?\"]\r\n"));
    assert!(export.contains("[Auction \"N\"]\r\nPass Pass Pass Pass\r\n"));
}

/// A minimal export-format game around `middle` (auction lines and more).
fn strict_text(middle: &str) -> String {
    format!(
        "[Event \"e\"]\r\n[Site \"s\"]\r\n[Date \"d\"]\r\n[Board \"1\"]\r\n[West \"w\"]\r\n[North \"n\"]\r\n\
         [East \"e\"]\r\n[South \"s\"]\r\n[Dealer \"N\"]\r\n[Vulnerable \"None\"]\r\n[Deal \"?\"]\r\n\
         [Scoring \"IMP\"]\r\n[Declarer \"N\"]\r\n[Contract \"1S\"]\r\n[Result \"7\"]\r\n[Auction \"N\"]\r\n{middle}"
    )
}

#[test]
fn strict_mode() {
    let ok = strict_text("1S Pass Pass Pass\r\n[Note \"1:x\"]\r\n");
    assert!(pbn::parse_strict(&ok).is_ok());
    assert!(pbn::parse_strict(&ok.replace("\r\n", "\n")).is_ok());
    let bad: &[(&str, u32, &str)] = &[
        ("1S pass Pass Pass\r\n", 17, "upper case"),
        ("1S! Pass Pass Pass\r\n", 17, "NAG"),
        ("1S Pass Pass +\r\n", 17, "import"),
        ("1N Pass Pass Pass\r\n", 17, "unrecognised"),
        ("1S\tPass Pass Pass\r\n", 17, "tab"),
        (
            "1S Pass Pass Pass\r\n[Note \"1:x\"] [Room \"Open\"]\r\n",
            18,
            "follow",
        ),
        ("1S Pass Pass Pass\r\n [Room \"Open\"]\r\n", 18, "column 1"),
        (
            "1S Pass Pass Pass\r\n[Room  \"Open\"]\r\n",
            18,
            "single spaces",
        ),
        ("1S Pass Pass Pass\r\n[Room \"Open\r\n", 18, "unterminated"),
        ("1S Pass Pass Pass\r\n{never closed\r\n", 18, "unterminated"),
        (
            "1S Pass Pass Pass\r\n[Room \"Open\"]\r\n[Room \"Closed\"]\r\n",
            19,
            "duplicate",
        ),
        ("1S Pass Pass Pass\r\n[Event \"x\"]\r\n", 18, "duplicate"),
        ("1S Pass Pass Pass\r\n\r\n[Note \"1:x\"]\r\n", 19, "Note"),
    ];
    for (middle, line, needle) in bad {
        let err = pbn::parse_strict(&strict_text(middle)).unwrap_err();
        assert_eq!(err.line, *line, "{middle:?}: {err}");
        assert!(err.message.contains(needle), "{middle:?}: {err}");
    }
    // Mandatory tags must come first and in order.
    let swapped = ok.replace(
        "[Site \"s\"]\r\n[Date \"d\"]",
        "[Date \"d\"]\r\n[Site \"s\"]",
    );
    let err = pbn::parse_strict(&swapped).unwrap_err();
    assert_eq!(err.line, 2, "{err}");
    let missing = ok.replace("[Result \"7\"]\r\n", "");
    let err = pbn::parse_strict(&missing).unwrap_err();
    assert!(err.message.contains("Result"));
    let err =
        pbn::parse_strict("[Event \"e\"]\r\n[Auction \"N\"]\r\n1S Pass Pass Pass\r\n").unwrap_err();
    assert_eq!(err.line, 1, "{err}");
    assert!(err.message.contains("Site"), "{err}");
    let ap = strict_text("1S AP\r\n");
    let file = pbn::parse_strict(&ap).unwrap();
    assert_eq!(file.games[0].sections[0].tokens.len(), 4);
    assert!(pbn::parse_strict("").unwrap().games.is_empty());
    let (_, warnings) = lenient(&ok);
    assert!(warnings.is_empty());
    // The two calls agree on a lenient read of the strict text.
    let call = |s: &str| Token::Call(s.parse::<Call>().unwrap());
    assert_eq!(
        pbn::parse_strict(&ok).unwrap().games[0].sections[0].tokens,
        vec![call("1S"), call("Pass"), call("Pass"), call("Pass")]
    );
}
