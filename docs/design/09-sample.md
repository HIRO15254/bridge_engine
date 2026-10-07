# 09. L4 `bridge-sample`: 配牌サンプラー

本書は L4 `bridge-sample` の詳細設計である。提案分布は `Proposal`（文脈ごとに 1 回の `prepare`）と `PreparedProposal`（配牌ごとの `propose` / `log_prob`）の 2 段トレイトにし、重点重みは対数領域で `ln w = ln L − ln π`、ESS は log-sum-exp で計算して `INFO` で常時報告する。RNG は `Xoshiro256PlusPlus` を明示し、サンプル `i` の乱数列を `(master_seed, i)` から splitmix64 で派生させることでスレッド数・ターゲットに依らず同一結果を得る（D12）。

関連文書: 05-constraint.md（`Sampler`、`KnownCards`、DNF）、07-bidding.md（`Interpretation`、`sequence_log_likelihood`）、08-play.md（`hard` / `soft`）、11-testing.md、13-decisions.md（D2、D3、D12、D13）。

---

## 1. 位置づけと依存

| 項目 | 内容 |
| --- | --- |
| クレート | `bridge-sample`（lib 名 `bridge_sample`） |
| 依存 | `bridge-core`、`bridge-constraint`、`bridge-bidding`、`bridge-play`（経由で `bridge-system`、`bridge-eval`）。`KnownCards` を再エクスポートする |
| 外部依存 | `rand_core` 0.10（`rand_core::Rng` が dyn 互換の基底トレイト）、`rand_xoshiro` 0.8（`Xoshiro256PlusPlus`）、`rayon` 1.12（optional）、`tracing`、`thiserror`、`serde`（optional） |
| feature | `default = ["std"]`、`std`、`serde`、`parallel = ["dep:rayon"]`（wasm32 では無効） |
| 出力 | 確率の表ではなく **重み付き配牌の集合**。下流の探索エンジンが必要とするのはサンプルであって密度関数ではない（仕様 §7） |

`rand_core` 0.10 では `RngCore` は非推奨の別名で、`Rng` が dyn 互換の基底トレイトである（`rand::Rng` は `RngExt` に改名。edition 2024 で `gen` が予約語のため 0.10 系を使う、D14）。

---

## 2. API

### 2.1 `Proposal` / `PreparedProposal`

```rust
use rand_core::Rng;

pub trait Proposal: Send + Sync {
    /// 文脈ごとに 1 回: DNF、席ごとの計画、席順、カウントのキャッシュ
    fn prepare<'c>(&self, ctx: &'c SampleContext<'c>) -> Result<Box<dyn PreparedProposal + Send + Sync + 'c>, SampleError>;
}
pub trait PreparedProposal {
    fn propose(&self, rng: &mut dyn Rng) -> Option<Deal>;   // None = 棄却された試行
    fn log_prob(&self, deal: &Deal) -> f64;                 // ln π(deal)。支持集合外は −∞
    fn pilot_attempts(&self) -> u64 { 0 }                   // 準備中に引いた提案の数（§6.5 のパイロット）
}
```

2 段階にする理由: 第 1 席の `Sampler::prepare` と DNF は文脈不変であり、毎秒 10^4 回再計算してはならない。`prepare` はフルデッキで 20〜60 μs かかる（05-constraint.md）。仕様の 1 段 API（`propose(ctx, rng)`）では実装がこのキャッシュを持てない。仕様 §7 の `propose(ctx, rng)` / `log_prob(ctx, deal)` は `proposal.prepare(ctx)?` に続けて呼ぶだけなので、便宜ラッパーは置かない。`sample_deals` は `propose` → `log_prob` の 2 回呼びで、`ConstraintProposal::log_prob` は席 2 以降の `Sampler::prepare` を再実行する（§6.3）。この二重計算が予算（§6.4）を超えるなら `propose_with_log_prob` をトレイトに足す（未決、フェーズ 5.4 のベンチで決める）。

差し替え可能な実装（仕様 §7）:

| 実装 | 内容 | 段階 |
| --- | --- | --- |
| `UniformProposal` | 既知カードを除いて一様に配る | v0 ベースライン（§5） |
| `ConstraintProposal` | 制約から階層サンプリング | v1 本命（§6） |
| `NeuralProposal` | 学習済み分布から提案（別クレート） | v2 |

v0 と v1 の差（ESS）を測れることが重要で、ニューラル版に進む前に制約ベースの到達点を数値で確認する。

### 2.2 文脈・結果・オプション

```rust
pub struct SampleContext<'a> {
    pub known: KnownCards,                                        // 自分の手、ダミー、既出カード
    pub interpretation: &'a Interpretation,                       // 07-bidding.md
    pub play_constraints: &'a [HandConstraint; 4],                // ハード制約。全代替に AND
    pub play_soft: Option<&'a [Vec<(HandConstraint, f32)>; 4]>,   // v1 軟情報（混合）
    pub bidding: Option<BiddingLikelihood<'a>>,                   // L の計算用。None なら interpretation.likelihood
}
pub struct BiddingLikelihood<'a> { pub table: &'a Table, pub auction: &'a Auction, pub ctx: &'a BidContext<'a> }

pub struct WeightedDeal { pub deal: Deal, pub log_weight: f64 }
impl WeightedDeal { pub fn normalized_weights(deals: &[WeightedDeal]) -> Vec<f64>; }   // Σ = 1

pub struct SampleOptions {
    pub seed: u64,
    pub max_attempts_per_sample: u32,   // 16。1 スロットあたりの試行上限
    pub max_attempt_factor: u32,        // 20。試行予算: 総試行数が n × 20 に達したら打ち切り（§6.5）
    pub threads: Threads,               // Auto | Single。結果は同一
}
pub enum Threads { Auto, Single }

pub struct SampleReport {
    pub requested: usize, pub produced: usize, pub attempts: u64,
    pub pilot_attempts: u64,            // 準備中に引いた提案（§6.5 のパイロット 128 回、なければ 0）。attempts・予算・受理率には入れない
    pub acceptance_rate: f64,           // produced / attempts
    pub ess: f64, pub ess_ratio: f64,   // ess / requested
    pub ess_per_attempt: f64,           // ess / (attempts + pilot_attempts)。棄却分とパイロットも数える比較用の指標
    pub log_weight_max: f64,
    pub budget_exhausted: bool,         // n 件そろう前に試行予算を使い切った
    pub elapsed: Duration,
    pub warnings: Vec<SampleWarning>,
}
pub enum SampleWarning {
    CustomConstraint { seat: Seat },            // 棄却法に退化した代替がある
    LowEss { ess: f64, requested: usize },      // ess < 0.5 × requested（フェーズ 5 の完了条件と同じ閾値）
    Truncated { produced: usize },              // n 件に届かなかった
    BudgetExhausted { attempts: u64, produced: usize },   // 試行予算（n × max_attempt_factor）を使い切った
    EmptySupport { seat: Seat },                // その席の代替がすべて既知カードと矛盾（ANY にフォールバック）
}
#[derive(Clone, PartialEq, Debug, thiserror::Error)]
pub enum SampleError {
    #[error("proposal could not be prepared: {0}")] Prepare(String),   // KnownCardsError / PrepareError / 文脈不整合の文字列化
    #[error("empty support: no deal satisfies the constraints")] EmptySupport,   // ハード制約と既知カードだけで配牌が存在しない
}

pub fn sample_deals(ctx: &SampleContext<'_>, proposal: &dyn Proposal, n: usize, opts: &SampleOptions)
    -> Result<(Vec<WeightedDeal>, SampleReport), SampleError>;
```

`SampleContext` と `BiddingLikelihood` は `Clone + Copy`、`SampleOptions` は `Clone + Copy + PartialEq + Eq + Debug + Default`（`seed: 0`）、`Threads` は `Default` が `Auto`。

### 2.3 `sample_deals` の手順

1. `ctx.known` の不変条件（互いに素、各 ≤ 13）と `Σ_s needed(s) == pool().len()` を確認し（違反は `SampleError::Prepare`）、`prepared = proposal.prepare(ctx)?`。ハード制約 `play_constraints` と既知カードだけで配牌が存在しない（ある席の `play_constraints[s]` が `known[s]` を固定して `count() == 0`）なら `prepare` は `Err(SampleError::EmptySupport)` を返し、試行は行わない。ある席の解釈代替がすべて落ちた場合は `SampleWarning::EmptySupport { seat }` を記録してその席を `ANY` にフォールバックし（尤度が補正する）、サンプリングは続ける。
2. スロット `i = 0, 1, 2, …` を **`n` 個ずつのチャンク** で処理する。スロット `i` は `rng = rng_for(opts.seed, i)` だけを使い、最大 `max_attempts_per_sample` 回 `propose` を試し、得られた `deal` に `log_prob` を呼ぶ。`(deal, ln π)` に対し §3 の `ln L` を計算し、有限なら `WeightedDeal { deal, log_weight: ln L − ln π }` をスロットの結果とする。`−∞` は棄却として数え、次の試行に進む。
3. 各チャンクの `n` 件のスロット結果を **スロット順に** たたみ込み、`deals.len()` が `n` に達した時点でそのチャンクの残りのスロットは `attempts` にも `deals` にも加えない（採用されない配牌の試行回数で `attempts`/`acceptance_rate` を水増ししないため）。試行予算も同じくスロット順に適用し、`attempts` が予算 `n × max_attempt_factor` に達したスロットでたたみ込みを止める（報告される `attempts` が予算を超えるのは最後のスロットの `max_attempts_per_sample` 回まで）。`produced < n` のまま予算に達したら `budget_exhausted = true` と `SampleWarning::BudgetExhausted` を報告する。次のチャンクを起動するかどうかの判定（`produced ≥ n` または `attempts ≥ n × max_attempt_factor` なら停止）はチャンク境界で行うが、この判定はチャンク単位の集計値 `produced`/`attempts` を見るだけで、どのスロットがそのチャンクの「余剰」かはスロット順（スレッド数に依らない）だけで決まるので、結果は依然としてスレッド数に依らない。
4. スロット順に最初の `n` 件を採用する（手順3により `deals.len()` は `n` を超えない）。
5. §3.2 の式で `ess` を計算し、`SampleReport` を組み立てる。`tracing::info!(requested, produced, attempts, pilot_attempts, acceptance_rate, ess, ess_ratio, ess_per_attempt, log_weight_max, budget_exhausted, elapsed_us)` を **常時 INFO** で出す（仕様 §9: ESS 報告は性能問題の一次診断情報）。メッセージは `"deal sampling finished"`。フィールドが返り値の `SampleReport` と一致することは `tests/tracing_info.rs` が最小の `tracing::Subscriber` で確認する（5.3）。

