# 05. `bridge-constraint` (L1): 制約言語と厳密サンプラー

本文書は `HandConstraint` の構造、DNF 正規化 (交差・排他的連鎖による否定・爆発対策)、充足不能判定、そして本クレートの核心である棄却なしの厳密一様サンプラー `Sampler` を確定する。長さ情報は `ShapeSet` に一本化し (D1)、確定カードは制約を変換せずサンプラー側で合成し (D3)、個数と重みは `u64` で扱う (D13)。`KnownCards` も本クレートに置く。依存は `bridge-core`, `bridge-eval`, `rand_core` 0.10, `tracing`, `thiserror`, `serde` (optional)。dev: `criterion`, `proptest`, `rand_xoshiro`。

## 1. 責務と四性質

仕様 §4 の四性質を API に対応付ける。

| 性質 | API | 用途 |
| --- | --- | --- |
| 合成可能 | `and` / `or` / `not`、`HandConstraint::{And, Or, Not}` | システム定義、プレイ制約、利用者の絞り込み |
| 判定可能 | `satisfies(&self, Hand) -> bool` (30〜80 ns) | プロパティテスト、フィルタ、棄却 |
| サンプル可能 | `Sampler::prepare` → `sample` (0.3〜0.5 μs/手)、互換 API `HandConstraint::sample` | 配牌生成 |
| 要約可能 | `hcp_range()` / `shapes()` / `suit_len(Suit)` (クラスの和は `shapes().classes()`) | 枝刈り、UI 表示 |

制約は常に **元の 13 枚の手** に対して定義する。プレイ途中で一部のカードが確定していても、制約自体は書き換えない (D3)。

## 2. 型定義

### 2.1 `Atom` とリテラル (`atom.rs`)

```rust
/// リテラルの連言。全フィールドは元の 13 枚に対するリテラル。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Atom {
    pub shapes: ShapeSet,                 // 長さ情報の唯一の真実 (D1)
    pub hcp: RangeInclusive<u8>,          // ⊆ 0..=37
    pub cards: Vec<CardRequirement>,      // 正規形: mask でソート、同一 mask はマージ
    pub eval: Vec<EvalRequirement>,       // 正規形: metric でソート、同一 metric はマージ
}

impl Atom {
    pub const ANY: Atom;                                       // shapes = ALL, hcp = 0..=37, cards/eval 空
    pub fn satisfies(&self, h: Hand) -> bool;
    pub fn intersect(&self, other: &Atom) -> Atom;             // §4.1
    pub fn negate(&self) -> Vec<Atom>;                         // §4.2、互いに素
    pub fn is_trivially_unsat(&self) -> bool;                  // §5
    pub fn normalize(&mut self);                               // ソート + マージ + クランプ
    // 要約
    pub fn hcp_range(&self) -> RangeInclusive<u8>;
    pub fn suit_len(&self, s: Suit) -> RangeInclusive<u8>;     // shapes.suit_len(s) の射影 (空なら 0..=0 ではなく 1..=0)
    // ビルダ (shapes ∩= は self.shapes = self.shapes.intersect(..) で直接書く)
    pub fn with_suit_len(self, s: Suit, r: RangeInclusive<u8>) -> Atom;   // shapes ∩= from_suit_len
    pub fn with_hcp(self, r: RangeInclusive<u8>) -> Atom;
    pub fn with_cards(self, c: CardRequirement) -> Atom;
    pub fn with_eval(self, e: EvalRequirement) -> Atom;
}

/// popcount(hand ∩ mask) ∈ count。
/// 「♦に A か K」= mask {♦A, ♦K}, 1..=2。「♠A なし」= 0..=0。「エース 2+」= 全 A, 2..=4。
/// 「♠のトップ 3 のうち 2 枚」= mask = ♠AKQ, 2..=3。
/// 「r より上の札が n 枚」(4th best 用) = mask = スート u の rank > r, count n..=n (in_suit で作る)。
#[derive(Clone, PartialEq, Eq, Hash, Debug)]          // RangeInclusive を持つので Clone のみ (Copy ではない)
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CardRequirement { pub mask: Hand, pub count: RangeInclusive<u8> }

impl CardRequirement {
    pub fn in_suit(s: Suit, ranks: Holding, count: RangeInclusive<u8>) -> CardRequirement;
    /// mask が 1 スートに収まるなら Some(suit) → サンプラーでスート別の厳密フィルタ
    pub fn single_suit(&self) -> Option<Suit>;
    pub fn holds(&self, h: Hand) -> bool;   // count.contains(&h.intersect(mask).len())
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]          // 同上、Copy ではない
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EvalRequirement { pub metric: Metric, pub range: RangeInclusive<u8> }
impl EvalRequirement { pub fn holds(&self, h: Hand) -> bool; }

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Metric {
    Controls,                 // 0..=12
    Losers(LtcMethod),        // half 単位 0..=24
    QuickTricks,              // half 単位 0..=16
    DistPoints(DistMethod),   // i8 を 0 でクランプ、上限 40
    TotalPoints(DistMethod),  // 上限 77
    SuitQuality(Suit),        // そのスートの honors5、0..=5
}

impl Metric {
    pub const fn max(self) -> u8;               // 値域の上限 (04-eval.md §6)
    pub fn eval(self, h: Hand) -> u8;           // bridge-eval を呼ぶ
}
```

