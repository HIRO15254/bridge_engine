# 08. L5 `bridge-play`: プレイ側の約束

本書は L5 `bridge-play` の詳細設計である。v1 は **ハード制約**（ショウアウトによるスート長の確定、既出カードの `KnownCards` への集約、整合性検査）と **宣言済み軟情報**（リード約束、シグナル約束、初回ディスカード）のルールテーブルを含み、相手の方策モデルを要する推論は含まない。制約はすべて `bridge-constraint` の語彙（`ShapeSet`、`CardRequirement { mask, count }`）で表し、複数イベントの結合は 07-bidding.md Step B と同じ直積・剪定・K=8 で行う。

関連文書: 02-core.md（`PlayHistory`、`Contract`）、05-constraint.md（`Atom`、`CardRequirement`、`KnownCards`）、07-bidding.md（Step B）、09-sample.md（`SampleContext.play_constraints` / `play_soft`）。

---

## 1. 位置づけ

| 項目 | 内容 |
| --- | --- |
| クレート | `bridge-play`（lib 名 `bridge_play`） |
| 依存 | `bridge-core`、`bridge-constraint`（経由で `bridge-eval`）。`HandConstraint` と `KnownCards` を再エクスポートする |
| 依存しないもの | `bridge-system`。プレイ側の約束はビディングシステムと独立に更新され作者も異なる（仕様 §2、§8）。ビディングとは別ファイル・別テーブル |
| 外部依存 | `thiserror`、`serde`（optional） |
| feature | `default = ["std"]`、`std`、`serde` |
| 主要 API | `PlayAgreements`、`hard_constraints`、`interpret_play`、`PlayInterpretation::into_seats`、`lead_constraints`、`signal_constraints` |

---

## 2. v1 スコープ

仕様 §8 の段階表と、v1 で実装する項目の対応。

| 段階 | 内容 | 実装方式 | v1 |
| --- | --- | --- | --- |
| ハード制約 | ショウアウト = 元の手のスート長が確定、カードカウント | 単なるフィルタ | 含む |
| 軟情報（宣言済み） | リード約束、シグナル約束 | ルールテーブル | 含む |
| 学習済み方策 | 「良いディフェンダーならこう打たない」 | 相手の方策モデル | 含まない |

| 含む | 含まない |
| --- | --- |
| `PlayHistory` からのハード制約（ショウアウト → 正確なスート長、既出カード → `KnownCards`、ダミー公開、枚数整合性検査） | 宣言者の方策推論、「良いディフェンダーなら」型の推論 |
| `LeadTable`: スポットリード 4th best / 3rd-5th / attitude、オナーリード Standard / Rusinow / Jack-denies（Journalist 系）、vs suit と vs NT の別 | オークション依存のリード規則（パートナーがビッドしたスート等）。フック（`LeadContext`）はあるが表は空 |
| `SignalTable`: パートナーのリードへの attitude、宣言者リードへの count、初回ディスカード（attitude / Lavinthal / odd-even）、Standard / Upside-down | トランプエコー、Smith エコー、obvious shift |

**ハード制約だけでも効果が大きい。** 実装コストがほぼゼロで、サンプリングの棄却率を劇的に下げるので、フェーズ 5.1 で最初に入れる。リード・シグナル表（5.8）はフェーズ 6 のオープニングリードアドバイザには不要（リード時点でプレイ履歴が存在しない）なので、ずれても構わない。

---

## 3. 開示義務という構造的利点（仕様 §8）

リード約束とシグナルは事前に宣言され、コンベンションカードに記載される開示義務のある情報である。したがって「4th best のリードだから、そのスートは 4 枚以上で、リードした札の上に 3 枚ある」はルールエンジンとして記述でき、学習を必要としない。ポーカーでは相手の戦略が一切開示されないため、これはブリッジ固有の利点であり、取りに行かない理由がない。

三段目（学習済み方策）を切る理由: 「相手がそのカードを選んだ」という情報から手を推論するには相手のプレイ方策のモデルが必要で、これは αμ 論文が扱う non-locality そのものである。将来は `bridge-sample::Proposal` の実装として外から差し込む。

---

## 4. 型

