# 07. L3 `bridge-bidding`: 解釈器と生成器

本書は L3 `bridge-bidding` の詳細設計である。`interpret` は各コールを「コール毎の重み付き選言」に分解し（Step A）、席ごとに直積で結合して上限 K=8 の選言に切り詰める（Step B、D11）。低信頼度の解釈は制約を緩めるのではなく ε-混合で表現し（D15）、`choose_bid` は `BidChoice` enum（D6）で「候補なし」をエラーではなく戻り値として返す。この層は `SystemIR` と `NaturalInference` を読むだけで、独自の判断ロジックを持たない。

フェーズ 4 で、方策と解釈を次のように改めた（D18、D19）。

- 方策 `call_distribution` は「システムの決定的選択 + ナチュラルへの逸脱 δ + 一様床 ε」にした。温度付きソフトマックスは廃止した。
- `interpret` はこの方策を写したもの（方策鏡像）にした。コール c の解釈の密度 Σ_i w_i·1[h ∈ C_i] は、その位置での p(c | h) にコールごとの定数倍で一致する。
- システムコールの排他領域は、`bridge-system` の `ExclusiveIndex` に派生データとして前計算する。直列化はしないので、`IR_FORMAT` は変えない。
- 以下で「ε-混合」「`eps_*`」と書いた箇所は、特に断らない限り旧経路 `InterpretMode::Legacy` の説明である（§4.2）。

関連文書: 05-constraint.md（`HandConstraint`、`Sampler`、`KnownCards`、`HcpShapeGrid`）、06-system.md（`AuctionTrie`、`NaturalInference`、`ExclusiveIndex`、`Lint`）、09-sample.md（尤度の消費側）、11-testing.md、13-decisions.md（D6、D11、D15、D18、D19、D20）。

---

## 1. 位置づけと責務境界

| 項目 | 内容 |
| --- | --- |
| クレート | `bridge-bidding`（lib 名 `bridge_bidding`） |
| 依存 | `bridge-core`、`bridge-constraint`、`bridge-system`（経由で `bridge-eval`）。`NaturalInference`、`NodeId`、`SystemIR` を再エクスポートする |
| 外部依存 | `smallvec`、`tracing`、`serde`（optional）。エラー型が無いので `thiserror` は使わない |
| feature | `default = ["std"]`、`std`、`serde = ["dep:serde", ..]`（下位の `serde` を伝播） |
| wasm32 | 可（外部依存はすべて wasm-safe） |
| 主要 API | `interpret`、`choose_bid`、`call_distribution`、`sequence_log_likelihood`、`AuctionPolicy`、`replay`、`InterpretCache` |

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

フェーズ 4 で次を追加した。

- `Lookup.parent: TrieId`：`matched_depth − 1` のトライノード。`resolve` のループの中で追跡するので追加コストはない。`interpret` は、いま一致したコールの兄弟集合を引くのに使う（厳密一致と、`resolve_lenient` の各試行の両方）。
- `bridge_system::exclusive`
  - `rank_cmp(sys, a, b)`：システム候補の全順序。**priority 降順 → `SystemMeta::tie_break` → コール index 昇順** の順に比べる。`choose_bid` の整列、`ExclusiveIndex`、ナチュラル候補の順位付けは、すべてこの 1 つの比較関数を使う。以前は安定ソートが暗黙に最後の比較を担っていたが、それを明示した。
  - `subtract(base, minus[]) -> HandConstraint`：base ∧ ¬(∪ minus) を計算する。
    - 原子レベルの厳密な差集合で、`Atom::negate` の素な連鎖を使う。
    - 結果は素な原子の平坦な `Or` で、上限は 48 原子。
    - 上限を超えると木 `And([base, Not(Or(minus))])` に退避する（集合としては同じ）。
  - `SystemIR::exclusive(&self) -> &ExclusiveIndex`：`OnceLock` に入れた派生索引。
    - `compile()` の最後に先行して構築する。
    - 直列化から復元した IR や手組みの IR では、初回アクセス時に構築する。
    - `#[serde(skip)]` なので、直列化形式と `IR_FORMAT` は変わらない。
- `ExclusiveIndex { keys: Vec<(TrieId /*親*/, u8 /*条件クラス*/, u32 /*グループ*/)>, groups: Vec<ExclusiveGroup> }`
  - 条件クラスは `(opener_pos−1) | we<<2 | they<<3` の 16 通り。これは `AuctionTrie::children` が席・バル条件で絞る単位と同じなので、1 グループは `choose_bid` の兄弟集合そのものになる。
  - `ExclusiveGroup { members /* rank 順 */, per_call: Vec<(Call, Vec<ExclusivePiece>)>, complement }`
  - `ExclusivePiece { node, branch, constraint /* 平坦な Or か木 */, summary }`
  - 同じ兄弟集合を持つグループは、(call, node) を rank 順に並べた列をキーに重複除去する。
- `NaturalInference::ranked_candidates(auction, owner)`（順位順、§5.2）と `infer_batch`。`infer_batch` は `classify` の共通部分を 1 回にまとめ、説明文字列を作らない。

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
4. **逆方向（締まり）**：`choose_bid` が到達する位置で、h が c の非 Fallback 片（システム片 X_c）に入るなら、`choose_bid` は c を選ぶ。前向き（選んだなら入る）と合わせると、X_c は「方策が c を選ぶ手の集合」に一致する。
5. **鏡像**：各位置 P と各合法コール c について、`interpret` の片の密度 D_c(h) = Σ_i w_i·1[h ∈ C_i] は、`call_distribution` の p(c | h) の定数倍に一致する。
   - 定数 `exp(log_scale)` は、コールごとに `CallInterpretation` に記録する。
   - リテラル（cards/eval）を持つ片は、上側近似（sup）でしか表せない場合がある。そのときだけ D ≥ p を許す。
   - D が p を下回ること（under-cover）は常に禁止する。

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
    pub alternatives: Vec<(HandConstraint, f32, CallExplanation)>,  // 合計 1（鏡像の「片」）
    pub log_scale: f64,     // ln Σ raw。p(call | h) = exp(log_scale)·Σ_i w_i·1[h ∈ C_i]（Legacy は 0.0）
    pub shadowed: bool,     // 方策がこの位置でこのコールを決して選ばない（X も Y も空）。Legacy は常に false
}

