# 14. `bridge-lead`: オープニングリードアドバイザ

本文書はフェーズ 6 (`12-roadmap.md` §7、6.1/6.2) の詳細設計である。`bridge-lead` は他のクレートに依存される L0〜L5 の一部ではなく、それらを消費する独立したアプリケーション層のクレート (ライブラリ + CLI) であり、`bridge-bidding::interpret` と `bridge-sample::sample_deals` が計算した「オークションに矛盾しない配牌の重み付き集合」を `bridge::dd::DoubleDummy` で解析し、オープニングリードの期待守備トリック数を推定する。

関連文書: `07-bidding.md` (`Table`, `interpret`, `sequence_log_likelihood`)、`09-sample.md` (`SampleContext`, `sample_deals`, `Proposal`, ESS)、`10-dds.md` (`lead_scores`, `Solutions::AllRanked`)、`03-format.md` (`GameView`)、`11-testing.md`。

---

## 1. 目的

オープニングリードは探索木を持たない 1 手の判断であり、かつビディングで得た情報を最大限使うべき局面である (仕様 §11、`12-roadmap.md` §7 の理由書き)。`bridge-lead` は次を提供する。

1. オークションとリーダーの手から、オークションに矛盾する確率が低い配牌を `bridge-sample` でサンプリングする。
2. 各サンプル配牌について `bridge::dd::DoubleDummy::lead_scores` でリーダーの 13 枚それぞれの守備トリック数を求める。
3. 自己正規化重点重みで集計し (平均、標準誤差、セット確率)、DD 値が全サンプルで一致するカードをグループ化し、上位 `top_k` 個のリードを返す。

システムやサンプラーの判断ロジックを持たない (`07-bidding.md` §1.1 と同じ原則): `bridge-lead` が書くのは集計とランキングだけで、制約や尤度の計算は下位クレートのものをそのまま使う。

## 2. API

```rust
// crates/bridge-lead/src/lib.rs (再エクスポート)
pub use bridge_bidding::{InterpretOptions, PolicyParams, Table};
pub use bridge_sample::{ConstraintProposal, Proposal, SampleOptions, UniformProposal};
pub use bridge::dd::{DdError, DoubleDummy};

pub struct LeadOptions {
    pub samples: usize,              // 既定 200
    pub seed: u64,                   // sample.seed を上書きする単一の種
    pub policy: PolicyParams,        // 尤度の方策。既定 PolicyParams::human() (フェーズ 4)
    pub interpret: InterpretOptions, // policy と implicit_pass は尤度のもので上書きされる (§3 手順 3)
    pub sample: SampleOptions,       // seed 以外のフィールド (attempts, threads) を使う
    pub top_k: usize,                // 既定 3
    pub scoring: LeadScoring,
}

pub enum LeadScoring {
    Tricks,                          // 平均守備トリック数の降順 (既定)
    SetProbability,                  // セット確率の降順
    Score,                            // 期待デクレアラースコアの昇順 (守備に最も不利なスコアが先頭)
}

pub struct LeadQuery<'a> {
    pub auction: &'a Auction,        // 完了かつパスアウトでない
    pub leader_hand: Hand,           // ちょうど 13 枚
}

pub struct LeadScore {
    pub card: Card,                  // グループの代表 (グループ内でランクが最も高いカード。グループはスートをまたぐことがある)
    pub equivalents: Vec<Card>,      // 全サンプルで card と守備トリック数が一致したカード
    pub mean_defence_tricks: f64,
    pub std_error: f64,
    pub set_probability: f64,        // P(守備トリック数 >= 8 − level)
    pub rank: usize,                 // 1 始まり、scoring による順位
}

pub struct LeadAdvice {
    pub contract: Contract,
    pub declarer: Seat,
    pub leader: Seat,
    pub leads: Vec<LeadScore>,       // 長さ <= top_k、グループ化後の代表のみ
    pub sample_report: SampleReport, // bridge_sample の報告 (ESS など)
    pub samples_used: usize,         // sample_report.produced と同じ
    pub ess: f64,                    // sample_report.ess と同じ (便宜フィールド)
}

pub enum LeadError {
    IncompleteAuction,
    PassedOut,
    WrongHandSize { got: usize },
    Sample(bridge_sample::SampleError),
    Dd(bridge::dd::DdError),
}

pub fn advise(
    table: &Table,
    query: &LeadQuery<'_>,
    proposal: &dyn Proposal,
    dd: &dyn DoubleDummy,
    opts: &LeadOptions,
) -> Result<LeadAdvice, LeadError>;

/// リード助言用の提案: 残差棄却ありの ConstraintProposal (サンプラーの既定は棄却なし)、受理率の下限 0.125 (サンプラーの既定は 0.5)
pub fn lead_proposal() -> ConstraintProposal;
```

