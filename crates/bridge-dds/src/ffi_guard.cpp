// Exception barrier between DDS and Rust.
//
// DDS is C++ and does not catch anything itself: an allocation failure (`std::bad_alloc` from
// the transposition table's or `Memory`'s containers) or a thread-creation failure
// (`std::system_error` from the STL threading backend) would otherwise unwind straight into
// the Rust caller through an `extern "C"` declaration, which is undefined behaviour. Every
// entry point `lib.rs` calls goes through one of these `noexcept` wrappers instead, which turn
// any C++ exception into DDS's own `RETURN_UNKNOWN_FAULT` (-1), so the Rust side sees an
// ordinary `DdsError::Code`.
//
// Compiled only when the DDS sources are vendored (see build.rs).

#include "dll.h"

extern "C" {

int bdds_SetResources(int maxMemoryMB, int maxThreads) noexcept
{
  try
  {
    SetResources(maxMemoryMB, maxThreads);
    return RETURN_NO_FAULT;
  }
  catch (...)
  {
    return RETURN_UNKNOWN_FAULT;
  }
}

int bdds_GetDDSInfo(DDSInfo * info) noexcept
{
  try
  {
    GetDDSInfo(info);
    return RETURN_NO_FAULT;
  }
  catch (...)
  {
    return RETURN_UNKNOWN_FAULT;
  }
}

int bdds_SolveBoard(
  deal dl,
  int target,
  int solutions,
  int mode,
  futureTricks * futp,
  int threadIndex) noexcept
{
  try
  {
    return SolveBoard(dl, target, solutions, mode, futp, threadIndex);
  }
  catch (...)
  {
    return RETURN_UNKNOWN_FAULT;
  }
}

int bdds_CalcDDtable(ddTableDeal tableDeal, ddTableResults * tablep) noexcept
{
  try
  {
    return CalcDDtable(tableDeal, tablep);
  }
  catch (...)
  {
    return RETURN_UNKNOWN_FAULT;
  }
}

int bdds_CalcAllTables(
  ddTableDeals * dealsp,
  int mode,
  int trumpFilter[5],
  ddTablesRes * resp,
  allParResults * presp) noexcept
{
  try
  {
    return CalcAllTables(dealsp, mode, trumpFilter, resp, presp);
  }
  catch (...)
  {
    return RETURN_UNKNOWN_FAULT;
  }
}

int bdds_SolveAllChunksBin(
  boards * bop,
  solvedBoards * solvedp,
  int chunkSize) noexcept
{
  try
  {
    return SolveAllChunksBin(bop, solvedp, chunkSize);
  }
  catch (...)
  {
    return RETURN_UNKNOWN_FAULT;
  }
}

int bdds_DealerParBin(
  ddTableResults * tablep,
  parResultsMaster * presp,
  int dealer,
  int vulnerable) noexcept
{
  try
  {
    return DealerParBin(tablep, presp, dealer, vulnerable);
  }
  catch (...)
  {
    return RETURN_UNKNOWN_FAULT;
  }
}

int bdds_AnalysePlayBin(
  deal dl,
  playTraceBin play,
  solvedPlay * solved,
  int thrId) noexcept
{
  try
  {
    return AnalysePlayBin(dl, play, solved, thrId);
  }
  catch (...)
  {
    return RETURN_UNKNOWN_FAULT;
  }
}

} // extern "C"
