# 10. `bridge-dds`: DDS v2.9.0 の FFI ラッパー

本文書は D10 の決定、すなわち DDS (Bo Haglund, Soren Hein) の v2.9.0 を `cc` でビルドし、手書きの `#[repr(C)]` バインディングを C++ 側の `sizeof`/`offsetof` プローブで検証する方式を確定する。DDS のソースは `crates/bridge-dds/vendor/` に置くが git には含めず `cargo xtask dds vendor` で取得し、未取得でも `cargo:warning` を出してワークスペース全体のビルドは通す。安全ラッパーは `SolveBoard` の `thrId` スロットで Rust スレッド間の並行を許し、非再入のバルク関数は `Mutex` で直列化する。

## 1. 検証済みの DDS の事実

2026-09-18 時点で `https://github.com/dds-bridge/dds` を確認した結果。

| 項目 | v2.9.0 (2018) | DDS3 v3.0.0 (2026-05) / v3.1.0 (2026-08) |
| --- | --- | --- |
| ビルドシステム | Makefile (`Makefile_linux_shared` 等)。`src/*.cpp` 27 ファイル + `src/*.h` + `include/dll.h`、C++11 | Bazel + hermetic LLVM 21 のみ。ソースは 8 サブディレクトリに分割され、include パスと define は `BUILD.bazel` が管理 |
| ライセンス | Apache-2.0 | Apache-2.0 |
| レガシー C API | 現行 API (`dll.h`) | `library/src/api/dll.h` に存在。全関数が `@deprecated`、"supported indefinitely for binary compatibility"。構造体 typedef が大文字化されているが C ABI は同一 |
| `SetMaxThreads` | 有効 | no-op |
| WASM | なし | Emscripten ビルドあり (`wasm32-emscripten` の将来経路) |
| 席のエンコード | N=0, E=1, S=2, W=3 | 同 |
| スートのエンコード | S=0, H=1, D=2, C=3, NT=4 | 同 |
| ランクのエンコード | bit 2..14 (2 = bit 2, A = bit 14) | 同 |
| `MAXNOOFBOARDS` | 200 | 同 |
| `MAXNOOFTABLES` | 40 | 同 |
| 再入性 | `SolveBoard` / `AnalysePlayBin` は `thrId` ごとに再入可。`SolveAllBoards`/`SolveAllChunksBin`、`CalcAllTables`、`AnalyseAllPlaysBin` は非再入 | 同 |
| スレッドバックエンド | マクロ `DDS_THREADS_BOOST` / `_GCD` / `_OPENMP` / `_WINAPI` / `_STL` のいずれか。無指定なら単一スレッド | Bazel 管理 |
| メモリ検出 | `System.cpp` が macOS で `popen("sysctl -n hw.memsize")`、Linux で `popen("free -k …")` | |

## 2. 決定 D10 と選択肢

| 選択肢 | 判定 | 理由 |
| --- | --- | --- |
| DDS3 (v3.1.0) のレガシー C API を `cc` でビルド | 不採用 (v1) | Bazel と hermetic LLVM 21 専用。`cc` で組むには `BUILD.bazel` の include/define を逆解析する必要がある。`SetMaxThreads` が no-op、全関数が `@deprecated` |
| **DDS v2.9.0 をベンダリングして `cc` でビルド** | **採用** | 1 つの `src/` に 27 `.cpp`、`include/dll.h`、C++11、Apache-2.0。既存ラッパーが皆使う API。ラッパーの公開面は DDS3 レガシー名とも同じシグネチャなので、将来の切替は `vendor/` と `build.rs` の差し替えで済む |
| git submodule | 不採用 | `cargo publish` / `cargo vendor` を壊す。上流 `develop` にはもう旧ツリーがない |
| ビルド時 `bindgen` | 不採用 (必須依存として) | 利用者全員と CI イメージに libclang を要求する。ヘッダは構造体約 25 個・関数約 35 個で 2014 年から不変。`bindgen` 0.73 は `cargo xtask dds regen-bindings` のオフライン生成にだけ使い、手書き `sys.rs` をレイアウトテストで守る |

## 3. ベンダリング配置と `VENDOR.md`