サンプラーの既定 (`ConstraintProposal::default()`) は、ESS スイートで有効サンプルあたりの時間が悪化するので残差棄却を無効にしている (09-sample.md §6.5)。`lead_proposal` がそれを有効にし、下限も既定より下げるのは、生成した配牌 1 つごとに DD 解析 (試行 1 回よりはるかに高い) が走るので、試行を増やしてでも重みを平らにしたほうが得だから (09-sample.md §6.5)。CLI (`lead-advisor`) とコーパス評価はこれを使う。

`LeadOptions.seed` と `LeadOptions.sample.seed` が両方あるのは冗長に見えるが、意図的である: 呼び出し側は `seed` だけを設定すればよく、`advise` が内部で `SampleOptions { seed: opts.seed, ..opts.sample }` を組み立てて `sample_deals` に渡す。`opts.sample` は attempts/threads だけを運ぶ器になる。

## 3. アルゴリズム (`advise` の手順)

1. **契約とリーダー**: `!auction.is_complete()` なら `IncompleteAuction`。`auction.contract()` が `None` (パスアウト) なら `PassedOut`。そうでなければ `contract = auction.contract().unwrap()`, `declarer = contract.declarer`, `leader = contract.leader()` (`declarer.next()`、`bridge-core` にすでにある), `trump = contract.bid.strain()`。
2. **手の検査**: `query.leader_hand.len() != 13` なら `WrongHandSize`。
3. **解釈**: `known = KnownCards::from_viewer(leader, query.leader_hand)`。プレイは未開始なので `play_constraints = [HandConstraint::ANY; 4]`, `play_soft = None`。`bid_ctx = BidContext { scoring: Scoring::Imp, natural: None, implicit_pass: ImplicitPass::Complement, policy: opts.policy }` を `advise` の中だけで組み立て (`Scoring` は `07-bidding.md` により v1 では素通しなので任意の値でよく、`natural: None` は尤度が `table.natural` を自動補完するので同じ結果になる)、`interpretation = bridge_bidding::interpret(table, query.auction, &InterpretOptions { policy, implicit_pass, ..opts.interpret })` とする。`policy` と `implicit_pass` は `InterpretOptions::for_context(&bid_ctx)` のもので、解釈は常に尤度の方策の鏡像になる (D19、09-sample.md §3.1)。方策の既定が `PolicyParams::human()` なのは、実際の卓ではシステム外のナチュラルなコールが多く (コーパスのオークションの大半)、`system_players()` ではそれがほぼ不可能な逸脱として読まれるため。
4. **文脈**: `ctx = SampleContext { known, interpretation: &interpretation, play_constraints: &play_constraints, play_soft: None, bidding: Some(BiddingLikelihood { table, auction: query.auction, ctx: &bid_ctx }) }`。尤度は `AuctionPolicy::log_likelihood` (D18) で計算される。
5. **サンプリング**: `sample_opts = SampleOptions { seed: opts.seed, ..opts.sample }`、`(deals, report) = sample_deals(&ctx, proposal, opts.samples, &sample_opts)?`。
6. **DD 解析**: `deals` の各要素について `dd.lead_scores(&weighted.deal, trump, leader)` を呼ぶ。`leader` の手は既知で固定なので、返る 13 枚の集合はどのサンプルでも同一である (facade の `lead_scores` は「リーダーの手の枚数だけ返す」契約、`crates/bridge/tests/dds.rs` の `assert_eq!(scores.len(), 13, ..)` で確認済み)。`parallel` feature では `deals.par_iter()` (rayon) で解析し、`collect::<Result<Vec<_>, _>>()` で順序を保ったまま集める (`.collect()` は要素順を保証する)。エラーは最初の 1 件を `LeadError::Dd` として伝播する。
7. **集計**: 正規化重み `w_i = WeightedDeal::normalized_weights(&deals)` (前段の `report.ess` と同じ対数重みから計算されるので、カードごとに作り直さない)。カード `c` ごとに:
   - `mean(c) = Σ_i w_i · score_i(c)`
   - `var(c) = Σ_i w_i · (score_i(c) − mean(c))²` (正規化重みでの重み付き分散)
   - `std_error(c) = sqrt(var(c) / ess)` (`ess = report.ess`。全カード共通の重みなので `ess` も共通)
   - `set_probability(c) = Σ_i w_i · [score_i(c) >= threshold]`, `threshold = 8 − contract.bid.level()` (守備側が契約を落とすのに必要なトリック数: デクレアラーに必要な `6 + level` に対し `13 − (6 + level) + 1 = 8 − level`)
