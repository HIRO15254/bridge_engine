# 13. 決定記録 (D1〜D17)

本文書は仕様からの意図的な差分と、仕様が方針に留めていた箇所の決定を ADR 形式で記録する。各項目は「決定」「理由」「仕様との差分」「影響するクレート」の 4 項で書き、仕様の未決事項 3 件 (BML パーサの方式、ナチュラル推定の精度測定、システム定義の配布形式) は D7、D8、D9 で決定する。設計文書間で食い違いがあれば本文書を正とする。

## 一覧

| # | 項目 | 決定 (要約) |
| --- | --- | --- |
| D1 | `Atom.shapes` と `suit_len` | `ShapeSet` に一本化、`suit_len` は射影 |
| D2 | HCP の「事前分布から抽選」 | 棄却なしの厳密一様サンプリング |
| D3 | 確定カードの扱い | 制約は元の 13 枚に対して定義、サンプラーが合成 |
| D4 | `Atom` の否定 | 排他的連鎖で互いに素な Atom に分解 |
| D5 | `Shape`/`ShapeClass`/`ShapeSet` の置き場 | `bridge-core` |
| D6 | `BidChoice` | enum に統一 |
| D7 | BML パーサ | 自前の Rust (winnow) 実装、`.bss` をオラクルに |
| D8 | ナチュラル推定の精度測定 | 3 種の擬似正解による測定 |
| D9 | システム定義の配布形式 | BML ソース配布 + ロード時コンパイル、`postcard` キャッシュは任意 |
| D10 | DDS 連携 | v2.9.0 をベンダリングし `cc` でビルド、手書き `#[repr(C)]` |
| D11 | `Interpretation` の選言数 | 席ごとに上限 K=8 |
| D12 | 並列化の再現性 | `(master_seed, i)` から splitmix64 で派生、`Xoshiro256PlusPlus` |
| D13 | 数値型 | 個数・重みは `u64`、`distribution_points` は `i8` |
| D14 | 依存バージョン | `winnow` 1.0、`rand` 0.10、`criterion` 0.8 |
| D15 | 解釈の低信頼度 | ε-混合 |
| D16 | BML 拡張 | `#+KEY:` メタと `{prio:N}` / `{w:X}` 注釈 |
| D17 | 席の条件 | `#SEAT` はオープナーの席位置、先頭パスは経路に含めない |

---

## D1. `Atom.shapes` と `suit_len` の一本化

**決定**: `Atom` の長さ情報は `ShapeSet` (順序付き 13 枚シェイプ 560 個の 576 bit 集合) だけが持つ。`suit_len` はフィールドではなく、コンストラクタ (`Atom::with_suit_len`、`ShapeSet::from_suit_len(s)`) と射影 (`Atom::suit_len(s) = shapes.suit_len(s)`) として提供する。

**理由**: 両方を持つと真実が二重になり、交差・否定・充足判定のたびに整合を取る必要が生じる。スート長の制約 `len[s] ∈ [lo, hi]` はシェイプ集合のマスク `from_suit_len` で完全に表現でき、直積でない集合 (「5-5 のどちらか」など) も表せる。読みやすい出力が要る場合は `ShapeSet::factor()` (直積なら 4 範囲)、`classes()` (クラスの和)、ノードの `description` で足りる。

**仕様との差分**: 仕様 §4 の `Atom { shapes: ShapeSet /* ShapeClass のビットセット */, suit_len: [RangeInclusive<u8>; 4], … }` から `suit_len` を削除し、`shapes` を「クラス集合」から「順序付きシェイプ集合」に変える。要約 API `suit_len(Suit)` は仕様通り残る。

**影響するクレート**: `bridge-core` (`ShapeSet`)、`bridge-constraint` (`Atom`)、`bridge-system` (説明文コンパイラが `with_suit_len` を使う)、`bridge-play` (ショウアウト制約を `from_suit_lens` で作る)。

## D2. HCP の抽選を厳密一様サンプリングにする

**決定**: 仕様 §4 の階層サンプリング (シェイプ抽選 → HCP 目標を事前分布から抽選 → 配分 → `eval` 検査で戻る) を、**棄却なしの厳密一様サンプリング** に置き換える。プールの部分集合をスートごとに `(長さ, 鍵)` でバケット化し、鍵 `hcp | (x << 6)` の疎畳み込みと接頭和で「制約を満たす手の数」を厳密に数え、その個数に比例して抽選する。

