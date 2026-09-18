# 07. L3 `bridge-bidding`: 解釈器と生成器

本書は L3 `bridge-bidding` の詳細設計である。`interpret` は各コールを「コール毎の重み付き選言」に分解し（Step A）、席ごとに直積で結合して上限 K=8 の選言に切り詰める（Step B、D11）。低信頼度の解釈は制約を緩めるのではなく ε-混合で表現し（D15）、`choose_bid` は `BidChoice` enum（D6）で「候補なし」をエラーではなく戻り値として返す。この層は `SystemIR` と `NaturalInference` を読むだけで、独自の判断ロジックを持たない。

関連文書: 05-constraint.md（`HandConstraint`、`Sampler`、`KnownCards`）、06-system.md（`AuctionTrie`、`NaturalInference`、`Lint`）、09-sample.md（尤度の消費側）、11-testing.md、13-decisions.md（D6、D11、D15）。

---

## 1. 位置づけと責務境界

| 項目 | 内容 |
| --- | --- |
| クレート | `bridge-bidding`（lib 名 `bridge_bidding`） |
| 依存 | `bridge-core`、`bridge-constraint`、`bridge-system`（経由で `bridge-eval`）。`NaturalInference`、`NodeId`、`SystemIR` を再エクスポートする |
| 外部依存 | `smallvec`、`tracing`、`serde`（optional）。エラー型が無いので `thiserror` は使わない |
| feature | `default = ["std"]`、`std`、`serde = ["dep:serde", ..]`（下位の `serde` を伝播） |
| wasm32 | 可（外部依存はすべて wasm-safe） |
| 主要 API | `interpret`、`choose_bid`、`call_distribution`、`sequence_log_likelihood`、`replay`、`InterpretCache` |

### 1.1 この層に書いてはいけないもの（仕様 §6）

仕様が明示する 3 項目に、計画から導かれる 3 項目を加える。判断ルールがこのクレートに書かれ始めたら、それは L2 の表現力不足のサインであり、L3 ではなく BML の語彙か `SystemMeta` を拡張する。

| 書いてはいけないもの | 正しい置き場 | 理由 |
| --- | --- | --- |
| ビッドの巧拙判断（「このビッドは下手」） | 上位アプリ | システムを差し替えたときに判断まで壊れる |
| ハンドの補正評価（アップグレード / ダウングレード） | `SystemMeta` の宣言（L2） | 評価方式はシステム定義の一部 |
| 採点形式による戦略変更 | `BidContext` を通じた L2 の条件分岐 | L3 は `scoring` を素通しするだけ |
| 説明文（`description`）のテキスト解析 | L2 の説明文コンパイラ | L3 が読む数値は `branch_weights` と `priority` だけ |
| `SystemIR` 内部のキャッシュ・可変状態 | 呼び出し側が持つ `InterpretCache` | 仕様 §9「キャッシュは呼び出し側が持つ」 |
| コールの合法性判定 | `Auction::is_legal`（L0） | システム定義に含まれる不正ビッドを検出できなくなる |

---

## 2. `bridge-system` から消費するインターフェース

L3 が呼ぶ L2 の API は以下に限る（計画 §5.3、§5.5、§5.7）。これ以外の L2 内部（`Row`、`CallPattern`、`Binding`）には触れない。

