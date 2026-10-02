# Vendored DDS sources

`vendor/dds-2.9.0/` is not committed. Fetch it with:

```bash
cargo xtask dds vendor
```

| Field | Value |
| --- | --- |
| Upstream | https://github.com/dds-bridge/dds |
| Tag | `v2.9.0` (2018) |
| License | Apache-2.0 (Bo Haglund, Soren Hein) |
| Layout expected by `build.rs` | `vendor/dds-2.9.0/src/*.cpp`, `vendor/dds-2.9.0/src/*.h`, `vendor/dds-2.9.0/include/dll.h`, `vendor/dds-2.9.0/include/portab.h`, `vendor/dds-2.9.0/LICENSE` |
| Local patches | none. `src/ffi_guard.cpp` reimplements `SetResources` (`bdds_SetResources`) without `System::GetHardware`, whose failure path `exit(1)`s the process; the vendored `Init.cpp` is unchanged (docs/design/10-dds.md §7.3) |
| Archive SHA-256 | recorded by `cargo xtask dds vendor` in `vendor/SHA256SUMS` |

Why 2.9.0 and not DDS3 (v3.x): DDS3 builds only with Bazel and a hermetic LLVM toolchain, and
its legacy C API is deprecated. 2.9.0 is 27 C++11 files with a plain Makefile and the C API every
existing wrapper uses. The wrapper's public surface is written so that switching to DDS3 later is a
`build.rs` change only.

## `cargo publish` (R14, docs/design/12-roadmap.md §10)

Decided (phase 5.5): `Cargo.toml`'s `include` lists `vendor/dds-2.9.0/src/**`,
`vendor/dds-2.9.0/include/**` and `vendor/dds-2.9.0/LICENSE` explicitly, so a published package
carries the extracted sources (~600 KB) despite `vendor/` being git-ignored, and `build.rs` finds
them without a `cargo xtask dds vendor` step. `vendor/dds-2.9.0.tar.gz` and `vendor/SHA256SUMS`
(the other ~14 MB of what `cargo xtask dds vendor` downloads) are deliberately left out: nothing
in `build.rs` reads them, only the already-extracted `src/`/`include`/`LICENSE`. Verify with
`cargo package -p bridge-dds --list` before a release; the vendored files should be present and
the `.tar.gz` should not.

## Stale vendor directory: `../include/portab.h: No such file or directory`

`src/dds.h` includes `../include/portab.h` (the per-OS typedefs, and the `<omp.h>` include when
`-fopenmp` is on), so `vendor/dds-2.9.0/include/portab.h` must exist next to `dll.h`. The
phase-1 version of `cargo xtask dds vendor` extracted only `include/dll.h`; a tree vendored by it
(or a `main` whose nightly job runs it) fails to build with the error above. The current
`xtask` extracts `portab.h` and refuses to finish without it, and `build.rs` treats a tree
without it as "not vendored" (FFI disabled, with a `cargo:warning`) instead of letting the C++
compiler fail. To recover, re-run `cargo xtask dds vendor`; it deletes `vendor/dds-2.9.0/` and
extracts it again from the cached, hash-checked archive.

## The `openmp` feature

By default DDS runs on the `std::thread` backend (`DDS_THREADS_STL`). With `--features openmp`,
`build.rs` additionally defines `DDS_THREADS_OPENMP` (DDS takes the first backend that is
defined, and OpenMP comes before STL) and passes `-fopenmp` (`-Xpreprocessor -fopenmp` plus
`-lomp` on macOS, `/openmp` on MSVC; `libgomp` is linked on Linux). It first compiles and links a
trivial OpenMP program with the same flags; when that fails (Apple clang without Homebrew's
`libomp`, whose keg-only headers and library are on no default search path, so `CPATH` and
`LIBRARY_PATH` would have to point at it) the build prints a `cargo:warning` and falls back to
the STL backend instead of failing.

Only `ubuntu-latest` is known to have a system OpenMP runtime (gcc's `libgomp`). The CI `dds`
job runs `cargo test -p bridge-dds --features openmp` there, and the nightly job reaches the same
build through `--all-features` after `cargo xtask dds vendor`. The other jobs' `--all-features`
runs never vendor DDS first, so their `build.rs` returns before it gets to the OpenMP code.