**理由**: 集合のサイズが厳密に出るので `log_prob` が厳密になり、重点重み付け (仕様 §7) の前提が満たせる。棄却が無いので「18+、メジャー 4-5、♦にコントロール」のような厳しい制約でも受理率が落ちない。試算 0.3〜0.5 μs/手で、目標 10^5 手/秒/コアの約 20 倍。`prepare` (20〜60 μs) は高コスト側なので `Sampler` を一級オブジェクトにして L4 がキャッシュする。

**仕様との差分**: 「`hcp` 範囲内の目標点数をその `Shape` 条件下の事前分布から抽選」「`eval` を検査し不合格なら戻る」の手順は無くなる。`eval` のうち加法的な指標 (コントロール、ルーザー、QT) は鍵の追加特徴として厳密に扱い、棄却は `Custom`・residual・スロット超過だけになる。

**影響するクレート**: `bridge-constraint` (`Sampler`)、`bridge-sample` (`ConstraintProposal` が `Sampler` をキャッシュ)。

## D3. 確定カードの扱い

**決定**: 制約は常に **元の 13 枚** に対して定義する。自分の手・ダミー・既出カードは `KnownCards { known: [Hand; 4] }` で持ち、`Sampler::prepare(c, pool, fixed)` が残り部分を列挙するときに `H_s = sub_s ∪ fixed_s` で全ての量を評価する。「残り部分に対する Atom」のような変換型は作らない。

**理由**: 変換を作ると HCP 範囲のシフト、長さのシフト、カード要件の充足/不能判定を制約側で正しく行う必要があり、誤りの温床になる。列挙時に合成すれば exact パスのリテラルは全て厳密なままで、`count()` は「既知カードに整合する元の手の数」、`log_prob = −ln count` はその席の手の条件付き密度になる。プレイ途中のサンプリングを同じ機構で厳密に扱える。

**仕様との差分**: 仕様 §4 の「既知カードを除外した状態でそのスートに必要な枚数が残っているかを確認する」は、実現不能なシェイプの重みが 0 になることで自動的に満たされる。`sample(&self, rng, excluded)` のシグネチャは互換 API として残す。

**影響するクレート**: `bridge-constraint` (`KnownCards` と `KnownCards::with_play`, `Sampler`)、`bridge-play` (`hard_constraints` が `with_play` で既出カードを集める)、`bridge-sample` (`SampleContext.known`)。

## D4. `Atom` の否定は排他的連鎖

**決定**: `¬(L1 ∧ … ∧ Ln) = ⋁_j (L1 ∧ … ∧ L(j−1) ∧ ¬Lj)` で **互いに素な** Atom に分解する。`¬(shape ∈ S) = Sᶜ`、`¬[lo, hi] = [0, lo−1] ∨ [hi+1, max]`。結果は高々 `3 + 2(|cards| + |eval|)` 個。

**理由**: 項が素なので和集合のサイズが `Σ|項|` で厳密に求まり、`log_prob` に多重度補正が要らない。多重度 `m(h)` の補正が必要なのは利用者が書いた `Or` だけになる。「否定したリテラルだけを持つ Atom」の方が小さいが重なるので採らない。

**仕様との差分**: 仕様 §4 の「`cards` の否定は `Atom` に否定フラグ付きで保持する」は不要になる。`CardRequirement.count` の範囲 (`0..=0` で「持たない」) が否定を範囲の補集合として表現する。

**影響するクレート**: `bridge-constraint` (`Atom::negate`, `to_dnf`)。

## D5. `Shape`/`ShapeClass`/`ShapeSet` を `bridge-core` に置く

**決定**: 3 型と `SHAPES`/`CLASSES`/`CLASS_OF`/`shape_index`/`MIN_HCP`/`MAX_HCP` は `bridge-core` の `shape.rs` に置く。同じ理由で、ダブルダミー表 `DdTable` (`[[u8; 4]; 5]`、`[strain][declarer]`) も `bridge-core` の `dd_table.rs` に置き、`bridge-dds` とファサード `bridge::dd` が再エクスポートする。

**理由**: `bridge-play` がショウアウトからのハード制約を作るときに `ShapeSet::from_suit_lens` を使い、`bridge-format` も `Hand::shape()` を表示する。`bridge-constraint` に置くと `bridge-play` の依存が増え、`bridge-core` は依存ゼロで置けるため下に置く方が自然。`DdTable` は `bridge-dds` が生成し `bridge-format` が PBN の `OptimumResultTable` から読むが、両クレートは互いに依存せず共に `bridge-core` にだけ依存するので、共通の置き場は `bridge-core` しかない。

