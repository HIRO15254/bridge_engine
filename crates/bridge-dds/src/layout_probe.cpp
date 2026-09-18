// Exposes sizeof/offsetof of the DDS structs so that tests/layout.rs can verify the
// hand-written #[repr(C)] declarations in sys.rs without bindgen.
//
// Compiled only when the DDS sources are vendored (see build.rs).

#include <cstddef>
#include "dll.h"

extern "C" {

size_t dds_sizeof_deal() { return sizeof(deal); }
size_t dds_offsetof_deal_remainCards() { return offsetof(deal, remainCards); }
size_t dds_sizeof_futureTricks() { return sizeof(futureTricks); }
size_t dds_offsetof_futureTricks_score() { return offsetof(futureTricks, score); }
size_t dds_sizeof_boards() { return sizeof(boards); }
size_t dds_offsetof_boards_mode() { return offsetof(boards, mode); }
size_t dds_sizeof_solvedBoards() { return sizeof(solvedBoards); }
size_t dds_sizeof_ddTableDeal() { return sizeof(ddTableDeal); }
size_t dds_sizeof_ddTableDeals() { return sizeof(ddTableDeals); }
size_t dds_sizeof_ddTableResults() { return sizeof(ddTableResults); }
size_t dds_sizeof_ddTablesRes() { return sizeof(ddTablesRes); }
size_t dds_sizeof_parResults() { return sizeof(parResults); }
size_t dds_sizeof_allParResults() { return sizeof(allParResults); }
size_t dds_sizeof_parResultsMaster() { return sizeof(parResultsMaster); }
size_t dds_sizeof_playTraceBin() { return sizeof(playTraceBin); }
size_t dds_sizeof_playTracesBin() { return sizeof(playTracesBin); }
size_t dds_sizeof_solvedPlay() { return sizeof(solvedPlay); }
size_t dds_sizeof_solvedPlays() { return sizeof(solvedPlays); }
size_t dds_sizeof_DDSInfo() { return sizeof(DDSInfo); }
size_t dds_offsetof_DDSInfo_systemString() { return offsetof(DDSInfo, systemString); }

}
