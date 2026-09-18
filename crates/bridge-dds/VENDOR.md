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
| Layout expected by `build.rs` | `vendor/dds-2.9.0/src/*.cpp`, `vendor/dds-2.9.0/src/*.h`, `vendor/dds-2.9.0/include/dll.h`, `vendor/dds-2.9.0/LICENSE` |
| Local patches | none |
| Archive SHA-256 | recorded by `cargo xtask dds vendor` in `vendor/SHA256SUMS` |

Why 2.9.0 and not DDS3 (v3.x): DDS3 builds only with Bazel and a hermetic LLVM toolchain, and
its legacy C API is deprecated. 2.9.0 is 27 C++11 files with a plain Makefile and the C API every
existing wrapper uses. The wrapper's public surface is written so that switching to DDS3 later is a
`build.rs` change only.
