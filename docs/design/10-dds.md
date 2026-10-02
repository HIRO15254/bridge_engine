# 10. `bridge-dds`: DDS v2.9.0 の FFI ラッパー

本文書は D10 の決定、すなわち DDS (Bo Haglund, Soren Hein) の v2.9.0 を `cc` でビルドし、手書きの `#[repr(C)]` バインディングを C++ 側の `sizeof`/`offsetof` プローブで検証する方式を確定する。DDS のソースは `crates/bridge-dds/vendor/` に置くが git には含めず `cargo xtask dds vendor` で取得し、未取得でも `cargo:warning` を出してワークスペース全体のビルドは通す。安全ラッパーは `SolveBoard`/`AnalysePlayBin` の `thrId` スロットで Rust スレッド間の並行を許し、非再入のバルク関数はスロットを全部集めることで直列化する (両者が同じ `thrId` 空間を奪い合わないよう、単なる別ロックではなく同じスロットプールを使う。§7.2 参照)。

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

要件: 27 個の `.cpp` を `-std=c++11 -O3` でビルド、`DDS_THREADS_STL` を定義 (std::thread、外部依存なし、MSVC/clang/gcc で動く)、feature `openmp` で `DDS_THREADS_OPENMP` (MSVC 以外は OpenMP ランタイムのプローブに成功したときだけ。補足を参照)、`wasm32` は何もしない (`lib.rs` が `compile_error!`)、`vendor/` 未取得なら警告して FFI を cfg で除外する (フェーズ 0 で確定済み)。

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

    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let msvc = target_env == "msvc";

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std(if msvc { "c++14" } else { "c++11" }) // MSVC の /std は c++14 から
        .opt_level(3)
        .include("vendor/dds-2.9.0/include")
        .include(src)
        .files(SOURCES.iter().map(|f| src.join(format!("{f}.cpp"))))
        .file("src/layout_probe.cpp")
        .define("DDS_THREADS_STL", None)
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-deprecated-declarations");
    if std::env::var("CARGO_FEATURE_OPENMP").is_ok() {
        if msvc {
            // MSVC は vcomp を同梱し /openmp で自動リンクするので、プローブしない。
            build.define("DDS_THREADS_OPENMP", None).flag("/openmp");
        } else {
            let (flags, link_lib): (&[&str], &str) = if target_os == "macos" {
                (&["-Xpreprocessor", "-fopenmp"], "omp") // Homebrew の libomp
            } else {
                (&["-fopenmp"], "gomp")
            };
            // 小さな OpenMP プログラムをコンパイルしてリンクできたときだけ OpenMP にする。
            if probe_openmp(&target_env, flags, link_lib) {
                build.define("DDS_THREADS_OPENMP", None);
                for flag in flags {
                    build.flag(flag);
                }
                println!("cargo:rustc-link-lib={link_lib}");
            } else {
                // ランタイムが無いホストでビルドを落とさず、STL のバックエンドだけで組む。
                println!("cargo:warning=bridge-dds: no OpenMP runtime found ... std::thread backend instead");
            }
        }
    }
    build.compile("dds");
    println!("cargo:rustc-cfg=dds_vendored");
}

// `omp.h` を使う 3 行のプログラムを `flags` と `-l{link_lib}` で試しにビルドする。
// 成功すれば true、ツールチェーンが OpenMP を持たなければ false (ビルドは落とさない)。
fn probe_openmp(target_env: &str, flags: &[&str], link_lib: &str) -> bool { /* … */ }
```

このリストは骨格を示す抜粋で、実際の `build.rs` は `src/ffi_guard.cpp` のコンパイル、`include/portab.h` の存在確認 (無ければ「未取得」扱い)、警告の抑制 (`.warnings(false)`)、MSVC 向けの `-EHsc` と `_CRT_SECURE_NO_WARNINGS` も持つ。食い違えば `build.rs` が正である。

補足:

- `DDS_THREADS_STL` は 2.9.0 の `Makefile_linux_shared` にあるバックエンドの 1 つ。マクロ無指定だと DDS は単一スレッドになる。
- `openmp` は `DDS_THREADS_STL` に加えて `DDS_THREADS_OPENMP` を定義する (DDS は定義されたバックエンドのうち番号の小さいものを取るので OpenMP が選ばれ、STL のコードは使われない)。MSVC は `/openmp` (同梱の vcomp、プローブなし)。それ以外は Linux/gcc が `-fopenmp` + `gomp`、macOS が `-Xpreprocessor -fopenmp` + `omp` (Homebrew の libomp、Apple clang は OpenMP を持たない) で小さな OpenMP プログラムのコンパイルとリンクをプローブし、失敗すれば `cargo:warning` を出して std::thread のバックエンドだけで組む。MSVC と Apple clang の対応は済んでいる (`VENDOR.md`)。CI で `--features openmp` を走らせるのは ubuntu だけで、Linux の実経路・MSVC の `/openmp`・macOS の libomp は実行では未確認 (`12-roadmap.md` 節「フェーズ 5 の完了」。Homebrew の libomp が無い Mac では probe が失敗して STL に戻る経路を確かめた)。
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
pub enum Mode { Auto, Search, ReuseTable }            // DDS には常に 1 を渡す (§7.4)

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
    #[error("DDS error {code}: {message}")] Code { code: i32, message: String },   // via ErrorMessage() or synthesized (bad `trick`)
    #[error("DDS is not available in this build (run `cargo xtask dds vendor` and rebuild)")] Unavailable,
}
```