8. **同値グループ化**: カード `c1`, `c2` が全サンプルで `score_i(c1) == score_i(c2)` (`i` を通じて完全一致するベクトル) ならグループに束ねる。代表はグループ内でランクが最も高いカード (エースが最高、スートは問わない。同点はカードのインデックス順で解消)、`equivalents` は代表を除いた残りをランク降順で並べる。「全サンプルで一致」は DDS 自身の `equals` (1 配牌内でのタッチする名誉札などの同値) を出発点とする条件で、facade はコミット `7bbae2a` 以降この `equals` を展開して返す (以前は捨てていた) ので、本クレートはそれを 1 配牌内の下限として、複数配牌にわたって頑健な同値だけを拾う。
9. **順位付けと切り詰め**: グループを `opts.scoring` に従って並べ替え (同点はカードのインデックス昇順で決定的に解消)、`rank` を 1 から振り、`opts.top_k` 個に切り詰める。
10. `LeadAdvice { contract, declarer, leader, leads, samples_used: report.produced, ess: report.ess, sample_report: report }` を返す。

### 3.1 `LeadScoring::Score` の中身 (「安くつくなら Imps/Matchpoints も」)

`bridge-core` に得点表がまだ無いため、本クレートに最小限の複製点数計算を持つ (`scoring.rs`、非公開)。標準のデュプリケート得点 (アンダートリックのペナルティ、メイドのボーナス、ダブル/リダブル) を `Contract`、`auction.vulnerability().is_vulnerable(declarer)` (呼び出し側に別途渡させない: オークション自身が持つ情報である)、各サンプルのデクレアラートリック数 `13 − score_i(card)` から計算する。得点は非線形 (ゲームボーナスの閾値など) なので、平均トリック数に事後で式を当てはめるのではなく、サンプルごとの得点をまず計算してから重み付き平均を取る。昇順 (デクレアラーにとって最も悪いスコアが先頭 = 守備にとって最良のリード) に並べる。真の IMP/MP (他家のテーブルとの比較) は他契約の DD 値が要るため対象外とし、未決事項に残す。

## 4. 評価方法