---

## 3. 重点重み

### 3.1 目標分布と尤度の分解

目標は元の配牌 `d` の事後分布 `P(d | auction, play) ∝ L(d) · P(d)`。`P(d)` は既知カードと整合する配牌上で一様なので定数であり、自己正規化で消える。

```text
ln L(d) = ln P(auction | d)                         // ビディング尤度
        + Σ_s ln 1[hard_s(h_s)]                     // プレイのハード制約（08-play.md §6）
        + Σ_s ln Σ_i w_{s,i} · 1[soft_{s,i}(h_s)]   // プレイの軟情報（08-play.md §7）
```

| 項 | `ctx.bidding` が `Some` | `None` |
| --- | --- | --- |
| `ln P(auction | d)` | `AuctionPolicy::new(table, auction, bid_ctx).log_likelihood(d)`（07-bidding.md §6、D18。`AuctionPolicy` は `sample_deals` の冒頭で 1 回だけ作り、局面ごとの候補を前計算するので、配牌あたりは各局面の選択の再評価だけ） | `Σ_s ln interpretation.likelihood(s, h_s)`（集合所属質量） |

`Some` のときは解釈の制約は提案分布の構築にだけ使われ、尤度は D18 の方策 `p(c|h) = (1 − ε)·[(1 − δ)·S + δ·M] + ε/n` から得る（`S` は `choose_bid` の決定的な選択、`M` はナチュラル方策の選択、`n` は合法コール数）。ε 床があるので支持集合は全合法コールに広がる。`(ε, δ)` は `BidContext::policy` のプリセットで、SAYC で生成したオークションには `PolicyParams::system_players()`（ε = 1e-3、δ = 0）、人間のオークション（コーパス、リード助言）には `PolicyParams::human()`（D20 のチューニング分割での最尤推定値、統合までは仮値 ε = 0.01、δ = 0.3）を使う。解釈は `InterpretOptions::for_context(&bid_ctx)` で同じ方策の鏡像として作る（D19）。各片の重みが方策の密度に較正されているので、件数比例の成分抽選（§6.1 点 4）がそのまま `q ∝ L` に近い提案になる。旧来の `priority / τ` のソフトマックスは D18 で廃止し、比較用に残していた `PolicyParams::legacy(τ)` もフェーズ 6 のリード評価の後に削除した（13-decisions D18）。`None` のときは ε-混合の防御枝が `likelihood` に ε の質量を残し、制約外の手にも正の尤度を与える。

### 3.2 重み・ESS・自己正規化

- `ln w_i = ln L(d_i) − ln π(d_i)`。`ln w_i = −∞` のサンプルは捨てて棄却として数える。
- `LSE(x) = m + ln Σ_i exp(x_i − m)`、`m = max_i x_i`（`weights::log_sum_exp`。空なら −∞）。
- `ESS = (Σ w)^2 / Σ w^2 = exp(2 · LSE(lw) − LSE(2 · lw))`（`weights::effective_sample_size(&log_weights)`）。
- 正規化重み `w̃_i = exp(lw_i − LSE(lw))`（`WeightedDeal::normalized_weights(&deals)`）。

**自己正規化に関する注記**: 重みはすべて `LSE(lw)` で正規化されるので、`π` の定数因子は打ち消される。具体的には (1) 棄却する提案の受理正規化定数（受理された `d` の密度は `π(d) / α` だが `α` は `d` に依らない）、(2) 配牌上の一様事前分布、(3) `UniformProposal` の定数 `log_prob`。したがって `log_prob` は **`d` に依存する部分が正確** であればよい。`SampleReport` の doc にこれを明記する。

ESS が要求数を大きく下回る場合、提案分布が悪いか制約が厳しすぎる。フェーズ 5 の完了条件は「ESS が要求数の 50% 以上」。

---

## 4. `KnownCards` の置き場

`KnownCards` は `bridge-constraint` に置く（計画 §4.1、05-constraint.md）。理由: `Sampler::prepare(c, pool, fixed, opts)` の `fixed = known[s]`、`pool = known.pool()` という対応が L1 の契約であり、L4 と L5 の両方が同じ型を使う。

```rust
/// 各席の元の手に属すると分かっているカード（自分の手、ダミー、その席が出したカード）
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KnownCards { pub known: [Hand; 4] }
impl KnownCards {
    pub fn new(known: [Hand; 4]) -> Result<Self, KnownCardsError>;   // 互いに素、各 ≤ 13
    pub const EMPTY: KnownCards;
    pub fn pool(&self) -> Hand;                 // FULL − 和集合
    pub fn needed(&self, s: Seat) -> u8;        // 13 − known[s].len()
    pub fn from_viewer(viewer: Seat, hand: Hand) -> Self;
    pub fn with_dummy(self, dummy: Seat, hand: Hand) -> Self;
    pub fn with_play(self, history: &PlayHistory) -> Self;   // 各席の既出カードを加える（08-play.md §5）
}
```

プレイ途中の文脈は `KnownCards::from_viewer(viewer, hand).with_dummy(dummy, dummy_hand).with_play(&history)` で組み立てる。

サンプリングの対象は常に **元の配牌** である。ビディング制約もプレイ制約も元の 13 枚について述べ、確定分は `Sampler` がスート表の構築時に合成する（D3）。

---

## 5. `UniformProposal`（v0）

```rust
#[derive(Clone, Copy, Debug, Default)]
pub struct UniformProposal;
struct PreparedUniform<'c> { ctx: &'c SampleContext<'c>, log_prob: f64 }   // pool と needed は ctx.known から都度読む
```

- `prepare`: `known.pool()` から `needed(s)` を計算し、`log_prob = −ln(|pool|! / Π_s needed(s)!)` を 53 要素の `ln_factorial` テーブルで求める。文脈ごとの定数。
- `propose`: `pool` を Fisher-Yates でシャッフルし、席順に先頭から `needed(s)` 枚ずつ割り当て、`known[s]` と和を取る。棄却なし（ハード制約もその他もすべて `L` が扱う）。
- `log_prob`: 定数を返す（`deal` が `known` と整合しなければ −∞）。
- 有界整数の抽選は自前の `bounded(rng, n)`（Lemire の nearly-divisionless、棄却付き）で行い、`rand` 内部の範囲抽選アルゴリズムに乱数列が依存しないようにする。`rand` エコシステムから使うのは `Xoshiro256PlusPlus::from_seed` と `next_u64` だけである。

---

## 6. `ConstraintProposal`（v1）

L1 の API（05-constraint.md）:

```rust
impl Sampler {
    pub fn prepare(c: &HandConstraint, pool: Hand, fixed: Hand, opts: &SampleOptions) -> Result<Sampler, PrepareError>;  // pool ∩ fixed = ∅
    pub fn count(&self) -> u64;  pub fn is_exact(&self) -> bool;
    pub fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Option<Sample>;   // Sample { hand, log_prob, tries }
    pub fn log_prob(&self, hand: Hand) -> f64;   // ln(Σ_{i ∋ h} 1/α_i) − ln C。非会員は −∞
}
```

`Sampler` は `HandConstraint` を受け取って内部で DNF 項ごとに準備し、項を `c_i` 比例で選んで一様抽選する（`P(h) = m(h) / C`、`m(h)` は `h` を含む項の数。05-constraint.md §4.4）。L4 は代替 1 つにつき `Sampler` 1 つを持ち、DNF の項レベルの重み `u_{i,l} = w_i · cnt_{i,l} / Σ_l cnt_{i,l}` は `Sampler` の内部に隠れる。

### 6.1 `prepare(ctx)`

1. **席ごとの代替**: `A_s = interpretation.seats[s] ⊗ play_soft[s]`（直積、重みは積。K = 8 を超える場合は推定質量 `w_i · count_i`（フルプールでの件数を掛けた、その代替の抽選上の取り分）の上位 K − 1 個を残し、残りを `play_constraints[s]` だけの受け皿候補 1 個（重みは落とした分の合計）にまとめる。`w_i` だけで並べると、軽いが広い代替（質量の大半を担うことがある）を落としてしまう。単純に上位 K 個へ切り詰めると、落とした積だけが覆う手（典型的には重みの低い入札解釈の全体）が目標分布では正の重みを持つのに提案密度 0 となって一度も引かれず、ESS は高いまま推定が偏るため）。`play_soft` が `None` なら `interpretation.seats[s]` そのもの。各代替を `play_constraints[s]` と `and` する。`needed(s) == 0` の席（自分、ダミー）は対象外。
2. **Sampler の準備**: 代替 `i` について `S_{s,i} = Sampler::prepare(&C_{s,i}, pool, known[s], opts)`（`pool = known.pool()`）。`count() == 0` の代替は落とす。`is_exact() == false`（`Custom` / `residual` / スロット超過を含む）なら `SampleWarning::CustomConstraint { seat: s }`。その代替は `Sampler` 内部で棄却法（`max_tries = 256`）になる。
3. **制約の厳しさ**: `mass_s = Σ_i w_i · S_{s,i}.count()`。席順 `σ` を `ln mass_s` 昇順（同点は席 index）に決める。制約が厳しい席から順に引く。`σ` の最後の席は残りを受け取る。全代替が落ちた席があれば `EmptySupport { seat }` を報告し、支持集合が空であることを `prepared` に記録する。
4. **キャッシュ**: 席 `σ_1` の `Sampler` 群と成分重み `v_i = w_i · cnt_i / Σ_{j: cnt_j > 0} w_j · cnt_j` を保持する（`σ_1` のプールは固定なので再利用できる）。`cnt_i` を掛けるのは、`Interpretation::likelihood` が採点する目標（満たす代替の `w` の和、07-bidding.md §4.4）における各代替の取り分は、その代替が満たすハンド数に比例するため — `count` を掛けずに `w_i` だけで選ぶと、満たすハンドが多い代替（ε-混合の防御枝 `ANY` など）が実際の取り分より少なくしか引かれず、その代替が生む少数のハンドの重要度重みが過大になって ESS が崩れる。席 `σ_2` 以降はプールが縮むので `propose` ごとに準備し直す（§6.4 (c)(d) の粗い要約と軽い代替の畳み込みを使う）。
5. **無制約の検出**: 代替 `C` が `ANY`（要約が `shapes = ALL`、`hcp = 0..=37`、`cards` / `eval` 空）なら `Sampler` を作らず、組合せ的な直接配りにマークする（§6.4 (a)）。ε-混合の防御枝の積（全コール Fallback の組合せ）はこれに該当する。