```rust
// agreements.rs（全て Clone + PartialEq + Debug + Default、serde 派生は feature）
pub struct PlayAgreements { pub leads: LeadTable, pub signals: SignalTable, pub discards: DiscardTable }

pub struct LeadTable { pub vs_suit: LeadStyle, pub vs_nt: LeadStyle }
pub struct LeadStyle { pub spot: SpotLead, pub honors: HonorLeads, pub confidence: f32 /* 既定 0.8 */ }
#[derive(Clone, Copy, Default)] pub enum SpotLead { FourthBest, ThirdFifth, Attitude, #[default] Unknown }
#[derive(Clone, Copy, Default)] pub enum HonorLeads { Standard, Rusinow, JackDenies, #[default] Unknown }

pub struct SignalTable { pub attitude: Polarity, pub count: Polarity, pub confidence: f32 /* 既定 0.7 */ }
#[derive(Clone, Copy, Default)] pub enum Polarity { Standard, UpsideDown, #[default] Unknown }
pub struct DiscardTable { pub first: FirstDiscard, pub polarity: Polarity }   // 重みは表の定数 0.6
#[derive(Clone, Copy, Default)] pub enum FirstDiscard { Attitude, Lavinthal, OddEven, #[default] Unknown }

// hard.rs
pub fn hard_constraints(history: &PlayHistory) -> ([HandConstraint; 4], KnownCards, Vec<PlayWarning>);

// interpret.rs
pub struct PlayInterpretation {
    pub known: KnownCards,                       // プレイから分かった各席の元の手のカード（既出カード）
    pub hard: [HandConstraint; 4],               // 席ごとの Atom（ショウアウトしたスートは長さ確定）
    pub soft: [Vec<(HandConstraint, f32)>; 4],   // 席ごとの結合済み重み付き選言（合計 1、長さ ≤ K = 8）
    pub events: Vec<PlayEvent>,                  // 監査用: どのカードにどの規則が発火したか
    pub warnings: Vec<PlayWarning>,
}
pub struct PlayEvent { pub seat: Seat, pub card: Card, pub rule: &'static str }
pub enum PlayWarning {
    Inconsistent { suit: Suit },                    // 最小長の合計 > 13、または既知カード > 13
    RevokeSuspected { trick: usize, seat: Seat },   // ショウアウト後に同じスートを出した
}

pub fn interpret_play(history: &PlayHistory, contract: &Contract, agreements: &[PlayAgreements; 4]) -> PlayInterpretation;
impl PlayInterpretation {
    /// 仕様の戻り型: 席ごとに hard ∧ soft_i の重み付き選言
    pub fn into_seats(self) -> [Vec<(HandConstraint, f32)>; 4];
}

// leads.rs / signals.rs（規則表の入口。interpret_play が呼ぶが単体でも使える）
pub fn lead_constraints(card: Card, contract: &Contract, style: &LeadStyle) -> Vec<(HandConstraint, f32)>;
pub struct SignalEvent { pub seat: Seat, pub card: Card, pub kind: SignalKind }
pub enum SignalKind { Attitude, Count, FirstDiscard }
pub fn signal_constraints(event: SignalEvent, signals: &SignalTable, discards: &DiscardTable) -> Vec<(HandConstraint, f32)>;
```

`agreements[s]` は席 `s` のペアの約束である。ディフェンダー 2 席は通常同じ `PlayAgreements` を共有するが、型は席ごとに持つ（ビディングの `Table` と同じ理由）。宣言者側の `agreements` は読まれない。「全部 Unknown」は `PlayAgreements::default()` で、規則は一切発火しない。`PlayEvent` の `trick` は `card` から `history.cards().iter().position(..) / 4` で引けるので保持しない。

---

## 5. `hard_constraints` と `KnownCards`

`KnownCards` 型は `bridge-constraint` に置く（05-constraint.md §2.5、09-sample.md §4）。プレイ履歴からの構築 `KnownCards::with_play(&PlayHistory)` も `bridge-constraint::known.rs` の inherent メソッドで（Rust の孤児規則により `bridge-play` から外部型にメソッドを足せない）、`bridge-play` は `KnownCards` を再エクスポートし、`hard_constraints` がそれを呼ぶ。