```rust
// bridge_system::trie
pub struct LookupKey<'a> {
    pub we_opened: bool,        // 最初の非パスコールが owner の側か
    pub calls: &'a [Call],      // 先頭パスを除いたコール列
    pub opener_pos: u8,         // オープナーの席位置 1..=4（#SEAT 条件）
    pub vul: RelVul,            // owner の側から見た we/they のバルネラビリティ
}
impl<'a> LookupKey<'a> {
    pub fn for_auction(auction: &'a Auction, owner: Seat) -> Option<LookupKey<'a>>;   // パスアウト / 空は None
}
pub struct Lookup {
    pub matched_depth: usize,                        // 条件を満たすエントリが揃った最長接頭辞の長さ
    pub by_depth: SmallVec<[Option<NodeId>; 16]>,    // 深さ i のノード（相手コールの深さは None）
    pub end: TrieId,                                 // 一致した接頭辞の末尾位置
    pub via_class: u8,                               // ワイルドカード辺（OppClass）を通った回数
}
impl Lookup { pub fn is_exact(&self, key: &LookupKey<'_>) -> bool; }   // matched_depth == key.calls.len()
impl AuctionTrie {
    pub fn resolve(&self, key: &LookupKey<'_>) -> Lookup;                                        // 深さ × 約 30 ns
    pub fn children(&self, at: TrieId, opener_pos: u8, vul: RelVul) -> Vec<(Call, NodeId)>;
    pub fn resolve_lenient(&self, key: &LookupKey<'_>, max_subst: u8) -> SmallVec<[(Lookup, u8); 4]>;  // 相手の未知コールをパス扱い
}
impl SystemIR {
    pub fn resolve(&self, auction: &Auction, owner: Seat) -> Option<Lookup>;                     // for_auction + index.resolve
    pub fn continuations(&self, auction: &Auction, owner: Seat) -> Option<Vec<(Call, NodeId)>>;  // 接頭辞がシステム外なら None
}

// bridge_system::ir（L3 が読むフィールドのみ）
pub struct Node {
    pub constraint: HandConstraint,        // Custom を含まない
    pub branch_weights: Option<Vec<f32>>,  // 先頭 Or の枝重み（{w:X}）。None は等分
    pub priority: i16,                     // {prio:N}。既定 0
    pub volume_log2: i16,                  // TieBreak::Narrowest 用
    pub side: Side, pub row: RowId, pub description: String, pub call: Call, pub flags: NodeFlags, /* .. */
}
pub enum TieBreak { RowOrder, Narrowest, LowestCall, HighestCall }   // SystemMeta.tie_break、既定 RowOrder

// bridge_system::natural
pub fn classify(auction: &Auction, index: usize, owner: Seat) -> CallContext;   // オークションだけから分類。prior 引数は無い
pub struct Inference { pub constraint: HandConstraint, pub confidence: f32, pub rule: &'static str, pub explanation: String }
impl NaturalInference {
    pub fn infer(&self, ctx: &CallContext) -> Inference;
    pub fn candidates(&self, auction: &Auction, owner: Seat) -> Vec<(Call, HandConstraint, i16)>;
}
```

### 2.1 `Lookup` の読み方

| フィールド | L3 での意味 |
| --- | --- |
| `matched_depth = d` | `key.calls[..d]` は「我々側の全コールに条件（seat / vul）を満たすエントリがある」接頭辞。`d == key.calls.len()` なら Exact |
| `by_depth[i]` | 深さ `i` の我々側ノード。相手コールの深さ、および `d` 以降は `None` |
| `end` | 一致した接頭辞の末尾の `TrieId`。`children(end, cond)` が次の候補 |
| `via_class` | UI 表示用。v1 の重み計算では使わない |

### 2.2 `classify` と解釈済み文脈

計画 §5.7 の `classify(.., prior: Option<&Interpretation>)` は L2 が L3 の型に依存することになり循環する（`bridge-bidding → bridge-system` の一方向依存に反する）ので、`classify(auction, index, owner)` はオークションだけから `CallContext` を作る。`CallContext.partner_constraint: Option<HandConstraint>` と `forcing_situation: bool` は `classify` では `None` / `false` で、L3 が Step A の途中結果（`per_call[..j]`）からパートナーの最終コールの最大重み代替とそのノードの `flags.forcing` を詰めてから `infer` に渡す（`CallContext` は公開フィールドの平易な構造体なので分類後に埋められる）。

### 2.3 双方向一致の契約

`interpret` と `choose_bid` が互いに逆関数であるために、L2 に以下を要求する。違反はプロパティテスト（§8）で検出される。

1. `children(end, opener_pos, vul)` が返す `(call, node)` は、その経路を `resolve` したときの `by_depth` 末尾ノードと一致する。
2. `natural.candidates(auction, s)` が返す `(call, C, prio)` について、`infer(classify(auction.with(call), j, s))` の制約は `C` と同一である（同じ規則表から生成する。`partner_constraint` を埋めても規則の選択は変わらない）。
3. `resolve_lenient` は決定的で、同じ `max_subst` なら同じ順序で返る。

---

## 3. 型

