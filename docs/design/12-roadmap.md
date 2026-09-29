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
| `cargo check` / `clippy` / wasm check が通る状態 | `systems/sayc/sayc.bml` の中身 (フェーズ 3.5) |

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
| 3.5 | `systems/sayc/sayc.bml` (オープニング) 執筆 + `#+META` 拡張 (D16) | コンパイル < 1 s、Error lint 0、`Custom` 0 |
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
| 4.7 | 方策鏡像: `ExclusiveIndex`、rank 方策、片の較正、`AuctionPolicy` (D18、D19) | `policy_mirror` の under-cover 0・exact ≥ 99%、`tightness` 0/0、argmax 100%、`interpret` 12 コール < 10 μs |
| 4.8 | replay の暴走修正 (ナチュラル・レベル下限、`NaturalParams.level_floor`) | 2000 生成配牌で 7 レベル ≤ 1%・6 レベル以上 ≤ 5%、`sayc_content` 通過 |
| 4.9 | 評価基盤: 固定フィクスチャ、コーパス分割 (列挙順の偶数 = 調整用、奇数 = 評価用)、`human()` プリセットの最尤推定 (D20) | `coverage_report.json` に (ε̂, δ̂) と対数尤度曲線が出る |

完了条件 (2026-09 改訂、`15-phase4-plan.md` の「完了基準の改訂案」と D20)。[G] は SAYC 自身が生成したオークション (整合したモデル)、[C] はコーパスで測る。どちらも毎回両方を報告する。

- (a) カバレッジ
  - [G] 固定 seed のランダム配牌 1000 本を replay (ナチュラル補完あり) したとき、≥ 80% が全コールシステム (ナチュラル補完も gap も無い)。システムの作者が制御できるカバレッジで、ナチュラル補完が穴の印になるので同語反復ではない。ただし「どんな手でもパス」だけが行の位置 (パスの連鎖や、将来の構造的な停止既定) は穴を隠すので、strict 集計で数える: そうした位置でナチュラルの選択 m_P(h) がパス以外なら、そこで離脱したものとする (`all_system_strict`)。基準は strict の値に適用し、素の値と連鎖なしの値を並べて報告する。
  - [C] コーパスの SAYC 互換オープニング部分集合 (真のオープナーの手が記録されたオープニングの排他領域 X に入る) で、コール単位のシステム解決率 (Exact または Partial) ≥ 80%。
  - オークション単位の全コール Exact 率をコーパス全体・評価用分割・部分集合で報告し、4.1 のベースラインを下回らないこと。
  - `EmptySupport`: 既定モードでコーパスのサンプラー `EmptySupport` 0。strict な支持が空の席数は報告する。
- (b) 再現率
  - [G] 生成オークション 100 本で中央値 ≥ 0.6 (`ConstraintProposal` と方策尤度で重み付け)。
  - [C] コーパスの SAYC 再現可能部分集合 (真の配牌を replay すると記録どおりのオークションになる) で中央値 ≥ 0.6。部分集合の件数も報告する。
  - 旧定義 (コーパス全体、一様提案) の中央値と、コール単位の実配牌での方策一致率も継続性のために報告する。
- (c) 新しいゲート: `policy_mirror` の under-cover 0 かつ exact ≥ 99%、`tightness` 0/0、整合性の gap 起因でない違反 0、`interpret` 12 コール < 10 μs (フェーズ 3 から持ち越し、フェーズ 3 では未達)。

改訂の理由: コーパス (2019 年世界選手権と BBO vugraph) の大半はストロング・クラブや 2/1 とその独自の約束事で競られているのに、SAYC として解釈・replay している。これはモデルの誤指定で、コーパス 100 本中 97 本がシステム上の位置でナチュラルなコールを含み、フェーズ 3 の再現率はどの試作でも解釈にかかわらず 0.0 だった (試作 B は 92/100 で ESS ≥ 30 でも中央値 0)。コーパスだけの閾値は SAYC のカバレッジやサンプラーの質ではなくコーパスのシステム構成を測ってしまう。棄却を許すと ESS の基準も曖昧になり (B の ESS/n 0.906 に対し試行あたり 0.371)、生成 ESS ケースは方策が変わるたびに変わり、いくつかのパラメータが評価スイートそのもので調整されていた。同じ閾値 (80%、0.6、0.5) を整合した集合に適用し、コーパス側の対応物を定義し、両方を必ず報告するという趣旨は保つ。