```
crates/bridge-dds/
├── Cargo.toml              # build = "build.rs", links = "dds", features: openmp (default = [])
├── VENDOR.md               # コミットする: 上流、タグ、ライセンス、期待するレイアウト、ローカルパッチ (none)、sha の記録先
├── build.rs
├── vendor/                 # git-ignored。`cargo xtask dds vendor` が作る
│   ├── SHA256SUMS          # 取得したアーカイブの sha256 (xtask が書く)
│   └── dds-2.9.0/
│       ├── include/dll.h
│       ├── src/*.cpp (27)  src/*.h
│       └── LICENSE
├── src/
│   ├── lib.rs              # 公開型と安全ラッパー関数、DdsError、wasm32 は compile_error!、`#[cfg(dds_vendored)] pub mod sys;`
│   ├── sys.rs              # 手書き #[repr(C)] と unsafe extern (cfg(dds_vendored) のときだけコンパイル)
│   ├── convert.rs          # core <-> DDS エンコード変換
│   └── layout_probe.cpp    # sizeof/offsetof プローブ (build.rs が DDS と一緒にコンパイル)
└── tests/                  # フェーズ 5
    ├── layout.rs           # sizeof/offsetof の突き合わせ
    ├── differential.rs     # list100.txt (masterDD.txt は --ignored)
    └── concurrency.rs      # 8 スレッド × 100 SolveBoard
```

`.gitignore` の `/crates/bridge-dds/vendor/` により DDS ソースはリポジトリに入らない。`VENDOR.md` (骨格でコミット済み) の内容:

| 項目 | 値 |
| --- | --- |
| Upstream | `https://github.com/dds-bridge/dds` |
| Tag | `v2.9.0` (2018) |
| License | Apache-2.0 (Bo Haglund, Soren Hein) |
| `build.rs` が期待するレイアウト | `vendor/dds-2.9.0/src/*.cpp`, `vendor/dds-2.9.0/src/*.h`, `vendor/dds-2.9.0/include/dll.h`, `vendor/dds-2.9.0/LICENSE` |
| ローカルパッチ | なし |
| アーカイブ sha256 | `cargo xtask dds vendor` が `vendor/SHA256SUMS` に記録する |

`VENDOR.md` には DDS3 を採らない理由 (§2) も書いてある。ランタイムの状態 (`Runtime`、スレッドスロット) は `lib.rs` の非公開項目で、`api.rs` / `runtime.rs` / `stub.rs` のようなモジュール分割はしない: 未ベンダリング時は同じ関数が `DdsError::Unavailable` を返すだけなので、`#[cfg(dds_vendored)]` は関数本体の中に置く。

未決: `cargo publish` 時に `vendor/` を同梱する方法 (`.gitignore` されたファイルは既定でパッケージに入らないため `include` を明示するか、公開前に取得を必須にする)。フェーズ 5.5 で決める。

## 4. `build.rs`

要件: 27 個の `.cpp` を `-std=c++11 -O3` でビルド、`DDS_THREADS_STL` を定義 (std::thread、外部依存なし、MSVC/clang/gcc で動く)、feature `openmp` で `DDS_THREADS_OPENMP`、`wasm32` は何もしない (`lib.rs` が `compile_error!`)、`vendor/` 未取得なら警告して FFI を cfg で除外する (フェーズ 0 で確定済み)。

```rust
//! Compiles the vendored DDS 2.9.0 sources with `cc`.
//!
//! The sources are not committed; `cargo xtask dds vendor` downloads them into
//! `vendor/dds-2.9.0/`. When they are absent the crate still builds, with the FFI layer
//! compiled out (`cfg(dds_vendored)` unset) so that the workspace checks on every machine.
use std::path::Path;

const SOURCES: &[&str] = &[
    "dds", "dump", "ABsearch", "ABstats", "CalcTables", "DealerPar", "File", "Init", "LaterTricks",
    "Memory", "Moves", "Par", "PlayAnalyser", "PBN", "QuickTricks", "Scheduler", "SolveBoard", "SolverIF",
    "System", "ThreadMgr", "Timer", "TimerGroup", "TimerList", "TimeStat", "TimeStatList", "TransTableS",
    "TransTableL",
];

fn main() {
    println!("cargo:rustc-check-cfg=cfg(dds_vendored)");
    println!("cargo:rerun-if-changed=vendor/dds-2.9.0");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        return; // lib.rs emits a compile_error! for wasm32
    }

    let src = Path::new("vendor/dds-2.9.0/src");
    if !src.join("dds.cpp").exists() {
        println!("cargo:warning=bridge-dds: DDS sources not vendored; run `cargo xtask dds vendor` (FFI disabled)");
        return;
    }

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++11")
        .opt_level(3)
        .include("vendor/dds-2.9.0/include")
        .include(src)
        .files(SOURCES.iter().map(|f| src.join(format!("{f}.cpp"))))
        .file("src/layout_probe.cpp")
        .define("DDS_THREADS_STL", None)
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-deprecated-declarations");
    if std::env::var("CARGO_FEATURE_OPENMP").is_ok() {
        build.define("DDS_THREADS_OPENMP", None).flag("-fopenmp");
        println!("cargo:rustc-link-lib=gomp");
    }
    build.compile("dds");
    println!("cargo:rustc-cfg=dds_vendored");
}
```

補足:

- `DDS_THREADS_STL` は 2.9.0 の `Makefile_linux_shared` にあるバックエンドの 1 つ。マクロ無指定だと DDS は単一スレッドになる。
- `openmp` は gcc (`-fopenmp`, `libgomp`) を前提にし、CI では Linux のみ有効化する。未決: MSVC (`/openmp`) と Apple clang (`libomp`) の対応。
- Windows: `dll.h` の `DLLEXPORT` は `__declspec(dllexport)` だが静的リンクなので問題ない。`STDCALL = __stdcall` は 32 bit x86 でのみ意味を持つが、`sys.rs` は `extern "C"` だけを宣言し、x86 Windows は「未検証・未サポート」と明記する (R6。`sys.rs` の NOTE コメント)。
- edition 2024 では `unsafe extern "C" { … }` ブロック。
- `wasm32-unknown-unknown` には libc もスレッドもないため `lib.rs` で `compile_error!`。ファサードは `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]` で本クレートを optional に持つ。
- `links = "dds"` を `Cargo.toml` に書き、同名ネイティブライブラリの二重リンクを Cargo に検出させる。
- ベンダリング先を環境変数で差し替える仕組みは持たない (`vendor/dds-2.9.0` 固定)。

`lib.rs` の先頭:

```rust
#![warn(missing_docs)]                       // unsafe を含むので forbid(unsafe_code) は付けない
#[cfg(target_arch = "wasm32")]
compile_error!("bridge-dds is native only; disable the `dds` feature of `bridge` for wasm32");

pub mod convert;
#[cfg(dds_vendored)]
pub mod sys;
pub use bridge_core::DdTable;

pub const fn is_available() -> bool { cfg!(dds_vendored) }
```

## 5. `sys.rs`: 手書き `#[repr(C)]` バインディング

`dll.h` (v2.9.0) を写した宣言。フィールド名は C 側と同じ (`#[allow(non_snake_case, non_camel_case_types, missing_docs)]`)。

