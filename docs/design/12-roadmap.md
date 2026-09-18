# 12. 実装順序とリスク

本文書はフェーズ 0〜6 を PR 単位のタスク表 (タスク id、PR の内容、証明となる完了基準) に分解し、フェーズごとの数値による完了条件と、計画 §14 のリスク一覧を確定する。今回の実行範囲は **フェーズ 0** (設計文書と骨格) までで、フェーズ 1 以降はレビュー後に別途着手する。各フェーズは前フェーズの完了条件を数値で満たしてから始め、フェーズ 3 で自作するテスト用システムは SAYC とする (2/1 はフェーズ 4 以降の任意追加)。

## 1. フェーズ 0: 設計の固定化 (今回の実行範囲)

### 1.1 タスク

| id | PR の内容 | 証明 / 完了基準 |
| --- | --- | --- |
| 0.1 | `docs/design/{README,01-workspace,02-core,03-format,04-eval,05-constraint,06-system,07-bidding,08-play,09-sample,10-dds,11-testing,12-roadmap,13-decisions}.md` を日本語で書き出す。README は索引と仕様→設計の対応表、13-decisions は D1〜D17 の理由付き一覧 | 14 ファイルが揃い、各文書が冒頭 2〜3 文の決定要約、型定義、数式、トークン表を含む。未確定事項は「未決」で明示 |
| 0.2 | `git init`、`.gitignore` (`target/`, `corpus/data/`, `systems/vendor/data/`, `crates/bridge-dds/vendor/`)。コミットはしない | `git status` が全ファイルを untracked として示す |
| 0.3 | ワークスペース骨格: ルート `Cargo.toml` (`[workspace.package]`, `[workspace.dependencies]`)、`rust-toolchain.toml`、`crates/*` 全 10 クレート (`bridge-core`, `bridge-format`, `bridge-eval`, `bridge-constraint`, `bridge-system`, `bridge-bidding`, `bridge-play`, `bridge-sample`, `bridge-dds`, `bridge`) の `Cargo.toml` + `src/lib.rs` とモジュールファイル (公開型・トレイト・関数シグネチャを英語 rustdoc 付きで宣言、本体は `todo!("phase N")`)、`xtask/src/main.rs` (サブコマンドの枠、全て「未実装」)、`.github/workflows/ci.yml`、`corpus/manifest.toml` (sha256 は空) と `systems/README.md`。`bridge-dds` は `build.rs` (ベンダリング検出、`cfg(dds_vendored)`)、`VENDOR.md`、`src/sys.rs`、`src/layout_probe.cpp` を含み、`vendor/` 未取得でもビルドが通る (`10-dds.md` §4) | 全クレートの公開面が設計文書の識別子と一致する (本ディレクトリの文書はコードに合わせて整合済み) |
| 0.4 | 検証: `cargo check --workspace --all-targets`、`cargo clippy --workspace -- -D warnings`、`cargo check --workspace --exclude bridge-dds --exclude xtask --target wasm32-unknown-unknown` (ターゲット未インストールなら `rustup target add` を提案して報告) | 3 コマンドがすべて通る |

骨格の実装方針 (計画からの差分):

- `todo!()` 本体の未使用引数・未使用フィールドの警告は、計画の `let _ = ...` ではなく、各クレートの `lib.rs` に置いた crate レベルの `#![allow(dead_code, unused_variables)]` で抑える (`bridge` ファサードだけは本体があるので不要)。この allow は本体を実装するフェーズ (`todo!("phase N")` の N) で外し、実装後に残る未使用はコードの整理で解消する。
- `const` / `static` 項目はコンパイル時に評価されるため `todo!()` にできない。したがって定数表 `SHAPES`、`CLASSES`、`CLASS_OF`、`ShapeSet` の定数 (`ALL`, `BALANCED`, `SEMI_BALANCED`)、`MAX_HCP`/`MIN_HCP`、`bridge_eval::SUIT` (8192 × 5 のスート表) と、それらが依存する `const fn` (`shape_index`, `Shape::class`, `ShapeSet::from_class` 等) は骨格の時点で完全に実装済みで、単体テスト (`shape.rs`, `tables.rs`) も付いている。フェーズ 1.2 / 2.1 のタスクはこれらの検証を追加するだけで、実装は要らない。