このフェーズでコードを書いている時間が長いなら L1/L2 の設計が間違っている (増えるべきはデータ)。

## 6. フェーズ 5: サンプラー本番化 (`bridge-play`, `bridge-sample`, `bridge-dds`)

| id | PR の内容 | 証明 / 完了基準 |
| --- | --- | --- |
| 5.1 | play: `hard_constraints` (ショウアウト → 長さ確定、既出カード → `KnownCards`、整合性検査) + `KnownCards::with_play` | unit、`PlayWarning::Inconsistent` |
| 5.2 | sample: `ConstraintProposal` (`prepare`/`propose`/`log_prob`、席の並べ替え、成分重み) | 小プールで 10^5 提案のヒストグラム vs `exp(log_prob)` (χ²) |
| 5.3 | sample: `sample_deals` + ビディング尤度 + `SampleReport` + `tracing` INFO の ESS | 固定 50 ケースの ESS スイートで ESS/n 中央値 ≥ 0.5 (全体・生成側)、試行あたり ESS を報告 |
| 5.4 | sample: ベンチ + プロファイル、性能対策 (a) 無制約席の直接配り (b) 対畳み込みの到達可能シェイプ限定 (c) 粗い項での提案 の選択 (R4) | ≥ 10^4 配牌/秒/コア |
| 5.5 | dds: `cargo xtask dds vendor` (2.9.0、`vendor/SHA256SUMS`)、`tests/layout.rs`、CI の `dds` ジョブ (`build.rs`、`sys.rs`、`layout_probe.cpp` は骨格で確定済み) | 3 OS でビルド、レイアウトテスト一致 |
| 5.6 | dds: 安全ラッパー本体 (`init`/`OnceLock`、`calc_dd_table(s)`、`solve_board` + スレッドスロット、`solve_all_boards` + `Mutex`、`analyse_play`、`dealer_par`、`info`) + 差分テスト | `list100.txt` で 100% 一致、8 スレッド並行テスト |
| 5.7 | ファサード `bridge::dd` の動作確認 (`dds` feature、`DoubleDummy` トレイト、`dds()` のフォールバック。骨格で実装済み) | wasm check が緑のまま、`dds()` が `None`/`Some` を返す |
| 5.8 | play: `lead_constraints`/`signal_constraints` の表と `interpret_play` の結合 (軟情報) + サンプラーの `play_soft` (フェーズ 6 には不要、ずれても可) | table-driven テスト |

完了条件 (2026-09 改訂): 固定した 50 ケースの ESS スイート (生成 25 ケースは `PolicyParams::system_players()`、評価用分割のコーパス 25 ケースは `PolicyParams::human()`、プリセットは事前登録) で ESS/n の中央値が全体 ≥ 0.5 かつ生成側 ≥ 0.5、コーパス側は報告 (目標 ≥ 0.4)。試行あたりの ESS、受理率、所要時間を常に報告する。残差棄却を使う場合は試行予算 ≤ 20n、予算を使い切るケースは 50 中 2 以下、所要時間は棄却なしの 2 倍以下。配牌 ≥ 10^4/秒/コア (Stayman 3NT を含む)。パラメータは調整用分割と調整用 seed だけで決める。DDS FFI が動き、サンプル配牌の解析結果が返る。

## 7. フェーズ 6: オープニングリードアドバイザ (別クレート)