（5.5–5.7 の見直しで `Deal(#[from] DealError)` と `TooManyBoards(usize)` は削除した: `calc_dd_tables`/`solve_all_boards` は内部でチャンク分割するので `TooManyBoards` を構築する経路が無く、`Deal` から不正な `Deal` を作る経路もこのクレートには無い — どちらも宣言されているだけで一度も構築されない死んだヴァリアントだった。実際に発生する不正は `Code`（DDS 自身の戻りコード、または `trick`/`target` の検証失敗をラッパーが `RETURN_SUIT_OR_RANK`/`RETURN_DUPLICATE_CARDS`/`RETURN_CARD_COUNT`/`RETURN_TARGET_WRONG_HI` として合成したもの。§7.4）だけなので、`match` で網羅していても取りこぼしは無い。）

`Position` にコンストラクタは無い: オープニングリードは `Position { deal, trump, leader: contract.leader(), trick: &[] }`、プレイ途中は残りカードの `Deal` (13 枚ずつでない配牌は `Deal` にならないので、`solve_board` は `deal` から `trick` と既出カードを引いた残りを内部で `remainCards` に変換する) と `history.current_trick()` を渡す。`ParResult.contracts` は `parResultsMaster.contracts` を DDS の文字列形式 (`"NS 4S"` 等) に整形したもので、構造化した `ParContract` 型は持たない。

### 7.1 各関数の対応

| 関数 | DDS 呼び出し | ロック | 備考 |
| --- | --- | --- | --- |
| `calc_dd_table` | `CalcDDtable` (内部でスレッド並列) | `slots` を全部 | 単発。`resTable` を `DdTable::new` に変換 (ストレイン軸反転) |
| `calc_dd_tables` | `CalcAllTables` を 40 件ずつ (`MAXNOOFTABLES` = 200 boards / 5 strains)、`mode = -1` (パー計算なし)、`trumpFilter = [0; 5]` (全ストレイン) | `slots` を全部 (チャンクごと) | `tests/batching.rs` が 39/40/41 の境界を検査 |
| `solve_board` | `SolveBoard(dl, target, solutions, mode, &mut fut, thrId)` | `slots` から 1 スロット (ブロッキング取得) | Rust スレッド間で並行可 |
| `solve_all_boards` | `SolveAllChunksBin(bop, solvedp, chunkSize = 1)` を 200 件ずつ | `slots` を全部 | 200 超はエラーにせず分割 (`tests/batching.rs` が 199/200/201 の境界を検査)。`boards` はヒープ |
| `analyse_play` | `AnalysePlayBin(dl, play, &mut solved, thrId)` | `slots` から 1 スロット | 戻り値 `tricks[0..=n]` (`tricks[0]` = プレイ前の DD 結果、常に `n + 1` 要素)。DDS 自身は最後のトリック (強制) を解析せず 49〜52 枚のプレイに対して 49 要素しか返さないので、ラッパーが 48 枚目の後の値を繰り返して `n + 1` 要素に埋める (強制トリック中に結果は変わらない。フェーズ 5 レビューで修正)。`trump`/`leader` は `PlayHistory::trump()`/`leader()` |
| `dealer_par` | `DealerParBin(&mut table, &mut pres, dealer, vul)` | なし (純関数) | |
| `info` | `GetDDSInfo` | なし | `versionString`, `noOfThreads`, `threading`, `systemString` を写す |