### 1.2 フェーズ 0 が納めるもの・納めないもの

| 納める | 納めない |
| --- | --- |
| 設計文書 14 本 (本ディレクトリ、コードと整合済み) | 動作するパーサ・サンプラー・コンパイラ・解釈器 (本体は `todo!()`) |
| git リポジトリの初期化と `.gitignore` | コミット (レビュー後にユーザーが行う) |
| 10 クレート + `xtask` の `Cargo.toml` と公開シグネチャ、`const` 表の実装 (`SHAPES` 等、`SUIT`) | テスト本体 (骨格に含まれるのは `const` 表の単体テストのみ)、ベンチの実行 (`benches/*.rs` は枠だけ) |
| `ci.yml`、`corpus/manifest.toml` (sha256 は空)、`systems/README.md`、`xtask/src/main.rs` の枠 | コーパスデータ、外部 BML、DDS ソース (`vendor/`) の取得 |
| `bridge-dds` の `build.rs` (ベンダリング検出付き)、`VENDOR.md`、`sys.rs` の宣言、`layout_probe.cpp` | DDS のビルド・レイアウトテストの実行 (`vendor/` が無いため cfg で除外) |
| `cargo check` / `clippy` / wasm check が通る状態 | `systems/sayc.bml` の中身 (フェーズ 3.5) |

## 2. フェーズ 1: 実データが読める (`bridge-core`, `bridge-format`)

| id | PR の内容 | 証明 / 完了基準 |
| --- | --- | --- |
| 1.1 | ワークスペース + CI + `xtask` を稼働させる (`xtask/src/main.rs` の `corpus fetch` を実装、CI の全ジョブが緑) | CI が 3 OS + wasm check で緑 |
| 1.2 | core: `Card`/`Holding`/`Hand` + `Display`/`FromStr` + `Shape`/`ShapeClass`/`ShapeSet` の `todo!()` 部分 (`suit_len`/`classes`/`factor`/`min_hcp`/`max_hcp`、イテレータ) と検証 (560/39/28、`shape_index` 全単射。表自体は骨格で実装済み) | unit / proptest 全通過 |
| 1.3 | core: `Seat`/`Call`/`Bid`/`Auction` (合法性、コントラクト導出)、`LegalCalls` の再エクスポート | 合法性 proptest 全通過 |
| 1.4 | core: `Deal`/`Board`/`PlayHistory`/`Vulnerability`/`DdTable::best_for` (トリック勝者、ボード番号からの導出)、`ShapeClass` の serde 派生 | unit 全通過、serde 往復 |
| 1.5 | format: Deal 文字列 (1 手・4 手・`PartialDeal`) | proptest ラウンドトリップ |
| 1.6 | format: PBN lexer/parser (lenient) + `Warning` モデル | スナップショット、fuzz ターゲット、パニック 0 |
| 1.7 | format: `GameView` (Deal/Auction/Play/Contract の型付き、`#`/`##`、`Note`、`AP`、`-`、`OptimumResultTable`) | 30 件のスナップショット一致 |
| 1.8 | format: PBN writer (export 形式) + proptest `parse_strict(write(g)) == normalize(g)` | proptest 10^4 ケース |
| 1.9 | `corpus/manifest.toml` (sha256 固定) + `cargo xtask corpus fetch` + パース率テスト | コーパス 1, 2, 4 で ≥ 99% のゲーム |
| 1.10 | format: LIN パーサ + `LinBoard::to_game()` | Vugraph 20 件で ≥ 95% のボードが Deal + Auction を返す、パニック 0 |

完了条件: PBN コーパスのパース率 ≥ 99%、PBN のラウンドトリップ (読み→書き→読み) が同値、LIN から BBO のハンド記録が読める。ここで止まって検証する (実データが入らない限り以降の層を検証できない)。

## 3. フェーズ 2: サンプラーが速い (`bridge-eval`, `bridge-constraint`, `bridge-sample` 骨格)

