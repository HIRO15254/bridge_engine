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
| サンプラー一般経路の draw χ² (単一スート `cards` 制約、`Controls` 加法特徴、`fixed` 併用。§4) | `bridge-constraint` `tests/sampler_chi_square.rs` | unit χ² (10^5 サンプル、全列挙との比較) | p ≥ 0.001、抽出手が全て厳密な上位集合に属する |
| サンプラーベンチ (`Sampler::sample`、`prepare`) | `bridge-constraint` `benches/sampler.rs` | criterion | ≥ 10^5 手/秒/コア、`prepare` 20〜60 μs |
| BML 実ファイル約 40 本のパース | `bridge-system` `tests/parse_real.rs` | 統合 (`systems/vendor/data/` 取得時) | Error lint 0、AST スナップショット一致 |
| BML 実ファイル全 54 本の説明文コンパイル (roadmap 3.2-3.4) | `bridge-system` `tests/compile_real.rs` | 統合 (取得時) | `Error` lint の集合が `tests/data/real_expected_errors.txt` (`<path>\t<line>\t<code>\t<reason>`) と完全一致。新規の `Error` も、期待ファイルの陳腐化したエントリも失敗。現在 24 件 (統合レビュー confirmed#12-24 の説明文コンパイラ修正で 64 件から 40 件減。新規 0) で、各エントリはソースファイル自体の矛盾 (class b) か、既知のコンパイラ/語彙ギャップとして記録された未対応分 (class c)。修正可能だったコンパイラのバグ (class a) は全て直し、回帰テストを追加済み (`#INCLUDE` 先の誤帰属を含む、`tests/compile_real.rs`/`tests/common/mod.rs` 自体のバグも修正: `lint.span.file` をルートファイルのパスと取り違えていた)。トリアージ全件は `tests/data/real_lint_triage.md` (原因ごとにグループ化、file:line・行・lint・分類) |
| BML 展開 == `.bss` 期待出力 (§5) | `bridge-system` `tests/bss_oracle.rs` | 統合 (取得時) | 一致 |
| 説明文コンパイラの認識率 | `bridge-system` `tests/recognition.rs` | 統合 (取得時) | jdh8 ≥ 0.65、gpaulissen ≥ 0.5 |
| `Custom` 非生成 (R10) | `bridge-system` `tests/no_custom.rs` | 統合 | 全ファイルで `Custom` 0、`postcard` 往復一致 |
| BML コンパイル時間 (`sayc.bml`、外部ファイル最大のもの) | `bridge-system` `tests/compile_time.rs` | 統合 | < 1 s |
| ナチュラル推定 3 測定 (§6) | `bridge-system` `tests/natural_metrics.rs` | `#[ignore]`、JSON 出力 | 数値が出る |
| **双方向整合性** `forward_consistency` (§2) | `bridge-bidding` `tests/consistency.rs` | `#[ignore]` release、10^6 配牌、`strict` | 違反 0。`NoCandidate`/`ImplicitPass` はノード別集計 → `target/coverage_report.json` |
| 再現率 `sayc_reproduction_rate` (§3) | `bridge-bidding` `tests/reproduction.rs` | `#[ignore]` release。(i) 生成フィクスチャ 100 本、(ii) コーパス評価用分割の SAYC 再現可能部分集合、(iii) 旧定義 500 本、(iv) コール単位の一致率 | (i) と (ii) の中央値 ≥ 0.6 (フェーズ 4)。(iii)(iv) は報告 |
| `policy_argmax_matches_choose_bid` | `bridge-bidding` `tests/policy.rs` | unit、両プリセット (`system_players()` と `human()`)、10^5 局面 (フェーズ 3 のソフトマックスの τ = 0.01 は、D18 で方策を改めた時に不要になった) | 100% |
| `illegal_call_is_lint` | `bridge-bidding` `tests/choose.rs` | unit (合成 `SystemIR` に不正な継続) | `Diagnostic::IllegalSystemCall`、パニックなし |
| `weights_sum_to_one`、`and_combination_drops_contradictions`、`seat_without_calls_is_any` | `bridge-bidding` `tests/interpret.rs` | unit | 全通過 |
| `interpret` ベンチ (12 コール) | `bridge-bidding` `benches/interpret.rs` | criterion | < 10 μs |
| `sequence_log_likelihood` ベンチ | 同 | criterion | 2〜5 μs / 配牌 |
| スレッド数不変 `deterministic_across_threads` | `bridge-sample` `tests/determinism.rs` | unit、同一 seed で `Threads::Single` と 7 スレッドプール | `Vec<WeightedDeal>` と `SampleReport` (`elapsed` 除く) がバイト一致 |
| `ConstraintProposal` の `log_prob` 整合 | `bridge-sample` `tests/log_prob.rs` | 小プールで 10^5 提案のヒストグラム vs `exp(log_prob)`（格子要約・軽い代替の畳み込み・残差棄却を含む。棄却がある文脈では失敗率も照合） | χ² 通過 |
| 残差棄却の厳密性 | `bridge-sample` `tests/residual.rs` | 全列挙での `log_prob` の差、提案の χ²、`sample_deals` の重み付き推定の不偏性（Wald χ²）。`T = U` の文脈に加え、下限で `T` が切り詰められ、中間席が粗化と畳み込みを受ける文脈でも確認する。パイロットが `pilot_attempts` として試行あたり ESS に算入されること | 差 < 1e-9、p > 0.01 |
| ESS スイート `uniform_vs_constraint_ess_suite` | `bridge-sample` `tests/ess_suite.rs` | `#[ignore]`、release、固定ケース 50 オークション × n = 1000、実ビディング尤度、`target/ess_report.json`（§13） | 既定の提案（残差棄却なし）で ESS/n の中央値 ≥ 0.5（全体・生成）、コーパス ≥ 0.4、予算切れ ≤ 2 件、試行あたり ESS ≥ 0.35。2026-09-29（固定ケース再生成後）: 0.5702 / 0.5695 / 0.5709、0 件、0.5702（達成。残差棄却あり下限 0.5 は 0.8420 / 0.8934 / 0.8201、試行あたり 0.5170。09-sample.md §10.2 の続き） |
| ESS 固定ケースの鮮度 `ess_fixture_generated_cases_are_on_policy` | `bridge-sample` `tests/ess_suite.rs` | 生成 25 件の各コールを配牌の実際の手が現行の方策で `p ≥ 0.01` で選ぶこと（§13） | 外れたコール 0 |
| 配牌サンプラーベンチ | `bridge-sample` `benches/deals.rs` | criterion | ≥ 10^4 配牌/秒/コア |
| ショウアウトからのハード制約 `hard_constraints_from_showout` | `bridge-play` `tests/hard.rs` | unit (手組みの履歴、`hard_constraints`) | 長さ確定、`KnownCards` 一致、不整合は `PlayWarning::Inconsistent` |
| リード・シグナル規則表 `lead_rules_table` | `bridge-play` `tests/leads.rs` | table-driven ((約束, リード札) → 期待制約の充足/不充足) | 全通過 |
| DDS レイアウト | `bridge-dds` `tests/layout.rs` | unit (C++ プローブ) | 全構造体・全フィールドで一致 |
| DDS 差分 `differential_dds` | `bridge-dds` `tests/differential.rs` | `list100.txt` (コーパス、または `cargo xtask dds vendor` が展開する `vendor/dds-2.9.0/hands/list100.txt`。CI の `dds` ジョブで必須)、`masterDD.txt` は `#[ignore]` | 100% 一致 |
| 並行 `SolveBoard` `concurrent_solve_board` | `bridge-dds` `tests/concurrency.rs` | 8 スレッド × 100 局面 | エラー 0、逐次結果と一致 |
| リード助言の集計と契約・エラー系 (`advise` の期待値の手計算との一致、同値グループ化、決定性、`IncompleteAuction` / `PassedOut` / `WrongHandSize` / `NoSamples`、得点表) | `bridge-lead` `tests/advise.rs`, `tests/contract.rs`, `src/aggregate.rs`, `src/scoring.rs` | unit (DDS 不要、`tests/common/mod.rs` の `FakeDd`、`14-lead.md` §4) | 全通過 |
| リード助言の DDS smoke | `bridge-lead` `tests/dds_smoke.rs` | `--features dds`、固定の配牌とオークション、少数サンプルで実 DDS (ベンダリング前は何もせず通る) | 全通過 |
| リード助言のコーパス評価 `corpus_eval` (フェーズ 6.2) | `bridge-lead` `tests/corpus_eval.rs` | `#[ignore]`、release、`--features dds,parallel`、`target/lead_report.json` (`14-lead.md` §4) | 評価分割 100 ボードで上位 3 の DD 最善命中率 ≥ 0.90 かつ上位 1 ≥ ベースライン (a)。ESS 中央値・ESS/n を報告 |
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
6. 実装の生成器 (`common::random_sayc_position_with_substitution`) は `replay` と同じく `NoCandidate` を `Pass` に置き換えて接頭辞を進め、その位置を記録する。手順 1 の `random_call_rate` (既定 0.05) による合法コールの一様な差し替えは、検査するシートより前の**他のシート**のコールにだけ行い、その位置も記録する (検査するシート自身のコールを差し替えると `satisfied_by` の違反が構成上必ず出るため)。`satisfied_by` の失敗の根本原因が強制 `Pass` である違反は「gap 起因」(`violations_gap_induced`) として別に数える: システムがその手にコールを持たないカバレッジの穴 (手順 3 の `NoCandidate`) の帰結であり、定義の矛盾ではない。合否は gap 起因でない違反 (`violations_not_gap_induced`) = 0 で判定する。`coverage_report.json` には実際の `random_call_rate` と、差し替えを含む接頭辞の数 (`random_call_prefixes`) を書く。2026-09-27 (フェーズ 3 再レビュー 3 の修正後) の実測: seed `0x5a1c0002`、`random_call_rate` 0.05、10^6 局面 (うち差し替えを含む接頭辞 173,235) でgap 起因でない違反 0、gap 起因 2,645、chosen 851,576 / `NoCandidate` 4,115 / `ImplicitPass` 144,309 (release で約 45 秒)。差し替えなし (0.0) だった 2026-09-26 の値は gap 起因 1,279、`NoCandidate` 2,410。

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

§2 は「`choose_bid` が選んだコールを `interpret` が説明できるか」(定義が狭すぎないか) を見る。再現率テストはその逆で、「あるオークションの解釈が受け入れる配牌を `replay` すると、同じオークションに戻るか」を見る。再現率が低いのは、解釈が緩すぎるか、そのオークションをシステムが競らないかのどちらかである。実装は `crates/bridge-bidding/tests/reproduction.rs` で、結果は `target/reproduction_report.json` に書く。

### 3.1 報告する 4 つの数値 (フェーズ 4 で再定義、15-phase4-plan の基準 (b))

コーパス (2019 年の世界選手権決勝) は主にストロング・クラブや 2/1 で競られている。それを SAYC として読むのはモデルの誤指定なので、コーパスだけの閾値はシステムの被覆率やサンプラーの質ではなく、コーパスのシステム構成を測ってしまう。そこで、SAYC が正しいモデルになる集合 (生成) を見出しにし、コーパスには対応する部分集合を定める。両方を毎回報告する。

| 部 | 集合 | 方策の preset | 解釈 | 完了条件 |
| --- | --- | --- | --- | --- |
| (i) 生成 | 固定フィクスチャ `tests/data/repro_generated.txt` の SAYC オークション 100 本 | `system_players()` | `InterpretOptions::for_context` (鏡像) | 中央値 ≥ 0.6 |
| (ii) コーパス部分集合 | 評価用分割 (奇数番目) のうち、真の配牌を `replay` すると記録どおりのオークションになるもの。件数も報告 | `human()` | 同上 | 中央値 ≥ 0.6 (件数を併記) |
| (iii) 旧定義 | コーパスの先頭 500 本 (両分割)。フェーズ 3 の定義をそのまま継続 | `system_players()` | `InterpretOptions::legacy()` | 報告のみ |
| (iv) コール単位の一致率 | 評価用分割の全コール。真の配牌の持ち主の手で `choose_bid` が記録どおりのコールを選ぶ率。システム位置とナチュラル位置に分ける | `human()` | ― | 報告のみ |

- (iv) の位置の分類は `choose_bid` 自身の判定に合わせる。ナチュラル推定と暗黙パスを切った `BidContext` で `choose_bid` を呼び、システムのコールを選ぶか、合法なシステム候補を `Rejected::Unsatisfied` で退けた位置をシステム位置とする (合法なシステム候補が 1 つ以上ある位置)。それ以外はナチュラル位置。`NoCandidate` は不一致として数え、`gaps` にも記録する。これを「`choose_bid` 探査による定義」と呼ぶ。レーン D の `cargo xtask coverage` も同じ名前の一致率を出すが、そちらは trie の解決 (`resolve_lenient` による近似一致を含む) と合法な子の有無で位置を分け、コーパスも PBN の後に LIN を足した集合を使う (§3.3)。定義が違うので 2 つの数は一致しない。報告では「再現率 harness の (iv)」「coverage の一致率」と出所を付けて並べる。
- (i) には、フィクスチャのうち真の配牌から今のシステムで再現しなくなった本数 (`fixture_drift`) も出す。フィクスチャは凍結するので、ずれは失敗ではなく情報である。
- (ii) は同じ部分集合を `system_players()` でも測る (`corpus_subset.system_players`)。`human()` (δ > 0) の strict な解釈には、システム内の位置でもナチュラル逸脱の片 Y_c が入る。Y_c の尤度は δ の分しか無いので、残った配牌を方策の尤度で重み付けしないと Y_c を過大に数える (§3.2)。見出しの率は重み付きで、重み 1 の率も `unweighted_rate` / `median_unweighted_rate` として並べて出す。

### 3.2 サンプラー

配牌の引き方は `Sampler` 列挙で切り替える。どの部も共通の関数 `evaluate(sampler, table, ctx, auction, opts, seed)` を通り、各配牌の重み w について「再現した配牌の重み / 重みの合計」を率とする。見出しの中央値に入れるのは、ESS = (Σw)² / Σw² が 30 以上のオークションだけである。このフィルタは配牌の数 (重み) だけを見て、再現したかどうかは見ない。

- `Sampler::StrictRejection { target, max_draws, weight }` (フェーズ 4 の見出し、`weight = RejectionWeight::Policy`)：一様な配牌を引き、4 シートすべての手が strict な解釈 (`InterpretOptions { strict: true, .. }` での `Interpretation::satisfied_by`、Fallback の片を除く) を満たすものだけを残す。残った配牌には、評価している `BidContext` での方策の尤度 `AuctionPolicy::log_likelihood` から w = exp(ln L − max) を付ける。strict な台に制限した一様提案は台の上で密度が一定なので、これは事後分布 p(deal | auction) ∝ L(deal) を strict な台に制限したもの (落ちるのは Fallback の片の質量だけ) に対する正確な重要度重みであり、フェーズ 5 の重み付きサンプラーと同じ量を測る。`system_players()` では L が strict な台の上でほぼ一定なので、重みはほぼ等しく、ESS は残った配牌の数にほぼ等しい。`human()` では Y_c の配牌の重みが小さくなる。`RejectionWeight::Unit` (重みはすべて 1、ESS = 残った配牌数) はフェーズ 3 の定義で、(iii) にだけ使う。
- 既定は目標 1000 配牌、上限 5,000,000 回。オークション全体の strict な解釈を満たす一様配牌は 1e-4 程度しかなく、フェーズ 3 の上限 200,000 回では生成 100 本のうち 44 本が 30 配牌に届かなかった。1 回の抽選と判定は 1 スレッドで約 125 ns (release、生成 1 本で 3 回測った最良値、loadavg 5.5) で、上限に達するオークション 1 本に 1 スレッドあたり約 0.6 秒かかる。共用機で 8 スレッドのときは 1 回あたり 200〜400 ns になり、ほとんどのオークションが上限に達する (i) 全体では 12〜25 秒になる。
- `Sampler::Weighted { proposal, n }`：`sample_deals` で提案分布から n 配牌を引き、方策の尤度 (`BiddingLikelihood`) / 提案密度で重み付けする。`ProposalKind::Uniform` はフェーズ 3 の「尤度重み付きの一様サンプル」で、(iii) の `weighted_uniform` にだけ使う。`choose_bid` が選ばないコールには方策が ε/n の床しか与えないので、再現する配牌が 1 つあればその重みが桁違いに大きくなり ESS ≈ 1 になる。このため 0/1 に近い統計量で、見出しには使わない (フェーズ 3 再レビュー 3。以前の見出し「この重み付き率の、ESS ≥ 30 のオークションでの中央値」は、構成上、再現率 0 のオークションだけを選んでいた)。
- (iii) の旧定義は、見出しのサンプラーが何であってもフェーズ 3 のサンプラー (`LEGACY_SAMPLER` = 棄却、重み 1、目標 1000、上限 200,000) に固定する。

**フェーズ 5 での差し替え**：`ConstraintProposal` と `AuctionPolicy` による重み付けが入ったら、`headline_sampler()` の既定を `Sampler::Weighted { proposal: ProposalKind::Constraint, n: 1000 }` に変える。変更はこの 1 か所だけで済む。解釈はすでに尤度と同じ `BidContext` から `InterpretOptions::for_context` で作っており、`evaluate` は重みを扱い、報告には ESS と試行数が既に入っている。`BiddingLikelihood` の中身を `AuctionPolicy::log_likelihood` に替えるのは `bridge-sample` 側 (レーン P) で、この harness は変えなくてよい。先に試すときは `SAYC_REPRO_SAMPLER=constraint` で切り替えられる (フェーズ 4 の系統では `ConstraintProposal::prepare` が `todo!()` なので panic する)。鏡像の提案が q ∝ L を満たせば重みはほぼ一定になり、ESS ≥ 30 のフィルタは棄却法の「30 配牌以上」と同じ意味になる。

### 3.3 生成フィクスチャとコーパス分割

- **生成フィクスチャ** (`tests/data/repro_generated.txt`)：1 行 1 本のタブ区切りで、`id  dealer  vul  deal (PBN、N から)  calls (空白区切り)` の形。`#` 行は注釈。乱数の種は `0x5A1C4001` で、配牌 i は `auction_seed(GEN_SEED, i)`。ボード番号 i + 1 からディーラーとバルネラビリティを決め、`system_players()`、SAYC のナチュラル推定、`ImplicitPass::Complement` で `replay` する。パスアウトと、最終コントラクトが 5 レベルを超えるものは捨てる (レベル下限が入るまでは、ナチュラル推定の 7 レベルへの暴走が混ざるため。09-sample §10.2 の ESS スイートと同じ規則)。フェーズ 4 の最終統合の SAYC では、最初の 100 配牌 (gen-0 から gen-99) がどれも捨てられずに 100 本になった (それ以前の SAYC では 247 配牌のうちパスアウト 6、5 レベル超 141 を捨てていた)。
- 再生成は `SAYC_REPRO_WRITE_FIXTURE=1 cargo test --release -p bridge-bidding --test reproduction -- --ignored write_generated_fixture`。環境変数なしで走らせると、ファイルと今のシステムの生成結果が何本食い違うかを表示するだけで、ファイルは書き換えない。このフィクスチャはフェーズ 4 の最終統合で、レーン D2 の行が入った後に凍結し直した (ad42581。100 本中 8 本のオークションが変わった。その前は 0928a7b と d127e42 で凍結し直していた)。これで固定し、以後は方策を変えてもケース集合は変えない (D20)。フェーズ 4 の最終レビューの修正 (レーン X、起点 441a4e4) でも、環境変数なしの実行で 100 本とも今のシステムの生成と一致した。
- 既定スイートでは `generated_fixture_is_well_formed` (100 本、id が一意、完了済み、パスアウトなし、5 レベル以下、書き出しと読み込みで一致) と、生成器の決定性、分割規則、棄却と重み付きの両方のサンプラー、(iv) の位置分類を確かめる (debug で約 3〜5 秒)。棄却法については、`system_players()` のパスアウトで率 ≥ 0.99 かつ重みが一定 (ESS = 残った配牌数) であること、δ > 0 のプリセットのパスアウトで重みが `AuctionPolicy` の尤度そのものであり、重み付きの率が重み 1 の率を 0.05 以上上回ることを確かめる (`policy_weighted_rejection_follows_the_likelihood`)。プリセットはフェーズ 4 の統合で当てはめた値 (ε 0.3404、δ 0.3959) を試験の定数で固定し、`human()` を当てはめ直しても幅が動かないようにした。200 配牌で 0.931 対 0.835 である。フェーズ 4 の仮置き (0.01, 0.3) では 0.948 で、統合前の基準は 0.1 だった。差の大きさは席ごとのパスの確率の比で決まる。システムがパスする席はほぼ全てナチュラルでもパスなので、その手は (1−ε) + ε/n、逸脱としてだけパスする手は (1−ε)δ + ε/n (n = 36) で、比は仮置きで約 3.3、当てはめた値で約 2.5 である。見込まれる差は約 0.09 で、0.05 はその約半分を余裕として残す (12-roadmap「フェーズ 4 の統合 (D3・len・perf のマージ後)」)。`generated_true_deals_lie_in_the_strict_mirror` は、フィクスチャの真の配牌が 4 シートとも strict な鏡像の台に入ること (解釈が生成元の配牌を取りこぼさないこと) を確かめる。真の配牌から記録どおりに再現しなくなったケース (再凍結前に SAYC を変えたときのずれ) はこの検査から外す。フィクスチャの配牌が今のシステムで再現するかどうか自体は既定スイートでは確かめない (再凍結前に SAYC を変えると落ちてしまうため)。
- **コーパスの列挙と分割** (`corpus_auctions`、15-phase4-plan D20 が使う名前)：`corpus/data/pbn` 以下の `.pbn` をパスでソートし、ファイル内の順に、`GameView` がオークションを解決したゲームだけを数える (2 つの `Optimum*Table.pbn` と `-` のゲームは入らない。view の解釈に失敗すると `#` の継承をリセットする)。この列挙番号が偶数なら調整用、奇数なら評価用 (D20)。いまのコーパスでは 625 本、評価用 312 本で、評価用はすべて真の配牌を持つ。
- レーン P の ESS スイート (`crates/bridge-sample/tests/ess_suite.rs`) はこの列挙をそのまま使う。レーン D の `cargo xtask coverage` は同じ PBN の列挙の後ろに `corpus/data/lin` の LIN ボードを足すので、PBN の部分の番号と分割はここと一致するが、評価用分割はこちらの 312 本を含む上位集合になる。PBN だけの 312 本を正準の評価用分割とし、coverage の数には「PBN + LIN」と付けて並べる。

### 3.4 実行と環境変数

`cargo test --release -p bridge-bidding --all-features --test reproduction -- --ignored sayc_reproduction_rate --nocapture`。(ii)〜(iv) はコーパス (`BRIDGE_CORPUS_DIR` または `corpus/data`) が無ければ飛ばす。重さの調整は次の環境変数で行う。

| 変数 | 既定 | 意味 |
| --- | --- | --- |
| `SAYC_REPRO_PARTS` | 全部 | `generated,corpus,legacy,agreement` のうち走らせる部 |
| `SAYC_REPRO_SAMPLER` | `rejection` | 見出しのサンプラー (`rejection` / `constraint` / `uniform`) |
| `SAYC_REPRO_TARGET` / `SAYC_REPRO_MAX_DRAWS` | 1000 / 5,000,000 | 棄却法の目標配牌数と抽選上限 |
| `SAYC_REPRO_GENERATED` | 100 | (i) で使うフィクスチャの本数 |
| `SAYC_REPRO_CORPUS_LIMIT` | 全部 | (ii) で標本を取る部分集合の本数 |
| `SAYC_REPRO_LIMIT` | 500 | (iii) のオークション数 |

報告には開始時と終了時の loadavg を入れる。

### 3.5 実測

**方策の尤度による重み付けの導入後** (見出しの値)：2026-09-27、wip/p4-api 4b131db + レーン R + wip/p4-B 0d133bf、棄却法の残った配牌を `AuctionPolicy` の尤度で重み付けした版。全 4 部で 42.4 秒、8 スレッド、loadavg 5.65 → 9.54。数値は統合時 (レーン S・D の後、フィクスチャの再凍結の後) に測り直す。

- (i) 生成 100 本：ESS 30 以上が 83 本、率の中央値 **1.0** (重み 1 でも 1.0)。残った配牌数・ESS の分布は下の「統合後」と同じ (0 本が 1、1〜29 本が 16、1000 本に達したのが 13、中央値 117.5、p10 15)。`system_players()` では重みがほぼ一定なので、見出しは変わらない。重み 1 で 0.930 だった `1NT Pass 2H X Pass Pass Pass` は重み付きで 1.000 になった (ESS 317、残った 341)。`fixture_drift` 0。12.5 秒 (4.64 億回の抽選、87 本が上限 5e6 回に達した)。
- (ii) コーパス部分集合 (4 本)：`human()` で重み付きの率は 0.832 / 0.512 / 0.504 / 0.799 (パスアウト、`P 1NT P P P`、`P P 1NT P P P`、`P P P 1NT P 2C P 2S P 3NT P P P`)、中央値 **0.655** (重み 1 では 0.556 / 0.231 / 0.217 / 0.589、中央値 0.3935)。ESS は 623〜794 (残った配牌はどれも 1000)。基準の 0.6 は超えるが、4 本の中央値なので意味は薄い。`system_players()` では 4 本とも 1.0。
- (iii)(iv) は重み付けの影響を受けず、下の「統合後」と同じ値 (28.7 秒)。

**レーン B (方策鏡像) の統合後**：2026-09-27、wip/p4-api 4b131db + レーン R + wip/p4-B 0d133bf。重みはまだすべて 1。strict な解釈は鏡像の非 Fallback 片 (X_c と、δ > 0 なら Y_c) になった。全 4 部で 52.6 秒、8 スレッド、loadavg 6.48 → 12.70 (他の 4 レーンと共用の 10 コア機)。

- (i) 生成 100 本：30 配牌以上残ったのは 83 本 (0 本が 1、1〜29 本が 16、1000 本に達したのが 13、残った配牌数の中央値 117.5、p10 15)。率の中央値 **1.0** (平均 0.999、p10 1.0、83 本すべてが 0.6 以上)。`fixture_drift` 0。17.3 秒。strict な解釈が「方策がそのコールを選ぶ手の集合」になったので、解釈を満たす配牌はほぼ必ず再現する。代わりに領域が狭くなり、残る配牌は B 統合前の中央値 984.5 から 117.5 に減った (上限 5e6 回で 30 に届かないのが 17 本)。
- (ii) コーパス部分集合：評価用 312 本のうち再現するのは **4 本 (1.3%)** (パスアウト 1 本、`P 1NT P P P`、`P P 1NT P P P`、`P P P 1NT P 2C P 2S P 3NT P P P`)。`human()` での率は 0.556 / 0.231 / 0.217 / 0.589 で中央値 **0.3935**。同じ 4 本を `system_players()` で測ると 4 本とも 1.0 (中央値 1.0)。差は、`human()` の strict な解釈に入る δ の片 (Y_c) を、重みの無い棄却法が過大に引くことによる (上の重み付き版で解消した)。4 本では中央値に意味が無く、この数はコーパスが SAYC で競られていないことを示している。
- (iii) 旧定義 (500 本)：30 配牌以上が 233 本 (0 本が 198、1〜29 本が 69、1000 本が 33)、率の中央値 0.0 (0 より大きいのが 83 本、p90 0.378)。種別では `Natural` 219 本の中央値 0.0、`Exact` 14 本の中央値 0.4145。尤度重み付きの一様サンプルが 1 つでも再現したのは 500 本中 24 本、その ESS の中央値 3.00。34.7 秒。旧定義は `InterpretOptions::legacy()` で読むので、レーン B の前後で変わらない。フェーズ 3 の値 (234 本、中央値 0.0、`Exact` 0.419、25 本) ともほぼ同じで、差は方策が τ = 1 のソフトマックスから決定的な方策に変わったことによる。
- (iv) 一致率 (評価用 312 本、3464 コール)：システム位置 930/1505 = **0.618** (うち `NoCandidate` 6)、ナチュラル位置 880/1959 = **0.449** (同 0。ナチュラル暗黙パスで gap が無くなった)、全体 1810/3464 = 0.523。

**レーン B の統合前** (wip/p4-api 4b131db + レーン R。`InterpretMode::Mirror` がまだフェーズ 3 の Step A を走らせていた)：76.8 秒、loadavg 10.08 → 18.17。(i) 30 配牌以上 95 本、残った配牌数の中央値 984.5、率の中央値 0.129 (平均 0.236、0.6 以上 13 本)。SAYC 自身が競ったオークションでも率が低かったのは、ノードの制約そのものが、`choose_bid` なら別のコールを選ぶ手を受け入れていたためである。(ii) 4 本、率 1.0 / 0.404 / 0.387 / 0.159、中央値 0.3955。(iii) は統合後と同じ。(iv) システム位置 0.618、ナチュラル位置 882/1959 = 0.450 (`NoCandidate` 9)、全体 0.523。

## 4. サンプラー厳密性テスト (`bridge-constraint`)

| テスト | 内容 | 基準 |
| --- | --- | --- |
| 全サンプルが項を満たす | 各テストの制約で 10^5 サンプル → `satisfies` | 100% |
| 周辺分布 χ² | 10^6 サンプルの (シェイプ, HCP) 周辺分布 vs `Sampler` 自身の重み表 (期待値は厳密) | p ≥ 0.001 (`#[ignore]`) |
| 一般経路 draw の χ² (小プール) | 単一スート `cards` 制約単体、および `Controls` 加法特徴 + `fixed` を併せた 2 通りで `count()`/`log_prob` だけでなく実際の `sample()` を 10^5 回引き、全列挙した充足手への一様性を検証 (`sample` が確認する「厳密な項の抽出手は必ず atom を満たす」というデバッグアサートも併走)。`count()`/`log_prob` の全列挙検証 (本節の他の行、および `tests/sampler_small_pool.rs`) は `prepare` 時の重み表だけを見るので、`draw` 固有のバグ (ペア分割の重み付け誤りなど) を素通りさせ得る — この行がその隙間を埋める | p ≥ 0.001 |
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

## 6. ナチュラル推定の測定 (D8、`bridge-bidding` `tests/natural_metrics.rs`、`#[ignore]`)

正解データが無い問題に対して、システム定義とコーパスを擬似正解として使う。3 つとも JSON (`target/natural_metrics.json`) に出す。測定 2 が `choose_bid` (`bridge-bidding` のみが持つ) を要求するため、3 つとも `bridge-system` ではなく `bridge-bidding` のテストとして置く (`bridge-bidding` は `bridge-system` に依存できるが逆はできない)。

| # | 測定 | 手順 | 出力 |
| --- | --- | --- | --- |
| 1 | 隠しノード比較 | 実システム (`sayc.bml`、コンパイルできた jdh8/gjp ファイル) の非人工ノードを 1 つずつ隠し、`classify` + `infer` の制約と元ノードの制約を比べる。各ノードで 1,000 手を元制約からサンプルし recall、推定制約からサンプルし precision、体積比 (`count()` の比) を出す | `Role × CallKind` 別の recall / precision / 体積比の平均と分位点 (`sayc.bml` は自作なので `vendor` (jdh8+gjp) とは別枠) |
| 2 | 再現率 | 各決定点 (測定 1 と同じノード集合、先手番のオープニング (空プレフィックス) は定義として除く。空のシステムではルートに子が無いので、`choose_bid` はオープニングもナチュラルで答える。`natural_tuning` のコーパスの率はオープニングを含む) で `NaturalInference::candidates` が返す各コールの制約から手をサンプルし、`choose_bid` (ノードを持たない空の `SystemIR` を `Table::uniform` で包んで渡すので、全プレフィックスがシステム外になり `ctx.natural` = `natural.candidates` が答える。自然ブランチの `CallContext` は、`interpret` の自然ステップと同じ `partner_context(prefix)` (`exclusion.rs` の `Reader`、`07-bidding.md` §4.1「パートナー文脈」) からパートナー制約とフォーシング状況を埋める) で再生して元のコールと一致する率 | 規則別の一致率 |
| 3 | コーパス充足 | コーパス実手 (PBN + LIN) の各コールについて `infer` の制約が満たされる率と制約体積のパレート (体積が小さく充足率が高いほど良い) | 規則別の (充足率, log 体積) 点列 |

閾値は設けない (数値が出ることがフェーズ 3 の完了条件)。フェーズ 4 で値を見て `NaturalParams` を調整する。2026-09-26 のフェーズ 3 統合 (統合レビュー修正の 3 レーンと SAYC の 2 レーン `sayc-nt`/`sayc-comp` を統合した後) で実行した結果 (`target/natural_metrics.json`) は次の通り: 実際にコンパイルできた実システムは `sayc.bml` と gjp `common/` の 18 ファイル (jdh8 の全ファイルと残りの gjp ファイルは現時点でエラー付きコンパイルのため対象外 -- `compile_if_clean` が「エラー lint 0 のファイルだけを測定対象にする」という、このレーン独自の基準を採っている); 隠しノード比較は sayc で 2,325 の対象ノードのうち上限の 1,500 ノード (SAYC の中身が増えたため、統合レビュー修正時点の 421 ノードから増加)、vendor (gjp、うち 15 ファイルが対象ノードを持つ) で 795 ノードを評価した。recall/precision はいずれも 0〜1 の範囲に収まるが、`Role × CallKind` ごとの log2 体積比の平均はノード数の少ない群ほどばらつきが大きく、vendor/Opener/Bid_Reverse (7 ノード) の −4.96 から vendor/Responder/Bid_Cue_Jump (3 ノード) の +5.56 まで散らばる (体積比が大きいほど推定制約が実際より緩い)。再現率は 600 決定点・8,295 候補で全体一致率 0.329 (SAYC 統合前は 8,059 候補・0.336、統合レビュー修正前は 7,593 候補・0.331、2026-09-25 の値は 486 決定点・6,411 候補・0.340)、コーパス充足は 27 ファイル・724 局・8,169 コールを走査した。この測定で `rule_rebid_own` がウィーク・ツー/プリエンプト/ストロング 2C 後の自己スート・リビッドを常に `opening_hcp` (12–21) と比べていた実装漏れが見つかり、`crates/bridge-system/src/natural.rs` で修正済み (詳細は `06-system.md` §8.3 の該当注記を参照)。

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
| 完全な配牌サンプリング (制約付き) | `bridge-sample` | ≥ 10^4 配牌/秒/コア (実 SAYC の 3 ケース `deals/sayc/*` を含む。既定の残差棄却なしと、残差棄却ありの両方で測る) | 仕様 §9 |
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
| ESS スイート | `cargo test --release -p bridge-sample --all-features --test ess_suite -- --ignored --nocapture` (`ESS_SUITE_MODE=tune` でチューニング集合、`ESS_SUITE_WRITE_FIXTURE=1` で固定ケースの再生成、§13) |
| リード助言 (フェーズ 6) | 単体: `cargo test -p bridge-lead --all-features` (`--features dds` で DDS smoke も)。コーパス評価: `cargo test -p bridge-lead --release --features dds,parallel --test corpus_eval -- --ignored --nocapture` (設定は `LEAD_*` 環境変数、`14-lead.md` §4。結果は `target/lead_report*.json`) |
| ベンチ | `cargo bench --workspace` (`hcp` < 10 ns、手サンプル ≥ 10^5/s、配牌 ≥ 10^4/s、`interpret` < 10 μs、BML コンパイル < 1 s) |
| 1 クレートのベンチ | `cargo bench -p bridge-constraint -- sampler` |
| カバレッジレポート (フェーズ 4) | `cargo xtask coverage --system systems/sayc/sayc.bml --corpus corpus/data/pbn` |
| fuzz | `cargo +nightly fuzz run pbn_parse_lenient -- -max_total_time=600` (`crates/bridge-format/fuzz/`) |
| スナップショット更新 | `cargo insta review` (`cargo install cargo-insta`) |
| バインディング参照の再生成 | `cargo xtask dds regen-bindings` (libclang が必要) |

レポート成果物 (計画 §13): `target/coverage_report.json` (NoCandidate 集計)、認識率レポート (`target/recognition_report.json`)、ナチュラル推定精度 (`target/natural_metrics.json`)、ESS レポート (`target/ess_report.json`)、再現率 (`target/reproduction_report.json`)。

## 12. 未決

- 未決: 再現率の閾値 (フェーズ 4 で中央値 ≥ 0.6、それ以前は報告のみ)。
- 未決: `random_position` の `random_call_rate` 既定値 0.05 (フェーズ 3.11 で Partial/Natural を入れた後に調整)。
- 未決: `deny.toml` の最終的な許可ライセンス一覧 (§10)。
- 未決: フェーズ 6 の上位 3 リード命中率の閾値 X。計画 (15-phase4-plan.md) で「上位 3 ≥ 0.90 かつ上位 1 ≥ ベースライン (a)」に固定済み。最終ライン (フェーズ 4 + 5 の統合後) で測定した (`14-lead.md` §4.3): 評価分割の 100 ボードで上位 3 = 0.93、上位 1 = 0.78 ≥ (a) 0.74 (同じボードの比較は達成。数字の 0.808 とは評価分割で 0.78 < 0.808 で未達、フェーズ 3 のボードでは 0.83 で達成)。

## 13. ESS スイートと評価データ (フェーズ 4)

**評価データの分割 (D20).** コーパスのオークションは、`crates/bridge-bidding/tests/reproduction.rs` の `corpus_auctions` と同じ列挙 (`corpus/data/pbn` の PBN をパス順に、各ファイルのゲームを順に読み、`view()` がオークションを返したものに 0 から番号を振る。`view()` が失敗したゲームは前のゲームの引き継ぎも切る) で、偶数番目をチューニング分割、奇数番目を評価分割とする。`(ε, δ)` の最尤推定 (`PolicyParams::human()`) や受理率の下限などの調整はチューニング分割だけで行い、評価分割は判定にだけ使う。ESS スイートのコーパスケース (`crates/bridge-sample/tests/ess_suite.rs`) とリード評価 (`crates/bridge-lead/tests/corpus_eval.rs`、14-lead.md §4) はこの定義を共有する。

**固定ケース.** ESS スイートは `crates/bridge-sample/tests/data/ess_cases.txt` の 50 ケースを使う。1 行 1 ケースで、タブ区切りの `label`、`source` (`generated` / `corpus`)、ディーラー、バルネラビリティ、配牌 (PBN)、コール列。`#` 行はコメント。生成 25 は deal seed `0x5A7C_0005_0003` の一様配牌を SAYC の `replay` で競らせたもの (スラムレベルとパスアウトを除く)、コーパス 25 は評価分割から完全な配牌と契約を持つものを等間隔に選んだもの。`ESS_SUITE_WRITE_FIXTURE=1` で現在の SAYC から作り直す。レーン S (レベル下限) とフェーズ 4.6 のナチュラルの調整が B から入った後に作り直した (それ以前の版は生成 25 件中 16 件が現行の方策から外れていた)。レーン D (SAYC の行) の変更が入った統合時に、下のテストが落ちればもう一度作り直す。固定後は SAYC を変えてもケースは変わらない (解釈と尤度は毎回計算し直す)。`ess_fixture_parses` (無視しないテスト) がファイルの形を、`ess_fixture_generated_cases_are_on_policy` (同) が生成ケースの各コールを配牌の実際の手が現行の方策で選ぶこと (`p ≥ 0.01`) を確認し、SAYC や方策の変更で固定ケースが古くなったら落ちる。

**方策のプリセット.** 生成ケースは `PolicyParams::system_players()`、コーパスケースは `PolicyParams::human()` で重み付けし、解釈は `InterpretOptions::for_context` でその鏡像にする (09-sample.md §3.1)。

**報告.** 各ケースで一様、残差棄却なし、残差棄却あり (既定の下限、`ESS_SUITE_RESIDUAL_MIN_ACCEPTANCE` で変更可) の 3 通りを同じ seed で引き、ESS/n、試行あたり ESS (残差棄却のパイロット 128 回を含む)、受理率、予算切れ、サンプリング時間を記録する。全体・生成・コーパス別の中央値、有効サンプルあたりの時間、棄却あり / なしの時間比、実行前後の loadavg を `target/ess_report.json` (チューニングモードでは `ess_report_tune.json`) に出す。`ESS_SUITE_BREAKDOWN=0` で方策の内訳 (09-sample.md §10.2 の on / shared / off 分類) を省く。`ESS_SUITE_THREADS=1` は単一スレッドで引く (配牌と ESS は同じで、時間が負荷に左右されにくい。時間比の判断にはこちらを使う)。

**チューニングモード.** `ESS_SUITE_MODE=tune` は別の deal seed (`0x7E57_0005_0004`) とサンプリング seed、コーパスのチューニング分割から同じ規則でケースを作る (固定ファイルは使わない)。残差棄却を既定で無効にする判断 (有効サンプルあたりの時間が 1.16〜1.21 倍に悪化する) と受理率の下限 0.5 はここで決めた (09-sample.md §6.5)。

**判定.** 評価モードで固定ケースがそろっているときだけ、既定の提案 (残差棄却なし) について assert する: ESS/n の中央値が全体・生成とも 0.5 以上、コーパスが 0.4 以上、予算切れが 2 件以下、試行あたり ESS の中央値が 0.35 以上。時間比 (棄却ありのサンプリング時間が棄却なしの 2 倍以内かと、既定を決める有効サンプルあたりの時間) は負荷に左右されるので表示だけにする。