- **単体テスト** (`tests/`、非 `#[ignore]`、本レーンで実行できるもの): 契約/リーダー導出、エラー系 (未完了、パスアウト、手の枚数違い)、集計の数式 (手計算値との一致)、同値グループ化、決定性 (同じ seed で同一の `LeadAdvice`、`parallel` feature 有無で同一)。DDS を使わないダミーの `DoubleDummy` 実装 (`tests/common/mod.rs` の `FakeDd`: リーダーの手だけから決まる決定的なルールで守備トリック数を返す。配牌の残り 39 枚に依存しないので、期待値が厳密に手計算できる) を使う。オークションとシステムは `bridge-system` を dev-dependency にして `bridge-bidding/tests/common/mod.rs` と同じ手法 (`SystemBuilder` 相当) で手組みする (単体テストは特定システムの入札表に依存させないため。実システムは下のコーパス評価で使う)。
- **DDS smoke テスト** (`--features dds`、非 `#[ignore]`、少数サンプル): 固定の配牌とオークションで `bridge::dd::dds()` を呼び、実際に解ける (`None` ならスキップ、ベンダリング済みなら solve する) ことを確認する。
- **コーパス評価ハーネス** (`tests/corpus_eval.rs`、`#[ignore]`、`--features dds`): `corpus/data/pbn` 以下を再帰的に探索したファイルをパス順にソートし、その順で「完了したオークション・完全な配牌・パスアウトでない契約」を持つボードを先頭から 100 件選ぶ (決定的だがシードは使わない)。フェーズ 4 からは既定で D20 の評価分割 (11-testing.md §13: `corpus_auctions` の列挙で奇数番目) から選ぶ (`LEAD_SPLIT=eval`)。`LEAD_SPLIT=all` はフェーズ 3 までと同じ選び方 (分割なし、§4.1 の 100 ボード。現状のコーパスでは `OptimumPlayTable.pbn` の 1 件と Bermuda Bowl 2019 決勝の 4 ファイル) で、比較の連続性のために残す。複数イベントにまたがる層化抽出は未決事項に残す。設定は環境変数で選ぶ: `LEAD_POLICY` (`human` 既定、`system`、`legacy1` = 退役したソフトマックス `PolicyParams::legacy(1.0)`)、`LEAD_INTERPRET` (`mirror` 既定: 尤度の方策の鏡像、`legacy1` では鏡像が無いので `human` の鏡像を提案に使う。`legacy`: フェーズ 3 の解釈 `InterpretOptions::legacy()`)、`LEAD_RESIDUAL` (既定 `1`: 提案は `lead_proposal()`、`0` で残差棄却なし)。解釈が尤度の方策の鏡像なら `advise` そのものを、そうでなければ同じパイプラインを `advise_with_context` で呼ぶ。各ボードで:
  - `truth`: 実際の配牌に対する `dd.lead_scores` の全 13 枚のスコアと、その最大値を達成するカード集合。
  - `advice`: `systems/sayc/sayc.bml` (`BRIDGE_SYSTEMS_DIR` 環境変数、既定はワークスペース直下の `systems/`) を `bridge_system::compile` (facade 経由 `bridge::system::compile`) でコンパイルし、`lead_proposal()` を使った `advise(...)` (サンプル数は環境変数 `LEAD_SAMPLES`、既定 100)。release で数分かかるため `#[ignore]` (実行方法は §4.1)。環境変数 `LEAD_UNIFORM=1` は本評価 (`advice`) のサンプリングだけを `ConstraintProposal` から `UniformProposal` に差し替える (ビディング尤度による重み付けはそのまま残る)。ベースライン (a) はこのフラグと無関係に、`advise` と同じ集計パイプライン (`advise_with_context`、`#[doc(hidden)]`) を `UniformProposal` かつ `bidding: None` (解釈は空、`ANY` 相当) で呼んで計算する。方策・解釈・提案に依存しないので、ボードごとの結果を `target/lead_eval/baseline_a-n{samples}-seed{seed}-{split}-boards{N}/` にキャッシュし、設定をまたいで共有する。
  - `hit` (主指標): 上位 k (k = 1, 3) のグループの**代表カード** (実際にリードするカード) に `truth` の要素が 1 つでも入っているか。同じ判定関数 (`hits_truth`) をベースライン (a) にも使う。ベースライン (b) も k 枚の単独カードを選ぶので同じ土俵で比較できる。
  - `group_hit` (副指標): グループの `equivalents` まで含めて数えた命中 (`hits_truth_group`)。上位 k のグループは k 枚より多くのカードを覆う (このコーパスでは上位 3 で平均 5.4 枚、上位 1 で 2.0 枚) ので、(b) と比べるときは k ではなく覆った枚数で (b) を評価した `baseline_random_covered_*` と比べる。各ボードの記録に `top{1,3}_cards_covered` を残す。
  - `tricks_lost`: 上位 1 のカードが**実際の配牌**で達成する守備トリック数 (`truth.all_scores` から引く) と `truth.max` の差。サンプルにわたる推定平均 (`mean_defence_tricks`) ではなく、選んだカードの実測値を使う (推定バイアスではなく選択の結果を測るため)。`mean_estimation_error_top1` として `|mean_defence_tricks − 実測値|` も別途報告する。
  - ボードごとの結果は設定ごとのディレクトリ `target/lead_eval/n{samples}-{proposal}-{policy}-{interpret}-{residual|plain}-{split}-seed{seed}-boards{N}/board_NNN.json` に書き、実行のたびにそのディレクトリの記録のうち同じ設定 (サンプル数・提案・方策・解釈・残差棄却と下限・分割・seed・選択ボード数・記録形式の版 `record_version`、現在 3) のものすべてから、その設定のレポートを作り直す。レポート名は既定設定 (n = 100、`ConstraintProposal`、`human`、鏡像、残差棄却あり、評価分割、seed 0、100 ボード) なら `target/lead_report.json`、それ以外は既定と異なる項目を並べた `target/lead_report_<suffix>.json` (例: `lead_report_n500.json`、`lead_report_legacy1.json`、`lead_report_all.json`)。記録には受理率、試行あたり ESS、予算切れも残す。設定の違う実行が互いの記録やレポートを上書きしない。`LEAD_BOARDS=a..b` (選択ボードの半開区間) で分割実行でき、`boards_with_records == boards_selected` になれば完全。実配牌の DD 解析や `advise` がエラー (`NoSamples` など) になったボードはパニックせず、理由付きで `skipped` に記録して命中率の分母から外す。
  - 出力 `target/lead_report.json` (ワークスペース直下の `target/`、`CARGO_MANIFEST_DIR` からの相対ではない): 上位 1/3 命中率 (主指標と副指標 `group_hit_rate_*`、それぞれ全ボードと、DD 同値クラスが 2 つ以上ある「非自明」ボードに絞った版の両方)、上位 1/3 が覆う平均カード枚数、上位 1 の選択が最適から失う実測の平均 DD トリック数、平均推定誤差、ESS の統計 (平均・中央値・最小・最大、ESS/n ≥ 0.5 のボード数、ESS < 5 のボード数)、ボードあたりの時間、スキップしたボードと理由、ベースライン (a) 無ビディング情報 (`UniformProposal`、解釈なし、`advise` と同じグループ化と命中判定) と (b) ランダム選択 (リーダーの**カード**13 枚から `k` 枚を無作為に選んだときに最適カードを 1 枚以上含む超幾何確率 `1 − C(13−m, k) / C(13, k)`、`m` は `truth` の最適カード枚数。DD 同値クラスの個数ではない — このコーパスは 1 ボードあたり最大 3 クラスしかなく、クラス単位で 3 つ選べば常に 1.0 になってしまうため。副指標との比較用に `k` を覆った枚数にした版も出す) の 2 つ。

