//! The lenient readers never panic: random bytes, mutated and truncated inputs.

mod common;

use bridge_format::{WriteOptions, lin, pbn};
use proptest::prelude::*;
use proptest::sample::Index;

/// The PBN 2.1 `OptimumPlayTable` example (CRLF).
const SAMPLE: &str = "[Event \"12th Cap Gemini World Top Tournament 1998\"]\r\n\
[Site \"Hotel Des Indes - The Hague, Netherlands, NLD\"]\r\n[Date \"1998.01.15\"]\r\n[Round \"2\"]\r\n\
[Board \"16\"]\r\n[West \"Blakset\"]\r\n[North \"Sun\"]\r\n[East \"Christiansen\"]\r\n[South \"Wang\"]\r\n\
[Dealer \"W\"]\r\n[Vulnerable \"EW\"]\r\n\
[Deal \"W:AJ9754.J62.K62.6 32..A954.AQJ9875 KQ86.KQT843.QJ.K T.A975.T873.T432\"]\r\n\
[Declarer \"N\"]\r\n[Contract \"5CX\"]\r\n[Auction \"W\"]\r\n2D 3C 4H 5C \r\nPass Pass X Pass \r\nPass Pass   \r\n\
[Play \"E\"]\r\nSK ST S4 S2 \r\nHK H5 H2 C7 \r\nS6 C2 S5 S3 \r\nH3 HA H6 D4 \r\n*\r\n[Result \"NS 11\"]\r\n\
[Score \"NS +550\"]\r\n[ScoreIMP \"NS +10\"]\r\n[OptimumPlayTable \"S\\8R;H\\8R;D\\8R;C\\8R\"]\r\n\
KQ86    -       QJ      K\r\nT       -       -       -\r\nAJ9754  -       -       -\r\n32      -       -       -\r\n\
{ a comment spanning\r\n two lines }\r\n\r\n\
[Event \"#\"]\r\n[Board \"17\"]\r\n[Deal \"N:- - - -\"]\r\n[Auction \"N\"]\r\n1NT =1= Pass 3NT $1\r\nAP\r\n[Note \"1:15-17\"]\r\n";

const LIN_SAMPLE: &str = "vg|Title,Session,I,1,2,A,0,B,0|pn|a,b,c,d,e,f,g,h|pg||\r\n\
qx|o1|st||md|3SA87HQT65DJ97C932,S952H843DAQ8CT764,SQJT6HK7DKT32CAKQ,|sv|o|mb|1C|mb|p|mb|1D!|an|x|mb|p|mb|3N|mb|p|mb|p|mb|p|pc|d6|pc|d7|pc|dA|pc|d3|pg||mc|10|pg||";

fn exercise(bytes: &[u8]) {
    let (file, _warnings) = pbn::parse_lenient(bytes);
    let mut previous = None;
    for game in &file.games {
        let view = game.view(previous.as_ref());
        previous = view.ok();
    }
    let export = pbn::write(&file, WriteOptions { export: true });
    let _ = pbn::parse_strict(&export);
    let _ = pbn::write(&file, WriteOptions::default());
    if let Ok(text) = std::str::from_utf8(bytes) {
        let _ = pbn::parse_strict(text);
    }
    let (boards, _warnings) = lin::parse_lenient(bytes);
    for board in &boards {
        let _ = board.to_game().view(None);
    }
}

fn mutate(base: &[u8], cut: usize, flips: &[(Index, u8)]) -> Vec<u8> {
    let mut bytes = base[..cut.min(base.len())].to_vec();
    for (index, value) in flips {
        if !bytes.is_empty() {
            let k = index.index(bytes.len());
            bytes[k] = *value;
        }
    }
    bytes
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn random_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..1500)) {
        exercise(&bytes);
    }

    #[test]
    fn pbn_looking_text(text in "[\\[\\]\"{};%#=$*+\\-!?\\\\ \\n\\r\\ta-zA-Z0-9\u{e9}|,:.]{0,400}") {
        exercise(text.as_bytes());
    }

    #[test]
    fn mutated_pbn_sample(
        cut in 0..=SAMPLE.len(),
        flips in proptest::collection::vec((any::<Index>(), any::<u8>()), 0..6),
    ) {
        exercise(&mutate(SAMPLE.as_bytes(), cut, &flips));
    }

    #[test]
    fn mutated_lin_sample(
        cut in 0..=LIN_SAMPLE.len(),
        flips in proptest::collection::vec((any::<Index>(), any::<u8>()), 0..6),
    ) {
        exercise(&mutate(LIN_SAMPLE.as_bytes(), cut, &flips));
    }
}

#[test]
fn every_truncation_of_the_samples() {
    for sample in [SAMPLE, LIN_SAMPLE] {
        for cut in 0..=sample.len() {
            exercise(&sample.as_bytes()[..cut]);
        }
    }
}

/// Truncations of the reference files; skipped when the corpus is absent.
#[test]
fn truncations_of_reference_files() {
    let Some(dir) = common::corpus_dir() else {
        return;
    };
    for name in [
        "pbn/OptimumResultTable.pbn",
        "pbn/OptimumPlayTable.pbn",
        "lin/87141.lin",
    ] {
        let path = dir.join(name);
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("{} not found; skipping", path.display());
            continue;
        };
        let step = (bytes.len() / 150).max(1);
        for cut in (0..=bytes.len()).step_by(step) {
            exercise(&bytes[..cut]);
        }
        exercise(&bytes);
    }
}
