# 11. テスト戦略

本文書はクレートごとのテスト種別と合格基準、仕様 §10 の双方向整合性プロパティテストと再現率テストの具体形、サンプラー・BML・ナチュラル推定・DDS の各検証、ベンチの目標値、CI とナイトリーの定義、開発者がローカルで回すコマンドを確定する。フェーズの完了条件は「コードが書けた」ではなく「数値が出た」で定義し (仕様 §11)、その数値を出すテストはすべてここに列挙する。重いテストは `#[ignore]` にしてナイトリーで `--release -- --ignored` により回し、既定の `cargo test` はネットワークにもコーパスにも触れない。

## 1. テスト層一覧

| テスト | クレート / 場所 | 種類 | 基準 |
| --- | --- | --- | --- |
| Shape テーブル (560 シェイプ / 39 クラス / 28 バランス、`shape_index` の全単射) | `bridge-core` `src/shape.rs`, `tests/shape.rs` | unit / proptest | 全通過 |
| Auction 合法性 (`is_legal`、`contract`、`is_complete`、ランダム列の `from_calls` と `push` の一致) | `bridge-core` `tests/auction.rs` | proptest | 全通過 |
| `Hand`/`Deal`/`Shape` の Display ↔ FromStr、serde 往復 | `bridge-core` `tests/fmt.rs` | proptest | 全通過 |
| Deal 文字列ラウンドトリップ (1 手・4 手・`PartialDeal`) | `bridge-format` `tests/deal_string.rs` | proptest | 全通過 |
| PBN ラウンドトリップ `parse_strict(write(g)) == normalize(g)`、`write(parse(write(g))) == write(g)` | `bridge-format` `tests/pbn_roundtrip.rs` | proptest 10^4 ケース | 全通過 |
| PBN / LIN スナップショット 30 件 | `bridge-format` `tests/pbn_snapshots.rs`, `tests/lin.rs`, `tests/snapshots/` | 回帰 (`insta`) | 一致 |
| fuzz `pbn_parse_lenient`, `lin_parse_lenient` | `bridge-format` `fuzz/` | `cargo fuzz` | パニック 0 |
| コーパス パース率 | `bridge-format` `tests/corpus.rs` | `#[ignore]` (コーパス取得時) | PBN ≥ 99%、LIN ≥ 95%、パニック 0 |
| 評価関数 vs 素朴実装 (全 8192 ホールディング、HCP/LTC/QT/オナー/コントロール) | `bridge-eval` `tests/differential.rs` | 差分 | 完全一致 |
| `hcp` ベンチ | `bridge-eval` `benches/eval.rs` | criterion | < 10 ns |
| DNF 否定の素性 (ランダム手で `A` と `¬A` の項のちょうど 1 つが真) | `bridge-constraint` `tests/dnf.rs` | proptest | 全通過 |
| DNF 交差・爆発対策 (`max_terms = 256` 超過で `residual` 退避) | `bridge-constraint` `tests/dnf.rs` | unit | 全通過、`truncated` フラグ |
| サンプラー厳密性 (§4) | `bridge-constraint` `tests/sampler.rs` | unit / `#[ignore]` χ² | 全通過。`count() == 30 897 212 184` |
| サンプラーベンチ (`Sampler::sample`、`prepare`) | `bridge-constraint` `benches/sampler.rs` | criterion | ≥ 10^5 手/秒/コア、`prepare` 20〜60 μs |
| BML 実ファイル約 40 本のパース | `bridge-system` `tests/parse_real.rs` | 統合 (`systems/vendor/data/` 取得時) | Error lint 0、AST スナップショット一致 |
| BML 展開 == `.bss` 期待出力 (§5) | `bridge-system` `tests/bss_oracle.rs` | 統合 (取得時) | 一致 |
| 説明文コンパイラの認識率 | `bridge-system` `tests/recognition.rs` | 統合 (取得時) | jdh8 ≥ 0.65、gpaulissen ≥ 0.5 |
| `Custom` 非生成 (R10) | `bridge-system` `tests/no_custom.rs` | 統合 | 全ファイルで `Custom` 0、`postcard` 往復一致 |
| BML コンパイル時間 (`sayc.bml`、外部ファイル最大のもの) | `bridge-system` `tests/compile_time.rs` | 統合 | < 1 s |
| ナチュラル推定 3 測定 (§6) | `bridge-system` `tests/natural_metrics.rs` | `#[ignore]`、JSON 出力 | 数値が出る |
| **双方向整合性** `forward_consistency` (§2) | `bridge-bidding` `tests/consistency.rs` | `#[ignore]` release、10^6 配牌、`strict` | 違反 0。`NoCandidate`/`ImplicitPass` はノード別集計 → `target/coverage_report.json` |
| 再現率 `reproduction_rate` (§3) | 同 | `#[ignore]`、コーパス 500 オークション × 1000 サンプル | ノード別に報告 (フェーズ 4 で中央値 ≥ 0.6) |
| `policy_argmax_matches_choose_bid` | `bridge-bidding` `tests/policy.rs` | unit、τ = 0.01、10^5 局面 | 100% |
| `illegal_call_is_lint` | `bridge-bidding` `tests/choose.rs` | unit (合成 `SystemIR` に不正な継続) | `Diagnostic::IllegalSystemCall`、パニックなし |
| `weights_sum_to_one`、`and_combination_drops_contradictions`、`seat_without_calls_is_any` | `bridge-bidding` `tests/interpret.rs` | unit | 全通過 |
| `interpret` ベンチ (12 コール) | `bridge-bidding` `benches/interpret.rs` | criterion | < 10 μs |
| `sequence_log_likelihood` ベンチ | 同 | criterion | 2〜5 μs / 配牌 |
| スレッド数不変 `deterministic_across_threads` | `bridge-sample` `tests/determinism.rs` | unit、同一 seed で `Threads::Single` と 7 スレッドプール | `Vec<WeightedDeal>` と `SampleReport` (`elapsed` 除く) がバイト一致 |
| `ConstraintProposal` の `log_prob` 整合 | `bridge-sample` `tests/proposal.rs` | 小プールで 10^5 提案のヒストグラム vs `exp(log_prob)` | χ² 通過 |
| ESS スイート `uniform_vs_constraint_ess` | `bridge-sample` `tests/ess.rs` | `#[ignore]`、50 オークション × n = 1000 | `ConstraintProposal` の ESS 中央値 ≥ 0.5n。Uniform は比較用に報告 |
| 配牌サンプラーベンチ | `bridge-sample` `benches/deals.rs` | criterion | ≥ 10^4 配牌/秒/コア |
| ショウアウトからのハード制約 `hard_constraints_from_showout` | `bridge-play` `tests/hard.rs` | unit (手組みの履歴、`hard_constraints`) | 長さ確定、`KnownCards` 一致、不整合は `PlayWarning::Inconsistent` |
| リード・シグナル規則表 `lead_rules_table` | `bridge-play` `tests/leads.rs` | table-driven ((約束, リード札) → 期待制約の充足/不充足) | 全通過 |
| DDS レイアウト | `bridge-dds` `tests/layout.rs` | unit (C++ プローブ) | 全構造体・全フィールドで一致 |
| DDS 差分 `differential_dds` | `bridge-dds` `tests/differential.rs` | `list100.txt` (コーパス取得時)、`masterDD.txt` は `#[ignore]` | 100% 一致 |
| 並行 `SolveBoard` `concurrent_solve_board` | `bridge-dds` `tests/concurrency.rs` | 8 スレッド × 100 局面 | エラー 0、逐次結果と一致 |
| wasm ビルド | CI `wasm` ジョブ | `cargo check --target wasm32-unknown-unknown` | 通る |

