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

// `std::min`/`std::max` are called as `(std::min)(...)`: on Windows `dll.h` pulls in
// `<windows.h>` (through `portab.h`), whose `min`/`max` macros would otherwise expand.
#include <algorithm>

#include "dll.h"
#include "Memory.h"
#include "Scheduler.h"
#include "System.h"
#include "ThreadMgr.h"

// DDS's own globals and helpers, defined (non-static) in the vendored Init.cpp; declared here
// so `bdds_SetResources` can drive them without patching the vendored sources.
extern System sysdep;
extern Memory memory;
extern Scheduler scheduler;
extern ThreadMgr threadMgr;
extern int _initialized;
void InitConstants();
void InitDebugFiles();

namespace
{

// Init.cpp's `SetResources` without the hardware probe (see bdds_SetResources below).
void set_resources_without_probe(int maxMemoryMB, int maxThreadsIn, int ncores)
{
  if (ncores < 1)
    ncores = 1;

  // Memory: the caller's value + 30% (as upstream), capped at 1800 MB on 32-bit systems, and
  // never below one small thread's worth. Upstream also caps at 70% of the probed free memory;
  // that cap is dropped together with the probe, so the caller's value is the only bound.
  int memMaxMB = (maxMemoryMB <= 0 ? 1000000 :
    static_cast<int>(1.3 * maxMemoryMB));
  if (sizeof(void *) == 4)
    memMaxMB = (std::min)(memMaxMB, 1800);
  memMaxMB = (std::max)(memMaxMB, THREADMEM_SMALL_MAX_MB);

  int thrMax;
  if (sysdep.IsSingleThreaded())
    thrMax = 1;
  else if (sysdep.IsIMPL() || maxThreadsIn <= 0)
    thrMax = ncores;
  else
    thrMax = (std::min)(maxThreadsIn, ncores);

  int noOfThreads, noOfLargeThreads, noOfSmallThreads;
  if (thrMax * THREADMEM_LARGE_MAX_MB <= memMaxMB)
  {
    noOfThreads = thrMax;
    noOfLargeThreads = thrMax;
    noOfSmallThreads = 0;
  }
  else if (thrMax * THREADMEM_SMALL_MAX_MB > memMaxMB)
  {
    noOfThreads = static_cast<int>(memMaxMB /
      static_cast<double>(THREADMEM_SMALL_MAX_MB));
    noOfLargeThreads = 0;
    noOfSmallThreads = noOfThreads;
  }
  else
  {
    const double d = static_cast<double>(
          THREADMEM_LARGE_MAX_MB - THREADMEM_SMALL_MAX_MB);
    noOfThreads = thrMax;
    noOfLargeThreads = static_cast<int>(
      (memMaxMB - thrMax * THREADMEM_SMALL_MAX_MB) / d);
    noOfSmallThreads = thrMax - noOfLargeThreads;
  }
  // Unreachable given the clamp above, but this is what keeps `Memory::GetPtr(0)` in
  // `InitDebugFiles` (and every later call) from hitting its `exit(1)`: at least one thread.
  if (noOfThreads < 1)
  {
    noOfThreads = 1;
    noOfLargeThreads = 0;
    noOfSmallThreads = 1;
  }

  sysdep.RegisterParams(noOfThreads, memMaxMB);
  scheduler.RegisterThreads(noOfThreads);

  memory.Resize(0, DDS_TT_SMALL, 0, 0);
  if (noOfLargeThreads > 0)
    memory.Resize(static_cast<unsigned>(noOfLargeThreads),
      DDS_TT_LARGE, THREADMEM_LARGE_DEF_MB, THREADMEM_LARGE_MAX_MB);
  if (noOfSmallThreads > 0)
    memory.Resize(static_cast<unsigned>(noOfThreads),
      DDS_TT_SMALL, THREADMEM_SMALL_DEF_MB, THREADMEM_SMALL_MAX_MB);

  threadMgr.Reset(noOfThreads);

  InitDebugFiles();

  if (! _initialized)
  {
    _initialized = 1;
    InitConstants();
  }
}

} // namespace

extern "C" {

// Replaces DDS's `SetResources` (Init.cpp) for this crate. Upstream always calls
// `System::GetHardware`, which shells out to `sysctl` (macOS) or `free | tail | awk` (Linux) and
// caps memory at 70% of what it reads. When the probe fails or reads 0 (no `sysctl` on PATH, a
// sandbox, a Linux host without swap where the pipeline reads the Swap line, a missing `free`),
// the cap is 0, DDS sizes itself to 0 threads and the next `Memory::GetPtr(0)` calls `exit(1)`,
// terminating the host process; a failed `popen` even dereferences NULL. This body is Init.cpp's
// with the probe removed: `ncores` comes from the caller (Rust's `available_parallelism`) and
// the thread count is clamped to at least 1. The vendored sources stay unmodified.
int bdds_SetResources(int maxMemoryMB, int maxThreads, int ncores) noexcept
{
  try
  {
    set_resources_without_probe(maxMemoryMB, maxThreads, ncores);
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