| id | PR の内容 | 証明 / 完了基準 |
| --- | --- | --- |
| 2.1 | eval: `distribution_points`/`shape_points` の実装 (スートテーブル `SUIT`、線形指標、`Half`、`DistMethod`/`LtcMethod` は骨格で実装済み) + 素朴実装との差分テスト + ベンチ | 全 8192 ホールディングで一致、`hcp` < 10 ns |
| 2.2 | constraint: `Atom`/`CardRequirement`/`EvalRequirement`/`satisfies`/交差/否定 (D4、排他的連鎖) | 否定の素性 proptest |
| 2.3 | constraint: DNF (`max_terms = 256`、`residual` 退避、`tracing::warn!`)、`is_satisfiable`、要約 (`hcp_range`/`shapes`/`suit_len`) | unit、爆発ケースで `truncated` |
| 2.4 | constraint: `Sampler` K=1 (シェイプ走査、HCP 窓、疎畳み込み、累積和) | 厳密カウント (`count() == 30 897 212 184`)、χ² 周辺分布、小プール全列挙 |
| 2.5 | constraint: 確定カード合成 (D3、`fixed`/`pool`) | 小プール全列挙で `count()` 一致 |
| 2.6 | constraint: K=2 追加特徴 + `residual`/α 推定 (棄却モード、`max_tries = 256`) | 全列挙一致、`is_exact()` の真偽 |
| 2.7 | constraint: `KnownCards` (`new`, `EMPTY`, `pool`, `needed`, `from_viewer`, `with_dummy`。`with_play` は 5.1) | unit |
| 2.8 | sample: `UniformProposal`、`WeightedDeal`、ESS、`SampleReport`、`rng_for` (splitmix64 → `Xoshiro256PlusPlus`)、`parallel` feature + 決定性テスト | `Threads::Single` と 7 スレッドでバイト一致 |

完了条件: 手書き制約 (「15-17 バランス」等) から ≥ 10^4 手/秒 (目標 10^5)、生成された手の HCP 分布とシェイプ分布が理論値と一致 (χ²)。システム定義はまだ不要。

## 4. フェーズ 3: オープニングだけ動く (`bridge-system`, `bridge-bidding`)

| id | PR の内容 | 証明 / 完了基準 |
| --- | --- | --- |
| 3.1 | system: BML lexer/parser (`#INCLUDE`、段落分類、クリップボード、行パース、木構築、回復) + AST スナップショット | `example1..6` + 実ファイル約 40 本で Error lint 0 |
| 3.2 | system: `CallPattern` 展開 + `AuctionTrie` (`resolve`/`children`/`resolve_lenient`) | `.bss` 期待出力と一致 |
| 3.3 | system: 説明文コンパイラ v1 (正規化 → 節文法 → Pass 1/2 → 組立 → `Recognition`) + 認識率レポート | jdh8 ≥ 0.65、gpaulissen ≥ 0.5 |
| 3.4 | system: lint (parse 系・展開系・制約系・カバレッジ系) | lint コードごとの unit |
| 3.5 | `systems/sayc.bml` (オープニング) 執筆 + `#+META` 拡張 (D16) | コンパイル < 1 s、Error lint 0、`Custom` 0 |
| 3.6 | bidding: 型 + `interpret` (Exact のみ、Step A/B、K=8) + ベンチ | `weights_sum_to_one` 等 unit、`interpret` < 10 μs |
| 3.7 | bidding: `choose_bid` (合法性 lint、priority、tie-break、`ImplicitPass::Complement`) | `illegal_call_is_lint`、合成システムのテスト |
| 3.8 | bidding: `call_distribution` / `sequence_log_likelihood` | argmax == `choose_bid` (τ = 0.01、10^5 局面) |
| 3.9 | bidding: `replay`、`InterpretCache` | unit |
| 3.10 | bidding: `forward_consistency` ハーネス + `coverage_report.json` | 10^6 配牌で違反 0 (SAYC オープニング) |
| 3.11 | bidding: Partial/Natural + ε 混合 + `strict` モード + system: `NaturalInference` (`classify`、規則表、`NaturalParams`) + 3 測定 | 意図的に切り詰めたシステムでのテスト、`natural_metrics.json` が出る |
| 3.12 | bidding: 再現率ハーネス | オープニングノード別に数値が出る |

