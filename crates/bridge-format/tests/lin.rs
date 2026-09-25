//! Hand-written LIN cases.

use bridge_core::{Call, Seat, Vulnerability};
use bridge_format::{WarningKind, lin};

const VUGRAPH: &str = "vg|64TH PULA BRIDGE FESTIVAL - OT -,ROUND 12_12,I,25,32,Zakrzes,0,Tosca,0|\r\n\
rs|3NW+1,3NW+1|\r\n\
pn|ZAKRZEWSKI,J JANSMA,BURAS,LINDEMAN,VAN LANKVE,NARKIEWICZ,VAN DEN BO,MARCINOWSK|pg||\r\n\
qx|o25|st||md|3S5H762DAT8CAJT864,SKJ73HQ853D43CKQ7,SQT98642HT9DQ6C93,SAHAKJ4DKJ9752C52|sv|e|nt|vugraphzpg: Welcome back|pg||\r\n\
mb|3S|nt|vugraphzpg: chat|pg||\r\n\
mb|d|mb|p|mb|3N|an|to play|mb|p|mb|p|mb|p|pc|hT|pc|h4|pc|h2|pc|hQ|pg||\r\n\
pc|d4|pc|d6|pc|d9|pc|dT|pg||\r\n\
pc|cJ|pc|cQ|pc|c3|pc|c2|pg||\r\n\
pc|d3|pc|dQ|pc|dK|mc|10|pg||\r\n\
qx|c25|st||md|3S5H762DAT8CAJT864,SKJ73HQ853D43CKQ7,SQT98642HT9DQ6C93,SAHAKJ4DKJ9752C52|sv|e|mb|3S|mb|d|mb|p|mb|3N|mb|p|mb|p|mb|p|pc|sT|pc|sA|pc|s5|pc|s3|pg||\r\n\
pc|dK|pc|d8|pc|d3|pc|d6|pg||\r\n\
pc|d2|pc|dT|pc|d4|pc|dQ|pg||\r\n\
pc|c9|pc|c2|pc|cT|pc|cQ|pg||\r\n\
pc|h3|pc|h9|pc|hA|pc|h2|pg||\r\n\
pc|d9|pc|dA|mc|10|pg||\r\n";

#[test]
fn vugraph_file_with_two_rooms() {
    let (boards, warnings) = lin::parse_lenient(VUGRAPH.as_bytes());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(boards.len(), 2);
    let open = &boards[0];
    assert_eq!(open.id.as_deref(), Some("o25"));
    assert_eq!(open.names.len(), 8);
    assert_eq!(open.board_no, Some(25));
    assert_eq!(open.dealer, Some(Seat::North));
    assert_eq!(open.vul, Some(Vulnerability::EW));
    let deal = open.deal.unwrap().complete().unwrap();
    assert_eq!(deal.hand(Seat::South).to_string(), "5.762.AT8.AJT864");
    assert_eq!(deal.hand(Seat::East).to_string(), "A.AKJ4.KJ9752.52");
    assert_eq!(open.calls.len(), 7);
    assert_eq!(open.calls[0], ("3S".parse().unwrap(), false, None));
    assert_eq!(open.calls[1].0, Call::Double);
    assert_eq!(open.calls[3].2.as_deref(), Some("to play"));
    assert_eq!(open.plays.len(), 15);
    assert_eq!(open.plays[0], "HT".parse().unwrap());
    assert_eq!(open.claim, Some(10));
    assert!(open.raw.iter().any(|(t, _)| t == "vg"));
    assert!(
        open.raw
            .iter()
            .any(|(t, v)| t == "nt" && v.contains("chat"))
    );
    // Six page breaks of the board plus the one of the file header.
    assert_eq!(open.raw.iter().filter(|(t, _)| t == "pg").count(), 7);

    let closed = &boards[1];
    assert_eq!(closed.id.as_deref(), Some("c25"));
    assert_eq!(closed.names, open.names);
    assert!(closed.raw.iter().all(|(t, _)| t != "vg"));

    // to_game: tags, sections and a view that agrees with the record.
    let game = open.to_game();
    let text = |name: &str| match game.get(name) {
        Some(bridge_format::TagValue::Str(s)) => s.clone(),
        other => panic!("{name}: {other:?}"),
    };
    assert_eq!(text("Event"), "64TH PULA BRIDGE FESTIVAL - OT -");
    assert_eq!(text("Board"), "25");
    assert_eq!(text("Dealer"), "N");
    assert_eq!(text("Vulnerable"), "EW");
    assert_eq!(text("Room"), "Open");
    assert_eq!(text("South"), "ZAKRZEWSKI");
    assert_eq!(text("West"), "J JANSMA");
    assert_eq!(text("North"), "BURAS");
    assert_eq!(text("East"), "LINDEMAN");
    assert_eq!(text("Contract"), "3NT");
    assert_eq!(text("Declarer"), "W");
    assert_eq!(text("Result"), "10");
    assert_eq!(
        text("Deal"),
        "N:QT98642.T9.Q6.93 A.AKJ4.KJ9752.52 5.762.AT8.AJT864 KJ73.Q853.43.KQ7"
    );
    let auction = game.sections.iter().find(|s| s.tag == "Auction").unwrap();
    assert_eq!(auction.arg, "N");
    assert_eq!(auction.notes, vec![(1, "to play".to_string())]);
    let play = game.sections.iter().find(|s| s.tag == "Play").unwrap();
    assert_eq!(play.arg, "N");
    assert_eq!(play.tokens.len(), 4 * 4 + 1); // four rows (one partial) and `*`

    let view = game.view(None).unwrap();
    let contract = view.contract.unwrap();
    assert_eq!(contract.to_string(), "3NT");
    assert_eq!(contract.declarer, Seat::West);
    assert_eq!(view.auction.unwrap().contract(), Some(contract));
    assert_eq!(view.play.unwrap().cards().len(), 15);
    assert_eq!(view.result, Some(10));

    let closed_game = boards[1].to_game();
    assert_eq!(
        closed_game.get("South"),
        Some(&bridge_format::TagValue::Str("VAN LANKVE".to_string()))
    );
    assert_eq!(
        closed_game.get("Room"),
        Some(&bridge_format::TagValue::Str("Closed".to_string()))
    );
}

