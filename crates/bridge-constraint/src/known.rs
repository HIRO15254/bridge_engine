//! Cards whose owner is known.

use bridge_core::{Hand, PlayHistory, Seat};

use crate::KnownCardsError;

/// Cards known to belong to each seat's ORIGINAL hand: a viewer's own hand, the exposed dummy,
/// and every card a seat has already played.
///
/// This is the deal-level counterpart of the sampler's `fixed` argument: seat `s` is sampled
/// with `fixed = known[s]` from `pool()`, and every constraint still refers to the original
/// 13-card hand.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KnownCards {
    /// Known cards per seat.
    pub known: [Hand; 4],
}

impl KnownCards {
    /// Validates that the four sets are pairwise disjoint and each has at most 13 cards.
    pub fn new(known: [Hand; 4]) -> Result<KnownCards, KnownCardsError> {
        todo!("phase 2")
    }

    /// Nothing known.
    pub const EMPTY: KnownCards = KnownCards {
        known: [Hand::EMPTY; 4],
    };

    /// A viewer who knows only their own hand.
    pub fn from_viewer(viewer: Seat, hand: Hand) -> KnownCards {
        let mut known = [Hand::EMPTY; 4];
        known[viewer.index() as usize] = hand;
        KnownCards { known }
    }

    /// Adds the exposed dummy.
    pub fn with_dummy(mut self, dummy: Seat, hand: Hand) -> KnownCards {
        self.known[dummy.index() as usize] = self.known[dummy.index() as usize].union(hand);
        self
    }

    /// Adds every card each seat has played so far.
    pub fn with_play(self, history: &PlayHistory) -> KnownCards {
        todo!("phase 5")
    }

    /// The cards whose owner is unknown.
    pub fn pool(&self) -> Hand {
        Hand::FULL.difference(
            self.known[0]
                .union(self.known[1])
                .union(self.known[2])
                .union(self.known[3]),
        )
    }

    /// How many more cards `seat` needs: `13 − known[seat].len()`.
    pub fn needed(&self, seat: Seat) -> u8 {
        13 - self.known[seat.index() as usize].len()
    }
}