仕様の `Atom.suit_len` は削除する (D1)。仕様の `cards` の否定フラグは `count` の範囲 (`0..=0`) で表現でき、範囲の補集合として綺麗に否定できるので不要。「加法的な追加特徴になれる指標」(`Controls`, `Losers`, `QuickTricks`) の判定はサンプラー内部の `match` で行い、`Metric` に述語は置かない。

### 2.2 `HandConstraint` (`constraint.rs`)

```rust
/// 名前付きの逃げ道。サンプル不可であることを型で表明する。
#[derive(Clone)]                                   // Debug は手動 ("Custom(name)")
pub struct CustomPred { pub name: String, pub f: Arc<dyn Fn(Hand) -> bool + Send + Sync> }   // name は tracing 用

#[derive(Clone, Debug)]                            // PartialEq は無い (Custom のため)
pub enum HandConstraint {
    Atom(Atom),
    Or(Vec<HandConstraint>),
    And(Vec<HandConstraint>),
    Not(Box<HandConstraint>),
    Custom(CustomPred),
}

impl HandConstraint {
    pub const ANY: HandConstraint;                       // Atom(Atom::ANY)
    pub fn satisfies(&self, h: Hand) -> bool;
    pub fn is_samplable(&self) -> bool;                  // Custom を含まない
    pub fn to_dnf(&self, opts: &DnfOptions) -> Result<Dnf, DnfError>;
    // 要約 (過大近似)
    pub fn hcp_range(&self) -> RangeInclusive<u8>;
    pub fn shapes(&self) -> ShapeSet;
    pub fn suit_len(&self, s: Suit) -> RangeInclusive<u8>;
    pub fn is_satisfiable(&self) -> bool;                // §5 の段階的判定
    // 合成 (多項の And/Or は fold で作る)
    pub fn and(self, other: HandConstraint) -> HandConstraint;   // 入れ子の And を平坦化
    pub fn or(self, other: HandConstraint) -> HandConstraint;    // 入れ子の Or を平坦化
    pub fn not(self) -> HandConstraint;
    /// 仕様互換の便宜 API。呼び出しごとに O(prepare) なので、繰り返し使う場合は Sampler を使う。
    pub fn sample<R: rand_core::Rng + ?Sized>(&self, rng: &mut R, excluded: Hand) -> Option<Hand>;
}
// serde (feature): 手動 impl。Custom を含む値の Serialize はエラー (SystemIR のキャッシュ直列化のため、コンパイラは Custom を生成しない)。
```

要約の意味論: `Or` は各枝の要約の和 (`hcp` は最小〜最大、`shapes` は和集合)、`And` は交差、`Not` は `ANY` の要約 (否定の要約は緩めるしかない)、`Custom` は `ANY`。要約は枝刈りと表示にだけ使い、`satisfies` には使わない。

### 2.3 DNF 型 (`dnf.rs`)

```rust
/// DNF の 1 項 = 厳密な Atom + サンプラーが棄却でしか扱えないもの。
#[derive(Clone, Debug)]
pub struct DnfTerm {
    pub atom: Atom,
    pub custom: Vec<(CustomPred, bool /* negated */)>,
    pub residual: Option<HandConstraint>,   // 爆発対策で退避した子 (satisfies で検査)
}
impl DnfTerm {
    pub fn satisfies(&self, h: Hand) -> bool;   // atom ∧ custom ∧ residual
    pub fn is_exact(&self) -> bool;             // custom.is_empty() && residual.is_none()
}

#[derive(Clone, Debug)]
pub struct Dnf { pub terms: Vec<DnfTerm>, pub truncated: bool }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DnfOptions { pub max_terms: usize /* 256 */, pub on_overflow: Overflow /* Residual */ }   // Default あり
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overflow { Residual, Error }

#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]   // error.rs
pub enum DnfError {
    #[error("DNF would have about {estimated} terms, more than the cap of {max_terms}")]
    TooLarge { estimated: usize, max_terms: usize },
}
```

### 2.4 サンプラー型 (`sampler/`)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]       // Default あり
pub struct SampleOptions {
    pub max_tries: u32,        // 256。residual/custom の棄却上限
    pub extra_features: u8,    // 1。DP 鍵に載せる追加の加法的特徴の数 (0 = K=1 高速パス、1 = K=2)
    pub burn_in: u32,          // 256。α 推定の試行数
    pub allow_rejection: bool, // true。false なら Custom/residual を含む制約は PrepareError::NotSamplable
}