### 6.2 `propose(rng)`

`|σ| = m` とする。`k = 1..=m−1` について:

1. `P_k` = `pool` からそれまでの席の抽選分を除いたもの。`s = σ_k`。
2. `k == 1` ならキャッシュを使う。それ以外は各代替について `S^{(k)}_{s,i} = Sampler::prepare(&C_{s,i}, P_k, known[s], opts)` を呼ぶ。`count() == 0` の代替は落とし、残りを `v_i ∝ w_i · S^{(k)}_{s,i}.count()` で正規化する（§6.1 点 4 と同じ重み付け）。残りがなければ `None`。
3. 成分 `i` を `v_i` 比例で選び、`h_s = S^{(k)}_{s,i}.sample(rng)?.hand`（棄却法の試行切れなら `None`）。
4. `ln π_k = ln Σ_{i'} v_{i'} · exp(S^{(k)}_{s,i'}.log_prob(h_s))`（重なる成分をすべて数える。§6.3）。
5. `P_{k+1} = P_k − h_s`。

最後の席 `σ_m`: `h = P_m ∪ known[σ_m]`。`play_constraints[σ_m].satisfies(h)` かつ `∃ i: C_{σ_m,i}.satisfies(h)` でなければ `None`。合格なら `Deal::new(hands)`（`debug_assert!` で成功を確認）。再試行は `sample_deals` の側で、そのスロット自身の乱数列から最大 `max_attempts_per_sample` 回行う。

### 6.3 `log_prob(deal)`

`deal` から同じ `σ` とプール列を再現し、`k = 1..=m−1` について `π_k(h_{σ_k}) = Σ_{i: h ∈ C_{σ_k,i}} v_i · exp(S^{(k)}_{σ_k,i}.log_prob(h))` を計算し、`ln π = Σ_k ln π_k` とする。項レベルに展開すると `π_k(h) = Σ_{(i,l): h ∈ a_{i,l}} u_{i,l} / cnt_{i,l}` であり、**重なる DNF 項・重なる代替をすべて足す**。混合密度は `h` を生成し得た全成分を数えなければ正しくない。最後の席は残りで決まるので対数領域で 0 を加える。いずれかの `π_k = 0`、または最後の席の検査に落ちる `deal` は −∞。

`log_prob` は実際の提案分布に対して厳密であり、これが ESS を正直な数値にする。`propose` が準備した席 2 以降の `S^{(k)}` は、同じスレッドで直後に呼ばれる同じ配牌の `log_prob` のためにスレッドローカルに残し、書き込んだ提案の id（`prepare` ごとに一意な連番。アドレスは解放後に再利用されうるので使わない）と `(pool, fixed)` の両方が一致したときだけ再利用する（一致しなければ自分の粗い候補から再準備するので、別の提案の配牌を同じスレッドで採点しても結果は常に厳密）。このため配牌ごとの `prepare` は §6.4 の見積りどおり 1 回分で済み、トレイト拡張（`propose_with_log_prob`）は足さない（§10.1）。

### 6.4 性能リスクと対策（計画 §8.3、R4）

| 数値 | 出典 |
| --- | --- |
| `Sampler::prepare`: フルデッキ 20〜60 μs、列挙が要るスート 1 つにつき +65 μs、未知 26 枚でシェイプが絞られた項なら 3〜10 μs・無制約 (560 シェイプ) なら 12〜15 μs | 05-constraint.md §9 |
| `Sampler::sample`: 0.3〜0.5 μs | 同上 |
| 配牌ごとに席 2〜3 で ≈ 5〜12 回の `prepare`（各 20〜40 μs） | 計画 §8.3 |
| 目標 10^4 配牌 / 秒 / コア = 100 μs / 配牌 | 仕様 §9 |

このままでは席 2〜3 の `prepare` だけで予算を超える。≈ 12 回の `prepare` を見込むなら 1 回あたり ≲ 5 μs に収める必要がある。上表のとおりシェイプ無制約の項はこれに届かない (実測 12〜15 μs) ので、そのような項を席 2 以降の提案に使うなら以下の (a)〜(c) のいずれかが必須になる。対策は 3 つで、フェーズ 5.4 のベンチで測って選ぶ。

| 対策 | 内容 | 効果 |
| --- | --- | --- |
| (a) 無制約席の直接配り | 相手がパスのみ等で代替が `ANY` の席は `Sampler` を作らず `C(|P_k|, needed)` から一様に引く。`log_prob = −ln C(|P_k|, needed)` | 対象席の `prepare` が 0 回 |
| (b) 対畳み込みの到達可能シェイプ限定 | `(l0,l1)`・`(l2,l3)` 対の疎畳み込みを、制約が許すシェイプが使う対（各 ≤ 105）に限定する（05-constraint.md、設計済み） | `prepare` の定数を下げる |
| (c) 粗い提案 | 再 prepare される席（2 番目から最後の 1 つ前まで）を、リテラルを持たない (シェイプ, HCP) 格子上の上側近似で提案し、細部は `L` の重みで補正する（下記） | `prepare` が高速パスに乗る。ESS は下がるが有限 |
| (d) 軽い代替の畳み込み | 同じ席で、質量の取り分が手の取り分に比べて無視できる代替を 1 つの一様成分にまとめる（下記） | 1 回の `propose` の `prepare` が主な代替の分だけになる |

**(c) の要約（フェーズ 4）.** 代替がリテラルを持たないアトム、またはその平坦な `Or`（方策鏡像の排他片、07-bidding.md §4）なら、それ自体が `Sampler` の厳密で安い経路に乗るので変えない。それ以外は `bridge_constraint::grid::bounds(c).sup`（(シェイプ, HCP) 格子上で `c` を含む最小の集合）をアトムの `Or` に戻したもの（最大 8 アトム、超える分は広げてまとめる）にする。格子が 1 つの箱ならアトム 1 つ、空なら `Or([])`。フェーズ 3 までの「`shapes()` と `hcp_range()` だけの 1 アトム」と違い、`Not(より上位の兄弟)` のような木の形を格子で表せる範囲で残すので、鏡像の片が要約で潰れない。要約は常に元の代替を含む（上側近似）ので、満たせる代替を満たせなくすることはない。

**(d) 軽い代替の畳み込み（`ConstraintProposal::light_threshold`、既定 1e-2）.** 方策鏡像は各席に重み 1e-5 程度の `Fallback` 片（`N_sys`、`N_nat`、他のコールの片）を数個ずつ残し、その粗い要約は主な片と大きく重なる。これらを毎回 `prepare` すると 1 回の `propose` の大半を占めていた。再 prepare される席で、代替 `j` のフルプールでの質量の取り分 `π_j = w_j · cnt_j / Σ_i w_i · cnt_i` が手の取り分 `f_j = cnt_j / C(|pool|, needed)` の `light_threshold` 倍以下なら、その代替を「軽い」とし、1 つの一様成分（残りプールから `needed` 枚を一様に配る）にまとめて、固定の抽選確率 `π_L = Σ_{軽い j} π_j` を与える。その席の密度は

```text
q(h) = (1 − π_L) · Σ_{i: 残した代替, h ∈ C_i} v_i(P_k) / cnt_i(P_k) + π_L / C(|P_k|, needed)
```

で、どんな固定の `π_L` でも厳密に計算できる。一様成分は軽い代替の手をすべて覆うので支持集合は変わらない。比 `π_j / f_j` は、その代替の手だけに落ちた配牌が `E[w²]` に足す分をおよそ上から押さえるので、既定値での ESS の損失は数 % 以内（実測では ESS スイートの中央値が 0.4295 → 0.4247、残差棄却ありで 0.9164 → 0.9228）。最も重い代替は常に残す。`propose` + `log_prob` は 1 配牌あたり Stayman で 79 → 31 μs、競り合いの 4♠ で 63 → 35 μs になった（§10.2 の続き）。

**畳み込みのフォールバック.** 軽い／重いの判定と `π_L` はフルプールで 1 回だけ決まるが、前の席の手によっては、残した代替が実際のプール `P_k` でほとんど場所を失い、畳み込んだ代替だけが入る余地を持つことがある（例: コーパスの `65936#26` で、残した片の質量が 1/600 程度に縮み、配牌の大半が一様成分から出て受理率 0.0015 で予算切れになった）。そこで毎回、残した代替の質量 `M_kept(P_k) = Σ_{残した i} w_i · cnt_i(P_k)` を、フルプールでの値に手の総数の縮み `C(|P_k|, needed) / C(|P_full|, needed)` を掛けたものと比べ、その 0.1 倍（`FOLD_FALLBACK_RATIO`）を下回ったら、その抽選だけは畳み込みをやめて全代替の適応的な混合（§6.4 (c) の粗い要約すべて）から引く。どちらを使うかはプール `P_k` だけで決まるので、`log_prob` は同じ判定を再現して厳密な密度を返す（`SeatMix`）。これで `65936#26` は ESS/n 0.99・受理率 0.98 に戻り、他のケースの数値は変わらない。

### 6.5 残差棄却と試行予算（フェーズ 4）

最後の席 `σ_m` は残りのカードをそのまま受け取るので、その席の尤度の因子は重みにそのまま残る（フェーズ 5.3 の分析で、最後の席が最悪の席になるのは 50 件中 26 件、§10.2）。`ConstraintProposal::residual_rejection`（既定 `false`、下の「既定値の決め方」）は、最後の席の手 `h` を確率

