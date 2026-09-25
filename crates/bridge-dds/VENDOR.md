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
| Local patches | none |
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
