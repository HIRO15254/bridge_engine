# 06. システム定義と BML コンパイラ (`bridge-system`, L2)

本文書は `bridge-system` クレートの詳細設計を確定する。BML (Bridge Bidding Markup Language) を著作フォーマットとして採用し、自前の Rust パーサ (winnow) で AST を作り、`bss.py` 相当の展開で具体トライを構築し、説明文を `HandConstraint` にコンパイルして `SystemIR` を得るまでの全段階を、型・文法・アルゴリズム・数値基準の粒度で定める。仕様 §5 と計画 §5、決定 D7 (自前パーサ、`.bss` をオラクル)、D8 (ナチュラル推定の 3 測定)、D9 (BML 配布 + ロード時コンパイル、`postcard` キャッシュは任意)、D16 (`#+KEY:` メタ拡張と `{prio:N}` / `{w:X}` 注釈)、D17 (`#SEAT` はオープナーの席位置) を前提とし、計画と他資料が食い違う箇所は計画に従う。

**BML を書く人へ**: 本コンパイラが受理する方言 (拡張を含む全構文、説明文の語彙、メタ行、解決の意味論、全 Lint、完全な EBNF、例) は `16-extended-bml.md` にまとめてある。システムファイルを書くときはそちらを正本とし、本文書は内部設計を参照するときに読む。両者が食い違う箇所は `16-extended-bml.md` の付録 B に挙げてある。

前提となる決定事項:

| 決定 | 内容 | 本文書での扱い |
| --- | --- | --- |
| D7 | BML パーサは Rust (winnow) で自前実装。Python 版 `bml.py` / `bss.py` は参照実装。`gpaulissen/bml/test/expected/*.bss` を展開結果のオラクルにする | §3, §4 |
| D8 | ナチュラル推定の精度は (1) 実システム定義のノードを隠して比較、(2) 再現率、(3) コーパス実手の充足率 × 体積 で測る | §8.5 |
| D9 | BML ソースを配布しロード時にコンパイル (目標 1 秒未満)。`postcard` 直列化した `SystemIR` は BML ハッシュをキーにした任意のキャッシュ | §10 |
| D16 | `#+KEY:` 形式のメタ拡張と、説明文末尾の `{prio:N}` / `{w:X}` 注釈。既存 BML ツールは無視するので互換性を保つ | §5.4, §7.1 |
| D17 | `#SEAT` はオープナーの席位置。先頭パスは経路に含めず条件にする | §4.4 |

---

## 1. BML の実態 (調査結果)

### 1.1 調査したソース

| ソース | 得られたもの |
| --- | --- |
| `gpaulissen/bml` `README.md` (BML 2, 2021-07) | 公式構文: ビディング表、変数、履歴行、競り合い、`#COPY/#CUT/#PASTE`、`#HIDE`、`#VUL/#SEAT`、`#INCLUDE`、`#+META`、コメント、フォント記法、ディール図 |
| `gpaulissen/bml` `src/bml/bml.py` (753 行) | 実際のパーサ: 段落分割、行とインデントの規則、継続行、クリップボード、`get_sequence()` (暗黙パス挿入)、`bid_type()` (トークン正規表現) |
| `gpaulissen/bml` `src/bml/bss.py` (446 行) | 変数束縛と展開の意味論 (`M/m/oM/om/X/Y/Z/red/black/step`)、席と vul のエンコード、「最初の定義が勝つ」 |
| `gpaulissen/bml` `test/data/example{1..6}.bml` + `test/expected/*.bss` | 正準例と、Python が出力する **展開済みの具体系列** (展開のオラクル) |
| `gpaulissen/bridge-systems` `common/{weopen,1N,1m-(1X),theyopen,1M-fit,2D,abbreviations}.bml` | オランダのパートナーシップ用実システム (著者スタイル A) |
| `jdh8/bridge-systems` `wj.bml`, `wj/{1C,1M}.bml`, `defense/{1X,1NT-STR}.bml`, `common/2X-Multi-Muiderberg.bml`, `common/evaluation.bml` | Polish Club / Blue Club / 防御メモ (著者スタイル B、jdh8 自身のフォークでビルド) |
| `jdh8/bml` フォークの grep | `!` 接頭辞と `#` トークンはどの BML ツールも処理していない (著者独自の慣例) ことを確認 |

**公開されている SAYC / 2/1 の BML ファイルは存在しない** (Polish Club、Blue Club、個人の 5 枚メジャー系のみ)。フェーズ 3 用のテストシステムは `systems/sayc/sayc.bml` として自作する (リスク R1)。

### 1.2 文書化された規則 (README、`bml.py` / `bss.py` で確認済み)

**ファイル構造**。要素は 1 行以上の空行で区切られた段落 (`re.split(r'([ ]*\n){2,}')`)。段落は次の順で分類する: 見出し (`*`, `**`, …)、リスト (`-`)、`#VUL`、`#SEAT`、番号付きリスト (`1.`)、4 列オークション表 (LaTeX 専用)、ビディング表 (`^\s*\(?\d[A-Za-z]+`)、パイプ表、ディール図、メタデータ (`#+KEY: value`、先勝ち、任意の `\w+` キーを受理)、その他の `#` 始まり段落 (ディレクティブ付きビディング表)、それ以外は段落。コメントは **列 0 の `^//.*` のみ** (BML 2)。`#INCLUDE <path>` は包含元ファイルからの相対パスによるテキスト包含。

**ビディング表**。README の例をそのまま示す。

```
1C  Any hand with 16+ hcp
  1D  Artificial. 0--7 hcp
    1H  Any hand with 20+ hcp
  1HS 5+ suit, forcing to game (8+ hcp)
  1N  Natural game force, 8+ hcp
  2CD  5+ suit, forcing to game (8+ hcp)
```

- 行 = `<call> <ws> [= <ws>] <description>`。コール直後の `=` は省略可能な糖衣。
- インデントが木を作る。BML 2 は固定単位 (既定 2 スペース、`-i` オプション) を強制し、`indentation % unit == 0` かつ `indentation / unit == parent.level()` でなければ `IndentationError`。木構築: `indent < ancestor.indent` の間は祖先をポップ、より深ければ子、等しければ兄弟。
- **継続行**: 前の行の説明が始まった桁とインデントが一致する行は、その説明に連結される (`desc += '\n' + line`)。この規則の帰結として、直前のビッドの説明と同じ桁からサブビッドを始めることはできない。

```
1C  Polish club, one of three:
    a) 12--14 NT
    b) 15+ hcp and 5+!c
    c) 18+ hcp any distribution
```

**コールトークン** (`bid_type()`): `P`, `D`, `R`; `[1-7][CDHSN]`; `NT` は `N` に書き換えられる (`1NT` ≡ `1N`)。それ以外は `(\d+)([a-zA-Z]+)$` に一致し、ストレイン部が `[CDHSN]+` (複数ストレイン: `1CD`, `2HS`, `3CDH`)、`M m oM om`、または大文字小文字を区別しない `X Y Z RED BLACK STEP STEPS` のいずれか。相手のコールは括弧付き: `(1N)`, `(D)`, `(P)`。

**変数** (README): `m`/`M` = その系列でまだ使われていない (リテラルでも変数でも) マイナー/メジャー; `om`/`oM` = もう一方 (`m`/`M` の束縛が必要); `X<Y<Z` = 任意のスートで順序付き; `red` = D,H; `black` = C,S (BML 2); `<n>step[s]` = 「親ビッド (直前に行われたビッド)」の n 段上。`bss.py` が意味を確定する: 変数は初出で束縛され部分木で固定; 新規束縛は **どちらの側であれ既にビッドされたストレイン** と **不十分ビッド** を除外; リテラルの複数ストレイン (`2HS`) は **フィルタされない**。`step` は経路を遡って最初に見つかる具体的な **ビッド** (P/D/R ではない) を基準にする。したがって `R` → `(D)` → `2red` → `1step` は 2D の上の 2H になる。変数は説明文の中でも置換される (`\bM\b` → `!h`, `4+M` → `4+!h`)。

**履歴行** (表の先頭行のみ): `-` または `;` を含む先頭行は履歴: `1N-2C;`, `1C-`, `1C---`, `1C-1D;`, `(1NT)---`, `(1N)-P-(P)---`, `1C-(1D)-`。`all_bids() = bid.rstrip(';-').split('-')`。末尾の `-`/`;` の個数は見た目だけ。先頭以外の行は `-`/`;` を含んではならない (assert)。

**暗黙パス** (`get_sequence()`): 経路のトークンを連結し、連続する 2 トークンが同じ側なら相手側のパスを挿入する (我々の 2 コールの間には `(P)`、相手の 2 コールの間には `P`)。先頭パスは決して生成されない: **席は条件であり経路の要素ではない**。`1N` → `2C` は `1N (P) 2C` になる。BSS 出力 `001CP1DP1HP2C` がこれを裏付ける。

**競り合い**: README はパスを含む全てのビッドを記述せよと述べる。バランシングは `(1NT)-P-(P)---`、直接介入は `(1NT)---`。

**`#SEAT s`** は `s ∈ {0,1,2,3,4,12,34}`; **`#VUL we them`** は各々 `Y/N/0`。どちらも段落レベルの粘着的ディレクティブで、以降の表に適用される。BSS は席を **オープニングビッドが行われる位置** としてエンコードする。実ファイルもこの読みを裏付ける: `#SEAT 4` + `(1X)- 1N = !UNT` に「1NT by passed hand」とあるのは、オープナーが 4 席目のときだけ直接オーバーコーラーがパス済みハンドになるから; `#SEAT 34` + `1H- 2C = MAX, 5+!c` に「natural 2/1 by passed hand」。

**`#COPY name … #ENDCOPY`** (本文を残しつつクリップボードへ)、**`#CUT name … #ENDCUT`** (クリップボードのみ、トップレベルに置ける)、**`#PASTE name [tgt=rep …]`**: テキスト置換で、貼り付けられた行には `#PASTE` 行のインデントが前置され、置換は指定順の単純な文字列置換。

```
#CUT transfer
2\R Transfer
  2\M Transfer accept
  3\M Super accept
#ENDCUT

1N---
2C Stayman
#PASTE transfer \R=D \M=H
#PASTE transfer \R=H \M=S
```

**`#HIDE`**: 表を BSS にのみ出力する (本クレートには無関係: 常にコンパイルする)。**`#BIDTABLE`**: 段落を強制的に表として扱う。

**「最初の定義が勝つ」**: 既に定義されたビッドは上書きされず、最初の定義が残る。通常ビッド (`2C`) は特殊ビッド (`2X`) より先に評価される。`bss.py` では系列キーが `seat+vul+sequence` で、後の定義は空の説明を埋めるときだけ効く。

スート記号 `!c !d !h !s` (小文字のみ) は説明文と見出しで使える。フォント記法 `/i/ *b* =mono=`。`--` は範囲の en-dash (`12--14`)。

### 1.3 実ファイルから推定した慣例 (文書にもツールにも無い)

| 観察 | 場所 | 採用した解釈 |
| --- | --- | --- |
| 説明文先頭の `!`: `1D = !NF NEG, 0--9 HCP`, `2N = !UNT, 5+!d, 5+!c`, `3C = ! 4+!d`。一方 `1H = F, 7+ HCP, 4+!h` には無い | jdh8 の全ファイル | **アラート印** (人工的/アラート対象のコール)。全 BML ツールはそのまま描画する。規則: 小文字 `c/d/h/s` が続かない `!` |
| 説明文中の `#`: `3M = !PRE, 3--6 HCP, 7+#`, `4X = !FG SPL, 0--1#, 4+!s`, `(2D)-D-(2HS)- D = PEN, good 4+#` | jdh8 | **「経路上の直近の変数/複数ストレインコールのスート」** (自分のコールを先に、次に祖先、相手の `(2HS)` も含む) |
| `2S/3H = NAT, F`, `4D/H = Texas` | gpaulissen `1M-fit.bml`, `theyopen.bml` | 代替コール (`AnyOf`)。Python は `2S/3H` の末尾 2 文字だけを黙って残し、`4D/H` ではクラッシュする。本実装は両方を受理 |
| `(nX)-3N` | gpaulissen `theyopen.bml` | レベルのワイルドカード `n` (Python はクラッシュ)。`Level::Any` として受理 |
| 末尾の `-`/`;` が **無い** 履歴行の後にインデントされた行: `1N-2C` / `  2D = no 4M` | gpaulissen の全ファイル | `1N-2C;` と同値。Python は `restructure()` の経路 (b) + `to_systemdata()` の履歴展開で処理する |
| `1C-` の後の列 0 継続行 (`example2.bml` のスタイル) と `1N-2C` の後のインデント行 | 両リポジトリ | 両方を受理。1 つの角事例で意図的に逸脱する (§1.4 の 2) |
| 説明文の語彙: WBF 略記の大文字 (`FG INV NF F1 SPL TRF (R) P/C S/O S/T CTRL SOL PRE NAT BAL UNBAL MIN MAX STR WK CONST NEG LIM QUANT PUP STAY UNT`)、長さ `5+!s`, `4=!h` (jdh8: ちょうど), `0--3!s`, `2--4!d`, `6+ suit`, `5+ M`, `4+m`、シェイプ `4414`, `(54)`, `54(31)`, `22(54)`, `33(43)`, `3(433)`, `(54)(xx)`, `55MM`, `5-5 minors`, `4-4 majors`, `5+4+MM`、強さ `12--14 NT`, `ca 15+`, `about 11--12 hcp`, `13+ points`, `9--17 points` | 至る所。`common/abbreviations.bml` が WBF の一覧を再掲 | §7.4 のトークン表を駆動する |
| 1 つの説明文内の列挙: `one of: 1) weak-two in a major 2) 22-24 NT 3) FG in !d`、継続行上の `a) … b) … c) …` | `2D.bml`, README | 列挙項目の選言 |
| hedge: `usually 5+#`, `normally 10+ HCP`, `may have 5M`, `rarely singleton K`, `(?)` | 両方 | ソフト制約 (§7.6) |

### 1.4 Python 実装からの意図的な差分

1. インデント単位は表ごとに推定する (最初の子のインデント)。固定 2 ではなく一貫した任意の単位を受理し、不整合はエラーではなく Lint にする。
2. 末尾記号の無い履歴行に **列 0** の行が続く場合: Python はそれらを新しい *オープニング* として扱うが、本実装は著者の意図どおり継続 (履歴の子) として扱い、`ColumnZeroContinuation` (Info) を出す。
3. Exact 行は *照合時* に行順に関係なくパターン行に勝つ (README の記述意図。`bss.py` は実際には行順で処理する)。
4. `2S/3H`, `4D/H`, `nX`, 裸の `X`/`XX` (= D/R)、小文字の `x/y/z` を受理する (Python: クラッシュか誤読)。それぞれ Lint を付けてファイルの可搬性を保つ。
5. パースエラーは行単位であり、ファイル単位にはしない。

---

## 2. 文法 (EBNF) と AST

### 2.1 EBNF (v1 部分集合。`(* ext *)` は本実装の拡張)

```ebnf
file            = { block } ;
block           = blank | comment | include | meta | seat | vul | heading
                | list | enum | bidtable | paragraph ;

blank           = { " " } , NL ;
comment         = "//" , { CHAR } , NL ;                         (* column 0 only *)
include         = { WS } , "#" , { WS } , "INCLUDE" , { WS } , PATH , { CHAR } , NL ;   (* bml.py: ^\s*#\s*INCLUDE\s*(\S+) *)
meta            = "#+" , KEY , ":" , [ WS ] , { CHAR } , NL ;      (* KEY = [A-Za-z_]+ ; unknown keys kept *)
seat            = "#SEAT" , WS , ( "0"|"1"|"2"|"3"|"4"|"12"|"34" ) , NL ;
vul             = "#VUL"  , WS , TRI , TRI , NL ;   TRI = "Y" | "N" | "0" ;
heading         = "*" , { "*" } , WS , { CHAR } , NL ;
list            = { WS0 , "-" , WS , { CHAR } , NL } ;
enum            = { WS0 , DIGIT , { DIGIT } , "." , WS , { CHAR } , NL } ;
paragraph       = { non-blank-line } ;

bidtable        = { tdirective } , ( history-row | row ) , { row | tdirective } ;
tdirective      = hide | bidtable-kw | copy | cut | paste | stop | anyorder ;
hide            = WS0 , "#HIDE" , NL ;
bidtable-kw     = WS0 , "#BIDTABLE" , NL ;
copy            = WS0 , "#COPY" , WS , NAME , NL , { rawline } , WS0 , "#ENDCOPY" , NL ;
cut             = WS0 , "#CUT"  , WS , NAME , NL , { rawline } , WS0 , "#ENDCUT"  , NL ;
paste           = WS0 , "#PASTE" , WS , NAME , { WS , TARGET , "=" , REPL } , NL ;
stop            = WS0 , "#STOP" , NL ;          (* ext: a system stop after the enclosing row, or at the table's history (§4.5) *)
anyorder        = WS0 , "#ANYORDER" , NL ;      (* ext: the table's fresh X/Y/Z ignore the X<Y<Z order (§4.7) *)

history-row     = WS0 , seq , [ WS , [ "=" , WS0 ] , description ] , NL , { contline } ;
seq             = calltok , { "-" , calltok } , { "-" | ";" } ;    (* at least one "-" or ";" present *)
row             = WS0 , calltok , [ WS , [ "=" , WS0 ] , description ] , NL , { contline } ;
contline        = INDENT(= description column of previous row) , { CHAR } , NL ;

calltok         = "(" , callcore , ")" | callcore ;
callcore        = "P" | "D" | "R"
                | "X" | "XX"                                        (* ext: double/redouble, bare only *)
                | level , strainspec
                | level , ( "step" | "steps" )                      (* case-insensitive *)
                | callcore , "/" , ( callcore | strainspec ) ;      (* ext: 2S/3H, 4D/H *)
level           = "1".."7" | "n" | "c" | "j" ;                      (* "n" ext: any level; "c"/"j" ext: cheapest / jump (§4.6) *)
strainspec      = literal | variable | "red" | "black" ;            (* red/black case-insensitive *)
literal         = "NT" | "N" | ( "C"|"D"|"H"|"S" ) , { "C"|"D"|"H"|"S" } ;   (* 1CD, 2HS, 3CDH *)
variable        = "M" | "m" | "oM" | "om" | "X" | "Y" | "Z" | "x" | "y" | "z" ;

description     = [ "!" , not-suit-letter ] , { CHAR } ;            (* "!" = alert marker (inferred) *)
```

説明文の内容は別の文法を持つ (§7.3)。説明文の `{stop}` 注釈 (ext) はその行の後にシステム停止を置く (§4.5)。`seq` の末尾記号が無い履歴行 (§1.3) は、先頭行のトークン列に `-` を含むことで `history-row` と判定する。

### 2.2 AST (`ast.rs`)

```rust
// ast.rs
pub struct FileId(pub u16);                    // 0 = ルート、#INCLUDE されたファイルが続く

pub struct Span {
    pub file: FileId,
    pub line: u32,                              // 1-based
    pub col: u16,                               // 0-based
    pub pasted_from: Option<(Arc<str>, u32)>,   // (#PASTE 元のクリップボード名, 元の行番号)
}

/// include 解決後の物理行 1 本。
pub struct RawLine { pub span: Span, pub text: String }

pub struct BmlFile {
    pub root: FileId,
    pub files: Vec<(Arc<str>, Arc<str>)>,      // (path, text)、FileId で添字付け (spans / hashing 用)
    pub blocks: Vec<Block>,
    pub lints: Vec<Lint>,                       // parse-stage diagnostics
}

pub enum Block {
    Meta      { key: String, value: String, span: Span },
    Seat      { cond: SeatCond, span: Span },
    Vul       { cond: VulCond,  span: Span },
    Heading   { level: u8, text: String, span: Span },
    Paragraph { text: String, span: Span },
    List      { items: Vec<String>, ordered: bool, span: Span },
    BidTable  (BidTable),
    Clipboard { name: String, lines: Vec<RawLine> },               // top-level #CUT
}

pub struct BidTable {
    pub hidden: bool,               // #HIDE (コンパイラは無視。ツール用)
    pub seat: SeatCond,             // この表に効いている粘着的ディレクティブの値
    pub vul: VulCond,
    pub history: Vec<CallToken>,    // オープニング表なら空、それ以外は先頭行の系列
    pub history_desc: Option<Description>,   // 履歴行に書かれた説明 (最後のコールに帰属)
    pub rows: Vec<BmlNode>,         // トップレベル行 (履歴の子、またはオープニング)
    pub stop: bool,                 // 表の最上位の #STOP: 履歴の位置のシステム停止 (§4.5)
    pub span: Span,
}

pub struct BmlNode {
    pub calls: Vec<CallToken>,      // 1 つ。ただし 2S/3H の代替は 1 トークンのまま保持
    pub description: Description,
    pub children: Vec<BmlNode>,
    pub indent: u16,
    pub stop: bool,                 // 子の位置の #STOP: この行の後のシステム停止 (§4.5)
    pub span: Span,
}

pub struct Description { pub text: String /* lines joined with '\n' */, pub alert: bool, pub col: u16 }

pub struct CallToken { pub side: Side, pub pattern: CallPattern, pub raw: String, pub span: Span }

#[derive(Default)] pub enum SeatCond { #[default] Any, First, Second, Third, Fourth, FirstOrSecond, ThirdOrFourth }   // #SEAT 0 1 2 3 4 12 34
impl SeatCond { pub const fn matches(self, position: u8) -> bool; pub const fn specificity(self) -> u8; }   // 0 / 1 / 2
#[derive(Default)] pub enum Tri { Yes, No, #[default] Any }                                                 // Y N 0
impl Tri { pub const fn matches(self, v: bool) -> bool; }
#[derive(Default)] pub struct VulCond { pub we: Tri, pub they: Tri }
impl VulCond { pub const fn matches(self, we: bool, they: bool) -> bool; pub const fn specificity(self) -> u8; }   // Any でない側の数 0..=2
```

`BidTable.seat` / `vul` は直前の `#SEAT` / `#VUL` の値を表ごとに焼き込む (粘着的ディレクティブの解決は AST 構築時に終える)。`Description.alert` は §1.3 の `!` 印。`Description.col` は継続行判定に使う説明開始桁。`SeatCond`/`Tri`/`VulCond`/`FileId`/`Span` は `serde` 派生を持つ (IR に埋め込まれるため)。`RawLine` は持たない。

---

## 3. パースパイプラインと回復規則

### 3.1 段階 (`lexer.rs`, `parser/{mod,call,clipboard}.rs`)

各段階は全体として失敗せず、失敗を `Lint` にして続行する。全体は `compile(root_path, source, loader, opts)` (§9.4) が駆動する。

1. **読込と `#INCLUDE` 解決** (`lexer::load`): ルートのテキストを受け取り、`#INCLUDE` 行を再帰的に解決 (循環ガード、深さ ≤ 16) して 1 本の `Vec<RawLine>` を作る。列 0 の `//` 行はここで落とす。`#INCLUDE` の認識は `bml.py` の `^\s*#\s*INCLUDE\s*(\S+)` に合わせる (行頭の空白・`#` の後の空白を許し、パスはキーワード直後の最初の語のみ)。`bml.py` は指令を `'\n' + text + '\n'` で置換するので、包含したファイルの前後には合成の空行 (段落区切り) を 1 行ずつ挿入し、連続した `#INCLUDE` でもファイル同士の段落が融合しないようにする。パスは `/` 区切りで正規化し、先頭の `/` と、取り除く実セグメントの無い先頭の `..` は保持する。ルートのパスも同じく正規化してから循環ガードに積む。存在しない include は `Lint::IncludeNotFound` (Warning) で行を落とす。循環は `Lint::IncludeCycle` (Error) で当該 include を無視する。読込は `SourceLoader` トレイト経由にし、`wasm32` でも `std::fs` 無しで include が動くようにする。

   ```rust
   // lexer.rs
   pub trait SourceLoader {
       /// `from` (包含元ファイルのパス) からの相対パス `path` のテキスト。
       fn load(&self, from: &str, path: &str) -> Result<String, String>;
   }
   pub struct FsLoader;                                   // std::fs。from のディレクトリからの相対パス
   pub struct MemLoader { pub files: Vec<(String, String)> }   // path → text。テスト・wasm・組み込みシステム用

   pub struct Loaded {
       pub files: Vec<(Arc<str>, Arc<str>)>,              // (path, text)、FileId で添字付け
       pub lines: Vec<RawLine>,                           // 読み順。コメント行除去・include 展開済み
       pub lints: Vec<Lint>,                              // 欠落・循環 include
   }
   pub fn load(root_path: &str, root_text: &str, loader: &dyn SourceLoader) -> Loaded;
   pub fn paragraphs(lines: &[RawLine]) -> Vec<Vec<RawLine>>;
   pub const ROOT: FileId = FileId(0);
   ```