```text
a(h) = min(1, m(h) / T)、m(h) = Σ_{i: h ∈ C_{σ_m,i}} w_i
```

で受理し、`log_prob` に `ln a(h)` を足す。受理された配牌の密度は `π_{他の席}(d) · a(h) / P_acc` で、`P_acc` は配牌に依らないので自己正規化で消える（§3.2）。したがって **どんな固定の `T > 0` でも推定は厳密** で、`T` は分散を重みと棄却率のあいだで移すだけになる（重みには `m(h)` の代わりに `max(m(h), T)` が残る）。

`T` は `prepare` で 1 回だけ、次の 3 つの最小値として決める。

1. `U`: `m` の全手にわたる上界。各代替 `i` について、格子上の上側近似が `i` のものと交わる代替の重みの和をとり、その最大値（方策鏡像の互いに素な片なら `max_i w_i`）。これより上では棄却しても何も得られない。
2. 固定の乱数列（`rng_for(0x9E51_D0A1_0000_0001, 0)`）で残差棄却なしの提案を 128 回引いたパイロットで見た `m` の最大値。これより上ではパイロットの重みはすでに平ら。`T` は文脈の決定的な関数になる。
3. パイロットの平均受理率が `residual_min_acceptance`（既定 0.5）まで下がる `T`（対数領域の二分法）。重い代替にめったに入らない最後の席で試行予算を使い切らないため。

棄却された試行は通常の棄却として数え、`SampleOptions` の試行予算（既定 `20n`）に算入し、`acceptance_rate` と `ess_per_attempt` に現れる。棄却された試行は `log_prob` と尤度を計算しないので、生成された配牌より安い。パイロットの 128 回は `SampleReport::pilot_attempts`（`PreparedProposal::pilot_attempts`）として報告し、`ess_per_attempt = ess / (attempts + pilot_attempts)` に算入する（試行予算と `acceptance_rate` には入れない）。n = 1000 では試行あたり ESS が約 7 % 下がり、n = 100 のリード助言ではほぼ半分になる。

**既定値の決め方（計画の手順 6、D20）.** 有効サンプル 1 個あたりの壁時計時間が棄却なしより下がる場合に限り既定で有効にする。判断はチューニング集合（`ESS_SUITE_MODE=tune`）で行い、評価用の固定ケースは確認にだけ使う。

- 下限の掃引（チューニング集合、B の最新のマージ前、loadavg 7.7〜8.2）: 下限 0.125 / 0.3 / 0.4 / 0.5 / 0.6 で ESS/n の中央値 0.88 / 0.78 / 0.66 / 0.60 / 0.51、サンプリング時間は棄却なしの 4.8 / 3.2 / 2.4 / 1.9 / 1.7 倍、有効サンプルあたりの時間は 2.5 / 1.9 / 1.5 / 1.3 / 1.3 倍。どの下限でも有効サンプルあたりの時間は悪化する。
- 現行のコード（固定ケース再生成後、下限 0.5、`ESS_SUITE_THREADS=1` の単一スレッド計測、各 3 回）: チューニング集合で ESS/n 0.7492（棄却なし 0.5443）、サンプリング時間 1.56〜1.61 倍、有効サンプルあたり 1.16〜1.21 倍（loadavg 14.9〜15.4）。評価用固定ケースでも 1.68〜1.75 倍 / 1.20〜1.25 倍（loadavg 15.4〜16.1）。並列の既定（`Threads::Auto`）では 1 ケース約 10 ms の計測が負荷で揺れ、比が 0.66〜1.23 倍まで散るので、判断には単一スレッドの値を使う。試行あたり ESS も棄却なしより低い（評価用 0.5170 対 0.5702）。
- 最終ヘッド（2026-10-02、wip/p5final 8abe651、固定ケース再生成後の評価用固定ケース、下限 0.5）: `ESS_SUITE_THREADS=1` でサンプリング時間 1.69 倍、有効サンプルあたり 1.26 倍（loadavg 3.98）。`Threads::Auto` では 1.85〜1.89 倍 / 1.38〜1.42 倍（loadavg 3.98、2 回）。試行あたり ESS は棄却ありが 0.4627、棄却なしが 0.5471。判断は変わらない。

したがって **既定は無効**（`residual_rejection: false`）。ESS の完了条件（§10.2 の続き）は棄却なしで満たす。以前の版はこの規則ではなく「サンプリング時間が 2 倍以内」で下限 0.5 を選び既定で有効にしていたが、それは計画の規則と違い、しかも古い固定ケース（生成 25 件中 16 件が現行の方策から外れていた）で ESS を過小に見積もった結果だった。配牌 1 つごとに DD 解析が走るリード助言（14-lead.md）では、提案 1 回よりも生成された配牌 1 つのほうがはるかに高いので、棄却を有効にし、下限も 0.125 まで下げる（`bridge_lead::lead_proposal()`）。`residual_min_acceptance` の既定 0.5 は、有効にしたときの試行を配牌 1 つあたり約 2 回に抑える値として残す。

**残した改善案.** 最後の 2 席を同時に引く（最後の席の格子制約を 1 つ前の席の提案に移す）と、棄却なしで残差の分散を消せる可能性がある。未実装。

`prepare` の結果を `(代替 id, P_k)` でキャッシュする案は、プールがほぼ繰り返さないため効果が薄い（配牌をまたぐキャッシュは採らない。1 回の `propose` の中で同じ `P_k` に対する代替どうしは `Sampler::prepare_many` で表を共有する、§10.1）。`prepare` 自体が本質的に安いこと（スート単位の DP、列挙でない）が前提になる。

---

## 7. RNG と決定的並列（D12）

```rust
// crates/bridge-sample/src/rng.rs
pub type SampleRng = rand_xoshiro::Xoshiro256PlusPlus;   // rand::rngs::SmallRng は使わない

fn splitmix64(state: &mut u64) -> u64 {           // Vigna の参照実装の定数
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
pub fn rng_for(master: u64, index: u64) -> SampleRng {
    let mut z = master ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    let mut seed = [0u8; 32];
    for chunk in seed.chunks_mut(8) { chunk.copy_from_slice(&splitmix64(&mut z).to_le_bytes()); }
    SampleRng::from_seed(seed)   // from_seed はアルゴリズムで完全に規定される。seed_from_u64 は版間の安定性が保証されない
}
```

| 論点 | 決定 | 理由 |
| --- | --- | --- |
| RNG 型 | `Xoshiro256PlusPlus` を明示 | `SmallRng` は 64 bit では Xoshiro256++ だが 32 bit ターゲット（wasm32）では Xoshiro128++ になり、ネイティブとブラウザで結果が変わる |
| シード派生 | `rng_for(master, i)`: splitmix64 で 32 バイト種を生成し `from_seed` | サンプル `i` の乱数列が `(master, i)` だけで決まる |
| 再試行 | 同じスロットの乱数列から消費 | 他のスロットの結果に依存しない |
| 版固定 | `Cargo.lock` で `rand_core` / `rand_xoshiro` を固定。`from_seed` と `next_u64` 以外を使わない | 種の再現性 |
| 並列 | `parallel` では各チャンクを `(start..end).into_par_iter().map(one_slot).collect::<Vec<Option<_>>>()`。順序保持。単一スレッドは同じ関数を `map` で回す | `Vec<WeightedDeal>` と `SampleReport`（`elapsed` を除く）がバイト一致 |
| `Threads::Auto` | rayon のグローバルプール（`parallel` 無効時は `Single` と同じ） | |

決定性テスト: 同じ `seed` で `Threads::Single` と 7 スレッドの `rayon::ThreadPoolBuilder` プールで実行し、`Vec<WeightedDeal>` と `SampleReport`（`elapsed` 以外）の等値を assert する。

---

## 8. モジュール構成

| ファイル | 内容 |
| --- | --- |
| `lib.rs` | 再エクスポート（`bridge_constraint::KnownCards` を含む）、`SampleError`、`sample_deals`（チャンク駆動、`parallel` 分岐、tracing） |
| `proposal.rs` | `Proposal`、`PreparedProposal`、`SampleContext`、`BiddingLikelihood` |
| `uniform.rs` | `UniformProposal`、`ln_factorial`、`bounded`、Fisher-Yates |
| `constraint_proposal.rs` | `ConstraintProposal { max_retries: 16, residual_rejection: false, residual_min_acceptance: 0.5, light_threshold: 1e-2 }`、§6（prepare / propose / log_prob、席順、(c) の格子要約、(d) の畳み込み、§6.5 の残差棄却） |
| `weights.rs` | `WeightedDeal`、`log_sum_exp`、`effective_sample_size`、`normalized_weights` |
| `rng.rs` | `SampleRng`、`splitmix64`、`rng_for` |
| `report.rs` | `SampleOptions`、`Threads`、`SampleReport`、`SampleWarning` |
| `benches/deals.rs` | criterion（制約付き完全配牌） |

---

## 9. テストと性能目標

