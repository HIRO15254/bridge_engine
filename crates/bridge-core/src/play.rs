//! The play of the cards.

use crate::{Card, Hand, Seat, Side, Strain, Suit};

/// The cards played so far, in order.
///
/// Seats are not stored: the player of card `i` is derived from the opening leader and the
/// winners of the previous tricks (`seat_at`). This keeps the history free of redundant state.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct PlayHistory {
    trump: Strain,
    leader: Seat,
    cards: Vec<Card>,
}

impl PlayHistory {
    /// An empty history for a contract in `trump` led by `leader`.
    pub fn new(trump: Strain, leader: Seat) -> PlayHistory {
        PlayHistory {
            trump,
            leader,
            cards: Vec::new(),
        }
    }

    /// The trump strain.
    pub fn trump(&self) -> Strain {
        self.trump
    }

    /// The opening leader.
    pub fn leader(&self) -> Seat {
        self.leader
    }

    /// The cards played so far.
    pub fn cards(&self) -> &[Card] {
        &self.cards
    }

    /// Whether `card` may be played now by the player to act, whose unplayed cards are
    /// `remaining`: the card must be held and unplayed, and must follow suit when possible.
    pub fn is_legal(&self, card: Card, remaining: Hand) -> bool {
        todo!("phase 1")
    }

    /// Plays `card`, or returns an error and leaves the history unchanged.
    pub fn play(&mut self, card: Card, remaining: Hand) -> Result<(), PlayError> {
        todo!("phase 1")
    }

    /// The seat that played (or will play) card `index`.
    pub fn seat_at(&self, index: usize) -> Seat {
        self.trick_leader(index / 4).offset((index % 4) as u8)
    }

    /// The seat to play next.
    pub fn next_to_play(&self) -> Seat {
        self.seat_at(self.cards.len())
    }

    /// The leader of trick `t` (0-based): the opening leader for `t == 0`, otherwise the winner
    /// of trick `t - 1`.
    pub fn trick_leader(&self, t: usize) -> Seat {
        todo!("phase 1")
    }

    /// The winner of trick `t`, or `None` if that trick is incomplete.
    ///
    /// A card's key is `32 + rank` for a trump, `16 + rank` for the led suit, `rank` otherwise;
    /// the highest key wins.
    pub fn trick_winner(&self, t: usize) -> Option<Seat> {
        todo!("phase 1")
    }

    /// The cards of the trick in progress (empty when a trick has just been completed).
    pub fn current_trick(&self) -> &[Card] {
        let start = self.cards.len() - self.cards.len() % 4;
        &self.cards[start..]
    }

    /// The suit led to the trick in progress, if a card has been led.
    pub fn led_suit(&self) -> Option<Suit> {
        self.current_trick().first().map(|c| c.suit())
    }

    /// Completed and in-progress tricks in order.
    pub fn tricks(&self) -> Tricks<'_> {
        Tricks {
            history: self,
            next: 0,
        }
    }

    /// Every card played so far.
    pub fn played(&self) -> Hand {
        todo!("phase 1")
    }

    /// The cards played by `seat`.
    pub fn played_by(&self, seat: Seat) -> Hand {
        todo!("phase 1")
    }

    /// Number of completed tricks won by `side`.
    pub fn tricks_won(&self, side: Side) -> u8 {
        todo!("phase 1")
    }
}

/// One trick, complete or in progress.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Trick {
    /// The seat that led.
    pub leader: Seat,
    /// The cards in play order (up to four).
    pub cards: [Option<Card>; 4],
    /// The winner, once four cards have been played.
    pub winner: Option<Seat>,
}

/// Iterator over the tricks of a [`PlayHistory`].
#[derive(Clone, Debug)]
pub struct Tricks<'a> {
    history: &'a PlayHistory,
    next: usize,
}

impl Iterator for Tricks<'_> {
    type Item = Trick;

    fn next(&mut self) -> Option<Trick> {
        todo!("phase 1")
    }
}

/// An illegal card was offered to a [`PlayHistory`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum PlayError {
    /// The card has already been played.
    #[error("card {0} already played")]
    AlreadyPlayed(Card),
    /// The player does not hold the card.
    #[error("card {0} is not held")]
    NotHeld(Card),
    /// The player could follow suit but did not.
    #[error("must follow suit {led}")]
    Revoke {
        /// The suit led.
        led: Suit,
    },
    /// All 52 cards have been played.
    #[error("play is complete")]
    Complete,
}