2. **段落分割** (`lexer::paragraphs`): 1 行以上の空白のみの行で区切る。
3. **分類** (`parser::classify(&[RawLine]) -> ParagraphKind`): §1.2 の順序 (`get_content_type` と同じ)。`ParagraphKind { Heading, List, Enumeration, Seat, Vul, BidTable, Meta, Directive, Paragraph }`。不明な `#DIRECTIVE` は `Lint::UnknownDirective` (Warning) で行を除去し、段落の残りは処理する。
4. **クリップボード展開** (`parser::clipboard::expand(paragraph, &mut Clipboard) -> (Vec<RawLine>, Vec<Lint>)`、表段落の内部、Python と同じ順): 全ての `#CUT` ブロックを取り出し、次に `#COPY` (本文は残す)、最後に `#PASTE` を展開する (テキスト置換、`Span.pasted_from` を保持)。`Clipboard { blocks: Vec<(String, Vec<RawLine>)> }` はファイル全体で大域的かつ順序依存 (include はテキスト包含なのでファイル横断でも動く)。`#PASTE` の置換は `tgt=rep` を指定順に単純置換し、貼り付け行には `#PASTE` 行のインデントを前置する。未定義名は `Lint::PasteUnknownName` (Warning)。同じ名前の再定義 (`#CUT`/`#COPY`) は Python の辞書代入と同じく **後の定義で上書き** し、以後の `#PASTE` は最新の本文を貼る。貼り付けた本文中の `#PASTE` も Python の `while True` 再走査と同じく展開する (インデントは累積)。自己参照に備えて入れ子は 16 段までとし、超えた `#PASTE` 行は `UnknownDirective` (Warning) で落とす。終端 `#ENDCUT`/`#ENDCOPY` と `#HIDE`/`#BIDTABLE` は Python の `#ENDCUT[ ]*` と同様に末尾空白を許す。終端の無い `#CUT`/`#COPY` の Lint には開始行の位置を付ける。
5. **行パース** (winnow, `parser/call.rs`): 残った各行の `indent` を計算し、`indent > 0 && indent == prev_row.description.col` なら継続行。それ以外は `calltok` をパースし、`WS [= WS] description` を読む。

   ```rust
   // parser/call.rs
   pub fn calltok(input: &mut &str) -> ModalResult<(Side, CallPattern)>;          // 括弧付きなら Side::Them
   pub fn history(input: &mut &str) -> ModalResult<Vec<(Side, CallPattern)>>;    // `1N-2C;`, `(1NT)---`, `1C-(1D)-`
   ```

   `calltok` は winnow の `alt` で `XX` → `P|D|R|X` → `level (steps | strainspec)` の順に試し、続く `/alt` を `CallPattern::AnyOf` にまとめる (`X` は `Double`、`XX` は `Redouble`)。
6. **木構築** (`parser::parse(loaded: Loaded) -> BmlFile`): インデントをキーにした開いている行のスタック (Python の意味論、寛容版)。先頭行が履歴行であるのは、そのトークン文字列が (パース前に) `-` か `;` を含むとき。履歴行の後の行は、インデントされていても列 0 でも履歴の子とする (§1.4 の 2。列 0 かつ末尾記号なしのときは `ColumnZeroContinuation` Info)。先頭以外で `-`/`;` を含む行は `Lint::SequenceNotFirst` (Error) で行と部分木を捨てる。インデント単位は表ごとに最初の子のインデントから推定する。
7. **回復**: 次節。

性能は問題にならない (ファイルは数十 KB)。winnow を使う目的はスループットではなく、コールトークンと説明文文法のエラー位置を正確に出すことにある。

### 3.2 回復規則

| 事象 | 処置 | Lint |
| --- | --- | --- |
| コールトークンがパースできない | 生テキストを添えて記録。その行とインデントされた部分木をスキップし、スキップした行数をメッセージに含める | `UnknownCallToken` (Warning) |
| インデントが開いている祖先のどれとも一致しない (例: 2 と 4 の間の 3) | 最寄りの浅い行の子として付ける | `IndentationMismatch` (Warning) |
| 末尾記号の無い履歴行の後の列 0 行 (§1.4 の 2) | 履歴の子として付ける | `ColumnZeroContinuation` (Info) |
| 説明文が空 | 許容。コンパイル時に Info | `EmptyDescription` (Info) |
| 拡張トークン (`X`, `XX`, `2S/3H`, `4D/H`, `nX`, `x/y/z`, `(any)`, `cS`/`jY` (§4.6)) | 受理 | `NonStandardToken` (Info) |
| 段落の先頭語が相対レベルのトークン (`cS is …`) | 表ではなく段落として扱う (相対レベルは直前のビッドが無いと意味を持たないので表を始めない) | 先頭行が行の形 (`cS = …`) なら `UnknownCallToken` (Warning、後続の行も散文になるため)。それ以外はなし |
| 先頭以外の行に `-`/`;` | 行と部分木をスキップ | `SequenceNotFirst` (Error) |
| include の欠落 / 循環 | 行を落とす / include を無視 | `IncludeNotFound` (Warning) / `IncludeCycle` (Error) |
| 不明な `#DIRECTIVE` | 行を除去し段落の残りを処理 | `UnknownDirective` (Warning) |
| 表の指示子 (`#ANYORDER`、`#STOP`) だけの段落 (空行の後に表) | 表を名指さないので効果なし | `UnknownDirective` (Warning) |
| `#PASTE` の未定義名 | 行を除去 | `PasteUnknownName` (Warning) |

公開 API は `lexer::load` → `parser::parse` の 2 段で、単独の `parse_str` は無い。通常は `compile` (§9.4) を使い、AST だけが要るツール (`insta` スナップショット) は `parser::parse(lexer::load(path, text, &loader))` と書く。

---

## 4. `CallPattern` と展開

### 4.1 型 (`pattern.rs`)

```rust
// pattern.rs
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub enum Side { Us, Them }

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Var { Major /*M*/, OtherMajor /*oM*/, Minor /*m*/, OtherMinor /*om*/, X, Y, Z }

/// 5 bit のストレイン集合 (C D H S N)。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)] pub struct StrainSet(pub u8);
impl StrainSet {
    pub const EMPTY: StrainSet; pub const RED: StrainSet /* D H */; pub const BLACK: StrainSet /* C S */;
    pub const MAJORS: StrainSet; pub const MINORS: StrainSet;
    pub const fn contains(self, strain: Strain) -> bool;
    pub const fn with(self, strain: Strain) -> StrainSet;
    pub fn iter(self) -> impl Iterator<Item = Strain>;   // ビッド順 C D H S N
}
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub enum Level { At(u8), Any /* "n" */, Cheapest /* "c" */, Jump /* "j" */ }   // c/j: §4.6
impl Level { pub const fn is_relative(self) -> bool; }   // Cheapest | Jump

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum CallPattern {
    /// P, D, R, 1C..7N (1NT ≡ 1N). Also ext: X, XX.
    Exact(Call),
    /// Literal multi-strain: 1CD, 2HS, 3CDH, 2red, 2black. No binding, no "unused" filter.
    Strains { level: Level, strains: StrainSet },
    /// Variable strain: 1M 2m 1oM 2om 1X 1Y 1Z. Binds on first use in a path; filtered by
    /// "strain not yet bid by either side", sufficiency, X<Y<Z.
    Var { level: Level, var: Var },
    /// 1step / 2steps: n steps above the last *bid* (not P/D/R) in the auction, either side.
    Step(u8),
    /// ext: 2S/3H, 4D/H
    AnyOf(Vec<CallPattern>),
    /// ext: (any) (bid) (suit): opponents' interference classes, kept as trie wildcard edges.
    Class(OppClass),
}
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum OppClass { AnyCall, AnyBid, AnySuitBid, AnyBidAtLevel(u8), Double, Pass }

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SidedPattern { pub side: Side, pub pat: CallPattern }

/// 1 つの展開における変数の束縛。
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct Binding { pub minor: Option<Strain>, pub major: Option<Strain>, pub x: Option<Strain>, pub y: Option<Strain>, pub z: Option<Strain> }
impl Binding {
    pub fn get(&self, var: Var) -> Option<Strain>;                       // oM/om は M/m から導く
    pub fn candidates(&self, var: Var, used: StrainSet) -> Vec<Strain>;  // 未使用 ∧ X<Y<Z の順序制約
    pub fn bind(self, var: Var, strain: Strain) -> Binding;
}
```

仕様 §5 の `CallPattern { Exact, AnyBid, AnySuit, Relative(i8), Opponent(OppClass) }` との対応: `1CD/1HS` は `Strains`; `1M/1m/1X` は `AnyOf` ではなく `Var` (BML は **束縛する**。README の「`1C` の後の系列で `m` を使えば `m` はダイヤになる」、jdh8 の Stenberg 表の `1M-2N-` 以下の `3oM` はこれに依存する); `AnyBidAtLevel(u8)` は 1 回だけ使う `Var{X}`; `Relative(i8)` は `Step(u8)` (BML に負のステップは無い)。相手であることはパターンの variant ではなく `Side` で表す: 暗黙パス挿入後は側が交互になるので、側は位置と「どちらがオープンしたか」で完全に決まる。

`StrainSet` の走査順は C, D, H, S, N。`OppClass` の各値は `Call` に対する述語 (`AnyCall` は全て、`AnyBid` は `Call::Bid(_)`、`AnySuitBid` は `Bid` かつ `strain != NoTrump`、`AnyBidAtLevel(l)` は `Bid` かつ `level == l`、`Double`/`Pass` はそのコール)。`Class` は BML 構文には無く、`(any)` / `(bid)` / `(suit)` の拡張トークンとしてのみ書ける (`NonStandardToken`)。

### 4.2 展開: パターン → 具体トライ辺 (`compile/mod.rs`、`bss.py::systemdata_bidtable` の移植)

**方針: 行 (`Row`) はパターン形のまま IR に残し、索引は即時展開した具体トライにする。** 理由:

1. 束縛ごとに制約が本当に異なる (`1M = 5+ M` は `suit_len[H]` か `suit_len[S]`)。展開ごとの具体的な `HandConstraint` はどのみち必要。
2. ファンアウトは有界 (新規変数 1 つあたり ≤ 4、再利用される変数は増やさない)。3,000 行のシステムでおよそ行数の 2〜3 倍、数 MB に収まる。上限は `CompileOptions.max_nodes` (既定 50,000) で、超えたら `TooManyNodes` (Error) で展開を打ち切る。
3. 照合が純粋なトライ走査になり (§6)、10 μs の目標に余裕で収まる。
4. `gpaulissen/bml/test/expected` の `.bss` 期待出力がこの段階の厳密なオラクルになる (D7)。

ワイルドカード辺が存在するのは `Class` 拡張のときだけ。

**アルゴリズム** (表ごとの DFS)。状態 = `env: Binding`、`used: StrainSet` (どちらかの側が既にビッドしたストレイン)、`auction: Auction` (bridge-core、ディーラーは North 固定、我々 = `we_opened` に応じて N/S か E/W)、`cond: (SeatCond, VulCond)`、`path: Vec<SidedPattern>`、`calls: Vec<Call>`。

1. 履歴行から始める: そのトークンを左から右へ、通常行と全く同じ規則で展開する (履歴で束縛された変数、例えば `(1Y)-` や `1M-2N-`、は全ての行から見える)。履歴行に説明があれば最後のコールに帰属させる (Python もそこへ移す)。
2. 兄弟リストの処理順は **Exact 行を先に (行順)、次にパターン行 (行順)**。各群の中では先勝ち (§1.4 の 3)。
3. パターン `p` を持つ行 `r` の候補コール:
   - `Exact(c)` → `[c]`。
   - `Strains{level, set}` → `level` における C,D,H,S(,N) 順のストレイン。`Level::Any` → 1..7 の全レベル (候補が 8 を超えたら `Lint::WideWildcard`)。
   - `Var{level, v}`: `env.get(v)` が `Some` → `[(level, strain)]`; `v ∈ {oM, om}` で `M`/`m` が未束縛 → `Lint::UnboundOther` (行をスキップ); それ以外は `env.candidates(v, used)` (領域 `M`: H,S; `m`: C,D; `X/Y/Z`: C,D,H,S を (a) `used` に無いストレイン、(c) 束縛済みのものに対して `X<Y<Z` でフィルタ) をさらに (b) `auction` の最終ビッドより上でフィルタし、各候補で `env.bind(v, strain)` を部分木の間だけ使う。候補なし → `Lint::VariableNoCandidate` (Info。Python は「Could not find a bid」をログする)。
   - `Step(n)` → `auction` の最終ビッド + n (ストレイン順 C<D<H<S<N をまたぐ)。最終ビッドが無い → `Lint::StepWithoutAnchor`。
   - `AnyOf(ps)` → 上記の和 (順序どおり)。
   - `Class(k)` → 具体コールを持たない単一のワイルドカード辺。部分木は `used`/`auction` を変えずに展開する (コールが未知のため)。したがってワイルドカードの下では `Step` と新規変数を禁止する (Lint)。ワイルドカードより下の合法性は「ワイルドカードが表しうるコールの少なくとも 1 つについて合法」で判定する (緩和した合法性: ビッドは既知の最終ビッドより上、ダブルは直前の非パス候補が相手のビッドまたはビッドを含むクラス、リダブルは相手のダブルまたはダブルを含むクラス、オークション終了は確実な場合のみ)。`Node.calls` と `resolved` ではワイルドカード位置に `Pass` の埋め草を置くが、兄弟の重複判定 (Exact 優先・`bids_processed`) はコールではなくトライ辺 (`Edge`) で行うので、ワイルドカードが実際の `(P)` 行や別クラスの兄弟と衝突することはない。
4. 各候補 `c` について: 同じ兄弟リストの Exact 行が既に `c` を生成していれば (Exact 優先規則) `Lint::ShadowedByExact` (Info) でスキップ; パターン行の候補が先行する *パターン* 兄弟の生成済みコールと一致すれば、`bss.py` の `bid not in bids_processed` と同じく Lint なしで (ノードを作る前に) スキップ (`1M …` の後の包括的な `1X …` の慣用); `auction.is_legal(c)` を検査し、違反は `Lint::IllegalCall` (Error) で部分木を捨てる; `r.side` が直前のコールの側と同じなら先に暗黙パスを挿入 (§4.3); 説明文の変数 (`\bM\b`, `(\d+)M\b`, `oM`, `m`, `om`, `X/Y/Z`, `#`) を置換して `description` を作る; この具体経路の文脈で説明文をコンパイル (§7); `Node` を生成; `AuctionTrie::insert(we_opened, &calls, seat, vul, node)` でトライへ挿入; `Err(existing)` (同一条件のエントリが既にある) なら最初のものを残し `Lint::DuplicatePath` (両方の説明が非空で異なれば Warning。最初のエントリの説明が空なら Python と同様に *埋める*)。
5. 更新した状態で子へ再帰する。戻るときに変数の束縛を解き、`used`/`auction` を復元する。

**オラクル**: `systems/vendor/data/bml-test/` に取得した `example{1..6}.bml` を展開し、生成した具体系列の集合 (席・vul・`we_opened` 込み) が対応する `.bss` と完全一致することを統合テストで確認する。`.bss` の系列表記 (`001CP1DP1HP2C`: 先頭 2 桁が席と vul、`*` 接頭辞が相手オープン、以降がコール列) からの復号は `tests/bss.rs` の補助関数に閉じ込める。

### 4.3 暗黙パス

`get_sequence` の移植: 具体経路を歩き、次のコールの側が直前のコールの側と同じなら反対側に `Pass` を挿入する。競り合いなしの `1N` → `2C` → `2D` は `1N (P) 2C (P) 2D` の 5 コールになる (BSS `001NP2CP2D` と一致)。暗黙パスはトライノードを持つが **`Node` は持たない** (行が無いので): `Lookup.by_depth[i] == None`。したがって `Node.calls` はオープニング以降の実際のコール 1 つにつき具体 `Call` を 1 つ持つ。

**補集合の前計算 (フェーズ 4)。** 上の「暗黙パス」(展開時にトライへ挿入する相手側の `Pass`) とは別に、`choose_bid` の `ImplicitPass::Complement` が合成する `Pass` は「兄弟のどれも満たさない手」を示す。この補集合は §5.4 の派生索引が、兄弟グループ × 条件クラスごとに `ExclusiveGroup::complement` として前計算する。形は素な原子の平坦な `Or` で、48 原子を超えるときだけ木 `Not(Or(...))` に退避する (集合としては同じ)。グリッド (`05-constraint.md` §2.6) で空と証明できる補集合は `Or([])` にする。SAYC では 714 グループのうち 711 が平坦、3 が木である (フェーズ 4 のシステム停止 §4.5 の後は 2,754 グループのうち木が 3。停止のパスを持つグループの補集合は空である)。

### 4.4 先頭パス、席、バルネラビリティ (D17)

**先頭パスは決して経路の一部にしない。** 条件として扱う。

```rust
pub enum SeatCond { Any, First, Second, Third, Fourth, FirstOrSecond, ThirdOrFourth }   // #SEAT 0 1 2 3 4 12 34
pub struct VulCond { pub we: Tri, pub they: Tri }   // #VUL YN etc.; Tri = Yes | No | Any
```

`SeatCond` は **オープナーの席位置** を指す (1 = ディーラーがオープン; 先頭パス k 個 ⇒ 位置 k+1)。BSS と §1.2 の実ファイルの証拠に従う。`VulCond` はシステム所有者のパートナーシップから見た相対値。

条件の照合と特定度:

| 条件 | 一致する `opener_pos` | 特定度 |
| --- | --- | --- |
| `Any` | 1..=4 | 0 |
| `FirstOrSecond` / `ThirdOrFourth` | {1,2} / {3,4} | 1 |
| `First` … `Fourth` | その 1 つ | 2 |
| `VulCond` | `we`/`they` の各 `Tri` が `Any` か一致 | `Any` でないフィールド数 (0..=2) |

エントリの特定度 = `seat.specificity() * 3 + vul.specificity()` (0..=8)。最大の特定度を持つエントリが勝ち、同点は先定義 (コンパイル時に `ConditionTie` Lint)。

履歴トークンの再トレース (`#SEAT 34` の表の `1H-` など) は説明が空の「プレースホルダ」エントリを作る。一般定義 (`1H 5+!h, …`) が先にあれば、それを覆う条件のプレースホルダは作らない。逆順 (`#INCLUDE` の順序などでプレースホルダが先) のときは、後から挿入される非空の定義が、その条件に覆われる空のプレースホルダを自分の内容で *埋める* (プレースホルダ自身の id・子・`#SEAT`/`#VUL` 条件は保持し、`DuplicatePath` Info)。これで照合結果はファイル順に依存しない。ただし埋める前に展開済みのプレースホルダの子は、`ANY` 制約の文脈でコンパイルされたままになる (既知の制限)。