## 2. 双方向整合性プロパティテスト (仕様 §10)

設計の最大の見返り。`choose_bid` が選んだコールを `interpret` が必ず説明できることを 10^6 局面で確かめ、`NoCandidate` はテスト失敗ではなく集計対象とする。

```rust
// crates/bridge-bidding/tests/consistency.rs
#[test]
#[ignore = "10^6 positions; run with `cargo test --release -- --ignored`"]
fn forward_consistency() {
    let table = common::sayc_table();            // systems[..] = sayc, natural = NaturalInference::default()
    let ctx = BidContext { scoring: Scoring::Imp, natural: None, implicit_pass: ImplicitPass::Never, policy: PolicyParams::default() };
    let opts = InterpretOptions { strict: true, ..InterpretOptions::default() };   // ε = 0, no Fallback branch
    let mut report = CoverageReport::new(&table, SEED, 1_000_000);

    for i in 0..1_000_000u64 {
        let mut rng = rng_for(SEED, i);                                            // bridge_sample::rng_for (D12)
        let (deal, auction) = random_position(&mut rng, &table, &ctx);            // see below
        let seat = auction.next_seat();
        let hand = deal.hand(seat);
        match choose_bid(&table.systems[seat.index() as usize], hand, &auction, &ctx) {
            BidChoice::Chosen(c) => {
                let after = auction.with(c.call).expect("choose_bid must return a legal call");
                let interp = interpret(&table, &after, &opts);
                if !interp.satisfied_by(seat, hand) {
                    report.record_violation(i, &auction, seat, hand, &c);
                }
                report.record_chosen(&c);
            }
            BidChoice::NoCandidate(nc) => report.record_gap(&auction, seat, hand, &nc),
        }
    }
    report.write_json("target/coverage_report.json").unwrap();
    assert_eq!(report.violations.len(), 0, "{}", report.summary());
}
```