```rust
#[derive(Clone, Debug)]
pub struct Table {
    pub systems: [Arc<SystemIR>; 4],      // 席ごとに異なるシステム（添字 = Seat）。相手は別システムを使う
    pub natural: Arc<NaturalInference>,
}
impl Table {
    pub fn uniform(system: Arc<SystemIR>, natural: Arc<NaturalInference>) -> Table;   // 4 席同一
}
// 席 s のシステムは table.systems[s.index() as usize]。4 席別々なら構造体リテラルで作る。

/// Ord: Exact < Partial < Natural < Fallback（「最も弱い部分」を取るために使う）
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResolutionKind { Exact, Partial { matched_depth: usize }, Natural, Fallback }

#[derive(Clone, Debug)]
pub struct CallExplanation {
    pub call_index: usize,
    pub call: Call,
    pub node: Option<NodeId>,    // Natural / Fallback / 暗黙パスは None
    pub kind: ResolutionKind,
    pub text: String,            // node.description またはナチュラル推定の explanation。Fallback は ""
}

#[derive(Clone, Debug)]
pub struct Explanation {
    pub text: String,                  // parts の text を " / " で連結
    pub node: Option<NodeId>,          // その席の最後のコールのノード
    pub resolution: ResolutionKind,    // parts の kind の最大値
    pub parts: Vec<CallExplanation>,   // その席のコール 1 つにつき 1 要素
}

#[derive(Clone, Debug)]
pub struct CallInterpretation {
    pub call_index: usize,
    pub seat: Seat,
    pub call: Call,
    pub kind: ResolutionKind,                                       // 主要な解釈の種別（防御枝を除く）
    pub alternatives: Vec<(HandConstraint, f32, CallExplanation)>,  // 合計 1
}

#[derive(Clone, Debug)]
pub struct Interpretation {
    pub seats: [Vec<(HandConstraint, f32, Explanation)>; 4],   // 仕様の型。席ごとに合計 1、長さ ≤ K
    pub per_call: Vec<CallInterpretation>,                     // 結合前の生データ（UI・尤度用）
    pub divergence: Option<usize>,                             // 最初に Exact でなくなったコールの index
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct InterpretOptions {
    pub max_alternatives: usize,   // K = 8（D11）
    pub eps_exact: f32,            // 0.02
    pub eps_partial: f32,          // 0.15
    pub eps_natural: f32,          // 0.30
    pub strict: bool,              // true: 全 ε = 0、防御枝なし（プロパティテスト用）
    pub lenient_decay: f32,        // 0.5。置換 1 回あたりの重み減衰 ρ（値は「未決」）
}
// resolve_lenient に渡す置換回数の上限は interpret.rs の定数 LENIENT_MAX_SUBST = 2（オプションにしない）。
```

仕様との差分: 仕様の `Explanation.resolution: Resolution` は `HandConstraint` を含むため、クローンを避けて `ResolutionKind`（`Copy`）に変える。制約そのものは `seats` / `alternatives` のタプル側にある。`lenient_decay` は計画 §6.1 の 5 フィールドへの追加である。暗黙パスの扱いは解釈側にオプションを持たない: システム上の位置で `Pass` が定義されていなければ常に兄弟の補集合で解釈する（§4.1 手順 2、5.1）。`ImplicitPass` は `choose_bid` 側（`BidContext`）だけの方針で、`Never` は「合成せず `NoCandidate` を返す」、`Complement` は「同じ補集合で `Pass` を合成する」を意味する。どちらでも `choose_bid` が返す `Pass` は解釈の補集合を満たすので双方向一致は保たれる。

---

## 4. `interpret(table, auction, opts) -> Interpretation`

呼び出し元は `auction` の全コールを既知とし、L3 は「コール j をした席 s の手」に関する制約を、`s` のシステムだけを使って求める。相手側のコールも同じ手続きで（相手のシステムで）解釈する。

### 4.1 Step A: コール毎の重み付き選言

各 `j in 0..auction.calls().len()` について次を行い、`per_call[j]` を作る。

1. `s = auction.seat_at(j)`, `sys = &table.systems[s.index() as usize]`, `lp = auction.leading_passes()`, `opener_pos = auction.position_of(auction.seat_at(lp))`, `vul = RelVul { we: vulnerability.is_vulnerable(s), they: vulnerability.is_vulnerable(s.next()) }`。
2. **先頭パス**（`j < lp`）: 経路には含まれず `#SEAT` 条件として扱われる（D17）ので、ノードは存在しない。`we_opened = true` のルートに対する `children(root, position_of(s), vul)` のうち合法なコールの制約を集め、`C = Not(Or(...))` を 1 つの代替とする。`kind = Exact`、`node = None`、`text = "no opening bid"`、`ε = eps_exact`。オープニング表が空なら手順 6（Natural）へ落ち、`classify` は `Role::Opener` の `Pass` として `open_pass` 規則に到達する。
3. **キー構築**: `key = LookupKey::for_auction(auction, s)` を取り、`calls` を `..=(j − lp)` に切り詰める（`we_opened` と `opener_pos` は `j ≥ lp` なら切り詰めで変わらない）。`n_k = key.calls.len()`、`lookup = sys.index.resolve(&key)`、`d = lookup.matched_depth`。
4. **Exact**（`d == n_k`）: `node = by_depth[n_k − 1]`。`node.constraint` が先頭 `Or(branches)` なら枝ごとに代替を作り、重みは `branch_weights[i]`（あれば）か `1/len`。そうでなければ `(node.constraint, 1.0)` の 1 代替。`kind = Exact`、`text = node.description`、`ε = eps_exact`。
5. **Partial**（`d < n_k`）: `divergence = min(divergence, j)` を記録し、以下を順に試す。
   1. **暗黙パス**: `d == n_k − 1` かつ `calls[j] == Pass` かつ `children(lookup.end, opener_pos, vul)` が空でない場合、合法な兄弟ノードの制約の補集合 `Not(Or(...))` を 1 代替とする。`kind = Exact`、`node = None`、`ε = eps_exact`。`choose_bid` 手順 (3) と同じ集合を使うので双方向が一致する。
   2. **寛容照合**: `resolve_lenient(&key, LENIENT_MAX_SUBST)` の結果のうち `matched_depth == n_k` のものそれぞれについて、`node = by_depth[n_k − 1]` から手順 4 と同様に代替を作り、重みに `ρ^subst`（`ρ = lenient_decay`）を掛ける。同じ `node` に複数の置換で到達したら重みを合算する。全体を正規化。`kind = Partial { matched_depth: d }`、`ε = eps_partial`。