**照合キーの構築** (`trie.rs`。L3 が呼ぶ):

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RelVul { pub we: bool, pub they: bool }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LookupKey<'a> {
    pub we_opened: bool,        // did the system owner's partnership make the first non-pass call
    pub calls: &'a [Call],      // auction.calls with the k leading passes stripped
    pub opener_pos: u8,         // k + 1  (1..=4)
    pub vul: RelVul,            // relative to the system owner
}
impl<'a> LookupKey<'a> {
    /// Strip leading passes; returns None for a passed-out or empty auction.
    pub fn for_auction(auction: &'a Auction, owner: Seat) -> Option<LookupKey<'a>>;
}
```

`for_auction` の手順: (1) `k = auction.leading_passes()`; `calls.len() == k` (空かパスアウト) なら `None`。(2) `opener = auction.seat_at(k)`; `we_opened = opener.side() == owner.side()`。(3) `calls = &auction.calls()[k..]`、`opener_pos = auction.position_of(opener)` (= `k + 1`)。(4) `vul.we = auction.vulnerability().is_vulnerable(owner)`、`vul.they = is_vulnerable(owner.next())`。`SystemIR::resolve(auction, owner)` は `for_auction` と `index.resolve` をまとめた便宜関数。

トライは `we_opened` の真偽で **2 つのルート** を持つ (BSS の `*` 接頭辞に対応)。したがってオープナーの側も経路の要素ではない。

### 4.5 システム停止 (`#STOP`、`{stop}`、フェーズ 4)

**目的。** 我々のパスは、行がそれを書いた所にしかトライの辺を持たない。したがって、パートナーシップが競りを止めた後の手番 (相手が何か言った後のパートナー、パートナーのパスの後の自分) ではシステムを外れ、ナチュラル推論が答える。フェーズ 4 の SAYC はこれを「パスの鎖」で書き出していた: クリップボード `pass-chain` (`P = {prio:-100} any hand`、その下に `(any)` と `P` を 6 巡) と `after-chain` (`(any)` から始まる同じ鎖。我々の最後のコールの行の下に貼る) を 2,543 か所に `#PASTE` し、38,737 行 / 49,800 ノードのうち約 87% を鎖が占めた。コンパイル 0.9〜1.0 s、排他索引 45〜47 ms、postcard IR 16.4 MB で、`tests/compile_time.rs` の `< 1 s` が落ちていた。システム停止は、同じ意味を 1 行の印で書き、鎖のノードを作らずにトライの構造で表す。

**BML 拡張 (`(* ext *)`)。**

- `P = {prio:-100} {stop} any hand`: 説明の `{stop}` 注釈 (`{prio:N}` と同じく説明文のどこに書いてもよく、本文からは取り除く。括弧の内側の空白は許す (`{ stop }`)。`{stopper}` などは該当しない) が付いた行は、自分自身の後に停止を置く。`pass-chain` を貼っていた位置の置き換えで、この行自体は普通の行 (最下位のパス) としてコンパイルする。
- `#STOP`: 行の子の位置 (その行より深い字下げ) に書くと、その行の後に停止を置く。`after-chain` を貼っていた位置の置き換えである。表の最上位に書くと、表の履歴 (`1C-1H-` など) の位置に停止を置く。履歴の無い表の最上位の `#STOP` は位置を名指さないので無視し、`UnknownDirective` Warning を出す。
- 停止の位置のノードには `NodeFlags::stop` を立てる (情報用。選択や解釈は読まない)。

**意味。** 位置 S の停止は、S の下に次の行を *際限なく* (表の `#SEAT` / `#VUL` の条件で)、*全ての表の後に* (システムの末尾に) 書いたのと同じである。

```text
(any)
  P = {prio:-100} any hand
    (any)
      P = {prio:-100} any hand
        ...
```

つまり、S からは相手の `(any)` (ワイルドカード辺) と我々の `P` が交互に続き、我々の手番では「どんな手でもパス」が `{prio:-100}` の最下位の候補として出る。鎖と違って 6 巡で尽きない。

「全ての表の後」なので、表が書いた行とワイルドカード辺は、ファイル上の順序にかかわらず停止に優先する (貼った鎖は先に書かれていれば後の表を覆い隠した。ここが鎖と違う)。

- S で後の表が書いた `(bid)` / `(suit)` / `(X)` などの辺は、停止の `(any)` より先に試される。照合は後戻りしないので、停止はその辺の部分木には入らない (部分木で停止させたいなら、そこにも `#STOP` を書く)。
- 停止のたどる位置に表が自分で書いた我々のパスの行は、その優先度と説明を保つ (空のプレースホルダだけは停止のパスで埋める)。停止はその行を通って先へ続く。
- 回帰テスト `tests/stop.rs::written_rows_and_wildcards_take_precedence_over_a_stop`。

**トライへの接ぎ木 (`compile/expand.rs::graft_stops`)。** 展開は停止の位置 `StopSite { we_opened, edges, seat, vul, span }` を記録するだけで、展開の最後 (不正コールの降格の前) にまとめてトライへ接ぐ。

1. (席条件、バル条件) の組ごとに合成ノードを 1 対だけ作る (`StopNodes`): 相手ノード (`Side::Them`、空の説明、`ANY`) と停止のパス (`Side::Us`、`Pass`、`{prio:-100}`、`ANY`、説明は `{prio:-100} {stop} any hand` から注釈を除いた `any hand`、`flags.stop` と `flags.synthesised`)。
2. 共有の分離トライノードの輪 `StopLoop` は、ある辺に着いた停止の条件の列 (書かれた順) ごとに 1 つ作る: 相手のノード `any_trie` と我々のノード `pass_trie` に、列の各条件について鎖の行が加えたはずのエントリを置き (停止のパスは同じ条件のエントリが無ければ、`(any)` は覆う条件のエントリが無ければ)、`any_trie --Pass--> pass_trie` と `pass_trie --(AnyCall)--> any_trie` で輪にする。これが際限の無さを表す。トライは木でなくなるが、`resolve` / `children` / `resolve_lenient` は辺をたどるだけなので変更は要らない。条件が 1 つだけの系 (SAYC など) では輪は 1 つである。
3. 各位置 S から、次の手番の側 (`(edges.len() が偶数) == we_opened` なら我々) に応じて `P` の辺と `(any)` の辺を交互にたどる。辺が既にある所 (表が自分で `(any)` や我々の `P` を書いている所) は既存のノードを通り、鎖の行が加えたはずのエントリを加える: `(any)` のエントリは覆う条件のエントリが無ければ加え、停止のパスは同じ条件のエントリが無ければ加える。同じ条件のエントリがあり、その説明が空のプレースホルダ (§4.4) なら停止のパスの内容で埋める (`DuplicatePath` Info)。停止のパスの条件に覆われる空のプレースホルダも同様に埋める。
4. 最初に辺が無い所で、その停止の条件だけの輪の `pass_trie` (我々の手番) か `any_trie` (相手の手番) へつなぐ。既に輪のノードに着いたら、そこで止める。その輪が停止の条件を含まなければ、辺を「その輪の条件の列 + 停止の条件」の輪へつなぎ替える (元の輪は、そこへつながる他の位置のために残す)。こうして、席やバルの条件が違う停止が同じ位置で出会っても、どの条件でも停止のパスが出る (回帰テスト `tests/stop.rs` の `stops_under_different_seat_conditions_at_one_position_each_keep_the_stop_pass` ほか 2 件。鎖と停止を比べる)。

**解決の結果。** 消費側 (`choose_bid`、`ExclusiveIndex`、`interpret`、`AuctionPolicy`、説明、ハーネス) は、鎖の行が作っていたのと同じものを見る。

- 停止の後の我々の手番では、`children` の結果に停止のパス (`Pass`、priority −100、`ANY`) が並ぶ。他の行があればその下位の候補で、他の候補がどれも当てはまらないときだけ選ばれる。排他領域 (§5.4) は `ANY` から上位の全メンバーを引いたもの。
- 相手の具体的なコールに表があれば、その辺がワイルドカードに勝つ (照合は後戻りしない)。例えば `1C-1N-(2S)-` に表があれば、`1C P 1N 2S` の子はその表の行だけで、停止のパスは出ない。
- 停止の後で我々がパス以外のコールをすると、それは行ではないので照合はその手前で止まり (`matched_depth` はそのコールの手前)、以後はシステム外 (ナチュラル) になる。鎖と同じである。
- 合成ノードは `flags.synthesised` を持つ (`Node::is_synthesised()`。`path` と `calls` も空だが、手組みの IR も `path` が空なので、判定は印で行う)。停止のパスの内容で埋めたプレースホルダは合成ノードではない。行 (`Row`) も合成で、`span` は file 0・行 0、認識率 1.0。`Lookup.by_depth` には合成ノードが入る。兄弟の曖昧さの Lint (`check_sibling_ambiguity`) は合成ノードを根にしない。
- 等価性: 鎖を 8 巡書いた系と停止で書いた系を比べる `tests/stop.rs` で、ランダムな 46,570 位置 (うち停止のパスを出すもの 15,208) の照合結果 (深さ、ワイルドカード、各深さのノード、子) が一致する。SAYC 全体では、鎖の 6 巡を超える位置だけが異なる (鎖は 6 巡で尽き、停止は尽きない)。停止の接ぎ木を 6 巡に制限した試験版では、`xtask coverage` の全指標と、生成したリプレイ 24,269 位置での `choose_bid`・`call_distribution`・`interpret` の尤度 (|Δ ln p| の最大 0.0) がフェーズ 4 の鎖版と完全に一致した。

**費用 (SAYC、release)。** 38,737 行 / 49,800 ノード → 6,496 行 / 7,174 ノード (合成ノード 2)。コンパイル 914〜941 ms → 437〜441 ms (best of 3、loadavg 3.2〜3.4)。排他索引の再構築 44.7 ms → 12.8 ms (§5.4)。postcard IR 16,415,084 → 2,524,018 バイト。コンパイル 1 回のピーク RSS 158 MB → 33 MB (`/usr/bin/time -l`)。`IR_FORMAT` は 2、`COMPILE_REVISION` は 3 (§10.2)。レビュー修正 (条件ごとの輪、`NodeFlags::synthesised`、合成ノードの説明) の後は `IR_FORMAT` 3、`COMPILE_REVISION` 5、postcard IR 2,531,202 バイト (SAYC のトライとノードは、合成ノードの印と説明文を除いて同一)。説明文の注釈をコンパイル時に 1 度だけ除くようにして `COMPILE_REVISION` 6 (§10.2。レーン D2 のマージ後は 9)、postcard IR 2,476,329 バイト。


### 4.6 相対レベル (`cS`、`jY`、フェーズ 4 拡張)

**目的。** BML のレベルは `1`〜`7` か `n` (全レベル) しか書けないので、「相手のスートより上なら 2 レベル、下なら 3 レベル」のように、直前のビッドによって最小レベルが変わるコールは、相手のスートやレベルごとに表を書き分けるしかなかった (SAYC の P10 の 6 組 18 表、`competition.bml` の「下位スートのオーバーコールはオープニングごとに書き出す」)。相対レベルは、そのコールを 1 行で書く。

**構文。** `level` に 2 つの値を加える (§2.1)。

- `c` (cheapest): そのストレインで十分 (合法) な最低のレベル。
- `j` (jump): `c` の 1 つ上 (シングルジャンプ)。

ストレインには他のレベルと同じものが書ける: リテラル (`cS`、`cN`、複数ストレインの `cHS`、`cred`)、変数 (`cM`、`coM`、`cm`、`cY`、`jX`)、代替 (`cD/H`、`/` の後の裸のストレインは直前のレベル `c` を引き継ぐ)。相手のコール (`(cX)`) にも書ける。

**意味。** 基準は経路の最後のビッド (どちらの側でもよい。P/D/R は数えない。`step` と同じ基準)。ビッドが無ければ (オープニングの位置) `c` は 1 レベル、`j` は 2 レベル。

| 直前のビッド | `cS` | `jS` | `cH` | `cN` |
| --- | --- | --- | --- | --- |
| なし | 1S | 2S | 1H | 1N |
| 1H | 1S | 2S | 2H | 1N |
| 2H | 2S | 3S | 3H | 2N |
| 2S | 3S | 4S | 3H | 2N |
| 7H | 7S | (候補なし) | (候補なし) | 7N |

- 変数は通常どおり束縛する。未束縛の変数の候補は §4.2 の規則 (未使用のストレイン、`X<Y<Z`) で選び、各候補のレベルをそのストレインの `c`/`j` にする。束縛済みの変数はそのストレインで `c`/`j`。例えば `(1X)-P-(2X)-` の下の `cM` は、`(1H)-P-(2H)` では `2S`、`(1S)-P-(2S)` では `3H` になる。
- 説明文の変数置換は他のレベルと同じ (`cM = 5+M` は `5+!s` に)。
- 候補は各ストレインにつき 1 つなので、`n` と違い `WideWildcard` は出ない。候補が `Level::At` と違って不十分になることは無いので、`IllegalCall` も出ない。
- Exact 行ではない (パターン行として、兄弟の Exact 行の後に処理する。§4.2 の 2)。同じコールを先の兄弟が作っていれば、パターン行の規則どおり黙って (Exact なら `ShadowedByExact` で) 捨てる。

**端の場合と Lint。**

- **ワイルドカードの下** (`(any)`、`(bid)`、`(suit)` の後): 最後の具体的なビッドより後に、ビッドでありうるワイルドカードがあれば、最後のビッドは不明である。そのとき相対レベルを含む行 (代替の一部だけが相対でも行全体) は `LevelWithoutAnchor` (Error) を出して、その位置では行と部分木を捨てる。ワイルドカードの後に我々の具体的なビッドがあれば基準は既知に戻る (`1C-(any)-1H-(P)-` の下の `cS` は `1S`)。変数の履歴の下で行が束縛ごとに展開されても、Lint は元の行 1 つにつき 1 件 (同じスパンの重複は出さない)。
- **7 を超える**: `j` (または最後のビッドが 7 レベルの `c`) が 7 を超えるストレインは候補にならない。束縛済み変数かリテラルで候補が 1 つも無ければ `NoSufficientLevel` (Info)。代替 (`jS/jN`) も、全ての代替が相対レベル (リテラルか束縛済み変数) なら同じ。`NonStandardToken` の理由は代替ごとに判定する (`2S/cH` も相対レベルとして知らせる)。未束縛の変数で候補が無ければ従来どおり `VariableNoCandidate` (Info)。
- **段落の先頭**: 相対レベルのトークンだけで始まる段落は表として扱わない (§3.2)。履歴行 (`-`/`;` を含む先頭行) の中の相対レベルは使える。
- **可搬性**: `bml.py` の `bid_type()` は `c`/`j` をレベルとして読まない (`nX` と同じく既存ツールでは読めない)。したがって D16 の「既存ツールが無視するか説明文として読む」原則から外れる。`NonStandardToken` (Info) を必ず出し、著者に知らせる (D16 の補遺、`13-decisions.md`)。

**実装。** `pattern.rs::Level::{Cheapest, Jump}`、`Level::is_relative`; `parser/call.rs::level` (`'c'`、`'j'`)、`nonstandard_reasons` ("relative level (c = cheapest, j = jump)"); `parser/mod.rs::is_bidtable_start` / `has_relative_level`; `compile/expand.rs::bids_at_level` (`minimum_sufficient_bid` とその 1 つ上)、`generate_candidates` (`Strains` の相対レベル)、`Frame::last_bid_known`、`pattern_has_relative_level`、`expand_row` (`LevelWithoutAnchor`)、`report_empty_candidates` (`NoSufficientLevel`)。試験: `parser/call.rs` の `relative_levels`、`tests/relative_level.rs`。`COMPILE_REVISION` 6。

---

### 4.7 変数の順序を外す表 (`#ANYORDER`、フェーズ 4 拡張)

**目的。** BML の変数 `X`、`Y`、`Z` は、新しく束縛するとき *未使用* のストレインを取るだけでなく、束縛済みのものと `X < Y < Z` (C < D < H < S) の順序を保つ (§4.2 の (c)、`bss.py` の規則)。そのため「相手のスートが我々のスートより上か下か」を問わない合意 (相手のスートを束縛変数として使う競り合いの表) は、上下で表を書き分けるか、スートごとのリテラル表を並べるしかなかった (SAYC の P12 バッチ 6 の `1C-(1D)-2H-(3D)-` など 10 表)。`M`/`m` には順序が無いが、定義域が 2 つのスートに限られる。

**構文。** 表の段落の中の 1 行 `#ANYORDER` (前後の空白は無視)。表の履歴行の前でも、行の間でも、行の下に字下げして書いてもよく、どこに書いても *その表全体* に効く (字下げは範囲を狭めない)。クリップボード (`#CUT`) の中に書けば、`#PASTE` した先の表に効く。`#SEAT`/`#VUL` のように独立した段落 (空行の後に表) に書くと表を名指さないので効果が無く、`UnknownDirective` (Warning) を出す (`#STOP` だけの段落も同じ。試験 `a_directive_in_its_own_paragraph_is_reported_not_silently_dropped`)。

**意味。** その表の展開では、新しい `X`/`Y`/`Z` の候補を「定義域 (C, D, H, S) のうち、どちらの側もまだビッドしていないストレイン」とし、束縛済みの `X`/`Y`/`Z` との大小を問わない。それ以外は通常どおり:

- 束縛済みの変数は束縛のまま。束縛したストレインはビッドされているので `used` にあり、異なる変数が同じストレインを取ることはない (区別は保たれる)。
- 新しい変数の候補は十分な (合法な) ビッドだけ (§4.2 の 3。`(1Y)` の下位スートは 1 レベルでは不十分なので候補にならない。`(2Y)` なら候補になる)。
- 説明文の置換、`M`/`m`/`oM`/`om`、相対レベル (§4.6)、ワイルドカードの下の規則は変わらない。
- 順序つきの展開の集合は、同じ表の `#ANYORDER` 版の展開の部分集合である (試験 `ordered_expansions_are_a_subset_of_any_order_ones`)。
- 表の間では独立: 同じファイルの他の表は順序を保つ。

**Lint。** `#ANYORDER` の表の履歴と行 (子孫を含む) が `X`/`Y`/`Z` のうち 2 つ以上を使っていなければ、外す順序が無いので `AnyOrderWithoutVariables` (Info) を出す (表は普通にコンパイルする)。

**可搬性 (D16 の補遺)。** `bml.py` はこの指示子を知らない。`#STOP` と同じく表の中の未知の行であり、`bml.py` では順序つきで展開される (こちらの展開の部分集合になる) か、未知の指示子として扱われる。本実装の旧版 (`COMPILE_REVISION` 6 以前) では `UnknownDirective` (Warning) を出して無視していた。

**実装。** `ast.rs::BidTable::any_order`; `parser/mod.rs::parse_table_paragraph` (`#ANYORDER`)、`xyz_variables` (Lint の判定); `pattern.rs::Binding::candidates_in_order` (`ordered == false` で順序の下限・上限を外す。`candidates` は `ordered = true` の版); `compile/expand.rs::Frame::any_order` (`expand_table` が表の値で根を作る)、`generate_candidates_in_order` (`expand_row` が `!frame.any_order` を渡す); `lint.rs::LintCode::AnyOrderWithoutVariables`。試験: `parser/mod.rs` の `any_order_is_a_table_directive`、`any_order_without_two_variables_is_reported`、`tests/any_order.rs` (9 件)。`COMPILE_REVISION` 7。

## 5. IR (`ir.rs`)

### 5.1 型

```rust
// ir.rs
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)] pub struct NodeId(pub u32);
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)] pub struct RowId(pub u32);

/// A compiled bidding system. Immutable; share through `Arc`.
pub struct SystemIR {
    pub meta: SystemMeta,
    pub rows: Vec<Row>,          // 1:1 with BML rows after include/paste (authoring provenance)
    pub nodes: Vec<Node>,        // concrete expansions; NodeId indexes this
    pub index: AuctionTrie,
    pub lints: Vec<Lint>,
}
impl SystemIR {
    pub fn node(&self, id: NodeId) -> &Node;
    pub fn row(&self, id: RowId) -> &Row;
    /// Resolves the auction from the point of view of `owner`; None for an empty or passed-out auction.
    pub fn resolve(&self, auction: &Auction, owner: Seat) -> Option<Lookup>;
    /// The candidate continuations for `owner`'s next call, or None when the prefix is off-system.
    pub fn continuations(&self, auction: &Auction, owner: Seat) -> Option<Vec<(Call, NodeId)>>;
}

pub struct Row {
    pub id: RowId,
    pub span: Span,
    pub path: Arc<[SidedPattern]>,        // pattern path incl. own call
    pub description_raw: String,          // pre-substitution, as written
    pub recognition: Recognition,         // §7.7 (max over expansions; usually identical)
    pub expansions: Vec<NodeId>,
}

pub struct Node {
    pub id: NodeId,
    pub row: RowId,
    pub side: Side,                       // whose hand this describes (Them for parenthesized rows)
    pub path: Arc<[SidedPattern]>,        // shared with Row (spec: Vec<CallPattern>)
    pub calls: Vec<Call>,                 // concrete path incl. this call, implicit passes included
    pub call: Call,
    pub binding: Binding,
    pub seat: SeatCond,
    pub vul: VulCond,
    pub constraint: HandConstraint,       // never contains HandConstraint::Custom
    pub branch_weights: Option<Vec<f32>>, // weights of the top-level Or branches ({w:X}); None = equal
    pub priority: i16,                    // {prio:N}; default 0
    pub volume_log2: i16,                 // estimated log2 of constraint volume (TieBreak::Narrowest)
    pub alertable: Alertability,
    pub flags: NodeFlags,
    pub description: String,              // after variable substitution ("4+M" -> "4+!h"), without the {prio:N}/{w:X}/{stop} annotations
    pub children: Vec<NodeId>,            // row-nodes one actual call deeper (skipping implicit passes)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Alertability { #[default] Unspecified, NotAlertable, Alertable, Announceable }

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Forcing { #[default] Unknown, NonForcing, OneRound, ToGame }

#[derive(Clone, PartialEq, Debug, Default)]
pub struct NodeFlags {
    pub artificial: bool,                 // ART, (R), TRF, PUP, named conventions, or the "!" marker
    pub forcing: Forcing,
    pub soft: bool,                       // any hedged fragment ("usually", "may")
    pub transfer_to: Option<Strain>,
    pub agreed_suit: Option<Suit>,
    pub sign_off: bool,                   // S/O, T/P
    pub stop: bool,                       // a system stop's position or its pass (§4.5; informational)
    pub synthesised: bool,                // a node the compiler synthesised for a system stop (§4.5)
}

/// Recognition statistics of one description (§7.7).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Recognition {
    pub covered: u16,                     // words covered by a recognised fragment
    pub total: u16,                       // words in the denominator (stopwords excluded unless covered)
    pub ratio: f32,                       // covered / total (1.0 for an empty description)
    pub unrecognized: Vec<(u16, u16)>,    // byte spans of unrecognised text
    pub constraint_bearing: bool,         // whether any fragment produced a constraint literal
    pub assumed: u8,                      // fragments resolved with an assumed (not stated) context value
    pub soft: u8,                         // hedged fragments
}
```

`Node.children` は暗黙パスを飛ばして「実際のコール 1 つ深い」行ノードを指す。`Node.calls` は暗黙パス込みの具体経路なので、`calls.len()` はトライの深さと一致する。`Forcing` に `ToLevel(Bid)` (`F2NT` 等の「特定レベルまで強制」) は無く、`F2NT` は `OneRound` に丸める。IR の型は全て `#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]` を持つ (`Row.span: Span` と `Lint.span` のために `Span` にも派生がある)。

### 5.2 `SystemMeta` とメタ拡張 (D16)

```rust
pub struct SystemMeta {
    pub name: String,                     // #+TITLE
    pub description: String,              // #+DESCRIPTION
    pub authors: Vec<String>,             // #+AUTHOR, split on "," / " and "
    pub version: String,                  // #+VERSION (ext); default "0"
    pub date: Option<String>,             // #+DATE (ext)
    pub source_hash: [u8; 32],            // blake3 of resolved source (§10)
    pub compiler_version: String,         // COMPILER_VERSION = env!("CARGO_PKG_VERSION")
    pub ir_format: u32,                   // IR_FORMAT; bump on breaking IR change
    pub dist_method: DistMethod,          // #+DISTPOINTS: 321 | bergen | none
    pub tie_break: TieBreak,              // #+TIEBREAK: row-order | narrowest | lowest-call | highest-call
    pub strength: StrengthVocab,          // #+STRENGTH: gf=25 inv=22-24 slam=31 strong=16 weak=9 opening=12 neg=7
    pub balanced: BalancedDef,            // #+BALANCED: 4333 4432 5332 [5422]  (semi adds 5422 6322)
    pub natural: NaturalParams,           // #+NATURAL: 1M=5 1m=3 1N=15-17 2N=20-21 weak2=6 overcall=5 ...
    pub conventions: ConventionDefaults,  // #+CONVENTION: transfer=5 stayman=4M splinter=4
    pub recognition_threshold: f32,       // #+RECOGNITION: 0.5
    pub extra: BTreeMap<String, String>,  // unknown #+KEYs preserved
}

pub struct StrengthVocab {
    pub gf_total: u8,                     // 25
    pub inv_total: RangeInclusive<u8>,    // 22..=24
    pub slam_total: u8,                   // 31
    pub strong_min: u8,                   // 16
    pub weak_max: u8,                     // 9
    pub neg_max: u8,                      // 7
    pub opening_min: u8,                  // 12
    pub hcp_max: u8,                      // 37
}
#[derive(Default)] pub enum TieBreak { #[default] RowOrder, Narrowest, LowestCall, HighestCall }

pub struct BalancedDef { pub balanced: Vec<ShapeClass> /* 4333 4432 5332 */, pub semi_balanced: Vec<ShapeClass> /* 追加分 5422 6322 */ }
pub struct ConventionDefaults { pub transfer_len: u8 /* 5 */, pub stayman_major: bool /* true: 4 枚メジャーを示す */, pub splinter_support: u8 /* 4 */ }
```

`BalancedDef` はクラスのリストで持ち、コンパイラが `ShapeSet::from_classes` で集合にする (`semi-bal` = `balanced ∪ semi_balanced`)。全てのメタ拡張は `#+KEY:` 形式を使う。BML ツールは任意の `\w+` キーを `content.meta[keyword] = value` として受理して無視する。

| キー | 値の文法 | 反映先 | 既定値 |
| --- | --- | --- | --- |
| `#+TITLE:` | 自由文 | `meta.name` | ファイル名 |
| `#+DESCRIPTION:` | 自由文 | `meta.description` | 空 |
| `#+AUTHOR:` | `,` / ` and ` 区切り | `meta.authors` | 空 |
| `#+DATE:` | 自由文 | `meta.date` | `None` |
| `#+VERSION:` (拡張) | 自由文 | `meta.version` | `"0"` |
| `#+STRENGTH:` (拡張) | `gf=25 inv=22-24 slam=31 strong=16 weak=9 opening=12 neg=7` | `meta.strength` | 表の値 |
| `#+NATURAL:` (拡張) | `1M=5 1m=3 1N=15-17 2N=20-21 weak2=6 overcall=5 …` (§8.1 のキー) | `meta.natural` | SAYC 既定 (§8.1) |
| `#+DISTPOINTS:` (拡張) | `321` \| `bergen` \| `none` | `meta.dist_method` | `321` (`GOREN_321`) |
| `#+TIEBREAK:` (拡張) | `row-order` \| `narrowest` \| `lowest-call` \| `highest-call` | `meta.tie_break` | `row-order` |
| `#+BALANCED:` (拡張) | シェイプクラスの列 `4333 4432 5332 [5422]` | `meta.balanced.balanced` | `[C4333, C4432, C5332]` / semi は `[C5422, C6322]` |
| `#+CONVENTION:` (拡張) | `transfer=5 stayman=4M splinter=4` (`stayman=any` で `stayman_major = false`) | `meta.conventions` | 表の値 |
| `#+RECOGNITION:` (拡張) | `0.0..=1.0` | `meta.recognition_threshold` | `0.5` |
| その他 | 任意 | `meta.extra` | |

値のパース失敗は `Lint::UnknownDirective` に準じた Warning を出し既定値を使う。同じキーの 2 回目以降は先勝ち (BML の規則)。

### 5.3 `{prio:N}` / `{w:X}` 注釈と優先度・体積

- `priority`: 既定 0。説明文末尾のインライン注釈 `{prio:N}` (`N` は `i16`) で上書きする。HTML/LaTeX では無害に描画される。`choose_bid` (L3) は合格候補を `priority` 降順に整列し、同点を `meta.tie_break` で解く。
- `branch_weights`: 制約の最上位が `Or` のとき、列挙項目や `or` 群の各項に付けた `{w:X}` (`X` は正の `f32`) を枝順に集めて合計 1 に正規化する。注釈の無い枝には残余を等分する。どの枝にも無ければ `None` (L3 が等分)。`Or` でないノードに付いた `{w:X}` は `NonStandardToken` (Warning) で無視する。
- `alertable`: 先頭 `!` (§1.3) で `Alertable`、無ければ `Unspecified`。`NotAlertable` / `Announceable` は予約で、v1 の説明文文法には対応する注釈が無い。
- `volume_log2`: `TieBreak::Narrowest` 用の制約体積の推定値。コンパイル時に決定的に計算する。`constraint.to_dnf()` の各項 `t` について `w_t = (hcp_hi − hcp_lo + 1) × |t.atom.shapes|` (`hcp` は `shapes.min_hcp()..=shapes.max_hcp()` でクランプ) とし、`volume_log2 = round(log2(Σ_t w_t))` を `i16` に収める。空なら `i16::MIN`。シェイプごとの手数による重み付けは 未決 (厳密な `Sampler::count()` は 20〜60 μs/項なので 1 秒のコンパイル予算に入らない可能性がある)。

---

### 5.4 派生索引 `exclusive` (`exclusive.rs`、フェーズ 4)

`SystemIR` は、システムコールの排他領域の索引 `ExclusiveIndex` を派生データとして持つ。

- **置き場所**: `SystemIR.exclusive_cell: OnceLock<ExclusiveIndex>` (`#[serde(skip)]`)。参照は `SystemIR::exclusive()` で行う。`compile()` の最後 (コンパイル後検査 §9.3 の直前) に先行して構築するので、コンパイル直後の IR では初回参照のコストは無い。直列化から復元した IR と手組みの IR は、初回参照時に構築する。
- **直列化しない**: 索引は直列化しないので、直列化形式と `IR_FORMAT` に影響しない (`IR_FORMAT` が 2・3 に上がったのは §4.5 のシステム停止による)。索引を入れた時点 (システム停止と、フェーズ 4 の SAYC の行の追加より前) の SAYC の postcard は 826,962 バイトでフェーズ 3 と同一、§9.2 の新しい Lint 2 種が `lints` に入る分だけ 845,860 バイト (+18,898、+2.3%) だった。現在の値は §4.5 (2,476,329 バイト)。プロトタイプ A のように索引まで直列化すると 1.24 MB (1.51 倍) になり、wasm の転送量に効くので避けた。
- **排他領域の定義**: 位置 (親 `TrieId`、条件クラス) で、`choose_bid` がコール c を選ぶ手の集合を X_c とする。X_c は、rank 順 (`exclusive::rank_cmp`: priority 降順 → `SystemMeta::tie_break` → コール index 昇順) で最初に満たされるメンバーのコールが c である手の集合である。各枝の片 (`ExclusivePiece`) は、同じノードの先の枝と上位のメンバー全部を `subtract` で引いたものである。`subtract` は原子レベルの厳密な差集合で、結果は素な原子の平坦な `Or` (上限 48 原子) になる。上限を超えたときだけ木 `And([base, Not(Or(minus))])` に退避する。したがって片は互いに素である。DNF が空の片と、グリッドで空と証明できる片 (`exclusive::grid_proves_empty`) は落とす。
- **条件クラス**: `(opener_pos − 1) | we << 2 | they << 3` の 16 通りで、`AuctionTrie::children` が席・バル条件で絞る単位と同じである。子が席・バル条件を持たない位置 (`AuctionTrie::children_are_conditioned` が偽) では子の計算を 1 回で済ませ、16 キーを同じグループに向ける。同じメンバー列を持つグループは重複除去する。
- **API**: `ExclusiveIndex::build(&ir)`、`group(parent, class)`、`entries()` (キーとグループの列挙)、`stats(&ir) -> ExclusiveStats`、`exclusive::grid_proves_empty(c)`、`AuctionTrie::has_children(at)` / `children_are_conditioned(at)`。
- **実測 (SAYC、システム停止の後)**: 2,754 グループ、44,064 キー、6,477 メンバー、7,586 枝、7,186 片、木へ退避した片 3、平坦な原子 10,838、木の補集合 3、決して選ばれないコール 389。構築は 12.6〜12.8 ms (release、best of 3、loadavg 3.2〜3.5)、compile 全体 437〜441 ms の約 2.9%。停止の前 (パスの鎖、24,067 グループ) は 44.7〜47.5 ms だった。停止のパスを持つグループが多い (その片は `ANY` から上位を引いたもの) ので、構築では次の 3 点で手間を省く (結果の索引は Debug 出力でバイト単位まで同一): `subtract` は引くものに `ANY` の原子があれば DNF の計算をせずに空の `Or` を返す (停止のパスを持つグループの補集合)、`grid_proves_empty` は平坦な片を原子ごとに実行可能セルと照らし、グリッドを組み立てない (`Or` の上界は原子の箱の和なので同じ答え)、1 回の構築の中では引く側の DNF を 1 度だけ求める (`DnfCache`、借りた IR の制約のアドレスをキーにする)。
- **実測 (SAYC、フェーズ 4 の SAYC 追加前)**: 714 グループ、11,424 キー、2,270 メンバー、2,794 枝、2,545 片、木へ退避した片 3 (0.12%)、平坦な原子 3,931、木の補集合 3、決して選ばれないコール 238。構築時間と compile 全体に占める割合は 次のとおり (release、best of 3、`tests/compile_time.rs` の `sayc_exclusive_index_share`)。索引の構築は 9.2〜9.5 ms で、compile 全体 161〜167 ms の約 5.7% (loadavg 7〜8)。フェーズ 3 の基点 (0f63599) の compile は同じ負荷で 152 ms なので、増分はほぼ索引の構築分であり、§9.3 の検査 8 の費用は測定誤差に収まる。
- **検証** (`tests/exclusive.rs`): ランダムな (キー、手) の組で、手が X_c に入ることと「rank 順で最初に満たすメンバーのコールが c」であることが一致するか、補集合と片が素か、1 グループで手の入る片が高々 1 つかを確かめる。既定スイートは 1e4 組、`#[ignore]` 版は 1e5 組で、どちらも不一致 0 (1e5 組のうち X_c に入る組は 71,028、release で 283 ms)。

## 6. `AuctionTrie` (`trie.rs`)

### 6.1 構造

```rust
// trie.rs
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub struct TrieId(pub u32);

pub struct AuctionTrie {
    nodes: Vec<TrieNode>,          // arena; index 0 = we-open root, 1 = they-open root
}
struct TrieNode {
    depth: u8,
    exact: Vec<(u8 /*Call::index() 0..38: P=0 D=1 R=2 bids 3..37*/, TrieId)>,   // sorted; binary search
    classes: Vec<(OppClass, TrieId)>,                                            // wildcard edges (ext), usually empty
    entries: Vec<Entry>,                                                         // rows attached to this call
}
struct Entry { seat: SeatCond, vul: VulCond, specificity: u8, node: NodeId }
```

コール索引は `Call::index()`: `Pass = 0`, `Double = 1`, `Redouble = 2`, `Bid(b) = 3 + b.index()` (1C = 3 … 7NT = 37)。各ノードの子は、ソート済み小ベクタの Exact 辺 (実測で ≤ 12 程度。プロファイルが求めれば `Box<[Option<TrieId>; 38]>` に替える) と、ワイルドカードのリストに分ける。`entries` はこの辺で到達できる行ノードを `#SEAT/#VUL` 条件ごとに持つ。最も特定的な条件が勝ち (`12` は `0` に勝ち、`seat` と `vul` の両方が特定的なら片方だけより勝つ)、同点は先定義 (コンパイル時に Lint) なので、**深さごとに高々 1 つの `Node`** が返る。多義のコール (「ナチュラルまたはスプリンター」) は 1 ノードの制約内の `Or` であり、複数ノードにはしない。

### 6.2 API

```rust
pub struct Lookup {
    pub matched_depth: usize,                       // number of calls matched (== key.calls.len() when exact)
    pub by_depth: SmallVec<[Option<NodeId>; 16]>,   // node for call i (None for implicit passes / no row)
    pub end: TrieId,                                // trie node at matched_depth (for children())
    pub via_class: u8,                              // how many wildcard edges were taken (0 = pure exact)
}
impl Lookup { pub fn is_exact(&self, key: &LookupKey<'_>) -> bool; }

impl AuctionTrie {
    pub fn new() -> AuctionTrie;                                                  // 2 つのルートだけ (Default も同じ)
    pub fn resolve(&self, key: &LookupKey<'_>) -> Lookup;                         // ~depth × 30 ns
    pub fn children(&self, at: TrieId, opener_pos: u8, vul: RelVul) -> Vec<(Call, NodeId)>;   // choose_bid candidates
    /// Fallback helper: retry with up to `max_subst` unmatched *opponent* calls replaced by Pass ("system on").
    pub fn resolve_lenient(&self, key: &LookupKey<'_>, max_subst: u8) -> SmallVec<[(Lookup, u8); 4]>;
    /// Compiler side. Err(existing) when an entry with identical conditions is already present (first definition wins).
    pub fn insert(&mut self, we_opened: bool, calls: &[Call], seat: SeatCond, vul: VulCond, node: NodeId) -> Result<(), NodeId>;
    pub fn len(&self) -> usize;          // トライノード数
    pub fn is_empty(&self) -> bool;      // ルート 2 つだけ
}

pub enum Resolution {
    Exact(NodeId),
    Partial { node: NodeId, matched_depth: usize },
    Natural(HandConstraint),
}
```

`resolve` の手順:

1. `root = if key.we_opened { 0 } else { 1 }`、`cur = root`、`d = 0`。
2. `key.calls[d]` について `cur.exact` を二分探索。見つかれば次へ; 無ければ `cur.classes` の中で述語が成り立つ最初の辺を取る (`via_class += 1`); どちらも無ければ停止。 後戻りはしない: Exact 辺が存在すればその部分木に入り、後続のコールがそこで見つからなくても、ワイルドカード辺の部分木を試し直すことはない (Exact が常に勝つ)。
3. 進んだ先の `entries` から `(key.opener_pos, key.vul)` に一致する最大特定度のエントリを選び `by_depth.push(Some(node))`。エントリが無い (暗黙パスのノード、または行の無い辺) なら `None`。
4. 深さ `d` のコールが我々側で、かつエントリが `None` かつ暗黙パスでもなければ、`matched_depth` はそこで止まる (次節)。
5. `end` は最後に到達したトライノード。

`children(at, opener_pos, vul)` は `at.exact` の各辺について、その先の `entries` から条件に一致する最良のエントリを 1 つ返す (無ければ辺を飛ばす)。`choose_bid` の候補列挙に使い、返却順は `exact` のソート順 (コール昇順)。

`resolve_lenient(key, max_subst)`: `resolve` が `matched_depth < calls.len()` で止まったとき、停止位置以降で最初の **相手側** コールを `Pass` に置き換えて再試行する。置換ごとに 1 を数え、`max_subst` まで繰り返す。戻り値は `(Lookup, 置換回数)` を置換回数の昇順に並べたもの (最大 4 件)。L3 は置換回数ごとに重みを減衰させる。

### 6.3 Partial の操作的定義

`calls = c_1..c_n` に対しトライを歩く。`matched_depth = d` は「**我々側** の全コールについて条件を満たすエントリが存在する最長の接頭辞 `c_1..c_d`」の長さ (暗黙パスのトライノードは一致とみなす)。`d == n` なら `Exact(by_depth[n])`。それ以外は `Partial { node: by_depth[d'], matched_depth: d }` で、`d'` は `d` 以下でノードを持つ最深の深さ。意味は「コール 1..d はシステムで解釈済み、d+1..n はシステム外」。L3 はその後 (1) `resolve_lenient` (予期しない相手のコールをパス扱い、置換ごとに重みを罰する)、(2) 残りの我々側の各コールに `NaturalInference`、の順で処理し、**一致済みの部分は弱めない**。仕様の「直近のビッドのみ解釈」は、接頭辞の制約を保ったまま最後のコールに (2) を適用することに相当する。`Resolution::Natural` はトライではなく L3 が作る。enum をここに置くのは両クレートで共有するため。

### 6.4 コストモデル

深さ ≤ 20 コール程度。コールごとに ≤ 12 辺の二分探索 1 回 + ≤ 3 エントリの条件フィルタなので、`resolve` 1 回は 1 μs を大きく下回る (目安 深さ × 30 ns)。`interpret` は `resolve` を 2 回 (パートナーシップごとに 1 回) と制約のクローンを要する。ホットパスで確保を行わない (`SmallVec`)。IR は不変で `Send + Sync`。メモ化は呼び出し側が持つ (仕様 §9)。

---

## 7. 説明文 → `HandConstraint` コンパイラ (`compile/desc/`)

### 7.1 パイプライン (展開ごと。`compile_description(text, ctx, meta) -> Compiled`)

1. **正規化** (`normalize.rs`): アラート `!` を取り除いて記録; `{prio:N}` / `{w:X}` 注釈を取り出す; `!c!d!h!s` を `♣♦♥♠` の番兵文字に写す; `--` → `-`; 空白を畳む; 改行は残す (列挙は行単位)。正規化後のバイト位置から元の位置への写像 (`offsets`) を保持する。照合は大文字小文字を区別しないが、`M/m`、`MM/mm`、`oM/om` だけは区別する。
2. **節パース** (`clause.rs`, winnow, §7.3) → バイトスパン付きの `Vec<Fragment>` と、断片添字上の論理構造 `Clause`。未認識のスパンも `FragmentKind::Unrecognized` として保持する。
3. **Pass 1** (`tokens.rs`): 文脈自由な断片 → `Token`。
4. **Pass 2** (`context.rs`): 文脈依存の断片 (`GF INV MIN MAX weak PRE STR S/T QUANT NAT SPL fit TRF #`) を、親連鎖 (両プレイヤーのコンパイル済み祖先ノード)、束縛変数、`SystemMeta` で解決し、`Atom` のリテラルと `Provenance` にする。
5. **組立** (`mod.rs`): `Clause` に沿って断片を `And` で結合; `or` 群と列挙は `Or`; 否定は `Not`; 簡約 (Atom が 8 個以下なら `to_dnf`) → `HandConstraint`。空なら `HandConstraint::Atom(Atom::ANY)`。
6. **報告** (`recognition.rs`): `Recognition`、`tracing` イベント、Lint。

### 7.2 断片と文脈の型

```rust
// compile/desc/tokens.rs
pub enum SuitRef { Fixed(Suit), Hash, Own, AnyMajor, AnyMinor, Agreed, Theirs }
//  Fixed: `!s`、または展開時に置換された束縛変数; Hash: `#`; Own: 自分のコールのスート;
//  AnyMajor/AnyMinor: 未束縛の M/oM/m/om; Agreed: 合意スート; Theirs: 相手のスート

pub enum Token {
    Hcp(RangeInclusive<u8>),                 // `12+ hcp`, `15-17`, `ca 15+`
    Points(RangeInclusive<u8>),              // `13+ points` (total points; method は meta.dist_method)
    SuitLen(SuitRef, RangeInclusive<u8>),    // `5+!s`, `4=!h`, `0-3!s`, `6+ suit`, `4+#`
    Shape(String),                           // `4414`, `(54)`, `54(31)`, `(54)(xx)`, `55MM`, `5-5 minors`, `4-4 majors`
    Balanced, SemiBalanced, Unbalanced,      // `bal`, `semi-bal`, `unbal`
    Strength(StrengthWord),                  // `GF`, `INV`, `INV+`, `MIN`, `MAX`, `weak`, `STR`, `PRE`, `S/T`, `QUANT`, `NEG`, `LIM`
    Forcing(Forcing),                        // `NF`, `F`, `F1`, `FG`
    Convention(String),                      // `ART`, `(R)`, `TRF`, `PUP`, `P/C`, `S/O`, `STAY`, `UNT`, `Multi`, …
    Quality(SuitRef, QualityWord),           // `SOL`, `S-SOL`, `2 of top 3`, `good suit`
    HonourRun(SuitRef, String, u8),          // `AKQ`, `AKQxx`, `KQJ109x`, `QJ10xx`: 列挙オナー (T 表記) と書かれた枚数
    Stopper(SuitRef),                        // `stopper`, `with stopper`
    Shortness(SuitRef, u8),                  // `singleton`, `void`, `short`, `0-1!h`
    Support(u8),                             // `fit`, `3+ SUPP`, `support`, `raise`
    Controls(RangeInclusive<u8>),            // `controls`, `2 controls`
    Losers(RangeInclusive<u8>),              // `7 losers`, `LTC`
    Natural,                                 // `NAT`, `natural`
    Splinter(Option<SuitRef>, bool),         // `SPL`, `SPL !c`, `SPL m`, `mini-splinter` (true = mini); None = 自分のコールのスート
    NoBound,                                 // `unlimited`, `any hand`: 認識済み、制約なし
}
pub enum StrengthWord { GameForcing, Invitational, InvitationalPlus, Min, Max, Weak, Strong, Preemptive, SlamTry, Quantitative, Negative, Limit }
pub enum QualityWord { Solid, SemiSolid, TwoOfTopThree, ThreeOfTopFive, Good }
pub fn recognize(text: &str) -> Option<(Token, usize)>;   // 1 断片を認識し、消費バイト数を返す

// compile/desc/clause.rs
pub struct Fragment { pub span: (u16, u16), pub negated: bool, pub hedged: bool, pub possibility: bool, pub kind: FragmentKind }
//  possibility: 可能性の hedge (`may`, `might`, `possibly`, `rarely`, `occasionally`, …) が付いた断片。§7.6
pub enum FragmentKind { Token(Token), Unrecognized(String) }
pub enum Clause { Leaf(usize) /* fragment index */, And(Vec<Clause>), Or(Vec<Clause>) }
pub fn parse(text: &str) -> (Vec<Fragment>, Clause);
pub fn fragment(input: &mut &str) -> ModalResult<Fragment>;

// compile/desc/normalize.rs
pub struct Normalized { pub text: String, pub alert: bool, pub priority: Option<i16>, pub weights: Vec<f32>, pub offsets: Vec<u16> }
pub fn normalize(text: &str) -> Normalized;

// compile/desc/context.rs
pub enum Source { Explicit, Context, NaturalDefault }
pub struct Provenance { pub span: (u16, u16), pub assumed: bool, pub source: Source }

/// Everything known about a row when its description is compiled (ancestors are done first).
pub struct RowContext<'a> {
    pub call: Call,
    pub side: bridge_core::Side,          // NS / EW (テーブル上の側)
    pub level: u8,                        // 0 for P/D/R
    pub is_jump: bool,
    pub binding: &'a Binding,
    pub hash_suit: Option<Suit>,          // `#` が指すスート
    pub own_prev: Option<&'a Node>,       // this player's previous row-node on this path
    pub partner_last: Option<&'a Node>,   // partner's last row-node
    pub their_last_bid: Option<Bid>,
    pub agreed_suit: Option<Suit>,
    pub role: Role,                       // Opener | Responder | Overcaller | Advancer | Balancer (from path shape)
    pub partner_hcp: Option<RangeInclusive<u8>>, // パートナーがこの経路で行った全ノードの HCP 範囲の交差 (None = 未追跡)
    pub own_hcp: Option<RangeInclusive<u8>>,     // 自分の同様の交差
}
// 1 トークンにつき 1 リテラル (普通は Atom。stopper は §7.4 の Or)
pub fn resolve(tokens: &[Token], ctx: &RowContext<'_>, meta: &SystemMeta) -> (Vec<HandConstraint>, Vec<Provenance>);

// compile/desc/mod.rs
pub struct Compiled { pub constraint: HandConstraint, pub branch_weights: Option<Vec<f32>>, pub priority: i16, pub flags: NodeFlags, pub recognition: Recognition, pub lints: Vec<Lint> }
pub fn compile_description(text: &str, ctx: &RowContext<'_>, meta: &SystemMeta) -> Compiled;

// compile/desc/recognition.rs
pub fn compute(text: &str, fragments: &[Fragment]) -> Recognition;
pub const STOPWORDS: &[&str];   // a an the and or with w/ in of at hand suit suits cards points hcp
```

パートナーが示したスートは `partner_last.constraint.suit_len(..)` から Pass 2 が都度読む。HCP 範囲は直前ノードだけでは足りない (直前が人工コールや HCP を書かない応答であることが多い) ので、展開 (`expand.rs`) が席ごとに経路上の全ノードの `hcp_range()` の交差を `Frame` に持ち、`RowContext.partner_hcp` / `own_hcp` として渡す (交差が空になったら後のノードの範囲を採る)。祖先が無い・ワイルドカード下・親が `Partial` にしか対応しない場合の `assumed` は `Provenance` に記録する。

### 7.3 節文法 (`clause.rs`)

```ebnf
description = { line } ;
line        = enumitem | clauses ;
enumitem    = ( LETTER ")" | DIGIT ")" | DIGIT "." | "(" lower ")" | "(" DIGIT+ ")" ) clauses ;
                                                                     (* items form an OR group *)
clauses     = orgroup { ( "," | ";" | "." ) [ "or" | "/" ] orgroup } ;
                                                                     (* "," = AND, loosest; "A, B, or C" = OR *)
orgroup     = andgroup { ( "or" | "/" ) andgroup } ;
andgroup    = fragment { ( "and" | "with" | "w/" | "+" | WS ) fragment } ;
fragment    = [ negation ] [ hedge ] atom ;
negation    = "not" | "no" | "without" | "w/o" | "denies" | "non" ;
hedge       = probable | possibility [ "be" | "have" | "hold" | "contain" | "include" ] ;
probable    = "usually" | "normally" | "typically" | "likely" | "mostly" ;
possibility = "may" | "might" | "possibly" | "perhaps" | "maybe" | "rarely" | "occasionally" | "sometimes" | "(?)" ;
atom        = callref | token (§7.4) | freetext ;
callref     = call { ( "-" | "/" ) call } ;   (* call = 1-7 + スート記号 | NT | N。下記の条件のときだけ *)
```

優先順位 (強い順): `and` > `or`/`/` > `,`/`;`。これにより `6+!c or 5!c and 4!h/!s, 11--15 hcp` は `(6+♣ ∨ (5♣ ∧ (4♥ ∨ 4♠))) ∧ 11-15` にコンパイルされ、README の著者の意図と一致する。1 つの長さトークン内の 2 つのスート参照の間の `/` (`4!h/!s`) は局所的な OR、節の間の `/` (`20--21 bal / Any game force`) は節の OR。列挙項目 (`a) b)`、`1) 2)`、`(a) (b)`、`(1) (2)`。括弧付きの文字は小文字のみで、`(R)` や `(?)` は原子のまま) は 1 つの OR 群を作り、列挙の前後の節 (`one of:` や共通の HCP) は全項に AND される。

`,`/`;` の直後に `or` が来る列挙 (`A, B, or C`、`A, or B`) は、その `or` で終わるコンマ列全体 (直前の `.`/`;` 以降、または前の `, or` 項以降) を 1 つの `Or` にする。`or` 項の後の普通のコンマで列は閉じる (`A, or B, 12-14 hcp` = `(A ∨ B) ∧ 12-14`)。長さ 0 の断片 (節頭の `or` の前など) は記録しない (空の `Unrecognized` 葉が `Or` を `ANY` に潰していたため)。

`callref` はコール名/オークションへの参照で、スート長ではない。コール形のトークン (`4!s`, `1NT`, `2!d-2!h-3!h`, `3!d/3!h`) が `to`/`over`/`after`/`for`/`than`/`via`/`opposite`/`like`/`see`/`from`/`into`/`then`/`by`/`bid`/`rebid`/`opening`/`open`/`bids`/`else`/`otherwise`/`instead` の直後にあるとき、または `-` でつながった厳密に昇順の列で、3 コール以上・NT を含む・レベルが変わる・1〜2 レベルの同レベル対のいずれかのとき (オークションとして読める。`5!h-4!s` のような降順/同レベルの長さ略記は対象外) に限り、1 つの `Unrecognized` 断片として消費する (`TRF to 4!s`、`see 1!h-1!s-2!c`、`(else 2!s)`)。

### 7.4 トークン語彙 (v1。`v2` と記した行を除く)

`Own` = ノードのコールのストレイン; `#` = 経路上の直近の変数/複数ストレインコール; 範囲は閉区間; `L` = コールのレベル。`[C]` = 文脈依存 (Pass 2)。全てのトークンは認識率のためにスパンを記録する。束縛済みの変数 (`M`, `oM`, `3M`, `X = oM` 等) は展開時に説明文の中で `!h` 等に置換されているので、ここでは `SuitRef::Fixed` として現れる。