### 4.1 測定結果 (2026-09-27、`wip/p6int`、フェーズ 3 の方策と解釈)

実行: `cargo test -p bridge-lead --release --features dds,parallel --test corpus_eval -- --ignored --nocapture` (10 コアの macOS、他のワークフローと同時実行)。SAYC (`systems/sayc/sayc.bml`) を 4 席共通に使い、`ConstraintProposal`、seed 0。3 回の実行で結果はビット一致 (決定性)。命中率は主指標 (代表カード)。ボードごとの記録は `target/lead_eval/n100-constraint-seed0-boards100/` と `target/lead_eval/n500-constraint-seed0-boards100/` に並んで残る。

| 項目 | n = 100 (既定、`target/lead_report.json`) | n = 500 (参考、`target/lead_report_n500.json`) |
| --- | --- | --- |
| 評価ボード / スキップ | 99 / 1 | 99 / 1 |
| 非自明ボード (DD 同値クラス ≥ 2) | 70 | 70 |
| 上位 1 命中率 (全体 / 非自明) | **0.758** / 0.657 | 0.798 / 0.714 |
| 上位 3 命中率 (全体 / 非自明) | **0.939** / 0.914 | 0.929 / 0.900 |
| 副指標: グループ込み命中率 上位 1 / 3 | 0.758 / 0.939 | 0.798 / 0.929 |
| 上位 1 / 3 が覆う平均カード枚数 | 2.02 / 5.43 | 1.96 / 5.17 |
| 上位 1 の実測損失 (平均 DD トリック) | 0.283 | 0.242 |
| 上位 1 の推定誤差 (平均 \|推定 − 実測\|) | 1.52 | 1.33 |
| ベースライン (a) 無ビディング 上位 1 / 3 | 0.808 / 0.899 | 0.838 / 0.909 |
| ベースライン (b) ランダム 上位 1 / 3 | 0.655 / 0.879 | 0.655 / 0.879 |
| ベースライン (b) を覆った枚数で評価 (副指標用) 上位 1 / 3 | 0.784 / 0.953 | 0.770 / 0.949 |
| ESS 平均 / 中央値 / 最小 / 最大 | 6.94 / 4.01 / 1.00 / 39.0 | 29.1 / 12.2 / 1.00 / 181 |
| ESS/n 平均 / 中央値 | 0.069 / 0.040 | 0.058 / 0.024 |
| ESS/n ≥ 0.5 のボード / ESS < 5 のボード | 0 / 60 | 0 / 24 |
| 実行時間 (100 ボード、ベースライン (a) 込み) | 105.6 s (最良、3 回: 154.8 / 105.6 / 133.7 s、開始時 loadavg 8.12 / 22.09 / 18.63) | 355.5 s + 232.9 s (2 分割、開始時 loadavg 18.12 / 36.13) |