6. **Natural**（手順 5 で代替が得られない場合）: `ctx = classify(auction, j, s)` を作り、`per_call[..j]` からパートナーの最終コールの最大重み代替を `ctx.partner_constraint` に、そのノードの `flags.forcing ∈ {OneRound, ToGame}` を `ctx.forcing_situation` に詰めてから `inf = table.natural.infer(&ctx)`。代替は `(inf.constraint, 1.0)` の 1 つ。`kind = Natural`、`text = inf.explanation`（`rule` 名を付記）、`ε = eps_natural`。`inf.confidence` は `text` に記録するだけで重みには使わない（ε に畳み込むかは「未決」）。
7. **ε-混合**: 全代替の重みに `(1 − ε)` を掛け、防御枝 `(HandConstraint::ANY, ε, CallExplanation { kind: Fallback, node: None, text: "" })` を末尾に追加する（D15）。`opts.strict` なら ε = 0 とし、防御枝を追加しない。
8. `CallInterpretation { call_index: j, seat: s, call: calls[j], kind, alternatives }` を `per_call` に push する。

手順 5 で `d < n_k − 1` の場合（相手のコールか、我々の以前のコールで既に分岐している場合）は、暗黙パスの補集合は「この位置でシステムが提示する選択肢」を表さないので手順 5.1 を飛ばす。手順 5 の順序（寛容照合 → ナチュラル推定）は計画 §5.5 の「(1) `resolve_lenient`、(2) 残りの我々側コールにナチュラル推定」に従う。

### 4.2 ε-混合の根拠（D15）

低信頼度の解釈を「HCP を 2 広げる」のような場当たりな緩和で表すと、緩め幅に根拠がなく、しかも緩めても支持集合が空になる場合を救えない。ε-混合は、コール毎に「無制約の代替」を重み ε で追加するだけで、次の性質を得る。

| 性質 | 理由 |
| --- | --- |
| 支持集合が空にならない | どの席の選言にも `ANY` の成分が残る（`strict` を除く） |
| 信頼度の差が重みの差になる | Exact 0.02 < Partial 0.15 < Natural 0.30 と、系統的に ε が増える |
| 事後補正できる | サンプラー（09-sample.md §3）は混合からサンプルし、重点重み `L / π` が防御枝から出た配牌を尤度で補正する |
| 検証可能 | `strict` で ε = 0 にすれば、双方向整合性プロパティテストが「制約そのもの」を検証できる |

### 4.3 「以前の制約を弱める」の操作的意味

仕様 §5 のフォールバック階層は「部分一致では直近のビッドのみ解釈し、それ以前は制約を弱める」と述べる。L3 における操作的意味は次の通り。

1. L3 は各コールに **そのノード自身の制約だけ** を使う。祖先ノードの制約は取り込まない（ノードの制約は説明文コンパイラが親連鎖を解決済みであり、AND は Step B で席ごとに行う）。
2. 分岐点（`divergence`）より前のコールは、その時点で Exact だった。ビッダーは当時から自分の手を知っていたので、その言明は今も有効であり、`eps_exact` のまま維持する。「一致済みの部分は弱めない」（計画 §5.5）。
3. 分岐点以降のノードは別経路向けに設計されたものである。寛容照合で得たノードは `eps_partial` と `ρ^subst` で信頼度を下げ、ノードが得られなければナチュラル推定に `eps_natural` を付ける。これが「弱める」の実体である。
4. 計画 §6.2 の「ダメなら `(node.constraint, 1.0)`」の `node`（`d` 以下で最深のノード）は、席 `s` 自身のノードならその index で既に適用済みで AND に何も加えず、別の席のノードなら `s` の手に適用してはならない。したがって L3 は寛容照合が失敗したらナチュラル推定へ進む。Partial ノードの制約を Natural の代わりに再利用するかは「未決」（フェーズ 3.11 の再現率で判断）。