`Target::Tricks(n)` が到達不能なとき DDS は `cards = 0` を返す。ラッパーは空の `cards` を返し、エラーにしない。未ベンダリングのビルド (`!is_available()`) では全関数が `DdsError::Unavailable` を返す。

### 7.2 スレッドスロットと Mutex の規則

1. `init` は `OnceLock<Runtime>` (非公開) で 1 回だけ `bdds_SetResources(max_memory_mb, max_threads, ncores)` (`ffi_guard.cpp` による、ハードウェア探索を除いた `SetResources` の再実装。§7.3) を呼び、続けて `GetDDSInfo` で `noOfThreads` を読んでスロット数とする。`SetMaxThreads` は使わない (DDS3 では no-op)。`init` を呼ばずにラッパー関数を呼んだ場合は `DdsConfig::default()` で暗黙に初期化する。
2. `SolveBoard` と `AnalysePlayBin` は `thrId` (0..threads) ごとに独立した作業領域を使うので、空きスロットを 1 つ貸し出す間だけ並行して呼べる。空きが無ければ `Condvar` で待つ (非ブロッキング版は持たない)。
3. `SolveAllChunksBin`、`CalcAllTables`、`CalcDDtable`、`AnalyseAllPlaysBin` は 2.9 のドキュメント通り非再入である「だけ」ではなく、バルク呼び出し自身も DDS 内部でこの同じ `thrId` 空間 (`track[]` などの per-thread-index 状態) を使って複数スレッドを回す。そのためバルク呼び出しは `slots` の全スロットを (空くまで `Condvar` で待って) 取ってから DDS を呼び、呼び終えたら全部返す。単に別の `Mutex` でバルク呼び出し同士だけを排他しても、バルク呼び出し中に外部から `solve_board`/`analyse_play` が同じ `thrId` に触れてしまい、DDS 内部状態が壊れる (`Moves::GetTrickData` の `"Sum N is not four"` や `ABsearch.cpp` のアサート落ち。`--include-ignored` で `masterdd_matches_upstream` と `list100_matches_upstream` が同一プロセス内で並行実行されたときに実際に踏んだ)。`tests/concurrency.rs` の `concurrent_bulk_and_slot_calls_do_not_corrupt_each_other` はこの組み合わせの恒久的な回帰テスト (元の再現手順はその場限りのリポプロで、コミットされたテストは無かった)。`acquire_slot` はバルク呼び出しが待機/保持中は新規スロットを渡さない (`batch_waiting` フラグ) ので、バルク呼び出し側が `solve_board`/`analyse_play` の絶え間ない要求で永久に待たされることもない。バルク呼び出しは DDS 内部で全スレッドを使うため、同時に `solve_board` を走らせても速くならない。
4. `FreeMemory()` はプロセス寿命の間呼ばない (ドキュメントに明記)。
5. `Runtime` は `Send + Sync`。`Position` の検証 (`trick.len() <= 3`、`trick` に重複が無い、`trick[i]` の持ち主が `leader` から数えて i 番目の席 = 枚数の整合) と `Target::Tricks(n)` の `n <= 13` はラッパーで行い (§7.4)、それ以外の不正は DDS の戻りコードを `DdsError::Code` に変換する (`ErrorMessage` の 80 バイト行。全メッセージは 80 バイト未満で、未知のコードには `"Not a DDS error code"` が入る)。
6. スロットは `acquire_slot`/`acquire_all_slots` が返す RAII ガード (`SlotGuard`/`AllSlotsGuard`) の `Drop` で返却する。取得から返却までの間に Rust 側のパニックが起きてもスロットが漏れない (漏れると以後のバルク呼び出しが永久に待つ)。ロックは FFI 呼び出しの全区間で保持され、呼び出しが戻った直後に `drop` で明示的に返す。

### 7.3 `popen` メモリ探索の緩和 (R6)