**仕様との差分**: 仕様 §3 は `Shape` と `ShapeClass` を L0 に置いており、`ShapeSet` と `DdTable` の置き場は明示していなかった。

**影響するクレート**: `bridge-core`、`bridge-constraint`、`bridge-play`、`bridge-format` (`GameView.dd_table`)、`bridge-dds`、`bridge` (`dd::DdTable`)。

## D6. `BidChoice` を enum に統一

**決定**: `pub enum BidChoice { Chosen(Chosen), NoCandidate(NoCandidate) }`。`Chosen { call, node: Option<NodeId>, source: ChoiceSource, explanation, alternatives: Vec<Alternative>, diagnostics }`、`NoCandidate { tried: Vec<Tried>, diagnostics: Vec<Diagnostic> }`。

**理由**: 仕様 §6 は struct、§9 は enum で書かれており矛盾している。「`NoCandidate` はエラーでなく戻り値」(仕様 §9) を型で保証するには enum が必要。

**仕様との差分**: 仕様 §6 の `pub struct BidChoice { call, node, explanation, alternatives }` を廃止。`alternatives` は `Vec<Alternative { call, node, priority }>` に構造化する。

**影響するクレート**: `bridge-bidding`、`bridge-sample` (`replay` の gap 記録)。

## D7. BML パーサは自前で Rust (winnow) 実装

**決定**: BML パーサ・展開・トライ構築は Rust で自前実装する。Python 版 `bml.py` / `bss.py` は参照実装として読み、gpaulissen/bml のテストデータと **期待 `.bss` 出力を展開結果のオラクル** に使う。

**理由**: ランタイムに Python を持ち込まない (wasm でも動く)。変数束縛 (`M`/`m`/`X`/`Y`/`Z`、`step`) の意味論は `bss.py` に依存するが、`.bss` オラクルがあるので移植の正しさを機械的に検証できる。

**仕様との差分**: 仕様 §11 の未決事項「BML パーサを自前で書くか、Python 実装を参照実装として移植するか」を「自前 + オラクル」で決定。Python 実装との意図的差分 (インデント単位の推定、末尾記号なし履歴行、Exact 行の優先、`2S/3H`/`4D/H`/`nX` の受理、行単位の回復) は `06-system.md` に列挙する。

**影響するクレート**: `bridge-system`、`xtask` (`systems fetch`)。

## D8. ナチュラル推定の精度測定

**決定**: 正解データが無い問題に対し、3 種の擬似正解で測る。(1) 実システム定義の非人工ノードを隠して `infer` と比較 (各 1,000 手サンプルで precision / recall / 体積比、`Role × CallKind` 別)、(2) `C_nat` からサンプルした手を `choose_bid` で再生して一致率 (再現率)、(3) コーパス実手の充足率 × 体積のパレート。`--ignored` 統合テストで JSON 出力する。

**理由**: システム定義とコーパスは「人間が正しいと認めた」データであり、ナチュラル推定の擬似正解として使える。3 指標を併用することで「緩すぎ」(充足率は高いが体積が大きい) と「厳しすぎ」(体積は小さいが再現率が低い) の両方を検出できる。

**仕様との差分**: 仕様 §11 の未決事項「ナチュラル推定モジュールの精度をどう測るか」を決定。

**影響するクレート**: `bridge-system` (`natural.rs`)、`bridge-bidding` (再生)、`xtask` (レポート)。

## D9. システム定義の配布形式

**決定**: **BML ソースを配布し、ロード時にコンパイルする**。`postcard` でシリアライズした `SystemIR` は、`blake3(解決済みソース ‖ compiler_version ‖ ir_format ‖ options)` をキーにした任意のキャッシュ (`SystemCache::load_or_compile`) であり、配布形式ではない。

**理由**: 仕様 §9 の「BML 1 システムのコンパイル 1 秒未満」が満たせるので毎回コンパイルで足りる。BML のまま配れば利用者が自分のシステムを編集して持ち込める (仕様 §5 のバージョニングの前提)。コンパイル済み IR を配ると `ir_format` の互換性管理が配布側に移り、`Custom` を含まない保証も配布側の責務になる。