### 4.4 Step B: 席ごとの結合（AND = 直積）

```text
for s in Seat::ALL:
    combos = [(ANY, 1.0, parts = [])]
    for cj in per_call where cj.seat == s (call_index 昇順):
        next = []
        for (C, w, parts) in combos:
            for (Ci, wi, ex) in cj.alternatives:
                C2 = C.and(Ci)                       // ANY は単位元。And は平坦化
                if !C2.is_satisfiable(): continue    // 要約による事前検査のみ（下記）
                next.push((C2, w * wi, parts + [ex]))
        next を (node, kind, branch) 列のキーで重複除去（同キーは重みを合算）
        next を重み降順に整列し K = opts.max_alternatives に切り詰め
        combos = next
    if combos.is_empty():
        combos = [(ANY, 1.0, [])];  tracing::warn!(seat, "seat contradicts itself")
    重みを合計 1 に正規化
    seats[s] = combos.map(|(C, w, parts)| (C, w, Explanation::from_parts(parts)))
```

1. コールのない席は `[(ANY, 1.0, Explanation::empty())]`。
2. `is_satisfiable` は Step B では **要約検査だけ** を使う（`shapes` 空、`hcp` 逆転、`hcp.start > shapes.max_hcp()`、`count` 範囲空）。`Sampler::prepare(...).count() == 0` による決定版（20〜60 μs）は 10 μs 目標に収まらないので呼ばない。
3. 重複除去のキーは `parts` の `(node, kind, branch_index)` 列。`HandConstraint` は `Custom` のため `PartialEq` を持たず、構造的に等しい別ノード由来の代替は重複として扱わない（K で有界）。同じノードに複数の寛容置換で到達した場合だけ合算が起きる。
4. 各ステップで K に切り詰めるので、1 ステップの AND 回数は ≤ K × (そのコールの代替数)。ε-混合では代替数は (枝数 + 1)。
5. `Explanation::from_parts`: `text` は各 part の `text` を `" / "` で連結、`node` は最後の part の `node`、`resolution` は `parts.iter().map(kind).max()`。

### 4.5 `satisfied_by` と `likelihood`

```rust
impl Interpretation {
    /// 厳密判定。Fallback 枝を無視する。forward_consistency が使う
    pub fn satisfied_by(&self, seat: Seat, hand: Hand) -> bool;
    /// Σ_i w_i · 1[C_i ∋ hand]。解釈混合の下での集合所属質量。ビディング方策の尤度ではない
    pub fn likelihood(&self, seat: Seat, hand: Hand) -> f32;
}
// サンプラーは interpretation.seats を直接読む（09-sample.md §6）。専用のアクセサは持たない。
```

| 関数 | 定義 | 計算に使うデータ |
| --- | --- | --- |
| `satisfied_by(s, h)` | `s` の全コール `j` について、`kind != Fallback` かつ `w > 0` の代替 `a_{j,i}` で `a_{j,i}.satisfies(h)` となるものが存在する | `per_call` |
| `likelihood(s, h)` | `Π_{j ∈ calls(s)} Σ_i w_{j,i} · 1[a_{j,i} ∋ h]` | `per_call` |

`satisfied_by` を `per_call` で定義する理由: 席の真の選言は「各コールから 1 代替を選んだ AND の和」であり、`h` がそれに属することと「各コールに `h` を含む代替がある」ことは同値である。`seats` は K で切り詰められているので、`seats` で判定すると枝の多いノード（4 枝 × 3 コール = 64 組合せ > 8）で偽の違反が出る。`likelihood` も同じ理由で積の形（切り詰め前の質量と一致し、正規化も不要）で計算する。`seats` は提案分布の構築（09-sample.md §6）に使い、切り詰めの影響は重点重みが補正する。

### 4.6 性能（目標 < 10 μs）

| 段階 | 見積り（12 コール） | 対策 |
| --- | --- | --- |
| Step A の `resolve` | 12 回 × 深さ ≤ 12 × 30 ns ≈ 4.3 μs（最悪） | `for_auction` は 1 席 1 回、`calls` の切り詰めはスライス操作のみ。超過時は側ごとの逐次照合（2 回）に変更 |
| Step A の代替構築 | 代替 ≤ 3/コール、`SmallVec<[_; 8]>` | ヒープ割当なし |
| Step B の AND | 4 席 × 3 コール × ≤ 16 回 ≈ 200 回 × 30 ns ≈ 6 μs | `Atom ∧ Atom` は `ShapeSet`（9 × u64）の AND と区間クランプ。`cards`/`eval` が空なら割当なし。要約検査を AND の前に行う |
| `classify` + `infer` | Natural のコールだけ | 規則表は述語の順次評価 |

