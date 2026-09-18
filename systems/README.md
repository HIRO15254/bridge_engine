# systems/

Bidding-system definitions in BML (Bridge Bidding Markup Language, gpaulissen/bml syntax with
the extensions documented in `docs/design/06-system.md`).

Production system definitions live outside this repository (users bring their own `.bml`).
This directory holds only what the library's own tests need.

| Path | Content | Committed |
| --- | --- | --- |
| `sayc.bml` | Our own SAYC, written in phase 3 (openings first, responses and competition in phase 4) | yes |
| `fixtures/*.bml` | Small files for unit tests: variables and binding, `#PASTE`, `#SEAT`/`#VUL`, competitive sequences, error recovery | yes |
| `vendor/manifest.toml` | URLs and SHA-256 of external files: gpaulissen/bml test data (`example*.bml` and the expected `.bss` outputs, the expansion oracle), selected real systems from gpaulissen/bridge-systems and jdh8/bridge-systems | yes |
| `vendor/data/` | The fetched files (`cargo xtask systems fetch`) | no (licenses not all verified) |

Tests locate this directory through `BRIDGE_SYSTEMS_DIR` or, by default, relative to the crate
manifest (`../../systems`), and skip (never fail) when vendored files are absent.

Compile a system and inspect its diagnostics with (phase 3):

```bash
cargo run -p bridge-system --example lint -- systems/sayc.bml
```