| トークン形 (ファイルからの実例) | `Token` | `Atom` への効果 |
| --- | --- | --- |
| `15-17`, `15--17`, `12+`, `0--7`, `12+ hcp`, `9+HCP`, `ca 15+`, `about 11--12 hcp`, `7--9 HCP`, `18+ hcp any distribution` | `Hcp(a..=b)` | `hcp = a..=b` (裸の `a-b` はスート/スート群/`cards` が続かない限り HCP。続けばシェイプ。ただし `a > b` か `a == b ≥ 4` の裸の対 (`6-5`, `5-4`, `5-5`。後ろに単位/スート/`SUPP` が無いとき) は 2 スート型の分布 (長い順に 2 スートが `≥ a`, `≥ b`) で、HCP ではない。`1-11-2017` のように `-数字` が続く列 (日付など) も範囲ではない) |
| `12--14 NT`, `20--21 bal`, `22-24 NT` | `Hcp` + `Balanced` | `hcp`、`shapes ∩= meta.balanced` |
| `13+ points`, `9--17 points`, `14+ points`, `TP`, `total points` | `Points(a..=b)` | `eval += TotalPoints(meta.dist_method), a..=b` |
| `5+!s`, `4!h`, `4=!h`, `0--3!s`, `2--4!d`, `6+!c`, `3=!h`, `4+ !h`, `at least 4!s`, `at most 3!h` | `SuitLen(Fixed(s), a..=b)` | `suit_len[s] = a..=b` (`4=` はちょうど 4、裸の `4!h` は 4..=4。`at least`/`at most` は後続の数値トークンの片側を開く: `at least 4!s` = 4..=13。HCP/points/controls/losers も同様)。コール参照 (§7.3 の `callref`) の中の `4!s` は長さではない |
| `6+ suit`, `5+ suit`, `6+ cards`, `5 card suit`, `6+ card`, `7+ suit` | `SuitLen(Own, …)` | `suit_len[Own]` (Own はスートであること。NT/P/D なら未認識 + Lint) |
| `5+#`, `7+#`, `0--1#`, `good 4+#` | `SuitLen(Hash, …)` | `#` を §1.3 の規則で解決 (`RowContext.hash_suit`) |
| `5+ M`, `5+M`, `4+m`, `4+ major`, `3+ minor`, `6+m`, `4M`, `5M` (未束縛のとき) | `SuitLen(AnyMajor \| AnyMinor, …)` | Own が群に属せば Own、そうでなければ群上の `Or` |
| `4+!h 4+!s`, `5+!s 4+!h`, `5!h-4!s`, `5+m-4M`, `5+!d-4+!c`, `4+!d 4+!c` | `SuitLen` × 2 | 連言 |
| `5-5 minors`, `4-4 majors`, `5-4 majors`, `at least 5-4 majors`, `55MM`, `54MM`, `44MM`, `5+4+MM`, `5(4)+4+MM`, `4+4+ MM`, `5+5+ red suits`, `5+5+ minors`, `5-5 !d+!c`, `both MM`, `5+ 5+ in lowest two unbid suits`[C] | `Shape(text)` (2 スート型) | 群の 2 スートへの長さ割当 2 通りの `Or` (`ShapeSet` の和); `both MM` = 4-4+; `unbid` は `our_suits ∪ their_suits` の補集合の下位 2 つ |
| `4414`, `4441`, `4405`, `3405`, `3433`, `4333` | `Shape(text)` (完全指定、S H D C 順) | `shapes = {その順序付きシェイプ}`。4 桁が 13 にならない数 (`RKCB 0314`, `1430`, 年号 `2017`)、固定桁の和が 13 を超えるパターン、空集合になるパターンはシェイプではない (未認識) |
| `(5431)`, `(54)`, `(4441)`, `33(43)`, `3(433)`, `22(54)`, `31(54)`, `40(54)`, `34(42)`, `54(31)`, `(54)(xx)`, `5m422`, `5M4oM22`, `4M(441)` | `Shape(text)` (パターン) | `shapes ∩= クラス集合`; 括弧内の数字は順不同、位置指定の数字は S,H,D,C に対応; `x` = 任意; `m/M/oM` は束縛で解決 |
| `bal`, `BAL`, `balanced`, `(semi)Balanced`, `semi-bal`, `SEMI-BAL`, `unbal`, `UNBAL`, `unbalanced`, `not 4333` | `Balanced` / `SemiBalanced` / `Unbalanced` | `shapes ∩= meta.balanced` / semi = bal ∪ `meta.semi_balanced` / unbal = 補集合 |
| `GF`, `FG`, `game force`, `game forcing`, `forcing to game`, `Any game force`, `FG!` | `Strength(GameForcing)`[C] + `Forcing(ToGame)` | `hcp.start = max(0, gf_total − partner_min)` |
| `INV`, `inv`, `invitational`, `INV+`, `at most invitational`, `Strongly invitational`, `Mildly invitational`, `LIM`, `limit`, `G/T`, `game try` | `Strength(Invitational \| InvitationalPlus \| Limit)`[C] | `hcp = inv_total − partner_min` (mild は −1、strong は +1 のシフト); `INV+` は `start` のみ; `at most` は `end` のみ |
| `MIN`, `minimum`, `MAX`, `maximum`, `min`, `max` | `Strength(Min \| Max)`[C] | 自分のこれまでの範囲の半分ずつ (§7.5)。`MIN/MAX` (どちらの端でもよい) は `NoBound` |
| `weak`, `WK`, `Weak`, `light`, `(very) light`, `NEG`, `negative`, `0+ hcp` | `Strength(Weak \| Negative)`[C] | `hcp = 0..=weak_max` (レスポンダー) / `0..=neg_max`; オープナーは `natural` の weak-two / preempt 範囲、オーバーコーラーは jump overcall 範囲 |
| `PRE`, `preemptive`, `Preemptive`, `barrage` | `Strength(Preemptive)`[C] | weak ∧ `suit_len[Own] ≥ 4 + L` (2→6, 3→7, 4→8)、明示の長さがあればそちら |
| `S/O`, `sign off`, `Sign off`, `T/P`, `to play`, `To play` | `Convention("S/O")` | Atom なし; `flags.sign_off = true` |
| `STR`, `strong`, `Strong` | `Strength(Strong)` | `hcp.start = strong_min` |
| `S/T`, `slam try`, `slam interest`, `QUANT`, `quantitative` | `Strength(SlamTry \| Quantitative)`[C] | `S/T`: `hcp.start = slam_total − partner_max`; `QUANT`: §7.5 |
| `F`, `F1`, `F1R`, `NF`, `Non forcing`, `forcing`, `F2NT`, `!F` | `Forcing(_)` | Atom なし; `flags.forcing` (`F`/`forcing`/`F1`/`F1R`/`F2NT` は `OneRound`。`Forcing::Unknown` は「何も書かれていない」の意味にだけ使う。否定された `F`/`forcing` (`Non forcing`, `non-forcing`, `not forcing`) は `NonForcing`。集約は §7.6) |
| `ART`, `artificial`, `Artificial`, `(R)`, `relay`, `Relay`, `ask`, `asks`, `asking`, `Asking for …`, `PUP`, `puppet`, `Puppet to 2!d`, `P/C`, `pass/correct`, `Pass/correct`, `Forced`, `forced`, `CoG`, `choice of games`, `cue`, `CUE`, `cuebid`, `Waiting` | `Convention(name)` | Atom なし (認識済み、`flags.artificial = true`); `Forced`/`P/C` は `constraint = ANY` を継承 |
| `TRF`, `transfer`, `Transfer`, `TRF !c`, `transfer to !h`, `TRF for !h`, `Texas TRF`, `Retransfer` | `Convention("TRF")`[C] | 明示スート → `suit_len[target] ≥ conventions.transfer_len`; スート無しで Own がスート → 次のストレイン (`C→D`, `D→H`, `H→S`); Own が `S` または NT のとき target は不定なので Atom なし + `AssumedContext`; `flags.transfer_to = target`; `flags.artificial` |
| `STAY`, `Stayman`, `stayman`, `Garbage STAY`, `Muppet STAY`, `Puppet Stayman`, `Smolen`, `Landy`, `Michaels`, `UNT`, `unusual`, `Unusual NT`, `Gambling`, `Lebensohl`, `Ogust`, `BW`, `RKCB`, `KCB`, `K/B`, `Multi`, `multi` | `Convention(name)` | 既定では Atom なし; `ConventionDefaults` が与えるものだけ Atom を作る (`stayman_major = true` → `Or(suit_len[H] ≥ 4, suit_len[S] ≥ 4)`; `UNT` → 最下位の 2 つの未ビッドスート (明示があればその群) に `≥ 5` ずつ、5 は定数); 名前付きコンベンションは `flags.artificial = true` |
| `SPL`, `splinter`, `Splinter`, `FG SPL`, `SPL !c`, `SPL m`, `mini-Splinter` | `Splinter(short, mini)`[C] | `suit_len[short] = 0..=1` (short = 語の直後の明示スート、無ければ Own。`SPL in the other major` は未解析で Own) ∧ `suit_len[agreed] ≥ conventions.splinter_support` ∧ `GF` の式 (mini は INV)。各部分は明示が勝つ: 説明文に明示の `Hcp`/`Points` があれば強さ部分なし、明示のショートネス (`Shortness` か `≤ 1` の `SuitLen`) があれば Own のショートネスなし、合意スートの明示の長さがあればサポート部分なし。合意スート = short のとき (そのスートへのスプリンター、トランスファー越しなど) もサポート部分なし。`flags.artificial` |
| `singleton`, `void`, `short !h`, `shortness in X`, `S/S`, `0--1!h`, `no short !h`, `not short`, `short in M` | `Shortness(suit, max)` | 指定スート: `suit_len[s] = 0..=max` (`void` 0、`singleton` 1、`short` 2。否定は `≥ max + 1`); 無指定: `shapes ∩= ShapeSet::filter(shortest ≤ max)` |
| `fit`, `FIT`, `3+ SUPP`, `SUPP`, `4+ trumps`, `support`, `raise`, `with fit`, パートナーのスートへの `3=!s` | `Support(min)`[C] | `suit_len[agreed] ≥ min` (明示 `n`、無ければ §7.5 の `support_min`; ジャンプ/リミットレイズは 4); `flags.agreed_suit` |
| `NAT`, `natural`, `Natural`, `nat` | `Natural`[C] | §7.5 の NAT 規則 (`NaturalInference` の該当規則を借用) |
| `stopper`, `with stopper`, `without stopper`, `stop`, `Axx in their suit`, `stopper in X`, `!s stopper`, `no stopper` | `Stopper(suit)` | `Or(A, K ∧ len≥2, Q ∧ len≥3, J ∧ len≥4)` を `CardRequirement` と `suit_len` の `Or` で表現; スート無指定なら `their_suits` の直近 (`Theirs`) |
| `SOL`, `solid`, `solid suit`, `S-SOL`, `semi-solid`, `2 of top 3`, `2 of 3 top`, `2/3 top`, `3 of top 5`, `AKQ`, `AKQxx`, `KQJ109x`, `QJ10xx`, `good suit`, `good 4+#`, `decent suit`, `reasonable 5 card suit`, `quality suit` | `Quality(suit, word)` / オナー列は `HonourRun(suit, honours, cards)` (`10` は `T`。`QT` 単独は quick tricks なので対象外) | `Solid`: `CardRequirement{mask = AKQ of s, count 3..=3}` ∧ `suit_len[s] ≥ 6`; `SemiSolid`: `{AKQ, 2..=3}` ∧ `≥ 6`; `TwoOfTopThree`: `{AKQ, 2..=3}`; `ThreeOfTopFive`: `{AKQJT, 3..=5}`; オナー列は列挙オナーを `count = 全部` で、`x` の個数は `suit_len ≥` に; `Good`: `EvalRequirement{SuitQuality(s), 2..=5}` (`honors5` = 上位 5 オナーの枚数、2 は定数) |
| `3+ controls`, `2 controls`, `controls`, `7 losers`, `≤ 6 losers`, `6-7 losers`, `LTC 7`, `LTC` | `Controls(range)` / `Losers(range)` | `EvalRequirement{Controls, range}` / `EvalRequirement{Losers(Classic), range}` (`controls` / `LTC` 単独は Atom なし) |
| `usually`, `normally`, `typically`, `likely`, `mostly` | hedge (`Fragment.hedged`) | 直後の断片に `hedged = true`; `flags.soft = true`; Atom はそのまま採用 (緩めない) |
| `may`, `may be`, `may have`, `might`, `possibly`, `perhaps`, `maybe`, `rarely`, `occasionally`, `sometimes`, `(?)` | 可能性の hedge (`Fragment.possibility`) | `hedged = true`、`flags.soft = true` だが Atom は採用しない (`ANY`)。「〜もありうる」「まれに〜」は要件ではない (`may be 6!h` を「ちょうど 6 枚」にしない)。`(may be …)` のように開き括弧の直後でも hedge。`be`/`have`/`hold`/… は読み飛ばす |
| `not`, `no`, `without`, `w/o`, `denies`, `non` | negation (`Fragment.negated`) | 直後の断片を `Not` |
| `unlimited`, `wide range`, `wide ranged`, `any distribution`, `any hand`, `any strength`, `any`, `might be strong`, `min/max` | `NoBound` | 認識済み、制約なし (`might be strong` と `min/max` は hedge/強さ語を剥がす前に句全体で照合する) |
| `CONST`, `constructive`, `positive`, `sound` | v2 | `StrengthWord` に variant が無い。未認識 (`Unrecognized`) として認識率に計上 |
| `CTRL` (cue-bid の意味), `no outside A/K`, `0--1 outside A/K`, `9 tricks`, `playing tricks`, `QT` | v2 | 未認識 |
| `1st/2nd`, `3rd seat`, `by passed hand`, `PH`, `NV`, `VUL` (文中の席/vul 条件) | v2 | 未認識 (行レベルの条件は `#SEAT`/`#VUL` で書く) |
| `!s>=!h`, `!s > !h`, `!h=!s`, `!d<=!c`, `!c<!d`, `M>oM`, `m>=om` (ext、フェーズ 4) | `LengthOrder(a, cmp, b)` | `shapes ∩= {a の枚数 cmp b の枚数 である全シェイプ}` (560 シェイプから前計算)。演算子は `>=` `>` `=` `<=` `<` の 5 つで、前後に空白 1 つまで置ける。両辺はスート記号 (`!c !d !h !s`) か、展開時にそれへ置換される束縛済み変数 (`M`、`oM`、`m`、`om`、`X/Y/Z`) で、同じスート同士は認識しない。右辺の直後が英数字か `+`/`-` なら認識しない (`!s>=!hx`、`!h>=!s+1`。差の指定は無く、`!h>=!s` と読むと著者が除いた同数を認めてしまう)。`suit_len` を固定しないので、`NAT` の既定長を打ち消さない (`Shape` と違い `suit_len_pins` の対象外)。`tokens.rs::match_length_order`、`length_order_shapes`、`context.rs::resolve_one` |
| 言葉によるスート間の相対比較 (`longer major`, `better minor`, `longest suit`, `5+ in a major`)、オナー位置 (`values in the bid suits`, `K or Q in partner's suit`, `CONC`)、`stoppers in two side suits`、相互参照 (`same structure as over 1NT-2!d`, `see 2M opening`) | v2 | 未認識 (`Unrecognized`) として認識率に計上。`description` にはそのまま残る |