完了条件: SAYC の BML からオープニング部分をコンパイルできる、双方向整合性プロパティテストが 10^6 配牌で違反 0、カバレッジの穴が `coverage_report.json` として出力される。ここでプロパティテストを書き、以降ずっと安全網にする。

## 5. フェーズ 4: データを厚くする (`bridge-system` のシステム定義)

| id | PR の内容 | 証明 / 完了基準 |
| --- | --- | --- |
| 4.1 | `xtask coverage`: コーパスのオークションを `interpret` し、全コール Exact の率と `EmptySupport` の有無を報告 | レポートが出る |
| 4.2 | `sayc.bml` レスポンス (1 レベル応答、NT 応答、レイズ、2/1 応答) | 整合性違反 0、`coverage_report.json` の穴が減る |
| 4.3 | `sayc.bml` オープナーのリビッド | 同上 |
| 4.4 | `sayc.bml` 競り合い (オーバーコール、テイクアウトダブル、ネガティブダブル、相手の介入後の応答) | 同上、`resolve_lenient` の使用率が下がる |
| 4.5 | (任意) `two-over-one.bml` | 同上 |
| 4.6 | ナチュラル推定の `NaturalParams` 調整 (3 測定の値を見て) | 3 測定の改善 |

完了条件: 実ハンドレコードのオークションの ≥ 80% が全コール Exact で解決される、残りがフォールバック階層で破綻せず (`EmptySupport` なし) 解決される、再現率の中央値 ≥ 0.6。このフェーズでコードを書いている時間が長いなら L1/L2 の設計が間違っている (増えるべきはデータ)。

## 6. フェーズ 5: サンプラー本番化 (`bridge-play`, `bridge-sample`, `bridge-dds`)

| id | PR の内容 | 証明 / 完了基準 |
| --- | --- | --- |
| 5.1 | play: `hard_constraints` (ショウアウト → 長さ確定、既出カード → `KnownCards`、整合性検査) + `KnownCards::with_play` | unit、`PlayWarning::Inconsistent` |
| 5.2 | sample: `ConstraintProposal` (`prepare`/`propose`/`log_prob`、席の並べ替え、成分重み) | 小プールで 10^5 提案のヒストグラム vs `exp(log_prob)` (χ²) |
| 5.3 | sample: `sample_deals` + ビディング尤度 + `SampleReport` + `tracing` INFO の ESS | ESS スイート 50 オークションで中央値 ≥ 0.5n |
| 5.4 | sample: ベンチ + プロファイル、性能対策 (a) 無制約席の直接配り (b) 対畳み込みの到達可能シェイプ限定 (c) 粗い項での提案 の選択 (R4) | ≥ 10^4 配牌/秒/コア |
| 5.5 | dds: `cargo xtask dds vendor` (2.9.0、`vendor/SHA256SUMS`)、`tests/layout.rs`、CI の `dds` ジョブ (`build.rs`、`sys.rs`、`layout_probe.cpp` は骨格で確定済み) | 3 OS でビルド、レイアウトテスト一致 |
| 5.6 | dds: 安全ラッパー本体 (`init`/`OnceLock`、`calc_dd_table(s)`、`solve_board` + スレッドスロット、`solve_all_boards` + `Mutex`、`analyse_play`、`dealer_par`、`info`) + 差分テスト | `list100.txt` で 100% 一致、8 スレッド並行テスト |
| 5.7 | ファサード `bridge::dd` の動作確認 (`dds` feature、`DoubleDummy` トレイト、`dds()` のフォールバック。骨格で実装済み) | wasm check が緑のまま、`dds()` が `None`/`Some` を返す |
| 5.8 | play: `lead_constraints`/`signal_constraints` の表と `interpret_play` の結合 (軟情報) + サンプラーの `play_soft` (フェーズ 6 には不要、ずれても可) | table-driven テスト |

完了条件: オークションから配牌をサンプリングして ESS が要求数の 50% 以上、DDS FFI が動きサンプル配牌の解析結果が返る。

## 7. フェーズ 6: オープニングリードアドバイザ (別クレート)