#[derive(Clone, Debug)]
pub struct Interpretation {
    pub seats: [Vec<(HandConstraint, f32, Explanation)>; 4],   // 仕様の型。席ごとに合計 1、長さ ≤ K
    pub per_call: Vec<CallInterpretation>,                     // 結合前の生データ（UI・尤度用）
    pub divergence: Option<usize>,                             // 最初に Exact でなくなったコールの index
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum InterpretMode { #[default] Mirror /* 方策鏡像（D19） */, Legacy /* フェーズ 3 の ε-混合。1 フェーズだけ残す */ }

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct InterpretOptions {
    pub max_alternatives: usize,   // K = 8（D11）
    pub strict: bool,              // true: Fallback 片を落とす（プロパティテスト用）
    pub mode: InterpretMode,       // 既定 Mirror
    pub policy: PolicyParams,      // 鏡像が写す方策（Legacy では無視）
    pub implicit_pass: ImplicitPass, // 鏡像が写す暗黙パス規則（Legacy では無視）。既定 Complement
    pub eps_exact: f32,            // Legacy のみ。0.02
    pub eps_partial: f32,          // Legacy のみ。0.15
    pub eps_natural: f32,          // Legacy のみ。0.30
    pub lenient_decay: f32,        // Legacy のみ。0.5。置換 1 回あたりの重み減衰 ρ
}
impl InterpretOptions {
    pub fn for_context(ctx: &BidContext<'_>) -> Self;  // policy と implicit_pass を ctx から取る。残りは既定
    pub fn legacy() -> Self;                            // mode = Legacy、ε は既定値
}
// Default は system_players() の鏡像、implicit_pass = Complement、K = 8、strict = false。
// resolve_lenient に渡す置換回数の上限は interpret.rs の定数 LENIENT_MAX_SUBST = 2（オプションにしない）。
```

仕様との差分:
- 仕様の `Explanation.resolution: Resolution` は `HandConstraint` を含む。クローンを避けるため、`ResolutionKind`（`Copy`）に変えた。制約そのものは `seats` / `alternatives` のタプル側にある。
- `lenient_decay` は計画 §6.1 の 5 フィールドへの追加である。

暗黙パスの扱い:
- **Legacy**：解釈側にオプションを持たない。システム上の位置で `Pass` が定義されていなければ、常に兄弟の補集合で解釈する（§4.1.1 手順 2、5.1）。
- **Mirror**：`implicit_pass` を方策と同じ値にする（`for_context`）。
  - `Complement` では、補集合が X_Pass になる。
  - `Never` では、方策に暗黙パスが無い。補集合は N^sys（どのシステム候補も満たさない手）として一様に読まれる。
- `ImplicitPass::Never` は `choose_bid` では「合成せず `NoCandidate` を返す」、`Complement` は「同じ補集合で `Pass` を合成する」を意味する。
- どちらの場合も、`choose_bid` が返す `Pass` は解釈の片に入るので、双方向一致は保たれる。

---

## 4. `interpret(table, auction, opts) -> Interpretation`

呼び出し元は `auction` の全コールを既知とし、L3 は「コール j をした席 s の手」に関する制約を、`s` のシステムだけを使って求める。相手側のコールも同じ手続きで（相手のシステムで）解釈する。

### 4.1 Step A: 方策鏡像（`InterpretMode::Mirror`、既定）

各コール j（位置 P = 接頭辞 `auction[..j]`、観測コール c、席 s、合法コール数 n）について、次の表の「片」を作る。p(c | h) はこれらの片の上で一定になる。

- 重みは生の値で計算し、`log_scale = ln Σ raw` を記録してから合計 1 に正規化する。
- したがって p(c | h) = exp(log_scale)·Σ_i w_i·1[h ∈ C_i] が成り立つ。

| 片 | 集合 | 生の重み | kind |
| --- | --- | --- | --- |
| X_c^(b) | システム排他領域の枝 b（下記） | (1−ε)(1−δ) | Exact / Partial |
| N^sys | P のシステム候補を 1 つも満たさない手。`complement` で、`ImplicitPass::Never` や明示的な `Pass` 行があるときだけ空でない | (1−ε)(1−δ)/n | Fallback |
| Y_c | ナチュラル排他領域（下記） | システム内なら (1−ε)δ、システム外なら (1−ε) | Natural |
| N^nat | ナチュラル候補もナチュラル暗黙 Pass も満たさない手 | システム内なら (1−ε)δ/n、システム外なら (1−ε)/n | Fallback |
| ANY | 全手 | ε/n | Fallback |

片の取捨:
- δ = 0 のときは Y_c と N^nat を作らない。システム内の位置ではナチュラル候補を列挙しない。
- 空の片は落とす。
- `opts.strict` のときは Fallback の片を落とす。前向き整合性は X_c と Y_c だけで判定する。

**システム内の位置**：`choose_bid` と同じ判定を使う（§5.2 手順 1）。厳密一致、または寛容照合の最初の完全一致の位置に、合法な子が 1 つ以上あればシステム内である。位置の列挙は `choose.rs` の `enumerate_position` を `choose_bid`・`call_distribution`・`interpret` で共有する。したがって、3 者が別々に候補を作ることはない。

**システム排他領域**：位置 P の兄弟グループ G は、`lookup.parent` と条件クラスで索引を引いて得る。コール c の領域は次の式で定める。

  X_c = ∪_{m ∈ G, call(m) = c} ( C_m ∧ ¬ ∪_{m' ∈ G, m' が m より上位, call(m') ≠ c} C_{m'} )

これを、ノードの先頭の `Or` の枝ごとに作る。

- 同じノードの枝どうしは、索引の構築時に素にする（b_i ← b_i ∧ ¬∪_{k<i} b_k）。
  - したがって片は重ならず、密度は正確に (1−ε)(1−δ) になる。
  - 素にできない（原子の上限を超えた）ときだけ木にする。
- BML の `{w:}` 枝重みは、提案の密度としては使わない。方策は枝を区別しないからである。枝重みは説明文にだけ残す。
- 上位の兄弟のコールが接頭辞の後で非合法な場合（ワイルドカード部分木、寛容照合の位置など）は、前計算した片を使わない。合法な兄弟だけから、実行時に同じ厳密な差集合（`subtract`）で計算し直す。
- 寛容照合の位置では、`choose_bid` が使う「置換回数最小の最初の完全一致」だけを使う（kind = Partial、重みは Exact と同じ）。他の寛容照合と `ρ^subst` は方策に無いので捨てる。
- システム片は索引から借用する（`Cow::Borrowed`）。したがって、再計算の場合を除いて割当はない。

**方策上選ばれないコール（shadowed）**：X_c も Y_c も空になるコールは、`CallInterpretation.shadowed = true` とし、Fallback の片（N^sys、N^nat、ANY）だけで読む。

- そのコールがあっても、尤度は（N 領域を除いて）手に依らない。したがって、これが方策どおりの読みである。
- ノード全体で読み直す案は採らない。プロトタイプ C でコーパス ESS を 0.35 から 0.15 に下げたからである。
- 説明文には元のノードやナチュラル規則の文を残し、「方策上は選ばれない」と付記する。

**先頭パスと暗黙パス**：ルート（またはその位置）のグループの `complement` を、そのまま X_Pass とする（`implicit_pass = Complement` のとき）。明示的な `Pass` 行があるときは、それを通常の member として扱う。

**システム停止のパス（`06-system.md` §4.5）**：停止の後の我々の手番では、L2 の `children` が停止のパス（`Pass`、priority −100、制約 `ANY`、`flags.stop`）を返す。L3 はこれを特別扱いしない。フェーズ 4 の SAYC がパスの鎖で書いていた `P = {prio:-100} any hand` の行と同じく、ふつうのシステム候補（member）として扱う。

- 候補：`enumerate_position` の手順 1 で他の子と並ぶ。最下位の候補で、`choose_bid` は他の合法な候補がどれも満たされないときだけ選ぶ（`source = System`、`node` は合成ノード）。`Pass` が候補にあるので、暗黙パス（`ImplicitPass`）は合成しない。
- 鏡像：X_Pass は `ANY` から上位の全メンバーを引いたもの（kind = Exact）で、索引が前計算する。グループの `complement` は空なので N^sys の片は無い。上位の兄弟が非合法なときの実行時の再計算も、同じ `subtract` を使う。
- 照合：`Lookup.by_depth` には合成ノードが入る（`Node::is_synthesised()`：`flags.synthesised`）。停止の後に我々がパス以外をコールすると、照合はその手前で止まり、以後はナチュラルになる。相手の具体的なコールに表があれば、そちらが優先される。
- 説明：`BidChoice.explanation`、`interpret` と排他領域の説明文は `Node::description` をそのまま使う。`compile()` がノードの説明文から `{prio:N}`・`{w:X}`・`{stop}` の注釈を 1 度だけ取り除いて格納する（注釈の内容は `priority`・`branch_weights`・`flags` にあり、書かれたままの文は `Row::description_raw` に残る）。呼ぶたびに取り除くより `interpret` のベンチで数 % 速い。合成ノードの説明は、それが表す行 `{prio:-100} {stop} any hand` と同じく注釈を除いたものなので、`{stop}` の行でも合成された停止のパスでも、説明は `any hand` になる。
- 集計：`xtask coverage` の strict [G] は、行が priority ≤ −100 のパスだけの位置を「既定パスだけの位置」として数える。停止のパスもこれに含まれるので、strict 集計は鎖のときと同じ規則で働く（`generated.system_stop_passes` は合成された停止のパスの選択数）。
- 等価性：接ぎ木を鎖と同じ 6 巡に制限した試験版では、生成したリプレイ 24,269 位置で `choose_bid` の選択、`call_distribution`（`system_players()` と (ε, δ) = (0.3357, 0.335)）、コールごとの `interpret` の尤度が、鎖版とすべて一致した（|Δ ln p| の最大 0.0）。停止は際限が無いので、実際の版では 6 巡を超える位置だけが変わる（24,266 位置のうち `choose_bid` の結果の違い 22、分布の違い 2。例：`1D P 1H P 1NT P 2S P 2NT P 3NT P 4H P 5D` の後、鎖は 6 巡で尽きてナチュラルが答え、停止はシステムのパスを出す）。

**ナチュラル排他領域**：ナチュラル候補を順位順に c_1, c_2, … とし、C_k = `infer(classify(prefix.with(c_k)))` とする。

  Y_c = C_c ∧ ¬ ∪_{k が c より上位} C_k

- `implicit_pass = Complement` のとき、Pass はナチュラル暗黙パスとして次の集合になる。
  (Pass 規則 ∨ ¬∪ 下位の候補) ∧ ¬∪ 上位の候補
  これは、Pass が候補に並んでいる場合も同じである。
- 計算は `bridge_constraint::HcpShapeGrid`（560 形 × HCP 0..=37 の厳密な集合）で行い、次の 2 つの形を持つ。
  - **提案形**（平坦な Or）：自分の側は sup で取る。上位の候補の側は sub で引く。上位の候補がリテラルを持つと上側近似にしかならない。自分の cards/eval リテラルは連言として付ける。
  - **所属判定形**（木）：`And(C_c, Not(Or(上位)))`。集合として厳密で、尤度（§6.2）に使う。提案形が厳密なら作らない。
- リテラルを持たない原子（ナチュラル推定のほぼ全部）は、HCP 範囲ごとに 1 つの箱にまとめてから差し引く（`union_sub`）。候補ごとにグリッドを作ることはない。
- `infer` は、バッチ API `infer_batch` で合法コールの分だけ呼ぶ。`classify` の共通部分を 1 回にまとめ、説明文字列は作らない。説明文は、そのコールの説明が実際に要るときにだけ作る。

**パートナー文脈**：`ctx.partner_constraint` には、パートナーの直前のコールの読みの要約を入れる。

- 読みは、そのコールのシステム領域の非 Fallback 片の和、システム領域が無ければナチュラル推定である。
- 要約は、シェイプの和と HCP の包を持つ 1 原子である。
- `choose_bid` と `interpret` は同じ `partner_context(prefix)` を呼ぶ（`exclusion.rs` の `Reader`）。
- 読みは遅延計算で、ナチュラルの計算が実際に必要とするパートナーのコールの連鎖だけを読む。
- ナチュラル排他は要約を変えないので、この経路では計算しない。

#### 4.1.1 旧 Step A（`InterpretMode::Legacy`、1 フェーズだけ残す）

フェーズ 3 の手順である。`InterpretOptions::legacy()` で選ぶ。各 `j in 0..auction.calls().len()` について次を行い、`per_call[j]` を作る。`log_scale` は 0、`shadowed` は false になる。

1. `s = auction.seat_at(j)`, `sys = &table.systems[s.index() as usize]`, `lp = auction.leading_passes()`, `opener_pos = auction.position_of(auction.seat_at(lp))`, `vul = RelVul { we: vulnerability.is_vulnerable(s), they: vulnerability.is_vulnerable(s.next()) }`。
2. **先頭パス**（`j < lp`）: 経路には含まれず `#SEAT` 条件として扱われる（D17）ので、ノードは存在しない。`we_opened = true` のルートに対する `children(root, position_of(s), vul)` のうち合法なコールの制約を集め、`C = Not(Or(...))` を 1 つの代替とする。`kind = Exact`、`node = None`、`text = "no opening bid"`、`ε = eps_exact`。ただしルートに明示的な `Pass` 行がある場合は補集合を作らず、その `Pass` ノードを手順 4 と同様に使う (`choose_bid` もその行を候補として出すので双方向が一致する。以前は `Pass` 行自身も補集合に含めていた)。オープニング表が空なら手順 6（Natural）へ落ち、`classify` は `Role::Opener` の `Pass` として `open_pass` 規則に到達する。
3. **キー構築**: `key = LookupKey::for_auction(auction, s)` を取り、`calls` を `..=(j − lp)` に切り詰める（`we_opened` と `opener_pos` は `j ≥ lp` なら切り詰めで変わらない）。`n_k = key.calls.len()`、`lookup = sys.index.resolve(&key)`、`d = lookup.matched_depth`。
4. **Exact**（`d == n_k`）: `node = by_depth[n_k − 1]`。`node.constraint` が先頭 `Or(branches)` なら枝ごとに代替を作り、重みは `branch_weights[i]`（あれば）か `1/len`。そうでなければ `(node.constraint, 1.0)` の 1 代替。`kind = Exact`、`text = node.description`、`ε = eps_exact`。`by_depth[n_k − 1] == None`（我々の `Pass` が、より深い行の経路だけが作った暗黙パスのトライノードに着地した）場合は、1 コール短いキーを解決し直した **親** 位置の合法な兄弟の補集合を暗黙パスとする（手順 5.1 と同じ意味）。兄弟が無ければ `d = n_k − 1` として手順 5 に進むが、手順 5.1 は飛ばす: `lookup.end` は我々のパスの **後** の位置で、その子は相手の次のコールだから、その補集合を我々のパスの意味にしてはならない。
5. **Partial**（`d < n_k`）: `divergence = min(divergence, j)` を記録し、以下を順に試す。
   1. **暗黙パス**: `d == n_k − 1` かつ `calls[j] == Pass` かつ `children(lookup.end, opener_pos, vul)` が空でない場合、合法な兄弟ノードの制約の補集合 `Not(Or(...))` を 1 代替とする。`kind = Exact`、`node = None`、`ε = eps_exact`。`choose_bid` 手順 (3) と同じ集合を使うので双方向が一致する。
   2. **寛容照合**: `resolve_lenient(&key, LENIENT_MAX_SUBST)` の結果のうち `matched_depth == n_k` のものそれぞれについて、`node = by_depth[n_k − 1]` から手順 4 と同様に代替を作り、重みに `ρ^subst`（`ρ = lenient_decay`）を掛ける。同じ `node` に複数の置換で到達したら重みを合算する。合計が 1 を超えるときだけ 1 に正規化し、1 未満ならそのまま残す（不足分 `1 − Σw` は手順 7 で防御枝に回る）。以前は常に正規化していたため、ノードが 1 つだけの通常の場合に `ρ^subst` が打ち消され、`lenient_decay` が効いていなかった。`kind = Partial { matched_depth: d }`、`ε = eps_partial`。
6. **Natural**（手順 5 で代替が得られない場合）: `ctx = classify(auction, j, s)` を作り、`per_call[..j]` からパートナーの最終コールの最大重み代替を `ctx.partner_constraint` に、そのノードの `flags.forcing ∈ {OneRound, ToGame}` を `ctx.forcing_situation` に詰めてから `inf = table.natural.infer(&ctx)`。パートナーのそのコールの後に相手が非パスのコール（ビッド・ダブル・リダブル）をしていれば、ビッドする義務は解けているので `forcing_situation = false`（その局面のパスは矛盾ではなく、`pass_forcing` の 0..=0 にしない）。代替は `(inf.constraint, 1.0)` の 1 つ。`kind = Natural`、`text = inf.explanation`（`rule` 名を付記）、`ε = eps_natural`。`inf.confidence` は `text` に記録するだけで重みには使わない（ε に畳み込むかは「未決」）。
7. **ε-混合**: 全代替の重みに `(1 − ε)` を掛け、防御枝 `(HandConstraint::ANY, ε + (1 − ε)(1 − Σw), CallExplanation { kind: Fallback, node: None, text: "" })` を末尾に追加する（D15）。`Σw` は手順 2–6 の代替の重みの合計で、手順 5.2 の減衰が残る場合を除き 1（したがって防御枝は通常 ε）。`opts.strict` なら ε = 0 とし、防御枝を追加せず、`Σw < 1` なら代替を正規化する。
8. `CallInterpretation { call_index: j, seat: s, call: calls[j], kind, alternatives }` を `per_call` に push する。

手順 5 で `d < n_k − 1` の場合（相手のコールか、我々の以前のコールで既に分岐している場合）は、暗黙パスの補集合は「この位置でシステムが提示する選択肢」を表さないので手順 5.1 を飛ばす。手順 5 の順序（寛容照合 → ナチュラル推定）は計画 §5.5 の「(1) `resolve_lenient`、(2) 残りの我々側コールにナチュラル推定」に従う。

### 4.2 ε-混合の根拠（D15）

**フェーズ 4 の改訂**：ε-混合は、方策の一様床 ε/n として定義し直した。

- `eps_exact` / `eps_partial` / `eps_natural` / `lenient_decay` は既定の経路から外し、`InterpretOptions::legacy()` として 1 フェーズだけ残す（ESS の前後比較用）。
- 「信頼度が低いほど ε を大きくする」という役割は、δ（システム外への逸脱）と、方策上選ばれないコールの床に移る。
- `InterpretOptions` は `InterpretOptions::for_context(&BidContext)` で作る。`PolicyParams` と `implicit_pass` を尤度と同じ値から取るので、解釈と尤度がずれることは構造上起きない。
- 方策のナチュラル推定器は常に定まる：`call_distribution`・`sequence_log_likelihood`・`AuctionPolicy` はいずれも `ctx.natural`、それが `None` なら `table.natural` を使う（レビュー修正で `call_distribution` もこの補完をするようにした。以前は `None` のとき M が一様になり、δ > 0 では鏡像とずれていた）。鏡像は `table.natural` で読むので、両者が一致するのは `ctx.natural` が `None` か `Some(&*table.natural)` のときである。別の推定器を鏡像にしたいときは、その推定器を持つ `Table` で解釈する。`choose_bid` にとっての `None`（システム外で `NoCandidate`）は変わらない。
- 以下の表はフェーズ 3 の根拠である。「支持集合が空にならない」「`strict` で制約そのものを検証できる」「事後補正できる」の 3 点は、ANY 片（ε/n）でもそのまま成り立つ。

低信頼度の解釈を「HCP を 2 広げる」のような場当たりな緩和で表すと、緩め幅に根拠がなく、しかも緩めても支持集合が空になる場合を救えない。ε-混合は、コール毎に「無制約の代替」を重み ε で追加するだけで、次の性質を得る。

| 性質 | 理由 |
| --- | --- |
| 支持集合が空にならない | どの席の選言にも `ANY` の成分が残る（`strict` を除く） |
| 信頼度の差が重みの差になる | Exact 0.02 < Partial 0.15 < Natural 0.30 と、系統的に ε が増える |
| 事後補正できる | サンプラー（09-sample.md §3）は混合からサンプルし、重点重み `L / π` が防御枝から出た配牌を尤度で補正する |
| 検証可能 | `strict` で ε = 0 にすれば、双方向整合性プロパティテストが「制約そのもの」を検証できる |

### 4.3 「以前の制約を弱める」の操作的意味

仕様 §5 のフォールバック階層は「部分一致では直近のビッドのみ解釈し、それ以前は制約を弱める」と述べる。L3 における操作的意味は次の通り。

1. L3 は各コールに、**兄弟の上位候補を除いた排他領域** を使う（フェーズ 4）。祖先ノードの制約は取り込まない（ノードの制約は説明文コンパイラが親連鎖を解決済みであり、AND は Step B で席ごとに行う）。以前の「そのノード自身の制約だけ」は廃止した（Legacy では残る）。
2. 分岐点（`divergence`）より前のコールは、その時点で Exact だった。ビッダーは当時から自分の手を知っていたので、その言明は今も有効であり、`eps_exact` のまま維持する。「一致済みの部分は弱めない」（計画 §5.5）。
3. 分岐点以降のノードは別経路向けに設計されたものである。寛容照合で得たノードは `eps_partial` と `ρ^subst` で信頼度を下げ、ノードが得られなければナチュラル推定に `eps_natural` を付ける。これが「弱める」の実体である。
項目 2、3 の `eps_*` と `ρ^subst` は Legacy の説明である。Mirror では、分岐点以降の信頼度は方策そのものが決める。寛容照合の位置は `choose_bid` と同じ最初の完全一致を重み (1−ε)(1−δ) で読み、システム外の位置はナチュラル片を (1−ε) で読む。

4. 計画 §6.2 の「ダメなら `(node.constraint, 1.0)`」の `node`（`d` 以下で最深のノード）は、席 `s` 自身のノードならその index で既に適用済みで AND に何も加えず、別の席のノードなら `s` の手に適用してはならない。したがって L3 は寛容照合が失敗したらナチュラル推定へ進む。Partial ノードの制約を Natural の代わりに再利用するかは「未決」（フェーズ 3.11 の再現率で判断）。

### 4.4 Step B: 席ごとの結合（AND = 直積）

```text
for s in Seat::ALL:
    combos = [(要約 ANY, 1.0, key = [], catch_all = true)]
    for cj in per_call where cj.seat == s (call_index 昇順):
        next = []
        for (S, w, key, ca) in combos:
            for (i, (_, wi, _)) in cj.alternatives:
                S2 = S.and(summary(cj, i))           // 前計算した片の要約。空なら枝刈り
                if S2 が空: continue
                next.push((S2, w * wi, key + [i], ca && i == cj の ANY 片))
        if next.len() > K:
            next を見積り質量 w · cells(S2) の降順に整列し K 個に切り詰める
            ただし catch_all の組合せが落ちるなら、K − 1 個 + catch_all にする
        combos = next
    if combos.is_empty():
        combos = [(ANY, 1.0, [])];  tracing::warn!(seat, "seat contradicts itself")
    重み降順に並べ、合計 1 に正規化
    生き残った組合せについてだけ、key から HandConstraint（And）と Explanation を組み立てる
```

**フェーズ 4 の変更点**:
- 直積と要約による枝刈りは従来どおり行う。片の要約は Step A で前計算してある（システム片は索引の `summary` を借用する）。したがって、組合せごとに要約を再計算しない。
- 切り詰めの順序は、重み w ではなく見積り質量 w·cells(要約) の降順にした。cells は要約の箱に入る (シェイプ, HCP) セルの数で、w·2^{volume_log2} の近似である。提案は成分を w·count に比例して引くので、目標質量の小さい組合せから捨てる。
- 全コールで ANY 片を取った受け皿の組合せ（重み Π ε/n）は、常に 1 つ残す。これで提案の支持集合が目標の支持集合を覆い、推定が偏らない。
- 組合せのキーは、各コールで選んだ片の index の列（`SmallVec<[u16; 8]>`）である。
  - 1 コールの片は互いに別物で、同じ片の列は 2 度現れない。したがって、重複除去（旧 `(node, kind, branch)` 列）は要らなくなった。
  - 計画の「(node, kind, branch, 片種別) 列」は、この index 列と同値である。
- 最終的な並びは重み降順（呼び出し側と説明文が期待する順）である。

1. コールのない席は `[(ANY, 1.0, Explanation::empty())]`。
2. `is_satisfiable` は Step B では **要約検査だけ** を使う（`shapes` 空、`hcp` 逆転、`hcp.start > shapes.max_hcp()`、`count` 範囲空）。`Sampler::prepare(...).count() == 0` による決定版（20〜60 μs）は 10 μs 目標に収まらないので呼ばない。
3. （フェーズ 3）重複除去のキーは `parts` の `(node, kind, branch_index)` 列だった。同じノードに複数の寛容置換で到達した場合だけ合算が起きていた。フェーズ 4 では上記のとおり不要である。
4. 各ステップで K に切り詰めるので、1 ステップの要約 AND の回数は ≤ K × (そのコールの片の数)。鏡像では、片の数は (枝数 + Fallback 片 1〜3)。
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

**フェーズ 4 の追記**：Mirror では、`likelihood(s, h)` は Π_j exp(−log_scale_j)·p_j(c_j | h) に等しい。つまり、方策の尤度そのもの（コールごとの定数倍を除く）になる。定数を含めた尤度は Σ_j [log_scale_j + ln D_j(h)] で求まり、`AuctionPolicy`（§6.2）はこの形で計算する。所属判定では、ナチュラルの片に所属判定形（厳密な木）を使う。`alternatives` に入るのは提案形なので、リテラルを持つ片では `likelihood` は上側近似になる。

`satisfied_by` を `per_call` で定義する理由: 席の真の選言は「各コールから 1 代替を選んだ AND の和」であり、`h` がそれに属することと「各コールに `h` を含む代替がある」ことは同値である。`seats` は K で切り詰められているので、`seats` で判定すると枝の多いノード（4 枝 × 3 コール = 64 組合せ > 8）で偽の違反が出る。`likelihood` も同じ理由で積の形（切り詰め前の質量と一致し、正規化も不要）で計算する。`seats` は提案分布の構築（09-sample.md §6）に使い、切り詰めの影響は重点重みが補正する。

### 4.6 性能（目標 < 10 μs）

| 段階 | 見積り（12 コール） | 対策 |
| --- | --- | --- |
| Step A の `resolve` | 12 回 × 深さ ≤ 12 × 30 ns ≈ 4.3 μs（最悪） | `for_auction` は 1 席 1 回、`calls` の切り詰めはスライス操作のみ。超過時は側ごとの逐次照合（2 回）に変更 |
| Step A の代替構築 | 代替 ≤ 3/コール、`SmallVec<[_; 8]>` | ヒープ割当なし |
| Step B の AND | 4 席 × 3 コール × ≤ 16 回 ≈ 200 回 × 30 ns ≈ 6 μs | `Atom ∧ Atom` は `ShapeSet`（9 × u64）の AND と区間クランプ。`cards`/`eval` が空なら割当なし。要約検査を AND の前に行う |
| `classify` + `infer` | Natural のコールだけ | 規則表は述語の順次評価 |

計測は criterion で 12 コールのオークション（フェーズ 3.6 のベンチ `interpret_bench`）。

**実装後の追記（perf レーン、フェーズ 3.12）**: 上表は計画時の見積りで、実装は以下の 3 点で見積りより有利になっている。

- `ShapeSet::min_hcp`/`max_hcp`（= `hcp_bounds`）は当初 560 個の全メンバー形を歩く実装だったが、`bridge-core` 側に「9 語 × 8 バイト」ごとの事前計算済みルックアップテーブル（`BYTE_MIN_HCP`/`BYTE_MAX_HCP`、値はビット単位ではなく非零バイトの値で引く）を持たせ、非零バイト 1 個につき 1 引きに落とした（最悪でも 72 引き、`min`/`max` は 1 パスにまとめた `hcp_bounds()` で同時に求める）。`shapes == ShapeSet::ALL` のときは従来どおりこの計算自体を省く。
- Step B の交叉積は `HandConstraint::And` の木を組合せ候補 1 個ごとに構築していたが、これを `Summary`（`shapes`/`hcp`/`bounds` の要約）だけを組合せごとに引き回す形に変え、`HandConstraint::And` の実体は重複除去・K 個への切り詰め後に生き残った組合せについてだけ、席ごとに高々 `max_alternatives` 回組み立てる（`materialize_constraint`、`CallExplanation` を切り詰め後にだけ組み立てる既存の `materialize_parts` と同じ考え方）。
- 交叉積の重複除去キー（`Combo::key`、`(node, kind, branch_index)` の列）は候補 1 個ごとに `Vec` をヒープ確保していたが、1 席あたりの実コール数は 12 コール・4 席の典型（各席が均等に 3 回発言する）では高々 4 程度なので、インライン容量 4 の `SmallVec`（`ComboKey`）に変え、その典型ケースでは確保自体をなくした。あわせて重複除去も、`deduped.iter_mut().find(|d| d.key == combo.key)` によるその場ごとの線形走査（候補数を n として O(n²)）から、`key` でソートしてから隣接する等しいキーをまとめる方式（O(n log n)）に変えた。

ベンチは `bridge-bidding/benches/interpret.rs` に集約されている: 手組みの 2 系統（`interpret/12-call-auction` は裸の HCP 制約のみで `shapes == ALL` を常に取る最良ケース、`interpret/12-call-auction-realistic` は各ノードが自分のスート長も課す意図的な最悪ケース）に加え、`systems/sayc/sayc.bml` から実コンパイルした SAYC を使う 3 本 —— `interpret/sayc-1nt-auction`（`1NT-P-2C-P-2H-P-3NT-P-P-P`、10 コール）、`interpret/sayc-competitive-auction`（`1S-(2H)-X-(P)-3S-(P)-P-P`、ネガティブダブル入りの競り合い、8 コール）、そして仕様 §9 / 本節が実際に定めている長さそのものを実 SAYC で解釈する `interpret/sayc-12-call-auction`（`1C-P-1H-P-1S-P-2NT-P-3NT-P-P-P`、12 コール）。

（フェーズ 4 の注記）`sayc-12-call-auction` の 3NT は方策上選ばれない（shadowed）。SAYC に `1C-1H-1S-2NT` の続きが無いのでこの位置はシステム外で、ナチュラル規則にもここでの 3NT 候補が無い（`rule_rebid_nt` が発火しない）。したがって両プリセットで p(3NT|h) = ε/n（`log_scale` ≈ −10.17）であり、17+ HCP の開始者も 3NT を選ばない。受け入れのベンチはこのまま残し、方策どおりの 12 コール `interpret/sayc-12-call-on-policy`（`P-P-1NT-P-2C-P-2S-P-4S-P-P-P`、shadowed 0）を並べて計る。

**目標未達**: 上記 3 点の最適化は実在する。`hcp_bounds` のルックアップテーブル化は正しさを差分テスト（`bridge-core::shape::tests::hcp_bounds_match_brute_force`、`shape_set.rs::min_max_hcp_matches_naive_walk_for_every_single_shape`・`random_sets_are_consistent`）で、全 560 単一形状と乱数集合について「全メンバー形を歩く」旧実装相当のブルートフォースと一致することを確認済み。`ComboKey`/重複除去の変更は専用の差分テストは追加していないが、`bridge-bidding` の Step B 既存ユニットテスト（`and_combination_drops_contradictions`・`weights_sum_to_one`・`seat_without_calls_is_any`・`implicit_pass_bidirectional` 等）が変更前後で green のままであることで確認した。だが、**12 コールのオークションを 10 μs 未満で解釈するケースは、実 SAYC・手組みのいずれでも観測されていない**。この節の計測は、本フェーズで並行して動いている他エージェントのビルド/ベンチ（`git worktree list` で 20 前後）の影響を強く受ける共有開発機上で行っており、`uptime` の負荷平均が常時 9〜11 という状態のため、単発の値ではなく多数回実行した範囲で書く: `interpret/sayc-12-call-auction`（実 SAYC、目標そのものの 12 コール）はおおむね 12〜20 μs、最良実行でも 11.6 μs 程度。裸の HCP 制約のみの最良ケース `interpret/12-call-auction` もおおむね 10〜16 μs、最良実行で 9.5 μs 程度。`interpret/12-call-auction-realistic` はおおむね 11〜20 μs（負荷スパイク時は 25 μs 超）。10 μs を安定して下回るのは `interpret/sayc-1nt-auction`（10 コール、おおむね 8〜16 μs、最良 8.5 μs 程度）と `interpret/sayc-competitive-auction`（8 コール、おおむね 6〜10 μs）だけだが、これらは仕様 §9 が要求する 12 コールより短いオークションであり、この 2 本が目標内であることは 12 コールの目標が満たされていることを意味しない。したがって 11-testing.md §9 の `interpret` 12 コール < 10 μs は、本レーンの時点では**未達**として記録する。ボトルネックは引き続き Step B の交叉積と見られる（上表の計画時見積りで Step A ≈ 4.3 μs に対し Step B ≈ 6 μs。本レーンの recheck で報告された実 SAYC 12 コールの内訳測定でも Step A は全体の一部（数 μs）に留まっている）。`Summary::and` のインライン化や `ComboKey` のさらなる圧縮など、追加の高速化余地が残っている。

2026-09-26 のフェーズ 3 統合時 (SAYC 2 レーン統合後、負荷平均 9〜16) の `cargo bench -p bridge-bidding --bench interpret`: `interpret/12-call-auction` 11.3 μs、`interpret/12-call-auction-realistic` 12.3 μs、`interpret/sayc-12-call-auction` 17.6 μs、`interpret/sayc-1nt-auction` 9.5 μs、`interpret/sayc-competitive-auction` 7.4 μs、`sequence_log_likelihood/12-call-auction` 5.5 μs、`sequence_log_likelihood/12-call-auction-realistic` 6.2 μs (いずれも 2 回目の実行の中央値。1 回目は負荷平均 21 で 12 コール 22〜32 μs と大きくぶれた)。12 コール < 10 μs は引き続き未達。

**フェーズ 4（方策鏡像）の目標と実測**：

| 項目 | 目標 | 手段 |
| --- | --- | --- |
| `interpret/sayc-12-call-auction`、`interpret/12-call-auction`（δ = 0、既定） | < 10 μs（中央値。loadavg を併記し、負荷が高いときは 3 回の最良値も記録） | システムの片は索引から借用する。要約は前計算。位置データはメモ（下記）。Step A/B の内訳を記録する |
| ナチュラル位置 1 コール（コールド） | 追加 ≤ 5 μs | `infer_batch`、HCP 範囲ごとの箱の合併（`union_sub`）、片と説明文の遅延計算 |
| δ > 0（human プリセット） | 12 コール ≤ 40 μs | システム内の位置でもナチュラル候補の列挙が要る |
| `AuctionPolicy::log_likelihood`（sayc-12） | ≤ 10 μs / 配牌 | 片の所属判定のみ |

`Table` には隠しキャッシュを持たせない（`Table` の構造体リテラルは壊さない）。オークション単位で再利用したい場合は、`InterpretCache` に `AuctionPolicy` を持たせる（§6.4）。

**位置メモ（`memo.rs`）**：位置ごとの手に依らないデータを、スレッドごと・有界のメモに置く。

- 対象は、`enumerate_position` の結果（`PositionCore`：トライの解決、寛容照合、合法性付きの子）と、ナチュラル位置での `NaturalPos`（順位順の候補、パートナー文脈、コールごとのナチュラル片と説明文。遅延計算）である。
- キーは (4 席の `SystemIR` とナチュラル推定器の `Arc` アドレス、`implicit_pass`、ディーラー、バル、コール列) である。ハッシュ衝突に備えて、参照時にキー全体を比べる。
- エントリは 5 つの `Arc` の `Weak` を持つ。したがって、エントリが生きている間は、同じアドレスが別の表に再利用されない（そのため `Arc::get_mut` は失敗する）。
- 容量は 1024 エントリ × 2 世代である。古い世代でヒットしたエントリは新しい世代に移し、新しい世代が満杯になったら古い世代を捨てる（近似 LRU）。
- ヒットは再計算と同一の値を返すので、観測できる違いは速度だけである。`choose_bid`、`call_distribution`、`interpret`、`AuctionPolicy::new` のすべてがこのメモを通る。

**実測（レビュー修正後、criterion の中央値、μs。warm-up 2 s・計測 4 s で 3 回、括弧内は開始→終了の loadavg）**：

| ベンチ | 1 回目（10.4→6.9） | 2 回目（6.3→5.4） | 3 回目（5.1→5.3） |
| --- | --- | --- | --- |
| `interpret/12-call-auction` | 8.27 | 8.32 | 7.62 |
| `interpret/12-call-auction-realistic` | 7.99 | 8.00 | 9.25 |
| `interpret/sayc-1nt-auction`（10 コール） | 8.58 | 7.33 | 7.27 |
| `interpret/sayc-competitive-auction`（8 コール） | 6.42 | 5.36 | 4.99 |
| `interpret/sayc-12-call-auction` | 10.75 | **9.90** | 10.01 |
| `interpret/sayc-12-call-on-policy`（shadowed 0） | 11.25 | 9.92 | 10.54 |
| `interpret-step-a/sayc-12-call-auction`（Step A のみ） | 3.63 | 3.64 | 3.60 |
| `interpret-step-a/sayc-1nt-auction` | 3.06 | 2.88 | 2.95 |
| `interpret-step-a/sayc-competitive-auction` | 2.61 | 2.46 | 2.48 |
| `interpret-step-a/12-call-auction` | 3.76 | 2.92 | 2.88 |
| `interpret/natural-heavy-auction`（コーパス、下記、メモ済み） | 11.48 | 8.29 | 8.24 |
| `interpret-cold/natural-heavy-auction`（反復ごとに新しいナチュラル推定器 = 全位置コールド） | 60.6 | 54.2 | 58.0 |
| `interpret-cold/sayc-12-call-auction`（同上） | 57.3 | 47.5 | 46.6 |
| `interpret-human/sayc-12-call-auction`（当時の仮置き ε = 0.01、δ = 0.3。最尤推定値の `human()` では 13.1〜13.2、12-roadmap） | 13.4 | 13.8 | 13.4 |
| `auction-policy/log-likelihood/sayc-12/system-players` | 0.126 | 0.112 | 0.159 |
| `auction-policy/log-likelihood/sayc-12/human` | 0.139 | 0.137 | 0.152 |
| `auction-policy/new/sayc-12/system-players` | 9.33 | 8.14 | 10.2 |
| `auction-policy/new/sayc-12/human` | 13.8 | 11.8 | 12.5 |
| `sequence_log_likelihood/sayc-12/system-players`（参照実装） | 3.87 | 3.82 | 3.89 |
| `sequence_log_likelihood/sayc-12/human`（参照実装） | 4.49 | 4.01 | 4.07 |

natural-heavy のベンチは、計画どおりコーパスの競り合いオークションに差し替えた：`1H-(1S)-2C-(2D)-X-(P)-2H-(P)-3H-(P)-P-(P)`（`corpus_auctions_with_deals` の列挙番号 615、ディーラー West、NS バル）。12 コールのうち 9 コールがナチュラル読み（うち 2 つは shadowed）である。レーン B の手組みの `1H 2C 2D 3C 3H P 4H P P P` は、コールド 78.9〜128 μs だった。

所見：

- **12 コール < 10 μs**
  - 手組みの `interpret/12-call-auction` は 7.6〜8.3 μs で達成した。
  - 実 SAYC の `interpret/sayc-12-call-auction` は 9.90〜10.75 μs で、3 回の最良値でようやく 10 μs を切る。中央値の基準としては**境界線上（未達扱い）**のままである。方策どおりの 12 コールも 9.92〜11.25 μs で同じ水準である。
  - レーン B 最終の計測（10.12 / 9.99 / 10.33）と同じ水準である。統合時に静かな機械で 3 回の最良値を取り直す。
- **内訳**：sayc-12 は Step A が約 3.6 μs、Step B が約 6.3 μs である。
  - Step B は、切り詰め後に生き残った 28 組合せの実体化にかかる。内訳は、`HandConstraint` の And と各片の複製（サンプリングで約 45%）、`CallExplanation` の複製（説明文の `String` を含む）、`Explanation` の連結である。
  - 片は小さい（1 片あたり 1〜10 アトム、28 組合せで計約 160 アトム）。したがって、残りは公開型（`CallExplanation.text: String`、所有する `HandConstraint`）の複製と割当である。
  - これ以上は、公開型を共有型（`Arc<str>`、`Arc` で共有する片）に変えない限り削りにくい。後続の課題とする（§9 の 11）。
  - フェーズ 3 統合時の 17.6 μs からは短縮した。
- **ナチュラル位置のコールド追加（≤ 5 μs）**
  - レビュー修正で、ナチュラル領域の計算を速くした。`union_sub` と `sup_of` は、アトムでないナチュラル候補を毎回 `grid::bounds` に通していた。これはシェイプ × HCP のグリッドを候補ごとに作って交わす処理である。レベル下限のため、高いレベルの候補はほとんどが `And([規則のアトム, HCP のアトム])` になる。
  - リテラルを持たないアトム、その And（形の積と HCP 範囲の積の 1 つの箱）、それらの Or は、HCP 範囲ごとの箱に直接まとめるようにした。領域は同一である。
  - コールドの natural-heavy は 108 → 49 μs になった（バッチ最小値、loadavg 約 9）。プロファイルでは、残りは `infer_batch`（bridge-system）とメモの世代入れ替えによる解放である。
  - 1 コールあたりのコールド追加は次のように出す：(コールド natural-heavy − warm natural-heavy − 12 位置 × システム位置 1 つのコールド費用) / 9。システム位置 1 つのコールド費用は (コールド sayc-12 − warm sayc-12) / 12 で、3.1〜3.9 μs である。
  - 結果は 3 回それぞれ 0.3 / 0.9 / 1.5 μs で、**達成**した。
- **δ > 0**：human プリセットの 12 コールは 13.4〜13.8 μs で、目標 40 μs 内である。
- **その他**：`sayc-1nt` 7.3〜8.6 μs（≤ 10）、`sayc-competitive` 5.0〜6.4 μs（≤ 8）、`12-call-auction-realistic` 8.0〜9.3 μs（≤ 12）で、いずれも達成した。
- **`AuctionPolicy::log_likelihood`**：0.11〜0.16 μs / 配牌で、目標 10 μs に対して 2 桁の余裕がある。参照実装の約 1/30 である。

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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolicyParams {
    pub epsilon: f32,                    // 一様床 ε。既定 1e-3
    pub deviation: f32,                  // システム外（ナチュラル）への逸脱 δ。既定 0.0。1/2 未満に保つ
    pub legacy_temperature: Option<f32>, // Some(τ) で旧 priority ソフトマックス（比較用、フェーズ 6 評価後に削除）。鏡像の保証は無い
}
impl PolicyParams {
    pub const fn system_players() -> Self; // = Default（ε = 1e-3、δ = 0）。システムどおりに競る前提（生成オークション）
    pub const fn human() -> Self;          // コーパス調整用分割で最尤推定した (ε, δ) = (0.3404, 0.3959)。フェーズ 4 の統合（wip/p4int 8669ffd の SAYC、調整用 ln L −7295.9）で設定。経緯は 12-roadmap
    pub const fn legacy(temperature: f32) -> Self; // 旧方策（ε = 1e-3、legacy_temperature = Some(τ)）
}
pub struct BidContext<'a> {
    pub scoring: Scoring,                        // v1 では素通し（L2 の条件に scoring がない。「未決」: #+SCORING 条件）
    pub natural: Option<&'a NaturalInference>,   // choose_bid: システム外のときのフォールバック（None なら NoCandidate）。方策（call_distribution / 尤度 / AuctionPolicy）: None なら table.natural
    pub implicit_pass: ImplicitPass,             // テストの既定 Never、アプリの既定 Complement
    pub policy: PolicyParams,
}

pub fn choose_bid(table: &Table, hand: Hand, auction: &Auction, ctx: &BidContext) -> BidChoice;   // 手番の席のシステムは table.systems[seat]
```

### 5.2 手順

`seat = auction.next_seat()`、`opener_pos = auction.position_of(seat)`（まだ誰もビッドしていなければ自分がオープナー）または `position_of(seat_at(leading_passes()))`、`vul = RelVul { we, they }`。

1. **候補集合**
   1. `key = LookupKey::for_auction(auction, seat)`。`None`（まだ誰もビッドしていない）なら `LookupKey { we_opened: true, calls: &[], opener_pos: position_of(seat), vul }` を直接作る。
   2. `lookup = resolve(&key)`。`matched_depth == key.calls.len()` なら `candidates = children(lookup.end, opener_pos, vul)`、`source = System`（`SystemIR::continuations(auction, seat)` はこの 2 手順をまとめた便宜関数で、接頭辞がシステム外なら `None`）。
   3. そうでなければ `resolve_lenient(&key, LENIENT_MAX_SUBST)` のうち完全一致した最初の（置換回数最小の）`Lookup` の `end` から `children`。`source = System`。
   4. それも無く `ctx.natural` が `Some` なら、`natural.ranked_candidates(auction, seat, partner, tie_break)` を順位順に使う（`source = Natural`）。順位は round(confidence·100) 降順、`tie_break` が `LowestCall`/`HighestCall` ならそれに従い、最後にコール index 昇順。`None` なら候補は空。2 または 3 で位置が得られても、その `children` に合法なコールが 1 つも無い（より深い行の経路だけが相手のコールの辺を作った葉など）ならシステム外とみなしてこの手順を適用する: `interpret` はそこでのどのコールも `Natural` と解釈するので、`NoCandidate` を返すと双方がずれる。非合法な子の `Tried { Illegal }` / `IllegalSystemCall` は従来通り記録する。
2. **フィルタ**: 各候補 `(call, node)` について、`!auction.is_legal(call)` なら `Tried { reason: Illegal }` と `Diagnostic::IllegalSystemCall`（システム定義の lint。パニックしない）。`!node.constraint.satisfies(hand)` なら `Tried { Unsatisfied }`。`node.constraint` が要約検査で充足不能なら `Diagnostic::UnsatisfiableNode` も付ける。残りを `kept` とする。
3. **暗黙パス**（`ctx.implicit_pass == Complement`）: フェーズ 4 で、ナチュラル分岐にも適用するようにした。ナチュラル候補のどれにも合わず `Pass` が合法なら、`Pass`（priority `i16::MIN + 1`、`source = ImplicitPass`）を合成する。これで、システム外の位置での強制 Pass の穴（gap）がなくなる（プロトタイプ C：1e5 局面で `no_candidate` 234 → 28、gap 起因の違反 111 → 0）。システム分岐では次のとおり。 `kept` にも候補集合にも `Pass` がなく、`Pass` が合法なら、合法な兄弟候補の制約の `Not(Or(...))` を制約とする `Pass` を合成し、`priority = i16::MIN + 1`、`source = ImplicitPass`、`node = None`。`hand` が補集合を満たすときだけ `kept` に加える。解釈側（§4.1.1 手順 2、5.1）も同じ集合の補集合を使うので双方向が一致する。兄弟が全手を覆う場合は補集合が充足不能で `Pass` は合成されない（カバレッジレポートは `ImplicitPass` を真の穴と分けて集計する）。
4. **整列**: `bridge_system::exclusive::rank_cmp`。`priority` 降順、同点は `system.meta.tie_break`（下表）、最後にコール index 昇順（フェーズ 4 で明示した）。決定的。`ExclusiveIndex` とナチュラル候補の順位付けも同じ比較関数を使う（unit テスト `rank_order_shared`）。
5. **結果**: `kept` が空なら `NoCandidate { tried, diagnostics }`。そうでなければ `Chosen { call: kept[0].call, alternatives: kept 全体, .. }`。

| `TieBreak` | 規則 |
| --- | --- |
| `RowOrder`（既定） | `node.row` 昇順（BML の「最初の定義が勝つ」） |
| `Narrowest` | `node.volume_log2` 昇順（制約体積が小さい方） |
| `LowestCall` | `Call` 昇順（`Pass < Double < Redouble < Bid`） |
| `HighestCall` | `Call` 降順 |

`Rejected::NotApplicable` は seat / vul 条件で弾かれた兄弟を診断用に記録するための予約で、v1 の `children` は条件一致の候補しか返さないため `tried` には現れない（条件無視の列挙 API を L2 に追加するかは「未決」）。`DuplicateCandidate` は同じコールを持つ候補が 2 つ以上あるとき（ナチュラル候補、あるいは Exact 辺と Class 辺の双方から到達）に付け、`choose_bid` は `rank_cmp` で 1 つを選ぶ。フェーズ 4 の方策は「最初に満たした候補のコール」を選ぶだけで、質量を足し合わせることはしない（旧方策では `call_distribution` が両方を質量に数えていた）。

候補の列挙（手順 1〜3 のうち手に依らない部分）は `enumerate_position` にまとめ、`choose_bid`・`call_distribution`・`interpret` が共有する。その結果は、スレッドごとの有界メモ（§4.6）で位置ごとに 1 回だけ計算する。

---

## 6. 確率的方策・尤度・再生

### 6.1 `call_distribution`

```rust
pub fn call_distribution(table: &Table, hand: Hand, auction: &Auction, ctx: &BidContext) -> Vec<(Call, f32)>;
```

フェーズ 4 で差し替えた（D18）。位置 P（接頭辞 `auction[..j]`、手番 s）の合法コールを L（n = |L|）とする。

- s_P(h)：P がシステム内なら、`choose_bid` のシステム選択。システム内とは、厳密一致、または寛容照合の最初の完全一致の位置に、合法な子が 1 つ以上あることをいう。候補が無ければ ⊥。
- m_P(h)：ナチュラル方策の選択。`ranked_candidates` を順に見て最初に満たしたもの。無ければナチュラルの暗黙 Pass（`Complement` のとき）。それも無ければ ⊥。
- S(c|h) = 1[s_P(h) = c]。ただし s_P(h) = ⊥ なら 1/n。
- M(c|h) = 1[m_P(h) = c]。ただし m_P(h) = ⊥ なら 1/n。
- π(c|h)：システム内の位置では (1−δ)·S + δ·M、システム外の位置では M。
- **p(c|h) = (1−ε)·π(c|h) + ε/n**

性質：
1. δ < 1/2 なら、argmax_c p は `choose_bid` の選択に一致する。これは τ に依存しない構造的な等式である。`tests/policy.rs` の 10^5 局面テストは、両プリセットで 100% になる。
2. 同じ優先度どうしでの質量の分け合い（旧 shared 25〜48%）は起きない。
3. δ = 0 のときは、システム内の位置で m_P を評価しない（計算不要）。
4. `legacy_temperature = Some(τ)` のときは旧式を返す。旧式は、priority/τ の logsumexp → softmax → ε 床の順に計算する。これは比較評価専用で、`interpret` の鏡像はこの場合を保証しない。
5. 全合法コールが正の確率（≥ ε/n）を持つ。したがって、重みが 0 になるサンプルは出ない。

旧定義（τ = 1 の priority ソフトマックス）を捨てる理由は D18 に書いた。要点は 2 つある。
- BML の priority は整列のための小さな整数で、対数オッズとして較正されていない。
- 1NT と 1C の両方を満たす手が 27% で 1C を開くといった分布は、システムの意味にも `replay` にも一致しない。

### 6.2 `sequence_log_likelihood`

```rust
pub fn sequence_log_likelihood(table: &Table, deal: &Deal, auction: &Auction, ctx: &BidContext) -> f64;
// = Σ_j ln p_j(calls[j])、p_j = call_distribution(table, deal.hand(seat_j), auction[..j], ctx)
```

各 `j` で接頭辞 `auction[..j]` を対象に `call_distribution` を呼ぶ。`ctx.natural` が `None` でも、`table.natural` を補って呼ぶ。コスト ≈ コール数 × 候補数 × `satisfies` ≈ 2〜5 μs / 配牌。これが 09-sample.md の `ln L` の第 1 項である。

**高速経路（フェーズ 4）**：

```rust
pub struct AuctionPolicy { /* 所有データのみ。Clone + Debug */ }
impl AuctionPolicy {
    pub fn new(table: &Table, auction: &Auction, ctx: &BidContext<'_>) -> AuctionPolicy;
    pub fn log_likelihood(&self, deal: &Deal) -> f64;   // = sequence_log_likelihood（|Δ ln L| ≤ 1e-5）
    pub fn auction(&self) -> &Auction;
    pub fn policy(&self) -> PolicyParams;
}
```

- 参照実装（各 j で `call_distribution` を呼ぶ）は、そのまま残す。
- `AuctionPolicy::new` は、オークション 1 本につき 1 回だけ、各コールの片を組み立てる。
  - 片は所属判定形で持つ。リテラルを含まない領域は `HcpShapeGrid`（1 回の表引き）、それ以外は `HandConstraint::satisfies` で判定する。
  - 生の重みも同時に組み立てる。
  - ANY 片は床 ε/n としてまとめる。
- `log_likelihood(deal) = Σ_j ln(floor_j + Σ_i raw_{j,i}·1[h_{s_j} ∈ C_{j,i}])` を、配牌ごとに評価する。
  - システムコールは X_c の所属判定で済み、兄弟候補を全部評価する必要はない。
  - δ > 0 では、手がシステム片とナチュラル片の両方に入り得るので、全片を判定する。
- `legacy_temperature` のときは、参照実装に委ねる（鏡像が無いため）。
- `bridge-sample` の `BiddingLikelihood` は、この高速経路を使う。
- `tests/policy.rs` の `fast_likelihood_matches_reference` で、参照実装との |Δ ln L| ≤ 1e-5 を保証する。

### 6.3 `replay`

```rust
pub struct Replay { pub auction: Auction, pub gaps: Vec<(usize, Seat)>, pub diagnostics: Vec<Diagnostic> }
pub fn replay(table: &Table, deal: &Deal, dealer: Seat, vul: Vulnerability, ctx: &BidContext) -> Replay;
```

`auction.is_complete()` まで: `seat = auction.next_seat()`、`choose_bid(table, deal.hand(seat), &auction, ctx)`。`Chosen` は push、`NoCandidate` は `Pass` を push して `gaps` に `(index, seat)` を記録。診断は連結。合法性が終了を保証するが、安全のため 320 コールで打ち切る。再現率テスト（§8）と `xtask coverage` が使う。

フェーズ 4 でも手順は変えない。7 レベルへの暴走（ESS 原因 (3)）は、06-system §8 のナチュラル・レベル下限で直す。これは方策（目標）の変更なので、解釈の変更とは別に入れて、単独で検証する。

### 6.4 `InterpretCache`

```rust
#[derive(Default)]
pub struct InterpretCache {
    map: HashMap<(AuctionKey, OptionsKey), Arc<Interpretation>>,
    policies: HashMap<(AuctionKey, PolicyKey), Arc<AuctionPolicy>>,
}
impl InterpretCache {
    pub fn new() -> Self;
    pub fn get_or_interpret(&mut self, table: &Table, auction: &Auction, opts: &InterpretOptions) -> Arc<Interpretation>;
    pub fn get_or_policy(&mut self, table: &Table, auction: &Auction, ctx: &BidContext<'_>) -> Arc<AuctionPolicy>;
    pub fn len(&self) -> usize;  pub fn is_empty(&self) -> bool;
}
```

`SystemIR` は不変で、キャッシュを持たない（仕様 §9）。`Table` も隠しキャッシュを持たない。オークション単位のキャッシュは、呼び出し側が所有する。

キー:
- `AuctionKey` は (ディーラー, バル, コール列) である。
- `OptionsKey` は、解釈を変える `InterpretOptions` のビット（K、strict、mode、policy、implicit_pass、Legacy の ε）である。
- `PolicyKey` は (policy, implicit_pass, ナチュラル推定器のアドレス) である。
- したがって、`opts` を変えても同じキャッシュを使える。ただし `Table` はキーに含まれないので、表ごとに 1 つのキャッシュを使う（呼び出し側の規約）。

---

## 7. モジュール構成

| ファイル | 内容 |
| --- | --- |
| `lib.rs` | 再エクスポート（`bridge_system::{NaturalInference, NodeId, SystemIR}` を含む）、crate doc、`Table`、`Scoring`、`ImplicitPass`、`BidContext` |
| `interpret.rs` | 型（§3）、Step A（Mirror / Legacy）/ Step B、`satisfied_by` / `likelihood`、`Explanation::from_parts`、`LENIENT_MAX_SUBST`、`InterpretOptions::{for_context, legacy}`、ベンチ用の `#[doc(hidden)] interpret_per_call`（Step A のみ） |
| `choose.rs` | `BidChoice` 系の型、`choose_bid`、合法性 lint、`rank_cmp` による整列、暗黙パス（システム・ナチュラル）、`enumerate_position`（3 者共有の候補列挙） |
| `exclusion.rs`（フェーズ 4） | 鏡像の片の組み立て（`mirror_call`）、システム片の実行時再計算、ナチュラル排他領域（グリッド、2 形）、`NaturalPos`（位置ごとのナチュラル候補・片・説明文）、`Reader` / `partner_context` |
| `auction_policy.rs`（フェーズ 4） | `AuctionPolicy` |
| `memo.rs`（フェーズ 4） | 位置ごとの手に依らないデータの、スレッドごと・有界のメモ（§4.6） |
| `policy.rs` | `PolicyParams`、`call_distribution`（D18 と旧式）、`sequence_log_likelihood`、`logsumexp` |
| `replay.rs` | `Replay`、`replay` |
| `cache.rs` | `InterpretCache`（`get_or_interpret`、`get_or_policy`） |
| `benches/interpret.rs` | criterion（§4.6 の全ベンチ） |

関連する他クレートのモジュール：`bridge-system/src/exclusive.rs`（`ExclusiveIndex`、`rank_cmp`、`subtract`）、`bridge-constraint/src/grid.rs`（`HcpShapeGrid`）。

---

## 8. テストと性能目標

| テスト | 場所 | 種類 | 基準 |
| --- | --- | --- | --- |
| `forward_consistency`（仕様 §10） | `tests/consistency.rs`、`#[ignore]`、release | 10^6 のランダム (hand, auction 接頭辞)、`InterpretOptions { strict: true, .. }`。`Chosen` なら `interpret(auction.with(call)).satisfied_by(seat, hand)` | 1e5 で gap 起因でない違反 0、gap 起因 ≤ 30。1e6 は報告する。`NoCandidate` と `ImplicitPass` はノード別に集計し `target/coverage_report.json`（上位 50 の穴） |
| `reproduction_rate` | 同ファイル | コーパスの 500 オークション × `sample_deals(1000)` → `replay == auction` の率 | ノード別に報告。フェーズ 4 で中央値 ≥ 0.6 |
| `policy_argmax_matches_choose_bid` | `tests/policy.rs` | 10^5 局面、`system_players()` と `human()` の両プリセット | 100% |
| `policy_mirror`（フェーズ 4） | `tests/mirror.rs` | 生成位置とコーパス位置の両方、固定の試験点 δ ∈ {0, 0.3}（ε = 1e-3）と最尤推定値の `human()`（フェーズ 4 の統合で追加）。既定スイートは 150 位置 × 40 手、`#[ignore]` 版（`policy_mirror_large`）は 2000 × 100（コーパスは 1 オークションから複数の異なるコールを取り、2000 位置に届かせる）。各 (コール, 手) で `exp(log_scale)·Σ w·1[h ∈ C]` を `call_distribution` と比べる | under-cover 0、厳密一致 ≥ 99%（リテラルによる over-cover ≤ 1%） |
| `policy_mirror_variants`（フェーズ 4） | 同上 | `ImplicitPass::Never`（δ ∈ {0, 0.3}）、`natural: None`（δ = 0.3）、接頭辞のコールを 0.3 の率で乱択の合法コールに置き換えた位置（寛容照合と X_c の実行時再計算）。既定は各 60 位置 × 20 手、`policy_mirror_large` では各 500 × 50 | under-cover 0。厳密一致は既定で ≥ 97%（小さい集合で 1 位置の over-cover が 1.7% に当たるため）、large で ≥ 99% |
| `recomputed_region_when_a_higher_sibling_is_illegal`（フェーズ 4） | unit | 手組みシステム。寛容照合の位置で、上位の兄弟（`1D`）が接頭辞 `1C-(1D)` の後で非合法 | `1H` は shadowed にならず（索引では `1D` に覆われる）、全ての手で鏡像 = `call_distribution` |
| `tightness`（フェーズ 4、プロトタイプ A 由来） | 同上 | δ = 0。`choose_bid` が到達する位置で、非 Fallback 片の内側 / 外側と「選ばれたか」を集計 | Exact：「内側なのに選ばれない」0、「外側なのに選ばれる」0 |
| `fast_likelihood_matches_reference`（フェーズ 4） | `tests/policy.rs` | 既定は 3 ソース × 50 オークション × (20 配牌 + 真の配牌) = 3150 配牌。受け入れの 50 × 1000 は `_large` 版（ソースごとに 50 オークション × 1001 配牌、計 150 オークション・150,150 配牌） | \|Δ ln L\| ≤ 1e-5 |
| `rank_order_shared`（フェーズ 4） | unit | 10^4 位置 | `choose_bid` の `alternatives` が、ノードから独立に計算した順位（priority 降順、`tie_break`、コール index 昇順）に並び、各候補の priority がノードの値と一致する（`choose_bid` 自身が使う `rank_cmp_keys` との比較は恒真なので使わない）。厳密に解決した位置では、索引の兄弟グループの順序とも一致 |
| `materialize_constraint_equals_the_and_one_more_fold` | `interpret.rs` の unit | 一括の And 組み立て | 逐次の fold と同じ木 |
| `illegal_call_is_lint` | unit | 不正な継続を含む合成 `SystemIR` | `Diagnostic::IllegalSystemCall` が返り、パニックしない |
| `weights_sum_to_one` | unit | 各席の `seats[s]` と各 `per_call` の重み | 合計 1（許容 1e-5） |
| `and_combination_drops_contradictions` | unit | 矛盾する 2 コール | 該当組合せが消え、残りが正規化される。全滅なら `ANY` + warn |
| `partial_and_natural_epsilon` | unit | 意図的に切り詰めたシステム | 分岐以降が Partial / Natural になり、`divergence` が正しい。`strict` で防御枝が消える |
| `implicit_pass_bidirectional` | unit | `Complement` で `Pass` を選んだ手 | 解釈の補集合を満たす |
| `interpret_bench` | `benches/` | criterion、12 コール | < 10 μs |
| コーパス解決率（フェーズ 4、`xtask coverage`） | `--ignored` | 実ハンドレコードのオークション | ≥ 80% が全コール Exact、残りに `EmptySupport` なし |

**フェーズ 4 の実測**（レビュー修正後、release）：

| テスト | loadavg | 結果 |
| --- | --- | --- |
| `forward_consistency` 1e5（seed 0x5a1c0002） | 4.1→13.8 | gap 起因でない違反 0、gap 起因 1（chosen 81607、no_candidate 15、implicit_pass 18378）、6.0 s |
| `forward_consistency` 1e6 | 13.1→10.9 | gap 起因でない違反 0、gap 起因 19（`no_candidate` 197）、27.5 s（レーン B 最終は 49.3 s。差は主にナチュラル領域の高速化と負荷） |
| `policy_argmax_matches_choose_bid` 1e5 | 4.1 | 両プリセットで 100%（system_players 1.75 s、human 1.86 s） |
| `fast_likelihood_matches_reference_large` | 4.4 | 150 オークション、150,150 配牌、最大 \|Δ ln L\| 5.3e-7 |
| `fast_likelihood_matches_reference`（既定） | — | 150 オークション、3150 配牌、最大 \|Δ ln L\| 3.9e-7 |
| `policy_mirror_large` | 4.6 | 表を参照 |
| `tightness_large`（2000 × 100） | 4.4 | `[[32953,0,0],[0,0,0],[63289,334,0]]`。Exact 0/0 |
| `rank_order_shared` | — | 10^4 位置すべてで、独立に計算した順位どおり。厳密に解決した 5474 位置では索引の兄弟グループの順序とも一致 |

`policy_mirror_large` の内訳（under-cover はすべて 0）：

| セル | 位置数 | 厳密一致 | over-cover | shadowed |
| --- | --- | --- | --- | --- |
| δ = 0、生成 | 2000 | 99.832% | 340 | 0 |
| δ = 0、コーパス | 2000 | 99.817% | 369 | 195 |
| δ = 0.3、生成 | 2000 | 99.638% | 731 | 0 |
| δ = 0.3、コーパス | 2000 | 99.631% | 746 | 169 |
| `Never`、δ = 0、生成 / コーパス | 500 / 500 | 99.812% / 99.733% | 48 / 68 | 105 / 120 |
| `Never`、δ = 0.3、生成 / コーパス | 500 / 500 | 99.671% / 99.612% | 84 / 99 | 0 / 41 |
| `natural: None`、δ = 0.3、生成 / コーパス | 500 / 500 | 99.431% / 99.612% | 145 / 99 | 0 / 41 |
| 置換位置、δ = 0 / δ = 0.3 / `Never` δ = 0.3 | 500 ずつ | 99.910% / 99.788% / 99.788% | 23 / 54 / 54 | 78 / 71 / 71 |

レーン B 最終のコーパスのセルは、1 オークションから 2 コールしか取らず 1250 位置だった。修正前の `natural: None` のセルは、`call_distribution` が M を一様にしていたため、under-cover が約半数あった（既定の 60 位置で 804 / 1260）。

**早期信号（ESS、レーン P の提案変更の前）**：フェーズ 5 の `ConstraintProposal`（literal-free coarsen も残差棄却も無い版）を、レーン B の解釈（e04ebca）と組み合わせて ESS スイートを回した。これは使い捨てのワークツリーで、wip/p4-P（febcf3d）の `constraint_proposal.rs` だけを wip/p4proto-base の版に戻したものである。ケースは固定フィクスチャ 50 件（生成 25 件は `system_players`、コーパス評価用分割 25 件は `human`）、n = 1000、SAMPLE_SEED 0xE55。結果は、ESS/n の中央値が全体 **0.390**、生成 0.349、コーパス 0.433 で、基準 ≥ 0.25 を**達成**した。受理率の中央値は 1.000（最小 0.957）、0.5 以上のケースは 22/50、一様提案では 0.004 だった（loadavg 10.7→10.9）。

実装順（計画 §12 フェーズ 3）: 3.6 型 + `interpret`（Exact のみ）+ ベンチ → 3.7 `choose_bid` → 3.8 `call_distribution` / `sequence_log_likelihood` → 3.9 `replay` → 3.10 `forward_consistency` + カバレッジレポート → 3.11 Partial / Natural + ε-混合 + `NaturalInference` 接続 → 3.12 再現率ハーネス。

---

## 9. 未決事項

| # | 項目 | 現在の仮置き |
| --- | --- | --- |
| 1 | `lenient_decay` ρ と `LENIENT_MAX_SUBST` の値 | ρ は Legacy のみ（0.5）。Mirror は `choose_bid` と同じ最初の完全一致だけを使う。`LENIENT_MAX_SUBST` = 2 |
| 2 | `Inference.confidence` を `eps_natural` に畳み込むか | フェーズ 4 で解消。confidence は順位としてだけ使う（`ranked_candidates`） |
| 3 | 寛容照合が失敗したとき Partial ノードの制約を再利用するか | しない（Natural へ） |
| 4 | `BidContext.scoring` を L2 の条件（`#+SCORING`）に接続する時期 | v1 は素通し |
| 5 | seat / vul 条件で弾かれた兄弟を `Tried { NotApplicable }` に載せるための L2 API | 未追加 |
| 6 | リテラルを持つ上位候補の差し引き（フェーズ 4） | sub による上側近似。over-cover は生成位置で約 0.2%、δ = 0.3 で約 0.4%。厳密な DNF への切り替えは保留 |
| 7 | δ を相手と味方で分けるか、位置の種類（競り合い・オープニング）で分けるか | 分けない（単一の δ） |
| 8 | `legacy_temperature` と `InterpretMode::Legacy` を削除する時期 | フェーズ 6 のリード評価で hard 方策と比較した後 |
| 9 | 方策上選ばれない枝（`ShadowedBranch` lint）を SAYC の側で消すか残すか | 残す（解釈は shadowed として Fallback だけで読む） |
| 10 | `human()` の (ε, δ) | フェーズ 4 の統合で解消。D3・len・perf のマージ後のヘッド（8669ffd）で当てはめた ε 0.3404、δ 0.3959（調整用 ln L −7295.9、評価用 −6663.2）に設定した。SAYC かナチュラル推定を変えたら `cargo xtask coverage` の `corpus.mle` で当てはめ直す（12-roadmap「フェーズ 4 の統合 (D3・len・perf のマージ後)」） |
| 11 | `interpret/sayc-12-call-auction` < 10 μs（中央値） | 3 回の最良値 9.90 μs、中央値 9.90〜10.75 μs で境界線上。2026-09-29 の再計測（コード変更なし、loadavg 3.4〜11）でも 3 回ずつ 2 組で 10.17〜13.6 μs、最良 10.17 μs だった。統合時に静かな機械で測り直す。10 μs 以上のままなら、Step B の実体化（`CallExplanation.text` と片の共有化。公開 API の変更）を後続で行う |
| 12 | `1C-1H-1S-2NT` の後の 3NT（ベンチ `sayc-12-call-auction` の最後の実質コール） | システム外の位置で、ナチュラル規則にも 3NT 候補が無いため shadowed。上位のナチュラル候補に覆われているのではない。レーン S（`rule_rebid_nt` がこの位置で発火しない。レベル下限が 6C などの充足不能な候補 `And([hcp 16..=18, hcp 20..=37])` を残す）とレーン D（SAYC に 1m-1M-1S-2NT の続きを足す）に回す |

`classify` に解釈済み文脈を渡す方法は §2.2 のとおり `CallContext.partner_constraint` / `forcing_situation` を L3 が後から埋める形で確定した。