```rust
pub fn hard_constraints(history: &PlayHistory) -> ([HandConstraint; 4], KnownCards, Vec<PlayWarning>);
```

1. 返す `KnownCards` は `KnownCards::EMPTY.with_play(history)`、すなわち各席 `s` について `known[s] = history.played_by(s)`（プレイから分かる分だけ）。
2. 自分の手とダミーは呼び出し側が加える: `known.with_dummy(dummy, dummy_hand)` と `KnownCards::from_viewer(viewer, hand)` との和（`with_*` は和集合なので順序は問わない。`hand` は残り手でも元の手でもよく、`with_play` が `played_by(viewer)` を合成する）。ダミーはオープニングリードの後に公開されるので、`history.cards()` が 1 枚以上のときだけ加える。
3. 互いに素・各 ≤ 13 の検証は `KnownCards::new` が行う。`with_play` の結果が矛盾する（同じカードが 2 席から出た）記録は `PlayWarning::Inconsistent` ではなく `KnownCardsError` として呼び出し側が検出する（記録そのものが壊れている）。

サンプリングの対象は常に **元の 13 枚の配牌** である（D3）。ビディング制約もプレイ制約も元の手について述べる。

---

## 6. ハード制約のアルゴリズム

`history.cards` を 4 枚ずつのトリックとして走査する。最後のトリックは未完でよい。

1. `leader_0 = history.leader()`。`hard_constraints` は `Contract` を受け取らないので、`history.leader()` が `contract.leader()`（宣言者の LHO）と一致することは `interpret_play` の呼び出し側（`GameView` の構築、09-sample.md の文脈組立）が保証する。
2. トリック `t` の `i` 枚目の席は `history.seat_at(4t + i) = trick_leader(t).offset(i)`。`led = cards[4t].suit()`。
3. 各カード `c`（席 `s`）について `played[s] |= c`。`i > 0` かつ `c.suit() != led` なら `shown_out[s][led] = true`（`s` はリードしていないので、フォローできなかった）。すでに `shown_out[s][c.suit()]` が真なら `PlayWarning::RevokeSuspected { trick: t, seat: s }`。
4. トリックが完了したら `history.trick_winner(t)` を次のリーダーとする（勝者は `key(card) = (切り札 32 | リードスート 16 | 0) + rank` の最大。02-core.md）。
5. 走査後、席 `s`・スート `u` について最小長 `min_len[s][u] = played[s].holding(u).len()`。`shown_out[s][u]` なら **元の手の `u` の長さはちょうど `min_len[s][u]`**（ショウアウト後は `u` を出せないので、出した枚数が元の枚数）。
6. `hard[s] = HandConstraint::Atom(Atom { shapes: ShapeSet::from_suit_lens(lens), hcp: 0..=37, cards: [], eval: [] })`。`lens[u] = n..=n`（ショウアウト）または `min_len[s][u]..=13`（それ以外）。`ShapeSet` は 13 枚の合計を満たすシェイプしか含まないので、他スートの下限から導かれる上限は自動的に効く（D1: `suit_len` フィールドは持たず、シェイプ集合のマスクで表す）。
7. **整合性検査**: 各スート `u` について `Σ_s min_len[s][u] ≤ 13`、各席について `Σ_u min_len[s][u] ≤ 13`、および `KnownCards` に入る既知カードが各スート 13 枚以下。違反は `PlayWarning::Inconsistent { suit }`（リボークまたは記録の誤り）。制約集合はそのまま返す（呼び出し側が `EmptySupport` として扱う）。
8. カードの同一性は `hard` に入れず、`KnownCards`（サンプラーの `fixed`）に入れる。`Sampler::prepare` は確定カードをスート表の構築時に合成する（D3）ので、`CardRequirement { mask: card, count: 1..=1 }` を並べるより速く、しかも厳密である。

例: 4♥ by South。トリック 1: W ♠K、N ♠4、E ♠2、S ♠A。トリック 2: S ♥A、W ♥3、N ♥5、E ♣2。East はハートをフォローできなかったので `shown_out[E][♥]`、`min_len[E][♥] = 0` → `hard[E]` は `♥ = 0..=0`（ボイド確定）。他の席は `♠ ≥ 1, ♥ ≥ 1`。`known[E] = {♠2, ♣2}`。

