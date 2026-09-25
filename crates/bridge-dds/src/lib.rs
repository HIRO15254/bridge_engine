//! Safe wrapper around DDS (Bo Haglund & Soren Hein, v2.9.0, Apache-2.0).
//!
//! Native only: this crate is excluded from `wasm32` builds. The C++ sources are vendored by
//! `cargo xtask dds vendor` and compiled by `build.rs`; without them the crate builds with the
//! FFI compiled out and every call returns [`DdsError::Unavailable`].
//!
//! Threading: `SolveBoard`/`AnalysePlayBin` are reentrant per thread index and may run
//! concurrently from several Rust threads (a slot is handed out per call); the bulk functions
//! (`SolveAllChunksBin`, `CalcAllTables`, `CalcDDtable`, `AnalyseAllPlaysBin`) are not reentrant
//! with each other *or* with a slot-holding call, since they also drive DDS's own
//! per-thread-index state internally, so a bulk call takes every slot for its duration. DDS is
//! initialised once (`SetResources`) with explicit limits, because 2.9's own memory probing
//! shells out to `sysctl` / `free` and can fail in sandboxes.
#![warn(missing_docs)]

#[cfg(target_arch = "wasm32")]
compile_error!("bridge-dds is native only; disable the `dds` feature of `bridge` for wasm32");

pub mod convert;
#[cfg(dds_vendored)]
pub mod sys;

pub use bridge_core::DdTable;
use bridge_core::{Card, Deal, Holding, PlayHistory, Seat, Strain, Vulnerability};

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
    #[cfg(dds_vendored)]
    {
        let rt = backend::runtime_with(cfg);
        if rt.cfg() != cfg {
            tracing::warn!(
                requested.max_threads = cfg.max_threads,
                requested.max_memory_mb = cfg.max_memory_mb,
                active.max_threads = rt.cfg().max_threads,
                active.max_memory_mb = rt.cfg().max_memory_mb,
                "bridge_dds::init called again with a different config; keeping the first"
            );
        }
        Ok(())
    }
    #[cfg(not(dds_vendored))]
    {
        let _ = cfg;
        Err(DdsError::Unavailable)
    }
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
    #[cfg(dds_vendored)]
    return backend::calc_dd_table(deal);
    #[cfg(not(dds_vendored))]
    {
        let _ = deal;
        Err(DdsError::Unavailable)
    }
}

/// Double-dummy tables for many deals (`CalcAllTables`, 40 per chunk).
pub fn calc_dd_tables(deals: &[Deal]) -> Result<Vec<DdTable>, DdsError> {
    #[cfg(dds_vendored)]
    return backend::calc_dd_tables(deals);
    #[cfg(not(dds_vendored))]
    {
        let _ = deals;
        Err(DdsError::Unavailable)
    }
}

/// Solves one position (`SolveBoard`; reentrant across threads).
pub fn solve_board(
    pos: &Position<'_>,
    target: Target,
    solutions: Solutions,
    mode: Mode,
) -> Result<FutureTricks, DdsError> {
    #[cfg(dds_vendored)]
    return backend::solve_board(pos, target, solutions, mode);
    #[cfg(not(dds_vendored))]
    {
        let (_, _, _, _) = (pos, target, solutions, mode);
        Err(DdsError::Unavailable)
    }
}