### 7.5 Pass 2 の解決規則 (`context.rs`)

親連鎖は `path` 上の祖先ノード (既にコンパイル済み) から `RowContext.own_prev` / `partner_last` として取る。`partner_min/max` はパートナーがこの経路で示した全範囲の交差 `RowContext.partner_hcp` (§7.2。未追跡なら `partner_last.constraint.hcp_range()`)。例えば `1N (15-17) - 2C (STAY) - 2H - 3S = S/T` の `S/T` は、HCP を書かない `2H` ではなく 1N の 15-17 から `31 − 17 = 14` になる。範囲が全域 `0..=hcp_max` のままなら「不明」であって「任意」ではない。パートナーの範囲が不明 (HCP を示したノードが無い、祖先にワイルドカード (`Class`) がある、祖先が `Partial` にしか対応しない) なら `opening_min..=21` を仮定し `assumed = true` とする (`AssumedContext` が出る)。閾値は全て `meta.strength` (`StrengthVocab`) から取る。

| 断片 | 式 | `Source` |
| --- | --- | --- |
| `GF` | `own_min = clamp(gf_total − partner_min)`; `flags.forcing = ToGame` | `Context` |
| `INV` | `hcp = (inv_total.start − partner_min)..=(inv_total.end − partner_min)` (mild −1 / strong +1 のシフト); `INV+` は `start` のみ; `at most` は `end` のみ; `0..=hcp_max` にクランプ | `Context` |
| `MIN` / `MAX` | 自分のこれまでの範囲 `[a, b]` (`RowContext.own_hcp`、同様に全域は不明扱い) → `MIN: [a, ⌊(a+b)/2⌋]`、`MAX: [⌊(a+b)/2⌋+1, b]`。範囲が不明なら、オープナーは `[opening_min, opening_min+2]` / `[opening_min+3, 21]` を仮定、それ以外 (レスポンダーやアドバンサーの最初のコール) は半分を取る基準が無いので制約なし。どちらも `assumed`。同じ説明文に連言で種別語 (`weak`/`PRE`/`NEG`/`STR`) があり、上の半分がその範囲と交わらない (追跡範囲がオープニングの `Or` 枝の包絡である場合など) ときは、その種別語の範囲の半分を取る (`weak-two, MAX` = ウィークツーの上半分) | `Context` |
| `weak` / `NEG` | レスポンダー: `0..=weak_max` / `0..=neg_max`; オープナーの 2 レベル: `natural.weak_two.1`; 3〜5 レベル: `natural.preempt[L].2`; オーバーコーラー: `natural.overcall[2].1` (jump overcall) | `Context` / `NaturalDefault` |
| `PRE` | `weak` ∧ `suit_len[Own] ≥ 4 + L` (明示の長さがあればそちらを優先) | `Context` |
| `STR` | `hcp.start = strong_min` | `Explicit` |
| `S/T` | `hcp.start = slam_total − partner_max` | `Context` |
| `QUANT` (NT の後) | スモールスラムへの招待: `small = slam_total + 2` (既定 33) として `hcp = (small − partner_max)..=(small − partner_min − 1)` (空なら下端に広げる)。1N 15-17 の後の 4N は `16..=17` (パートナーが最大なら 6NT に届き、最小なら届かない)。同じ説明文の `INV`/`INV+`/mild/strong は `QUANT` が招待であることを言っているだけなので HCP を作らない (`QUANT INV to 6NT` がゲーム招待の範囲と交差して空になるのを防ぐ)。旧式 `(gf_total − partner_max + 1)..=(slam_total − partner_min)` (= `9..=16`) はゲーム招待の値まで含み誤りだったので改めた | `Context` |
| `NAT` | `NaturalInference::infer(classify(path 相当のオークション, own index, owner))` の制約を採用し、明示の断片と交差する (衝突は明示が勝つ)。信頼度は使わない | `NaturalDefault` |
| (全 `StrengthWord` 共通) | その語と**連言で並ぶ** (同じ `And` の中にあり、否定されておらず、`may`/`maybe` などの可能性ヘッジでもない。`Or` はすべての枝が述べる場合のみ数える) 明示の `Hcp`/`Points` 断片があれば、`GF`/`INV`/`MIN`/`MAX`/`weak`/`PRE`/`STR`/`S/T`/`QUANT` など全ての `StrengthWord` が Pass 2 で作る HCP の Atom は採用せず `Atom::ANY` にする (`NAT` 用の「衝突は明示が勝つ」を全語に一般化したもの。例: jdh8 `3C = INV, 7+!c, 4--7 HCP` は著者自身の `4--7 HCP` を残し、`INV` の文脈由来レンジは捨てる)。`Forcing`/`flags` など HCP 以外への効果 (`GF` の `Forcing::ToGame` 等) はそのまま残る。否定 (`GF, not 20+ HCP` → 13..=19)、可能性 (`GF, may have 11 HCP` → `GF` のまま)、別の `Or` 枝 (`weak or 16+ HCP`) の数値では語を捨てない。`NAT` の自スート長、`SPL` の各部分の「明示が勝つ」も同じ連言判定を使う (`NAT, not 4!c` や `NAT, maybe 3!c` や `NAT, 6+!d or 4!s` では `NAT` の長さを残す) | `Context` (Atom は破棄) |
| `#`, `Own`, `AnyMajor/AnyMinor`, `Agreed`, `Theirs` | 具体スート、または群上の `Or`。`#` は経路を自分のコールから遡り、最初の `Var`/`Strains`/`AnyOf` コール (相手の `(2HS)` も含む) のスート (`RowContext.hash_suit`)。`Strains` で複数なら `Or` | `Context` |
| `support` / `fit` / `SUPP` / レイズ行 | `agreed = agreed_suit` かパートナーの最後のスートビッド (`flags.artificial` なコール、例えば puppet やステップレスポンスは対象外。人工コールは実際のスートを示さないので、それをそのまま合意スートに使うと `SPL` の明示のショートネス断片と衝突しうる); `support_min = max(3, 8 − partner_len_min)` (`partner_len_min` は `partner_last.constraint.suit_len(agreed).start`、不明なら 3); `suit_len[agreed] ≥ support_min`。`call.strain == agreed` のレイズ行は、文が無くても `meta.natural.implicit_raise_support` (既定 true) なら `Support(3)` を付ける (`assumed`) | `Context` |
| `SPL` | `suit_len[short] ≤ 1` ∧ `suit_len[agreed] ≥ conventions.splinter_support` ∧ `GF` の式 (§7.4 の `SPL` 行の除外規則つき) | `Context` |
| (強さ語どうしの衝突) | 同じ説明文で `Or` の下に無い (連言の) 強さ語の HCP 範囲が互いに素になったら、`assumed` な文脈に基づく語の Atom を捨てて、既知の文脈に基づく語を残す (既知が 1 つも無ければ何もしない)。例: gjp `1C-2H (5-9 HCP) - 2N - 3H = MAX, FG`: `MAX` は自分の 5-9 から `8..=9`、`FG` は不明なパートナー範囲を仮定した `13+` なので `FG` 側を捨てる | `Context` |
| `TRF` | §7.4 の行のとおり。`flags.transfer_to` | `Context` |
| `UNT` / 2 スート型の `unbid` | `unbid = 全スート − our_suits − their_suits` の下位 2 つに `≥ 5` (定数); 2 スート型は同じ集合上で長さ割当の `Or` | `Context` |

各解決は `Provenance { span, assumed, source }` を残し、`assumed == true` の断片が 1 つでもあれば `AssumedContext` (Info) を出す (リスク R3)。`Source::Explicit` は説明文に書かれた値、`Context` は経路から導いた値、`NaturalDefault` はナチュラル既定から借りた値。

### 7.6 組立と hedge

- 断片は `andgroup` 内で `And`、`orgroup` で `Or`、`clauses` の `,`/`;` で `And`。列挙は `Or` 群。
- 否定は `Not` で包み、`to_dnf` (`05-constraint.md` §4.2 の排他的連鎖) に任せる。
- 蓋然性の hedge (`usually`, `normally`, …) は制約を緩めず `hedged = true` を記録し、ノードの `flags.soft = true` と `SoftConstraint` (Info) を出す。L3 の ε 混合 (D15) が事後補正を担うので、ここで「HCP を 2 広げる」ような場当たりな緩和はしない。可能性の hedge (`may`, `might`, `rarely`, `occasionally`, …) は可能性や例外を述べるだけなので、断片のリテラルを採らない (`ANY`)。`soft` と `SoftConstraint` は同じく出す。
- `flags.forcing` は節の木で集約する: `And` の中では強い方 (`ToGame` > `OneRound` > `NonForcing`)、`Or` の枝どうしでは全枝が一致する値だけ (全枝が forcing だが程度が違えば `OneRound`、そうでなければ不明)。`GF`/`FG` は `ToGame` を述べる。否定された `GF` (`not GF`) と可能性の hedge が付いた語は何も述べない。したがって `weak or GF` や `PRE 7+!c or FG 6+!c` は `ToGame` にならない。
- `Convention` / `Forcing` / `NoBound` は Atom を生まず `flags` だけを更新する。全断片が Atom を生まなければ `constraint = ANY` で `Recognition.constraint_bearing = false`。
- `Or` の枝数が `InterpretOptions.max_alternatives` (8) を超えることは許すが、`to_dnf` の `max_terms = 256` を超えた場合は `residual` に退避される (`05-constraint.md`)。このとき `DnfTruncated` を出す (Warning。`CompileOptions.strict_dnf` なら Error で、そのノードの制約は `ANY` に落とす)。

### 7.7 認識率の定義 (`recognition.rs`)

`Recognition` の型は §5.1。単位は **語** で、正規化後のテキストを空白で分割し、句読点を削り、スート番兵は語に付けたまま (`5+♠` で 1 語) 数える。

- **語集合**: ストップワード `a an the and or with w/ in of at hand suit suits cards points hcp` (`STOPWORDS`) は、断片のスパンに含まれない限り分母から除く (含まれれば中立)。
- **`ratio`** = `covered / (total − 覆われていないストップワード数)`。Atom を生まない認識済みコンベンション (`(R)`, `P/C`, `Forced`) も `covered` に数える。認識はしたが Pass 2 で `assumed` になった断片も `covered` に数え、代わりに `assumed` を増やす。
- 空の説明は `ratio = 1.0`、`total = 0` (`EmptyDescription` Info)。
- `ratio < meta.recognition_threshold` (既定 0.5) で `LowRecognition` (Warning。`constraint_bearing == false` の純コンベンション行は Info)。
- `Row.recognition` は展開ノードの `Recognition` のうち `ratio` 最大のもの (通常は同一)。
- ファイル単位の認識率 (実装順の完了条件 §11) はノードのマイクロ平均 `Σ covered / Σ total` とする (中央値を併記)。

### 7.8 トレース出力

構造化フィールドのみを使い、自由文のフォーマットはしない (grep とパースができる)。ノードごとに `DEBUG` を 1 件、コンパイルごとに `INFO` を 1 件、Error 級 Lint ごとに `WARN` を 1 件。

```
INFO  bridge_system::compile: name="Strawberry Polish Club" rows=1843 nodes=4210 mean_recognition=0.71 rows_below_threshold=212 empty_rows=88 lints_error=3 lints_warn=40 lints_info=210 elapsed_ms=312
DEBUG bridge_system::compile::desc: row="wj/1C.bml:14" path="1C-(P)-2H" desc="!P/C, 7--9 HCP, 4+!h, 5+!s" ratio=1.00 fragments="alert,conv(P/C),hcp(7..=9),len(H,4..=13),len(S,5..=13)" unrecognized="" assumed=0
WARN  bridge_system::lint: code=LowRecognition row="common/2D.bml:12" ratio=0.20 unrecognized="Multi-coloured|one of|weak-two in a major"
```

| レベル | target | 単位 | フィールド |
| --- | --- | --- | --- |
| `DEBUG` | `bridge_system::compile::desc` | ノードごと | `row`, `path`, `desc`, `ratio`, `fragments`, `unrecognized` (先頭 3 件を `\|` 区切り), `assumed` |
| `INFO` | `bridge_system::compile` | コンパイルごと | `name`, `rows`, `nodes`, `mean_recognition`, `rows_below_threshold`, `empty_rows`, `lints_error`, `lints_warn`, `lints_info`, `elapsed_ms` |
| `WARN` | `bridge_system::lint` | Error/Warning 級 Lint ごと | `code`, `row`, 付随する数値 (`ratio` 等) |

### 7.9 v1 と以降

v1 は §7.4 の `v2` 行を除く全て: 節文法、列挙、hedge、長さ/シェイプ/バランスの否定、Pass 2 の強さ語、`#` と変数、ナチュラル既定経由の `NAT`。以降に送るもの: オナー位置と cue の control 意味、`9 tricks` / playing tricks、スート間相対比較、相互参照 (`see …`)、文中の席/vul 条件、トークンごとの信頼度を L3 の選言重みに流す拡張。これらは `Unrecognized` として認識率に計上され、`UnrecognizedFragment` (Info) に現れるので、頻度が高いものから拾う。

---

## 8. `NaturalInference` (`natural.rs`)

### 8.1 型