手順:

1. `random_position`: `rng` で配牌を 1 つ引き、ディーラーとバルネラビリティをランダムに選び、`replay` と同じ手順で `choose_bid` を `k` 回 (k は `0..=完了` の一様乱数) 進めた接頭辞を局面とする。システム内の分布から局面を取るので、フェーズ 3 のオープニングだけの段階では `k = 0` が大半になる。`random_call_rate` (既定 0.05) の確率で合法コールを一様に差し替え、システム外の接頭辞 (Partial/Natural) も生成する (フェーズ 3.11 以降)。
2. `strict` モードでは ε = 0、防御枝なし。`satisfied_by` は `Fallback` 枝を無視するので、違反は本当に「解釈が生成器を説明できない」場合だけになる。
3. 検出されるもの (仕様 §10): 生成器が制約外のビッドをした = システム定義の矛盾 (違反)、`NoCandidate` = カバレッジの穴 (集計)、制約は満たすが不自然なビッド = 定義が緩すぎる (再現率テストが拾う)。
4. `NoCandidate` と `ImplicitPass` は「局面のトライ位置 (`Lookup.end`)」ごとに数え、頻度上位 50 件を報告する。`ImplicitPass` は本当の穴と分けて数える (兄弟の補集合は緩すぎることがある)。
5. `Diagnostic::IllegalSystemCall` と `UnsatisfiableNode` はシステム定義の lint として JSON に載せ、違反には数えない。

`target/coverage_report.json` の形:

```json
{
  "system": "sayc", "system_hash": "blake3:…", "compiler_version": "0.0.1",
  "seed": 42, "positions": 1000000, "random_call_rate": 0.05,
  "violations": [],
  "counts": { "chosen": 987654, "no_candidate": 10234, "implicit_pass": 2112 },
  "gaps": [
    { "trie": 118, "path": "1N (P)", "seat_rel": "responder", "no_candidate": 1234, "implicit_pass": 56,
      "positions": 40321, "rate": 0.032, "sample_hand": "AKQJ..AKQJ.23456", "sample_hcp": 14 }
  ],
  "diagnostics": { "illegal_system_call": [ { "node": 77, "call": "1C" } ], "unsatisfiable_node": [] }
}
```

`gaps` は `rate` 降順の上位 50 件。この一覧がシステム定義の改善優先度になる (フェーズ 4)。

## 3. 再現率テスト (仕様 §10 逆方向)

```rust
#[test]
#[ignore = "needs BRIDGE_CORPUS_DIR"]
fn reproduction_rate() {
    let Some(dir) = corpus_dir() else { return };
    let table = common::sayc_table();
    let bid_ctx = BidContext { implicit_pass: ImplicitPass::Complement, ..common::bid_ctx() };
    let mut report = ReproductionReport::new();
    for auction in corpus_auctions(&dir).take(500) {
        let interp = interpret(&table, &auction, &InterpretOptions::default());
        let ctx = SampleContext { known: KnownCards::EMPTY, interpretation: &interp,
                                  play_constraints: &[HandConstraint::ANY; 4], play_soft: None, bidding: None };
        let opts = SampleOptions { seed: SEED, ..SampleOptions::default() };
        let (deals, _) = sample_deals(&ctx, &ConstraintProposal::default(), 1000, &opts).unwrap();
        let reproduced = deals.iter()
            .filter(|d| replay(&table, &d.deal, auction.dealer(), auction.vulnerability(), &bid_ctx).auction == auction)
            .count();
        report.record(&auction, &interp, reproduced as f64 / 1000.0);
    }
    report.write_json("target/reproduction_report.json").unwrap();
}
```

再現率が低いノードは「解釈が緩すぎる」ノードで、下流のサンプリング精度が落ちる箇所と一致する。報告はノード別 (最後に Exact 解決したノード) と `ResolutionKind` 別の中央値・分位点。閾値はフェーズごとに文書化し、フェーズ 4 で中央値 ≥ 0.6 を完了条件にする。