スキップ 1 件は `bermuda-bowl-2019-final/65946.pbn` のボード 23 (4H、ゲーム 13): `ConstraintProposal` が 100 回 (n = 500 では 500 回) の提案をすべて棄却し `LeadError::NoSamples`。リーダー自身の手が SAYC の解釈と両立しない (§3 手順 5 の想定どおりの経路)。

所見:

- 上位 3 は実際にリードする代表カードで DD 最善を 93.9% で含み、ベースライン (a) 89.9%、(b) 87.9% を上回る (どれも k 枚の単独カードで数えた同じ土俵の比較)。グループ込みの副指標はこのデータでは主指標と全ボードで一致する (同値カードで命中して代表カードで外すボードが無い) が、覆う枚数をそろえたランダム選択 (上位 3 で 0.953、上位 1 で 0.784) には届かない: グループ込みの数え方は同値カードの多さで水増しされるので、主指標で読む。ただし上位 1 は 75.8% で (a) の 80.8% を**下回る**。原因は ESS の低さ: ビディング尤度の重み (`sequence_log_likelihood`) で ESS 中央値が n = 100 で 4.0 (ESS/n 0.04) しかなく、少数の配牌が集計を支配する。ESS ≥ 20 のボード (n = 100 で 8 件、n = 500 で 38 件) に限ると上位 1 は 0.875 / 0.895 で (a) と同等、ESS < 5 のボードでは (a) より 5〜8 ポイント低い。フェーズ 5.3 の ESS 未達 (`12-roadmap.md` §6、09-sample.md §10.2: `interpret` と方策 `call_distribution` の不整合) がそのまま効いている。
- n を 5 倍にすると上位 1 は 4 ポイント上がるが上位 3 は 1 ポイント下がり、ESS/n はむしろ下がる (重みの裾が重い)。サンプル数ではなく提案と目標の乖離が律速。
- ランダム選択でも上位 3 が 0.879 になるのは、このコーパスでは最善カードが平均して多い (同値カードが多い) ため。命中率は必ずベースラインとの差で読む。