pub struct Sampler { terms: Vec<PreparedTerm>, cum: Vec<u64>, total: u64, pool: Hand, fixed: Hand, exact: bool }
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Sample { pub hand: Hand, pub log_prob: f64, pub tries: u32 }

#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]   // error.rs
pub enum PrepareError {
    #[error("pool and fixed cards overlap")] Overlap,
    #[error("fixed part has {0} cards, more than 13")] TooManyFixed(u8),
    #[error("constraint is not samplable and rejection sampling is disabled")] NotSamplable,
}

impl Sampler {
    /// pool ∩ fixed = ∅ が必須 (違反は Err)。充足不能はエラーではなく count() == 0。DNF 化は内部で行う
    /// (DnfOptions::default()、Overflow::Residual なので DnfError は出ない)。
    pub fn prepare(c: &HandConstraint, pool: Hand, fixed: Hand, opts: &SampleOptions) -> Result<Sampler, PrepareError>;
    pub fn count(&self) -> u64;                       // Σ c_i (項が互いに素なら |U|)
    pub fn is_exact(&self) -> bool;                   // 全項が residual/custom を持たない
    pub fn sample<R: rand_core::Rng + ?Sized>(&self, rng: &mut R) -> Option<Sample>;   // None: count() == 0 または max_tries 超過
    pub fn log_prob(&self, hand: Hand) -> f64;        // §8.3。和集合に含まれなければ −∞
    pub fn pool(&self) -> Hand; pub fn fixed(&self) -> Hand;
}
```

`Sampler` は `prepare` 後は不変で `Send + Sync` (内部可変性なし)。受理統計は `Sample::tries` として返し、呼び出し側が集計する。項ごとの個数が要る場合 (L4 の成分重み) は代替ごとに `Sampler` を分けて `count()` を読む。

### 2.5 `KnownCards` (`known.rs`)

```rust
/// 各席の元の 13 枚に属すると分かっているカード。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KnownCards { pub known: [Hand; 4] }   // 添字 = Seat