## 4. サンプラー厳密性テスト (`bridge-constraint`)

| テスト | 内容 | 基準 |
| --- | --- | --- |
| 全サンプルが項を満たす | 各テストの制約で 10^5 サンプル → `satisfies` | 100% |
| 周辺分布 χ² | 10^6 サンプルの (シェイプ, HCP) 周辺分布 vs `Sampler` 自身の重み表 (期待値は厳密) | p ≥ 0.001 (`#[ignore]`) |
| 小プール全列挙 | 未知 20 枚・6 枚を配る等の小プールで全部分集合を列挙し、`count()` と一致、`Σ_h exp(log_prob(h)) = 1` (誤差 1e-9) | 一致 |
| フルデッキの厳密カウント | `15..=17 ∧ BALANCED` で `count() == 30 897 212 184` | 一致 |
| 確定カード合成 (D3) | `fixed` を含む手を列挙して `count()` と比較、制約は元の 13 枚に対して評価 | 一致 |
| K=2 追加特徴 | `Controls` 等の 1 特徴付きで小プール全列挙 | 一致 |
| 棄却モード | `Custom` を含む制約で α 推定と `is_exact() == false`、`tracing::warn!` の発火 | 通過 |
| 和集合サンプリング | 重なりのある `Or` で `log_prob = ln m(h) − ln C` を全列挙と比較 | 一致 |

## 5. BML `.bss` オラクルテスト (`bridge-system`)

1. `cargo xtask systems fetch` が `systems/vendor/manifest.toml` (gpaulissen/bml のテストデータと期待 `.bss`、選抜した実ファイル、URL + sha256) を `systems/vendor/data/` に取得する。未取得ならテストはスキップ。
2. 各 `.bml` を `compile()` し、展開結果 (`SystemIR.index` の全経路と説明文) を `.bss` 形式に書き戻して期待出力と比較する。差分は行単位で表示する。
3. 意図的差分 (`06-system.md` §1: 末尾記号なし履歴行、Exact 優先、`2S/3H` 等の受理) は期待出力側に注記付きの上書きファイル `*.bss.override` を置いて吸収し、上書きの件数を報告する。
4. `example1..6` と実ファイル約 40 本で Error lint 0 (Warning/Info は数だけ記録)。
5. 認識率 (`Recognition.ratio` の平均) を JSON に出し、jdh8 ≥ 0.65、gpaulissen ≥ 0.5 を閾値にする。

## 6. ナチュラル推定の測定 (D8、`bridge-system` `tests/natural_metrics.rs`、`#[ignore]`)

正解データが無い問題に対して、システム定義とコーパスを擬似正解として使う。3 つとも JSON (`target/natural_metrics.json`) に出す。

| # | 測定 | 手順 | 出力 |
| --- | --- | --- | --- |
| 1 | 隠しノード比較 | 実システム (`sayc.bml`、jdh8 Polish Club) の非人工ノードを 1 つずつ隠し、`classify` + `infer` の制約と元ノードの制約を比べる。各ノードで 1,000 手を元制約からサンプルし recall、推定制約からサンプルし precision、体積比 (`count()` の比) を出す | `Role × CallKind` 別の recall / precision / 体積比の平均と分位点 |
| 2 | 再現率 | `C_nat` からサンプルした手を `choose_bid` (システム外なので `natural.candidates`) で再生し、元のコールと一致する率 | 規則別の一致率 |
| 3 | コーパス充足 | コーパス実手について `infer` の制約が満たされる率と制約体積のパレート (体積が小さく充足率が高いほど良い) | 規則別の (充足率, log 体積) 点列 |

閾値は設けない (数値が出ることがフェーズ 3 の完了条件)。フェーズ 4 で値を見て `NaturalParams` を調整する。

## 7. DDS 差分テスト (`bridge-dds`)

1. `corpus/data/dds/list100.txt` (`BRIDGE_CORPUS_DIR` 必須) の各配牌を Deal 文字列パーサで読む。
2. `calc_dd_table` の結果を `TABLE` 行 (20 値) と比較、`dealer_par` を `PAR` 行と、`analyse_play` を `PLAY`/`TRACE` 行と比較する。基準は 100% 一致。
3. `masterDD.txt` (83,691 配牌) は `#[ignore]` でナイトリーのみ。
4. 仕様 §10 の「差分テストの相手として DDS を使う」は、将来の自前 DD 計算 (対象外) や `PlayHistory::trick_winner` の検証 (`analyse_play` の `tricks` 列との整合) にも使う。