```rust
use core::ffi::{c_char, c_int, c_uint};

pub const DDS_HANDS: usize = 4;
pub const DDS_SUITS: usize = 4;
pub const DDS_STRAINS: usize = 5;
pub const MAXNOOFBOARDS: usize = 200;
pub const MAXNOOFTABLES: usize = 40;

pub const RETURN_NO_FAULT: c_int = 1;
// Negative codes are transcribed from dll.h (-1 … -301):
pub const RETURN_UNKNOWN_FAULT: c_int = -1;
pub const RETURN_ZERO_CARDS: c_int = -2;
pub const RETURN_TARGET_TOO_HIGH: c_int = -3;
pub const RETURN_DUPLICATE_CARDS: c_int = -4;
pub const RETURN_TARGET_WRONG_LO: c_int = -5;
pub const RETURN_TARGET_WRONG_HI: c_int = -7;
pub const RETURN_SOLNS_WRONG_LO: c_int = -8;
pub const RETURN_SOLNS_WRONG_HI: c_int = -9;
pub const RETURN_TOO_MANY_CARDS: c_int = -10;
pub const RETURN_SUIT_OR_RANK: c_int = -12;
pub const RETURN_PLAYED_CARD: c_int = -13;
pub const RETURN_CARD_COUNT: c_int = -14;
pub const RETURN_THREAD_INDEX: c_int = -15;
pub const RETURN_MODE_WRONG_LO: c_int = -16;
pub const RETURN_MODE_WRONG_HI: c_int = -17;
pub const RETURN_TRUMP_WRONG: c_int = -18;
pub const RETURN_FIRST_WRONG: c_int = -19;
pub const RETURN_PLAY_FAULT: c_int = -98;
pub const RETURN_PBN_FAULT: c_int = -99;
pub const RETURN_TOO_MANY_BOARDS: c_int = -101;
pub const RETURN_THREAD_CREATE: c_int = -102;
pub const RETURN_THREAD_WAIT: c_int = -103;
pub const RETURN_NO_SUIT: c_int = -201;
pub const RETURN_TOO_MANY_TABLES: c_int = -202;
pub const RETURN_CHUNK_SIZE: c_int = -301;

#[repr(C)] #[derive(Clone, Copy)] pub struct deal {
    pub trump: c_int,                       // 0 S, 1 H, 2 D, 3 C, 4 NT
    pub first: c_int,                       // leader of the current trick: 0 N, 1 E, 2 S, 3 W
    pub currentTrickSuit: [c_int; 3],       // cards already played to the current trick, in play order
    pub currentTrickRank: [c_int; 3],       // 2..14; unused entries 0
    pub remainCards: [[c_uint; DDS_SUITS]; DDS_HANDS],   // [hand][suit], bits 2..14
}
#[repr(C)] #[derive(Clone, Copy)] pub struct futureTricks {
    pub nodes: c_int, pub cards: c_int,
    pub suit: [c_int; 13], pub rank: [c_int; 13], pub equals: [c_int; 13], pub score: [c_int; 13],
}
#[repr(C)] pub struct boards {
    pub noOfBoards: c_int, pub deals: [deal; MAXNOOFBOARDS],
    pub target: [c_int; MAXNOOFBOARDS], pub solutions: [c_int; MAXNOOFBOARDS], pub mode: [c_int; MAXNOOFBOARDS],
}
#[repr(C)] pub struct solvedBoards { pub noOfBoards: c_int, pub solvedBoard: [futureTricks; MAXNOOFBOARDS] }
#[repr(C)] #[derive(Clone, Copy)] pub struct ddTableDeal { pub cards: [[c_uint; DDS_SUITS]; DDS_HANDS] }
#[repr(C)] pub struct ddTableDeals { pub noOfTables: c_int, pub deals: [ddTableDeal; MAXNOOFBOARDS] }          // 200 = MAXNOOFTABLES × DDS_STRAINS
#[repr(C)] #[derive(Clone, Copy)] pub struct ddTableResults { pub resTable: [[c_int; DDS_HANDS]; DDS_STRAINS] }   // [strain][declarer]
#[repr(C)] pub struct ddTablesRes { pub noOfBoards: c_int, pub results: [ddTableResults; MAXNOOFBOARDS] }
#[repr(C)] #[derive(Clone, Copy)] pub struct parResults { pub parScore: [[c_char; 16]; 2], pub parContractsString: [[c_char; 128]; 2] }
#[repr(C)] pub struct allParResults { pub presults: [parResults; MAXNOOFTABLES] }                                 // 40
#[repr(C)] #[derive(Clone, Copy)] pub struct contractType { pub underTricks: c_int, pub overTricks: c_int, pub level: c_int, pub denom: c_int, pub seats: c_int }
#[repr(C)] #[derive(Clone, Copy)] pub struct parResultsMaster { pub score: c_int, pub number: c_int, pub contracts: [contractType; 10] }
#[repr(C)] #[derive(Clone, Copy)] pub struct playTraceBin { pub number: c_int, pub suit: [c_int; 52], pub rank: [c_int; 52] }
#[repr(C)] pub struct playTracesBin { pub noOfBoards: c_int, pub plays: [playTraceBin; MAXNOOFBOARDS] }
#[repr(C)] #[derive(Clone, Copy)] pub struct solvedPlay { pub number: c_int, pub tricks: [c_int; 53] }
#[repr(C)] pub struct solvedPlays { pub noOfBoards: c_int, pub solved: [solvedPlay; MAXNOOFBOARDS] }
#[repr(C)] #[derive(Clone, Copy)] pub struct DDSInfo {
    pub major: c_int, pub minor: c_int, pub patch: c_int, pub versionString: [c_char; 10],
    pub system: c_int, pub numBits: c_int, pub compiler: c_int, pub constructor: c_int,
    pub numCores: c_int, pub threading: c_int, pub noOfThreads: c_int,
    pub threadSizes: [c_char; 128], pub systemString: [c_char; 1024],
}

// `cc` emits the `cargo:rustc-link-lib=static=dds` line, so no `#[link]` attribute is needed.
// NOTE: 32-bit Windows uses __stdcall for these; unsupported and untested.
unsafe extern "C" {
    pub fn SetMaxThreads(userThreads: c_int);
    pub fn SetResources(maxMemoryMB: c_int, maxThreads: c_int);
    pub fn SetThreading(code: c_int) -> c_int;
    pub fn FreeMemory();
    pub fn SolveBoard(dl: deal, target: c_int, solutions: c_int, mode: c_int, futp: *mut futureTricks, threadIndex: c_int) -> c_int;
    pub fn CalcDDtable(tableDeal: ddTableDeal, tablep: *mut ddTableResults) -> c_int;
    pub fn CalcAllTables(dealsp: *mut ddTableDeals, mode: c_int, trumpFilter: *mut c_int /* [5] */, resp: *mut ddTablesRes, presp: *mut allParResults) -> c_int;
    pub fn SolveAllChunksBin(bop: *mut boards, solvedp: *mut solvedBoards, chunkSize: c_int) -> c_int;
    pub fn DealerParBin(tablep: *mut ddTableResults, presp: *mut parResultsMaster, dealer: c_int, vulnerable: c_int) -> c_int;
    pub fn AnalysePlayBin(dl: deal, play: playTraceBin, solved: *mut solvedPlay, thrId: c_int) -> c_int;
    pub fn AnalyseAllPlaysBin(bop: *mut boards, plp: *mut playTracesBin, solvedp: *mut solvedPlays, chunkSize: c_int) -> c_int;
    pub fn GetDDSInfo(info: *mut DDSInfo);
    pub fn ErrorMessage(code: c_int, line: *mut c_char /* [80] */);

    // layout_probe.cpp
    pub fn dds_sizeof_deal() -> usize;
    pub fn dds_offsetof_deal_remainCards() -> usize;
    pub fn dds_sizeof_futureTricks() -> usize;
    pub fn dds_offsetof_futureTricks_score() -> usize;
    pub fn dds_sizeof_boards() -> usize;
    pub fn dds_offsetof_boards_mode() -> usize;
    pub fn dds_sizeof_solvedBoards() -> usize;
    pub fn dds_sizeof_ddTableDeal() -> usize;
    pub fn dds_sizeof_ddTableDeals() -> usize;
    pub fn dds_sizeof_ddTableResults() -> usize;
    pub fn dds_sizeof_ddTablesRes() -> usize;
    pub fn dds_sizeof_parResults() -> usize;
    pub fn dds_sizeof_allParResults() -> usize;
    pub fn dds_sizeof_parResultsMaster() -> usize;
    pub fn dds_sizeof_playTraceBin() -> usize;
    pub fn dds_sizeof_playTracesBin() -> usize;
    pub fn dds_sizeof_solvedPlay() -> usize;
    pub fn dds_sizeof_solvedPlays() -> usize;
    pub fn dds_sizeof_DDSInfo() -> usize;
    pub fn dds_offsetof_DDSInfo_systemString() -> usize;
}
```

- `deal` と `playTraceBin` は `dll.h` 通り **値渡し**。
- `boards` (約 250 KB) と `solvedBoards` (約 60 KB) はスタックに置かず、必ず `Box::<T>::new_zeroed().assume_init()` でヒープに確保する。`playTracesBin` (約 85 KB) も同様。
- 未決: `contractType.denom` のエンコード (`dll.h` のコメントでは 0 = NT, 1 = S, 2 = H, 3 = D, 4 = C と記憶しているが、`SolveBoard` のストレイン順と異なるためベンダリング時に `dll.h` で確認して `convert.rs` に固定する)。

### 5.1 レイアウトプローブ

`src/layout_probe.cpp` は `build.rs` が DDS と一緒にコンパイルする。全構造体の `sizeof` と、末尾フィールドの `offsetof` (`deal.remainCards`, `futureTricks.score`, `boards.mode`, `DDSInfo.systemString`) を返す `extern "C"` 関数を手書きで並べる (macro 生成にはしない。関数一覧は §5 の末尾のとおり)。

```cpp
#include <cstddef>
#include "dll.h"
extern "C" {
size_t dds_sizeof_deal() { return sizeof(deal); }
size_t dds_offsetof_deal_remainCards() { return offsetof(deal, remainCards); }
size_t dds_sizeof_futureTricks() { return sizeof(futureTricks); }
size_t dds_offsetof_futureTricks_score() { return offsetof(futureTricks, score); }
/* … 全 20 関数 (§5) … */
size_t dds_offsetof_DDSInfo_systemString() { return offsetof(DDSInfo, systemString); }
}
```

`tests/layout.rs` (`#![cfg(dds_vendored)]`) は `std::mem::size_of::<sys::deal>()` と `std::mem::offset_of!(sys::deal, remainCards)` を対応する C 関数の戻り値と `assert_eq!` する。`sizeof` が全構造体で一致し、末尾フィールドの `offsetof` が一致すれば、`#[repr(C)]` の規則上、中間フィールドの配置も一致する。これで bindgen 同等の安全性をビルド時ツールなしで得る。