2.9 の `System.cpp` は搭載メモリを macOS で `popen("sysctl -n hw.memsize")`、Linux で `popen("free -k | tail -n+3 | head -n1 | awk '{print $NF}'")` により調べる (`System::GetHardware`)。上流の `SetResources` は引数の `maxMemoryMB` にかかわらず **必ず** この探索を行い、メモリ上限を `min(1.3 × maxMemoryMB, 0.7 × 探索値)` とする。探索が失敗して 0 を読むと (サンドボックスや `PATH` に `sysctl` が無い macOS、最近の procps で上のパイプラインが Swap 行を読んでしまう swap 無しの Linux、`free` が無いイメージ)、スレッド数 0 で `Memory` が作られ、`InitDebugFiles` の `Memory::GetPtr(0)` が `Memory::GetPtr: 0 vs. 0` を出力して `exit(1)` する。`popen` 自体が失敗すると `fscanf(NULL)` で落ちる。いずれも C++ 内部でプロセスを終了させるので、`noexcept` ガードでも Rust 側でも捕まえられない。`max_memory_mb ≤ 23` も `floor(1.3 × M / 30) = 0` スレッドで同じ経路に入る (フェーズ 5 レビューで発見。以前の文書は「明示値を渡せば探索を避けられる」としていたが誤り)。

対策 (ベンダリングしたソースは無改変のまま):

- `ffi_guard.cpp` の `bdds_SetResources(maxMemoryMB, maxThreads, ncores)` は `Init.cpp` の `SetResources` の本体を写したもので、`GetHardware` を呼ばない。コア数は Rust 側の `std::thread::available_parallelism()` を渡し、メモリ上限は `1.3 × maxMemoryMB` (32 ビットでは 1800 MB 上限) で、探索値による 70% 上限は無くなる。さらにメモリ上限を 1 スレッド分 (`THREADMEM_SMALL_MAX_MB` = 30 MB) 以上に、スレッド数を 1 以上に切り上げる。`sysdep`/`memory`/`scheduler`/`threadMgr`/`_initialized` と `InitDebugFiles`/`InitConstants` は `Init.cpp` の非 static な大域なので `extern` 宣言で足りる。
- `Runtime::new` は `max_memory_mb` を `MIN_MEMORY_MB` (= 24 = ceil(30 / 1.3)) 未満なら 24 に切り上げ、`tracing::warn!` を出す。`DdsConfig::default()` (`max_memory_mb = 0`) は従来どおり `threads × 95` MB を明示する。
- 生の `sys::SetResources`/`SetMaxThreads` は探索を行うので安全ラッパーからは呼ばない (`sys.rs` に注記)。このビルドでは `dds.cpp` のライブラリ初期化 (`DllMain`/`USES_CONSTRUCTOR`) も有効にならないので、探索はどこからも走らない。
- 回帰テスト `tests/fatal_paths.rs` は自分のテストバイナリを子プロセスとして起動し、`PATH=/nonexistent` (探索失敗) と `max_memory_mb` = 1/20/23/24 で `init` → `info` → `calc_dd_table` が正常終了することを確かめる (修正前は子プロセスが終了コード 1 で落ちるのを確認済み)。

残る `exit(1)` は `TransTableS.cpp`/`TransTableL.cpp` の `malloc`/`calloc` 失敗時 (メモリ枯渇) だけで、設定から到達する経路ではない (Rust 自身もメモリ確保失敗では abort する)。

`info()` の `threads` と `system` で DDS が実際に何を設定したかを確認でき、`tests/concurrency.rs` はこれを検査する。

### 7.4 5.5–5.7 の健全性レビューで直したもの

`unsafe` の見直し (全 FFI 構造体が `#[repr(C)]` でレイアウトテスト済み、大きな構造体 (`boards`/`solvedBoards`/`ddTableDeals`/`ddTablesRes`/`allParResults`) はヒープ確保、1 スロットを 2 スレッドが同時に使わない、非再入呼び出しのロックを呼び出し全体で保持) は問題なし。Rust から C へコールバックを渡す箇所は無いので Rust のパニックが FFI を越えることは無い。一方、次の 5 点は実際の不具合で、いずれも修正前に失敗するテストを付けた。

1. **`Mode::ReuseTable` (DDS mode 2) でセグフォルト**。mode 2 は前回の呼び出しと同じ配牌・切り札かを確かめずに置換表のリセットを省く (`SolverIF.cpp`)。これが正しいのは「同じ `thrId` の前回の呼び出し」が同じ配牌・切り札だったときだけで、スロットを呼び出しごとに貸し出すこのラッパーでは呼び出し側に保証する手段が無い。無関係な配牌の置換表が残ったスロットで mode 2 を呼ぶと DDS 内部で SIGSEGV (安全な Rust から到達可能な未定義動作)。`tests/reuse_table.rs` (DDS を 1 スレッドに固定して必ず同じスロットに当てる) が再現する。
2. **`Mode::Auto` (DDS mode 0) の強制 1 枚で得点 0**。mode 0 は合法手が 1 枚だけの局面を探索せずに返し、得点に番兵 `-2` を入れる。ラッパーは負値を 0 に丸めていたので、たとえば途中局面でシングルトンをフォローする手番の得点が誤って 0 トリックになった (`tests/edge_cases.rs`)。
   → 1 と 2 の対策として `Mode` の 3 値はすべて DDS に `1` (常に探索) として渡す。`Mode` 型は互換のため残し、rustdoc に理由を書いた。失うものは無い: mode 0/1 でも DDS は同じ `thrId` で配牌が同一か類似 (同じボードの後の局面など) かつ切り札が同じなら置換表を自動で引き継ぐ。mode 0 の近道が効くのは合法手 1 枚の局面だけで、その場合も探索量は同じボードの通常局面と変わらない。