## 8. コーパステスト

| 対象 | テスト | 基準 |
| --- | --- | --- |
| PBN (コーパス 1, 2, 4) | `bridge-format` パース率 | ≥ 99% のゲームで `view()` が Deal を返し Auction/Play がタグと矛盾しない |
| LIN (コーパス 3) | `bridge-format` パース率 | ≥ 95% のボードが Deal と Auction を返す |
| PBN オークション | `bridge-bidding` 再現率 (§3)、`xtask coverage` (フェーズ 4: 全コール Exact ≥ 80%、残りが `EmptySupport` なし) | フェーズ 4 の完了条件 |
| DDS ハンド | `bridge-dds` 差分 (§7) | 100% |
| BML (systems/vendor) | `.bss` オラクル、認識率 (§5) | 上記 |

取得は `cargo xtask corpus fetch` と `cargo xtask systems fetch` (`03-format.md` §7)。テストは `BRIDGE_CORPUS_DIR` 未設定なら即 `return` する。

## 9. ベンチ (criterion 0.8) と目標値

| ベンチ | クレート | 目標 | 出典 |
| --- | --- | --- | --- |
| `hcp(hand)` | `bridge-eval` | < 10 ns | 仕様 §9 |
| `Sampler::sample` (`Atom` からのハンド生成) | `bridge-constraint` | ≥ 10^5 手/秒/コア (試算 0.3〜0.5 μs) | 仕様 §9、D2 |
| `Sampler::prepare` (フルデッキ) | `bridge-constraint` | 20〜60 μs (列挙が要るスート 1 つにつき +65 μs、未知 26 枚で 3〜10 μs) | 計画 §4.3 |
| 完全な配牌サンプリング (制約付き) | `bridge-sample` | ≥ 10^4 配牌/秒/コア | 仕様 §9 |
| `interpret` (12 コール) | `bridge-bidding` | < 10 μs | 仕様 §9 |
| `sequence_log_likelihood` | `bridge-bidding` | 2〜5 μs / 配牌 | 計画 §6.4 |
| `AuctionTrie::resolve` | `bridge-system` | 深さ × 約 30 ns | 計画 §5.5 |
| BML 1 システムのコンパイル | `bridge-system` (統合テスト) | < 1 s | 仕様 §9 |
| `calc_dd_table`, `solve_board(AllRanked)` | `bridge-dds` | 目標なし (記録) | |

`cargo bench --workspace --no-run` を毎 PR で通し (ベンチのコンパイル切れを防ぐ)、実行はナイトリーとフェーズ完了時。criterion の `html_reports` を成果物として保存する。

## 10. CI 定義 (`.github/workflows/ci.yml`)

方針: `lint` (fmt、clippy `--all-features -D warnings`) → `test` (3 OS: ubuntu / macos / windows、既定 feature と `--all-features`、`bench --no-run`) → `wasm` check → `msrv` 1.85 → `deny` (ライセンス)。ナイトリーで `--ignored` リリーステスト、コーパス、DDS ベンダリング、ベンチ。フェーズ 0 でコミット済みの `ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
  schedule:
    - cron: "0 3 * * *" # nightly: ignored (long) tests, corpus, benches

env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: -D warnings

jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets --all-features -- -D warnings

  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace
      - run: cargo test --workspace --all-features
      - run: cargo bench --workspace --no-run

  wasm:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-unknown-unknown
      - uses: Swatinem/rust-cache@v2
      # Every crate except the FFI wrapper and the dev tool must build for the browser.
      - run: cargo check --workspace --exclude bridge-dds --exclude xtask --target wasm32-unknown-unknown
      - run: cargo check -p bridge --no-default-features --features std,format,serde --target wasm32-unknown-unknown

  msrv:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.85
      - uses: Swatinem/rust-cache@v2
      - run: cargo check --workspace --exclude xtask

  deny:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: EmbarkStudios/cargo-deny-action@v2
        with:
          command: check licenses

  nightly:
    if: github.event_name == 'schedule'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - uses: actions/cache@v4
        with:
          path: corpus/data
          key: corpus-${{ hashFiles('corpus/manifest.toml') }}
      - run: cargo xtask corpus fetch
      - run: cargo xtask dds vendor
      - run: cargo test --release --workspace --all-features -- --ignored
        env:
          BRIDGE_CORPUS_DIR: ${{ github.workspace }}/corpus/data
      - run: cargo bench --workspace
      - uses: actions/upload-artifact@v4
        with:
          name: criterion-reports
          path: target/criterion
```