**仕様との差分**: 仕様 §11 の未決事項「BML のまま配るか、コンパイル済み `SystemIR` を配るか」を決定。

**影響するクレート**: `bridge-system` (`cache.rs`)、`systems/` ディレクトリ。

## D10. DDS 連携は v2.9.0 のベンダリング + `cc` + 手書き `#[repr(C)]`

**決定**: dds-bridge/dds の **v2.9.0** (Apache-2.0、Makefile 版、27 ファイル、C++11) を `crates/bridge-dds/vendor/` に取り込み (`cargo xtask dds vendor`、コミットしない)、`build.rs` の `cc::Build` で `-std=c++11 -O3 -DDDS_THREADS_STL` でビルドし、ソースが無ければ `cfg(dds_vendored)` を立てずに FFI を除外する (全関数が `DdsError::Unavailable`)。バインディングは手書き `#[repr(C)]` + `unsafe extern "C"` (`src/sys.rs`) とし、`build.rs` が `src/layout_probe.cpp` (各構造体の `sizeof`/`offsetof`) も compile して `tests/layout.rs` で `size_of`/`offset_of!` と突き合わせる。`bindgen` は `cargo xtask dds regen-bindings` のオフライン生成にだけ使う。ファサードは `bridge::dd::{DdTable, DoubleDummy, DdError, dds()}` を `bridge-core` の型だけで定義し、`dds` feature の有無でトレイトの形を変えない。

**理由**: 上流は Bazel + hermetic LLVM 専用の DDS3 (v3.x) に移行しており、レガシー C API は `@deprecated`。利用者に Bazel も libclang も要求しないため v2.9.0 を固定する。レイアウトのずれは C++ 側のプローブで検出する。

**仕様との差分**: 仕様 §2/§9 の「`bindgen` + `cc`」から、ビルド時 `bindgen` を外す。

**影響するクレート**: `bridge-dds`、`bridge` (feature `dds`)、`xtask`。

## D11. `Interpretation` の選言数は席ごとに上限 K=8

**決定**: `interpret` の Step B (席ごとの AND = 直積) で、`is_satisfiable` で剪定し、`(node, kind)` 列で重複除去した後、重み降順で **K=8** に切り詰めて正規化する。`InterpretOptions.max_alternatives` で変更可。

**理由**: 同じ席の複数コールがそれぞれ複数の意味を持つと、AND の直積で代替が指数的に増える。上位 K を残せば重みの大半を保ちつつ下流のサンプラーのコストを有界にできる。

**仕様との差分**: 仕様 §6 は「重み付き選言」とだけ述べ、上限を定めていない。

**影響するクレート**: `bridge-bidding`、`bridge-play` (複数イベントの結合も同じ直積・剪定・K=8)、`bridge-sample`。

## D12. 並列化の再現性

**決定**: サンプル `i` の RNG を `(master_seed, i)` から splitmix64 で 32 バイトの種を生成し `Xoshiro256PlusPlus::from_seed` で作る。`sample_deals` はサンプル `i` を `rng_for(seed, i)` だけから計算し、`parallel` では `into_par_iter().map(..).collect()` で順序を保つ。`Threads::Single` と 7 スレッドプールでバイト一致をテストする。

**理由**: 仕様 §7 の「単一スレッド実行と同じ結果を出すモード」を、モードではなく常時の性質にする。`SmallRng` は wasm32 で別アルゴリズムになるためターゲット間で結果が変わる。`Xoshiro256PlusPlus` を明示すればスレッド数・ターゲットに依らず同一結果。

**仕様との差分**: 仕様 §7 の `SmallRng` を `Xoshiro256PlusPlus` (`rand_xoshiro` 0.8) に変える。

**影響するクレート**: `bridge-sample`。

## D13. 数値型

**決定**: サンプラーの個数・重み・累積和は **`u64`**。`distribution_points` は **`i8`**、`total_points` は 0 で飽和させた `u8`。

**理由**: `C(52, 13) = 6.35×10^11`、シェイプ別の個数は最大 `1.67×10^10` で `u32` (4.29×10^9) を超える。中間値 `n01 · BoxSum23` は `8.7×10^12` に達するが `u64` に 6 桁の余裕がある。Bergen の adjust-3 は `−1` になるので `u8` では表現できない。

**仕様との差分**: 仕様 §4 の `distribution_points(hand, method) -> u8` を `i8` に変更。

**影響するクレート**: `bridge-eval`、`bridge-constraint`。