### 4.2 フェーズ 4 の測定結果 (2026-09-27、`wip/p4-P`)

実行: 上と同じコマンド (10 コアの macOS、他のワークフローと同時実行、loadavg 5.9〜29.5)。B の最新のマージ (`AuctionPolicy` の位置メモなど) と、09-sample.md §6.4 (d) の畳み込みのフォールバックの後に取り直した値。n = 100、seed 0、100 ボード、スキップ 0。解釈は方策の鏡像 (D19)、尤度は D18 の方策 (`AuctionPolicy`)、提案は `lead_proposal()` (残差棄却、下限 0.125)。`human()` はレーン D の最尤推定が入る前の仮値 (ε = 0.01、δ = 0.3)。決定的で、同じ設定を 2 回回した結果はビット一致。2026-09-29 に既定の設定を取り直し (サンプラーの既定を残差棄却なしに変え、パイロットを試行あたり ESS に算入した後。`lead_proposal()` は変わらないので配牌も同じ)、上位 1 / 3 = 0.80 / 0.94、ESS 中央値 85.6、ベースライン (a) 0.74 / 0.91 で一致した (70 s、loadavg 6.7 → 17.2)。記録の `ess_per_attempt` は今はパイロット 128 回を含む。

評価分割 (既定、`LEAD_SPLIT=eval`) の 100 ボード。非自明ボード 71。ベースライン (a) 上位 1 / 3 = 0.74 / 0.91、(b) = 0.659 / 0.879。

| 設定 | 上位 1 | 上位 3 | 上位 1 / 3 (非自明) | ESS 中央値 / 最小 | ESS < 5 のボード | 受理率の中央値 | 上位 1 の実測損失 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **`human`、鏡像、残差棄却 (既定)** | **0.80** | **0.94** | 0.718 / 0.915 | **85.6** / 1.3 | 3 | 0.619 | 0.24 |
| `system`、鏡像、残差棄却 | 0.78 | 0.95 | 0.690 / 0.930 | 88.8 / 3.9 | 1 | 0.643 | 0.26 |
| `human`、鏡像、残差棄却なし | 0.80 | 0.92 | 0.718 / 0.887 | 53.2 / 1.9 | 4 | 1.0 | 0.22 |
| `legacy1` (τ = 1)、`human` の鏡像、残差棄却 | 0.78 | 0.94 | 0.690 / 0.915 | 52.6 / 1.0 | 12 | 0.619 | 0.26 |
| `legacy1`、フェーズ 3 の解釈、残差棄却 | 0.75 | 0.89 | 0.648 / 0.845 | 9.8 / 1.0 | 31 | 0.725 | 0.28 |

フェーズ 3 の 100 ボード (`LEAD_SPLIT=all`、§4.1 と同じボード)。非自明ボード 70。ベースライン (a) 上位 1 / 3 = 0.81 / 0.90 (§4.1 の 0.808 / 0.899 と同じボード、スキップが無くなったので 100 件)、(b) = 0.659 / 0.881。

| 設定 | 上位 1 | 上位 3 | ESS 中央値 | ESS < 5 のボード |
| --- | --- | --- | --- | --- |
| `human`、鏡像、残差棄却 | **0.85** | 0.93 | 83.6 | 2 |
| `legacy1`、`human` の鏡像、残差棄却 | 0.85 | 0.95 | 54.7 | 7 |

B の最新のマージ前の値 (評価分割で `human` 0.78 / 0.94・ESS 85.7、`legacy1` 0.76 / 0.94・ESS 53.9、フェーズ 3 のボードで 0.88 / 0.92 と 0.82 / 0.93) と比べ、結論は変わらない。

所見:

- **判定 (計画の P、リード助言)**: 既定の設定で上位 3 = 0.94 ≥ 0.90、上位 1 = 0.80 ≥ 同じボードのベースライン (a) 0.74 (フェーズ 3 のボードでは 0.85 ≥ 0.81、計画に書かれた 0.808 も上回る)、ESS 中央値 85.6 ≥ 20 (フェーズ 3 は 4.0)、予算切れ 0 件。§4.1 で上位 1 が (a) を下回っていた原因 (ESS の低さ) は解消した。**注意**: 計画 (15-phase4-plan.md) の条件は「上位 1 ≥ ベースライン (a) (0.808)」で、0.808 はフェーズ 3 のボードでの (a) の値。ここではベースライン (a) をボード集合ごとに測り直して同じボードで比べており、評価分割の上位 1 = 0.80 は数字としての 0.808 を下回る。評価分割は (a) 自体が 0.74 と難しいボードの集合なので同じボードでの比較が妥当と考えるが、閾値の読み替えなので統合時に計画の持ち主が確認すること。
- **hard (D18) と τ = 1 のソフトマックスの比較**: 同じ提案 (`human` の鏡像) で比べると、上位 1 は hard が 0.80 対 0.78 (評価分割)、0.85 対 0.85 (フェーズ 3 のボード) で、hard が悪くなることはない。計画の再検討条件 (hard が上位 1 で 0.02 を超えて悪い) には当たらないので D18 は維持する。τ = 1 は重みの裾が重く、ESS 中央値が 53〜55 と hard の 84〜86 より低い。`legacy_temperature` はこの比較の役目を終えたので、統合後に削除してよい (15-phase4-plan.md の未決事項 3)。
- 残差棄却は ESS 中央値を 53 → 86 に上げ、ESS < 5 のボードを 4 → 3 に減らし、上位 3 を 0.02 上げる (上位 1 は同じ)。受理率 0.62 で試行は約 1.6 倍になるが、時間の大半は DD 解析なので 1 ボードあたりの時間はほぼ変わらない (0.5〜1.4 s、負荷による)。
- `system` プリセットと `human` の仮値の差は 0.01〜0.02 で、どちらが良いとは言えない (上位 1 は `human` 0.80 対 0.78、上位 3 は `system` 0.95 対 0.94)。`human()` の値がレーン D の最尤推定に置き換わったら再測定する。
- フェーズ 3 の解釈 (`InterpretOptions::legacy()`) のままでは、同じ τ = 1 の尤度でも ESS 中央値 9.8 (ESS < 5 が 31 ボード) で上位 3 が 0.89 に落ちる。改善は鏡像の解釈 (D19) によるもので、方策の形 (D18) の寄与は上位 1 の差の分。

## 5. 未決事項

| # | 項目 | 現状 |
| --- | --- | --- |
| 1 | 上位 3 命中率の閾値 X (`12-roadmap.md` §7、6.2) | 計画 (15-phase4-plan.md、フェーズ 6) で「100 ボードで上位 3 ≥ 0.90 かつ上位 1 ≥ ベースライン (a)、`human` プリセット」に固定。フェーズ 4 の測定 (§4.2) で達成: 評価分割で 0.94 / 0.80 (a: 0.74)。`human()` の最尤推定値が入った統合時に再測定する |
| 5 | `human()` の値 | 仮値 (ε = 0.01、δ = 0.3)。レーン D のチューニング分割での最尤推定に置き換える (D18、D20) |
| 2 | `LeadScoring::Score` の得点表を `bridge-core` に上げて共有するか | 現状は `bridge-lead` 内に複製 (非公開)。他クレートが得点計算を必要にした時点で `bridge-core` へ移す |
| 3 | 真の IMP/Matchpoints (他契約との比較) | 対象外。他契約の DD 値の総当たりが要り、フェーズ 6 の範囲を超える |
| 4 | `LeadOptions.sample.seed` を無視して `opts.seed` で上書きする API は分かりにくいという指摘 | 現状の決定。単一の `seed` を露出したいという設計上の理由を doc コメントに明記する |
