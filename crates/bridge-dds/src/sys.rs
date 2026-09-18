//! Hand-written bindings to the DDS 2.9.0 legacy C API (`include/dll.h`).
//!
//! Encodings on the DDS side: hands N=0 E=1 S=2 W=3; suits S=0 H=1 D=2 C=3, NT=4; ranks as
//! bits `2..=14` of a `u32` (deuce = bit 2). See `convert.rs` for the mapping to `bridge-core`.
//!
//! `deal` and `playTraceBin` are passed by value as in the header. `boards` is about 250 KB
//! and `solvedBoards` about 60 KB: always heap-allocate them.
//!
//! Verified against the C++ side by `tests/layout.rs` (sizeof/offsetof probes compiled in
//! `layout_probe.cpp`).
#![allow(non_camel_case_types, non_snake_case, missing_docs)]

use core::ffi::{c_char, c_int, c_uint};

pub const DDS_HANDS: usize = 4;
pub const DDS_SUITS: usize = 4;
pub const DDS_STRAINS: usize = 5;
pub const MAXNOOFBOARDS: usize = 200;
pub const MAXNOOFTABLES: usize = 40;

pub const RETURN_NO_FAULT: c_int = 1;
pub const RETURN_UNKNOWN_FAULT: c_int = -1;
pub const RETURN_ZERO_CARDS: c_int = -2;
pub const RETURN_TARGET_TOO_HIGH: c_int = -3;
pub const RETURN_DUPLICATE_CARDS: c_int = -4;
pub const RETURN_TARGET_WRONG_LO: c_int = -5;
pub const RETURN_TARGET_WRONG_HI: c_int = -7;
pub const RETURN_SOLNS_WRONG_LO: c_int = -8;
pub const RETURN_SOLNS_WRONG_HI: c_int = -9;
pub const RETURN_TOO_MANY_CARDS: c_int = -10;
pub const RETURN_SUIT_OR_RANK: c_int = -12;
pub const RETURN_PLAYED_CARD: c_int = -13;
pub const RETURN_CARD_COUNT: c_int = -14;
pub const RETURN_THREAD_INDEX: c_int = -15;
pub const RETURN_MODE_WRONG_LO: c_int = -16;
pub const RETURN_MODE_WRONG_HI: c_int = -17;
pub const RETURN_TRUMP_WRONG: c_int = -18;
pub const RETURN_FIRST_WRONG: c_int = -19;
pub const RETURN_PLAY_FAULT: c_int = -98;
pub const RETURN_PBN_FAULT: c_int = -99;
pub const RETURN_TOO_MANY_BOARDS: c_int = -101;
pub const RETURN_THREAD_CREATE: c_int = -102;
pub const RETURN_THREAD_WAIT: c_int = -103;
pub const RETURN_NO_SUIT: c_int = -201;
pub const RETURN_TOO_MANY_TABLES: c_int = -202;
pub const RETURN_CHUNK_SIZE: c_int = -301;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct deal {
    pub trump: c_int,
    pub first: c_int,
    pub currentTrickSuit: [c_int; 3],
    pub currentTrickRank: [c_int; 3],
    pub remainCards: [[c_uint; DDS_SUITS]; DDS_HANDS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct futureTricks {
    pub nodes: c_int,
    pub cards: c_int,
    pub suit: [c_int; 13],
    pub rank: [c_int; 13],
    pub equals: [c_int; 13],
    pub score: [c_int; 13],
}

#[repr(C)]
pub struct boards {
    pub noOfBoards: c_int,
    pub deals: [deal; MAXNOOFBOARDS],
    pub target: [c_int; MAXNOOFBOARDS],
    pub solutions: [c_int; MAXNOOFBOARDS],
    pub mode: [c_int; MAXNOOFBOARDS],
}

#[repr(C)]
pub struct solvedBoards {
    pub noOfBoards: c_int,
    pub solvedBoard: [futureTricks; MAXNOOFBOARDS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ddTableDeal {
    pub cards: [[c_uint; DDS_SUITS]; DDS_HANDS],
}

#[repr(C)]
pub struct ddTableDeals {
    pub noOfTables: c_int,
    pub deals: [ddTableDeal; MAXNOOFBOARDS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ddTableResults {
    /// `[strain][declarer]` in DDS order.
    pub resTable: [[c_int; DDS_HANDS]; DDS_STRAINS],
}

#[repr(C)]
pub struct ddTablesRes {
    pub noOfBoards: c_int,
    pub results: [ddTableResults; MAXNOOFBOARDS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct parResults {
    pub parScore: [[c_char; 16]; 2],
    pub parContractsString: [[c_char; 128]; 2],
}

#[repr(C)]
pub struct allParResults {
    pub presults: [parResults; MAXNOOFTABLES],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct contractType {
    pub underTricks: c_int,
    pub overTricks: c_int,
    pub level: c_int,
    pub denom: c_int,
    pub seats: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct parResultsMaster {
    pub score: c_int,
    pub number: c_int,
    pub contracts: [contractType; 10],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct playTraceBin {
    pub number: c_int,
    pub suit: [c_int; 52],
    pub rank: [c_int; 52],
}

#[repr(C)]
pub struct playTracesBin {
    pub noOfBoards: c_int,
    pub plays: [playTraceBin; MAXNOOFBOARDS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct solvedPlay {
    pub number: c_int,
    pub tricks: [c_int; 53],
}

#[repr(C)]
pub struct solvedPlays {
    pub noOfBoards: c_int,
    pub solved: [solvedPlay; MAXNOOFBOARDS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DDSInfo {
    pub major: c_int,
    pub minor: c_int,
    pub patch: c_int,
    pub versionString: [c_char; 10],
    pub system: c_int,
    pub numBits: c_int,
    pub compiler: c_int,
    pub constructor: c_int,
    pub numCores: c_int,
    pub threading: c_int,
    pub noOfThreads: c_int,
    pub threadSizes: [c_char; 128],
    pub systemString: [c_char; 1024],
}

// NOTE: 32-bit Windows uses __stdcall for these; unsupported and untested (see docs/design/10-dds.md).
unsafe extern "C" {
    pub fn SetMaxThreads(userThreads: c_int);
    pub fn SetResources(maxMemoryMB: c_int, maxThreads: c_int);
    pub fn SetThreading(code: c_int) -> c_int;
    pub fn FreeMemory();
    pub fn SolveBoard(
        dl: deal,
        target: c_int,
        solutions: c_int,
        mode: c_int,
        futp: *mut futureTricks,
        threadIndex: c_int,
    ) -> c_int;
    pub fn CalcDDtable(tableDeal: ddTableDeal, tablep: *mut ddTableResults) -> c_int;
    pub fn CalcAllTables(
        dealsp: *mut ddTableDeals,
        mode: c_int,
        trumpFilter: *mut c_int,
        resp: *mut ddTablesRes,
        presp: *mut allParResults,
    ) -> c_int;
    pub fn SolveAllChunksBin(
        bop: *mut boards,
        solvedp: *mut solvedBoards,
        chunkSize: c_int,
    ) -> c_int;
    pub fn DealerParBin(
        tablep: *mut ddTableResults,
        presp: *mut parResultsMaster,
        dealer: c_int,
        vulnerable: c_int,
    ) -> c_int;
    pub fn AnalysePlayBin(
        dl: deal,
        play: playTraceBin,
        solved: *mut solvedPlay,
        thrId: c_int,
    ) -> c_int;
    pub fn AnalyseAllPlaysBin(
        bop: *mut boards,
        plp: *mut playTracesBin,
        solvedp: *mut solvedPlays,
        chunkSize: c_int,
    ) -> c_int;
    pub fn GetDDSInfo(info: *mut DDSInfo);
    pub fn ErrorMessage(code: c_int, line: *mut c_char);

    // layout_probe.cpp
    pub fn dds_sizeof_deal() -> usize;
    pub fn dds_offsetof_deal_remainCards() -> usize;
    pub fn dds_sizeof_futureTricks() -> usize;
    pub fn dds_offsetof_futureTricks_score() -> usize;
    pub fn dds_sizeof_boards() -> usize;
    pub fn dds_offsetof_boards_mode() -> usize;
    pub fn dds_sizeof_solvedBoards() -> usize;
    pub fn dds_sizeof_ddTableDeal() -> usize;
    pub fn dds_sizeof_ddTableDeals() -> usize;
    pub fn dds_sizeof_ddTableResults() -> usize;
    pub fn dds_sizeof_ddTablesRes() -> usize;
    pub fn dds_sizeof_parResults() -> usize;
    pub fn dds_sizeof_allParResults() -> usize;
    pub fn dds_sizeof_parResultsMaster() -> usize;
    pub fn dds_sizeof_playTraceBin() -> usize;
    pub fn dds_sizeof_playTracesBin() -> usize;
    pub fn dds_sizeof_solvedPlay() -> usize;
    pub fn dds_sizeof_solvedPlays() -> usize;
    pub fn dds_sizeof_DDSInfo() -> usize;
    pub fn dds_offsetof_DDSInfo_systemString() -> usize;
}