---

## 7. 軟情報: ルールテーブル

### 7.1 記法

制約はすべて 05-constraint.md の `Atom` で書く。`CardRequirement { mask: Hand, count: RangeInclusive<u8> }` は `popcount(hand ∩ mask) ∈ count` を意味する。

```rust
fn above(u: Suit, r: Rank) -> Hand;              // スート u の rank > r のカード集合（4th best 用: 「r より上の札」）
fn honors(u: Suit) -> Hand;                      // スート u の {A, K, Q, J}
fn card(u: Suit, r: Rank) -> Hand;               // 1 枚
fn req(mask: Hand, count: RangeInclusive<u8>) -> CardRequirement;
fn len(u: Suit, lo: u8, hi: u8) -> ShapeSet;     // ShapeSet::from_suit_len(u, lo, hi)
fn lens(u: Suit, set: &[u8]) -> ShapeSet;        // 列挙した長さの和集合（偶奇に使う）
```

| 略記 | 意味 | `Atom` での表現 |
| --- | --- | --- |
| `len[u] ≥ 4` | スート u が 4 枚以上 | `shapes = len(u, 4, 13)` |
| `len[u] = 3` | ちょうど 3 枚 | `shapes = len(u, 3, 3)` |
| `above(u, r) = 3` | リードした札 r より上が 3 枚 | `cards = [req(above(u, r), 3..=3)]` |
| `Has(u, r)` | その札を持つ | `cards = [req(card(u, r), 1..=1)]` |
| `Lacks(u, r)` | その札を持たない | `cards = [req(card(u, r), 0..=0)]` |
| `honors(u) ≥ 1` / `= 0` | オナーあり / なし | `cards = [req(honors(u), 1..=4)]` / `[req(honors(u), 0..=0)]` |
| `even[u]` | 偶数長 | `shapes = lens(u, &[2, 4, 6, 8, 10, 12])`（1 つの `ShapeSet` で表せるので `Or` 不要） |

「高い」スポット = rank ≥ 7、「低い」= rank ≤ 5、6 は曖昧。`Polarity::UpsideDown` は高低を反転する。`Unknown` は規則を無効化する。各規則は 1 席・1 イベントについて合計 1 の `Vec<(HandConstraint, f32)>` を返し、残り `1 − w` は常に `ANY` に付ける。

### 7.2 リード規則（オープニングリード。リーダー = ディフェンダー、スート `u`、リードした rank `r`）

`contract.bid.strain() == NoTrump` なら `leads.vs_nt`、それ以外は `leads.vs_suit` を使う。

| イベント | 約束 | 制約（リーダーの元の手） | w |
| --- | --- | --- | --- |
| スポット（r ≤ 9）のリード | `FourthBest` | `len[u] ≥ 4 ∧ above(u, r) = 3` | 0.8（残り 0.2 は `ANY`） |
| スポットのリード | `ThirdFifth` | `Or(len[u] = 3 ∧ above(u, r) = 2, len[u] ≥ 5 ∧ above(u, r) = 4)` | 0.8 |
| スポットのリード | `Attitude` | r ≥ 7: `honors(u) = 0`。r ≤ 6: `honors(u) ≥ 1`（r は既知なので `Or` は 1 枝に潰れる） | 0.7 |
| K のリード vs suit | `Standard` | `Or(Has(u, A), Has(u, Q))`（K 自体は `KnownCards` に入る） | 0.9 |
| K のリード vs NT | `Standard` | `Or(Has(u, Q) ∧ req({J, T}, 1..=2), req({A, J, T}, 3..=3))`（KQJ / KQT、または AKJT） | 0.85 |
| K / Q / J / T / 9 のリード | `Rusinow` | `Has(u, r + 1)`（K→A、Q→K、J→Q、T→J、9→T） | 0.9 |
| J のリード | `JackDenies` | `req({A, K, Q}, 0..=0)` | 0.9 |
| T のリード | `JackDenies` | `Or(Has(u, J) ∧ req({A, K, Q}, 1..=3), Has(u, 9) ∧ req({A, K, Q, J}, 0..=0))`（J + 上位オナー、または T9x） | 0.9 |