## 6. エンコード変換 (`convert.rs`)

| 概念 | core | DDS | 変換 | 関数 |
| --- | --- | --- | --- | --- |
| 席 | N=0, E=1, S=2, W=3 | 同 | 恒等 | `seat(Seat) -> i32` |
| スート | C=0, D=1, H=2, S=3 | S=0, H=1, D=2, C=3 | `dds = 3 − core` | `suit(Suit) -> i32` |
| ストレイン | C=0 … S=3, NT=4 | S=0 … C=3, NT=4 | `if nt { 4 } else { 3 − core }` | `strain(Strain) -> i32` |
| ホールディング (13 bit、Two = bit 0) | `Holding(u16)` | bit 2..14 (Two = bit 2) | `(h.bits() as u32) << 2` (シフト 1 回) | `holding(Holding) -> u32` |
| ランク | Two=0 … Ace=12 | 2..14 | `+2` | `rank(core_rank: u8) -> i32` |
| バルネラビリティ (Par) | `None, NS, EW, Both` | 0 none, 1 all, 2 NS, 3 EW | 表引き | `vulnerability(Vulnerability) -> i32` |
| ディーラー (Par) | `Seat` | 0 N … 3 W | 恒等 | `seat` |
| 残りカード | `Deal` | `remainCards[hand][dds_suit]` | `holding(hands[hand].holding(core_suit))` | `remain_cards(&Deal) -> [[u32; 4]; 4]` |
| DD 表 | `DdTable[strain][declarer]` (core `Strain` 順) | `resTable[dds_strain][hand]` | ストレイン軸を反転 | `calc_dd_table` の内部 |

