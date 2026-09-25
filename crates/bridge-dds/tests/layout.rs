//! Verifies the hand-written `#[repr(C)]` bindings in `sys.rs` against `layout_probe.cpp`,
//! compiled from the vendored `dll.h` (docs/design/10-dds.md §5.1). `sizeof` must match for
//! every struct; the tail field's `offsetof` must also match, which, given `#[repr(C)]`'s
//! sequential-layout rule, implies every field in between is at the same offset too.
#![cfg(dds_vendored)]

use bridge_dds::sys;

#[test]
fn struct_sizes_match_the_cpp_probe() {
    // SAFETY: each `dds_sizeof_*`/`dds_offsetof_*` function takes no arguments and returns a
    // plain `usize`; the FFI declarations in `sys.rs` are checked by this very test.
    unsafe {
        assert_eq!(
            size_of::<sys::deal>(),
            sys::dds_sizeof_deal(),
            "sizeof(deal)"
        );
        assert_eq!(
            core::mem::offset_of!(sys::deal, remainCards),
            sys::dds_offsetof_deal_remainCards(),
            "offsetof(deal, remainCards)"
        );

        assert_eq!(
            size_of::<sys::futureTricks>(),
            sys::dds_sizeof_futureTricks(),
            "sizeof(futureTricks)"
        );
        assert_eq!(
            core::mem::offset_of!(sys::futureTricks, score),
            sys::dds_offsetof_futureTricks_score(),
            "offsetof(futureTricks, score)"
        );

        assert_eq!(
            size_of::<sys::boards>(),
            sys::dds_sizeof_boards(),
            "sizeof(boards)"
        );
        assert_eq!(
            core::mem::offset_of!(sys::boards, mode),
            sys::dds_offsetof_boards_mode(),
            "offsetof(boards, mode)"
        );

        assert_eq!(
            size_of::<sys::solvedBoards>(),
            sys::dds_sizeof_solvedBoards(),
            "sizeof(solvedBoards)"
        );
        assert_eq!(
            size_of::<sys::ddTableDeal>(),
            sys::dds_sizeof_ddTableDeal(),
            "sizeof(ddTableDeal)"
        );
        assert_eq!(
            size_of::<sys::ddTableDeals>(),
            sys::dds_sizeof_ddTableDeals(),
            "sizeof(ddTableDeals)"
        );
        assert_eq!(
            size_of::<sys::ddTableResults>(),
            sys::dds_sizeof_ddTableResults(),
            "sizeof(ddTableResults)"
        );
        assert_eq!(
            size_of::<sys::ddTablesRes>(),
            sys::dds_sizeof_ddTablesRes(),
            "sizeof(ddTablesRes)"
        );
        assert_eq!(
            size_of::<sys::parResults>(),
            sys::dds_sizeof_parResults(),
            "sizeof(parResults)"
        );
        assert_eq!(
            size_of::<sys::allParResults>(),
            sys::dds_sizeof_allParResults(),
            "sizeof(allParResults)"
        );
        assert_eq!(
            size_of::<sys::parResultsMaster>(),
            sys::dds_sizeof_parResultsMaster(),
            "sizeof(parResultsMaster)"
        );
        assert_eq!(
            size_of::<sys::playTraceBin>(),
            sys::dds_sizeof_playTraceBin(),
            "sizeof(playTraceBin)"
        );
        assert_eq!(
            size_of::<sys::playTracesBin>(),
            sys::dds_sizeof_playTracesBin(),
            "sizeof(playTracesBin)"
        );
        assert_eq!(
            size_of::<sys::solvedPlay>(),
            sys::dds_sizeof_solvedPlay(),
            "sizeof(solvedPlay)"
        );
        assert_eq!(
            size_of::<sys::solvedPlays>(),
            sys::dds_sizeof_solvedPlays(),
            "sizeof(solvedPlays)"
        );

        assert_eq!(
            size_of::<sys::DDSInfo>(),
            sys::dds_sizeof_DDSInfo(),
            "sizeof(DDSInfo)"
        );
        assert_eq!(
            core::mem::offset_of!(sys::DDSInfo, systemString),
            sys::dds_offsetof_DDSInfo_systemString(),
            "offsetof(DDSInfo, systemString)"
        );
    }
}