#[test]
fn omitted_fourth_hand_is_computed() {
    let (boards, warnings) = lin::parse_lenient(
        b"pn|a,b,c,d|md|1SAKQJT98765432,HAKQJT98765432,DAKQJT98765432,|sv|0|mb|1n|mb|2C!|an|Precision|mb|P|mb|p|mb|p|pc|DT|",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(boards.len(), 1);
    let board = &boards[0];
    assert_eq!(board.id, None);
    assert_eq!(board.dealer, Some(Seat::South));
    assert_eq!(board.vul, Some(Vulnerability::None));
    let deal = board.deal.unwrap().complete().unwrap();
    assert_eq!(deal.hand(Seat::East).to_string(), "...AKQJT98765432");
    assert_eq!(board.calls[0].0, "1NT".parse().unwrap());
    assert_eq!(
        board.calls[1],
        ("2C".parse().unwrap(), true, Some("Precision".to_string()))
    );
    assert_eq!(board.plays, vec!["DT".parse().unwrap()]);
    let game = board.to_game();
    let view = game.view(None).unwrap();
    assert_eq!(view.contract.unwrap().to_string(), "2C");
    assert_eq!(view.play.unwrap().cards().len(), 1);
}

#[test]
fn bad_hands_and_fragments_become_warnings() {
    // Twelve spades: the hand is dropped, not repaired, and the deal stays partial.
    let (boards, warnings) =
        lin::parse_lenient(b"md|2SAKQJT9876543,HAKQJT98765432,DAKQJT98765432,CAKQJT98765432|");
    assert_eq!(boards.len(), 1);
    let deal = boards[0].deal.unwrap();
    assert!(deal.hands[Seat::South.index() as usize].is_none());
    assert!(deal.hands[Seat::West.index() as usize].is_some());
    assert!(deal.complete().is_none());
    assert_eq!(boards[0].dealer, Some(Seat::West));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].kind, WarningKind::BadDeal);

    // Unknown tags are kept raw with one warning per name per board; junk is skipped.
    let (boards, warnings) = lin::parse_lenient(
        b"qx|o1|zz|x|zz|y|garbage without bar|pc|xx|mb|8C|sv|q|mc|14|an|late|\nqx|o2|zz|z|ah|Board 2|dangling",
    );
    assert_eq!(boards.len(), 2);
    assert_eq!(
        boards[0].raw,
        vec![
            ("zz".to_string(), "x".to_string()),
            ("zz".to_string(), "y".to_string())
        ]
    );
    let kinds: Vec<WarningKind> = warnings.iter().map(|w| w.kind).collect();
    assert_eq!(
        kinds,
        vec![
            WarningKind::UnknownTag,
            WarningKind::MalformedToken, // garbage fragment
            WarningKind::MalformedToken, // pc
            WarningKind::MalformedToken, // mb
            WarningKind::MalformedToken, // sv
            WarningKind::MalformedToken, // mc
            WarningKind::MalformedToken, // an
            WarningKind::UnknownTag,     // zz again in board 2
            WarningKind::MalformedToken, // dangling
        ]
    );
    assert_eq!(warnings[7].game, 1);
    assert_eq!(warnings[7].line, 2);
    assert_eq!(boards[1].board_no, Some(2));

    // Board number gives dealer and vulnerability when the record has none.
    let game = boards[1].to_game();
    let view = game.view(None).unwrap();
    assert_eq!(view.board, Some(2));
    assert_eq!(view.dealer, Some(Seat::East));
    assert_eq!(view.vulnerable, Some(Vulnerability::NS));

    // Empty input: nothing.
    assert!(lin::parse_lenient(b"").0.is_empty());
}