計測は criterion で 12 コールのオークション（フェーズ 3.6 のベンチ `interpret_bench`）。

---

## 5. `choose_bid` と `BidChoice`（D6）

### 5.1 型

```rust
pub enum BidChoice { Chosen(Chosen), NoCandidate(NoCandidate) }   // NoCandidate はエラーではない（仕様 §9）

pub struct Chosen {
    pub call: Call,
    pub node: Option<NodeId>,            // Natural / ImplicitPass は None
    pub source: ChoiceSource,
    pub explanation: String,
    pub alternatives: Vec<Alternative>,  // 合法かつ充足した全候補。先頭が選択されたもの
    pub diagnostics: Vec<Diagnostic>,
}
pub enum ChoiceSource { System, Natural, ImplicitPass }
pub struct Alternative { pub call: Call, pub node: Option<NodeId>, pub priority: i16 }

pub struct NoCandidate {
    pub tried: Vec<Tried>,               // 全候補と却下理由
    pub diagnostics: Vec<Diagnostic>,
}
pub struct Tried { pub node: NodeId, pub call: Call, pub reason: Rejected }   // 却下はシステムのノードにしか起きない
pub enum Rejected { Unsatisfied, Illegal, NotApplicable /* seat / vul 条件 */ }

pub enum Diagnostic {
    IllegalSystemCall { node: NodeId, call: Call },                   // lint。パニックしない
    DuplicateCandidate { node_a: NodeId, node_b: NodeId, call: Call },
    UnsatisfiableNode { node: NodeId },
}
impl BidChoice { pub fn call(&self) -> Option<Call>; pub fn is_chosen(&self) -> bool; }

#[derive(Clone, Copy)] pub enum Scoring { Imp, Mp, Total }
#[derive(Clone, Copy)] pub enum ImplicitPass { Never, Complement }
#[derive(Clone, Copy)] pub struct PolicyParams { pub temperature: f32 /* 1.0 */, pub epsilon: f32 /* 1e-3 */ }
pub struct BidContext<'a> {
    pub scoring: Scoring,                        // v1 では素通し（L2 の条件に scoring がない。「未決」: #+SCORING 条件）
    pub natural: Option<&'a NaturalInference>,   // 接頭辞がシステム外のときのフォールバック
    pub implicit_pass: ImplicitPass,             // テストの既定 Never、アプリの既定 Complement
    pub policy: PolicyParams,
}

pub fn choose_bid(system: &SystemIR, hand: Hand, auction: &Auction, ctx: &BidContext) -> BidChoice;
```

### 5.2 手順

`seat = auction.next_seat()`、`opener_pos = auction.position_of(seat)`（まだ誰もビッドしていなければ自分がオープナー）または `position_of(seat_at(leading_passes()))`、`vul = RelVul { we, they }`。

1. **候補集合**
   1. `key = LookupKey::for_auction(auction, seat)`。`None`（まだ誰もビッドしていない）なら `LookupKey { we_opened: true, calls: &[], opener_pos: position_of(seat), vul }` を直接作る。
   2. `lookup = resolve(&key)`。`matched_depth == key.calls.len()` なら `candidates = children(lookup.end, opener_pos, vul)`、`source = System`（`SystemIR::continuations(auction, seat)` はこの 2 手順をまとめた便宜関数で、接頭辞がシステム外なら `None`）。
   3. そうでなければ `resolve_lenient(&key, LENIENT_MAX_SUBST)` のうち完全一致した最初の（置換回数最小の）`Lookup` の `end` から `children`。`source = System`。
   4. それも無く `ctx.natural` が `Some` なら `natural.candidates(auction, seat)`、`source = Natural`。`None` なら候補は空。
2. **フィルタ**: 各候補 `(call, node)` について、`!auction.is_legal(call)` なら `Tried { reason: Illegal }` と `Diagnostic::IllegalSystemCall`（システム定義の lint。パニックしない）。`!node.constraint.satisfies(hand)` なら `Tried { Unsatisfied }`。`node.constraint` が要約検査で充足不能なら `Diagnostic::UnsatisfiableNode` も付ける。残りを `kept` とする。
3. **暗黙パス**（`ctx.implicit_pass == Complement`）: `kept` にも候補集合にも `Pass` がなく、`Pass` が合法なら、合法な兄弟候補の制約の `Not(Or(...))` を制約とする `Pass` を合成し、`priority = i16::MIN + 1`、`source = ImplicitPass`、`node = None`。`hand` が補集合を満たすときだけ `kept` に加える。解釈側（§4.1 手順 2、5.1）も同じ集合の補集合を使うので双方向が一致する。兄弟が全手を覆う場合は補集合が充足不能で `Pass` は合成されない（カバレッジレポートは `ImplicitPass` を真の穴と分けて集計する）。
4. **整列**: `priority` 降順。同点は `system.meta.tie_break`（下表）。決定的。
5. **結果**: `kept` が空なら `NoCandidate { tried, diagnostics }`。そうでなければ `Chosen { call: kept[0].call, alternatives: kept 全体, .. }`。