## D14. 依存バージョン

**決定**: `winnow` 1.0、`rand` / `rand_core` 0.10、`rand_xoshiro` 0.8、`rayon` 1.12、`criterion` 0.8、`proptest` 1.11、`thiserror` 2、`cc` 1.4、`bindgen` 0.73 (xtask のみ)。

**理由**: crates.io の現行版。仕様の `winnow` 0.7 は旧版。`rand` 0.8 の `gen()` は edition 2024 で予約語 `gen` と衝突する。`rand_core::Rng` が dyn 互換の基底トレイトなので `PreparedProposal::propose(&self, rng: &mut dyn rand_core::Rng)` と書ける。

**仕様との差分**: 仕様 §2 の `nom または winnow` を `winnow` 1.0 に固定。

**影響するクレート**: 全クレート (`[workspace.dependencies]`)。

## D15. 解釈の低信頼度は ε-混合

**決定**: `Partial` / `Natural` の解釈は制約を「緩める」のではなく、全代替に `(1 − ε)` を掛け、防御枝 `(ANY, ε, Fallback)` を加える **ε-混合** で表す。`eps_exact = 0.02`、`eps_partial = 0.15`、`eps_natural = 0.30`。`strict` モードでは ε = 0 で防御枝なし。

**理由**: サポートが空にならないので配牌サンプリングが必ず進み、重点重み (ビディング尤度) が事後に補正する。「HCP を 2 広げる」のような場当たりな緩和は、どの程度広げるかに根拠が無く、双方向整合性テストでも検証できない。`satisfied_by` は Fallback 枝を無視するのでプロパティテストの厳密性は保たれる。

**仕様との差分**: 仕様 §5 の「部分一致: 直近のビッドのみ解釈し、それ以前は制約を弱める」の「弱める」を操作的に定義したもの。L3 は各コールにそのノード自身の制約だけを使い、分岐点より前は Exact のまま維持し、分岐点以降は `eps_partial` で信頼度を下げる。

**影響するクレート**: `bridge-bidding`、`bridge-sample` (Fallback 枝の重み)。

## D16. BML 拡張

**決定**: メタ行 `#+KEY: value` (`STRENGTH`, `NATURAL`, `DISTPOINTS`, `TIEBREAK`, `BALANCED`, `CONVENTION`, `RECOGNITION`, `VERSION`) と、説明文末尾の `{prio:N}` / `{w:X}` 注釈を追加する。

**理由**: 既存の BML ツール (`bml.py` 等) は `#+` 行と `{…}` を無視するか説明文として扱うので、拡張を含むファイルも既存ツールで処理できる。`SystemMeta` の評価方式・強さ語彙・タイブレークを BML 内で宣言でき (仕様 §4 の「システム定義がどの方式を前提とするかを宣言できる」)、`priority` と枝重みを著者が制御できる。

**仕様との差分**: 仕様 §5 は BML の採用を定めているが拡張には触れていない。

**影響するクレート**: `bridge-system` (パーサ、説明文コンパイラ、`SystemMeta`)。

## D17. 席の条件

**決定**: `#SEAT` は **オープナーの席位置** (1〜4、`12`/`34` も可) の条件とし、先頭のパスは経路 (`Node.calls`) に含めず `SeatCond` で表す。`LookupKey.calls` は先頭パスを除去し、`opener_pos` を別に持つ。

**理由**: 実ファイルと `.bss` の挙動で確認した BML の意味論に合わせる。先頭パスを経路に含めると同じ表を 4 席分展開することになり、ノードが 4 倍になる。

**仕様との差分**: 仕様は席条件の扱いを定めていない。

**影響するクレート**: `bridge-system` (`SeatCond`, `LookupKey::for_auction`, 展開)、`bridge-bidding` (`LookupKey::for_auction` / `SystemIR::resolve` の利用)、`bridge-core` (`Auction::leading_passes`, `Auction::position_of`)。

---

## 仕様の未決事項との対応

| 仕様 §11 の未決事項 | 決定 |
| --- | --- |
| BML パーサを自前で書くか、Python 実装を移植するか | D7 |
| ナチュラル推定モジュールの精度をどう測るか | D8 |
| システム定義の配布形式 | D9 |

## 未決

- 未決: 本設計で「未決」と記した項目 (各文書末尾) は D18 以降として本文書に追記する。決定時にはフェーズと PR 番号を添える。
