//! Contract bridge foundation library: the facade crate.
//!
//! Applications depend on this crate only. It re-exports the layered crates:
//!
//! | Module | Crate | Layer |
//! | --- | --- | --- |
//! | (root) | `bridge-core` | L0 domain types |
//! | [`eval`] | `bridge-eval` | L1 evaluation |
//! | [`constraint`] | `bridge-constraint` | L1 constraints and sampler |
//! | [`system`] | `bridge-system` | L2 system definitions |
//! | [`bidding`] | `bridge-bidding` | L3 interpreter and generator |
//! | [`play`] | `bridge-play` | L5 play inference |
//! | [`sample`] | `bridge-sample` | L4 deal sampler |
//! | [`format`](mod@format) | `bridge-format` | PBN / LIN (feature `format`, on by default) |
//! | [`dd`] | `bridge-dds` behind a trait | double dummy (feature `dds`, native only) |
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub use bridge_core::*;

/// Hand evaluation.
pub mod eval {
    pub use bridge_eval::*;
}

/// Constraint language and sampler.
pub mod constraint {
    pub use bridge_constraint::*;
}

/// Bidding-system definitions.
pub mod system {
    pub use bridge_system::*;
}

/// Auction interpreter and bid generator.
pub mod bidding {
    pub use bridge_bidding::*;
}

/// Play inference.
pub mod play {
    pub use bridge_play::*;
}

/// Deal sampling.
pub mod sample {
    pub use bridge_sample::*;
}

/// Record formats.
#[cfg(feature = "format")]
pub mod format {
    pub use bridge_format::*;
}

/// Double-dummy analysis behind a trait, so that applications compile with or without DDS.
pub mod dd {
    use std::sync::Arc;

    pub use bridge_core::DdTable;
    use bridge_core::{Card, Deal, Seat, Strain};

    /// A double-dummy solver.
    pub trait DoubleDummy: Send + Sync {
        /// The full table for a deal.
        fn dd_table(&self, deal: &Deal) -> Result<DdTable, DdError>;

        /// Every legal opening lead for `leader` against `trump` with the tricks the defence
        /// then takes (the opening-lead advisor's query).
        fn lead_scores(
            &self,
            deal: &Deal,
            trump: Strain,
            leader: Seat,
        ) -> Result<Vec<(Card, u8)>, DdError>;
    }

    /// A double-dummy query failed.
    #[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
    pub enum DdError {
        /// No solver is compiled into this build (feature `dds`, native targets only).
        #[error("no double-dummy solver available in this build")]
        Unavailable,
        /// The solver reported an error.
        #[error("double-dummy solver: {0}")]
        Backend(String),
    }

    /// The DDS-backed solver, if this build has one.
    pub fn dds() -> Option<Arc<dyn DoubleDummy>> {
        #[cfg(all(feature = "dds", not(target_arch = "wasm32")))]
        {
            if bridge_dds::is_available() {
                return Some(Arc::new(DdsBackend));
            }
        }
        None
    }

    #[cfg(all(feature = "dds", not(target_arch = "wasm32")))]
    struct DdsBackend;

    #[cfg(all(feature = "dds", not(target_arch = "wasm32")))]
    impl DoubleDummy for DdsBackend {
        fn dd_table(&self, deal: &Deal) -> Result<DdTable, DdError> {
            bridge_dds::calc_dd_table(deal).map_err(|e| DdError::Backend(e.to_string()))
        }

        fn lead_scores(
            &self,
            deal: &Deal,
            trump: Strain,
            leader: Seat,
        ) -> Result<Vec<(Card, u8)>, DdError> {
            let pos = bridge_dds::Position {
                deal,
                trump,
                leader,
                trick: &[],
            };
            let ft = bridge_dds::solve_board(
                &pos,
                bridge_dds::Target::Max,
                bridge_dds::Solutions::AllRanked,
                bridge_dds::Mode::Auto,
            )
            .map_err(|e| DdError::Backend(e.to_string()))?;
            Ok(ft.cards.into_iter().map(|c| (c.card, c.score)).collect())
        }
    }
}