| テスト | 場所 | 種類 | 基準 |
| --- | --- | --- | --- |
| `deterministic_across_threads` | `tests/determinism.rs` | 同じ seed、`Single` vs 7 スレッドプール | バイト一致 |
| `log_prob_consistency` | `tests/log_prob.rs` | 小さいプール（未知 8〜12 枚）で `ConstraintProposal` から 10^5 回提案し、配牌ごとのヒストグラムを `exp(log_prob)` と χ² 比較 | 棄却されない（有意水準 0.01） |
| `middle_seat_multi_component_and_last_seat_rejection_log_prob_consistency` | `tests/log_prob.rs` | 未知 9 枚、再 prepare される中間席が生き残る 2 成分混合（coarsen で潰れない HCP 窓）を持ち、最終席が `Sampled` で棄却もあり得る文脈での同じ χ² 比較 | `Σ exp(log_prob) ≤ 1`、最終席のみの不整合で厳密に `-inf`、棄却されない |
| `middle_seat_merged_summaries_log_prob_consistency` | `tests/log_prob.rs` | 未知 9 枚、再 prepare される中間席に要約が一致する 2 代替 + 別の HCP 窓の代替（§10.1 (i) の併合経路）での全列挙 + χ²。`Σ exp(log_prob)` を提案の成功確率とし、10^5 回の提案の失敗率がそれと 5σ 以内で一致することも確認する（残差棄却がある文脈用） | 失敗率が一致、棄却されない |
| `middle_seat_light_tier_log_prob_consistency` | `tests/log_prob.rs` | 同じ文脈で `light_threshold = ∞`（最も重い代替以外をすべて §6.4 (d) の一様成分に畳み込む） | 同上 |
| `residual_log_prob_adds_ln_acceptance` | `tests/residual.rs` | 未知 8 枚・最後の席が `Sampled` の小さい文脈（重なる代替 / 互いに素な代替の 2 通り）を全列挙し、残差棄却ありの `log_prob` が「なし + `ln(m(h)/T)`」に等しいこと、`T` が §6.5 の上界 `U` に等しいこと | 差 < 1e-9 |
| `residual_proposals_match_log_prob` | `tests/residual.rs` | 同じ文脈で残差棄却ありの 4000 提案の受理率と配牌ヒストグラムを `exp(log_prob)` と χ² 比較 | 棄却されない（有意水準 0.01） |
| `weighted_estimates_are_unbiased` | `tests/residual.rs` | 同じ文脈と下の切り詰め + 粗化の文脈で `sample_deals`（n = 6000、残差棄却あり / なし）の自己正規化重み付き推定（ハートとダイヤのエースの持ち主、9 区分）を厳密な事後分布と比較する Wald χ²（共分散は SNIS のデルタ法） | p > 0.01 |
| `clipped_threshold_with_coarsened_middle_seat_is_exact` | `tests/residual.rs` | 多くの実ケースが通る経路: 最後の席（西: 赤のエース 2 枚 0.9 + `ANY` 0.1）のパイロット受理率が `U` で下限 0.5 を割り、`T` が二分法で `min m < T < max m` に決まって重い手では `a(h)` が 1 に切り詰められる。再 prepare される南は `cards` リテラル（ハートのキング）を粗化で落とされ、軽い `ANY` 代替が §6.4 (d) で一様成分に畳み込まれる。全列挙で `log_prob` の差 = `ln a`、下限付近の受理率、提案のプール付き χ² | 差 < 1e-9、棄却されない |
| `residual_pilot_is_charged_to_ess_per_attempt` | `tests/residual.rs` | 残差棄却ありでは `pilot_attempts == 128` で `ess_per_attempt = ess / (attempts + 128)`、なしでは 0 | |
| `attempt_budget_is_honoured_and_reported` | unit（`lib.rs`） | ほとんど受理しない提案で、総試行数が予算 `20n` を 1 スロット分以内で守り、`budget_exhausted` と `BudgetExhausted` を報告すること。予算内で終わる実行はどちらも報告しない | |
| `log_prob_matches_the_unmerged_reference` | unit（`constraint_proposal.rs`） | 実 SAYC に似た 4 席の文脈で、併合・`ANY` 直接配り・§6.4 (d) の畳み込み・§6.5 の残差棄却・スレッドローカルキャッシュを使う `log_prob` を、要約を 1 つずつ毎回 prepare する参照実装と比較（提案配牌と一様配牌の両方） | 差 < 1e-9、支持集合一致 |
| `fold_fallback_matches_the_reference` | unit（`constraint_proposal.rs`） | 北が 4 枚の 2 を持つか 22 点以上、南（再 prepare）が 19 点以上 + 畳み込まれる `ANY` の文脈で、畳み込みの混合とフォールバックの両方が 50 回以上起き、どちらでも `log_prob` が参照実装と一致すること | 差 < 1e-9 |
| `log_prob_ignores_another_proposals_cache_entries` | unit（`constraint_proposal.rs`） | 中間席の重みだけが違う 2 つの提案を同じスレッドで交互に使い、`a.propose` 直後の `b.log_prob` を参照実装と比較（キャッシュが提案 id で区別されることの回帰） | 差 < 1e-9 |
| `uniform_log_prob_constant` | unit | 全提案で `log_prob` が等しく、`Σ exp(log_prob)` が全列挙で 1 | |
| `ess_formula` | unit | 手計算の小例（等重み n 個 → ESS = n、1 個だけ重い → ESS ≈ 1） | |
| `normalized_weights_sum_to_one` | unit | | 1 ± 1e-12 |
| `empty_support_early_return` | unit | 矛盾する `play_constraints` | `Err(SampleError::EmptySupport)`、試行 0 回 |
| `seat_fallback_warns` | unit | ある席の解釈代替がすべて既知カードと矛盾 | `SampleWarning::EmptySupport { seat }`、その席は `ANY` で配られ `produced == n` |
| `uniform_vs_constraint_ess` | `tests/ess.rs`、`#[ignore]` | 手組みの合成解釈（`bidding = None`）での比較 | 報告のみ |
| `uniform_vs_constraint_ess_suite` | `tests/ess_suite.rs`、`#[ignore]`、release | 固定ケース `tests/data/ess_cases.txt` の 50 オークション（SAYC の `replay` 25 + コーパスの評価分割 25）× n = 1000、実ビディング尤度（生成は `system_players`、コーパスは `human`）、既知 = オープニングリーダーの手。一様・残差棄却なし・残差棄却ありの 3 通りについて ESS/n、試行あたり ESS、受理率、予算切れ、時間、loadavg を `target/ess_report.json` に出す（11-testing.md §13） | 既定の提案（残差棄却なし）で ESS/n の中央値 ≥ 0.5（全体と生成）、コーパス ≥ 0.4、予算切れ ≤ 2 件、試行あたり ESS ≥ 0.35（達成、§10.2 の続き）。残差棄却ありは並べて報告する。時間比（サンプリング時間と有効サンプルあたりの時間）は負荷に左右されるので表示のみ（`ESS_SUITE_THREADS=1` で単一スレッド計測）。最新（2026-10-02、wip/p5final 8abe651、固定ケース再生成後）は 0.5471 / 0.7113 / 0.4608、予算切れ 0、試行あたり 0.5471（2026-09-29 の旧ケースでは 0.5702 / 0.5695 / 0.5709） |
| `ess_fixture_parses` | `tests/ess_suite.rs` | 固定ケースが 25 + 25 件の完結したオークションと完全な配牌に読めること | |
| `ess_fixture_generated_cases_are_on_policy` | `tests/ess_suite.rs` | 固定ケースの生成 25 件の各コールを、配牌の実際の手が現行の方策（`system_players`、ナチュラルの予備）で `p ≥ 0.01` で選ぶこと。SAYC や方策の変更で外れたら `ESS_SUITE_WRITE_FIXTURE=1` で固定し直す | 外れたコール 0 |
| `sample_deals_emits_the_info_line_with_ess` | `tests/tracing_info.rs` | 最小の `tracing::Subscriber` で INFO 行を記録 | INFO 行が 1 本出て、`ess` / `ess_ratio` / `requested` / `produced` が `SampleReport` と一致 |
| `deals_bench` | `benches/deals.rs` | criterion、制約付き完全配牌。実 SAYC の 3 ケースは鏡像の解釈と `system_players` の方策尤度で、既定（`single_thread`、残差棄却なし）と `single_thread_residual`（あり、下限 0.5）の両方 | ≥ 10^4 配牌 / 秒 / コア（両方で達成、§10.2 の続き）。最新（2026-10-02、3 回の中央値）: Stayman 3NT 34.3 K（残差棄却あり 30.5 K）、競り合い 31.8 K / 30.4 K（27.6 K / 14.6 K） |

実装順（計画 §12）: 2.8 骨格（`UniformProposal`、`WeightedDeal`、ESS、`SampleReport`、`rng_for`、`parallel` + 決定性テスト。`bridge-core` と `KnownCards` だけで書ける）→ 5.2 `ConstraintProposal`（`log_prob` χ²）→ 5.3 `sample_deals` + ビディング尤度 + tracing（ESS スイート）→ 5.4 ベンチ + プロファイル（§6.4 の (a)(b)(c) を選ぶ）。

---

## 10. 未決事項

| # | 項目 | 現在の仮置き |
| --- | --- | --- |
| 1 | §6.4 の (a)(b)(c) のどれを採るか | 決定（フェーズ 5.4 のベンチ、下記） |
| 2 | `play_soft` を L5 側で `interpretation.seats` に事前結合するか | L4 で `⊗` する（L5 は L3 に依存しないため） |
| 3 | `Threads::Auto` で専用プールを作るか | rayon のグローバルプール |
| 4 | `PreparedProposal` に `propose_with_log_prob` を足すか（§2.1、§6.3） | 足さない（下記、実測を踏まえて確定） |
| 5 | `bidding` が `Some` のとき `interpretation.per_call.len()` と `auction.len()` の不一致を `SampleError::Prepare` にするか | 検査する（最小限） |

`LowEss` の閾値は `ess < 0.5 × requested` で確定（フェーズ 5 の完了条件と同じ）。

### 10.1 フェーズ 5.4 のベンチ結果と (a)(b)(c) の採否

**計測条件.** release、`Threads::Single`、Apple Silicon 10 コア。別のワークフローが同じマシンでビルド・ベンチを並行して回していたため、各数値の横に `sysctl -n vm.loadavg`（1 分平均）を記す。最終値は 3 回実行の最良値（criterion、`--warm-up-time 1 --measurement-time 4 --sample-size 10`）。`deals/*` は 1 反復 = `sample_deals` で 1,000 配牌（`sample_deals` 冒頭の §2.3 支持集合プローブと `prepare` を含む）。

**実 SAYC ケース（§10.1 で追加）.** 合成した `Interpretation` ではなく、`systems/sayc/sayc.bml` をコンパイルして `bridge_bidding::interpret` した解釈と、そのオークションの実際のビディング尤度（`SampleContext::bidding`、`sequence_log_likelihood`）で重み付けする 3 ケース:

- `deals/sayc/stayman_to_3nt`: 1NT - P - 2♣ - P - 2♥ - P - 3NT - P - P - P
- `deals/sayc/competitive_raise_to_4s`: 1♠ - 2♥ - 2♠ - P - 4♠ - P - P - P
- `deals/sayc/four_seat_competitive`: 1♠ - 2♥ - 2♠ - 3♥ - 4♠ - P - P - P（4 席すべてがコール）

