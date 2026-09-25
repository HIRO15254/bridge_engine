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
        self.check(card, remaining).is_ok()
    }

    /// Plays `card`, or returns an error and leaves the history unchanged.
    pub fn play(&mut self, card: Card, remaining: Hand) -> Result<(), PlayError> {
        self.check(card, remaining)?;
        self.cards.push(card);
        Ok(())
    }

    /// The legality check behind [`PlayHistory::is_legal`] and [`PlayHistory::play`].
    ///
    /// Checked in this order: the play is not complete, the card is held, the card is unplayed,
    /// and the card follows suit when the player still holds the suit led. Cards of
    /// `remaining` that have already been played are ignored for the follow-suit test, so the
    /// player's original hand may be passed instead of the exact remainder.
    fn check(&self, card: Card, remaining: Hand) -> Result<(), PlayError> {
        if self.cards.len() >= 52 {
            return Err(PlayError::Complete);
        }
        if !remaining.contains(card) {
            return Err(PlayError::NotHeld(card));
        }
        let played = self.played();
        if played.contains(card) {
            return Err(PlayError::AlreadyPlayed(card));
        }
        match self.led_suit() {
            Some(led)
                if card.suit() != led && !remaining.difference(played).holding(led).is_empty() =>
            {
                Err(PlayError::Revoke { led })
            }
            _ => Ok(()),
        }
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
    ///
    /// # Panics
    /// Panics if trick `t - 1` is incomplete (there is then no leader of trick `t`). `t` may be
    /// at most the number of completed tricks, which is what [`PlayHistory::seat_at`] and
    /// [`PlayHistory::next_to_play`] always satisfy.
    pub fn trick_leader(&self, t: usize) -> Seat {
        let mut leader = self.leader;
        for k in 0..t {
            let offset = self
                .winner_offset(k)
                .expect("PlayHistory::trick_leader: the previous trick is incomplete");
            leader = leader.offset(offset);
        }
        leader
    }

    /// The winner of trick `t`, or `None` if that trick is incomplete.
    ///
    /// A card's key is `32 + rank` for a trump, `16 + rank` for the led suit, `rank` otherwise;
    /// the highest key wins.
    pub fn trick_winner(&self, t: usize) -> Option<Seat> {
        let offset = self.winner_offset(t)?;
        Some(self.trick_leader(t).offset(offset))
    }

    /// Offset from the leader of the winner of trick `t`, or `None` if it is incomplete.
    fn winner_offset(&self, t: usize) -> Option<u8> {
        self.cards
            .get(t * 4..t * 4 + 4)
            .map(|cards| winner_offset(cards, self.trump))
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
            leader: self.leader,
        }
    }

    /// Every card played so far.
    pub fn played(&self) -> Hand {
        self.cards.iter().fold(Hand::EMPTY, |h, c| h.with(*c))
    }

    /// The cards played by `seat`.
    pub fn played_by(&self, seat: Seat) -> Hand {
        self.tricks().fold(Hand::EMPTY, |hand, trick| {
            let k = (seat.index() + 4 - trick.leader.index()) % 4;
            match trick.cards[k as usize] {
                Some(card) => hand.with(card),
                None => hand,
            }
        })
    }

    /// Number of completed tricks won by `side`.
    pub fn tricks_won(&self, side: Side) -> u8 {
        self.tricks()
            .filter(|t| t.winner.is_some_and(|w| w.side() == side))
            .count() as u8
    }
}

/// Offset from the leader of the winner of a complete trick (`cards.len() == 4`).
///
/// Key: `32 + rank` for a trump, `16 + rank` for the led suit, `rank` otherwise. The cards are
/// distinct, so there is never a tie.
fn winner_offset(cards: &[Card], trump: Strain) -> u8 {
    let led = cards[0].suit();
    let trump = trump.suit();
    let key = |c: &Card| {
        let group = if trump == Some(c.suit()) {
            32
        } else if c.suit() == led {
            16
        } else {
            0
        };
        group + c.rank().index()
    };
    cards
        .iter()
        .enumerate()
        .max_by_key(|(_, c)| key(c))
        .map_or(0, |(j, _)| j as u8)
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
    /// Leader of trick `next` (carried forward so the walk is linear).
    leader: Seat,
}

impl Iterator for Tricks<'_> {
    type Item = Trick;

    fn next(&mut self) -> Option<Trick> {
        let history = self.history;
        let start = self.next * 4;
        if start >= history.cards.len() {
            return None;
        }
        let end = (start + 4).min(history.cards.len());
        let chunk = &history.cards[start..end];
        let leader = self.leader;
        let mut cards = [None; 4];
        for (slot, card) in cards.iter_mut().zip(chunk) {
            *slot = Some(*card);
        }
        let winner = (chunk.len() == 4).then(|| leader.offset(winner_offset(chunk, history.trump)));
        if let Some(w) = winner {
            self.leader = w;
        }
        self.next += 1;
        Some(Trick {
            leader,
            cards,
            winner,
        })
    }
}

impl core::iter::FusedIterator for Tricks<'_> {}

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