`convert` の関数は `remain_cards` 以外すべて `const fn`。逆変換 (`futureTricks.suit/rank` → `Card`) は `Card::new(Suit::from_index(3 − suit), Rank::from_index(rank − 2))`。

## 7. 安全ラッパー (`lib.rs`)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DdsConfig {
    pub max_threads: u32,     // 0 = DDS decides
    pub max_memory_mb: u32,   // 0 = DDS decides; the default passes threads × 95 explicitly (§7.3)
}

/// Initialise DDS once for the process. The first caller's config wins; later calls with a
/// different config log a warning and keep the first.
pub fn init(cfg: DdsConfig) -> Result<(), DdsError>;
/// `true` when the DDS sources were vendored and compiled in.
pub const fn is_available() -> bool;

/// Double-dummy table: tricks for every (strain, declarer). Defined in bridge-core, re-exported here.
pub use bridge_core::DdTable;

/// A position to solve.
#[derive(Clone, Copy, Debug)]
pub struct Position<'a> {
    pub deal: &'a Deal,            // remaining cards (the original deal for an opening lead)
    pub trump: Strain,
    pub leader: Seat,              // the player to lead (or on lead when `trick` is empty)
    pub trick: &'a [Card],         // 0..=3 cards already played to the current trick, in play order
}

pub enum Target { Max, ListLegal, Tricks(u8) }        // -1 / 0 / 1..=13
pub enum Solutions { One, AllOptimal, AllRanked }     // 1 / 2 / 3 (AllRanked = every legal card scored; the lead advisor's query)
pub enum Mode { Auto, Search, ReuseTable }            // 0 / 1 / 2

pub struct CardScore { pub card: Card, pub equals: Holding, pub score: u8 }   // `equals`: lower cards of the same suit with the same score
pub struct FutureTricks { pub nodes: u32, pub cards: Vec<CardScore> }

pub struct ParResult { pub score: i32, pub contracts: Vec<String> }   // score from NS's point of view; contracts as DDS strings
pub struct DdsInfo { pub version: String, pub threads: u32, pub threading: i32, pub system: String }