| id | 内容 | 証明 / 完了基準 |
| --- | --- | --- |
| 6.1 | ライブラリ側: `Solutions::AllRanked` によるリード評価 API (`DoubleDummy::lead_scores`)、PBN オークションからの `Interpretation`、ESS 報告 | unit |
| 6.2 | アプリ (別クレート): オークション + 自分の手 → サンプル配牌 → 各リードの DD 期待トリック → 上位 3 リード | コーパス 100 ボード (`human()` プリセット) で上位 3 リードに DD 最善が含まれる率 ≥ 0.90 かつ上位 1 がベースライン (a) (0.808) 以上 (2026-09 改訂で閾値 X を確定)。hard と `legacy_temperature` = 1 の比較と ESS 中央値 (目標 ≥ 20) を報告 |

理由 (仕様 §11): 判断が 1 手に閉じるため探索木を掘る必要がなく、ビディング情報の利用が本質そのもので、上級者でも間違えるため価値が実感できる。リード時点ではプレイ履歴が無いので `bridge-play` の軟情報は不要。

## 8. フェーズごとの数値完了基準 (まとめ)

| フェーズ | 数値基準 | 測るテスト (`11-testing.md`) |
| --- | --- | --- |
| 0 | `cargo check --workspace --all-targets`、`clippy -D warnings`、wasm check の 3 つが通る | CI `lint`, `test`, `wasm` |
| 1 | PBN パース率 ≥ 99%、ラウンドトリップ同値 (proptest 10^4)、LIN ≥ 95%、fuzz パニック 0 | §1 format 行、§8 |
| 2 | ハンド生成 ≥ 10^4/秒 (目標 10^5)、`count() == 30 897 212 184`、χ² 通過、eval 8192 一致、`hcp` < 10 ns、スレッド数不変 | §1 eval/constraint/sample 行、§4、§9 |
| 3 | `.bss` 一致、Error lint 0、認識率 jdh8 ≥ 0.65 / gpaulissen ≥ 0.5、コンパイル < 1 s、整合性 10^6 で違反 0、`interpret` < 10 μs、argmax 100%、`coverage_report.json` と `natural_metrics.json` が出る | §2、§5、§6、§9 |
| 4 | [G] 生成 1000 本の ≥ 80% が全コールシステム (strict 集計)、[C] SAYC 互換オープニング部分集合のシステム解決率 ≥ 80%、全コール Exact 率 (全体・評価用・部分集合) がベースライン以上、サンプラー `EmptySupport` 0、再現率中央値 ≥ 0.6 ([G] 生成 100 本、[C] SAYC 再現可能部分集合)、`policy_mirror` under-cover 0・exact ≥ 99%、`tightness` 0/0、整合性の gap 起因でない違反 0、`interpret` 12 コール < 10 μs | §2、§3、§8、`cargo xtask coverage` |
| 5 | 固定 50 ケースで ESS/n 中央値が全体 ≥ 0.5・生成側 ≥ 0.5 (コーパス側は目標 ≥ 0.4)、試行あたり ESS・受理率・所要時間を報告、配牌 ≥ 10^4/秒/コア (Stayman 3NT を含む)、DDS 差分 100%、レイアウト一致、並行テスト通過 | §1 sample/dds 行、§7、§9 |
| 6 | 100 ボードで上位 3 リードの DD 最善命中率 ≥ 0.90 かつ上位 1 ≥ ベースライン (a) (`human()` プリセット) | フェーズ 6.2 |

フェーズ 4 以降は、次の値を整合した集合 ([G]、SAYC 生成) とコーパス ([C]) の両方で毎回報告する (D20): 全コール Exact 率とシステム解決率 (生成、コーパス全体・評価用・部分集合)、再現率 (生成、コーパス部分集合、旧定義のコーパス)、ESS/n と試行あたり ESS (生成とコーパス)、コーパスでのコール単位の実配牌一致率、(ε, δ) の最尤推定値。

## 9. 各 PR と各フェーズの検証方法 (計画 §13)

