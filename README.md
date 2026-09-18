# bridge_engine

A foundation library for contract bridge, in Rust. A single declarative bidding-system
definition (BML) is compiled into an intermediate representation from which both the auction
interpreter (`interpret`) and the bid generator (`choose_bid`) are derived, so that the two can
never drift apart. On top of that sit an exact constraint-satisfying hand sampler, a weighted
deal sampler for double-dummy analysis, and readers for the PBN and LIN record formats.

The design lives in [`docs/design/`](docs/design/README.md) (Japanese); the roadmap and the
numeric completion criteria of each phase are in
[`docs/design/12-roadmap.md`](docs/design/12-roadmap.md).

## Layout

| Crate | Layer | Content |
| --- | --- | --- |
| `bridge-core` | L0 | cards, hands, shapes, seats, calls, auctions, deals, play, text formats |
| `bridge-format` | | PBN 2.1 and BBO LIN readers, PBN writer, deal strings |
| `bridge-eval` | L1 | HCP, controls, losers, quick tricks, distribution points |
| `bridge-constraint` | L1 | the hand-constraint language and the exact sampler |
| `bridge-system` | L2 | BML parser and compiler, `SystemIR`, natural inference |
| `bridge-bidding` | L3 | `interpret` and `choose_bid` |
| `bridge-play` | L5 | hard constraints and carding agreements from the play |
| `bridge-sample` | L4 | weighted deal sampling with pluggable proposals |
| `bridge-dds` | | safe wrapper around the vendored DDS double-dummy solver (native only) |
| `bridge` | | facade re-exporting everything |

## Building and testing

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --workspace --exclude bridge-dds --exclude xtask --target wasm32-unknown-unknown
```

Real data is not committed. Fetch the test corpora (hash-verified, into `corpus/data/`) and the
external BML system files, then run the corpus tests:

```bash
cargo xtask corpus fetch
cargo xtask systems fetch
cargo test -p bridge-format --release -- --ignored --nocapture
```

The double-dummy solver sources are fetched with `cargo xtask dds vendor` (phase 5).

## Status

| Phase | Content | State |
| --- | --- | --- |
| 0 | design documents, workspace skeleton | done |
| 1 | `bridge-core`, `bridge-format`, corpus tooling | done: PBN corpus 724/724 games parsed, round trip exact, LIN 99/99 boards |
| 2 | evaluation and the exact constraint sampler | next |
| 3 | BML compiler, `interpret` / `choose_bid`, bidirectional consistency test | |
| 4 | system definitions (SAYC) | |
| 5 | deal sampler, play inference, DDS | |
| 6 | opening-lead advisor | |

## License

MIT OR Apache-2.0. The vendored DDS sources are Apache-2.0 (Bo Haglund, Soren Hein).