補足:

- `wasm` ジョブは `bridge-dds` と `xtask` を除外する。ファサードは既定 feature に `dds`/`parallel`/`cache` を含まないので、そのままの check と `std,format,serde` の check の両方を通す。
- 骨格の CI には毎 PR の `dds` ジョブが無い。DDS の `layout`/`concurrency` テストはナイトリーの `cargo xtask dds vendor` の後に `--all-features -- --ignored` の一部として回る。フェーズ 5.5 で 3 OS の `dds` ジョブ (`actions/cache` で `crates/bridge-dds/vendor` を `VENDOR.md` のハッシュでキャッシュし、Linux では `--features openmp` も) を追加し、`differential` はコーパスが要るのでナイトリーに残す。
- ナイトリーの `systems fetch`、`systems/vendor/data` のキャッシュ、レポート成果物 (`coverage_report.json` 等) のアップロード、`workflow_dispatch` はフェーズ 3〜4 で `ci.yml` に足す。
- `deny` は現状 `check licenses` のみ。`bans sources` の追加と `deny.toml` の許可ライセンス (`MIT`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Unicode-3.0`, `Zlib`) はフェーズ 5.5。ベンダリングする DDS (Apache-2.0) は Cargo 依存ではないが `deny.toml` のコメントと `VENDOR.md` に記載する。未決: 実際の依存ツリーで必要になる追加ライセンス。
- MSRV ジョブは `check` のみ (dev-dependencies の MSRV は問わない)。
- `RUSTFLAGS: -D warnings` はフェーズ 0 の骨格でも通る: 各クレートの `lib.rs` が `#![allow(dead_code, unused_variables)]` で `todo!()` 由来の未使用警告を抑えている (実装とともに外す。`12-roadmap.md` §1)。

## 11. ローカルで回すコマンド

| 目的 | コマンド |
| --- | --- |
| 各 PR の最低限 (計画 §13) | `cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace` |
| wasm 確認 | `rustup target add wasm32-unknown-unknown` (初回) → `cargo check --workspace --exclude bridge-dds --exclude xtask --target wasm32-unknown-unknown` |
| コーパス取得 | `cargo xtask corpus fetch` (出力先 `corpus/data/`、`BRIDGE_CORPUS_DIR` で変更可) |
| 外部 BML 取得 | `cargo xtask systems fetch` |
| DDS 取得とテスト | `cargo xtask dds vendor && cargo test -p bridge-dds` |
| フェーズ完了時の重いテスト | `BRIDGE_CORPUS_DIR=corpus/data cargo test --release --workspace -- --ignored` (整合性 10^6、ESS、コーパス、DDS 差分、ナチュラル推定測定) |
| ベンチ | `cargo bench --workspace` (`hcp` < 10 ns、手サンプル ≥ 10^5/s、配牌 ≥ 10^4/s、`interpret` < 10 μs、BML コンパイル < 1 s) |
| 1 クレートのベンチ | `cargo bench -p bridge-constraint -- sampler` |
| カバレッジレポート (フェーズ 4) | `cargo xtask coverage --system systems/sayc.bml --corpus corpus/data/pbn` |
| fuzz | `cargo +nightly fuzz run pbn_parse_lenient -- -max_total_time=600` (`crates/bridge-format/fuzz/`) |
| スナップショット更新 | `cargo insta review` (`cargo install cargo-insta`) |
| バインディング参照の再生成 | `cargo xtask dds regen-bindings` (libclang が必要) |

レポート成果物 (計画 §13): `target/coverage_report.json` (NoCandidate 集計)、認識率レポート (`target/recognition_report.json`)、ナチュラル推定精度 (`target/natural_metrics.json`)、ESS レポート (`target/ess_report.json`)、再現率 (`target/reproduction_report.json`)。

## 12. 未決

- 未決: 再現率の閾値 (フェーズ 4 で中央値 ≥ 0.6、それ以前は報告のみ)。
- 未決: `random_position` の `random_call_rate` 既定値 0.05 (フェーズ 3.11 で Partial/Natural を入れた後に調整)。
- 未決: `deny.toml` の最終的な許可ライセンス一覧 (§10)。
- 未決: フェーズ 6 の上位 3 リード命中率の閾値 X (測定してから決める)。
