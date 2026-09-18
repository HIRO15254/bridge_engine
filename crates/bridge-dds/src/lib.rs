//! Safe wrapper around DDS (Bo Haglund & Soren Hein, v2.9.0, Apache-2.0).
//!
//! Native only: this crate is excluded from `wasm32` builds. The C++ sources are vendored by
//! `cargo xtask dds vendor` and compiled by `build.rs`; without them the crate builds with the
//! FFI compiled out and every call returns [`DdsError::Unavailable`].
//!
//! Threading: `SolveBoard` is reentrant per thread index and may run concurrently from several
//! Rust threads (a slot is handed out per call); the bulk functions (`SolveAllChunksBin`,
//! `CalcAllTables`, `AnalyseAllPlaysBin`) are not reentrant and are serialised by a mutex.
//! DDS is initialised once (`SetResources`) with explicit limits, because 2.9's own memory
//! probing shells out to `sysctl` / `free` and can fail in sandboxes.
#![warn(missing_docs)]
// Phase 0 skeleton: the public surface is final, bodies are `todo!()`.
#![allow(dead_code, unused_variables)]

#[cfg(target_arch = "wasm32")]
compile_error!("bridge-dds is native only; disable the `dds` feature of `bridge` for wasm32");

pub mod convert;
#[cfg(dds_vendored)]
pub mod sys;

pub use bridge_core::DdTable;
use bridge_core::{Card, Deal, DealError, Holding, PlayHistory, Seat, Strain, Vulnerability};

/// Resource limits given to DDS once.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DdsConfig {
    /// Threads (0 = DDS decides).
    pub max_threads: u32,
    /// Memory in MB (0 = DDS decides; the default passes `threads × 95` explicitly).
    pub max_memory_mb: u32,
}

/// Initialises DDS once; later calls with a different config log a warning and keep the first.
pub fn init(cfg: DdsConfig) -> Result<(), DdsError> {
    todo!("phase 5")
}

/// `true` when the DDS sources were vendored and compiled in.
pub const fn is_available() -> bool {
    cfg!(dds_vendored)
}

/// A position to solve.
#[derive(Clone, Copy, Debug)]
pub struct Position<'a> {
    /// Remaining cards.
    pub deal: &'a Deal,
    /// Trump.
    pub trump: Strain,
    /// The player to lead (or on lead when `trick` is empty).
    pub leader: Seat,
    /// Cards already in the current trick (0 to 3).
    pub trick: &'a [Card],
}

/// `SolveBoard` target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// Maximum tricks (`-1`).
    Max,
    /// Just list the legal cards (`0`).
    ListLegal,
    /// Whether at least `n` tricks can be made (`1..=13`).
    Tricks(u8),
}

/// `SolveBoard` solutions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Solutions {
    /// One optimal card (`1`).
    One,
    /// All optimal cards (`2`).
    AllOptimal,
    /// Every legal card with its score (`3`); the lead advisor's query.
    AllRanked,
}

/// `SolveBoard` mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Automatic (`0`).
    Auto,
    /// Always search (`1`).
    Search,
    /// Reuse the transposition table (`2`).
    ReuseTable,
}

/// A scored card from `SolveBoard`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CardScore {
    /// The card.
    pub card: Card,
    /// Lower cards of the same suit with the same score.
    pub equals: Holding,
    /// Tricks for the side on lead.
    pub score: u8,
}

/// The result of `SolveBoard`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FutureTricks {
    /// Nodes searched.
    pub nodes: u32,
    /// Scored cards.
    pub cards: Vec<CardScore>,
}

/// Par result for a board.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParResult {
    /// Score from NS's point of view.
    pub score: i32,
    /// Par contracts as strings.
    pub contracts: Vec<String>,
}

/// DDS build information.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DdsInfo {
    /// Version string.
    pub version: String,
    /// Threads in use.
    pub threads: u32,
    /// Threading backend code.
    pub threading: i32,
    /// Free-form system description.
    pub system: String,
}

/// The double-dummy table of a deal (`CalcDDtable`; internally threaded).
pub fn calc_dd_table(deal: &Deal) -> Result<DdTable, DdsError> {
    todo!("phase 5")
}

/// Double-dummy tables for many deals (`CalcAllTables`, 40 per chunk).
pub fn calc_dd_tables(deals: &[Deal]) -> Result<Vec<DdTable>, DdsError> {
    todo!("phase 5")
}

/// Solves one position (`SolveBoard`; reentrant across threads).
pub fn solve_board(
    pos: &Position<'_>,
    target: Target,
    solutions: Solutions,
    mode: Mode,
) -> Result<FutureTricks, DdsError> {
    todo!("phase 5")
}

/// Solves many positions (`SolveAllChunksBin`, 200 per chunk; serialised).
pub fn solve_all_boards(
    positions: &[(Position<'_>, Target, Solutions, Mode)],
) -> Result<Vec<FutureTricks>, DdsError> {
    todo!("phase 5")
}

/// Double-dummy tricks after each card of a play (`AnalysePlayBin`).
pub fn analyse_play(deal: &Deal, play: &PlayHistory) -> Result<Vec<u8>, DdsError> {
    todo!("phase 5")
}

/// Par score and contracts (`DealerParBin`).
pub fn dealer_par(
    table: &DdTable,
    dealer: Seat,
    vul: Vulnerability,
) -> Result<ParResult, DdsError> {
    todo!("phase 5")
}

/// Build information (`GetDDSInfo`).
pub fn info() -> Result<DdsInfo, DdsError> {
    todo!("phase 5")
}

/// A DDS call failed.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum DdsError {
    /// DDS returned an error code.
    #[error("DDS error {code}: {message}")]
    Code {
        /// The code.
        code: i32,
        /// `ErrorMessage` text.
        message: String,
    },
    /// The deal is invalid.
    #[error(transparent)]
    Deal(#[from] DealError),
    /// More than 200 boards in one call.
    #[error("too many boards: {0} > 200")]
    TooManyBoards(usize),
    /// The DDS sources were not vendored at build time.
    #[error("DDS is not available in this build (run `cargo xtask dds vendor` and rebuild)")]
    Unavailable,
}