**手順 1（再計測、変更前 = 8933b71）.**

| ケース | single_thread | loadavg |
| --- | --- | --- |
| `deals/uniform` | 4.59 M 配牌/秒 | 9.2 |
| `deals/constraint/1nt_opener` | 2.00 M 配牌/秒 | 9.2 |
| `deals/constraint/four_call_three_seats` | 14.1 K 配牌/秒 | 9.2 |
| `deals/sayc/stayman_to_3nt` | 3.41 K 配牌/秒 | 9.2 |
| `deals/sayc/competitive_raise_to_4s` | 5.07 K 配牌/秒 | 9.2 |
| `deals/sayc/four_seat_competitive` | 4.67 K 配牌/秒 | 6.7 |

`bridge-constraint` の `sampler` ベンチ（loadavg 7.5）: `prepare` フルデッキ balanced 15-17 で 8.08 µs、中盤（26 枚プール・6 枚固定）で 12.6 µs、`sample` 197 ns / 140 ns。旧版 §10.1 が前提にしていた「中盤 ≈ 100 µs/prepare」はフェーズ 2 の性能修正で既に解消済みで、`four_call_three_seats` はこの時点で目標を満たしていた。一方、実 SAYC の 3 ケースは 3.4〜5.1 K 配牌/秒で未達。

**プロファイル（macOS `sample`、`competitive_raise_to_4s` / `stayman_to_3nt`）.** 1 配牌あたりの時間は `propose` が 6〜7 割、`sequence_log_likelihood`（`bridge-bidding`）が 3〜4 割、`log_prob` はほぼ 0（2e2c8d2 のスレッドローカルキャッシュで `propose` の `Sampler` を再利用するため）。`propose` の中身はほぼすべて再 prepare される中間席の `Sampler::prepare`（スート表の列挙、対畳み込み、形ごとの重み、DNF 化、前回の `Sampler` の解放）だった。原因は実解釈の形にある: 1 席に 3〜8 個の代替があり、そのうち多くが `cards` / `eval` の細部だけが違うため §6.4 (c) の粗い要約が一致する（Stayman の東のパス 8 代替はすべて `ANY` に潰れる）。それぞれが毎回、同じプールに対して同じスート表と対畳み込みを作り直していた。

**手順 3 の最適化（すべて厳密性を保つ。出力は `sample_deals` の配牌列まで不変、ただし (i) の併合で成分選択の乱数消費が変わるケースを除く）.**

1. (i) 同一要約の併合（`bridge-sample`）: 再 prepare される席の粗い要約のうち同一のものを 1 成分に併合し重みを合算する。同一成分は `cnt` を共有するので混合分布は不変。併合後の要約が `ANY` 1 つなら、その席は `SeatPlan::Direct` と同じく残りプールから一様に配る（`ANY` の `Sampler` と同じ密度、`prepare` 0 回）。
2. (ii) `Sampler::prepare_many`（`bridge-constraint`）: 同じ `(pool, fixed)` に対する複数制約の prepare で、スート別フィルタも追加特徴も持たない（= 表が制約に依存しない）項の 4 スート表と `(l0,l1)` / `(l2,l3)` 対畳み込みを共有する。ホットパスでは「代替数 × 表構築」が「1 回の表構築 + 対畳み込みの和集合」になる。得られる `Sampler` は単独の `prepare` とビット一致（count・exact・同じ乱数列での標本・`log_prob` を差分テストで確認）。
3. (iii) `Sampler::prepare_many_dnf`: 粗い要約の DNF 化を `ConstraintProposal::prepare` で 1 回だけ行い、毎回の `to_dnf`（形集合の全走査を含む正規化・自明充足不能判定、`propose` の約 1 割）を省く。
4. (iv) `Sampler::prepare` 自体の定数削減: 追加特徴なしの項では形の重みを 1 次元 CDF の 2 回参照に、対畳み込みを密な 11×11 積和（非ゼロ範囲のみ）に。対畳み込みを `PairMap` の平坦な 2 本の `Vec` に格納し、1 回の prepare 呼び出しで作った表を `Arc` 1 組で共有（対ごとの `Box`/`Vec`/`Arc` の確保・解放が `propose` の約 2 割を占めていた）。平凡なスート表（フィルタなし・鍵 = HCP）の第 1 パス（ヒストグラム）を、オナー部分集合（≤ 16）× スポット枚数の二項係数の閉形式に置換（配置パスは同じ列挙順のまま、`build` とビット一致をテスト）。スート表の長さ別疎リストを 1 本のバッファにまとめる。

**最終結果（7e7e89f 相当、3 回の最良値）.**

| ケース | 変更前 | 最終 | loadavg |
| --- | --- | --- | --- |
| `deals/uniform` | 4.59 M/秒 | 4.67 M/秒 | 5.4 |
| `deals/constraint/1nt_opener` | 2.00 M/秒 | 1.93 M/秒 | 5.4 |
| `deals/constraint/four_call_three_seats` | 14.1 K/秒 | **32.8 K/秒** | 6.6 |
| `deals/sayc/stayman_to_3nt` | 3.41 K/秒 | **8.09 K/秒** | 6.4 |
| `deals/sayc/competitive_raise_to_4s` | 5.07 K/秒 | **10.4 K/秒** | 6.6 |
| `deals/sayc/four_seat_competitive` | 4.67 K/秒 | **10.4 K/秒** | 6.6 |

`uniform` と `1nt_opener` は変更の影響を受けない経路で、差は測定誤差（同じ負荷で旧版と交互に測ると 1.79 M/秒 vs 1.77 M/秒）。同じ負荷（loadavg 5〜6.5）で旧版と交互に測った A/B でも `four_call_three_seats` 12.9 K → 28.2 K、SAYC 3 ケース 3.05 / 3.81 / 4.13 K → 7.31 / 9.09 / 8.83 K（途中の版、最終版より 1 コミット前）。

`sampler` ベンチ（最終、loadavg 5.4〜5.7、3 回の最良値）: `prepare` フルデッキ balanced 15-17 で 3.42 µs（変更前 8.08 µs）、中盤 7.44 µs（同 12.6 µs）、`sample` 164 ns / 135 ns（同 197 ns / 140 ns）。

**1 配牌あたりの内訳（`sample_deals` 2,000 配牌 × 5 回の最良値、同じ解釈・同じ乱数列で `propose` + `log_prob` と `sequence_log_likelihood` を別々に計測、loadavg 5.1〜5.8）.**

| ケース | `propose`+`log_prob` 変更前 → 最終 | ビディング尤度 | `sample_deals` 変更前 → 最終 |
| --- | --- | --- | --- |
| `stayman_to_3nt` | 183.7 µs → 29.5 µs | 90〜92 µs | 3.53 K → 8.21 K/秒 |
| `competitive_raise_to_4s` | 136.8 µs → 39.6 µs | 55〜56 µs | 5.13 K → 10.2 K/秒 |
| `four_seat_competitive` | 143.7 µs → 39.7 µs | 56〜59 µs | 5.18 K → 10.2 K/秒 |

目標 10^4 配牌/秒/コア（= 100 µs/配牌）に対し、`four_call_three_seats` と競り合いの 2 ケースは達成。`stayman_to_3nt` は未達だが、残りの 8 割近くは `bridge-bidding` の `sequence_log_likelihood`（1 配牌 ≈ 92 µs、主に `NaturalInference::candidates` / `infer` と `summary_satisfiable` の `ShapeSet::min_hcp` / `max_hcp` 全走査）で、提案側をゼロにしても ≈ 10.8 K/秒が上限。これ以上はこのレーンの範囲（`bridge-sample`・`bridge-constraint/src/sampler`）の外で、ビディング尤度側の最適化（`ShapeSet` の HCP 範囲のキャッシュ、`candidates` の確保削減など）を別タスクとして扱う。

**採否.**

- **(a) 無制約席の直接配り**: 採用（実装済み）。今回、再 prepare される席の粗い要約が併合後 `ANY` になる場合にも拡張した（上記 (i)）。
- **(b) 対畳み込みの到達可能シェイプ限定**: 採用（実装済み）。フェーズ 2 の時点で `PairMap` は実行可能なシェイプが使う対だけを遅延構築していた（旧版の「未実装」は誤り）。今回はさらに、同じプールに対する複数の項・代替のあいだで対畳み込みとスート表を共有する（上記 (ii)、`Sampler::prepare_many`）。
- **(c) 粗い提案**: 採用（実装済み、変更なし）。再 prepare される中間席は `cards` / `eval` を落とした要約で提案する。実解釈では要約が高い確率で一致・`ANY` 化するので、(i)(ii) と組み合わせたときに最も効く。ESS の代償は従来どおり。
- **`(代替, プール)` 単位のキャッシュ**: 1 回の `propose` の中では (ii) で実質的に実現（同じプールに対する全代替が表と畳み込みを共有、同一要約は (i) で 1 回だけ）。`propose` と `log_prob` のあいだは 2e2c8d2 のスレッドローカルキャッシュで再利用。配牌をまたぐキャッシュはプールがほぼ繰り返さないので採らない（§6.4 の判断どおり）。

`propose_with_log_prob`（未決事項 4）: 足さないで確定。`log_prob` は `propose` が作った `Sampler` をスレッドローカルキャッシュ経由で再利用するので、プロファイル上 `log_prob` は 1 配牌あたりの時間のほぼ 0（`competitive_raise_to_4s` で 1% 未満）。統合しても測定可能な改善がなく、1 段 API（`propose` / `log_prob`）を保つ。

**ESS についての注意（5.3 の範囲、未解決）.** 上記の実 SAYC 3 ケースを `sequence_log_likelihood` で重み付けすると、2,000 配牌での ESS 比は 0.7% / 2.6% / 0.2%（変更前後で同水準、`competitive_raise_to_4s` と `four_seat_competitive` は完全一致）で、フェーズ 5 の完了条件（ESS ≥ 0.5n）から大きく外れる。スループットの問題ではなく、提案分布（`interpret` の解釈 + ε 混合 + (c) の粗い要約）と目標（ポリシーの `call_distribution` に基づく尤度）の乖離であり、5.3 の ESS スイート側で扱う。（→ §10.2）

### 10.2 フェーズ 5.3 の ESS スイートと原因分析（2026-09-26）