```rust
// natural.rs
/// #+NATURAL: または TOML から読める。SystemMeta.natural に埋め込まれる。
pub struct NaturalParams {
    pub opening_hcp: RangeInclusive<u8>,                 // 12..=21
    pub open_1m_len: u8,                                 // 3
    pub open_1major_len: u8,                             // 5
    pub nt: Vec<(u8, RangeInclusive<u8>)>,               // [(1, 15..=17), (2, 20..=21), (3, 25..=27)]  (level, hcp)
    pub weak_two: (u8, RangeInclusive<u8>),              // (6, 5..=10)  (min length, hcp)
    pub preempt: Vec<(u8, u8, RangeInclusive<u8>)>,      // [(3, 7, 5..=9), (4, 8, 5..=10), (5, 8, 5..=11)]  (level, min length, hcp)
    pub strong_two_c: u8,                                // 22 (minimum hcp)
    pub overcall: [(u8, RangeInclusive<u8>); 3],         // 1-level (5, 8..=16), 2-level (5, 10..=16), jump (6, 5..=10)
    pub nt_overcall: RangeInclusive<u8>,                 // 15..=18
    pub takeout_double: (u8, u8, u8),                    // (12, 2, 3)  (min hcp, max in their suit, min in unbid suits)
    pub response: ResponseParams,
    pub rebid: RebidParams,
    pub advance: AdvanceParams,
    pub balancing_shift: i8,                             // -3 (HCP shift in the balancing seat)
    pub implicit_raise_support: bool,                    // true
    #[serde(skip)] pub level_floor: LevelFloor,          // LevelFloor::STANDARD (§8.6; not serialised)
}
pub struct LevelFloor { pub suit: [u8; 7], pub nt: [u8; 7] }   // combined HCP by level 1..=7; 0 = no floor
pub struct ResponseParams {
    pub new_suit_1: (u8, u8),                            // (4, 6)   (min length, min hcp)
    pub new_suit_2: (u8, u8),                            // (5, 10)
    pub raise: (u8, RangeInclusive<u8>),                 // (3, 6..=9)   (min support, hcp)
    pub jump_raise: (u8, RangeInclusive<u8>),            // (4, 10..=12)
    pub nt: Vec<(u8, RangeInclusive<u8>)>,               // [(1, 6..=10), (2, 11..=12), (3, 13..=15)]
    pub jump_shift: u8,                                  // 17
}
pub struct RebidParams {
    pub reverse: u8,                                     // 17
    pub jump_rebid: RangeInclusive<u8>,                  // 16..=18
    pub nt_1: RangeInclusive<u8>,                        // 12..=14
    pub nt_2: RangeInclusive<u8>,                        // 18..=19
    pub raise: RangeInclusive<u8>,                       // 12..=15
    pub jump_raise: RangeInclusive<u8>,                  // 16..=18
}
pub struct AdvanceParams {
    pub raise: (u8, RangeInclusive<u8>),                 // (3, 6..=9)
    pub new_suit: (u8, u8),                              // (5, 8)
    pub cue: u8,                                         // 10
}

pub enum Role { Opener, Responder, Overcaller, Advancer, Balancer }
pub enum DoubleKind { Takeout, Penalty, Negative, Responsive, Support, Unknown }
pub enum CallKind {
    Pass,
    Double(DoubleKind),
    Redouble,
    Bid { new_suit: bool, raise: bool, nt: bool, jump: u8, cue: bool, reverse: bool, rebid_own: bool },
}

/// The context of one call, derived from the auction alone.
pub struct CallContext {
    pub role: Role,
    pub kind: CallKind,
    pub call: Call,
    pub level: u8,                        // 0 for P/D/R
    pub position: u8,                     // opener position 1..=4
    pub passed_hand: bool,
    pub vul: (bool, bool),                // (we, they)
    pub competitive: bool,                // both sides have bid
    pub partner_last: Option<Call>,
    pub partner_constraint: Option<HandConstraint>,   // None from classify; the bidding layer may fill it in
    pub our_suits: StrainSet,
    pub their_suits: StrainSet,
    pub agreed_suit: Option<Suit>,
    pub last_bid: Option<Bid>,
    pub forcing_situation: bool,          // false from classify; the bidding layer may fill it in
    pub opener_first_suit: Option<(Suit, u8)>,  // owner's first suit Call::Bid; NT skipped
    pub opener_first_bid: Option<Bid>,          // owner's first Call::Bid of any strain, NT included
    pub owner_acted: bool,                      // owner already made a non-pass call (bid/double/redouble)
    pub partner_actions: u8,                    // number of partner's non-pass calls so far
    pub partner_first_action: Option<Call>,     // partner's first non-pass call
    pub partner_first_jump: u8,                 // levels skipped by that call when it is a bid (0 otherwise)
}

/// System-independent classification of auction[index] as made by `owner`; unit-testable.
pub fn classify(auction: &Auction, index: usize, owner: Seat) -> CallContext;

pub struct Inference { pub constraint: HandConstraint, pub confidence: f32, pub rule: &'static str, pub explanation: String }

pub struct NaturalInference { params: NaturalParams }
impl NaturalInference {
    pub fn new(params: NaturalParams) -> Self;
    pub fn params(&self) -> &NaturalParams;
    pub fn infer(&self, ctx: &CallContext) -> Inference;                 // first matching rule
    /// Candidate (call, constraint, priority) triples for choose_bid when off-system.
    pub fn candidates(&self, auction: &Auction, owner: Seat) -> Vec<(Call, HandConstraint, i16)>;
    // Phase 4 (§8.6):
    pub fn infer_batch(&self, auction: &Auction, owner: Seat, partner: &PartnerContext, calls: &[Call]) -> Vec<NaturalCandidate>;
    pub fn ranked_candidates(&self, auction: &Auction, owner: Seat, partner: &PartnerContext, tie_break: TieBreak) -> Vec<NaturalCandidate>;
    pub fn implicit_pass(ranked: &[NaturalCandidate]) -> Option<NaturalCandidate>;
}
pub struct PartnerContext { pub partner_constraint: Option<HandConstraint>, pub forcing_situation: bool }
pub struct NaturalCandidate { pub call: Call, pub constraint: HandConstraint, pub confidence: f32, pub rule: &'static str }
impl Default for NaturalInference { /* NaturalParams::default() */ }
```

コメントの値が `NaturalParams::default()` (SAYC 準拠。骨格では `todo!()` で、フェーズ 3.11 で上の値を実装する) で、`#+NATURAL:` と TOML で上書きできる。`balancing_shift` は `Role::Balancer` のとき HCP 下限に加える (−3)。`implicit_raise_support` は説明文コンパイラのレイズ行 (§7.5) が読む。`classify` は L3 の型に依存しない (L2 → L3 の循環を避ける) ので、`partner_constraint` と `forcing_situation` は `None` / `false` で返し、L3 が解釈済みノードから埋めてよい (`07-bidding.md` §2.2)。

### 8.2 `classify` の判定

1. `role`: 我々側の最初の非パスがオープニングなら `Opener`/`Responder`、相手のオープニングの後なら `Overcaller`/`Advancer`。相手の `(bid) P (P)` の直後のコールは `Balancer`。
2. `kind`: `Pass` / `Redouble` はそのまま。`Double` は上から順に、直前の相手のコールがスートビッドで 2 レベル以下かつパートナーがまだビッドしていなければ (自分の以前のオーバーコールは妨げない。再開のダブルもテイクアウト) `Takeout`、パートナーの最後のビッドが NT (1NT オープンなど) なら `Penalty`、パートナーがオープンした後の相手のオーバーコール (2S 以下) に対してなら `Negative`、相手が NT または 4 レベル以上、あるいは我々が既にスートを合意していれば (自分とパートナーの両方がビッドしたスートがある。`agreed_suit` と同じ判定) `Penalty`、パートナーのテイクアウトダブルの後の相手のレイズに対してなら `Responsive`、パートナーの応答スートへの相手の介入に対する低レベルなら `Support`、それ以外 `Unknown`。`Bid` は `our_suits` / `their_suits` / `partner_last` から `new_suit`、`raise` (パートナーが **先に** ビッドしたスート。自分が先にビッドしパートナーがサポートしただけのスート (`1H-P-2H-P-3H`) は `raise` ではなく `rebid_own`)、`nt`、`jump` (最小合法レベルとの差)、`cue` (相手のスート)、`reverse` (オープナーが 1 レベルで開いたスートより上位の新スートを 2 レベルで)、`rebid_own` を決める。
3. `position`、`passed_hand`、`vul`、`competitive`、`our_suits`、`their_suits`、`agreed_suit` (同じスートをパートナーと自分がビッドした)、`last_bid`、`owner_acted`、`partner_actions`、`partner_first_action`、`partner_first_jump` はオークションから直接。`partner_constraint = None`、`forcing_situation = false`。

### 8.3 規則表 (順序付き。最初に述語が成り立つ行を採る)

| 規則 | 述語 | 制約 (`NaturalParams` から) | confidence |
| --- | --- | --- | --- |
| `open_1M` | `Opener`, `Bid{new_suit}`, `level 1`, メジャー | `suit_len[s] ≥ open_1major_len` ∧ `hcp = opening_hcp` | 0.6 |
| `open_1m` | `Opener`, `level 1`, マイナー | `suit_len[s] ≥ open_1m_len` ∧ `hcp = opening_hcp` | 0.55 (旧 0.6) |
| `open_nt` | `Opener`, `nt`, レベル `L` が `nt` 表にある | `hcp = nt[L]` ∧ `shapes = BALANCED` | 0.7 |
| `open_weak2` | `Opener`, `level 2`, スート ≠ C | `suit_len[s] ≥ weak_two.0` ∧ `hcp = weak_two.1` | 0.5 |
| `open_2c` | `Opener`, `2C` | `hcp ≥ strong_two_c` (シェイプなし) | 0.5 |
| `open_preempt` | `Opener`, `level 3..=5`, スート | `suit_len[s] ≥ preempt[L].1` ∧ `hcp = preempt[L].2` | 0.55 (旧 0.5) |
| `open_pass` | `Opener` の席の `Pass` (パス済みでない、かつまだ誰もビッドしていない: `last_bid == None`。オープナーの後のパスはここに来ない) | `hcp ≤ opening_hcp.start − 1` | 0.45 (旧 0.5。`open_weak2` より下に置く。§8.6 の「レビュー後の修正」) |
| `overcall` | `Overcaller` の最初のアクション (`!owner_acted`), `new_suit` (相手スートのキュービッドを除く), `jump == 0` | `suit_len[s] ≥ overcall[l].0` ∧ `hcp = overcall[l].1` (`l` = 0: 1 レベル、1: 2 レベル以上); `Balancer` は下限に `balancing_shift` | 0.35 (旧 0.5) |
| `jump_overcall` | `Overcaller` の最初のアクション, `new_suit`, `jump == 1` | `suit_len[s] ≥ overcall[2].0` ∧ `hcp = overcall[2].1` | 0.5 (旧 0.4) |
| `nt_overcall` | `Overcaller` の最初のアクション, 最安の NT (`jump == 0`) で 2 レベル以下 (1 レベルのオープンに 1N、ウィーク・ツーに 2N) | `hcp = nt_overcall` ∧ `BALANCED` ∧ `Stopper(their suit)` | 0.6 |
| `takeout_x` | `Double(Takeout)`: パートナー未ビッド、相手のスートが 2 レベル以下 | `hcp ≥ takeout_double.0` ∧ `suit_len[their] ≤ takeout_double.1` ∧ 未ビッドスート各 `≥ takeout_double.2` (未ビッドが 3 つ以上なら `Or` で 2 つ以上を要求) | 0.45 (旧 0.5) |
| `penalty_x` | `Double(Penalty)`: パートナーの最後のビッドが NT、相手が NT または 4 レベル以上、または我々がスートを合意済み | `hcp ≥ 10` ∧ `suit_len[their] ≥ 4` | 0.3 |
| `negative_x` | `Double(Negative)`: パートナーがスートを開き RHO が 2 レベル以下でオーバーコール | `hcp ≥ response.new_suit_1.1 + 2 × (level − 1)` ∧ 未ビッドメジャーの条件: 両メジャーが未ビッドで 1 レベルで言える (`1C (1D) X`) なら両方 `≥ 4`; 1 つだけ未ビッドで 1 レベルで言える (`1C (1H) X`) ならちょうど 4 枚; それ以外 (2 レベルのオーバーコール) は未ビッドメジャー `≥ 4` (`Or`) | 両メジャーの場合 0.5、それ以外 0.4 (旧 0.5) |
| `raise` | パートナーが先にビッドしたスート `s` を我々がビッド | `suit_len[s] ≥ raise.0` ∧ 役割/レベル別の `hcp` (`Responder`: `response.raise.1` 単純、`response.jump_raise.1` ジャンプ、ゲームレイズ `13+`; `Advancer`: `advance.raise`; `Opener`: `rebid.raise` / `rebid.jump_raise`) | 0.45 (旧 0.6) |
| `new_suit_resp_1` | `Responder`, `new_suit`, `level 1` | `suit_len[s] ≥ response.new_suit_1.0` ∧ `hcp ≥ response.new_suit_1.1`。相手がメジャーでオーバーコールした後 (`1C (1H) 1S`) は長さ `≥ 5` (4 枚はネガティブ・ダブル) | 0.5 |
| `new_suit_resp_2` | `Responder`, `new_suit`, `level 2`, `jump == 0` | `suit_len[s] ≥ response.new_suit_2.0` (5) ∧ `hcp ≥ response.new_suit_2.1` (10); `jump == 1` は `hcp ≥ response.jump_shift` | 0.6 (旧 0.5) |
| `resp_nt` | `Responder`, `nt`, レベル `L` | `hcp = response.nt[L]` (2N/3N は `BALANCED` 寄り: 4 メジャー否定は v2)。最初の応答の 1N はシンプル・レイズを否定する: パートナーのスート `s` について ¬(`suit_len[s] ≥ 支持` ∧ `hcp ∈ response.raise.1`)。支持はメジャーで `response.raise.0` (3)、マイナーで 5 (SAYC の `1H-1N` は「3 枚以上のハートなし」、`1C-1N` は「5 枚以上のクラブなし」)。10 HCP の支持付きは 1N でよい | 0.5 |
| `rebid_own` | `Opener`, `rebid_own` | `suit_len[s] ≥ 6` ∧ `hcp = opening_hcp` (`jump == 1` なら `rebid.jump_rebid`)。1 スートのオープン後に合意済みスート (自分が先にビッドしパートナーがサポートしたスート) を再び上げる場合は別枝: 長さはオープンの最小長 (2 番目のスートなら 4)、`hcp` は競り合いなしのジャンプなしが `rebid.jump_rebid` (ゲームトライ)、競り合いでは `opening_hcp`、ジャンプは `rebid.jump_rebid.start..=opening_hcp.end` | ジャンプ (`jump ≥ 1`、`rebid.jump_rebid`) は 0.55、それ以外は 0.5 (旧はどちらも 0.5。ジャンプの 16〜18 は非ジャンプの 12〜21 に含まれ、同点ではコール順で安い非ジャンプが勝つので、ジャンプ・リビッドが一度も選ばれなかった) |
| `reverse` | `Opener`, `reverse` (最初のスートの 2 レベルより上の新スート) | `hcp ≥ rebid.reverse` ∧ 最初のスート `≥ 5` ∧ 2 番目 `≥ 4` | 0.55 (旧 0.4) |
| `rebid_nt` | `Opener` (1 スートのオープン後), `nt`, 2 レベル以下、合意スートなし | `BALANCED` ∧ `hcp = rebid.nt_1` (`jump == 0`) / `rebid.nt_2` (`jump == 1`)。それより大きいジャンプは `fallback` | 0.45 (旧 0.5) |
| `rebid_new_suit` | `Opener` (1 スートのオープン後), `new_suit` (リバースでない。`reverse` が先に当たる) | `suit_len[s] ≥ 4` ∧ `hcp = opening_hcp.start..=rebid.jump_raise.end` (`jump ≥ 1` のジャンプシフトは `hcp ≥ rebid.jump_rebid.end + 1`) | 0.35 (旧 0.4) |
| `advance_new_suit` | `Advancer` の最初のアクション, パートナーの最初のアクションがビッド (テイクアウトダブルへの応答は除く), `new_suit`, `jump == 0` | `suit_len[s] ≥ advance.new_suit.0` ∧ `hcp ≥ advance.new_suit.1` | 0.5 |
| `cue` | 相手のスートのビッド | シェイプなし; `Opener`/`Responder` で `partner_constraint` があれば `hcp ≥ gf_total − partner_min` (`gf_total = 25`)、それ以外 (`Advancer`/`Overcaller`/`Balancer`、または `partner_constraint` なし) は `advance.cue`; `flags.artificial` | 0.3 |
| `pass_forcing` | `Pass` かつ `forcing_situation` (矛盾) | `hcp = 0..=0` (充足不能に近い制約。L3 が ε 混合で重みを下げる) | 0.1 |
| `pass_default` | `Pass` | 上限は、自分がまだアクションしておらず (`!owner_acted`) パートナーの非パスがオープン/オーバーコールの 1 回だけ (`partner_actions == 1`) のときだけ付き、パートナーのコールで決まる: `Responder` で 1 レベルのスートオープン → `hcp ≤ response.new_suit_1.1 − 1`、1N/2N オープン → `hcp ≤ gf_total − nt[L].end − 1` (1N なら 7); `Advancer` でジャンプでない 1〜2 レベルのスートオーバーコール → `hcp ≤ advance.raise.1.start − 1`、NT オーバーコール → `hcp ≤ gf_total − nt_overcall.end − 1`。それ以外 (ウィーク・ツー、プリエンプト、2C、ダブル、ジャンプオーバーコール、2 回目以降のパス、`Opener`/`Overcaller` のパス) は `ANY` | 上限付き 0.4、`ANY` は 0.3 (旧 0.2。どの手も満たすパスは、同じ優先度の規則を `choose_bid` のコール順タイブレークで常に負かす。0.3 は `cue` と `penalty_x` だけと同点で、この 2 規則を意図して覆う。§8.6) |
| `fallback` | 上記のどれにも該当しない | `ANY` | 0.05 |

confidence 列の値はフェーズ 4.6 で調整したもので、「旧」はフェーズ 3 の値である (§8.6)。確信度は rank の priority `round(confidence × 100)` としてしか使わないので、値そのものより順序に意味がある。

`Balancer` は対応する `Overcaller` 規則を使い、HCP 下限に `balancing_shift` を加える。`candidates(auction, owner)` は合法コールの各々に `classify` + `infer` を適用し、`priority = round(confidence × 100)` を付けて返す (`fallback` 行のコールは除く)。

**フェーズ 3 統合レビューによる訂正 (2026-09-26)。** (1) `open_pass` はオープナーの後のパス (`1H-P-2H-P-P`、`1NT-P-3NT-P-P`) にも当たり、オープンと矛盾する 0–11 HCP を与えていた: `last_bid == None` を条件に追加。(2) `pass_default` はレスポンダー/アドバンサーのあらゆるパスを 0–5 に制限していた (1S と応答した後のパスや、パートナーの 1NT・ウィーク・ツーへのパスも): 上表の通り最初のアクションだけに、パートナーのコールに応じた上限を付ける。(3) `overcall`/`jump_overcall` がキュービッド (`1H-(2H)`) とオーバーコーラー自身の 2 回目以降のビッドを新しいオーバーコールと読み、`nt_overcall` が任意レベルの NT (`1H-(2NT)` のアンユージュアル、3NT) に当たっていた: 最初のアクション・`new_suit`・最安 NT に限定。(4) `cue` の GF 式はアドバンサーのキュー (パートナーのオーバーコール最小 8 なら 17+) やオーバーコーラーのキューには当てはまらない: `Opener`/`Responder` だけに限定。(5) ダブル分類を §8.2 の文言に合わせた: パートナーの NT の後は `Penalty`、テイクアウトの条件は「パートナーが未ビッド」(自分のオーバーコール後の再開ダブルも `Takeout`)、合意スートは自分とパートナーの両方がビッドしたスートで判定。(6) `raise` はパートナーが先にビッドしたスートに限り、オープナーの合意スートの再レイズ (`1H-P-2H-P-3H`) は `rebid_own` の専用枝 (ゲームトライ) に回す。(7) `NaturalParams` にあるのにどの規則も使っていなかった `rebid.nt_1`/`nt_2`/`advance.new_suit` に規則 (`rebid_nt`、`rebid_new_suit`、`advance_new_suit`) を足した。これらの通常のナチュラルコールは従来 `fallback` に落ち、`candidates` が決して出さなかった。既知の限界: レスポンダーやオーバーコーラーが、パートナーのサポートを受けた自分のスートを再び上げるコール (`1H-P-1S-P-2S-P-3S` のレスポンダー) は `raise` でなくなり、対応する規則が無いので `fallback` になる。(8) (1) の後、オープナーの後のパスは `pass_default` の `ANY` になるが、confidence 0.4 のままでは `choose_bid` のコール順タイブレーク (`Pass` が最小) で同じ 0.4 の規則 (`reverse`、`rebid_new_suit`、`jump_overcall`) を常に負かした (再現率の `reverse` が 0.73 → 0.0): 上限なしのパスは 0.2 にした。回帰テストは `crates/bridge-system/tests/natural_regressions.rs`。これらの修正後の §8.5 測定 2 (再現率) は、このレーン単独では 600 決定点・8,046 候補で全体一致率 0.342 (修正前 7,593 候補・0.331)。記述コンパイラ・パーサの修正と統合した後の値 (8,059 候補・0.336) は `11-testing.md` §6 にある。

**測定 (§8.5) から見つかった訂正と既知の限界。** `rule_rebid_own` は元々ジャンプなしの自己スート・リビッドを常に `opening_hcp` (12–21) と比較していたが、オープニングがウィーク・ツーやプリエンプトの場合これは無関係などころか非交叉の範囲であり (`weak_two.1` は 5–10)、隠しノード比較の recall/precision が恒常的に 0 になっていた。`crates/bridge-system/src/natural.rs` の `opening_level_hcp` ヘルパーで、オープナー自身の最初の `Call::Bid` (`CallContext::opener_first_bid`。NT を除外する `opener_first_suit` とは違い NT オープンも含む) のストレイン・レベルからウィーク・ツー (`weak_two.1`)、プリエンプト (`preempt` 表の該当レベル)、ストロング 2C (`strong_two_c..=37`) の HCP を選ぶよう修正済み。当初 `opener_first_suit` のレベルだけを見る実装で直したところ、1NT/2NT のトランスファー完成 (例 `1NT-P-2D-P-2H`) が「レベル 2 のスート」として記録され、後続のリビッドがウィーク・ツーの範囲と誤判定される回帰が見つかったため、`opener_first_bid` (NT を含むオープナーの最初のビッドそのもの) に基づく判定に直した (回帰テスト `rebid_own_after_weak_two_uses_weak_two_hcp` / `rebid_own_after_preempt_uses_preempt_hcp` / `rebid_own_after_nt_transfer_completion_uses_opening_hcp` / `rebid_own_after_strong_2c_uses_strong_two_c_hcp`)。同じ測定で `rule_resp_nt` にも既知の限界が見つかった: パートナー自身の 1NT/2NT オープンへの定量的レイズ (例 `1NT-P-2NT` は sayc.bml で 8–9 hcp) と、スート・オープンへのジャンプ NT レスポンス (`response.nt` 表、レベル 2 は 11–12 hcp) を同じ規則・同じ HCP 表で扱っており、前者は後者の表と非交叉になる。`NaturalParams` の既存フィールドだけでは区別できず (新フィールドの追加は `ir.rs`/`compile/meta.rs` 側の変更を伴い、このレーンの担当範囲外)、フェーズ 4 で `ResponseParams` に「パートナーの NT オープンへのレイズ」用のフィールドを足すかどうか検討する。

### 8.4 位置づけ

L3 は `Resolution::Natural` を作るときこのモジュールを呼ぶ (`eps_natural = 0.30` の ε 混合)。`NaturalParams` は `SystemMeta.natural` に埋め込まれるので、席ごとに異なるナチュラル既定を持てる。モジュールはシステム定義から独立しており、`classify` と `infer` は `bridge-core` の `Auction` だけで単体テストできる。

### 8.5 精度測定 (D8。`--ignored` 統合テスト、JSON 出力)

正解データが無いので、システム定義とコーパスを擬似正解に使う (測定 1, 2 はラベル不要、3 は実手)。

| # | 測定 | 手順 | 出力 |
| --- | --- | --- | --- |
| 1 | 隠しノード比較 | 実システム (`sayc.bml`、取得済みなら jdh8 Polish Club / gpaulissen) の `flags.artificial == false` かつ `alertable != Alertable` の各ノードについて、そのノードを隠して同じオークション・同じコールで `infer(classify(...))` を計算。ノード制約 `C_sys` から 1,000 手、推定制約 `C_nat` から 1,000 手をサンプルし、`recall = P(h ⊨ C_nat \| h ~ C_sys)`、`precision = P(h ⊨ C_sys \| h ~ C_nat)`、体積比 `vol(C_nat) / vol(C_sys)` (`2^(volume_log2 の差)`) | `Role × CallKind` 別の平均と分位、ノード別一覧。コンベンショナルなノードは「expected miss」として別枠 |
| 2 | 再現率 (仕様 §10 の逆方向) | 推定制約 `C_nat` から手をサンプルし、`choose_bid` (システム外なので `natural.candidates`) でその局面を再生して同じコールが選ばれる率 | 規則別の一致率 |
| 3 | コーパス充足率 | フェーズ 1 の PBN コーパス (deal 付きオークション) の各コールについて、実際の手が `C_nat` を満たす率と `volume_log2` の散布 (パレート: 緩い推定は充足率 100% で体積が大きい) | 規則別の (充足率, log 体積) 点列 |