impl KnownCards {
    pub fn new(known: [Hand; 4]) -> Result<KnownCards, KnownCardsError>;   // 互いに素、各 <= 13 を検証
    pub const EMPTY: KnownCards;                                          // 何も分かっていない
    pub fn from_viewer(viewer: Seat, hand: Hand) -> KnownCards;           // known[viewer] = hand
    pub fn with_dummy(self, dummy: Seat, hand: Hand) -> KnownCards;       // known[dummy] |= hand
    pub fn with_play(self, history: &PlayHistory) -> KnownCards;          // 各席に history.played_by(s) を加える (フェーズ 5)
    pub fn pool(&self) -> Hand;                       // FULL − ⋃_s known[s] (未知カード)
    pub fn needed(&self, seat: Seat) -> u8;           // 13 − known[seat].len()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]   // error.rs
pub enum KnownCardsError {
    #[error("card {0} is known for two seats")] Duplicate(Card),
    #[error("{seat} has {count} known cards, more than 13")] TooMany { seat: Seat, count: u8 },
}
```

プレイ途中の既知カードは `KnownCards::from_viewer(viewer, hand).with_dummy(dummy, dummy_hand).with_play(&history)` と組み立てる (`with_*` は和集合なので順序は問わない。`hand` は残り手でも元の手でもよく、`with_play` が `played_by(viewer)` を合成する)。`bridge-play::hard_constraints(history)` はプレイ由来の分 (`EMPTY.with_play(history)`) だけを返し、自分の手とダミーは呼び出し側が加える (`08-play.md` §5)。互いに素なら `Σ_s needed(s) == pool().len()` が自動的に成り立つ。席 `s` のサンプルは `Sampler::prepare(c, known.pool(), known.known[s.index() as usize], opts)` で行う。

## 3. `satisfies` の意味論

`Atom::satisfies(h)` は次を順に評価し、短絡する。

1. `shapes.contains(h.shape())` (`debug_assert!(h.len() == 13)`)。
2. `hcp.contains(&hcp(h))`。
3. 各 `CardRequirement`: `count.contains(&(h ∩ mask).len())`。
4. 各 `EvalRequirement`: `range.contains(&metric.eval(h))`。

`HandConstraint::satisfies` は木を再帰し、`Custom` は `(f)(h)`。コストは `Atom` で 30〜80 ns (シェイプ bit + hcp + カード要件の popcnt + 評価関数)。

## 4. DNF 正規化

### 4.1 交差 (`And` に使う)

`A ∩ B`:

- `shapes = A.shapes ∩ B.shapes`
- `hcp = max(A.lo, B.lo) ..= min(A.hi, B.hi)`
- `cards`: 連結し、同一 `mask` は `count` の交差でマージ
- `eval`: 連結し、同一 `metric` は `range` の交差でマージ
- クランプ: `hcp` を `0..=37`、`count` を `0..=popcount(mask)`、`range` を `0..=metric.max()`

### 4.2 否定 (排他的連鎖、D4)

Atom をリテラル `L1 = shape ∈ S`, `L2 = hcp ∈ [lo, hi]`, `L3.. = cards, eval` の連言と見て、

```
¬Atom = ⋁_j ( L1 ∧ … ∧ L(j−1) ∧ ¬Lj )
¬(shape ∈ S)          = shape ∈ Sᶜ                                   (1 Atom、S = ALL なら省略)
¬(hcp ∈ [lo, hi])     = hcp ∈ [0, lo−1]  ∨  hcp ∈ [hi+1, 37]         (≤ 2 Atom、互いに素)
¬(count ∈ [a, b])     = count ∈ [0, a−1] ∨ count ∈ [b+1, popcount]   (≤ 2 Atom)
¬(metric ∈ [a, b])    = metric ∈ [0, a−1] ∨ metric ∈ [b+1, MAX]      (≤ 2 Atom)
```

結果は高々 `1 + 2 + 2·(|cards| + |eval|)` 個の **互いに素な** Atom。素なので `|¬A| = Σ|項|` が厳密に成り立ち、和集合サンプリングに多重度補正が要らない。「否定したリテラルだけを持つ Atom を 1 個ずつ」の方が小さいが重なるので採らない。リテラルは安価で、厳密な個数が得られることが目的だからである。

`¬Custom` は否定フラグ付き `Custom`、`¬Or` / `¬And` は de Morgan、`¬¬x = x`。

### 4.3 `to_dnf` の手順

1. `Not` を押し下げる: `¬¬x = x`、`¬Or = And(¬…)`、`¬And = Or(¬…)`、`¬Atom = §4.2 の連鎖`、`¬Custom = Custom(negated)`。
2. 再帰的に展開: `dnf(Atom) = [term]`、`dnf(Custom) = [ANY + custom リテラル]`、`dnf(Or) = 連結`、`dnf(And) = 直積を intersect`。
3. `And` を展開する前に `Π |dnf(子)|` を見積もる。`max_terms` (256) を超えるなら、DNF が最大の子から順に `residual` へ退避し (残りの子の直積の各項に `And` で付ける)、見積もりが収まるまで繰り返す。`truncated = true` とし `tracing::warn!` (ノード名付き)。`Overflow::Error` なら `DnfError::TooLarge { estimated, max_terms }` を返す (システムコンパイルの `strict_dnf` で使う)。
4. 単純化: 各 Atom を `normalize`、自明に充足不能な項を除去、同一項を除去。リテラル単位の包含除去 (`shapes ⊆`、`hcp ⊆`) は行わない: 否定由来の項は互いに素で包含が起きず、利用者の `Or` は小さいので O(k²) の検査に見合わない。

`Dnf` の項は、利用者が書いた `Or` 由来の項は重なり得るが、否定由来の項は互いに素である。

## 5. 充足不能判定

段階的に安価な順で行い、コンパイル時の矛盾検出 (仕様 §4) に使う。

| 段階 | 判定 | コスト | 厳密性 |
| --- | --- | --- | --- |
| 1 | `shapes.is_empty()` | 9 語 | 必要条件 |
| 2 | `hcp.start > hcp.end` | O(1) | 必要条件 |
| 3 | `hcp.start > shapes.max_hcp()` または `hcp.end < shapes.min_hcp()` (`MAX_HCP`/`MIN_HCP[len]` の和のメンバー最大/最小) | O(|shapes|) | シェイプ + HCP だけなら厳密。「6-6 で 25+ HCP」(最大 20)、「13 枚スートで HCP <= 9」を捕まえる |
| 4 | `count` / `range` が空、`count.start > popcount(mask)`、`range.start > metric.max()` | O(リテラル数) | 必要条件 |
| 5 | 決定版: `Sampler::prepare(atom, Hand::FULL, Hand::EMPTY, &opts).count() == 0` | 20〜60 μs/Atom | リテラル間の相互作用 (「♠のトップ 3 のうち 2 枚」∧「♠ <= 1」) を含めて厳密 (exact パスのリテラルについて) |

`Atom::is_trivially_unsat` は 1〜4、`HandConstraint::is_satisfiable` は DNF の全項が 1〜4 で落ちれば `false`、それ以外は 5 を実行する。数千ノードのシステムでも 1 秒未満で収まる。

## 6. 厳密サンプラー: 概要

`Sampler` は DNF の各項について「`shape ∈ S`、`hcp ∈ W(shape)`、スート別フィルタ、追加の加法的特徴」の集合を **厳密に数え、一様に抽選する**。原理は、スートごとに部分集合を `(長さ, 鍵)` でバケット化し、2 スート対の疎畳み込みと接頭和で「全体の鍵の和が窓に入る」組合せ数を求めることである。

### 6.1 鍵の詰め込み

```
key = hcp | (x << 6)
```

- `hcp <= 37 < 64` なので下位 6 bit に収まる。
- `x` は任意の 1 個の加法的なスート別特徴 (コントロール、エース数、ルーザー ×2、QT ×2、複数スートのカード要件の枚数)。全手で `4·x_max <= 24 < 64` なので、詰めた鍵の加算が成分ごとの加算に等しい。
- `extra_features = 0` (K=1) は `x` なしの高速パス。`extra_features = 1` (K=2) で追加指標 1 つが厳密になる。鍵空間は K=1 で 38、K=2 で最大 `25 × 64 = 1600`。

### 6.2 リテラルの分類 (`prepare` 時、各リテラルはちょうど 1 つの扱いに入る)

| リテラル | 扱い | 厳密 |
| --- | --- | --- |
| `shapes` | 到達可能シェイプを走査 | ○ |
| `hcp` | シェイプごとの HCP 窓 `W(shape)` | ○ |
| `TotalPoints(m)`, `m.is_shape_only()` | `W(shape) = hcp ∩ (range − shape_points(shape))` | ○ |
| `DistPoints(m)`, `m.is_shape_only()` | `shape_points(shape) ∈ range` でシェイプをフィルタ | ○ |
| `CardRequirement` で `single_suit() == Some(s)` | スート `s` の部分集合列挙時にフィルタ | ○ |
| `SuitQuality(s)` | スート `s` のフィルタ (`honors5[H_s] ∈ range`) | ○ |
| 複数スートの `CardRequirement` (エース数、「どこかにトップオナー」) | 加法的特徴 `x_s = popcount(H_s ∩ mask_s)` | スロットが空いていれば ○ |
| `Controls`, `Losers(_)`, `QuickTricks` | `SUIT` テーブル経由の加法的特徴 | スロットが空いていれば ○ |
| `BergenStarting` の `DistPoints`/`TotalPoints` | 棄却 (residual) | × (K=3 拡張は未決) |
| スロット超過の加法的特徴、`Custom`、DNF の `residual` | 棄却。`max_tries = 256`、受理率 α を burn-in で推定 | × |

### 6.3 データ構造

```rust
// sampler/suit_table.rs (pub(crate))
const KEYS: usize = 64 * 32;             // hcp 0..=37 を下位 6 bit、追加特徴を bit 6.. に詰めた鍵の数
struct SuitTable {                       // スートごと、準備済み項ごと
    holdings: Vec<u16>,                  // 残り部分の部分集合 (確定分を合成済み) を (len, key) でカウンティングソート
    start: Box<[[u32; KEYS + 1]; 14]>,   // start[len][key]。bucket(len, key) = holdings[start[l][k]..start[l][k+1]]
    counts: [SparseVec; 14],             // counts[len] = (key, n) で n > 0 のもの
}
struct SparseVec(Vec<(u16 /* key */, u64 /* count */)>);
struct PairConv { p: SparseVec, prefix: Box<[u64]> }   // K=1: hcp 上の 1 次元接頭和、K=2: (hcp, x) の 2 次元接頭和
impl SuitTable {
    fn build(pool: Holding, fixed: Holding, filter: &dyn Fn(Holding) -> bool, key: &dyn Fn(Holding) -> u16) -> SuitTable;
    fn bucket(&self, len: u8, key: u16) -> &[u16];
}

// sampler/term.rs (pub(crate))
struct PreparedTerm {
    suits: [SuitTable; 4],
    pair01: HashMap<(u8, u8), PairConv>,   // 到達可能シェイプが使う (l0, l1) 対のみ、<= 105
    pair23: HashMap<(u8, u8), PairConv>,
    shapes: Vec<(Shape, u64 /* weight */, (u8, u8) /* hcp 窓 */)>,   // x 窓は K=2 のとき鍵に畳む
    cum: Vec<u64>, total: u64,
    term: DnfTerm,                         // residual / custom の検査に使う元の項
    alpha: Option<f64>,                    // 棄却リテラルの推定受理率 (厳密なら None)
}
impl PreparedTerm {
    fn prepare(term: DnfTerm, pool: Hand, fixed: Hand, opts: &SampleOptions) -> PreparedTerm;
    fn draw<R: rand_core::Rng + ?Sized>(&self, rng: &mut R) -> Hand;   // residual 検査前の 1 手
}
```

フルデッキ・フィルタなし・K=1 の共有テーブル (`FULL_SUIT`、`LazyLock` で実行時 1 回構築、16 KB + オフセット) は §7 手順 3 の高速パスとしてフェーズ 2.4 で追加する (骨格には無い)。`const fn` で作れなくはないが `Vec` を使うため実行時構築とする。

## 7. `prepare` の手順 (DNF 項ごと)

1. 検証: `fixed ∩ pool = ∅` (違反は `PrepareError::Overlap`)、`|fixed| <= 13` (違反は `TooManyFixed`)。`m = 13 − |fixed|`。`|pool| < m` なら項の個数は 0。`to_dnf(&DnfOptions::default())` で項に分ける。
2. リテラルを §6.2 の表で分類する。スート別フィルタのクロージャ (`single_suit` のカード要件、`SuitQuality`)、追加特徴のテーブルポインタ (K=2 のとき)、シェイプフィルタ (`DistPoints` の shape-only) を決める。スロットに入らない加法的特徴・`BergenStarting`・`custom`・`residual` は棄却リテラルとして `term` 側に残す。棄却リテラルがあり `opts.allow_rejection == false` なら `PrepareError::NotSamplable`。
3. 各スート `s` について `P = pool.holding(s)`、`F = fixed.holding(s)`:
   - 高速パス: `P == FULL` かつ `F == EMPTY` かつ `s` にフィルタなし かつ K=1 なら共有 `FULL_SUIT` を参照する (列挙不要)。
   - それ以外: `sub ⊆ P` を全列挙 (`sub = (sub − 1) & P`、`Holding::submasks`)。`H = sub ∪ F` を評価し、スートフィルタが `H` を拒否すれば飛ばす。`len = H.len()`、`key = SUIT.hcp[H] | feat[H] << 6`。カウンティングソート (第 1 パスで個数、第 2 パスで配置) で `holdings` と `start` を作る。コスト `2 × 2^|P|` 反復、約 4 ns/反復。
   - `counts[len]` をヒストグラムから疎リストにする。
   - **確定カードはここで合成される** ので制約は無変換 (D3)。`len` は元の長さ、`key` は確定分の寄与を含む。
4. `atom.shapes` を走査し (シェイプフィルタ適用)、`l_s < |F_s|` または `l_s − |F_s| > |P_s|` のシェイプは飛ばす。残ったシェイプが使う `(l0, l1)` 対と `(l2, l3)` 対に印を付ける (各 <= 105)。印の付いた対ごとに疎畳み込み `counts0[l0] * counts1[l1]` (鍵で添字付けた密なスクラッチに累積して圧縮) とその接頭和 (K=1 は hcp 上の 1 次元、K=2 は `(hcp, x)` 上の 2 次元) を計算する。
5. 各候補シェイプについて `W = atom.hcp ∩ shift(TotalPoints)`、追加特徴の箱 `X = [xlo, xhi]` を求め、
   `weight = Σ_{(a_h, a_x) ∈ pair01.p} n01(a_h, a_x) · BoxSum23(W − a_h, X − a_x)`
   を計算する (`BoxSum23` は接頭和の 2 回参照 (K=1) または 4 回参照 (K=2))。`weight > 0` のシェイプだけ `shapes` に積む。
6. `cum` = シェイプ重みの接頭和、`total = cum.last()`。
7. `residual` が非空で `total > 0` なら `opts.burn_in` 回サンプルして `α` (受理率) を推定する。
8. 和集合レベル: 項の `total` を並べて `cum`、`total = Σ c_i`。

## 8. `sample` の手順と確率

### 8.1 `sample` (K=1。K=2 は箱が 2 次元になるだけで同じ)

1. 項: `r ← random_range(0..total)`、`i = cum.partition_point(|c| *c <= r)`。
2. シェイプ: 項の `cum` で同様に `(l0, l1, l2, l3)` と窓 `[lo, hi]` を引く。
3. 前半の HCP 和 `a` (スート 0+1): `r ← random_range(0..weight)`。`pair01.p` の `a` を走査し `n01[a] · (PS23[hi − a] − PS23[lo − a − 1])` を引いていき、負になったところで止める (<= 21 歩)。ここで使う和は `prepare` の手順 5 で足したものと同じなのでずれない。
4. 後半の HCP 和 `b` (スート 2+3): `b ∈ [lo − a, hi − a]` を `pair23.p` 上で `n23[b]` に比例して選ぶ (<= 21 歩)。
5. `h0 ∝ counts0[l0][h0] · counts1[l1][a − h0]`、`h1 = a − h0`。同様に `h2 ∝ counts2[l2][h2] · counts3[l3][b − h2]`、`h3 = b − h2` (各 <= 11 歩)。
6. 各スート: `bucket = holdings[start[l_s][h_s] .. start[l_s][h_s + 1]]`、`sub_s = bucket[random_range(0..bucket.len())]`。
7. `hand = fixed ∪ Hand::from_holdings(sub_0, sub_1, sub_2, sub_3)`。`debug_assert!(term.atom.satisfies(hand))`。
8. `residual` / `custom` リテラルがあれば検査し、不合格なら手順 2 へ戻る (`max_tries` で打ち切り、`tries` を数える)。
9. `log_prob` (§8.3) を計算して `Sample` を返す。

乱数 9 回 (`random_range` で約 5 ns ずつ)、整数演算約 80、添字付きロード 4 回で **0.3〜0.5 μs**。乱数は `u64` の整数 `random_range` (厳密、浮動小数の丸めなし)。

### 8.2 和集合 (選言) からのサンプリング

項 `T_1..T_k`、現在のプールでの厳密な個数 `c_i`、`C = Σ c_i` とする。

1. `i ∝ c_i` で項を選び (累積 `u64` + `partition_point`、<= 10 回の比較)、`T_i` から一様に `h` を引く。`P(h) = m(h) / C`、`m(h) = #{ i : h ∈ T_i }` (各項の完全な述語を residual/custom 込みで評価)。`log_prob(h) = ln m(h) − ln C`。厳密で、任意の `h` について有効 (`m = 0` なら −∞)。L4 が重点重み付けをするのでこれで足りる。
2. 和集合上の厳密一様 (確率 `1/m(h)` で受理する方式) は公開 API に持たない。テストデータ生成で必要なら `log_prob` を使って呼び出し側が受理すればよい (`P(受理 ∧ h) = 1/C`)。
3. `Custom` を含む場合は `tracing::warn!` を出す (仕様 §4)。

否定由来の項は互いに素なので `m(h) = 1` が保証され、`log_prob = −ln C` になる。多重度補正が必要なのは利用者が書いた `Or` だけである (D4)。

### 8.3 `log_prob` の式 (residual 棄却込み)

項 `i` の内部で棄却 (受理率 `α_i`) がある場合、密度は

```
P(h) = (1 / C) · Σ_{i ∋ h} 1 / α_i
log_prob(h) = ln( Σ_{i ∋ h} 1 / α_i ) − ln C
```

厳密な項では `α_i = 1`。それ以外は `prepare` の burn-in (`n = 256`) で推定し、返却された `tries` から呼び出し側が精緻化できる。`is_exact()` が `false` なら L4 は ESS を近似値として報告する。

### 8.4 確定カードの合成 (D3)

「残り部分に対する Atom」という型は作らない。`prepare(atom, pool, fixed)` はスート別の全ての量を `H_s = sub_s ∪ fixed_s` で評価する。

| 量 | 扱い |
| --- | --- |
| シェイプ | バケットは `len(H_s)` = 元の長さで索引。シェイプ走査は元の `ShapeSet`。実現不能なシェイプ (`l_s < |fixed_s|`、`l_s − |fixed_s| > |pool_s|`) は空バケットで重み 0 |
| HCP と加法的特徴 | `key(H_s)` が確定分の寄与を含むので `hcp` 範囲・評価範囲はシフトせずそのまま使う |
| スート別カード要件 | `H_s` で評価。「♦A が確定済み」なら自動的に充足、「♦A が他所で既出」ならプールに無いので自動的に不能 |
| 複数スートの加法的特徴 | 同上、スートごと |

したがって exact パスのリテラルは全て厳密なままで、不正確なリテラルはフルデッキの場合と同じ集合 (`Custom`、residual、スロット超過) に限られ、それらは `fixed ∪ 残り` の完全な手で棄却検査される。帰結: (1) `count()` は「既知カードと Atom に整合する元の手の数」で、`log_prob = −ln count` はその席の手の既知カード条件付きの厳密な密度。(2) `count() == 0` はその解釈代替が既知カードと矛盾することを意味し、L4 はエラーではなく証拠として扱う (代替を落として再正規化)。

### 8.5 整数幅 (D13)

| 量 | 上限 | 備考 |
| --- | --- | --- |
| スート別 `counts` | 1,716 (`C(13, 6)`) | `u16` で足りるが統一のため `u64` |
| 対畳み込みの要素 | `1716² ≈ 2.9×10^6` | |
| シェイプ別重み | `1.67×10^10` | `u32` (4.29×10^9) を超える |
| `total` | `C(52, 13) = 635,013,559,600 ≈ 6.35×10^11` | |
| 中間値 `n01 · BoxSum23` | `≈ 8.7×10^12` | |

全て `u64` に 6 桁の余裕を持って収まる。仕様の `u32` 相当では表現できない。乱数も `u64` の `random_range`。

## 9. コストとメモリ

| 操作 | コスト | 備考 |
| --- | --- | --- |
| `Atom::satisfies` | 30〜80 ns | |
| `to_dnf` | O(Π 子サイズ)、256 項で打ち切り | + O(k²) の包含除去 |
| `prepare`、フルデッキ、スートフィルタなし、K=1 | 20〜60 μs | 対 <= 105 × 121 積和、シェイプ <= 560 × 21 |
| `prepare`、部分集合列挙が要るスート | +65 μs/スート (2 × 2^13 × 4 ns) | フィルタ・確定・縮小プールのあるスート。フルデッキ最悪 +260 μs |
| `prepare`、プレイ途中 (未知 26 枚) | 3〜10 μs | スートあたり 2^6.5 |
| `prepare`、K=2 | ×3〜5 | スートあたり 160 状態、2 次元接頭和 |
| `sample`、K=1 | 0.3〜0.5 μs | 乱数 9 回、約 80 演算 |
| `sample`、K=2 | 1〜2 μs | <= 147 対状態の走査 |
| `sample`、residual あり (受理率 α) | (0.5 μs + 50 ns) / α | `max_tries` で有界 |
| `log_prob(hand)`、k 項 | k × 50 ns | |
| 準備済み項のメモリ | 典型 20 KB、最悪 120 KB (K=1) | 共有 `FULL_SUIT` 16 KB |

目標との対比: 10^5 手/秒/コアには 10 μs/手で足りるところ、exact パスは約 20 倍速い (約 2×10^6 手/秒)。`prepare` が高コスト側なので `Sampler` を一級オブジェクトにし、L4 が `(項, pool, fixed)` でキャッシュする。仕様互換の `HandConstraint::sample` は O(prepare) と明記する。**無制約 (`ANY` 相当) の項は組合せ的に直接配る高速パス** (`pool` から `m` 枚を Fisher-Yates で選ぶ、`count = C(|pool|, m)`) を用意し、`prepare` を省く。

## 10. 検証テスト

厳密に数えられることから直接導かれるテスト。

| テスト | 内容 | 基準 |
| --- | --- | --- |
| 項の充足 (proptest) | 全サンプルが自身の項を満たす | 100% |
| 周辺分布 (χ²) | 10^6 サンプルのシェイプ分布と HCP 分布を、サンプラー自身の `weight` 表と `W` 表 (期待値が厳密) と比較 | p > 0.01 |
| 小プール全列挙 | `|fixed| = 9`、`|pool| = 8` → 候補 `C(8, 4) = 70` 手を全列挙し、`count()` と一致、`Σ exp(log_prob) = 1` | 完全一致 |
| フルデッキの厳密個数 | 15-17 バランスの `count() == 30,897,212,184` (`= 0.04866 × C(52, 13)`) | 完全一致 |
| 否定の素性 (proptest) | ランダムな手で `A` と `¬A` の項のちょうど 1 つが真 | 100% |
| 交差の整合 | `(A ∩ B).satisfies(h) == A.satisfies(h) && B.satisfies(h)` | 100% |
| 和集合の `log_prob` | 重なる `Or` で `Σ_h exp(log_prob(h)) = 1` (小プール) | 誤差 1e-9 |
| 確定カード | フルデッキで引いた手を一部確定させ、条件付き分布が全列挙と一致 | χ² |
| 決定版の充足不能 | 「♠ トップ 3 のうち 2 枚」∧「♠ <= 1」で `count() == 0` | |
| ベンチ | `sample` >= 10^5 手/秒/コア、`prepare` 20〜60 μs | criterion |

## 11. モジュール構成

```
bridge-constraint/src/
├── lib.rs                  // 再エクスポート (bridge_core::{Hand, Shape, ShapeClass, ShapeSet}、bridge_eval::{DistMethod, Half, LtcMethod} も)
├── atom.rs                 // Atom (intersect/negate/normalize), CardRequirement, EvalRequirement, Metric
├── constraint.rs           // CustomPred, HandConstraint (to_dnf、要約、合成、satisfies、sample)、手動 serde
├── dnf.rs                  // DnfTerm, Dnf, DnfOptions, Overflow
├── known.rs                // KnownCards
├── error.rs                // DnfError, PrepareError, KnownCardsError
└── sampler/                // pub mod
    ├── mod.rs              // Sampler, Sample, SampleOptions、和集合レベル、log_prob
    ├── suit_table.rs       // SuitTable, SparseVec, PairConv、部分集合列挙とカウンティングソート (pub(crate))
    └── term.rs             // PreparedTerm、対畳み込み、シェイプ重み、項内サンプル (pub(crate))
```

`benches/sampler.rs` (criterion) を持つ。実装順 (フェーズ 2): `Atom`/`satisfies`/交差/否定 → DNF (cap/residual) → `Sampler` K=1 (厳密個数テスト、χ²) → 確定カード合成 → K=2 + residual/α → `KnownCards` → 無制約高速パス。

## 12. 未決

- 未決: `BergenStarting` を K=3 (長スート点はシェイプ、品質スート数と adjust-3 を追加特徴 2 つ) で厳密に扱う拡張は、フェーズ 2.6 のベンチで K=2 の余裕を見てから決める。v1 は棄却。

`KnownCards::with_play(&PlayHistory)` は本クレートの inherent メソッドとして確定した (`bridge-play` はそれを呼ぶだけ)。`to_dnf` の包含除去は行わない (§4.3)。
