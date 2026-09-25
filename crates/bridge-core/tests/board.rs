//! Dealer and vulnerability schedules, `Board` and `Deal::new` validation.

use bridge_core::{Board, Card, DdTable, Deal, DealError, Hand, Seat, Side, Strain, Vulnerability};

#[test]
fn vulnerability_schedule_boards_1_to_16() {
    use Vulnerability::{Both, EW, NS, None as Love};
    let expected = [
        Love, NS, EW, Both, NS, EW, Both, Love, EW, Both, Love, NS, Both, Love, NS, EW,
    ];
    for (i, v) in expected.iter().enumerate() {
        let n = i as u16 + 1;
        assert_eq!(Vulnerability::from_board_number(n), *v, "board {n}");
        assert_eq!(
            Vulnerability::from_board_number(n + 16),
            *v,
            "board {}",
            n + 16
        );
    }
    assert_eq!(
        Vulnerability::from_board_number(0),
        Vulnerability::from_board_number(16)
    );
    assert!(Both.is_vulnerable_side(Side::NS));
    assert!(Both.is_vulnerable(Seat::East));
    assert!(NS.is_vulnerable(Seat::South));
    assert!(!NS.is_vulnerable(Seat::West));
    assert!(!Love.is_vulnerable_side(Side::EW));
}

#[test]
fn dealer_schedule() {
    assert_eq!(Seat::dealer_of_board(1), Seat::North);
    assert_eq!(Seat::dealer_of_board(2), Seat::East);
    assert_eq!(Seat::dealer_of_board(3), Seat::South);
    assert_eq!(Seat::dealer_of_board(4), Seat::West);
    assert_eq!(Seat::dealer_of_board(5), Seat::North);
    assert_eq!(Seat::dealer_of_board(16), Seat::West);
}

fn sample_deal() -> Deal {
    "N:AKQ.234.AKQ.2345 J2.AKQJ.J.AKQJT9 T98.T98.T9876.87 76543.765.5432.6"
        .parse()
        .unwrap()
}

#[test]
fn board_construction() {
    let deal = sample_deal();
    let b = Board::new(2, deal);
    assert_eq!(b.number, 2);
    assert_eq!(b.dealer, Seat::East);
    assert_eq!(b.vulnerability, Vulnerability::NS);
    assert_eq!(b.deal, deal);
    let b = Board::with_conditions(2, Seat::West, Vulnerability::Both, deal);
    assert_eq!(b.dealer, Seat::West);
    assert_eq!(b.vulnerability, Vulnerability::Both);
}

#[test]
fn deal_new_validates() {
    let deal = sample_deal();
    let hands = deal.hands();
    assert_eq!(Deal::new(hands), Ok(deal));

    // A short hand is reported before anything else, with its seat.
    let mut short = hands;
    short[2] =
        short[2].without(Card::from_index(short[2].cards().next().unwrap().index()).unwrap());
    assert_eq!(
        Deal::new(short),
        Err(DealError::HandSize {
            seat: Seat::South,
            count: 12
        })
    );

    // Same size, but a card held twice: the lowest such card is reported.
    let mut dup = hands;
    let east_low = hands[1].cards().next().unwrap();
    let west_low = hands[3].cards().next().unwrap();
    dup[3] = dup[3].without(west_low).with(east_low);
    assert_eq!(dup[3].len(), 13);
    assert_eq!(Deal::new(dup), Err(DealError::Duplicate(east_low)));
    assert_eq!(
        DealError::Duplicate(east_low).to_string(),
        format!("card {east_low} is held by two seats")
    );

    assert_eq!(
        Deal::new([Hand::EMPTY; 4]),
        Err(DealError::HandSize {
            seat: Seat::North,
            count: 0
        })
    );
    for card in Hand::FULL.cards() {
        assert!(deal.hand(deal.owner(card)).contains(card));
    }
}

#[test]
fn dd_table_best_for() {
    let table = DdTable::new([
        [7, 6, 7, 6],   // C
        [8, 5, 8, 5],   // D
        [10, 3, 10, 3], // H
        [10, 3, 10, 3], // S
        [9, 4, 9, 4],   // NT
    ]);
    assert_eq!(table.tricks(Strain::Hearts, Seat::North), 10);
    assert_eq!(table.tricks(Strain::NoTrump, Seat::East), 4);
    // Ties go to the higher strain.
    assert_eq!(table.best_for(Seat::North), (Strain::Spades, 10));
    assert_eq!(table.best_for(Seat::East), (Strain::Clubs, 6));
    assert_eq!(table.as_array()[4][2], 9);
    let flat = DdTable::new([[0; 4]; 5]);
    assert_eq!(flat.best_for(Seat::West), (Strain::NoTrump, 0));
}