3. **DDS の入力エラーでカレントディレクトリに `dump.txt`**。`SolveBoard` の入力検査 (`BoardRangeChecks`/`BoardValueChecks`) は失敗のたびに `DumpInput` で `dump.txt` を書く。型付き API から到達できたのは `Target::Tricks(n > 13)` (`RETURN_TARGET_WRONG_HI`) と手番違いの `trick` カード (`RETURN_CARD_COUNT`) の 2 つで、どちらもラッパーが DDS を呼ぶ前に同じコードで弾くようにした (`check_target`、`position_deal`)。`Position` は常に完全な配牌から高々 3 枚を引いたものなので 13 トリックが残り、`RETURN_TARGET_TOO_HIGH` 等の他の検査には到達しない。`analyse_play` は合法性検査済みの `PlayHistory` しか受け取らない。
4. **C++ 例外が `extern "C"` を越えて Rust へ巻き戻る**。DDS は例外を一切捕まえないので、コンテナの `std::bad_alloc` や STL スレッドの `std::system_error` がそのまま Rust のフレームへ巻き戻る (未定義動作)。`src/ffi_guard.cpp` に `noexcept` の薄いラッパー (`bdds_SolveBoard` 等、ラッパーが呼ぶ全エントリポイント + `SetResources`/`GetDDSInfo`) を置き、`catch (...)` を `RETURN_UNKNOWN_FAULT` に変える。`lib.rs` はこれらだけを呼ぶ (`ErrorMessage` は `strcpy` の `switch` なので投げない)。`SetResources` の失敗は `tracing::error!` で報告する。
5. **ファサードの `lead_scores` が同等カードを落とす** (§9)。

あわせて、DDS から読み戻す枚数 (`futureTricks.cards`、`solvedPlay.number`、`parResultsMaster.number`) は配列長で頭打ちにし、壊れた値でも範囲外添字にならないようにした。`DealerParBin` は書き込み可能な大域状態を持たない (`DealerPar.cpp` の大域は読み取り専用の表だけ) ことを確認したので、ロックなしのままとする。

## 8. テスト

| テスト | 場所 | 内容 | 基準 |
| --- | --- | --- | --- |
| レイアウト | `tests/layout.rs` | §5.1 の `sizeof`/`offsetof` 突き合わせ | 全構造体で一致 |
| 差分 `list100` | `tests/differential.rs` | `hands/list100.txt` の各配牌で `calc_dd_table` と `TABLE` 行を比較。データはコーパス (`corpus/data/dds/list100.txt`) か、無ければ `cargo xtask dds vendor` が同じ DDS アーカイブ (SHA-256 検証済み) から展開する `vendor/dds-2.9.0/hands/list100.txt` を使う。CI の `dds` ジョブ (3 OS) は `BRIDGE_REQUIRE_LIST100=1` で実行し、データが無ければスキップせず失敗させる。nightly は `--include-ignored` (以前の `--ignored` は `#[ignore]` の無いこのテストを除外していた。フェーズ 5 レビューで修正) | 100% 一致 |
| 差分 `masterDD` | 同 (`#[ignore]`) | 83,691 配牌 | 100% 一致 |
| 並行 `SolveBoard` | `tests/concurrency.rs` | 8 スレッド × 100 局面を `solve_board`、逐次実行の結果と比較。`info().threads` の確認 | エラー 0、結果一致 |
| `analyse_play` | `tests/differential.rs` | `list100.txt` の `PLAY`/`TRACE` 行 | 一致 |
| `dealer_par` | 同 | `PAR` 行 | 一致 |
| バッチ境界 | `tests/batching.rs` | `calc_dd_tables` を 39/40/41 件、`solve_all_boards` を 199/200/201 件で呼び、単発の結果と比較。空入力 | 全件一致 |
| バルクとスロットの混在 | `tests/concurrency.rs` | `calc_dd_tables`/`solve_all_boards` と他スレッドの `solve_board` を同時に走らせる (§7.2 規則 3 の回帰テスト) | 異常終了なし、結果一致 |
| `init` の冪等性 | `tests/init.rs` (専用バイナリ) | 2 回目の `init` が別設定でもエラーにならず最初の設定を保つ | 一致 |
| エラー経路 | `tests/edge_cases.rs`、`tests/position_trick.rs` | `Target::Tricks(14)`、手番違い・重複・4 枚の `trick` が `DdsError::Code` になり `dump.txt` が作られない。強制 1 枚の局面が全 `Mode`/`Solutions` で正しい得点 | 期待コード、ファイルなし |
| `ReuseTable` | `tests/reuse_table.rs` (専用バイナリ、DDS 1 スレッド) | 無関係な配牌の後の `Mode::ReuseTable` が新規探索と一致 (§7.4) | 一致、クラッシュなし |
| 計時 | `tests/timing.rs` (`#[ignore]`、release) | 疑似乱数 100 配牌で `calc_dd_table`、`calc_dd_tables`、`solve_board(AllRanked)` の平均時間 | 目標なし。数値を記録 (下記) |

