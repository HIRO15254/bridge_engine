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
    pub max_attempt_factor: u32,        // 50。総試行数 ≤ n × 50 で打ち切り
    pub threads: Threads,               // Auto | Single。結果は同一
}
pub enum Threads { Auto, Single }

pub struct SampleReport {
    pub requested: usize, pub produced: usize, pub attempts: u64,
    pub acceptance_rate: f64,           // produced / attempts
    pub ess: f64, pub ess_ratio: f64,   // ess / requested
    pub log_weight_max: f64, pub elapsed: Duration,
    pub warnings: Vec<SampleWarning>,
}
pub enum SampleWarning {
    CustomConstraint { seat: Seat },            // 棄却法に退化した代替がある
    LowEss { ess: f64, requested: usize },      // ess < 0.5 × requested（フェーズ 5 の完了条件と同じ閾値）
    Truncated { produced: usize },              // 総試行上限で打ち切り
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
3. チャンク境界で `produced ≥ n` または `attempts ≥ n × max_attempt_factor` なら停止する。判定がチャンク境界にあるので、結果はスレッド数に依らない。
4. スロット順に最初の `n` 件を採用する。
5. §3.2 の式で `ess` を計算し、`SampleReport` を組み立てる。`tracing::info!(requested, produced, attempts, acceptance_rate, ess, ess_ratio, log_weight_max, elapsed_us)` を **常時 INFO** で出す（仕様 §9: ESS 報告は性能問題の一次診断情報）。

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
| `ln P(auction | d)` | `sequence_log_likelihood(table, d, auction, bid_ctx)`（07-bidding.md §6.2、2〜5 μs / 配牌） | `Σ_s ln interpretation.likelihood(s, h_s)`（集合所属質量） |

`Some` のときは解釈の制約は提案分布の構築にだけ使われ、尤度は `choose_bid` の確率的評価（`call_distribution` の softmax、`priority / τ`、ε 床）から得る。生成器が決定的でも `priority` からソフトマックスで確率化されているので、支持集合は全合法コールに広がる。`None` のときは ε-混合の防御枝が `likelihood` に ε の質量を残し、制約外の手にも正の尤度を与える。

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

1. **席ごとの代替**: `A_s = interpretation.seats[s] ⊗ play_soft[s]`（直積、重みは積、重み上位 K = 8 に切り詰め）。`play_soft` が `None` なら `interpretation.seats[s]` そのもの。各代替を `play_constraints[s]` と `and` する。`needed(s) == 0` の席（自分、ダミー）は対象外。
2. **Sampler の準備**: 代替 `i` について `S_{s,i} = Sampler::prepare(&C_{s,i}, pool, known[s], opts)`（`pool = known.pool()`）。`count() == 0` の代替は落とす。`is_exact() == false`（`Custom` / `residual` / スロット超過を含む）なら `SampleWarning::CustomConstraint { seat: s }`。その代替は `Sampler` 内部で棄却法（`max_tries = 256`）になる。
3. **制約の厳しさ**: `mass_s = Σ_i w_i · S_{s,i}.count()`。席順 `σ` を `ln mass_s` 昇順（同点は席 index）に決める。制約が厳しい席から順に引く。`σ` の最後の席は残りを受け取る。全代替が落ちた席があれば `EmptySupport { seat }` を報告し、支持集合が空であることを `prepared` に記録する。
4. **キャッシュ**: 席 `σ_1` の `Sampler` 群と成分重み `v_i = w_i / Σ_{i: cnt > 0} w_i` を保持する（`σ_1` のプールは固定なので再利用できる）。席 `σ_2` 以降はプールが縮むので `propose` ごとに準備し直す。
5. **無制約の検出**: 代替 `C` が `ANY`（要約が `shapes = ALL`、`hcp = 0..=37`、`cards` / `eval` 空）なら `Sampler` を作らず、組合せ的な直接配りにマークする（§6.4 (a)）。ε-混合の防御枝の積（全コール Fallback の組合せ）はこれに該当する。

### 6.2 `propose(rng)`

`|σ| = m` とする。`k = 1..=m−1` について:

1. `P_k` = `pool` からそれまでの席の抽選分を除いたもの。`s = σ_k`。
2. `k == 1` ならキャッシュを使う。それ以外は各代替について `S^{(k)}_{s,i} = Sampler::prepare(&C_{s,i}, P_k, known[s], opts)` を呼ぶ。`count() == 0` の代替は落とし、残りの `v_i` を正規化する。残りがなければ `None`。
3. 成分 `i` を `v_i` 比例で選び、`h_s = S^{(k)}_{s,i}.sample(rng)?.hand`（棄却法の試行切れなら `None`）。
4. `ln π_k = ln Σ_{i'} v_{i'} · exp(S^{(k)}_{s,i'}.log_prob(h_s))`（重なる成分をすべて数える。§6.3）。
5. `P_{k+1} = P_k − h_s`。

最後の席 `σ_m`: `h = P_m ∪ known[σ_m]`。`play_constraints[σ_m].satisfies(h)` かつ `∃ i: C_{σ_m,i}.satisfies(h)` でなければ `None`。合格なら `Deal::new(hands)`（`debug_assert!` で成功を確認）。再試行は `sample_deals` の側で、そのスロット自身の乱数列から最大 `max_attempts_per_sample` 回行う。

### 6.3 `log_prob(deal)`

`deal` から同じ `σ` とプール列を再現し、`k = 1..=m−1` について `π_k(h_{σ_k}) = Σ_{i: h ∈ C_{σ_k,i}} v_i · exp(S^{(k)}_{σ_k,i}.log_prob(h))` を計算し、`ln π = Σ_k ln π_k` とする。項レベルに展開すると `π_k(h) = Σ_{(i,l): h ∈ a_{i,l}} u_{i,l} / cnt_{i,l}` であり、**重なる DNF 項・重なる代替をすべて足す**。混合密度は `h` を生成し得た全成分を数えなければ正しくない。最後の席は残りで決まるので対数領域で 0 を加える。いずれかの `π_k = 0`、または最後の席の検査に落ちる `deal` は −∞。

`log_prob` は実際の提案分布に対して厳密であり、これが ESS を正直な数値にする。`propose` が準備した席 2 以降の `S^{(k)}` は `propose` の中で捨てられ、`log_prob` が再準備するので、配牌ごとの `prepare` 回数は §6.4 の見積りの 2 倍になる。これを避けるトレイト拡張（`propose_with_log_prob`）は §2.1 の未決。

### 6.4 性能リスクと対策（計画 §8.3、R4）

| 数値 | 出典 |
| --- | --- |
| `Sampler::prepare`: フルデッキ 20〜60 μs、列挙が要るスート 1 つにつき +65 μs、未知 26 枚で 3〜10 μs | 05-constraint.md |
| `Sampler::sample`: 0.3〜0.5 μs | 同上 |
| 配牌ごとに席 2〜3 で ≈ 5〜12 回の `prepare`（各 20〜40 μs） | 計画 §8.3 |
| 目標 10^4 配牌 / 秒 / コア = 100 μs / 配牌 | 仕様 §9 |

このままでは席 2〜3 の `prepare` だけで予算を超える。≈ 12 回の `prepare` を見込むなら 1 回あたり ≲ 5 μs に収める必要がある。対策は 3 つで、フェーズ 5.4 のベンチで測って選ぶ。

| 対策 | 内容 | 効果 |
| --- | --- | --- |
| (a) 無制約席の直接配り | 相手がパスのみ等で代替が `ANY` の席は `Sampler` を作らず `C(|P_k|, needed)` から一様に引く。`log_prob = −ln C(|P_k|, needed)` | 対象席の `prepare` が 0 回 |
| (b) 対畳み込みの到達可能シェイプ限定 | `(l0,l1)`・`(l2,l3)` 対の疎畳み込みを、制約が許すシェイプが使う対（各 ≤ 105）に限定する（05-constraint.md、設計済み） | `prepare` の定数を下げる |
| (c) 粗い提案 | 席 2 以降を「シェイプ + HCP のみ」の粗い項（`cards` / `eval` を外した要約 `Atom`）で提案し、細部は `L` の重みで補正する | `prepare` が高速パスに乗る。ESS は下がるが有限 |

`prepare` の結果を `(代替 id, P_k)` でキャッシュする案は、プールがほぼ繰り返さないため効果が薄い。`prepare` 自体が本質的に安いこと（スート単位の DP、列挙でない）が前提になる。

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
| `constraint_proposal.rs` | `ConstraintProposal { max_retries: 16 }`、§6（prepare / propose / log_prob、席順、残り席の検査） |
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
| `uniform_log_prob_constant` | unit | 全提案で `log_prob` が等しく、`Σ exp(log_prob)` が全列挙で 1 | |
| `ess_formula` | unit | 手計算の小例（等重み n 個 → ESS = n、1 個だけ重い → ESS ≈ 1） | |
| `normalized_weights_sum_to_one` | unit | | 1 ± 1e-12 |
| `empty_support_early_return` | unit | 矛盾する `play_constraints` | `Err(SampleError::EmptySupport)`、試行 0 回 |
| `seat_fallback_warns` | unit | ある席の解釈代替がすべて既知カードと矛盾 | `SampleWarning::EmptySupport { seat }`、その席は `ANY` で配られ `produced == n` |
| `uniform_vs_constraint_ess` | `tests/ess.rs`、`#[ignore]` | 50 オークション × n = 1000 | `ConstraintProposal` の ESS 中央値 ≥ 0.5n。`Uniform` は比較用に報告 |
| `deals_bench` | `benches/deals.rs` | criterion、制約付き完全配牌 | ≥ 10^4 配牌 / 秒 / コア |

実装順（計画 §12）: 2.8 骨格（`UniformProposal`、`WeightedDeal`、ESS、`SampleReport`、`rng_for`、`parallel` + 決定性テスト。`bridge-core` と `KnownCards` だけで書ける）→ 5.2 `ConstraintProposal`（`log_prob` χ²）→ 5.3 `sample_deals` + ビディング尤度 + tracing（ESS スイート）→ 5.4 ベンチ + プロファイル（§6.4 の (a)(b)(c) を選ぶ）。

---

## 10. 未決事項

| # | 項目 | 現在の仮置き |
| --- | --- | --- |
| 1 | §6.4 の (a)(b)(c) のどれを採るか | フェーズ 5.4 のベンチで決定 |
| 2 | `play_soft` を L5 側で `interpretation.seats` に事前結合するか | L4 で `⊗` する（L5 は L3 に依存しないため） |
| 3 | `Threads::Auto` で専用プールを作るか | rayon のグローバルプール |
| 4 | `PreparedProposal` に `propose_with_log_prob` を足すか（§2.1、§6.3） | 足さない。ベンチで決める |
| 5 | `bidding` が `Some` のとき `interpretation.per_call.len()` と `auction.len()` の不一致を `SampleError::Prepare` にするか | 検査する（最小限） |

`LowEss` の閾値は `ess < 0.5 × requested` で確定（フェーズ 5 の完了条件と同じ）。
