# systems/

Bidding-system definitions in BML (Bridge Bidding Markup Language, gpaulissen/bml syntax with
our extensions). The dialect the compiler accepts -- file structure, call tokens, the description
vocabulary, `#+KEY:` meta lines, `{prio:N}`/`{w:X}`/`{stop}`, resolution semantics, every lint,
the full EBNF and worked examples -- is specified in `docs/design/16-extended-bml.md` (in
Japanese); start there when writing a new system file.

Production system definitions live outside this repository (users bring their own `.bml`).
This directory holds only what the library's own tests need.

| Path | Content | Committed |
| --- | --- | --- |
| `sayc/sayc.bml` | Our own SAYC (root, `#INCLUDE`s the parts below): openings, notrump, the strong 2!c, weak twos, preempts, responses, opener's rebids and competitive bidding, all written in phase 3 | yes |
| `sayc/openings-only.bml` | An alternate root that `#INCLUDE`s only the opening-bid parts (`openings.bml`, `notrump.bml`, `strong-2c.bml`, `weak-twos.bml`, `preempts.bml`). Used by the phase 3 forward-consistency harness, which only needs opener's first call to be well defined for every hand | yes |
| `sayc/{openings,notrump,strong-2c,weak-twos,preempts,responses-major,responses-minor,rebids,competition}.bml` | The parts, one per booklet section: suit openings; 1NT/2NT/3NT and their responses; the strong artificial 2!c; weak two-bids; opening preempts; responses to a major/minor opening; opener's rebids; overcalls, 1NT overcalls, weak jump overcalls, takeout and negative doubles, Michaels, the unusual notrump, responses to them, and balancing | yes |
| `sayc/NOTES.md` | Where the ACBL SAYC System Booklet (revised January 2006) leaves room for partnership judgment, or where the v1 description vocabulary (`docs/design/06-system.md` §7.4) cannot state exactly what the booklet says, and the choice this file makes in each case | yes |
| `fixtures/*.bml` | Small files for unit tests: variables and binding, `#PASTE`, `#SEAT`/`#VUL`, competitive sequences, error recovery | yes |
| `vendor/manifest.toml` | URLs and SHA-256 of external files: gpaulissen/bml test data (`example*.bml` and the expected `.bss` outputs, the expansion oracle), selected real systems from gpaulissen/bridge-systems and jdh8/bridge-systems | yes |
| `vendor/data/` | The fetched files (`cargo xtask systems fetch`) | no (licenses not all verified) |

Tests locate this directory through `BRIDGE_SYSTEMS_DIR` or, by default, relative to the crate
manifest (`../../systems`), and skip (never fail) when vendored files are absent.

There is no command-line lint runner. To inspect a system's diagnostics, compile it with
`bridge_system::compile` and print each `Lint` (`file:line: severity[Code]: message`):

```rust
use bridge_system::{CompileOptions, compile, lexer::FsLoader};

let path = "systems/sayc/sayc.bml";
let text = std::fs::read_to_string(path)?;
let (_ir, lints) = compile(path, &text, &FsLoader, &CompileOptions::default());
for lint in &lints {
    println!("{lint}");
}
```

`docs/design/16-extended-bml.md` §8 lists every lint code with its trigger and fix.