| `TieBreak` | 規則 |
| --- | --- |
| `RowOrder`（既定） | `node.row` 昇順（BML の「最初の定義が勝つ」） |
| `Narrowest` | `node.volume_log2` 昇順（制約体積が小さい方） |
| `LowestCall` | `Call` 昇順（`Pass < Double < Redouble < Bid`） |
| `HighestCall` | `Call` 降順 |

`Rejected::NotApplicable` は seat / vul 条件で弾かれた兄弟を診断用に記録するための予約で、v1 の `children` は条件一致の候補しか返さないため `tried` には現れない（条件無視の列挙 API を L2 に追加するかは「未決」）。`DuplicateCandidate` は同じコールを持つ候補が 2 つ以上あるとき（ナチュラル候補、あるいは Exact 辺と Class 辺の双方から到達）に付け、`choose_bid` は `priority` と `tie_break` で 1 つを選び、`call_distribution` は両方を質量に数える。

---

## 6. 確率的方策・尤度・再生

### 6.1 `call_distribution`

```rust
pub fn call_distribution(system: &SystemIR, hand: Hand, auction: &Auction, ctx: &BidContext) -> Vec<(Call, f32)>;
```

1. `kept` = §5.2 手順 1〜3 の結果。
2. `kept` に現れる相異なるコール `c` ごとに `score(c) = logsumexp_{node ∈ kept, node.call == c}(priority / τ)`（`τ = ctx.policy.temperature`）。同じコールに 2 つの意味が合致すれば質量が増える。
3. `softmax(c) = exp(score(c) − LSE(score))`。
4. `legal = auction.legal_calls()`（≤ 38）。`ε = ctx.policy.epsilon`。`p(c) = (1 − ε) · softmax(c) + ε / |legal|`。`kept` が空なら `p(c) = 1 / |legal|`。全合法コールが正の確率を持つので重みが 0 になるサンプルは出ず、候補外のコールのコストは `ln ε`。
5. `τ → 0` で `choose_bid` と一致する。テスト: `τ = 0.01`、10^5 局面で `argmax_c p(c)`（同点は `tie_break` で解消）が `choose_bid(...).call()` と等しい。

### 6.2 `sequence_log_likelihood`

```rust
pub fn sequence_log_likelihood(table: &Table, deal: &Deal, auction: &Auction, ctx: &BidContext) -> f64;
// = Σ_j ln p_j(calls[j])、p_j = call_distribution(&table.systems[seat_j], deal.hand(seat_j), auction[..j], ctx)
```

各 `j` で接頭辞 `auction[..j]` を対象に `call_distribution` を呼ぶ。`ctx.natural` が `None` でも、`table.natural` を補って呼ぶ。コスト ≈ コール数 × 候補数 × `satisfies` ≈ 2〜5 μs / 配牌。これが 09-sample.md の `ln L` の第 1 項である。

### 6.3 `replay`

```rust
pub struct Replay { pub auction: Auction, pub gaps: Vec<(usize, Seat)>, pub diagnostics: Vec<Diagnostic> }
pub fn replay(table: &Table, deal: &Deal, dealer: Seat, vul: Vulnerability, ctx: &BidContext) -> Replay;
```

`auction.is_complete()` まで: `seat = auction.next_seat()`、`choose_bid(&table.systems[seat.index() as usize], deal.hand(seat), &auction, ctx)`。`Chosen` は push、`NoCandidate` は `Pass` を push して `gaps` に `(index, seat)` を記録。診断は連結。合法性が終了を保証するが、安全のため 320 コールで打ち切る。再現率テスト（§8）と `xtask coverage` が使う。

### 6.4 `InterpretCache`

```rust
#[derive(Default)]
pub struct InterpretCache { map: HashMap<(Seat, Vulnerability, Vec<Call>), Arc<Interpretation>> }
impl InterpretCache {
    pub fn new() -> Self;
    pub fn get_or_interpret(&mut self, table: &Table, auction: &Auction, opts: &InterpretOptions) -> Arc<Interpretation>;
    pub fn len(&self) -> usize;  pub fn is_empty(&self) -> bool;
}
```