**スイート.** `tests/ess_suite.rs`（`#[ignore]`、release で約 20 秒）。50 オークションそれぞれを SAYC（`systems/sayc/sayc.bml`）で `interpret` し、`SampleContext::bidding = Some`（`sequence_log_likelihood`）で `n = 1000` 配牌を `UniformProposal` と `ConstraintProposal` の両方で引く。既知カードはオープニングリーダー（ディクレアラーの LHO）の手（フェーズ 6 のリード問題と同じ状況）、残り 3 席をサンプリングする。`ESS_SUITE_KNOWN=none` で 4 席すべてをサンプリング（参考値）、`ESS_SUITE_CASES=a..b` で分割実行。結果は `target/ess_report.json`。

- 生成 25: 固定 seed の一様乱数配牌を `bridge_bidding::replay`（4 席 SAYC、ディーラーは N/E/S/W の順、ノンバル）で競り終わりまで競らせる。`replay` には尤度の方策と同じナチュラル補完（`natural = Some(table.natural)`）を渡す。`natural = None` だと `NoCandidate` の穴がパスになり、そのパスは尤度側の方策（ナチュラル補完あり）では実際の手でも ε 床（`p ≈ 3e-5`）になるため、オークションが構成上オフポリシーになる（実測: 実際の手の席尤度が `ln L_s ≈ −10`）。一方ナチュラル補完ありの `replay` は、両陣営または片方のパートナーシップが 1 巡ごとに 1 段ずつ上げ続けて 7 のレベルに達する暴走（例 `1H 1S P 2H X 2NT P 3H X 3NT … 7NT`）をしばしば起こすので、スラムレベル（6・7）の契約は除外した（`bridge-bidding` のナチュラル推定の問題、下記）。パスアウトも除外。
- コーパス 25: `corpus/data/pbn` の PBN（`bridge-format` の `parse_lenient`）から、完全な配牌と完結したオークション（パスアウト以外）を持つボードをファイル順に並べ、等間隔に 25 件。人間が自分たちのシステムで競ったもので、SAYC での解釈は近似（オフシステム）。コーパスが無い環境ではこの半分を飛ばし、その旨を stderr と JSON に書き、基準は生成分だけで判定する。

**結果（`n = 1000`、ESS/n の中央値）.**

| | 全 50 | 生成 25 | コーパス 25 |
| --- | --- | --- | --- |
| `UniformProposal` | 0.0026 | | |
| `ConstraintProposal` | **0.0282** | 0.0468 | 0.0168 |
| 参考: 既知カードなし（4 席サンプリング） | 0.0085 | 0.0116 | 0.0055 |

**完了条件（中央値 ≥ 0.5n）は未達.** 0.5 を超えたのは `1NT P 3NT P P P`（0.51）だけで、0.25 を超えるのも 6 件。`ConstraintProposal` は一様提案の約 11 倍だが、基準との差は 1 桁以上ある。

**原因の内訳.** スイートは `ConstraintProposal` の各配牌・各非ビューア席のコールについて、尤度そのものの方策（`call_distribution`）でそのコールの確率 `p` を求め、`p ≥ 0.9`（on）、`0.01 ≤ p < 0.9`（shared: 同優先度の同点や複数のナチュラル候補で質量を分け合う）、`p < 0.01` でも手がそのコール自身の非 `Fallback` 解釈代替を満たす（off_inside_node: 解釈が方策と食い違う）、満たさない（off_outside_node: 提案が `Fallback` 枝・(c) の粗い要約・残り席から引いた手）に分類して JSON に出す。生成 25 件の集計:

| コールの解決 | on | shared | off_inside_node | off_outside_node |
| --- | --- | --- | --- | --- |
| Exact | 53% | 25% | 2% | 20% |
| Partial | 43% | 48% | 0% | 9% |
| Natural | 30% | 32% | 13% | 24% |

コーパスでは Natural の off_inside_node が 23%、さらに実際の配牌自身が 274 コール中 111 コール（41%）でオフポリシー（`true_deal_off_policy_calls`、生成は 0%）。1 配牌に非ビューアのコールが 6〜10 個あるので、全コールがオンになる配牌の割合（`constraint_all_on_policy`）は生成でも中央値 8% 程度にとどまり、ESS はほぼこれで決まる。席ごとに見ると（席 `s` のコールだけの尤度 `L_s` の ESS）、提案順で 1 番目 0.42、2 番目 0.40、最後の残り席 0.19（中央値）で、残り席が最悪の席になるのは 50 件中 26 件。残り席は他の 3 席の残りをそのまま受け取り、代替のどれか（ε 混合の `ANY` を含む）を満たすかしか検査しないので、実質的に一様提案と同じ質になる。

具体例（`Pass 1C 2S Pass Pass 3S …`、南）: 解釈の `1C` 代替は「12–21 HCP、3+♣」だが、方策はその手の 24% を `1H`、22% を `1NT` で開く（より優先度の高い兄弟ノードを除外していない）。同じ南の `3S` はナチュラル推定で「25+ HCP のキュービッド」と解釈されるのに、方策はそうした手では `3C` や `7C` を選ぶ。北の `Pass`（ナチュラル）は「0–5 HCP」と解釈されるが、方策はもっと広い手でパスする。

**`bridge-sample` 側で試したこと（いずれも厳密性は保つ = `log_prob` は実際の提案密度。採用せず）.** 同じ 50 件、ESS/n 中央値（全体 / 生成 / コーパス）:

| 変更 | 全体 | 生成 | コーパス | 備考 |
| --- | --- | --- | --- | --- |
| なし（現行） | 0.028 | 0.047 | 0.017 | |
| 成分重みを方策尤度の平均で置換（成分あたり 32 手のパイロット） | 0.017 | 0.018 | 0.009 | 入れ子の成分（`ANY` ⊃ 主代替）で同じオンポリシー質量が二重に数えられる |
| 成分重みをパイロット上の EM で最適化（防御的混合 5%） | 0.029 | 0.048 | 0.017 | 成分そのものが方策と合わないので重みでは直らない |
| `Fallback` を含む代替の重み ×0.1（ε 混合の `ANY` の取り分を方策の ε 床に寄せる） | 0.059 | 0.086 | 0.010 | 生成は改善、オフシステムのコーパスは悪化 |
| 同 ×0.01 / ×0.001 | 0.045 / 0.043 | 0.087 / 0.075 | 0.008 / 0.008 | |
| (c) の粗い要約をやめ中間席も細かい代替で提案 | 0.033 | 0.056 | 0.017 | 5.4 の性能を失う |
| ×0.05 + 粗い要約なし | 0.065 | 0.075 | 0.009 | 最良だが目標の 1/8 |
| 席ごとに方策尤度 `L_s` で受理（`r = L_s ≤ 1`、配牌ごとやり直し） | 0.000 | 0.017 | 0.000 | 受理率 < 1%、多くのスロットが空 |
| 解釈を満たさない席を確率 δ = 0.1 でだけ受理 | 0.052 | 0.104 | 0.015 | 受理率 7%。δ = 0.01 で 0.015 |
| 席順を非 `Fallback` 質量で決める | 0.024 | 0.045 | 0.014 | |

どれも中央値を最大 2 倍程度にしか上げず（しかもコーパス側を悪化させるか、5.4 のスループットを失う）、0.5 には届かない。受理・棄却型は ESS を受理率に付け替えるだけで、有効サンプル 1 個あたりの計算量は改善しない。

**結論.** 主因は `bridge-sample` ではなく、`interpret` の解釈と `call_distribution` の方策の不整合（`bridge-bidding`）:

1. Exact / Partial 解釈がノード自身の制約だけを使い、方策がその手でより優先度の高い兄弟を選ぶ領域を除いていない（上の `1C` の例）。方策と同じ `priority` 順で「より上位の兄弟の否定」を AND すれば、解釈の成分が方策のオン領域とほぼ一致するはず。
2. ナチュラル推定の解釈（例: パス = 0–5 HCP、`3S` = 25+ のキュー）がナチュラル方策の実際の選択と食い違う（生成で Natural の off_inside_node 13%）。
3. ナチュラル補完ありの `replay` の暴走エスカレーション（上記）。
4. 同優先度の同点・複数のナチュラル候補による質量の分割（shared 25〜48%）は尤度の性質そのもので、提案側では再現できない分の重みのばらつきとして残る。

`bridge-sample` 側に残る改善余地（`Fallback` の取り分、(c)、残り席）は上表のとおり合わせても 2 倍前後で、1〜2 を直さない限り完了条件には届かない。1〜3 は `bridge-bidding` の別タスクとして扱い、直したらこのスイート（`target/ess_report.json` の `policy_breakdown_*` と `constraint_all_on_policy`）で再評価する。そのときに `Fallback` の取り分（方策の ε 床への較正）を改めて評価する。

**フェーズ 4 での解決（2026-09-27）.** 上の結論 1〜3 は `bridge-bidding` 側で、方策を D18（`p(c|h) = (1 − ε)·[(1 − δ)·S + δ·M] + ε/n`、`choose_bid` の決定的な選択に ε 床を足したもの。旧来の `priority / τ` のソフトマックスは退役）に置き換え、解釈をその方策の較正された鏡像（D19: 片 `X_c`・`N_sys`・`Y_c`・`N_nat`・`ANY` の重みが方策の密度そのもの）にすることで扱った。`bridge-sample` 側では §6.4 (c) の格子要約、(d) の畳み込み、§6.5 の残差棄却と試行予算、§6.1 の質量順の切り詰めを入れた。

プロトタイプの比較（50 オークション、ESS/n の中央値、全体 / 生成 / コーパス、計画 15-phase4-plan.md から転記）:

| 版 | ESS/n | 備考 |
| --- | --- | --- |
| 基準（フェーズ 3、τ = 1 のソフトマックス + 旧解釈） | 0.036 / 0.040 / 0.030 | |
| A（コンパイル時の排他 + rank 方策 τ = 0.2、ε の較正なし） | 0.086 / 0.127 / 0.084 | |
| C（実行時の排他 + τ = 0.1・tie_gap 5・ε 0.003・レベル下限） | 0.301 / 0.298 / 0.350 | B の提案側の変更を足すと 0.327、さらに残差棄却で 0.608（試行あたり 0.287） |
| B（方策鏡像、τ = 1） | 0.426 / 0.503 / 0.377 | 残差棄却を足すと 0.906（試行あたり 0.371、受理率の中央値 0.52） |

