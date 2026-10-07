# 13. 決定記録 (D1〜D20)

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
| D11 | `Interpretation` の選言数 | 席ごとに上限 K=8（フェーズ 4 で切り詰めを質量順 + 受け皿保持に改訂） |
| D12 | 並列化の再現性 | `(master_seed, i)` から splitmix64 で派生、`Xoshiro256PlusPlus` |
| D13 | 数値型 | 個数・重みは `u64`、`distribution_points` は `i8` |
| D14 | 依存バージョン | `winnow` 1.0、`rand` 0.10、`criterion` 0.8 |
| D15 | 解釈の低信頼度 | ε-混合（フェーズ 4 で方策の床 ε/n に一本化） |
| D16 | BML 拡張 | `#+KEY:` メタと `{prio:N}` / `{w:X}` 注釈 |
| D17 | 席の条件 | `#SEAT` はオープナーの席位置、先頭パスは経路に含めない |
| D18 | ビディング方策 | 決定的なシステム選択 + ナチュラル逸脱 δ + 一様床 ε。priority ソフトマックスは廃止し、比較用の `legacy_temperature` もフェーズ 6 の評価の後に削除した |
| D19 | 解釈 | 方策の鏡像。システムの排他領域は派生索引 `ExclusiveIndex` に前計算 |
| D20 | 評価手順 | 固定フィクスチャ、コーパスの調整用 / 評価用分割、ESS では調整しない |

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

**改訂 (フェーズ 4)**: K = 8 は維持する。ただし切り詰めは重み w の降順ではなく、見積り質量 `w · cells(要約)` (要約の箱に入る (シェイプ, HCP) セルの数) の降順で行う。提案は成分を `w · count` に比例して引くので、目標質量の小さい組合せから捨てる方が偏りが小さい。さらに、全コールで `ANY` 片を取った受け皿の組合せ (重み Π ε/n) を必ず残す。これで提案の支持集合が目標の支持集合を覆う。重複除去のキーは `(node, kind, branch, 片種別)` の列にする。詳細は 07-bidding.md §4.4。

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

**改訂 (フェーズ 4、D19)**: ε は方策の一様床 ε/n に一本化する。`eps_exact` / `eps_partial` / `eps_natural` と `lenient_decay` は既定の経路から退役させ、`InterpretOptions::legacy()` (`InterpretMode::Legacy`) として残す (当初は ESS の前後比較用に 1 フェーズだけの予定だったが、再現 (iii) などが読むので (iii) を退役させるまで残す。D18 の「残したもの」、07-bidding.md §9 の 8)。「信頼度が低いほど防御枝を重くする」役割は、δ (システム外への逸脱) と、方策上選ばれないコール (shadowed) の Fallback 片に移る。支持集合が空にならない性質は `ANY` 片 (重み ε/n) が保つ。`strict` の意味 (Fallback 片をすべて落とす) は変わらない。

## D16. BML 拡張

**決定**: メタ行 `#+KEY: value` (`STRENGTH`, `NATURAL`, `DISTPOINTS`, `TIEBREAK`, `BALANCED`, `CONVENTION`, `RECOGNITION`, `VERSION`) と、説明文末尾の `{prio:N}` / `{w:X}` 注釈を追加する。

**理由**: 既存の BML ツール (`bml.py` 等) は `#+` 行と `{…}` を無視するか説明文として扱うので、拡張を含むファイルも既存ツールで処理できる。`SystemMeta` の評価方式・強さ語彙・タイブレークを BML 内で宣言でき (仕様 §4 の「システム定義がどの方式を前提とするかを宣言できる」)、`priority` と枝重みを著者が制御できる。

**仕様との差分**: 仕様 §5 は BML の採用を定めているが拡張には触れていない。

**影響するクレート**: `bridge-system` (パーサ、説明文コンパイラ、`SystemMeta`)。

**補遺 (フェーズ 4、SAYC の厚み付け)**: ユーザーは「BML の記法が足りなければ拡張してよい」とした。フェーズ 4 の SAYC は記法の硬さの症状を示していた: 我々の停止を表すパスの鎖 2,467 か所 (システム停止 `#STOP` / `{stop}` に置換済み、`06-system.md` §4.5)、相手のスートやレベルだけが違うほぼ同一のリテラル表 (P10 の 6 組 18 表)、書くと高くつくので既定のパスに任せた位置。これに対して次の拡張 3 つ (1〜3) と書き換え 1 つ (4) を加える。