pub fn calc_dd_table(deal: &Deal) -> Result<DdTable, DdsError>;
pub fn calc_dd_tables(deals: &[Deal]) -> Result<Vec<DdTable>, DdsError>;
pub fn solve_board(pos: &Position<'_>, target: Target, solutions: Solutions, mode: Mode) -> Result<FutureTricks, DdsError>;
pub fn solve_all_boards(positions: &[(Position<'_>, Target, Solutions, Mode)]) -> Result<Vec<FutureTricks>, DdsError>;
pub fn analyse_play(deal: &Deal, play: &PlayHistory) -> Result<Vec<u8>, DdsError>;   // trump and leader come from `play`
pub fn dealer_par(table: &DdTable, dealer: Seat, vul: Vulnerability) -> Result<ParResult, DdsError>;
pub fn info() -> Result<DdsInfo, DdsError>;

#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum DdsError {
    #[error("DDS error {code}: {message}")] Code { code: i32, message: String },   // via ErrorMessage()
    #[error(transparent)] Deal(#[from] DealError),
    #[error("too many boards: {0} > 200")] TooManyBoards(usize),
    #[error("DDS is not available in this build (run `cargo xtask dds vendor` and rebuild)")] Unavailable,
}
```

`Position` にコンストラクタは無い: オープニングリードは `Position { deal, trump, leader: contract.leader(), trick: &[] }`、プレイ途中は残りカードの `Deal` (13 枚ずつでない配牌は `Deal` にならないので、`solve_board` は `deal` から `trick` と既出カードを引いた残りを内部で `remainCards` に変換する) と `history.current_trick()` を渡す。`ParResult.contracts` は `parResultsMaster.contracts` を DDS の文字列形式 (`"NS 4S"` 等) に整形したもので、構造化した `ParContract` 型は持たない。

### 7.1 各関数の対応

| 関数 | DDS 呼び出し | ロック | 備考 |
| --- | --- | --- | --- |
| `calc_dd_table` | `CalcDDtable` (内部でスレッド並列) | `batch` | 単発。`resTable` を `DdTable::new` に変換 (ストレイン軸反転) |
| `calc_dd_tables` | `CalcAllTables` を 40 件ずつ (`MAXNOOFTABLES` = 200 boards / 5 strains)、`mode = -1` (パー計算なし)、`trumpFilter = [0; 5]` (全ストレイン) | `batch` (チャンクごと) | |
| `solve_board` | `SolveBoard(dl, target, solutions, mode, &mut fut, thrId)` | `slots` から 1 スロット (ブロッキング取得) | Rust スレッド間で並行可 |
| `solve_all_boards` | `SolveAllChunksBin(bop, solvedp, chunkSize = 1)` を 200 件ずつ | `batch` | 200 超は `TooManyBoards` ではなく分割。`boards` はヒープ |
| `analyse_play` | `AnalysePlayBin(dl, play, &mut solved, thrId)` | `slots` | 戻り値 `tricks[0..=n]` (`tricks[0]` = プレイ前の DD 結果)。`trump`/`leader` は `PlayHistory::trump()`/`leader()` |
| `dealer_par` | `DealerParBin(&mut table, &mut pres, dealer, vul)` | なし (純関数) | |
| `info` | `GetDDSInfo` | なし | `versionString`, `noOfThreads`, `threading`, `systemString` を写す |

`Target::Tricks(n)` が到達不能なとき DDS は `cards = 0` を返す。ラッパーは空の `cards` を返し、エラーにしない。未ベンダリングのビルド (`!is_available()`) では全関数が `DdsError::Unavailable` を返す。

### 7.2 スレッドスロットと Mutex の規則

1. `init` は `OnceLock<Runtime>` (非公開) で 1 回だけ `SetResources(max_memory_mb, max_threads)` を呼び、続けて `GetDDSInfo` で `noOfThreads` を読んでスロット数とする。`SetMaxThreads` は使わない (DDS3 では no-op)。`init` を呼ばずにラッパー関数を呼んだ場合は `DdsConfig::default()` で暗黙に初期化する。
2. `SolveBoard` と `AnalysePlayBin` は `thrId` (0..threads) ごとに独立した作業領域を使うので、空きスロットを 1 つ貸し出す間だけ並行して呼べる。空きが無ければ `Condvar` で待つ (非ブロッキング版は持たない)。
3. `SolveAllChunksBin`、`CalcAllTables`、`CalcDDtable`、`AnalyseAllPlaysBin` は 2.9 のドキュメント通り非再入なので `batch: Mutex<()>` を保持している間だけ呼ぶ。バルク呼び出しは DDS 内部で全スレッドを使うため、同時に `solve_board` を走らせても速くならない。
4. `FreeMemory()` はプロセス寿命の間呼ばない (ドキュメントに明記)。
5. `Runtime` は `Send + Sync`。`Position` の検証 (`trick.len() <= 3`、`trick` のカードが `deal` に含まれる、枚数の整合) はラッパーで行い、それ以外の不正は DDS の戻りコードを `DdsError::Code` に変換する (`ErrorMessage` の 80 バイト行)。

### 7.3 `popen` メモリ探索の緩和 (R6)

2.9 の `System.cpp` は搭載メモリを macOS で `popen("sysctl -n hw.memsize")`、Linux で `popen("free -k …")` により調べる。サンドボックスやコンテナではこれが失敗し、既定値が不適切になりうる。対策として `DdsConfig::default()` (`max_memory_mb = 0`) は 0 を DDS に渡さず、`threads × 95` MB を明示して `SetResources` に渡す。`info()` の `threads` と `system` で DDS が実際に何を設定したかを確認でき、`tests/concurrency.rs` はこれを検査する。

## 8. テスト

| テスト | 場所 | 内容 | 基準 |
| --- | --- | --- | --- |
| レイアウト | `tests/layout.rs` | §5.1 の `sizeof`/`offsetof` 突き合わせ | 全構造体で一致 |
| 差分 `list100` | `tests/differential.rs` | `hands/list100.txt` の各配牌で `calc_dd_table` と `TABLE` 行を比較 (`BRIDGE_CORPUS_DIR` 必須) | 100% 一致 |
| 差分 `masterDD` | 同 (`#[ignore]`) | 83,691 配牌 | 100% 一致 |
| 並行 `SolveBoard` | `tests/concurrency.rs` | 8 スレッド × 100 局面を `solve_board`、逐次実行の結果と比較。`info().threads` の確認 | エラー 0、結果一致 |
| `analyse_play` | `tests/differential.rs` | `list100.txt` の `PLAY`/`TRACE` 行 | 一致 |
| `dealer_par` | 同 | `PAR` 行 | 一致 |
| ベンチ | `benches/dds.rs` | `calc_dd_table`、`solve_board(AllRanked)` | 目標なし。数値を記録 |

## 9. ファサード `bridge::dd`

`crates/bridge/src/lib.rs` のインラインモジュール `dd`:

```rust
pub mod dd {
    pub use bridge_core::DdTable;

    /// A double-dummy solver.
    pub trait DoubleDummy: Send + Sync {
        fn dd_table(&self, deal: &Deal) -> Result<DdTable, DdError>;
        /// Every legal opening lead for `leader` against `trump` with the tricks the defence then takes.
        fn lead_scores(&self, deal: &Deal, trump: Strain, leader: Seat) -> Result<Vec<(Card, u8)>, DdError>;
    }

    #[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
    pub enum DdError {
        #[error("no double-dummy solver available in this build")] Unavailable,
        #[error("double-dummy solver: {0}")] Backend(String),   // DdsError::to_string()
    }

    /// The DDS-backed solver, if this build has one (feature `dds`, non-wasm, sources vendored).
    pub fn dds() -> Option<Arc<dyn DoubleDummy>>;
}
```

- `DdTable` は DDS なしでも使える (PBN の `OptimumResultTable` の読み書き、`03-format.md` §2.2)。定義は `bridge-core` に置き、ここで再エクスポートする。
- `DoubleDummy` の引数は `bridge-core` の型だけ (`Deal`, `Strain`, `Seat`, `Card`) なので、`dds` feature の有無でトレイトの形は変わらない。`Position` 等の `bridge-dds` の型はファサードから再エクスポートしない: 細かい制御が要るアプリは `bridge-dds` に直接依存する。
- `dds()` は `cfg(all(feature = "dds", not(target_arch = "wasm32")))` かつ `bridge_dds::is_available()` のときだけ `Some` を返す。バックエンドは `calc_dd_table` と `solve_board(Target::Max, Solutions::AllRanked, Mode::Auto)` を呼び、`DdsError` は `DdError::Backend(e.to_string())` に写す。
- 上位アプリは `bridge::dd::dds()` が `None` なら `DdError::Unavailable` を扱う。wasm では常に `None`。

## 10. `xtask` コマンド

### 10.1 `cargo xtask dds vendor [--force]`

1. `https://github.com/dds-bridge/dds/archive/refs/tags/v2.9.0.tar.gz` を `ureq` で取得する。
2. アーカイブの sha256 を `vendor/SHA256SUMS` に記録した値と照合する。ファイルが無ければ計算値を書いて終了コード 2 (初回)、不一致なら失敗。
3. `include/dll.h`、`src/*.cpp` (27 個、`build.rs` の `SOURCES` と同じ一覧)、`src/*.h`、`LICENSE` だけを `crates/bridge-dds/vendor/dds-2.9.0/` に展開する。Makefile やテストデータは含めない。
4. `--force` なしで既に揃っていればスキップ。
5. 完了後に `cargo test -p bridge-dds --test layout` を案内する。

### 10.2 `cargo xtask dds regen-bindings [--check]`

1. `bindgen` 0.73 を `vendor/dds-2.9.0/include/dll.h` に適用し (`--allowlist-function '(SetMaxThreads|SetResources|SetThreading|FreeMemory|SolveBoard|CalcDDtable|CalcAllTables|SolveAllChunksBin|DealerParBin|AnalysePlayBin|AnalyseAllPlaysBin|GetDDSInfo|ErrorMessage)'`、`--allowlist-type` に §5 の構造体、`--allowlist-var 'MAXNOOF.*|RETURN_.*|DDS_.*'`)、生成結果を手書きの `src/sys.rs` と突き合わせる。libclang が必要なのでこのコマンドだけが要求する。
2. `--check` は生成結果と `sys.rs` の宣言 (フィールドの型と順序) が一致しなければ非 0 で終了する (CI では実行しない。ヘッダ更新時のレビュー用)。生成物はコミットしない。
3. 手書き `sys.rs` が正で、生成物は差分を目視するための参照。両者の乖離は `tests/layout.rs` が検出する。

## 11. 未決

- 未決: `cargo publish` での `vendor/` 同梱方法 (§3)。
- 未決: `contractType.denom` のエンコード確認 (§5)。
- 未決: `openmp` feature の MSVC / Apple clang 対応 (§4)。
- 未決: DDS3 への移行時期。ラッパーの公開面はレガシー名と同一に保ち、`vendor/` と `build.rs` の差し替えだけで済むようにしておく。DDS3 の Emscripten ビルドは将来の `wasm32-emscripten` feature の候補 (対象外)。

アーカイブの sha256 は `VENDOR.md` ではなく `vendor/SHA256SUMS` (git-ignored) に記録するので、コミット sha を文書に固定する作業は無い。`DoubleDummy::lead_scores` は `bridge-core` の型だけを取るので、`Position` を `bridge-core` へ移す必要は無くなった。