`ThirdFifth` の 4 枚からの 3rd（`len[u] = 4 ∧ above = 2`）を枝に加えるかは「未決」（計画の例に従い v1 は 3 枚と 5 枚以上のみ）。オープニングリード以外のリード（中盤のリード）に同じ規則を適用するかも「未決」で、v1 はトリック 0 の 1 枚目だけを対象にする。

### 7.3 シグナル規則

| イベント | 約束 | 制約 | w |
| --- | --- | --- | --- |
| パートナーのリードに 3rd hand がスポットでフォローし、トリックを取らない | attitude `Standard` | r ≥ 7: `honors(u) ≥ 1`。r ≤ 5: `honors(u) = 0`。r = 6: 両枝 0.35 ずつ | 0.7 |
| 宣言者（またはダミー）がリードしたスートにディフェンダーが 2 度目のスポットを出す | count `Standard` | 1 枚目 > 2 枚目: `even[u]`。それ以外: `odd[u]`（`lens(u, &[3, 5, 7, 9, 11, 13])`） | 0.7 |

`UpsideDown` では attitude は高低を反転し、count は高低 = 奇数になる。「トリックを取らない」は `trick_winner(t) != s` で判定する。2 枚目の count はスート `u` でその席が出した 2 枚がともにスポット（rank ≤ 9）で、どちらもトリックを取っていない場合にのみ発火する。

### 7.4 ディスカード規則（そのディフェンダーの最初のディスカード。スート `v` の札、切り札でのラフは除く）

| 約束 | 制約 | w |
| --- | --- | --- |
| `Attitude` | 高い札: `honors(v) ≥ 1`。低い札: `honors(v) = 0` | 0.6 |
| `Lavinthal` | 高い札: 残り 2 スート（切り札とリードスートを除く）の高い方に `honors ≥ 1`。低い札: 低い方に `honors ≥ 1`（制約への写像は「未決」） | 0.6 |
| `OddEven` | 奇数: `honors(v) ≥ 1`。偶数の高い札 / 低い札: Lavinthal と同じ（「未決」） | 0.6 |

### 7.5 適用条件

1. 規則はディフェンダー（`contract.declarer` と反対側の 2 席）にのみ適用する。宣言者とダミーは約束を持たない。
2. `agreements[s]` の該当フィールドが `Unknown` なら規則は発火しない。`confidence` はテーブルの既定 w を上書きする（`LeadStyle.confidence` はリード規則全体、`SignalTable.confidence` はシグナル規則全体）。`DiscardTable` には `confidence` が無く、ディスカード規則の重みは表の定数 0.6 で固定する。
3. 発火した規則は `PlayEvent { seat, card, rule }` として `events` に残す。制約に変換できなかったイベントは残さない（監査は発火の有無を見る）。
4. スポット / オナーの境界: オナーリード規則は r ∈ {K, Q, J, T, 9}（`Rusinow`）、{K}（`Standard`）、{J, T}（`JackDenies`）。A のリードと Q のリードの規則は v1 では持たない（「未決」）。

### 7.6 複数イベントの結合と `into_seats`

同じ席に複数のイベント（リード規則 + 後のシグナル）が発火したら、07-bidding.md §4.4 Step B と同じ手続きで結合する。

1. `combos = [(ANY, 1.0)]` から始め、各イベントの `Vec<(HandConstraint, f32)>` と直積し `and` する。
2. `is_satisfiable`（要約検査）で剪定する。`hard[s]` と矛盾する枝も落とす。
3. 重み降順に K = 8 に切り詰め、正規化する。空になったら `[(ANY, 1.0)]` と `tracing::warn!`。
4. `into_seats` は `[(hard[s].and(soft_i), w_i)]` を返す。イベントのない席は `[(hard[s], 1.0)]`。

`SampleContext` は `hard` を `play_constraints`、`soft` を `play_soft` として別々に受け取る（09-sample.md §2.2）ので、サンプラーからは `into_seats` を呼ばず、`hard` / `soft` を直接渡す。`into_seats` は仕様互換の便宜 API である。

---

