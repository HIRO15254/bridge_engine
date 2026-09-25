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
pub use bridge_bidding::{InterpretOptions, Table};
pub use bridge_sample::{Proposal, SampleOptions, UniformProposal};
pub use bridge::dd::{DdError, DoubleDummy};

pub struct LeadOptions {
    pub samples: usize,              // 既定 200
    pub seed: u64,                   // sample.seed を上書きする単一の種
    pub interpret: InterpretOptions,
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
```

`LeadOptions.seed` と `LeadOptions.sample.seed` が両方あるのは冗長に見えるが、意図的である: 呼び出し側は `seed` だけを設定すればよく、`advise` が内部で `SampleOptions { seed: opts.seed, ..opts.sample }` を組み立てて `sample_deals` に渡す。`opts.sample` は attempts/threads だけを運ぶ器になる。

## 3. アルゴリズム (`advise` の手順)

1. **契約とリーダー**: `!auction.is_complete()` なら `IncompleteAuction`。`auction.contract()` が `None` (パスアウト) なら `PassedOut`。そうでなければ `contract = auction.contract().unwrap()`, `declarer = contract.declarer`, `leader = contract.leader()` (`declarer.next()`、`bridge-core` にすでにある), `trump = contract.bid.strain()`。
2. **手の検査**: `query.leader_hand.len() != 13` なら `WrongHandSize`。
3. **解釈**: `known = KnownCards::from_viewer(leader, query.leader_hand)`。プレイは未開始なので `play_constraints = [HandConstraint::ANY; 4]`, `play_soft = None`。`interpretation = bridge_bidding::interpret(table, query.auction, &opts.interpret)`。
4. **文脈**: `bid_ctx = BidContext { scoring: Scoring::Imp, natural: None, implicit_pass: ImplicitPass::Complement, policy: PolicyParams::default() }` を `advise` の中だけで組み立てる (`Scoring` は `07-bidding.md` により v1 では素通しなので任意の値でよく、`natural: None` は `sequence_log_likelihood` が `table.natural` を自動補完するので同じ結果になる)。`ctx = SampleContext { known, interpretation: &interpretation, play_constraints: &play_constraints, play_soft: None, bidding: Some(BiddingLikelihood { table, auction: query.auction, ctx: &bid_ctx }) }`。
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

- **単体テスト** (`tests/`、非 `#[ignore]`、本レーンで実行できるもの): 契約/リーダー導出、エラー系 (未完了、パスアウト、手の枚数違い)、集計の数式 (手計算値との一致)、同値グループ化、決定性 (同じ seed で同一の `LeadAdvice`、`parallel` feature 有無で同一)。DDS を使わないダミーの `DoubleDummy` 実装 (`tests/common/mod.rs` の `FakeDd`: リーダーの手だけから決まる決定的なルールで守備トリック数を返す。配牌の残り 39 枚に依存しないので、期待値が厳密に手計算できる) を使う。オークションとシステムは `bridge-system` を dev-dependency にして `bridge-bidding/tests/common/mod.rs` と同じ手法 (`SystemBuilder` 相当) で手組みする — `bridge_system::compile` は本レーンの基点でまだ `todo!()` である。
- **DDS smoke テスト** (`--features dds`、非 `#[ignore]`、少数サンプル): 固定の配牌とオークションで `bridge::dd::dds()` を呼び、実際に解ける (`None` ならスキップ、ベンダリング済みなら solve する) ことを確認する。
- **コーパス評価ハーネス** (`tests/corpus_eval.rs`、`#[ignore]`、`--features dds`): `corpus/data/pbn` 以下を再帰的に探索したファイルをパス順にソートし、その順で「完了したオークション・完全な配牌・パスアウトでない契約」を持つボードを先頭から 100 件選ぶ (決定的だがシードは使わない。現状のコーパスでは 1 ファイル分に収まるため、複数イベントにまたがる層化抽出は未決事項に残す)。各ボードで:
  - `truth`: 実際の配牌に対する `dd.lead_scores` の全 13 枚のスコアと、その最大値を達成するカード集合。
  - `advice`: `systems/sayc.bml` (`BRIDGE_SYSTEMS_DIR` 環境変数、既定はワークスペース直下の `systems/`) を `bridge_system::compile` (facade 経由 `bridge::system::compile`) でコンパイルし、`bridge_sample::ConstraintProposal` を使った `advise(...)` (サンプル数は環境変数 `LEAD_SAMPLES`、既定 100)。`bridge_system::compile` と `ConstraintProposal` は他レーンの担当で本レーン基点では `todo!()` なので、このテストは **今はコンパイルだけを保証し、実行はしない** (`#[ignore]` に加え、実装が揃うまでは統合担当が回す)。環境変数 `LEAD_UNIFORM=1` は本評価 (`advice`) のサンプリングだけを `ConstraintProposal` から `UniformProposal` に差し替える (ビディング尤度による重み付けはそのまま残る)。ベースライン (a) はこのフラグと無関係に、`advise` と同じ集計パイプライン (`advise_with_context`、`#[doc(hidden)]`) を `UniformProposal` かつ `bidding: None` (解釈は空、`ANY` 相当) で呼んで毎回別途計算する。
  - `hit`: 上位 3 (グループの `equivalents` を含めて数える) に `truth` の要素が 1 つでも入っているか。同じ判定関数 (`hits_truth`) をベースライン (a) にも使う。
  - `tricks_lost`: 上位 1 のカードが**実際の配牌**で達成する守備トリック数 (`truth.all_scores` から引く) と `truth.max` の差。サンプルにわたる推定平均 (`mean_defence_tricks`) ではなく、選んだカードの実測値を使う (推定バイアスではなく選択の結果を測るため)。`mean_estimation_error_top1` として `|mean_defence_tricks − 実測値|` も別途報告する。
  - 出力 `target/lead_report.json` (ワークスペース直下の `target/`、`CARGO_MANIFEST_DIR` からの相対ではない): 上位 1/3 命中率 (全ボードと、DD 同値クラスが 2 つ以上ある「非自明」ボードに絞った版の両方)、上位 1 の選択が最適から失う実測の平均 DD トリック数、平均推定誤差、平均 ESS 比、ボードあたりの時間、ベースライン (a) 無ビディング情報 (`UniformProposal`、解釈なし、`advise` と同じグループ化と命中判定) と (b) ランダム選択 (リーダーの**カード**13 枚から `k` 枚を無作為に選んだときに最適カードを 1 枚以上含む超幾何確率 `1 − C(13−m, k) / C(13, k)`、`m` は `truth` の最適カード枚数。DD 同値クラスの個数ではない — このコーパスは 1 ボードあたり最大 3 クラスしかなく、クラス単位で 3 つ選べば常に 1.0 になってしまうため) の 2 つ。

## 5. 未決事項

| # | 項目 | 現状 |
| --- | --- | --- |
| 1 | 上位 3 命中率の閾値 X (`12-roadmap.md` §7、6.2) | コーパス評価が実行できてから測って決める (他レーンの `compile`/`ConstraintProposal` 待ち) |
| 2 | `LeadScoring::Score` の得点表を `bridge-core` に上げて共有するか | 現状は `bridge-lead` 内に複製 (非公開)。他クレートが得点計算を必要にした時点で `bridge-core` へ移す |
| 3 | 真の IMP/Matchpoints (他契約との比較) | 対象外。他契約の DD 値の総当たりが要り、フェーズ 6 の範囲を超える |
| 4 | `LeadOptions.sample.seed` を無視して `opts.seed` で上書きする API は分かりにくいという指摘 | 現状の決定。単一の `seed` を露出したいという設計上の理由を doc コメントに明記する |