3 つとも `criterion` を使わない `--ignored` 統合テスト (`tests/natural_metrics.rs`) で、`target/natural_metrics.json` に出す。`NaturalParams` の変種を掃引できる。`sayc.bml` は自作なので測定 1 は循環の懸念があり、外部ファイルが取得できる環境では jdh8/gpaulissen を優先し、`sayc.bml` の結果は別枠で報告する。測定 3 はコーパスの取得 (フェーズ 1) に依存し、それまでは 1, 2 だけを回す。

### 8.6 フェーズ 4 の追加 (順位、一括推定、レベル下限、暗黙パス、4.6 の調整)

**順位 (`ranked_candidates`)。** ナチュラル候補は、合法コールのうち `fallback` でないものを、`exclusive::natural_rank_cmp` の順に並べたものである。順は priority `round(confidence × 100)` 降順 → `tie_break` (`LowestCall` / `HighestCall` のときだけ効く) → コール index 昇順。ナチュラル方策は、手が最初に満たす候補を選ぶ。`choose_bid` のナチュラル分岐と `interpret` のナチュラル排他は、どちらもこの順を使う。`partner` (`PartnerContext`) は、`classify` がオークションだけからは作れない `partner_constraint` / `forcing_situation` を与える。`choose_bid` は、プレフィックスの解釈からこれを埋める (`07-bidding.md` §2.2)。

**一括推定 (`infer_batch`)。** 同じ履歴に対する複数のコールの推定を 1 回の呼び出しで行う。

- `classify` の履歴依存部分 (役割、ビッド済みスート、パートナーの行動など) は 1 回だけ計算し、コールごとには `kind` / `call` / `level` だけを差し替える。
- パートナー文脈は 1 回だけ設定し、継続オークションは作らず、説明文字列も作らない。
- 結果の制約・確信度・規則は、各コールについて `classify(&auction.with(call), …)` + `infer` と同一でなければならない。合法でないコールは `fallback` (`ANY`、確信度 0.05) を返す。
- 同一性の確認:
  - `tests/natural_batch.rs`: 固定 6 + 乱数 60 オークション × 3 パートナー文脈 × 2 エンジンで、114,570 組。
  - `bridge-bidding` の `natural_metrics.rs`: `#[ignore]` 版は生成 1000 + コーパス 500 オークションで、859,954 組 (確信度の調整前は 1,025,270 組、4.6 の採用時点は 860,516 組。規則の変更で生成オークションが変わるため)。
- 速度 (`1NT P 2C` の後の合法 31 コール、release、best of 3、loadavg 6.5):

  | 方法 | 時間 |
  | --- | --- |
  | コールごとの `classify` + `infer` | 13.2 μs |
  | `infer_batch` | 2.46 μs |
  | `ranked_candidates` (並べ替えまで含む) | 2.87 μs |

  目標の 4 μs 以下を満たす。

**レベル下限 (`NaturalParams.level_floor`)。** 3 レベル以上のナチュラルな継続ビッドに、パートナーシップの合計 HCP の下限を課す。対象は、自分かパートナーが既に行動していて、`partner_constraint` が分かっている場合に限る。このとき、自分の HCP ≥ combined(level, NT) − パートナーの最小 HCP を推定制約に `And` する。

- 既定の表 `LevelFloor::STANDARD` (プロトタイプ C の値):
  - スート: 3 → 18、4 → 22、5 → 26、6 → 31、7 → 35
  - NT: 3 → 24、4 → 28、5 → 30、6 → 32、7 → 36
- 表の値 0 は「下限なし」を意味する。`LevelFloor::NONE` はフェーズ 3 の挙動に戻す。
- オープニング、パートナーが無言のまま自分が初めて行動する場合、`partner_constraint` が無い場合 (素の `classify`) は、下限を課さない。したがって §8.5 の測定 1 と 3 (素の `classify` + `infer`) は、下限の影響を受けない。
- `level_floor` は `#[serde(skip)]` である。直列化形式と `IR_FORMAT` は変わらず、復元した IR は既定の表を持つ。
- 検証 (`natural_metrics.rs` の `level_floor_limits_replay_escalation_2000`): 固定シードの生成配牌 2000 を SAYC + ナチュラル補完で `replay` した。最終コントラクトのレベル分布は次のとおり。

  | 条件 | 分布 [パスアウト, 1..7] | 7 レベル | 6 レベル以上 |
  | --- | --- | --- | --- |
  | 下限なし、旧確信度 | [32, 48, 208, 256, 139, 45, 12, 1260] | 63.0% | 63.6% |
  | 下限なし、4.6 の確信度 | [32, 92, 379, 573, 396, 131, 28, 369] | 18.5% | 19.9% |
  | 既定の表、4.6 の確信度 | [32, 95, 608, 855, 290, 100, 18, 2] | 0.1% | 1.0% |
  | 下限なし、レビュー後の規則 | [32, 92, 374, 573, 395, 134, 30, 370] | 18.5% | 20.0% |
  | 既定の表、レビュー後の規則 (現行) | [32, 95, 603, 854, 294, 101, 19, 2] | 0.1% | 1.05% |

  基準 (7 レベル ≤ 1%、6 レベル以上 ≤ 5%) を満たす。既定スイートでは 200 配牌版を回す。なお、途中で強制パスの穴 (gap) に落ちたリプレイは、下限ありで 115 件、下限なしで 54 件である。下限が継続ビッドを禁じた手が、SAYC の表にもナチュラル候補にも行き場を失う分である (ナチュラル暗黙パスの有無では変わらない)。

**ナチュラル暗黙パス (`NaturalInference::implicit_pass`)。** `ImplicitPass::Complement` のもとで、どのナチュラル候補も満たさない手はパスする (`07-bidding.md` §5.2 の手順 3)。

- 全候補 (`Pass` の規則が候補に入っていればそれも含む) の和の否定 ¬(C_1 ∨ … ∨ C_k) を制約とする `Pass` を返す (候補が無ければ `ANY`)。戻り値は `Option` ではない。
- 上限付きの `pass_default` (例: `1H P` の後の 0〜5) が候補にあっても省かない。ナチュラルの `Pass` の領域は `07-bidding.md` §4.1 のとおり (パスの規則 ∧ ¬上位) ∨ ¬(全候補の和) になる。例: `1H P` の後の 17 点 AK2.32.AQ32.KJ32 はどの候補も満たさないのでパスする (以前は `None` を返し、`m_P(h)` が ⊥ になっていた)。レビュー時点 (34d51a3) の規則では、`1H P` の一様な手 10 万のうち 0.21% がこの場合に当たった。レーン B の `natural_choice` (wip/p4-B) も同じく `Pass` の有無を見ない。
  - 確信度は 0、規則名は `IMPLICIT_PASS_RULE` (`"implicit_pass"`)、説明は「pass: no natural call fits this hand」。
  - この `Pass` は全候補の後に位置し、構成上どの候補とも素なので、その領域はそのまま排他領域になる。

**フェーズ 4.6 の調整。** 対象は、規則の確信度 (priority としてだけ効く) とレベル下限の表である。規則の制約 (HCP 範囲など) は変えないので、測定 1 と 3 は構成上変わらない。調整後の全測定で、測定 1 の全グループと測定 3 の全規則が、調整前と同一であることを確かめた (4.6 の調整の時点。後の「レビュー後の修正」は規則の制約も変えたので、測定 1 と 3 も動く)。

- データの分割 (`15-phase4-plan.md` D20):
  - コーパス (27 ファイル、724 ゲーム、8,169 コール) を列挙の添字で分ける。偶数が調整用、奇数が評価用。
  - 測定 2 の 600 決定点も、添字の偶奇で調整用と評価用の半分に分ける。
- 目的関数 (調整用だけで計算する) は次の 2 つの平均。
  - コーパス調整用分割での実配牌一致率: 実際の手でナチュラル方策が選ぶコールと、実際のコールが一致する率。候補を 1 つも満たさなければパスと見なす。
  - 測定 2 の文脈付き一致率 (下記) の調整用半分。
- 探索: 規則 × 旧確信度ごとの priority を 0, 5, …, 100 の範囲で座標降下する。改善が厳密なときだけ値を置き換え、同点なら今の値に近い方を採る。レベル下限は、表 5 種 (下限なし、STANDARD、STANDARD ± 2、4 レベルから) のそれぞれで調整して比べた。
- `natural_tuning` (`#[ignore]`) は次の 2 点を確かめる。
  - 予測が `choose_bid` と一致すること (旧定義の全手、既定の priority で、下限なしと STANDARD の両方)。
  - 部分的に戻したときの目的関数 (環境変数 `TUNE_FIX`)。

測定 2 の定義についての注意: フェーズ 3 の定義は、手を素の `candidates` (パートナー文脈なし) の制約から引く。一方 `choose_bid` は、パートナー文脈付きの制約 (レベル下限込み) で順位を付ける。このため下限を入れると、`choose_bid` が決して選ばない手 (例: 下限を満たさない 4♠) を引いて外れに数え、一致率が 0.331 → 0.301 に下がる。そこで「文脈付き」の定義を加えた。こちらは `choose_bid` 自身が並べる制約から手を引く (`ReproductionReport.contextual_agreement_rate`)。下限なしでは、2 つの定義は同じ制約から引く (乱数列だけが違う)。

結果 (release、loadavg 7〜9):

| 設定 | 測定 2 (旧定義) 全体 / 評価半分 | 測定 2 (文脈付き) 全体 / 評価半分 | 実配牌一致率 調整用 / 評価用 |
| --- | --- | --- | --- |
| フェーズ 3 (下限なし、旧確信度) | 0.331 / 0.334 | 0.333 / 0.335 | 0.461 / 0.478 |
| STANDARD、旧確信度 | 0.301 / 0.300 | 0.375 / 0.376 | 0.523 / 0.541 |
| STANDARD、4.6 の確信度 (採用) | 0.338 / 0.336 | 0.415 / 0.415 | 0.599 / 0.618 |
| 下限なし、4.6 の確信度 (参考) | 0.373 / 0.375 | 0.374 / 0.376 | 0.559 / 0.577 |

`natural_inference_metrics` 本体での値 (測定 2 は 600 決定点、8,326 候補): 旧定義 0.338 (下限だけを入れた状態では 0.301、フェーズ 3 は 0.329)、文脈付き 0.415。

採用した変更 (確信度、括弧内は旧値):

| 規則 | 新 (旧) | 規則 | 新 (旧) |
| --- | --- | --- | --- |
| `raise` | 0.45 (0.6) | `new_suit_resp_2` | 0.6 (0.5) |
| `rebid_nt` | 0.45 (0.5) | `rebid_new_suit` | 0.35 (0.4) |
| `negative_x` | 0.4 (0.5) | `takeout_x` | 0.45 (0.5) |
| `overcall` | 0.35 (0.5) | `jump_overcall` | 0.5 (0.4) |
| `reverse` | 0.55 (0.4) | `open_1m` | 0.55 (0.6) |
| `open_preempt` | 0.55 (0.5) | `pass_default` の `ANY` | 0.3 (0.2) |

- 最大の寄与は `pass_default` の `ANY` を 0.3 に上げたことである。これで `cue` と `penalty_x` (どちらも 0.3) より前に来る (同点では `Pass` がコール順で勝つ)。
  - 調整直後の確信度でこの 1 つだけを 0.2 に戻すと、実配牌一致率 (評価用) が 0.611 から 0.553 に落ちる。
  - 人間は、他に合うコールが無いときキュービッドやペナルティ・ダブルではなくパスする。
- 座標降下は `new_suit_resp_1` を 0.5 → 0.4 (初回) / 0.25 (再調整)、`resp_nt` を 0.5 → 0.55 にも動かした。これは 1 レベルの新スート応答より 1NT 応答を優先する順で、標準的な応答順と逆になる。調整用の目的関数では 0.5073 対 0.5066 と差が誤差程度なので、採らずに旧値 (ともに 0.5) を残した。
- レベル下限の表は STANDARD のままにした。調整用の目的関数は、STANDARD が 0.5066、STANDARD+2 が 0.5099、STANDARD−2 が 0.5026、4 レベルからが 0.5071、下限なしが 0.4655 で、下限なし以外の差は 0.004 以内である。一方 STANDARD+2 は旧定義の測定 2 を 0.011 下げる。

**レビュー後の修正: 影に隠れた規則 (2026-09-28)。** 4.6 の確信度は `raise` (0.45) を 1 レベルの `resp_nt` (0.5) の下に置いた。1 レベルの 1NT 応答はシェイプに制限が無く (6〜10)、シンプル・レイズ (3 枚以上、6〜9) の領域を丸ごと含むので、`1x P` の後のナチュラル方策は一度もレイズしなかった (レイズ規則を満たす手 2000 のうち 0)。δ > 0 (`human()`) の解釈では、ナチュラルの 2M/2m の片 `Y_c` が空になる。目的関数はこれを見分けない。同じ種類の「全域の影」を網羅的に調べ、次を直した。

- `resp_nt`: 最初の応答の 1NT はシンプル・レイズを否定する (§8.3 の表)。SAYC の書き方 (`1H-1N` は「3 枚以上のハートなし」、`1C-1N` は「5 枚以上のクラブなし」) に合わせた。1 レベルで言える 4 枚メジャーは否定しない。`new_suit_resp_1` と同点でコール順で勝つので、排他領域からは既に抜けている。生の制約に入れると測定 1 の再現率を大きく下げた (試した版で SAYC の `Responder`/`Bid_NT` 0.316 → 0.208) ので外した。
- `negative_x` と `new_suit_resp_1`: `1C (1H)` では 1S が 4 枚以上を示してダブル (0.4) を丸ごと覆っていた。SAYC どおり、メジャーのオーバーコールの後の 1 レベルの新メジャーは 5 枚以上、ダブルはちょうど 4 枚にした。`1C (1D)` のように両メジャーが 1 レベルで言えるときは、ダブルは両メジャー 4 枚以上で、確信度 0.5 (1H/1S と同点になり、コール順でダブルが勝つ)。2 レベルのオーバーコールの後は従来どおり、どちらかの未ビッドメジャー 4 枚以上。
- `open_pass`: 0.5 → 0.45。ウィーク・ツー (0.5、5〜10) は `open_pass` (0〜11) に含まれ、同点ではコール順でパスが勝つので、ナチュラル方策は一度もウィーク・ツーを開かなかった (フェーズ 3 からの問題)。
- `rebid_own`: ジャンプ・リビッドを 0.55 にした (§8.3 の表。フェーズ 3 からの問題)。
- `implicit_pass`: `Pass` の規則が候補にあっても省かない (上の「ナチュラル暗黙パス」)。

回帰テストは `tests/natural_shadow.rs` である。

- 26 の代表局面で、どの規則も全域が影にならないこと。例外は `cue` と `penalty_x` だけで、上限なしのパス (0.3) の後ろに意図して置いている。
- 17 局面の 106 の代表的なコール (オープニング、各応答、ネガティブ・ダブル、オーバーコール、アドバンス、オープナーのリビッド) が、ある手で選ばれること。
- 判定は、サンプラーが除去なしで数えられるときは「C_i ∧ ¬(上位の和)」の手数が正であることで厳密に行う。それ以外は C_i から 4000 手を引いて調べる。
- レビューで示された手 (`1H P` で 7.KJ84.QT763.982 は 2H、`1S P` で Q84.K73.J9763.82 は 2S) の確認も入れた。
- 残る影は、より高いレベルが同じ制約を繰り返すものだけである。5〜7 レベルのレイズは 4 レベルと同じ制約で、アドバンサーの 3H/4H は `advance.raise` を共有し、オープナーの 4 レベルのジャンプシフトは 3 レベルと同じ制約である。規則表の既知の限界として、代表コールには入れていない。

測定 (release、loadavg 18〜100。機械を他のレーンと共有):

| 設定 | 測定 2 (旧定義) 全体 / 調整半分 / 評価半分 | 測定 2 (文脈付き) 全体 / 調整半分 / 評価半分 | 実配牌一致率 調整用 / 評価用 |
| --- | --- | --- | --- |
| 4.6 の採用値 (修正前) | 0.3382 / 0.3400 / 0.3362 | 0.4147 / 0.4145 / 0.4149 | 0.5988 / 0.6180 |
| 修正後 (現行) | 0.3506 / 0.3535 / 0.3477 | 0.4294 / 0.4302 / 0.4285 | 0.5978 / 0.6175 |

- `natural_inference_metrics` 本体では、測定 2 は旧定義 0.338 → 0.350、文脈付き 0.415 → 0.429 (600 決定点、8,326 候補)。
- 規則の制約を変えたので、測定 1 と 3 も動く。
  - 測定 1 (全ノード平均) は、SAYC の再現率 0.6500 → 0.6481、精度 0.6422 → 0.6438 (1500 ノード)。vendor の再現率 0.5829 → 0.5803、精度 0.5961 → 0.5965 (795 ノード)。
  - グループ別で 0.01 を超えて動いたもの:
    - `Responder`/`Bid_NT`: 再現率が SAYC で 0.316 → 0.293、vendor で 0.284 → 0.232。精度は 0.525 → 0.529 / 0.637 → 0.644。
    - `Responder`/`Double(Negative)`: 再現率が SAYC で 0.593 → 0.558 (精度 0.740 → 0.796)、vendor (6 ノード) で 0.468 → 0.333。
  - 測定 3 (コーパス 8,169 コールの充足率) は 0.7991 → 0.7981。規則別では `negative_x` 0.649 → 0.544 (57 コール)、`resp_nt` 0.313 → 0.303 (198 コール)。
- `open_pass` だけを 0.5 に戻すと、実配牌一致率は 0.6005 / 0.6197 (+0.0027 / +0.0022) で、測定 2 は変わらない。コーパスの実際の手では、ウィーク・ツーの形でもパスするほうが多い。それでもナチュラル方策がウィーク・ツーを開かないのは SAYC の補完として誤りなので、0.45 を採った。
- 修正後の規則で座標降下をやり直すと、目的関数は 0.5140 → 0.5156 (+0.0016) で、動かすのは次の 3 つである。どれも採らなかった。
  - `new_suit_resp_1` を 0.5 → 0.4 にする。これは、1NT 応答を 1 レベルの新スートより前に置く、既に退けた順である。
  - `negative_x` を 0.4 → 0.25 にする。
  - `open_weak2` を 0.5 → 0.45 にする。`open_pass` と同点になり、ウィーク・ツーが再び影になる。
  - 3 つを入れても、評価用の実配牌一致率は 0.6175 → 0.6140 に下がる。

受け入れ基準 (レーン S「ナチュラル」)。数値はレビュー後の修正を含む現行の値である。

- (a) 3 測定のどれも 0.01 を超えて下がらないこと。満たす。
  - 測定 1 と 3 の全体の値は、上のとおり 0.003 以内でしか下がらない。グループ別・規則別には、上に挙げた下がり方がある。
  - 測定 2 は、旧定義で 0.329 → 0.350、文脈付きで 0.429 に上がった。
- (b) 決定点の一致率が 0.329 から 0.02 以上上がること。
  - 旧定義では 0.350 (+0.021) で満たす。調整用・評価用の半分では、フェーズ 3 の 0.331 / 0.334 に対して 0.3506 / 0.3477 (+0.020 / +0.014)。
  - 文脈付きの定義では、同じ定義のフェーズ 3 の値 0.333 / 0.335 (全体 / 評価半分) に対して 0.429 / 0.4285 (+0.096 / +0.094) で満たす。4.6 の採用時点の「+0.086」は、文脈付きの値を旧定義の基準 0.329 と比べていたので、定義をそろえると +0.082 (全体) / +0.080 (評価半分) だった。
  - 旧定義は、下限が除外する手を引いて外れに数えるので、下限だけで −0.030 になる (上の注意)。レベル下限はリプレイの暴走を止めるために必要なので、下限を入れたまま評価する。
- 実配牌一致率 (評価用分割) は、フェーズ 3 の 0.478 から 0.6175 に上がった (4.6 の採用時点では 0.618)。

---

## 9. Lint (`lint.rs`)

### 9.1 型

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Severity { Info, Warning, Error }         // 昇順

pub struct Lint {
    pub severity: Severity,
    pub code: LintCode,
    pub row: Option<RowId>,
    pub node: Option<NodeId>,
    pub span: Option<Span>,
    pub message: String,
}
impl Display for Lint { /* `file:line: severity[Code]: message` */ }

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LintSummary { pub errors: usize, pub warnings: usize, pub infos: usize }
impl LintSummary { pub fn of(lints: &[Lint]) -> LintSummary; }
```

### 9.2 コード (段階別)

| 段階 | コード | 既定の severity | 意味 |
| --- | --- | --- | --- |
| parse | `IncludeNotFound` | Warning | `#INCLUDE` 先が無い |
| parse | `IncludeCycle` | Error | include の循環 (深さ ≤ 16 の超過も) |
| parse | `UnknownDirective` | Warning | 不明な `#DIRECTIVE`、メタ値のパース失敗 |
| parse | `PasteUnknownName` | Warning | `#PASTE` の未定義クリップボード名 |
| parse | `UnknownCallToken` | Warning | コールトークンが読めない (行と部分木をスキップ) |
| parse | `SequenceNotFirst` | Error | 先頭以外の行に `-`/`;` |
| parse | `IndentationMismatch` | Warning | 祖先に一致しないインデント |
| parse | `NonStandardToken` | Info (誤った `{w:X}` は Warning) | 拡張トークン・注釈の使用 |
| parse | `ColumnZeroContinuation` | Info | 末尾記号なし履歴行の後の列 0 行を履歴の子として扱った (§1.4 の 2) |
| expansion | `IllegalCall` | Error (下記の全展開失敗時のみ。それ以外は Info に降格) | 展開結果が `Auction::is_legal` に反する (部分木を捨てる) |
| expansion | `UnboundOther` | Error | `oM`/`om` の相方が未束縛 (行をスキップ) |
| expansion | `VariableNoCandidate` | Info | 変数の候補が空 |
| expansion | `StepWithoutAnchor` | Error | `step` の基準ビッドが無い |
| expansion | `WideWildcard` | Warning | `Level::Any` や `Class` で候補が 8 を超える |
| expansion | `DuplicatePath` | Warning (説明が非空で異なる) / Info (空を埋めた) | 同一条件・同一経路の再定義 |
| expansion | `ShadowedByExact` | Info | パターン行の候補が Exact 行と衝突して捨てられた |
| expansion | `ConditionTie` | Info | 同一経路で同じ特定度の条件が両方一致しうる (先定義が勝つ) |
| expansion | `TooManyNodes` | Error | ノード数が `CompileOptions.max_nodes` (50,000) を超え、展開を打ち切った |
| constraint | `UnsatisfiableConstraint` | Error | `constraint.is_satisfiable() == false` (ノードは残し、L3 が飛ばす) |
| constraint | `ContradictsOwnHistory` | Warning | 自分の祖先ノードの制約との `And` が充足不能 |
| constraint | `LowRecognition` | Warning (`constraint_bearing == false` なら Info) | `ratio < meta.recognition_threshold` |
| constraint | `EmptyDescription` | Info | 説明が空 |
| constraint | `UnrecognizedFragment` | Info | `Unrecognized` 断片がある (先頭 3 件をメッセージに) |
| constraint | `SoftConstraint` | Info | hedge 付き断片がある |
| constraint | `AssumedContext` | Info | Pass 2 が仮定で解決した |
| constraint | `SiblingSubset` | Warning (`priority` が異なれば Info) | 後の兄弟の制約が先の兄弟の制約の部分集合 (到達不能) |
| constraint | `SiblingOverlap` | Info | 兄弟の制約が重なる |
| constraint | `DnfTruncated` | Warning (`strict_dnf` なら Error) | `to_dnf` が `max_terms` (256) を超えて `residual` に退避した |
| coverage | `MissingOpeningCoverage` | Warning | どのオープニングも満たさない (パス以外の) 手の割合 |
| coverage | `MissingResponseCoverage` | Info | あるノードの子のどれも満たさない手の割合 |
| exclusive | `ShadowedBranch` | Warning | 上位の兄弟に全域を覆われ、`choose_bid` が決して選ばない我々側の枝 (§9.3 の 8) |
| exclusive | `OverlappingBranches` | Info | 同じノードの枝どうしが重なり、排他索引が後の枝を素化した (§9.3 の 8) |
| expansion | `LevelWithoutAnchor` | Error | 相対レベル (`cS`、`jY`) の基準となる最後のビッドがワイルドカードのため不明 (その位置で行と部分木を捨てる。§4.6) |
| expansion | `NoSufficientLevel` | Info | 相対レベルが 7 を超え、候補が無い (§4.6) |
| parse | `AnyOrderWithoutVariables` | Info | `#ANYORDER` の表が `X`/`Y`/`Z` のうち 2 つ以上を使っていない (順序が無いので効果が無い。§4.7) |
| stop | `StopUnderForcing` | Warning | パートナーのフォーシングのコールに相手がパスした後、またはゲームフォース中でゲーム未満の位置で、停止のパス (`{prio:-100} {stop} any hand`、書いたものか合成) が候補になる (§4.5、§9.3 の 9) |

### 9.3 コンパイル後の九つの検査

parse / expansion の Lint は各段階が発生時に出す。`lint.rs` は完成した IR に対して次の 9 検査を順に走らせる (`bridge-constraint` の DNF / 交差が要る)。