/// Solves many positions (`SolveAllChunksBin`, 200 per chunk; serialised).
pub fn solve_all_boards(
    positions: &[(Position<'_>, Target, Solutions, Mode)],
) -> Result<Vec<FutureTricks>, DdsError> {
    #[cfg(dds_vendored)]
    return backend::solve_all_boards(positions);
    #[cfg(not(dds_vendored))]
    {
        let _ = positions;
        Err(DdsError::Unavailable)
    }
}

/// Double-dummy tricks after each card of a play (`AnalysePlayBin`).
pub fn analyse_play(deal: &Deal, play: &PlayHistory) -> Result<Vec<u8>, DdsError> {
    #[cfg(dds_vendored)]
    return backend::analyse_play(deal, play);
    #[cfg(not(dds_vendored))]
    {
        let (_, _) = (deal, play);
        Err(DdsError::Unavailable)
    }
}

/// Par score and contracts (`DealerParBin`).
pub fn dealer_par(
    table: &DdTable,
    dealer: Seat,
    vul: Vulnerability,
) -> Result<ParResult, DdsError> {
    #[cfg(dds_vendored)]
    return backend::dealer_par(table, dealer, vul);
    #[cfg(not(dds_vendored))]
    {
        let (_, _, _) = (table, dealer, vul);
        Err(DdsError::Unavailable)
    }
}

/// Build information (`GetDDSInfo`).
pub fn info() -> Result<DdsInfo, DdsError> {
    #[cfg(dds_vendored)]
    return backend::info();
    #[cfg(not(dds_vendored))]
    Err(DdsError::Unavailable)
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
    /// The DDS sources were not vendored at build time.
    #[error("DDS is not available in this build (run `cargo xtask dds vendor` and rebuild)")]
    Unavailable,
}

/// The FFI-backed implementation, compiled only when `build.rs` vendored and compiled DDS.
/// Every public function above just delegates into here (or returns [`DdsError::Unavailable`]
/// when this module does not exist), so the public API is identical either way.
#[cfg(dds_vendored)]
mod backend {
    use core::ffi::{CStr, c_char, c_int};
    use std::sync::{Condvar, Mutex, OnceLock};

    use bridge_core::{Deal, PlayHistory, Seat, Strain, Vulnerability};

    use crate::{
        CardScore, DdTable, DdsConfig, DdsError, DdsInfo, FutureTricks, Mode, ParResult, Position,
        Solutions, Target, convert, sys,
    };

    /// The `free`/`batch_waiting` state behind `SlotPool`'s single `Mutex` (see `SlotPool`
    /// below for why both live under one lock).
    struct SlotState {
        /// Thread-index slots not currently held by a `solve_board`/`analyse_play` call.
        free: Vec<c_int>,
        /// Total slot count (`= noOfThreads`), i.e. `free.len()` once every slot is back.
        total: c_int,
        /// Set while a batch call is waiting for (or holding) every slot; blocks new
        /// `acquire_slot` calls so a steady stream of `solve_board`/`analyse_play` traffic
        /// cannot starve the batch call out (see `acquire_all_slots`).
        batch_waiting: bool,
    }

    /// The thread-index slot pool, shared by every DDS entry point that touches DDS's
    /// per-thread-index internal state (`track[]`/`ABsearch`'s `Evaluate`, etc.).
    ///
    /// DDS documents `SolveBoard`/`AnalysePlayBin` as reentrant given a distinct `thrId` each
    /// (docs/design/10-dds.md §7.2 rule 2), and `SolveAllChunksBin`/`CalcAllTables`/
    /// `CalcDDtable`/`AnalyseAllPlaysBin` as merely non-reentrant *with each other* (rule 3).
    /// Both are true in isolation, but the two families are not independent: the bulk
    /// functions also solve internally using DDS's own thread pool, indexed the same way as
    /// the explicit `thrId` the reentrant functions pass in. Calling a bulk function
    /// concurrently with a slot-holding `solve_board`/`analyse_play` call from a different Rust
    /// thread lets both sides drive the *same* `track[i]`/`ABsearch` state for some `i` at
    /// once, corrupting it (observed as `Moves::GetTrickData`'s `"Sum N is not four"` abort or
    /// an `ABsearch.cpp` assertion under concurrent load). So a bulk call needs every slot
    /// (i.e. the exclusive side of a one-writer/many-readers lock over the same index space),
    /// not just its own separate mutex.
    pub(super) struct SlotPool {
        state: Mutex<SlotState>,
        slot_available: Condvar,
    }

    impl SlotPool {
        fn new(total: c_int) -> SlotPool {
            SlotPool {
                state: Mutex::new(SlotState {
                    free: (0..total).collect(),
                    total,
                    batch_waiting: false,
                }),
                slot_available: Condvar::new(),
            }
        }

        /// Blocks until a `SolveBoard`/`AnalysePlayBin` thread-index slot is free (and no batch
        /// call is waiting for/holding every slot), then takes it.
        pub(super) fn acquire_slot(&self) -> c_int {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if !state.batch_waiting {
                    if let Some(id) = state.free.pop() {
                        return id;
                    }
                }
                state = self
                    .slot_available
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
        }

        pub(super) fn release_slot(&self, id: c_int) {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.free.push(id);
            drop(state);
            // Every waiter (other `acquire_slot` calls and a possible `acquire_all_slots`
            // waiter) must recheck, since only one of them can actually be the one this slot
            // unblocks.
            self.slot_available.notify_all();
        }

        /// Blocks until every slot is free (marking a batch call as waiting/active first, so
        /// `acquire_slot` stops handing new slots out and this cannot starve), then takes them
        /// all, giving the calling bulk function exclusive use of DDS's per-thread-index state.
        pub(super) fn acquire_all_slots(&self) -> Vec<c_int> {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                // Reasserted on every iteration: a *different* batch call already waiting here
                // may have just released all slots back (clearing this on its way out) while
                // this call still has not collected them, and `acquire_slot` must keep seeing
                // it set or a steady stream of slot-sized traffic could starve this call
                // forever.
                state.batch_waiting = true;
                if state.free.len() as c_int == state.total {
                    return std::mem::take(&mut state.free);
                }
                state = self
                    .slot_available
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
        }

        pub(super) fn release_all_slots(&self, mut ids: Vec<c_int>) {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.free.append(&mut ids);
            state.batch_waiting = false;
            drop(state);
            self.slot_available.notify_all();
        }
    }

    /// Per-process DDS state: the config that won the `init`/first-call race, and the
    /// `SlotPool` every entry point (reentrant or bulk) goes through (see docs/design/10-dds.md
    /// §7.2).
    pub(super) struct Runtime {
        cfg: DdsConfig,
        pub(super) slots: SlotPool,
    }

    static RUNTIME: OnceLock<Runtime> = OnceLock::new();

    impl Runtime {
        fn new(cfg: DdsConfig) -> Runtime {
            let requested_threads = if cfg.max_threads > 0 {
                cfg.max_threads
            } else {
                std::thread::available_parallelism().map_or(1, |n| n.get() as u32)
            };
            // R6: DDS 2.9's own memory probe (`System::GetHardware`) shells out to
            // `sysctl`/`free`, which can fail in a sandbox; pass an explicit value rather than
            // leaving it to DDS's own guess.
            let memory_mb = if cfg.max_memory_mb > 0 {
                cfg.max_memory_mb
            } else {
                requested_threads.saturating_mul(95)
            };
            // SAFETY: `SetResources` takes two plain integers and has no other precondition;
            // the surrounding `OnceLock` guarantees exactly one call for the process, before
            // any other DDS entry point runs.
            unsafe {
                sys::SetResources(memory_mb as c_int, cfg.max_threads as c_int);
            }
            let threads = dds_info().noOfThreads.max(1) as u32;
            Runtime {
                cfg,
                slots: SlotPool::new(threads as c_int),
            }
        }

        pub(super) fn cfg(&self) -> DdsConfig {
            self.cfg
        }
    }

    /// The process-wide runtime, initialising it with `DdsConfig::default()` on first use if
    /// `init` was never called (docs/design/10-dds.md §7.2 rule 1).
    pub(super) fn runtime() -> &'static Runtime {
        RUNTIME.get_or_init(|| Runtime::new(DdsConfig::default()))
    }

    /// Used by `init`: initialises with `cfg` if this is the first call anywhere in the
    /// process, otherwise returns the already-active runtime unchanged.
    pub(super) fn runtime_with(cfg: DdsConfig) -> &'static Runtime {
        RUNTIME.get_or_init(|| Runtime::new(cfg))
    }

    fn dds_info() -> sys::DDSInfo {
        let mut info = zeroed::<sys::DDSInfo>();
        // SAFETY: `info` is a validly sized, zero-initialised `DDSInfo` that `GetDDSInfo` only
        // writes into.
        unsafe {
            sys::GetDDSInfo(&mut info);
        }
        info
    }

    /// A zero-initialised value of a `sys` DDS struct: every one of them is a plain
    /// `#[repr(C)]` aggregate of integers (and arrays thereof), and DDS's own C++ side
    /// `calloc`s the very same structs, so the all-zero bit pattern is always a valid value.
    ///
    /// Only ever instantiated at the concrete `sys::*` types used below; not a safe general
    /// tool for arbitrary `T`.
    fn zeroed<T>() -> T {
        // SAFETY: see the function doc comment; every call site below names a concrete `sys`
        // struct.
        unsafe { core::mem::MaybeUninit::<T>::zeroed().assume_init() }
    }

    /// A large `sys` DDS struct (hundreds of KB: `boards`, `solvedBoards`, `ddTableDeals`, …),
    /// heap-allocated and zeroed directly, without ever materialising it on the stack
    /// (docs/design/10-dds.md §5).
    fn boxed_zeroed<T>() -> Box<T> {
        let layout = std::alloc::Layout::new::<T>();
        // SAFETY: `layout` is non-zero-sized for every DDS struct this is used with. The
        // pointer, once checked non-null, addresses a fresh allocation of exactly `T`'s size
        // and alignment; writing all zero bytes into it is valid for the same reason `zeroed`
        // above is (a plain aggregate of integers), so constructing a `Box<T>` from it is sound.
        unsafe {
            let ptr = std::alloc::alloc_zeroed(layout).cast::<T>();
            if ptr.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            Box::from_raw(ptr)
        }
    }

    /// `ErrorMessage(code)` as an owned string.
    fn error_message(code: c_int) -> String {
        let mut buf = [0 as c_char; 80];
        // SAFETY: `buf` is exactly the 80 bytes `ErrorMessage` documents (`dll.h`:
        // `char line[80]`); DDS always null-terminates within it.
        unsafe {
            sys::ErrorMessage(code, buf.as_mut_ptr());
        }
        // SAFETY: `buf` was just written by `ErrorMessage` above, within this same call, and is
        // null-terminated within its 80 bytes.
        let cstr = unsafe { CStr::from_ptr(buf.as_ptr()) };
        cstr.to_string_lossy().into_owned()
    }

    fn dds_error(code: c_int) -> DdsError {
        DdsError::Code {
            code,
            message: error_message(code),
        }
    }

    /// An error this wrapper detected itself, before ever calling into DDS (e.g. a malformed
    /// [`Position`]); reuses one of DDS's own `RETURN_*` codes and `dll.h`'s text for it; see
    /// `error_message`, which this deliberately does not call (there is nothing for DDS's own
    /// `ErrorMessage` to look up for a call DDS never made).
    fn synthetic_error(code: c_int, message: &str) -> DdsError {
        DdsError::Code {
            code,
            message: message.to_string(),
        }
    }

    /// A NUL-terminated fixed-size `char` buffer (`DDSInfo.versionString`/`.systemString`) as a
    /// `String`, replacing any invalid bytes rather than failing (these are diagnostic fields).
    fn fixed_str(bytes: &[c_char]) -> String {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let raw: Vec<u8> = bytes[..end].iter().map(|&b| b as u8).collect();
        String::from_utf8_lossy(&raw).into_owned()
    }

    const fn target_code(target: Target) -> c_int {
        match target {
            Target::Max => -1,
            Target::ListLegal => 0,
            Target::Tricks(n) => n as c_int,
        }
    }

    const fn solutions_code(solutions: Solutions) -> c_int {
        match solutions {
            Solutions::One => 1,
            Solutions::AllOptimal => 2,
            Solutions::AllRanked => 3,
        }
    }

    const fn mode_code(mode: Mode) -> c_int {
        match mode {
            Mode::Auto => 0,
            Mode::Search => 1,
            Mode::ReuseTable => 2,
        }
    }

    /// Builds the DDS `deal` for a [`Position`]: the whole `pos.deal`, minus the cards already
    /// played to the current (incomplete) trick — the only history a `Position` carries.
    /// Validates `pos.trick` first (`docs/design/10-dds.md` §7: length and no duplicates), the
    /// same way `SolveBoard` itself would reject a bad `currentTrickSuit`/`currentTrickRank`.
    fn position_deal(pos: &Position<'_>) -> Result<sys::deal, DdsError> {
        if pos.trick.len() > 3 {
            return Err(synthetic_error(
                sys::RETURN_SUIT_OR_RANK,
                "currentTrickSuit or currentTrickRank has wrong data",
            ));
        }
        for i in 0..pos.trick.len() {
            for j in (i + 1)..pos.trick.len() {
                if pos.trick[i] == pos.trick[j] {
                    return Err(synthetic_error(
                        sys::RETURN_DUPLICATE_CARDS,
                        "cards duplicated",
                    ));
                }
            }
        }

        let mut remain = convert::remain_cards(pos.deal);
        let mut current_trick_suit = [0 as c_int; 3];
        let mut current_trick_rank = [0 as c_int; 3];
        for (i, &card) in pos.trick.iter().enumerate() {
            let owner = pos.deal.owner(card);
            let seat_idx = convert::seat(owner) as usize;
            let suit_idx = convert::suit(card.suit()) as usize;
            let dds_rank = convert::rank(card.rank().index());
            remain[seat_idx][suit_idx] &= !(1u32 << dds_rank);
            current_trick_suit[i] = convert::suit(card.suit());
            current_trick_rank[i] = dds_rank;
        }

        Ok(sys::deal {
            trump: convert::strain(pos.trump),
            first: convert::seat(pos.leader),
            currentTrickSuit: current_trick_suit,
            currentTrickRank: current_trick_rank,
            remainCards: remain,
        })
    }

    fn future_tricks_from(fut: &sys::futureTricks) -> FutureTricks {
        let n = fut.cards.max(0) as usize;
        let mut cards = Vec::with_capacity(n);
        for i in 0..n {
            cards.push(CardScore {
                card: convert::card_from_dds(fut.suit[i], fut.rank[i]),
                equals: convert::holding_from_dds(fut.equals[i].max(0) as u32),
                score: fut.score[i].max(0) as u8,
            });
        }
        FutureTricks {
            nodes: fut.nodes.max(0) as u32,
            cards,
        }
    }

    fn table_from_dds(result: &sys::ddTableResults) -> DdTable {
        let mut tricks = [[0u8; 4]; 5];
        for strain in Strain::ALL {
            let dds_s = convert::strain(strain) as usize;
            for seat in Seat::ALL {
                tricks[strain.index() as usize][seat.index() as usize] =
                    result.resTable[dds_s][convert::seat(seat) as usize].max(0) as u8;
            }
        }
        DdTable::new(tricks)
    }

    pub(super) fn calc_dd_table(deal: &Deal) -> Result<DdTable, DdsError> {
        let rt = runtime();
        let table_deal = sys::ddTableDeal {
            cards: convert::remain_cards(deal),
        };
        let mut result = zeroed::<sys::ddTableResults>();
        let held_slots = rt.slots.acquire_all_slots();
        // SAFETY: `table_deal` is passed by value; `result` is a validly sized
        // `ddTableResults` the callee only writes into. `CalcDDtable` is documented
        // non-reentrant with the other bulk calls, and also drives DDS's own per-thread-index
        // state internally, so this call holds every slot for its duration (`SlotPool`'s doc
        // comment).
        let rc = unsafe { sys::CalcDDtable(table_deal, &mut result) };
        rt.slots.release_all_slots(held_slots);
        if rc != sys::RETURN_NO_FAULT {
            return Err(dds_error(rc));
        }
        Ok(table_from_dds(&result))
    }

    pub(super) fn calc_dd_tables(deals: &[Deal]) -> Result<Vec<DdTable>, DdsError> {
        let rt = runtime();
        let mut out = Vec::with_capacity(deals.len());
        for chunk in deals.chunks(sys::MAXNOOFTABLES) {
            let mut dealsp = boxed_zeroed::<sys::ddTableDeals>();
            dealsp.noOfTables = chunk.len() as c_int;
            for (i, deal) in chunk.iter().enumerate() {
                dealsp.deals[i] = sys::ddTableDeal {
                    cards: convert::remain_cards(deal),
                };
            }
            let mut trump_filter = [0 as c_int; sys::DDS_STRAINS];
            let mut resp = boxed_zeroed::<sys::ddTablesRes>();
            let mut presp = boxed_zeroed::<sys::allParResults>();
            let held_slots = rt.slots.acquire_all_slots();
            // SAFETY: `dealsp`/`resp`/`presp` are heap-allocated, validly sized instances of
            // their DDS struct types, kept alive for the whole call; `trump_filter` is a
            // `[c_int; DDS_STRAINS]` matching the documented `trumpFilter[DDS_STRAINS]` (`0`
            // everywhere: no strain excluded). `CalcAllTables` is documented non-reentrant with
            // the other bulk calls, and also drives DDS's own per-thread-index state
            // internally, so this call holds every slot for its duration.
            let rc = unsafe {
                sys::CalcAllTables(
                    dealsp.as_mut(),
                    -1, // no par calculation (docs/design/10-dds.md §7.1)
                    trump_filter.as_mut_ptr(),
                    resp.as_mut(),
                    presp.as_mut(),
                )
            };
            rt.slots.release_all_slots(held_slots);
            if rc != sys::RETURN_NO_FAULT {
                return Err(dds_error(rc));
            }
            out.extend(resp.results[..chunk.len()].iter().map(table_from_dds));
        }
        Ok(out)
    }

    pub(super) fn solve_board(
        pos: &Position<'_>,
        target: Target,
        solutions: Solutions,
        mode: Mode,
    ) -> Result<FutureTricks, DdsError> {
        let rt = runtime();
        let dl = position_deal(pos)?;
        let slot = rt.slots.acquire_slot();
        let mut fut = zeroed::<sys::futureTricks>();
        // SAFETY: `dl` is passed by value; `fut` is a validly sized `futureTricks` the callee
        // only writes into; `slot` is a thread index this call exclusively holds until it is
        // released just below, matching `SolveBoard`'s per-`threadIndex` reentrancy contract
        // (docs/design/10-dds.md §7.2 rule 2); `acquire_slot` also guarantees no bulk call
        // holds (or is waiting to hold) every slot at the same time (`SlotPool`'s doc comment).
        let rc = unsafe {
            sys::SolveBoard(
                dl,
                target_code(target),
                solutions_code(solutions),
                mode_code(mode),
                &mut fut,
                slot,
            )
        };
        rt.slots.release_slot(slot);
        if rc != sys::RETURN_NO_FAULT {
            return Err(dds_error(rc));
        }
        Ok(future_tricks_from(&fut))
    }

    pub(super) fn solve_all_boards(
        positions: &[(Position<'_>, Target, Solutions, Mode)],
    ) -> Result<Vec<FutureTricks>, DdsError> {
        let rt = runtime();
        let mut out = Vec::with_capacity(positions.len());
        for chunk in positions.chunks(sys::MAXNOOFBOARDS) {
            let mut bop = boxed_zeroed::<sys::boards>();
            bop.noOfBoards = chunk.len() as c_int;
            for (i, (pos, target, solutions, mode)) in chunk.iter().enumerate() {
                bop.deals[i] = position_deal(pos)?;
                bop.target[i] = target_code(*target);
                bop.solutions[i] = solutions_code(*solutions);
                bop.mode[i] = mode_code(*mode);
            }
            let mut solvedp = boxed_zeroed::<sys::solvedBoards>();
            let held_slots = rt.slots.acquire_all_slots();
            // SAFETY: `bop`/`solvedp` are heap-allocated, validly sized `boards`/`solvedBoards`,
            // kept alive for the whole call. `SolveAllChunksBin` is documented non-reentrant
            // with the other bulk calls, and also drives DDS's own per-thread-index state
            // internally, so this call holds every slot for its duration.
            let rc = unsafe { sys::SolveAllChunksBin(bop.as_mut(), solvedp.as_mut(), 1) };
            rt.slots.release_all_slots(held_slots);
            if rc != sys::RETURN_NO_FAULT {
                return Err(dds_error(rc));
            }
            out.extend(
                solvedp.solvedBoard[..chunk.len()]
                    .iter()
                    .map(future_tricks_from),
            );
        }
        Ok(out)
    }

    pub(super) fn analyse_play(deal: &Deal, play: &PlayHistory) -> Result<Vec<u8>, DdsError> {
        let rt = runtime();
        let cards = play.cards();
        if cards.len() > 52 {
            return Err(synthetic_error(
                sys::RETURN_PLAY_FAULT,
                "AnalysePlay input error",
            ));
        }
        let dl = sys::deal {
            trump: convert::strain(play.trump()),
            first: convert::seat(play.leader()),
            currentTrickSuit: [0; 3],
            currentTrickRank: [0; 3],
            remainCards: convert::remain_cards(deal),
        };
        let mut suit = [0 as c_int; 52];
        let mut rank = [0 as c_int; 52];
        for (i, card) in cards.iter().enumerate() {
            suit[i] = convert::suit(card.suit());
            rank[i] = convert::rank(card.rank().index());
        }
        let trace = sys::playTraceBin {
            number: cards.len() as c_int,
            suit,
            rank,
        };
        let slot = rt.slots.acquire_slot();
        let mut solved = zeroed::<sys::solvedPlay>();
        // SAFETY: `dl`/`trace` are passed by value; `solved` is a validly sized `solvedPlay`
        // the callee only writes into; `slot` is exclusively held for this call, matching
        // `AnalysePlayBin`'s per-`thrId` reentrancy contract; `acquire_slot` also guarantees no
        // bulk call holds (or is waiting to hold) every slot at the same time.
        let rc = unsafe { sys::AnalysePlayBin(dl, trace, &mut solved, slot) };
        rt.slots.release_slot(slot);
        if rc != sys::RETURN_NO_FAULT {
            return Err(dds_error(rc));
        }
        let n = solved.number.max(0) as usize;
        Ok(solved.tricks[..n].iter().map(|&t| t.max(0) as u8).collect())
    }

    pub(super) fn dealer_par(
        table: &DdTable,
        dealer: Seat,
        vul: Vulnerability,
    ) -> Result<ParResult, DdsError> {
        let _rt = runtime();
        let mut table_res = zeroed::<sys::ddTableResults>();
        for strain in Strain::ALL {
            let dds_s = convert::strain(strain) as usize;
            for seat in Seat::ALL {
                table_res.resTable[dds_s][convert::seat(seat) as usize] =
                    table.tricks(strain, seat) as c_int;
            }
        }
        let mut pres = zeroed::<sys::parResultsMaster>();
        // SAFETY: `table_res`/`pres` are validly sized local values, kept alive for the call.
        // `DealerParBin` is a pure function of its inputs (docs/design/10-dds.md §7.2 rule 3:
        // "no lock").
        let rc = unsafe {
            sys::DealerParBin(
                &mut table_res,
                &mut pres,
                convert::seat(dealer),
                convert::vulnerability(vul),
            )
        };
        if rc != sys::RETURN_NO_FAULT {
            return Err(dds_error(rc));
        }
        let n = pres.number.max(0) as usize;
        let contracts = pres.contracts[..n]
            .iter()
            .map(convert::format_par_contract)
            .collect();
        Ok(ParResult {
            score: pres.score,
            contracts,
        })
    }

    pub(super) fn info() -> Result<DdsInfo, DdsError> {
        let _rt = runtime();
        let raw = dds_info();
        Ok(DdsInfo {
            version: fixed_str(&raw.versionString),
            threads: raw.noOfThreads.max(0) as u32,
            threading: raw.threading,
            system: fixed_str(&raw.systemString),
        })
    }
}