`SystemIR` は不変でキャッシュを持たない（仕様 §9）。キャッシュは呼び出し側が所有する。キーはオークションだけなので、`opts` と `Table` を変えるときは新しいキャッシュを作る（呼び出し側の規約。クリアは値を作り直す）。

---

## 7. モジュール構成

| ファイル | 内容 |
| --- | --- |
| `lib.rs` | 再エクスポート（`bridge_system::{NaturalInference, NodeId, SystemIR}` を含む）、crate doc、`Table`、`Scoring`、`ImplicitPass`、`BidContext` |
| `interpret.rs` | 型（§3）、Step A / Step B、`satisfied_by` / `likelihood`、`Explanation::from_parts`、`LENIENT_MAX_SUBST` |
| `choose.rs` | `BidChoice` 系の型、`choose_bid`、合法性 lint、`priority` / `tie_break`、暗黙パス |
| `policy.rs` | `PolicyParams`、`call_distribution`、`sequence_log_likelihood`、`logsumexp` |
| `replay.rs` | `Replay`、`replay` |
| `cache.rs` | `InterpretCache` |
| `benches/interpret.rs` | criterion（12 コールの `interpret`、`sequence_log_likelihood`） |

---

## 8. テストと性能目標

| テスト | 場所 | 種類 | 基準 |
| --- | --- | --- | --- |
| `forward_consistency`（仕様 §10） | `tests/consistency.rs`、`#[ignore]`、release | 10^6 のランダム (hand, auction 接頭辞)、`InterpretOptions { strict: true, .. }`。`Chosen` なら `interpret(auction.with(call)).satisfied_by(seat, hand)` | 違反 0。`NoCandidate` と `ImplicitPass` はノード別に集計し `target/coverage_report.json`（上位 50 の穴） |
| `reproduction_rate` | 同ファイル | コーパスの 500 オークション × `sample_deals(1000)` → `replay == auction` の率 | ノード別に報告。フェーズ 4 で中央値 ≥ 0.6 |
| `policy_argmax_matches_choose_bid` | `tests/policy.rs` | `τ = 0.01`、10^5 局面 | 100% |
| `illegal_call_is_lint` | unit | 不正な継続を含む合成 `SystemIR` | `Diagnostic::IllegalSystemCall` が返り、パニックしない |
| `weights_sum_to_one` | unit | 各席の `seats[s]` と各 `per_call` の重み | 合計 1（許容 1e-5） |
| `and_combination_drops_contradictions` | unit | 矛盾する 2 コール | 該当組合せが消え、残りが正規化される。全滅なら `ANY` + warn |
| `partial_and_natural_epsilon` | unit | 意図的に切り詰めたシステム | 分岐以降が Partial / Natural になり、`divergence` が正しい。`strict` で防御枝が消える |
| `implicit_pass_bidirectional` | unit | `Complement` で `Pass` を選んだ手 | 解釈の補集合を満たす |
| `interpret_bench` | `benches/` | criterion、12 コール | < 10 μs |
| コーパス解決率（フェーズ 4、`xtask coverage`） | `--ignored` | 実ハンドレコードのオークション | ≥ 80% が全コール Exact、残りに `EmptySupport` なし |

実装順（計画 §12 フェーズ 3）: 3.6 型 + `interpret`（Exact のみ）+ ベンチ → 3.7 `choose_bid` → 3.8 `call_distribution` / `sequence_log_likelihood` → 3.9 `replay` → 3.10 `forward_consistency` + カバレッジレポート → 3.11 Partial / Natural + ε-混合 + `NaturalInference` 接続 → 3.12 再現率ハーネス。

---

## 9. 未決事項

| # | 項目 | 現在の仮置き |
| --- | --- | --- |
| 1 | `lenient_decay` ρ と `LENIENT_MAX_SUBST` の値 | 0.5 / 2。フェーズ 3.11 の再現率で調整 |
| 2 | `Inference.confidence` を `eps_natural` に畳み込むか | 畳み込まない（`text` に記録のみ） |
| 3 | 寛容照合が失敗したとき Partial ノードの制約を再利用するか | しない（Natural へ） |
| 4 | `BidContext.scoring` を L2 の条件（`#+SCORING`）に接続する時期 | v1 は素通し |
| 5 | seat / vul 条件で弾かれた兄弟を `Tried { NotApplicable }` に載せるための L2 API | 未追加 |

`classify` に解釈済み文脈を渡す方法は §2.2 のとおり `CallContext.partner_constraint` / `forcing_situation` を L3 が後から埋める形で確定した。