教訓: 排他（締まり）は必要だがそれだけでは足りない。片の重みを方策の密度に較正して初めて、件数比例の成分抽選が `q ∝ L` を与える。

**最終値（このレーン、B の最新のマージと畳み込みのフォールバックの後、固定ケースの再生成後、評価用固定ケース、n = 1000、release）.** ケースは `tests/data/ess_cases.txt`（生成 25 は deal seed `0x5A7C_0005_0003` の `replay`、コーパス 25 は D20 の評価分割 = `corpus_auctions` の列挙で奇数番目から等間隔に）。最初の版は S の自然なレベル下限とフェーズ 4.6 のナチュラルの調整が B から入る前に作ったもので、生成 25 件中 16 件に、配牌の実際の手が現行の方策では `p < 0.01` でしか選ばないコールが残っていた（生成分は「方策どおり」の集合のはずなので、ESS を過小に見積もる）。`ESS_SUITE_WRITE_FIXTURE=1` で再生成し（生成 21 行が変わり、コーパス分は不変）、今は生成 25 件すべてで外れたコールは 0。`ess_fixture_generated_cases_are_on_policy`（非 ignore）が、SAYC や方策の変更で固定ケースが古くなったことを検出する。D の SAYC 変更が入った統合時にこのテストが落ちたら、もう一度固定し直す。最終ヘッド（wip/p4int 4e0f30d のマージ後）で実際に落ちた（13 / 25 件）ので、2026-10-02 に固定し直した（下の「最終ヘッドでの再測定」）。

| 提案 | ESS/n（全体 / 生成 / コーパス） | 試行あたり ESS（パイロット込み） | 受理率（中央値 / 最小） | 予算切れ | ≥ 0.5 の件数 |
| --- | --- | --- | --- | --- | --- |
| `UniformProposal` | 0.0030 / 0.0030 / 0.0031 | 0.0030 | 1 / 1 | 0 | 0 |
| 残差棄却なし（既定） | **0.5702 / 0.5695 / 0.5709** | **0.5702** | 1.000 / 0.957 | 0 | 29 |
| 残差棄却あり、下限 0.5 | 0.8420 / 0.8934 / 0.8201 | 0.5170 | 0.628 / 0.430 | 0 | 37 |
| 残差棄却あり、下限 0.125（リード助言） | 0.9157 / 0.9399 / 0.8979 | 0.5170 | 0.628 / 0.110 | 0 | 43 |

完了条件（ESS/n ≥ 0.5 を全体と生成で、コーパス ≥ 0.4、予算切れ ≤ 2 件、試行あたり ESS ≥ 0.35）は既定の提案（残差棄却なし）で満たす。単一スレッド（`ESS_SUITE_THREADS=1`）で、残差棄却あり（下限 0.5）は棄却なしに対しサンプリング時間 1.68〜1.75 倍、有効サンプルあたりの時間 1.20〜1.25 倍（3 回、loadavg 15.4〜16.1）、下限 0.125 では 5.0〜5.2 倍 / 3.1〜3.2 倍（2 回、loadavg 6.9〜7.3）。スイート全体は単一スレッドで約 3 秒。チューニングモードでは棄却なしが 0.5443（生成 0.3910、コーパス 0.6277）、下限 0.5 が 0.7492（生成 0.7504、コーパス 0.7284）、1.56〜1.61 倍 / 1.16〜1.21 倍（loadavg 14.9〜15.4）。チューニング集合の生成分は棄却なしで 0.39 と評価用より低く、生成ケースの ESS は 25 件の選び方でかなり揺れる。B のマージ前の値（棄却なし 0.4247 / 0.5262 / 0.4178）から上がったのは、主に B の方策と鏡像の変更と固定ケースの再生成による。チューニングモード（`ESS_SUITE_MODE=tune`: 別の deal seed と、コーパスのチューニング分割）での下限の掃引と既定の決定は §6.5 に記した。

**スループット（`deals/sayc/*/single_thread`、1 反復 = 1,000 配牌、3 回の最良値）.** 鏡像の解釈と `system_players` の方策尤度（B の `AuctionPolicy` で 1 配牌 0.1〜0.2 μs）で:

| ケース | 既定（残差棄却なし） | 残差棄却あり（下限 0.5、`single_thread_residual`） | loadavg |
| --- | --- | --- | --- |
| `stayman_to_3nt` | 28.4 K/秒（(d) の前は 7.1 K/秒） | 19.1 K/秒 | 7.6〜12.0 |
| `competitive_raise_to_4s` | 30.0 K/秒（同 9.6 K/秒） | 19.0 K/秒 | 同 |
| `four_seat_competitive` | 28.4 K/秒（同 14.2 K/秒） | 13.8 K/秒 | 同 |

目標 10^4 配牌/秒/コアは 3 ケースとも、棄却の有無どちらでも達成。ビディング尤度は B の `AuctionPolicy` で配牌あたりほぼ 0 になり、残りは再 prepare される中間席の `Sampler::prepare` で、(d) がその大半を消した。残差棄却ありの数値は棄却分とパイロット（128 回）を含む。負荷が高い時間帯の計測なので、統合時に静かなマシンで取り直すこと。

**最終ヘッドでの再測定（2026-10-02、wip/p5final 8abe651）.** フェーズ 4 の最終ヘッド（wip/p4int 4e0f30d）をマージしたあと、固定ケースが古くなった（生成 25 件中 13 件がオフポリシーで、`ess_fixture_generated_cases_are_on_policy` が落ちた）。`ESS_SUITE_WRITE_FIXTURE=1` で固定し直し（50 行中 13 行が変わった: 生成の 11 行はオークションが変わり、2 行は別の局に置き換わった。コーパス 25 行は不変）、上の 2026-09-29 の値を取り直した。`bridge-sample` と `bridge-constraint` のコードは 2a536e7 から変わっていない。n = 1000、評価用固定ケース、release。ESS の値は seed が固定で負荷に依らず、4 回の実行で一致した。

| 提案 | ESS/n（全体 / 生成 / コーパス） | 試行あたり ESS（パイロット込み） | 受理率（中央値 / 最小） | 予算切れ | ≥ 0.5 の件数 |
| --- | --- | --- | --- | --- | --- |
| `UniformProposal` | 0.0060 / 0.0040 / 0.0086 | 0.0060 | 1 / 1 | 0 | 0 |
| 残差棄却なし（既定） | **0.5471 / 0.7113 / 0.4608** | **0.5471** | 1.000 / 1.0000 | 0 | 27 |
| 残差棄却あり、下限 0.5 | 0.7860 / 0.8513 / 0.7650 | 0.4627 | 0.618 / 0.4314 | 0 | 34 |
| 残差棄却あり、下限 0.125（リード助言） | 0.8616 / 0.9356 / 0.7915 | 0.4627 | 0.618 / 0.1014 | 0 | 42 |

完了条件（全体と生成で ESS/n ≥ 0.5、コーパス ≥ 0.4、予算切れ ≤ 2 件、試行あたり ESS ≥ 0.35）は既定の提案で満たす。生成が 0.5695 から 0.7113 に上がり、コーパスが 0.5709 から 0.4608 に下がったのは、固定ケースの入れ替えと、フェーズ 4 の最終ヘッドの `bridge-system`（ナチュラル推定）と `bridge-bidding`（`choose_bid`・方策）の変更による。コーパスの 25 行は同じなので、コーパス側の変化はコードの変更だけで起きている。調整用の集合（`ESS_SUITE_MODE=tune`、単一スレッド）では棄却なしが 0.5044（生成 0.5142、コーパス 0.3513）、棄却ありが 0.7414 で、コーパス側の余裕は 25 件の選び方に依る。方策の内訳（`off_inside_node`: 解釈が方策と食い違う）は、生成で 0 件、コーパスで 134 件（Exact の 0.1%）まで下がった（フェーズ 5.3 では Exact 2%、Natural 13%）。時間比（棄却あり / なし、サンプリング時間 / 有効サンプルあたり）: 単一スレッド 1.69 / 1.26 倍（loadavg 3.98）、`Threads::Auto` 1.85〜1.89 / 1.38〜1.42 倍（3.98、2 回）、下限 0.125 は単一スレッド 3.89 / 2.47 倍（loadavg 2.64）。

**スループット（再測定、`deals/*/single_thread`、criterion、1 反復 = 1,000 配牌、3 回の中央値と最良値）.** 開始時の loadavg は 3.98 / 3.69 / 3.72（終了時 6.30 / 3.72 / 4.31、自身の 1 スレッドを含む）。前の列は、sayc の 3 ケースが 2026-09-29 の最良値（loadavg 7.6〜12.0 の時間帯）、残りが §10.1 の最終値（loadavg 5.4〜6.6）。

| ケース | 既定（残差棄却なし） | 残差棄却あり（下限 0.5） | 前の値（既定 / 棄却あり） |
| --- | --- | --- | --- |
| `sayc/stayman_to_3nt` | 34.3 K/秒（最良 35.1 K） | 30.5 K/秒（30.8 K） | 28.4 K / 19.1 K |
| `sayc/competitive_raise_to_4s` | 31.8 K/秒（32.2 K。1 回目は負荷の山で 20.3 K） | 27.6 K/秒（28.4 K） | 30.0 K / 19.0 K |
| `sayc/four_seat_competitive` | 30.4 K/秒（30.8 K） | 14.6 K/秒（14.8 K） | 28.4 K / 13.8 K |
| `constraint/four_call_three_seats` | 29.4 K/秒（33.1 K） | | 32.8 K（§10.1） |
| `constraint/1nt_opener` | 2.00 M/秒（2.02 M） | | 1.93 M（§10.1） |
| `uniform` | 4.85 M/秒（5.02 M） | | 4.67 M（§10.1） |

目標 10^4 配牌/秒/コアは、棄却の有無どちらでも全ケースで達成（最小は `four_seat_competitive` の残差棄却あり 14.6 K）。Stayman 3NT を含む。