1. 各 PR: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`、`cargo check --workspace --exclude bridge-dds --exclude xtask --target wasm32-unknown-unknown`。
2. フェーズ完了時: `cargo test --release --workspace -- --ignored` (整合性 10^6、ESS、コーパス、DDS 差分) と `cargo bench` (`hcp` < 10 ns、手サンプル ≥ 10^5/s、配牌 ≥ 10^4/s、`interpret` < 10 μs、BML コンパイル < 1 s)。
3. レポート成果物: `target/coverage_report.json` (NoCandidate 集計)、認識率レポート、ナチュラル推定精度 JSON、ESS レポート。

## 10. リスク (計画 §14)

| # | リスク | 対策 | 影響フェーズ |
| --- | --- | --- | --- |
| R1 | 公開 SAYC / 2-1 の BML が無い | フェーズ 3 で `systems/sayc/sayc.bml` を自作。Polish Club (jdh8) を早期テストに使う | 3, 4 |
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

- 未決: `two-over-one.bml` の着手時期 (フェーズ 4.5、任意)。
- 未決: フェーズ 5.8 (軟情報) をフェーズ 6 の後ろに回すか。
- 未決: `cargo publish` での `vendor/` 同梱 (R14、フェーズ 5.5)。

## 実績

| フェーズ | 完了日 | 完了条件の実測値 |
| --- | --- | --- |
| 0 | 2026-09-18 | 設計文書 14 本、10 クレート + xtask の骨格。check / clippy / doc / fmt / test / wasm32 チェックすべて通過 |
| 1 | 2026-09-18 | PBN コーパス 724/724 ゲーム解析（100%、2019 年世界選手権 4 大会 20 ファイル + PBN 2.1 参考例 2 ファイル）、export 形式でのラウンドトリップ 724 ゲーム不一致 0、BBO vugraph LIN 99/99 ボード、DDS `list100.txt` の deal 文字列 100/100。`bridge-core` 58 テスト、`bridge-format` 32 テスト + コーパステスト 4 本（`--ignored`） |
| 2 | 2026-09-25 | `hcp` 0.8 ns（目標 < 10 ns）。手の抽出 約 0.2 µs/手（約 450 万手/秒、目標 10^5 手/秒）。`Sampler::prepare` フルデッキ 7〜14 µs、プレイ途中（未知 26 枚・6 枚固定）で形の制限なし 12 µs・バランス型限定 3 µs（負荷の低い環境での計測。見積り 3〜10 µs に対し制限なしのみ未達）。15〜17 HCP バランスの厳密数 30,897,212,184 と C(52,13) が一致、周辺分布の χ²（10^6 抽出を含む）通過、小プール全列挙で Σexp(log_prob)=1。eval は 8192 ホールディング全件一致。一様配牌 約 200 万配牌/秒。スレッド数不変（Single と 7 スレッドでバイト一致）。ワークスペースのテスト 191 件通過。Opus 統合レビューで確定した 15 件（否定の重なり、棄却項の log_prob、充足可能性の誤判定、準備の性能など）を修正済み |
| 3 | 2026-09-26 | `systems/sayc/*.bml` (オープニング・応答・リビッド・競り合い・NT・2C・ウィーク・ツー・プリエンプト) がエラー lint 0 でコンパイルされ、`sayc.bml` のコンパイル 342 ms・外部最大の jdh8 `wj/1C.bml` 40 ms（release、目標 < 1 s）。replay 方式の生成器（他シートのコールを 5% の確率でランダムな合法コールに差し替える、フェーズ 3.11）による strict 双方向整合性 10^6 局面（seed `0x5a1c0002`、差し替えを含む接頭辞 173,235）で、gap 起因でない違反 0（完了条件）。システムにコールの無い手に強制した Pass が根本原因の gap 起因の違反 2,645 件はカバレッジの穴として別集計（`11-testing.md` §2 手順 6）。chosen 851,576 / `NoCandidate` 4,115 / `ImplicitPass` 144,309、`coverage_report.json` の頻度上位はディーラーの 0–11 HCP のパスなど正しいパスで、`NoCandidate` 上位は 1D-(3C) のレスポンダー 36、P-P-1D-(1H) のレスポンダー 31、1C-(1H) のレスポンダー 25（統合時に最多だった 1NT へのバランシング後のアドバンサー 505 件は `competition.bml` に表を足して解消）。`call_distribution` の argmax == `choose_bid` 10^5 局面 100%。`.bss` オラクル 228/228 一致（9 ファイル）、実 BML 54 本の Error lint 集合が期待 24 件と完全一致、認識率 jdh8 0.858・gjp 0.524（目標 0.65・0.5）。再現率（コーパス 500 オークション。各シートの strict な解釈を満たす一様配牌を棄却法で最大 1000 残し、`replay` が再現した素の割合）: 配牌が 30 以上残った 234 オークションの中央値 0.0（0 より大きいのは 83 件）、最終コールが Exact の 14 件は中央値 0.419、Natural の 220 件は 0.0（閾値はフェーズ 4）。以前の「ESS ≥ 30 の中央値」は再現 0 のオークションを構成上選ぶ統計量だったので廃止（`11-testing.md` §3）。ナチュラル推定: 隠しノード sayc 1,500・vendor 795、再現率 600 決定点・8,326 候補で一致率 0.331。`interpret` ベンチ: 12 コールは手組み 11.3 µs・実 SAYC 17.6 µs で目標 < 10 µs **未達**（負荷平均 9〜16 の共有機、10 コールの `sayc-1nt` 9.5 µs、8 コールの競り合い 7.4 µs）。ワークスペースのテスト 647 件通過（15 件 ignored）。再レビュー 3 で、説明文コンパイラの「明示が勝つ」規則（否定・可能性・別の Or 枝の断片では文脈語を捨てない）と、SAYC の全手を拾う行・兄弟に隠れた行（相手のマイケルズ、テイクアウトダブルとそのアドバンス、1NT-(2X) の 3 レベル、1M-4M、バランシングのジャンプ、マイナーへの 1♠ 応答、2C-2D 後のリビッド）を修正 |

フェーズ 1 の注記: PBN の警告 196 件は、`[Play]` セクションの `-`（不明カード）による打ち切り 194 件、Latin-1 バイト 1 件、完了後の余分なパス 1 件で、すべて意図した緩和処理である。`1N` は PBN では非標準のため lenient でも `Raw` + 警告として扱う（LIN と `bridge-core` の `FromStr` は受理する）。

### フェーズ 4 のカバレッジ: ベースラインと SAYC 追加後 (レーン D、`cargo xtask coverage`)

`cargo xtask coverage` (`xtask/src/coverage.rs`、release) が `target/coverage_report.json` に書く値。詳細と SAYC の追加内容 (P1〜P9) は `systems/sayc/NOTES.md` の「Phase 4 tables」「Phase 4 coverage」にある。生成は seed `0xC0FE4001` の 1000 本、局面は整合性生成器 (seed `0x5a1c0002`、5% ランダムコール) の 10^6 局面、コーパスは 27 ファイル 724 オークション (列挙順の偶数 = 調整用、奇数 = 評価用)。

| 項目 | ベースライン (wip/p4-api 4b131db、SAYC 変更前) | 基本 SAYC を S のコードで (wip/p4-S マージ後、SAYC 変更前) | SAYC 追加後 (wip/p4-D、P1〜P10) | 参考: 連鎖を除いた SAYC 追加後 | 基準 |
| --- | --- | --- | --- | --- | --- |
| [G] 生成 1000 本の全コールシステム率 (strict / 素) | 0.027 (素) | 0.027 (素) | **strict 550/1000 (0.550)**、素 894/1000 (0.894) (別 seed 2000 本の素: 0.895 / 0.875 / 0.8755) | 0.044 / 0.044 | strict ≥ 0.80 **未達** |
| 既定パスしか行の無い位置 / うちナチュラルがパス以外 (生成 1000 本) | — | — | 2,706 / 461 (366 本、素では全コールシステムの 344 本を含む) | — | 報告 |
| フェーズ 3 の `NoCandidate` 上位 (10^6 局面あたり): 1D-(3C) / 1D-(1H) / 1C-(1H) のレスポンダー | 27 / 22 / 15 (フェーズ 3 報告時 36 / 31 / 25) | 19 / 146 / 101 | 0 / 0 / 0 | 0 / 0 / 0 (2×10^5 局面) | 各 80% 以上減 達成 |
| `NoCandidate` (10^6 局面) | 4,115 | 8,826 (システム上 203) | 3,009 (システム上 142) | 18,410 相当 (2×10^5 局面で 3,682) | — |
| lint | Error 0、`ShadowedBranch` 249 (我々側 18、相手側 231) | 同左 | Error 0、`ShadowedBranch` 400 (我々側 18、相手側 382。相手側の新規 151 はすべて表見出しノード)、要件の無い我々のコール 0 (P9 前 6)。統合時に lint が相手側のノードを数えなくなり、`ShadowedBranch` 18 (すべて我々側) | Error 0、我々側 18 | Error 0 達成、我々側の新規 0 |
| 整合性 (release) | フェーズ 3: 10^6 で gap 起因でない違反 0、gap 起因 2,645 | — | 10^5: 0 / gap 起因 26、10^6: 0 / gap 起因 426 | — | gap 起因でない違反 0 達成 |
| コーパス全コール Exact 率 (全体 / 評価用 / 部分集合) | 0.041 / 0.039 / 0.034 | 0.041 / 0.039 / 0.034 | 0.305 / 0.309 / 0.326 | 0.047 / 0.047 / 0.044 | ベースライン以上 達成 (大半は連鎖による) |
| [C] システム解決率 (Exact または Partial、全体 / 評価用 / 部分集合) | 0.405 / 0.409 / 0.424 | 0.405 / 0.409 / 0.424 | 0.641 / 0.661 / **0.665** (部分集合の評価用 0.692) | 0.443 / 0.450 / 0.463 | 部分集合 ≥ 0.80 **未達** |
| `resolve_lenient` の使用 | 5 / 8,169 コール | 5 / 8,169 コール | 1 / 8,169 コール | 9 / 8,169 コール | ベースライン未満 達成 |
| strict な支持が空の席 / サンプラー `EmptySupport` | 30 / 1 | — / 1 | 18 / 0 | 25 / 0 | `EmptySupport` 0 達成 |
| 実配牌での方策一致率 (システム位置 / ナチュラル位置) | 0.600 / 0.443 | 0.600 / 0.627 | 0.675 / 0.588 (評価用 0.691 / 0.597) | 0.584 / 0.657 | 報告 |
| (ε, δ) の最尤推定 (調整用 4,135 コール) | ε 0.490、δ 0.309、ln L −9351.0 | ε 0.375、δ 0.460 | **ε 0.3357、δ 0.335**、ln L −7320.7 (−1.770/コール)、最大値から 1.92 以内は δ 0.30〜0.38 | ε 0.361、δ 0.412、ln L −7665.6 | 報告 (`human()` の値としてレーン B に渡す。連鎖の設計に依存する) |
| 生成の最終コントラクト・レベル [パスアウト, 1..7] | [14, 21, 94, 143, 79, 29, 9, 611] | [14, 49, 315, 420, 142, 48, 10, 2] | [14, 92, 304, 450, 136, 3, 1, 0] | [14, 58, 279, 430, 148, 57, 13, 1] | — |
| `cargo xtask coverage` の所要時間 | 52.5 s (10^6 局面、負荷平均 9〜10) | — | 既定 6.1 s、10^6 局面 17.6 s (負荷平均 2.6〜2.9) | 既定 8.0 s | ≤ 120 s 達成 |

ベースラインから「基本 SAYC を S のコードで」への変化はレーン S のコード (タスク 4.8 のナチュラル・レベル下限と SAYC 形のナチュラル規則) によるもので、SAYC の行の効果ではない。7 レベルの最終コントラクト 611 → 2 はレベル下限の効果である。SAYC 追加後の列と連鎖なしの列の差が、パスの連鎖 (P1、`passes.bml`) の寄与である。連鎖は「一人がパスしたら、相手が何をしてもパートナーもどんな手でもパス」を `(any)` で 6 巡書き出したもので、全行の約 87% を占め、素の [G] 0.894・コーパスの全コール Exact 率の大半・(ε, δ) の推定値を左右する。連鎖の各段の位置は行がパスだけなので、素の集計ではナチュラル補完 (穴の印) が構成上出ない。strict 集計はこれを補正する。

δ を最尤推定値に固定したときの ln L は ε = 0.001 / 0.01 / 0.1 / 0.2 / 0.316 / 0.398 / 0.501 で −13750.6 / −10817.0 / −8102.6 / −7514.2 / −7324.0 / −7351.6 / −7528.2、ε を固定したときは δ = 0 / 0.1 / 0.2 / 0.3 / 0.4 / 0.5 で −7666.3 / −7402.7 / −7341.8 / −7321.9 / −7324.6 / −7345.2。仮置きの `human()` (0.01, 0.3) は −10822.0、`system_players()` は −15274.7、評価用分割での最尤推定値は 4,034 コールで −6707.5。

[G] (strict) が未達の理由: 既定パスしか行の無い位置でナチュラルがコールを選ぶ 461 件は長い裾で、最多の位置 (`1S-(P)-P-(X)` のオープナー) でも 9 件。P10 で、レビューで挙がった強い手のパス (ネガティブ・ダブル後に相手がレイズしたときのオープナー、3 レベルのオーバーコールへのリオープン、1NT-3NT への 4 レベルのバランシングへのペナルティ・ダブルなど) に行を書き、strict は 0.540 から 0.550 になった。残りの多くは相手の構築的なオークション中の我々の 4 番手・バランシング席で、ナチュラルが SAYC ならパスする手で競る例も含む。連鎖を `(P)` だけで続ける案も測った (素 0.333: 相手が競り続けるオークションは、ナチュラルもパスする位置でもすべて離脱になる) が採らなかった。

P9 (負の double 後のアドバンス) の前は、表見出し `(1X)-1Y-(D)-2X-(P)-` が行の無いアドバンサーのキュービッドを作り、どんな手でもキュービッドしていた (要件の無い我々のコール 6 ノード)。これがコーパスで唯一の既定モード `EmptySupport` の原因で、修正後は 0。

[C] が未達の理由: 部分集合 384 本のうち 259 本がシステムを外れる。最初のナチュラルなコールは、137 本が「システム上の位置で記録されたコールが行に無い」(1♣-2♦ などプレイヤー自身の方法や 2/1 の続き)、80 本が「既定パスしか行の無い位置で記録されたコールが行に無い」(P10 前は 91 本。SAYC に判断があるのにファイルが書いていない位置で、プレイヤーの方法ではない)、42 本がその他の SAYC 側の穴 (相手のパスがトライに無い 19、システムの尽き 18、相手のコール 5)。以前はこの 80 (91) 本もプレイヤーの方法に数えていたので、「SAYC の行を足しても 0.80 に届く見込みは無い」という判断は取り下げる。ただし既定パスのみの位置は 1 位置あたり数件の長い裾で、行を足して届く量かは測り直しが要る。これは完了基準の改訂理由 (コーパスの誤指定) と並べて報告する。

コスト: SAYC は 1,611 行 / 2,415 ノードから 38,737 行 / 49,800 ノードに増えた (パスの連鎖が約 87%)。release のコンパイルは 957〜983 ms (負荷平均約 5)、`tests/compile_time.rs` の release 限定の `< 1 s` は単独でも 1.24 s、他のテストと同時で 1.42 s で**失敗する** (負荷平均 3.8〜4.6)。排他索引の再構築 45.7〜49.2 ms (レーン S の基準 ≤ 15 ms を超える。連鎖なしでも 17.9 ms)、postcard IR 16,438,261 バイト (追加前 845,860)、既定サイズの coverage 実行のピーク RSS 241 MB。連鎖なしなら 5,010 行、396 ms、1,913,553 バイト。解決は構造的な停止既定: bridge-system が我々のパスの辺 (と我々の最後のコール後に尽きたノード) に停止の印を付け、bridge-bidding の gather / interpret / `AuctionPolicy` が解決がそこで止まったときに「どんな手でもパス」をシステムの選択として合成する (レーン S と B、推定 300〜500 行)。我々のパスを単に透過させるのは誤り (接頭辞の表を別の席に当ててしまう)。同じ strict 集計が必要。