1. **充足可能性**: 全ノードで `constraint.is_satisfiable()` (DNF に非空の `Atom` が無い)。偽なら `UnsatisfiableConstraint` (Error)。ノードは残してフラグを付け、L3 は `Diagnostic::UnsatisfiableNode` として飛ばす。
2. **自分の履歴との整合**: ノードの `side` と同じ側の祖先ノード (`path` 上) の制約を `And` して `is_satisfiable()`。偽なら `ContradictsOwnHistory` (Warning。例: `1N 15-17` の後のリビッドが `18+` を示す)。
3. **不正コール**: 展開中の `Auction::is_legal` 違反は `IllegalCall` で部分木を捨てる (§4.2)。ここでは残っていないことを確認するだけ。変数束縛された祖先 (履歴トークンや `Var`/`Strains` パターン) の下にある exact 行は束縛ごとに 1 回ずつ展開されるので、同じ行が「ある束縛では違法、別の束縛では合法」になりうる (jdh8/blue/1C.bml の `1C-(1X)-` 下の `1H = F, 4=!h` は `X = H/S` でのみ違法)。全展開が完了した後、その行がどれか 1 つの束縛で成功していれば当該 `IllegalCall` は Info に降格し (`bss.py` が黙って落とすのと同じ扱い)、どの束縛でも一度も成功しなかった行だけが Error のまま残る (`compile::expand::demote_illegal_call_for_bindings_that_succeeded`)。
4. **重複と条件の同点**: 同じトライノードに同一条件で説明の異なる非空エントリが 2 つ → `DuplicatePath` (Warning、先勝ち)。照合時に同じ特定度で両方一致しうる条件 → `ConditionTie` (Info)。
5. **認識率**: §7.7 の閾値判定で `LowRecognition` (非空かつ `ratio < threshold`。`constraint_bearing == false` の純コンベンション行は Info)、付随して `EmptyDescription` / `UnrecognizedFragment` / `SoftConstraint` / `AssumedContext` / `DnfTruncated`。
6. **兄弟の曖昧さ** (同じ親、同じ側、同じ条件): DNF の Atom 上の記号的検査。`A ⊆ B` は A の各 Atom が B のいずれかの Atom に含まれること (`hcp` / `shapes` / `suit_len` は区間・ビット集合の包含、`cards` / `eval` は集合比較)。先の兄弟に含まれる後の兄弟は `SiblingSubset` (Warning。`priority` が異なれば Info)。Atom 対の交差が非空なら `SiblingOverlap` (Info。ナチュラル系では非常に多いので Info のみ)。DNF の項数上限 256 を超える場合は検査を省略し Info を出す。
7. **カバレッジ** (任意、`CompileOptions.coverage_samples` (既定 10,000、0 で無効)): 手を一様に引き、各 `SeatCond` (`opener_pos` 1..=4) について `hcp ≥ opening_min` なのに `Pass` 以外のどのオープニングノードも満たさない手の割合を `MissingOpeningCoverage` (Warning) に添える。同様に子を持つ各ノードについて、親文脈から (一様に) レスポンダーの手を引き、どの子も満たさない割合を `MissingResponseCoverage` (Info) に添える (L3 の `NoCandidate` 集計のコンパイル時版)。サンプル数はノード数に応じて `min(coverage_samples, 10^6 / nodes)` に落とす。閾値は設けず割合を報告するだけで、判断は `coverage_report.json` (`11-testing.md` §2) と合わせて行う。

8. **排他領域** (フェーズ 4、`check_exclusive_branches`): §5.4 の索引を読む。(a) ある (ノード, 枝) の片がどのグループにも無く、そのノードを含む全グループで「枝 − 上位」がグリッドで空と証明できる (`grid_proves_empty`) とき `ShadowedBranch` (Warning)。枝単独で空のもの (検査 1 の対象) と、相手側のノード (`Side::Them`) は除く。相手側のコールは我々の方策の選択ではなく木の辺にすぎず、多くは要件の無い表見出し (`1C-(1H)-` など) なので、同じ位置の他のコールより下位ならすべて覆われて見えてしまう。メッセージは「never chosen: higher-ranked siblings cover it」(複数枝なら「branch j/n never chosen: …」)。(b) 同じノードの枝 j と k の sup グリッドが実行可能なセルで交わるとき `OverlappingBranches` (Info)。行ごとに (j, k) 1 件にまとめる。SAYC では `ShadowedBranch` 18 件 (すべて我々側。相手側を数えていた時点ではフェーズ 3 の SAYC で 249 件、うち相手側 231 件)、`OverlappingBranches` 268 件 (フェーズ 3 の SAYC では 90 件、行ごとにまとめる前は 186 件)、Error 0 件 (フェーズ 4 統合時、P1〜P10 の SAYC)。

9. **フォーシングの下の停止** (レーン D2 のレビュー、`check_stop_under_forcing`): 両方の根からトライをたどり、(a) 我々の側の最後のコールがフォーシング (`Forcing::OneRound`/`ToGame`) で相手がその後パスした、(b) 我々の側がゲームフォース (`ToGame`) のコールをし、その後にゲーム以上のビッド (3NT、4H/4S、5C/5D 以上) が無い、のどちらかが成り立つ我々の手番で、停止のパス (`Pass`、`flags.stop`、優先度 -100 以下) が、排他索引 (§5.4) のあるクラスの組 (`group_for`) で上位の行に覆われずに (`is_shadowed(Pass)` でない) 候補になるなら `StopUnderForcing` (Warning) を位置ごとに 1 件出す。どの行にも当たらない手はフォーシングのコールをパスすることになるからである。メッセージは位置 (BML の記法、相手のコールは括弧、ワイルドカードは `(any)`) とフォーシングのコールを示し、行はフォーシングのコールの行を指す。相手のビッドは 1 巡のフォーシングを解く。相手の手番では、パスが通る辺 (完全一致の `Pass`、無ければパスを含む最初のワイルドカード) をパスとして続け、他のワイルドカードは水準の分からないコールとして両方を解く。停止のパスの先 (停止の輪) へは進まない。試験 `tests/stop.rs` の `a_stop_under_partners_forcing_call_is_reported`、`a_stop_below_game_after_a_game_force_is_reported`。SAYC ではレーン D2 のレビュー修正の後 27 件で、すべてフェーズ 4 のレーン D が `continuations.bml` に置いた開始者の再ビッドの `{stop}` 受け (1 段の応答、2/1、1M-2NT、1S-2C-2H-3D/3S、1H-1S-2C-2D、相手の割り込み後の自由なビッド) である。これらの表はまだすべての手を覆っていないので、受けを外すと暗黙のパスになって監査から消えるだけであり、既知の警告として残す (`systems/sayc/NOTES.md` #P12)。その集合は `crates/bridge-system/tests/sayc.rs` の `sayc_stops_under_forcing_calls_are_only_the_known_rebid_sinks` が固定する (新しい位置が増えても、直った位置が残っても失敗する)。

検査 6 と 7 は `Sampler::prepare` (20〜60 μs) を使うので、コンパイル 1 秒の予算を圧迫する場合は `coverage_samples = 0` で 7 を無効化できる (`load_or_compile` のキャッシュがあれば実質 1 回だけ)。

### 9.4 出力形式

```rust
// compile/mod.rs
pub struct CompileOptions {
    pub coverage_samples: u32,   // 10_000。0 で検査 7 を無効化
    pub strict_dnf: bool,        // false。true なら DnfTruncated を Error にし、Overflow::Error で失敗させる
    pub max_nodes: usize,        // 50_000。超えたら TooManyNodes で展開を打ち切る
}
/// Never fails as a whole: problems are returned as lints (and also stored in `SystemIR::lints`).
pub fn compile(root_path: &str, source: &str, loader: &dyn SourceLoader, opts: &CompileOptions) -> (SystemIR, Vec<Lint>);
```

- テキスト形式 (`impl Display for Lint`): `{file}:{line}: {severity}[{code}]: {message}`。`(file, line, code)` でソートする。
- JSON 形式: `serde` 派生で `Vec<Lint>` をそのまま出す (`xtask` の認識率レポートが使う)。
- 集計行: `LintSummary::of(&lints)` の `errors` / `warnings` / `infos` を §7.8 の `INFO` トレースと同じフィールド (`lints_error` 等) で出す。
- `compile()` は Error 級の Lint があっても IR を返す (`Result` にしない)。Error があれば失敗にしたい CI は `LintSummary::of(&lints).errors > 0` で判定する。認識率の閾値は `#+RECOGNITION:` (`SystemMeta.recognition_threshold`) だけで決まり、`CompileOptions` には無い。

---

## 10. 直列化、キャッシュ、バージョニング、`systems/`

### 10.1 直列化

- `SystemIR` と配下の型 (`Row`, `Node`, `AuctionTrie`, `SystemMeta`, `Lint`, `Recognition`, `NaturalParams`, `SeatCond`, `VulCond`, パターン型 …) は `#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]`。`bridge-constraint` 側の `HandConstraint` / `Atom` / `CardRequirement` / `EvalRequirement` と `bridge-core` の `ShapeSet` / `ShapeClass` にも同じ feature の派生が要る。`HandConstraint` は `05-constraint.md` の手動 serde を使い、`Custom` は直列化不能 (シリアライズ時にエラー)。
- **コンパイラは `HandConstraint::Custom` を決して生成しない** (D9、リスク R11)。`Node` 生成時の `debug_assert!(node.constraint.is_samplable())` と、`systems/` の全ファイル (自作 + fixtures + 取得済み vendor) を対象に `postcard::to_allocvec` が成功することを確認するテストで保証する。将来のトークンが `Custom` を要するなら Lint で拒否する。
- `Span` の `pasted_from: Option<(Arc<str>, u32)>` は文字列として直列化する。

### 10.2 キャッシュ (`cache.rs`、feature `cache = ["std", "serde", "dep:postcard", "dep:blake3"]`、D9)

```rust
// lib.rs
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const IR_FORMAT: u32 = 3;   // 1: phase 3; 2: NodeFlags::stop and the system-stop trie links (§4.5); 3: NodeFlags::synthesised

// cache.rs
pub struct SystemCache { dir: PathBuf }
impl SystemCache {
    pub fn new(dir: impl Into<PathBuf>) -> SystemCache;                       // dir は初回書き込み時に作る
    /// blake3(resolved source ‖ compiler_version ‖ ir_format ‖ compile_revision ‖ options)
    pub fn key(source: &[u8], opts: &CompileOptions) -> [u8; 32];
    /// Loads the cached IR for `path` if its key matches, otherwise compiles and stores it. Lints are stored with the IR.
    pub fn load_or_compile(&self, path: &Path, loader: &dyn SourceLoader, opts: &CompileOptions) -> std::io::Result<(SystemIR, Vec<Lint>)>;
}
```

`compile_revision` は `COMPILE_REVISION` (フェーズ 3 が 1、フェーズ 4 が 2、システム停止 §4.5 が 3、条件の違う停止が共有する輪が 4、合成された停止のパスの説明文が 5、相対レベル §4.6 とスート長の比較 §7.4 が 6、`#ANYORDER` §4.7 が 7、レーン D2 のレビュー修正 (指示子だけの段落・相対レベルで始まる行・代替の相対レベルの Lint、`LevelWithoutAnchor` の重複除去、`!h>=!s+1` を比較と読まない、`StopUnderForcing`) が 8、ノードの説明文から注釈を除いて格納するのが 9 (統合線ではレーン D2 のマージ前に 6 だった)) である。`compile()` の出力が形式を変えずに変わるとき (新しい Lint など) に上げる。クレートのバージョンと `IR_FORMAT` が同じでも、古いコンパイラが書いたエントリは別のキーになり、読まれずに再コンパイルされる (フェーズ 4 の排他索引の Lint を持たない IR が、温まったキャッシュから返るのを防ぐ。回帰テスト `an_entry_under_the_pre_revision_key_is_a_miss`)。

手順: (1) `loader` で `path` を読み、`lexer::load` で include を解決して `resolved source` (全ファイルの連結、`Loaded.files` の順) を得る。(2) `key` を計算し `dir/<hex(key)>.ir` を探す。(3) あれば `postcard` でデコードする。ヘッダの `ir_format` が `IR_FORMAT` と違う、`compiler_version` が違う、デコードに失敗する、のいずれも「不一致」として再コンパイルし上書きする (エラーにはしない)。(4) 無ければ `compile` して書く。書き込みは一時ファイル + rename で原子的に行い、I/O の失敗だけが `Err`。`std` 無し (wasm) では `SystemCache` を提供せず、`compile` だけを使う。

`SystemMeta.source_hash` はキーのソース部分なので、別の場所で読み込まれた IR からでも元の BML を辿れる。配布形式は BML ソース (D9)。コンパイルの目標は最大ファイルで 1 秒未満なので毎回コンパイルしても足り、キャッシュは任意 (最適化であって配布形式ではない)。

### 10.3 バージョニング

- `SystemMeta.version` (`#+VERSION:`) はシステム定義の著者が管理する文字列。`SystemMeta.source_hash` は解決済みソースの blake3 で、キャッシュキーの一部でもある。
- `SystemMeta.compiler_version` は `COMPILER_VERSION`、`SystemMeta.ir_format` は `IR_FORMAT`。IR の破壊的変更 (フィールド追加を含む) で `IR_FORMAT` を上げる。デシリアライズは別の `ir_format` を拒否する。
- L3 の `Table` は `[Arc<SystemIR>; 4]` を持つので、席ごとに異なる `version` のシステムを同時に使える。互換性検査は `ir_format` のみ (システム定義の意味的な互換性は検査しない)。

### 10.4 `systems/` の配置

ワークスペース直下 (`bridge_engine/systems/`)、どのクレートの外にも置く。

```
systems/
├── README.md                  # 書き方、拡張 (#+KEY:, {prio:N}, {w:X}, !, #) の説明、ライセンス方針 (骨格でコミット済み)
├── sayc.bml                   # 自作 SAYC (フェーズ 3: オープニング → フェーズ 4: レスポンス・リビッド・競り合い)
├── two-over-one.bml           # 任意 (フェーズ 4 以降)
├── fixtures/                  # 小さな検証用ファイル (コミットする)
│   ├── variables.bml          #   M/m/oM/om/X/Y/Z/red/black/step の束縛
│   ├── paste.bml              #   #CUT/#COPY/#PASTE と置換
│   ├── seat_vul.bml           #   #SEAT/#VUL の条件と特定度
│   ├── competitive.bml        #   相手のコール、暗黙パス、(1N)-P-(P)---
│   ├── extensions.bml         #   2S/3H, 4D/H, nX, X/XX, {prio:N}, {w:X}, #+KEY:
│   └── lint_*.bml             #   各 Lint コードを 1 つずつ再現
└── vendor/
    ├── manifest.toml          # URL + sha256 + 取得先パス (gpaulissen/bml test/data と test/expected、選抜した実ファイル ~40 本)
    └── data/                  # `cargo xtask systems fetch` の出力。.gitignore 済み
```

外部ファイルはライセンス未確認のため **取得のみでコミットしない** (リスク R10)。`manifest.toml` の各エントリは `[[file]] name, url, sha256, dest, kind = "bml" | "bss"` を持ち、`xtask` が sha256 を検証する。統合テストは `BRIDGE_SYSTEMS_DIR` (既定はクレートのマニフェストからの相対 `../../systems`) にファイルが無ければスキップする (失敗にしない)。

---

## 11. モジュール構成と実装順

### 11.1 モジュール

| モジュール | 内容 |
| --- | --- |
| `lib.rs` | 再エクスポート、`COMPILER_VERSION`、`IR_FORMAT`、crate doc (パイプライン図) |
| `ast.rs` | §2.2 の AST、`FileId`、`Span`、`RawLine`、`SeatCond`/`VulCond`/`Tri` |
| `lexer.rs` | `SourceLoader`、`FsLoader`、`MemLoader`、`Loaded`、`load` (`#INCLUDE`)、`paragraphs`、`ROOT` |
| `parser/mod.rs` | `parse(Loaded) -> BmlFile`、`ParagraphKind`、`classify`、木構築、回復規則 |
| `parser/call.rs` | winnow のコールトークン文法 (`calltok`, `history`) |
| `parser/clipboard.rs` | `Clipboard`、`expand` (`#CUT/#COPY/#PASTE`) |
| `pattern.rs` | `CallPattern`, `Side`, `Var`, `StrainSet`, `Level`, `OppClass`, `SidedPattern`, `Binding` (候補生成) |
| `ir.rs` | `SystemIR`, `Row`, `Node`, `NodeFlags`, `Alertability`, `Forcing`, `Recognition`, `SystemMeta`, `StrengthVocab`, `TieBreak`, `BalancedDef`, `ConventionDefaults` |
| `trie.rs` | `AuctionTrie`, `TrieId`, `Lookup`, `LookupKey` (`for_auction`), `RelVul`, `Resolution` |
| `compile/mod.rs` | `CompileOptions`、`compile`、展開 DFS、暗黙パス、トライ挿入 |
| `compile/desc/mod.rs` | `Compiled`、`compile_description` (組立) |
| `compile/desc/normalize.rs` | `Normalized`、`normalize` (正規化と注釈抽出) |
| `compile/desc/clause.rs` | `Fragment`、`FragmentKind`、`Clause`、`parse`、`fragment` (節文法、winnow) |
| `compile/desc/tokens.rs` | `SuitRef`、`Token`、`StrengthWord`、`QualityWord`、`recognize` (Pass 1 の語彙) |
| `compile/desc/context.rs` | `Source`、`Provenance`、`RowContext`、`resolve` (Pass 2) |
| `compile/desc/recognition.rs` | `compute`、`STOPWORDS` |
| `natural.rs` | `NaturalParams` (+ `ResponseParams`, `RebidParams`, `AdvanceParams`, `LevelFloor`), `Role`, `DoubleKind`, `CallKind`, `CallContext`, `classify`, `Inference`, `NaturalInference`, `PartnerContext`, `NaturalCandidate` (§8.6) |
| `exclusive.rs` | `ExclusiveIndex`, `ExclusiveGroup`, `ExclusivePiece`, `ExclusiveStats`, `rank_cmp`, `natural_rank_cmp`, `subtract`, `grid_proves_empty` (§5.4、フェーズ 4) |
| `lint.rs` | `Severity`, `LintCode`, `Lint`, `LintSummary`、八つの検査 |
| `cache.rs` | `SystemCache` (feature `cache`) |

feature: `default = ["std"]`, `std`, `serde = ["dep:serde", ..]`, `cache = ["std", "serde", "dep:postcard", "dep:blake3"]`。依存: `bridge-core`, `bridge-eval`, `bridge-constraint`, `winnow 1.0`, `smallvec`, `thiserror 2`, `tracing 0.1`, `serde` (optional), `postcard` / `blake3` (optional、`cache`)。dev: `insta`, `rand_xoshiro` (`tests/natural_shadow.rs` のサンプリング)。`lib.rs` の公開面: 定数 `COMPILER_VERSION` / `IR_FORMAT` / `COMPILE_REVISION` (§10.2)、`compile::{CompileOptions, compile}`、`ir::*`、`lint::{Lint, LintCode, Severity}`、`natural::{CallContext, CallKind, Inference, NaturalInference, NaturalParams, Role}`、`pattern::*`、`trie::{AuctionTrie, Lookup, LookupKey, RelVul, Resolution}` に加え、`ast`, `cache`, `compile`, `lexer`, `lint`, `natural`, `parser`, `pattern`, `trie` は `pub mod`。

### 11.2 実装順と完了条件

| 順 | タスク | 完了条件 (数値) |
| --- | --- | --- |
| 1 | パーサ (`ast`, `lexer`, `parser/*`) | `example{1..6}.bml`、`test.bml`、取得済みの実ファイル約 40 本を Error 級 Lint 0 でパース。AST を `insta` スナップショット化 (`.snap` をコミット) |
| 2 | 展開 + トライ (`pattern`, `ir`, `trie`, `compile/mod`) | `.bss` 期待出力のある各 `.bml` について、生成した (席, vul, 具体系列, 置換済み説明) の集合が `.bss` の行と §1.4 の意図的差分を除いて 100% 一致。`resolve` のベンチ < 1 μs (深さ 10)。実ファイルの展開ノード数が行数の 3 倍以内 |
| 3 | 説明文コンパイラ v1 (`compile/desc/*`、トークン + Pass 2) | 実ファイルの認識率レポート: ファイル単位のマイクロ平均が jdh8 (簡潔な WBF 略記) ≥ 0.65、gpaulissen ≥ 0.5。未認識スパンを全て列挙。仕様 §5 の 5 例 (`3+!c, 12+ hcp` 等) がスナップショット一致。最大ファイルのコンパイル < 1 秒 (`criterion`) |
| 4 | Lint (`lint.rs`。`bridge-constraint` の DNF / 交差が要る) | `fixtures/lint_*.bml` で全コードが期待どおりの severity・行で 1 件ずつ出る。実ファイルで `UnsatisfiableConstraint` 0 件 (あれば説明文コンパイラのバグとして修正) |
| 5 | ナチュラル推定 + 3 測定 (`natural.rs`) | `classify` の単体テスト (役割・種別ごとに 1 局面以上)。3 測定が JSON を出力する (閾値は設けない。数値が出ることが完了条件で、値を見てフェーズ 4 で `NaturalParams` を調整する) |
| 6 | `serde` / キャッシュ + `sayc.bml` (`cache.rs`, `systems/`) | `load_or_compile` の 2 回目が 1 回目の 1/10 以下の時間。`sayc.bml` (オープニング) が Error 0、認識率 ≥ 0.9、L3 の双方向整合性 10^6 配牌で違反 0 (フェーズ 3.10) |

順 1〜2 はフェーズ 3.1〜3.2、3〜4 は 3.3〜3.4、5 は 3.11、6 は 3.5 と 3.12 に対応する (`12-roadmap.md`)。`bridge-constraint` に早めに要求するもの: `Atom::intersect`、`Atom::is_trivially_unsat`、Atom の包含判定 (検査 6 用)、`ShapeSet` の定数と演算 (`BALANCED`、`from_classes`、`filter`、補集合)、`CardRequirement::in_suit`、`EvalRequirement` (`TotalPoints`, `SuitQuality`, `Controls`, `Losers`)、`HandConstraint::hcp_range()`、optional `serde` 派生、安価な体積推定。

---

## 12. リスク

| # | リスク | 対策 |
| --- | --- | --- |
| R1 | 公開 SAYC / 2/1 の BML が無く、フェーズ 3 は自作に依存する | フェーズ 3 で `sayc.bml` を自作。早期テストには最も構造化された実ファイル Polish Club (jdh8) を使う |
| R2 | 認識率の天井: 散文的なファイル (gpaulissen の `2D.bml`) は 20〜40% に留まり、制約が緩くサンプリング精度が落ちる | Lint と `NAT`/ナチュラル既定へのフォールバック。黙って捏造しない。未認識断片の頻度から v2 語彙を決める |
| R3 | 文脈の連鎖: `GF`/`INV`/`MIN`/`MAX` はパートナー/自分の以前の範囲に依存し、未認識の親が `assumed` 範囲を子孫に伝播する | `Provenance.assumed` と `AssumedContext` Lint で追跡。親が `Partial` の場合は既定範囲を使う |
| R4 | 説明文中の変数置換 (`\bM\b`) が英文に当たる (`M` 単独は稀。`X` は jdh8 の散文段落 "X remains penalizing" に現れるが、段落はコンパイルしない) | 置換は表の行の説明文だけに適用し、単語境界で照合する |
| R5 | 深い防御系の表 (`(1X)-1Y-…`) で即時展開が膨らむ | 有界だが計測が必要。`CompileOptions.max_nodes` (50,000) の天井と `TooManyNodes` Lint、`WideWildcard` で候補 > 8 を警告 |
| R6 | 席の意味論 (オープナーの位置) を L3 が `LookupKey` を作るときに同一に適用しないとずれる | `LookupKey::for_auction` を L2 に置き L3 はそれだけを使う。パスアウトと 4 席目オープニングの明示的なテスト |
| R7 | 説明付きの `Them` 側の行 (相手のコールに対する我々の仮定) と、`Table` の相手自身の `SystemIR` のどちらを重んじるか | 重み付けは L3 の決定。L2 は `side` を保存するだけ |
| R8 | アラート規定は管轄で異なる | `Alertability` は参考情報で、推定した `!` 慣例から部分的に導かれるに過ぎない |
| R9 | `winnow` の API 変更 (0.7 で `PResult` → `ModalResult`) | 版を 1.0 に固定 |
| R10 | 外部 BML のライセンス (jdh8 には LICENSE がある。gpaulissen/bridge-systems は未確認) | 取得のみ、コミットしない。`manifest.toml` で sha256 固定 |
| R11 | `HandConstraint::Custom` を生成するとキャッシュ直列化が実行時に失敗する | コンパイラは決して生成しない (debug assert + 全ファイルの直列化テスト `no_custom.rs`) |
| R12 | ナチュラル推定の測定 3 はフェーズ 1 の PBN コーパス (記録されたオークション付き) が要る | それまでは測定 1〜2 だけを回す。測定 1 は自作 `sayc.bml` に対して循環するので外部ファイルを優先し、`sayc.bml` の結果は別枠で報告 |
| R13 | BML の「最初の定義が勝つ」は `#INCLUDE` の順序に依存し、著者が意図しない上書きが起きうる | 同じ意味論を再現し、`DuplicatePath` Lint を安全網にする |