| id | 内容 | 証明 / 完了基準 |
| --- | --- | --- |
| 6.1 | ライブラリ側: `Solutions::AllRanked` によるリード評価 API (`DoubleDummy::lead_scores`)、PBN オークションからの `Interpretation`、ESS 報告 | unit |
| 6.2 | アプリ (別クレート): オークション + 自分の手 → サンプル配牌 → 各リードの DD 期待トリック → 上位 3 リード | コーパス 100 ボードで上位 3 リードに DD 最善が含まれる率を数値化 (閾値 X は測定後に決める。未決) |

理由 (仕様 §11): 判断が 1 手に閉じるため探索木を掘る必要がなく、ビディング情報の利用が本質そのもので、上級者でも間違えるため価値が実感できる。リード時点ではプレイ履歴が無いので `bridge-play` の軟情報は不要。

## 8. フェーズごとの数値完了基準 (まとめ)

| フェーズ | 数値基準 | 測るテスト (`11-testing.md`) |
| --- | --- | --- |
| 0 | `cargo check --workspace --all-targets`、`clippy -D warnings`、wasm check の 3 つが通る | CI `lint`, `test`, `wasm` |
| 1 | PBN パース率 ≥ 99%、ラウンドトリップ同値 (proptest 10^4)、LIN ≥ 95%、fuzz パニック 0 | §1 format 行、§8 |
| 2 | ハンド生成 ≥ 10^4/秒 (目標 10^5)、`count() == 30 897 212 184`、χ² 通過、eval 8192 一致、`hcp` < 10 ns、スレッド数不変 | §1 eval/constraint/sample 行、§4、§9 |
| 3 | `.bss` 一致、Error lint 0、認識率 jdh8 ≥ 0.65 / gpaulissen ≥ 0.5、コンパイル < 1 s、整合性 10^6 で違反 0、`interpret` < 10 μs、argmax 100%、`coverage_report.json` と `natural_metrics.json` が出る | §2、§5、§6、§9 |
| 4 | コーパスのオークション ≥ 80% が全コール Exact、残りに `EmptySupport` なし、再現率中央値 ≥ 0.6 | §3、§8 |
| 5 | ESS 中央値 ≥ 0.5n、配牌 ≥ 10^4/秒/コア、DDS 差分 100%、レイアウト一致、並行テスト通過 | §1 sample/dds 行、§7、§9 |
| 6 | 上位 3 リードの DD 最善命中率 (閾値 X 未決) | フェーズ 6.2 |

## 9. 各 PR と各フェーズの検証方法 (計画 §13)