## 8. サンプラーとの接続

| `PlayInterpretation` | `SampleContext` | 役割 |
| --- | --- | --- |
| `known` | `known: KnownCards` | `Sampler::prepare(.., pool, fixed = known[s], ..)` の `fixed` |
| `hard[s]` | `play_constraints[s]` | 全代替に AND される。尤度では `ln 1[hard_s]` |
| `soft[s]` | `play_soft[s]` | 提案分布では `interpretation.seats[s] ⊗ play_soft[s]`、尤度では `ln Σ_i w_{s,i} 1[soft_{s,i}]` |

ハード制約は「長さ」だけなので `Sampler` の高速パス（シェイプ走査 + HCP 窓）に乗り、`cards` の列挙フィルタを要さない。

---

## 9. 先送り（v2 以降）

| 項目 | 理由 |
| --- | --- |
| 宣言者の方策推論、「良いディフェンダーならこう打たない」 | 相手の方策モデルが必要（αμ の non-locality）。`Proposal` 実装として外付け |
| オークション依存のリード規則（パートナーのスート、ダブルしたスート） | `LeadContext { auction: Option<&Auction> }` のフックだけ用意し、表は空 |
| トランプエコー、Smith エコー、obvious shift | v1 のシグナル表に含めない |
| 中盤のリード、A / Q のオナーリード | 7.2 の「未決」 |
| 学習済みシグナル解釈 | 「相手がそのカードを選んだ」情報の利用はスコープ外 |

---

## 10. モジュール構成・テスト・実装順

| ファイル | 内容 |
| --- | --- |
| `agreements.rs` | `PlayAgreements`、`LeadTable`、`LeadStyle`、`SignalTable`、`DiscardTable` と enum（`Default` = 全部 `Unknown`） |
| `hard.rs` | `hard_constraints`: §6 の走査、`hard`、`KnownCards::with_play`、整合性検査 |
| `leads.rs` | `lead_constraints`: §7.2 の表 |
| `signals.rs` | `SignalEvent`、`SignalKind`、`signal_constraints`: §7.3 / §7.4 の表 |
| `interpret.rs` | `interpret_play`、`PlayInterpretation`、`into_seats`、`PlayEvent`、`PlayWarning`、§7.6 の結合（07-bidding.md の Step B と同じ手続き。共有はしない: L5 は L3 に依存しない） |

| テスト | 種類 | 基準 |
| --- | --- | --- |
| `hard_constraints_from_showout` | unit、手組みの履歴 | ショウアウトしたスートの長さが確定、`KnownCards` が既出カードと一致、矛盾した記録で `Inconsistent` / `RevokeSuspected` |
| `lead_rules_table` | table-driven: (約束, リード札) → 標本ホールディングで満たす / 満たさない | 表の各行 |
| `signal_rules_table` | table-driven: attitude / count / 初回ディスカード、`Standard` と `UpsideDown` | 表の各行 |
| `combine_caps_at_k` | unit | 3 イベント × 2 枝で K = 8 に収まり、合計 1 |
| `declarer_side_has_no_events` | unit | 宣言者・ダミーのカードで `events` が空 |

実装順（計画 §12）: 5.1 `hard_constraints` + `KnownCards::with_play`（`KnownCards` の他のメソッドは `bridge-constraint` 2.7 で先行）→ 5.8 `lead_constraints` / `signal_constraints` の表と `interpret_play` の結合（フェーズ 6 には不要。ずれても可）。

---

## 11. 未決事項

| # | 項目 | 現在の仮置き |
| --- | --- | --- |
| 1 | `ThirdFifth` の 4 枚からの 3rd を枝に加えるか | 加えない |
| 2 | 中盤のリードに §7.2 を適用するか | トリック 0 のみ |
| 3 | A / Q のオナーリード規則 | なし |
| 4 | `Lavinthal` / `OddEven` の制約への写像 | 残り 2 スートのオナー有無、w = 0.6 |
| 5 | `honors(u)` に T を含めるか（attitude 規則） | 含めない（A K Q J） |
| 6 | 高低の境界（≥ 7 / ≤ 5）を手札との相対で判定するか | 絶対値。相対判定は方策モデルの領域 |