1. **相対レベル `c` / `j`** (`06-system.md` §4.6): コールトークンのレベルに「そのストレインの最低の十分なレベル」と「その 1 つ上」を書ける (`cS`、`cM`、`jY`)。**D16 の原則からの逸脱**: `bml.py` の `bid_type()` はこれをレベルとして読まない (既存の `nX` と同じ扱い)。コールトークンは本文ではなく木の構造なので、既存ツールに「無視させる」形では書けない。そこで、トークンを見たら必ず `NonStandardToken` (Info) を出し、相対レベルで段落を始めさせない (散文を表と誤読しない) ことで、可搬性の損失を著者に見えるようにする。
2. **スート長の比較** (`06-system.md` §7.4): 説明文の `!s>=!h`、`M>oM` などを、2 スートの枚数の大小関係のシェイプ集合にコンパイルする。説明文の中なので既存ツールは本文として表示する (D16 の原則どおり)。

1 と 2 は既存の系を変えない (実ファイルの parse / lint の件数、`.bss` オラクルは不変)。`COMPILE_REVISION` を 6 に上げる (4 と 5 はシステム停止のレビュー修正が使った)。

3. **`#ANYORDER`** (`06-system.md` §4.7、`COMPILE_REVISION` 7): 表の指示子。その表の新しい `X`/`Y`/`Z` の束縛から `X < Y < Z` の順序を外す (未使用・相互に異なる、は保つ)。相手のスートを束縛変数として書く競り合いの表で、上下の書き分けやスートごとのリテラル表を不要にする。**D16 の原則からの逸脱**: 既存ツールはこの行を知らず、`bml.py` では順序つきの (こちらの部分集合の) 展開になるか未知の指示子になる。`#STOP` と同じ扱いとし、効果の無い使い方には `AnyOrderWithoutVariables` (Info) を出す。
4. **P10 の書き換え** (拡張ではない): P10 のリテラル表を、相対レベル `cM`/`cm` と既存の変数でまとめた: 負のダブルと相手のレイズの後の 12 表を 4 表 (`1m-(2Y)-D-(3Y)-`、`1M-(2m)-D-(3m)-` と、メジャー対メジャーのリテラル 2 表) に、相手の 1 レベルオーバーコールのレイズの後のリオープンの 6 組 18 表を 4 表 (`1X-(1Y)-P-(2Y)-` とその下の 2 表、`1H-(1S)-P-(2S)-D-(P)-` のマイナー) に。コンパイル結果のノード集合 (8,834 位置の呼び、側、条件、説明、制約、優先度、フラグ、子の数) は書き換え前と同一 (`systems/sayc/NOTES.md` #P12)。

調査 (`systems/sayc/NOTES.md` #P12) の分類 (ii)〜(v) のうち、ここまでの拡張で書けないまま残るのは (iii) の一部である: 相手の任意の競り (`(any)`/`(bid)`) の下で、そのレベルに応じたコール (相対レベル) を書くこと。これは `LevelWithoutAnchor` のとおり基準のビッドが不明なので、ワイルドカードを具体的なコールへ展開する仕組みが要る。今回は加えず、該当位置の件数を NOTES に記録する。

5. **`#EXACTPASS` / `#EXACTPASS FILE`** (`06-system.md` §4.8、`COMPILE_REVISION` 10、フェーズ 4 のレーン guard): 表の指示子と、「同じファイルの後の全ての表」に効く独立の段落の形。表の我々の行の直前の相手のパスを厳密にし、そこでの相手のほかのコール (どの表も停止も辺を与えていないもの) を、`resolve_lenient` がパスと読み替える代わりにシステム外にする (全ての表と停止の後に、行の無い `(any)` を他の辺の後に加える)。SAYC が機械的に並べていた 121 行の `…-(any)-` (`interference-guards.bml`) を置き換え、`xtask coverage` の指標は全て同一だった (`systems/sayc/NOTES.md` #P16)。範囲はファイル (include の 1 回分) の後の表に限り、include の先にも元にも及ばない。システム全体のメタ (`#+KEY:`) にしなかったのは、レーン D2 より前のファイルが今も lenient な読みに頼る位置を持つため (`tests/sayc.rs` の上限) で、守る範囲を著者が表かファイルの単位で選べるようにした。**D16 の原則からの逸脱**は `#STOP`・`#ANYORDER` と同じである (上流では表の中の未知の行、未知の指示子)。無視されても辺が加わらないだけで、既定の「割り込みをパスと読む」動作に戻る。効果の無い使い方には `ExactPassWithoutPass` (Info)、置き場所の誤りには `UnknownDirective` (Warning) を出す。

## D17. 席の条件

**決定**: `#SEAT` は **オープナーの席位置** (1〜4、`12`/`34` も可) の条件とし、先頭のパスは経路 (`Node.calls`) に含めず `SeatCond` で表す。`LookupKey.calls` は先頭パスを除去し、`opener_pos` を別に持つ。

**理由**: 実ファイルと `.bss` の挙動で確認した BML の意味論に合わせる。先頭パスを経路に含めると同じ表を 4 席分展開することになり、ノードが 4 倍になる。

**仕様との差分**: 仕様は席条件の扱いを定めていない。

**影響するクレート**: `bridge-system` (`SeatCond`, `LookupKey::for_auction`, 展開)、`bridge-bidding` (`LookupKey::for_auction` / `SystemIR::resolve` の利用)、`bridge-core` (`Auction::leading_passes`, `Auction::position_of`)。

## D18. 方策は「決定的なシステム選択 + ナチュラル逸脱 δ + 一様床 ε」

**決定**: `call_distribution` を次の式に改める (フェーズ 4、07-bidding.md §6.1)。

p(c | h) = (1 − ε) · [(1 − δ) · S(c | h) + δ · M(c | h)] + ε / n

- S はシステムの決定的な選択 (`choose_bid`)。候補が無ければ一様 1/n。
- M はナチュラルの決定的な選択 (順位順のナチュラル候補で最初に満たしたもの、無ければナチュラルの暗黙 Pass)。
- システム外の位置では π = M (δ に依らない)。
- `PolicyParams { epsilon, deviation }`。プリセットは 2 つを事前に固定する (フェーズ 6 の評価までは、第 3 のフィールド `legacy_temperature` があった。下の「決定済み」を参照)。
  - `system_players()` (ε = 1e-3、δ = 0) は SAYC の生成オークション用。
  - `human()` はコーパスの調整用分割で最尤推定した (ε, δ) で、コーパスとリードアドバイザ用。値は 12-roadmap に記録する。フェーズ 4 の統合 (D3・len・perf のマージ後) で、その SAYC の最尤推定値 ε 0.3404、δ 0.3959 に設定した (それまでは仮置き ε = 0.01、δ = 0.3)。フェーズ 4 の完了時の最尤推定値は ε 0.3420、δ 0.3943 で、調整用 ln L の差が 0.02 しかないので据え置いた (12-roadmap「フェーズ 4 の完了」)。
- 温度付き priority ソフトマックスは `legacy_temperature = Some(τ)` (`PolicyParams::legacy(τ)`) としてだけ残し、フェーズ 6 のリード評価で比較した後に削除する、としていた。**決定済み (フェーズ 6 の評価の後、2026-10-02): 削除した** (コード 377c93c)。
  - 根拠 (14-lead.md §4.3、評価分割 100 ボード、seed 0。`#1` の hard と `#3` の `LEAD_POLICY=legacy1`): hard は τ = 1 のソフトマックスと上位 1 / 3 の命中率が同じ (0.78 / 0.93 対 0.78 / 0.92)、上位 1 の実測損失は 0.28 対 0.27、ESS 中央値は 78.1 対 26.1、ESS < 5 のボードは 1 対 13。再検討条件 (hard が上位 1 で 0.02 を超えて悪い) には当たらず (差 0.00)、D18 を維持する。ただし命中率の差は 100 ボード 1 セットの雑音の範囲なので、「hard のほうが命中率が高い」ではなく「同等で ESS が大幅に良い」ことが削除の根拠である。
  - 削除したもの: `PolicyParams::legacy_temperature` と `PolicyParams::legacy(τ)`、`call_distribution` のソフトマックスの経路 (`legacy_distribution`、`logsumexp`)、`AuctionPolicy` の参照実装への委譲、`InterpretCache` のキーの `legacy_temperature` の枠、リード評価ハーネスの `LEAD_POLICY=legacy1` (と、鏡像が無いので `human` の鏡像で代用していた分岐)、試験 `legacy_temperature_restores_the_priority_softmax`。`PolicyParams` のほかの部分と、hard の経路は変えていない (残る試験の結果は一致)。
  - 残したもの: `InterpretMode::Legacy` / `InterpretOptions::legacy()` (D19、フェーズ 3 の解釈)。再現 (iii) (フェーズ 3 の継続指標。11-testing.md §3) と `review_regressions.rs`、`unit.rs`、リード評価の `LEAD_INTERPRET=legacy` (フェーズ 3 の解釈との比較) が今もこれで読むので、(iii) を退役させるまで残す (07-bidding.md §9 の 8)。方策の形 (D18) とは独立なので、この決定では動かさない。
- 同じコールを持つ候補が複数あっても (Exact 辺と Class 辺など)、選択は「最初に満たした候補のコール」であり、質量は足さない。

**理由**:
- BML の priority は整列のための小さな整数で、対数オッズとして較正されていない。
- τ = 1 のソフトマックスは、1NT と 1C の両方を満たす手が 27% で 1C を開くといった分布を与える。これはシステムの意味にも `replay` にも一致しない。
- δ < 1/2 なら argmax_c p = `choose_bid` が τ に依らず構造的に成り立つ。`policy_argmax_matches_choose_bid` は両プリセットで 10^5 局面 100%。
- 決定的な S と M は、解釈を方策の正確な鏡像にできる (D19)。

**退けた案**:
- τ = 1 の priority ソフトマックス: 較正されていない整数を対数オッズとして扱う。shared の分割は replay と矛盾する。
- τ = 0.1〜0.2 の rank / priority ソフトマックス: 2 位の候補に e^{−1/τ} の質量が残るので、鏡像が複雑になるか under-cover する。
- `tie_gap`: パラメータが 2 つになり、しかも ESS スイートで調整されていた。

**帰結**: δ = 0 では、システム内の位置でのシステム外コールは手について情報を持たない (尤度が手に依らない)。これはモデルどおりである。人間のオークションは δ > 0 のプリセットで読み、システム外のコールをナチュラルに読む。

**仕様との差分**: 仕様 §6 は確率的方策の形を定めていない。フェーズ 3 の実装 (priority/τ のソフトマックス) を置き換える。

**影響するクレート**: `bridge-bidding` (`policy.rs`、`choose.rs`)、`bridge-sample` (尤度)、`bridge-lead` (プリセットの選択)。

## D19. 解釈は方策の鏡像。システムの排他領域は派生索引として前計算する

**決定**: `interpret` の Step A は、各コール c について「p(c | h) が一定になる手の集合」の片を作る (07-bidding.md §4.1)。

- 片は X_c^(b)、N^sys、Y_c、N^nat、ANY の 5 種。
- 生の重みは D18 の式から導く。
- `CallInterpretation.log_scale = ln Σ raw` を記録する。これで p(c | h) = exp(log_scale) · Σ_i w_i · 1[h ∈ C_i] が成り立つ。
- cards / eval リテラルを持つ片は上側近似になり得るので、over-cover は許す。under-cover は常に禁止する。

システム片について:
- システム片 X_c は「rank 順で最初に満たした兄弟のコールが c」になる手の集合で、次の式で求める。
  X_c = ∪_{m: call(m) = c} (C_m ∧ ¬∪_{m' が上位, call(m') ≠ c} C_{m'})
- 比較関数は `bridge_system::exclusive::rank_cmp` の 1 つだけを使う。順は priority 降順 → tie_break → コール index 昇順。
- BML の `{w:}` 枝重みは提案密度には使わない。説明文にだけ残す。
- 排他は `bridge-system` の派生索引 `ExclusiveIndex` に前計算する。
  - 格納先は `OnceLock` で `#[serde(skip)]` とし、`compile()` の最後に構築する。
  - 直列化しないので `IR_FORMAT` は変わらない。
- 上位の兄弟が接頭辞の後で非合法な位置では、合法な兄弟だけから実行時に計算し直す。

ナチュラル片とそれ以外:
- ナチュラル片 Y_c は、実行時に厳密なシェイプ × HCP グリッド (`HcpShapeGrid`) で計算する。形は 2 つ持つ。
  - 提案用の平坦な上側近似。
  - 所属判定と尤度用の厳密な木。
- 方策が決して選ばないコールは `shadowed = true` とし、Fallback 片だけで読む。
- `InterpretOptions::for_context(&BidContext)` は、`PolicyParams` と `implicit_pass` を尤度と同じ `BidContext` から取る。
- 方策のナチュラル推定器は `ctx.natural`、それが `None` なら `table.natural` とする（`call_distribution`・`sequence_log_likelihood`・`AuctionPolicy` で共通）。鏡像は `table.natural` で読むので、鏡像と方策が一致する前提は `ctx.natural` が `None` か `table.natural` であることである。別の推定器は `Table` に入れて渡す。
- 高速尤度 `AuctionPolicy` は、オークションごとに片を所属判定形で組み立て、配牌ごとには所属判定だけを行う。参照実装 `sequence_log_likelihood` との差は |Δ ln L| ≤ 1e-5 を保証する。

**理由**:
- 片の重みを方策の密度に較正して初めて、サンプラーの件数比例の成分抽選が q ∝ L (席因子) を与える。
- プロトタイプ (50 オークションの ESS スイート、ESS/n の中央値) の比較。
  - 基準は 0.036。
  - 排他のみ (A) は 0.086。
  - 実行時排他 + ε 調整 (C) は 0.301。
  - 方策鏡像 (B) は 0.426、残差棄却を加えて 0.906。
- 締まり (排他) は必要だが、それだけでは足りない。
- shadowed コールをノード全体で読み直す案は、プロトタイプ C でコーパス ESS を 0.35 から 0.15 に下げた。

**退けた案**:
- 実行時だけの排他: `Table` に無界のキャッシュが要り、`Table` の API も壊れる。コールドで 134〜276 μs かかる。
- IR への直列化: postcard の SAYC IR が 825 KB から 1.24 MB (1.51 倍) に膨らむ。
- τ = 1 のままの完全な鏡像 (プロトタイプ B): SAYC で 65〜205 μs かかり、K = 8 が拘束になる。

**実装上の決定 (フェーズ 4 レーン B)**:
- `Table` には隠しキャッシュも非公開フィールドも持たせない。`Table` の構造体リテラルはそのまま通る。
- 位置ごとの手に依らないデータは、`bridge-bidding` 内部のスレッドごと・有界のメモ (`memo.rs`) に置く。
  - 対象はシステム候補、ナチュラル候補の順位、パートナー文脈、コールごとのナチュラル片と説明文。
  - キーは (4 席のシステムとナチュラル推定器の `Arc` アドレス、`implicit_pass`、ディーラー、バル、コール列)。
  - エントリは各 `Arc` の `Weak` を持つので、エントリが生きている間は同じアドレスが別の表に再利用されない。
  - 容量は 1024 エントリ × 2 世代 (近似 LRU)。
  - ヒットは再計算と同一の値を返す。
- 副作用として、メモのエントリが残っている間は、表の `Arc` に対する `Arc::get_mut` が失敗する。
- `InterpretCache` は利用者が持つオークション単位のキャッシュとして残し、`get_or_policy` で `AuctionPolicy` も持てるようにする。

**仕様との差分**: 仕様 §6 の「重み付き選言」を、方策の密度に較正した選言として具体化する。仕様 §9 の「キャッシュは呼び出し側が持つ」について、`SystemIR` と `Table` は引き続き不変でキャッシュを持たない。例外は、手に依らない位置データのスレッドローカルなメモだけである (結果は再計算と同一で、観測できるのは速度だけ)。

**影響するクレート**: `bridge-system` (`exclusive.rs`、`Lookup.parent`、lint `ShadowedBranch` / `OverlappingBranches`)、`bridge-constraint` (`grid.rs`)、`bridge-bidding` (`exclusion.rs`、`auction_policy.rs`、`memo.rs`、`interpret.rs`)、`bridge-sample` (`BiddingLikelihood` が `AuctionPolicy` を使う)。

## D20. 評価手順

**決定**:
- ESS スイートの生成ケースは固定フィクスチャ (`tests/data/ess_cases.txt`) に凍結する。凍結は、レベル下限と SAYC の追加が入った後に 1 回行う。
- コーパスは `corpus_auctions` の列挙順で分割する。偶数番目が調整用、奇数番目が評価用である。
- δ、ε、`NaturalParams`、レベル下限は、調整用分割で尤度か一致率によって決める。ESS スイートでは決めない。
- ESS は ESS/n に加えて、試行あたりの ESS、受理率、壁時計時間、そのときの loadavg を報告する。
- 完了条件はすべて「整合したモデルで測る集合 (SAYC 生成)」と「コーパス」の両方で報告する (12-roadmap §8)。

**理由**: フェーズ 3 までの調整は ESS スイートそのもので行われ、評価と調整が分かれていなかった。方策を変えると生成ケースも変わるので、凍結しないと前後比較ができない。

**仕様との差分**: 仕様は評価手順を定めていない。

**影響するクレート**: `bridge-sample` (ESS レポート)、`bridge-bidding` (テストのコーパス読み込み)、`xtask`。

---

## 仕様の未決事項との対応

| 仕様 §11 の未決事項 | 決定 |
| --- | --- |
| BML パーサを自前で書くか、Python 実装を移植するか | D7 |
| ナチュラル推定モジュールの精度をどう測るか | D8 |
| システム定義の配布形式 | D9 |

## 未決

- 未決: 本設計で「未決」と記した項目 (各文書末尾) は D21 以降として本文書に追記する。決定時にはフェーズと PR 番号を添える。
