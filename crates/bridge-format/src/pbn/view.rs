//! Typed view of a game.

use bridge_core::{Auction, Contract, DdTable, Hand, PlayHistory, Seat, Vulnerability};

use crate::pbn::Game;

/// A deal in which some hands may be unknown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PartialDeal {
    /// Known hands indexed by [`Seat`]; `None` for `-`.
    pub hands: [Option<Hand>; 4],
}

impl PartialDeal {
    /// The full deal, if all four hands are known and valid.
    pub fn complete(&self) -> Option<bridge_core::Deal> {
        todo!("phase 1")
    }
}

/// The interpreted content of a game: typed values derived from the tags and sections.
#[derive(Clone, Debug, Default)]
pub struct GameView {
    /// `Board`.
    pub board: Option<u16>,
    /// `Dealer`.
    pub dealer: Option<Seat>,
    /// `Vulnerable`.
    pub vulnerable: Option<Vulnerability>,
    /// `Deal`.
    pub deal: Option<PartialDeal>,
    /// The `Auction` section as a validated auction.
    pub auction: Option<Auction>,
    /// The `Play` section rotated into trick order.
    pub play: Option<PlayHistory>,
    /// `Contract` (with `Declarer`).
    pub contract: Option<Contract>,
    /// `Declarer`.
    pub declarer: Option<Seat>,
    /// `Result` (tricks taken by declarer).
    pub result: Option<u8>,
    /// `OptimumResultTable`.
    pub dd_table: Option<DdTable>,
}

impl Game {
    /// Interprets the tags and sections. `previous` supplies values for `#` inheritance.
    pub fn view(&self, previous: Option<&GameView>) -> Result<GameView, ViewError> {
        todo!("phase 1")
    }
}

/// A game could not be interpreted.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum ViewError {
    /// A tag value has the wrong form.
    #[error("tag {tag}: {message}")]
    BadTag {
        /// Tag name.
        tag: String,
        /// Detail.
        message: String,
    },
    /// The auction section is not a legal auction.
    #[error("auction: {0}")]
    Auction(#[from] bridge_core::AuctionError),
    /// The play section is not a legal play.
    #[error("play: {0}")]
    Play(#[from] bridge_core::PlayError),
    /// A `#` value has nothing to inherit from.
    #[error("tag {0} inherits from a previous game but there is none")]
    NothingToInherit(String),
}