`NoSlot` 相当のエラーは無い: スロットが空かなければ `Condvar` で待つ (§7.2 規則 2)。`TooManyBoards` も無い: 上限を超える入力は内部で分割する (§7 の注記)。

計時 (2026-09-26、10 コアの macOS、release、`tests/timing.rs`、3 回の最良値。別ワークフローのビルドと同時実行のため `vm.loadavg` を併記): `calc_dd_table` 78.8 ms/配牌 (load 7.55)、`calc_dd_tables` 36.3 ms/配牌 (load 7.55)、`solve_board(Target::Max, AllRanked)` 40.0 ms/回 (load 10.00)。`calc_dd_table` と `calc_dd_tables` は DDS 内部で全スレッドを使う値、`solve_board` は 1 スロットでの逐次値。負荷の高い環境での値なので上限の目安として扱う。

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
- `dds()` は `cfg(all(feature = "dds", not(target_arch = "wasm32")))` かつ `bridge_dds::is_available()` のときだけ `Some` を返す。バックエンドは `calc_dd_table` と `solve_board(Target::Max, Solutions::AllRanked, Mode::Auto)` を呼び、`DdsError` は `DdError::Backend(e.to_string())` に写す。DDS の `AllRanked` は同等カードの代表 1 枚だけを返し、残りを `equals` に入れるので、`lead_scores` は `equals` を展開して手札の全カードを (代表と同じ得点で) 返す (5.5–5.7 の見直しまでは代表だけを返しており、`AKQ2.T98.T98.T98` で 13 枚中 5 枚しか返らなかった。`tests/dds.rs` の `lead_scores_lists_every_card_including_equivalent_ones`)。
- `tests/dd_from_pbn.rs`: DDS なしで PBN の `OptimumResultTable` から `DdTable` を読めることと、DDS があるときはその表が `dds()` での計算結果と一致することを確かめる。`tests/dd_without_feature.rs` は `dds` feature なしで `dds()` が `None` を返すこと、`tests/dds.rs` の `dds_none_iff_unavailable` は feature ありで `dds().is_some() == is_available()` を確かめる。
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
- 解決済み: `openmp` feature の MSVC / Apple clang 対応 (§4。MSVC は `/openmp`、macOS は libomp のプローブで、無ければ STL にフォールバックする)。未確認: `openmp` feature の実経路 (Linux/gcc、MSVC `/openmp`、macOS の libomp) は実行していない。コードは対応済み。
- 未決: DDS3 への移行時期。ラッパーの公開面はレガシー名と同一に保ち、`vendor/` と `build.rs` の差し替えだけで済むようにしておく。DDS3 の Emscripten ビルドは将来の `wasm32-emscripten` feature の候補 (対象外)。

アーカイブの sha256 は `VENDOR.md` ではなく `vendor/SHA256SUMS` (git-ignored) に記録するので、コミット sha を文書に固定する作業は無い。`DoubleDummy::lead_scores` は `bridge-core` の型だけを取るので、`Position` を `bridge-core` へ移す必要は無くなった。