1. 各 PR: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`、`cargo check --workspace --exclude bridge-dds --exclude xtask --target wasm32-unknown-unknown`。
2. フェーズ完了時: `cargo test --release --workspace -- --ignored` (整合性 10^6、ESS、コーパス、DDS 差分) と `cargo bench` (`hcp` < 10 ns、手サンプル ≥ 10^5/s、配牌 ≥ 10^4/s、`interpret` < 10 μs、BML コンパイル < 1 s)。
3. レポート成果物: `target/coverage_report.json` (NoCandidate 集計)、認識率レポート、ナチュラル推定精度 JSON、ESS レポート。

## 10. リスク (計画 §14)

| # | リスク | 対策 | 影響フェーズ |
| --- | --- | --- | --- |
| R1 | 公開 SAYC / 2-1 の BML が無い | フェーズ 3 で `systems/sayc.bml` を自作。Polish Club (jdh8) を早期テストに使う | 3, 4 |
| R2 | 散文的 BML の認識率が 20〜40% に留まる | lint + `NAT`/ナチュラル既定へのフォールバック。黙って捏造しない (未認識は `description` に残し `Recognition` で報告) | 3 |
| R3 | `GF`/`INV`/`MIN`/`MAX` の文脈連鎖で `assumed` が伝播 | `Provenance.assumed` と `AssumedContext` lint | 3, 4 |
| R4 | 配牌サンプラーの `prepare` コスト (席 2〜3 で 5〜12 回 × 20〜40 μs) | (a) 無制約席の組合せ的直接配り、(b) 対畳み込みの到達可能シェイプ限定、(c) 席 2 以降を「シェイプ + HCP のみ」の粗い項で提案し細部は重みで補正。フェーズ 5.4 のベンチで選ぶ | 5 |
| R5 | `Partial` の意味論のずれ (L2/L3) | `06-system.md` の操作的定義 (`matched_depth` = 我々側の全コールに条件を満たすエントリがある最長接頭辞) に統一 | 3 |
| R6 | DDS 2.9 の `popen` メモリ探索、Windows x86 `stdcall` 未検証 | 明示 `SetResources` (threads × 95 MB)、x86 は未サポート・未検証と明記 | 5 |
| R7 | コーパス URL の不安定さ (computerbridge.se の `?ph=` トークン、BBO `linfetch`) | 形式ごとに 2 ソース、タグ固定の DDS `hands/` ファイルを常設 | 1 |
| R8 | 外部 BML のライセンス未確認 | fetch のみ、コミットしない (`systems/vendor/data/` は git-ignored) | 3 |
| R9 | `winnow` / `rand` の API 変更 | 版を固定 (`winnow` 1.0、`rand`/`rand_core` 0.10、`rand_xoshiro` 0.8)。seed は `from_seed` のみ使う (`seed_from_u64` は版間の安定性保証がない) | 1, 2 |
| R10 | `HandConstraint::Custom` がキャッシュ直列化を壊す | コンパイラは決して生成しない (debug assert + 全ファイルのテスト `no_custom.rs`) | 3 |

補足リスク (設計エージェントの指摘、上表に含まれないもの):

| # | リスク | 対策 |
| --- | --- | --- |
| R11 | DNF の項が重なると `ConstraintProposal::log_prob` が全成分を数えるためコストが増える | D4 の排他的連鎖で否定由来の項は素。利用者の `Or` だけが重なりうる |
| R12 | `HandConstraint` に `PartialEq` が無いため Step B の重複除去はノード id でしか行えず、構造的に等しい代替が二重に残る | K=8 で有界。`(node, kind)` 列のキーで除去 |
| R13 | `ImplicitPass::Complement` の Pass が緩すぎる、または兄弟が全域を覆って充足不能 | `coverage_report.json` で `ImplicitPass` を本当の穴と分けて数える |
| R14 | `cargo publish` 時に git-ignored の `vendor/` が同梱されない | フェーズ 5.5 で `include` 指定か公開前取得の必須化を決める (未決) |
| R15 | DDS3 への移行 | ラッパーの公開面をレガシー名と同一に保ち、`vendor/` と `build.rs` の差し替えだけで済ませる |

## 11. 未決

- 未決: フェーズ 6 の上位 3 リード命中率の閾値 X。
- 未決: `two-over-one.bml` の着手時期 (フェーズ 4.5、任意)。
- 未決: フェーズ 5.8 (軟情報) をフェーズ 6 の後ろに回すか。
- 未決: `cargo publish` での `vendor/` 同梱 (R14、フェーズ 5.5)。

## 実績

| フェーズ | 完了日 | 完了条件の実測値 |
| --- | --- | --- |
| 0 | 2026-09-18 | 設計文書 14 本、10 クレート + xtask の骨格。check / clippy / doc / fmt / test / wasm32 チェックすべて通過 |
| 1 | 2026-09-18 | PBN コーパス 724/724 ゲーム解析（100%、2019 年世界選手権 4 大会 20 ファイル + PBN 2.1 参考例 2 ファイル）、export 形式でのラウンドトリップ 724 ゲーム不一致 0、BBO vugraph LIN 99/99 ボード、DDS `list100.txt` の deal 文字列 100/100。`bridge-core` 58 テスト、`bridge-format` 32 テスト + コーパステスト 4 本（`--ignored`） |

フェーズ 1 の注記: PBN の警告 196 件は、`[Play]` セクションの `-`（不明カード）による打ち切り 194 件、Latin-1 バイト 1 件、完了後の余分なパス 1 件で、すべて意図した緩和処理である。`1N` は PBN では非標準のため lenient でも `Raw` + 警告として扱う（LIN と `bridge-core` の `FromStr` は受理する）。
