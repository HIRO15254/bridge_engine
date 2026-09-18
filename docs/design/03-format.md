# 03. `bridge-format`: PBN 2.1 / LIN / Deal 文字列の読み書き

本文書は `bridge-format` のデータモデル、`parse_lenient` / `parse_strict` / `write` の契約、実装する PBN 2.1 と LIN の文法、Deal 文字列の形式、テストとコーパスの取得機構を確定する。パーサは `winnow` 1.0 で書き、`parse_lenient` は決して失敗もパニックもせず読める分だけを `Warning` 付きで返し、`parse_strict` は export 形式への準拠を検査する。コーパスはリポジトリに含めず `corpus/manifest.toml` と `cargo xtask corpus fetch` で取得し、パース率 PBN ≥ 99%、LIN ≥ 95% をフェーズ 1 の完了条件とする (仕様 §11)。

## 1. 責務と範囲

| 形式 | 読み | 書き | 優先度 | 備考 |
| --- | --- | --- | --- | --- |
| PBN 2.1 | ○ (import 形式を lenient に) | ○ (export 形式) | 最高 | トーナメント記録の標準。`Note` 参照とエスケープに注意 |
| LIN (BBO) | ○ (lenient) | × (`LinBoard::to_game()` 経由で PBN として書く) | 高 | 実データに不正な断片が混じるため、壊れたファイルから読める分だけ読む |
| Deal 文字列 | ○ | ○ | 高 | `AKQ.234.AKQ.2345` (1 手) と `N:… … … …` (4 手)。テストとデバッグ用 |
| RBN | × | × | 低 | v1 対象外。未決: 需要が出た時点で v2 に追加する |

依存: `bridge-core`、`winnow` 1.0、`thiserror` 2、`serde` 1 (optional)。dev: `proptest` 1.11、`insta` 1。feature は `default = ["std"]`, `std`, `serde` (`serde` は `bridge-core/serde` を有効化)。このクレートは `#![forbid(unsafe_code)]`。

## 2. データモデル

### 2.1 ファイル・ゲーム・タグ・セクション (`pbn/model.rs`)

```rust
/// A parsed PBN file: games in file order plus the `%` escape lines.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PbnFile {
    pub games: Vec<Game>,
    pub directives: Vec<Directive>,
}

/// `%` escape line. `% PBN 2.1` and `% EXPORT` are recognised; everything else is kept verbatim.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Directive {
    Version(u8, u8),          // `% PBN <major>.<minor>`
    Export,                   // `% EXPORT`
    Other(String),            // e.g. `%BoardsPerPage 4`, kept as written (without the `%`)
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Game {
    pub tags: Vec<TagPair>,           // file order preserved; look up by name with `get`
    pub sections: Vec<Section>,       // Auction / Play / *Table sections
    pub commentary: Vec<Comment>,     // `{ … }` and `;` comments with their anchor
}

impl Game {
    pub fn get(&self, name: &str) -> Option<&TagValue>;                                    // first tag of that name (case-sensitive)
    pub fn view(&self, previous: Option<&GameView>) -> Result<GameView, ViewError>;       // `previous` resolves `#` / `##`
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TagPair { pub name: String, pub value: TagValue, pub line: u32 }   // line is 1-based

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TagValue {
    Str(String),      // ordinary value, escapes already resolved
    Inherited,        // `#`: same value as in the previous game (PBN 2.1 import)
    Default(String),  // `##text`: `text` unless the previous game overrides
}

/// Data lines that follow an `[Auction "N"]`, `[Play "W"]` or `[…Table "…"]` tag.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Section {
    pub tag: String,                  // "Auction", "Play", "OptimumResultTable", …
    pub arg: String,                  // the tag value: "N", "W", "Declarer;Denomination\2R;Result\2R"
    pub tokens: Vec<Token>,
    pub notes: Vec<(u8, String)>,     // `[Note "n:text"]` attached to this section (n in 1..=32)
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Token {
    Call(Call),          // 1C..7NT, Pass/P, X, XX (AP is expanded into Pass tokens by the lexer)
    Card(Card),          // SA, H2, … (Play sections)
    Unknown,             // `-`: unknown call / card / hand
    Terminator,          // `*`
    Continuation,        // `+` (import only)
    NoteRef(u8),         // `=n=`
    Nag(u8),             // `$n`
    Suffix(String),      // `!`, `?`, `!!`, `??`, `!?`, `?!` (import only; export converts to NAG)
    Raw(String),         // table rows and anything not recognised (a Warning is emitted for the latter)
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Comment {
    pub text: String,                 // without braces
    pub after_tag: Option<usize>,     // index of the tag pair the comment follows; None for a leading comment
    pub line: u32,
}
```

`Token` は字句をそのまま保持する。`AP` だけは例外で、レキサがオークション完了に必要な数の `Call(Pass)` に展開する (writer が末尾の 3 パスを `AP` に畳むので往復は保たれる)。コメントの位置はタグ対の添字 (`after_tag`) で表し、トークン単位のアンカーは持たない (トークン列中のコメントは直前のタグ対に付く)。

### 2.2 型付きビュー (`pbn/view.rs`)

```rust
/// The interpreted content of a game. Every field is optional because real files omit tags.
#[derive(Clone, Debug, Default)]
pub struct GameView {
    pub board: Option<u16>,                      // `Board`
    pub dealer: Option<Seat>,                    // `Dealer`
    pub vulnerable: Option<Vulnerability>,       // `Vulnerable`
    pub deal: Option<PartialDeal>,               // `Deal`; `-` hands stay None
    pub auction: Option<Auction>,                // the Auction section, validated by `Auction::push`
    pub play: Option<PlayHistory>,               // the Play section rotated into trick order (§3 item 8)
    pub contract: Option<Contract>,              // `Contract` (declarer from `Declarer`); None when "Pass"
    pub declarer: Option<Seat>,                  // `Declarer` (`^` prefix stripped)
    pub result: Option<u8>,                      // `Result`: tricks taken by declarer
    pub dd_table: Option<DdTable>,               // `OptimumResultTable`
}

/// A deal with possibly unknown hands (`-` in PBN, omitted 4th hand in LIN).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PartialDeal { pub hands: [Option<Hand>; 4] }   // indexed by Seat

impl PartialDeal {
    pub fn complete(&self) -> Option<Deal>;   // all four Some and `Deal::new` succeeds
}
```

`GameView` は所有型で、文字列タグ (`Event`, `Site`, `Date`, `Scoring`, 選手名) は保持しない。それらは `Game::get(name)` で原文を読む。`Contract "Pass"` は `contract = None` で、パスアウトかどうかは `auction.is_passed_out()` で判定する。`Declarer`/`Result` の `^` 接頭辞 (不規則な宣言者・結果) はビューでは剥がすだけで、フラグは持たない。

`DdTable` は `[[u8; 4]; 5]` (`[strain][declarer]`、core の `Strain` 順) の純粋なデータ型で、DDS なしでも `OptimumResultTable` を読めるようにする。定義は `bridge_core::DdTable` (`02-core.md` §2.10) で、`bridge-dds` とファサード `bridge::dd` が再エクスポートする。LIN の 4 手目省略は `PartialDeal` の `None` になり、残り 13 枚からの補完は `lin::parse_lenient` が `md` を読む時点で行う (`PartialDeal` 自体に補完 API は無い)。

### 2.3 入口関数・エラー・警告

```rust
// pbn/parser.rs
/// Never fails and never panics (fuzzed). Reads as much as it can and reports the rest as warnings.
pub fn parse_lenient(input: &[u8]) -> (PbnFile, Vec<Warning>);
/// Export-format conformance: the first violation becomes the error.
pub fn parse_strict(input: &str) -> Result<PbnFile, ParseError>;

// pbn/writer.rs
pub fn write(file: &PbnFile, opts: WriteOptions) -> String;
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WriteOptions { pub export: bool }   // export = PBN export format (§3 item 11); false = verbatim order, LF

// warning.rs
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Warning {
    pub game: usize,        // 0-based game (PBN) or board (LIN) index
    pub line: u32,          // 1-based
    pub kind: WarningKind,
    pub message: String,    // human-readable detail (what was skipped or repaired)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WarningKind {
    Encoding,        // not UTF-8; decoded as ISO-8859-1
    MalformedTag,    // tag pair or directive that could not be parsed, unterminated string
    UnknownTag,      // LIN: unrecognised two-letter tag (skipped)
    MalformedToken,  // unrecognised auction/play token (kept as Token::Raw), dangling LIN pair, bad LIN value
    BadDeal,         // hand size, duplicate card, missing hand
    BadAuction,      // illegal call at index n (auction truncated there)
    BadPlay,         // illegal card at index n (play truncated there)
    NoteReference,   // `=n=` out of range, `[Note]` without a section
    Truncated,       // `-` in Play cut the section, unterminated brace comment
    Other,           // duplicate tag, inheritance without a previous game, anything else
}

// lib.rs
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
#[error("line {line}: {message}")]
pub struct ParseError { pub line: u32, pub message: String }

// pbn/view.rs
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum ViewError {
    #[error("tag {tag}: {message}")] BadTag { tag: String, message: String },
    #[error("auction: {0}")] Auction(#[from] AuctionError),
    #[error("play: {0}")] Play(#[from] PlayError),
    #[error("tag {0} inherits from a previous game but there is none")] NothingToInherit(String),
}
```

`view()` はタグ値の解釈に失敗した場合のみ `Err` を返す。タグの欠落は `None` である。`Deal` タグの不正 (枚数、重複) は `BadTag` ではなく `parse_lenient` の `BadDeal` 警告で報告し、ビューには読めた手だけの `PartialDeal` が入る。`parse_lenient` はゲーム単位で回復する。ゲームの途中で回復不能な行に当たったらそのゲームの残りを `Raw` に落として次の空行から再開する。

公開面 (`lib.rs`): `pub mod deal_string; pub mod lin; pub mod pbn;` と、`pbn::{Game, GameView, PartialDeal, PbnFile, Section, TagPair, TagValue, Token, WriteOptions}`、`Warning`, `WarningKind`, `ParseError` の再エクスポート。`pbn` モジュールはさらに `Comment`, `Directive`, `ViewError`, `parse_lenient`, `parse_strict`, `write` を公開する。

## 3. PBN 2.1 文法 (実装通り)

パーサは `winnow` 1.0 の `ModalResult` で行指向に書く。各パーサ関数は失敗を `Warning` に変えて回復する。仕様の根拠は `https://www.tistis.nl/pbn/pbn_v21.txt`。

1. **バイト列 → テキスト**: UTF-8 として復号し、失敗したら ISO-8859-1 (バイト → 同値の `char`) で復号して `WarningKind::Encoding` を出す。CRLF と CR は LF に正規化する。
2. **エスケープ行**: 第 1 列が `%` の行は `Directive`。`% PBN x.y` は `Version(x, y)`、`% EXPORT` は `Export`、それ以外は `Other` に原文で保持する (`%BoardsPerPage` 等)。ゲームの途中に現れても受理する。
3. **コメント**: `;` から行末、および `{ … }` (入れ子なし、複数行可)。文字列リテラル内を除くあらゆる場所で読み飛ばし、`Comment { after_tag }` として直前のタグ対に結び付ける (ゲーム先頭なら `None`)。閉じ `}` がなければ `Truncated` を出しファイル末尾までをコメントとする。
4. **ゲーム区切り**: ブレースコメントの外側にある半空行 (空白だけの行)。最初のゲームは空行なしで始まってよい。連続する空行は 1 つの区切り。
5. **タグ対**: `[` ws `Name` ws `"` string `"` ws `]`。`Name = [A-Za-z][A-Za-z0-9_]*`。文字列のエスケープは `\\` と `\"`。値 `#` は `TagValue::Inherited` (前ゲームの同名タグ。`Game::view(previous)` で解決し、`previous` が `None` なら `ViewError::NothingToInherit`)、`##text` は `TagValue::Default(text)`。import 形式では 1 行に複数のタグ、複数行にまたがるタグを許す。同名タグの重複は `Other` 警告 (2 つ目も保持し、`get` は最初を返す)。
6. **セクション**: `Name ∈ {Auction, Play}` または `Name` が `Table` で終わるタグの後続行のうち `[` で始まらない行はトークン行で、次の `[` かゲーム区切りまで続く。直後に続く `[Note "n:text"]` (`n` は 1..=32) は最後のセクションの `notes` に付く。名前空間はセクションごとに独立 (Auction の `=1=` と Play の `=1=` は別)。セクションのないところに `Note` があれば `NoteReference`。
7. **Auction トークン** (import では大文字小文字を区別しない):

   | トークン | `Token` | 備考 |
   | --- | --- | --- |
   | `[1-7](C\|D\|H\|S\|NT)` | `Call(Bid)` | `N` 単独は受理しない (export では `NT`) |
   | `Pass`, `P` | `Call(Pass)` | |
   | `X`, `XX` | `Call(Double)`, `Call(Redouble)` | |
   | `AP` | `Call(Pass)` × 必要数 | オークションが完了するまでパスを補う |
   | `-` | `Unknown` | 不明・不完全なコール。以降のオークションは `Auction` に取り込まない |
   | `*` | `Terminator` | |
   | `+` | `Continuation` | |
   | `=n=` | `NoteRef(n)` | 直前のコールへの注釈参照 |
   | `$n` | `Nag(n)` | |
   | `!`, `?`, `!!`, `??`, `!?`, `?!` | `Suffix` | export では NAG `$1..$6` に変換 |
   | それ以外 | `Raw` | `MalformedToken` 警告 |

   `[Auction "N"]` の `N` はディーラーで、ビューは `Auction::new(dealer, vulnerable)` に `Call` トークンを順に `push` し、違反は `BadAuction` (index 付きメッセージ) で報告してそこで打ち切る。
8. **Play トークン**: `[SHDC][AKQJT98765432]` → `Card`、`-`、`*`、`+`、注釈と NAG は Auction と同じ。`[Play "W"]` の `W` は各行の先頭列の席で、行は 1 トリック、列は `W` から時計回りに固定である。実際の出順はトリック `t` のリーダー (t = 0 はオープニングリーダー = `W`、以降は前トリックの勝者) から時計回りなので、ビューは行ごとに列を回転して `PlayHistory::play` に流す。`-` を含む行はそのトリック以降を切り捨て `Truncated` を出す。トランプは `Contract` タグから、リーダーが `Declarer` の LHO と一致しなければ `ViewError::BadTag`。
9. **Deal**: `<first>:<h1> <h2> <h3> <h4>`。`<first>` は `N|E|S|W`、手はそこから時計回り。各手は `S.H.D.C` の順にランクを並べ、10 は `T`、ボイドは空文字。import ではランクの順序を問わず、`-` は不明な手 (`PartialDeal` の `None`)。13 枚でない手や重複は `BadDeal` を出し `PartialDeal` に落とす (該当の手は `None`)。
10. **タグ値の慣例**:

    | タグ | 値 | ビュー |
    | --- | --- | --- |
    | `Vulnerable` | `None\|Love\|-\|NS\|EW\|All\|Both` | `Vulnerability` (`Love`,`-` → `None`; `All` → `Both`) |
    | `Dealer` | `N\|E\|S\|W` | `Seat` |
    | `Contract` | `<1-7><S\|H\|D\|C\|NT>[X\|XX]` または `Pass` | `Contract` (declarer は `Declarer` タグから) / `Pass` は `None` |
    | `Declarer` | `[^]N\|E\|S\|W` | `declarer` (`^` は剥がす) |
    | `Result` | `n \| NS n \| EW n \| ^…` | 宣言者側のトリック数。`NS n`/`EW n` は宣言者の側に読み替える |
    | `Board` | 整数 | `u16` |
    | `OptimumResultTable` | `"Declarer;Denomination\2R;Result\2R"` + 行 `N C 9` | `DdTable::new(..)` (20 行揃わなければ `BadTag`) |
    | `OptimumScore`, `ParContract` 等 | 文字列 | `get` で参照するのみ |

11. **Writer (export 形式)**: 先頭に `% PBN 2.1` と `% EXPORT`。MTS (Mandatory Tag Set) を固定順 `Event, Site, Date, Board, West, North, East, South, Dealer, Vulnerable, Deal, Scoring, Declarer, Contract, Result` で出し、無いタグは `"?"` (Deal は `"#"` を許さず、無ければ `"?"`)、補助タグはアルファベット順、1 行 1 タグ、行末 CRLF、タブなし、ランクとコールは大文字、`Deal` のランクは降順で `<first>` = ディーラー、セクションはそのタグの直後、注釈参照 `=n=` は `$n` より前、`Suffix` は NAG に変換 (`!`→`$1`, `?`→`$2`, `!!`→`$3`, `??`→`$4`, `!?`→`$5`, `?!`→`$6`)、Auction は 1 行 4 コール、Play は 1 行 4 枚、末尾の 3 パスは `AP`。`export = false` ではタグ順・値・コメントを原文通りに、LF で書く。

## 4. LIN 文法 (BBO、lenient、`lin.rs`)

ストリームは `tag|value|` の列で、タグは 2 文字。`|` を含まない断片や末尾の対にならない断片は `MalformedToken` で読み飛ばす。

```rust
pub fn parse_lenient(input: &[u8]) -> (Vec<LinBoard>, Vec<Warning>);   // lin::parse_lenient

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LinBoard {
    pub id: Option<String>,                              // `qx|o14|` → "o14" (first char is the room)
    pub names: Vec<String>,                              // `pn|` as given (4 names, or 8 in vugraph files)
    pub board_no: Option<u16>,                           // `ah|Board n|`
    pub dealer: Option<Seat>,
    pub vul: Option<Vulnerability>,
    pub deal: Option<PartialDeal>,
    pub calls: Vec<(Call, bool, Option<String>)>,        // (call, alerted, announcement)
    pub plays: Vec<Card>,
    pub claim: Option<u8>,                               // `mc|n|`: tricks claimed by declarer (total)
    pub raw: Vec<(String, String)>,                      // every other recognised tag, in order
}

impl LinBoard {
    pub fn to_game(&self) -> Game;                       // PBN: tags + Auction/Play sections + Notes
}
```

`names` は `pn|` の原文順 (S, W, N, E。Vugraph は オープン室 4 名 + クローズ室 4 名) で、席への割り当てと室の選択は `to_game()` が `id` の先頭文字を見て行う。オークションは `to_game()` の `Auction` セクションからビューで得る (ディーラーが無ければ `board_no` から `Seat::dealer_of_board`)。

| タグ | 意味 | 扱い |
| --- | --- | --- |
| `vg\|title,session,I\|O,first,last,…\|` | Vugraph ヘッダ | `raw` |
| `pn\|S,W,N,E\|` | 名前。Vugraph は 8 名 `pn\|s,w,n,e,s,w,n,e\|` (オープン室 4 名 + クローズ室 4 名) | `names` に原文順で保持。`to_game()` が室で 4 名を選ぶ |
| `qx\|o14\|`, `qx\|c14\|` | ボード境界。`o`/`c` は室、数字はボード番号 | 新しい `LinBoard` を開始。`qx` が無ければ単一ボードファイル (`st\|\|`, `md\|…` で開始) |
| `rh\|\|`, `ah\|Board 1\|`, `st\|\|` | 見出し | `raw`。`ah` から `board_no` を補う |
| `md\|<d><hands>\|` | ディール。`d` ∈ 1=S, 2=W, 3=N, 4=E がディーラー。手は S, W, N, E の順に `,` 区切り、各手は `S…H…D…C…` (10 は `T` または `10`) | `dealer` + `PartialDeal`。4 手目が欠落または空なら残りから計算する。13 枚でなければ `BadDeal` 警告のまま `PartialDeal`。`d = 0` はボード番号から `Seat::dealer_of_board` |
| `sv\|o\|`, `sv\|n\|`, `sv\|e\|`, `sv\|b\|` | バルネラビリティ (`0` と大文字も可) | `None` / `NS` / `EW` / `Both`。無ければ `Vulnerability::from_board_number` |
| `mb\|<call>\|` | コール。`p`, `d`, `r`, `1C`, `1N`/`1NT`、大文字小文字を区別しない。末尾 `!` はアラート | `(call, alerted, None)` |
| `an\|text\|` | 説明 | 直前の `mb` の第 3 要素 |
| `pc\|<card>\|` | プレイ。`SK`, `h2` (大文字小文字を区別しない) | `plays` |
| `mc\|n\|` | クレーム (宣言者の総トリック数) | `claim` |
| `pg\|\|` | ページ区切り | 無視 |
| `nt\|…\|` | チャット・コメント | `raw` |
| `o1..o4\|`, `bn\|`, `bt\|`, `tu\|`, `cr\|`, `cs\|`, `hc\|`, `lc\|`, `mp\|`, `pf\|`, `up\|` | その他 | `raw` |
| 不明タグ | | `WarningKind::UnknownTag` で読み飛ばす |

`to_game()` は `Board`, `Dealer`, `Vulnerable`, `Deal` (`N:` 形式)、`West/North/East/South` (名前)、`Auction` セクション (アラートと `an` は `=n=` + `[Note "n:…"]`、注釈が無いアラートは `Note "n:Alert"`)、`Play` セクション (オープニングリーダーからの列順)、`Contract`/`Declarer` (`Auction::contract()`)、`Result` (プレイが完了していればトリック数、`claim` があればその値) を生成する。

## 5. Deal 文字列 (`deal_string.rs`)

| 形式 | 例 | 型 | 定義場所 |
| --- | --- | --- | --- |
| 1 手 | `AKQJ..AKQJ.23456` (♠.♥.♦.♣、ボイドは空) | `Hand` | `bridge_core::fmt` の `Display`/`FromStr` (`02-core.md`)。`FromStr` は 13 枚未満も受理する (残り手を表すため) |
| 4 手 | `N:AKQJ..AKQJ.23456 … … …` | `Deal` / `PartialDeal` | 本クレート `deal_string::parse` / `deal_string::write`。PBN `Deal` タグ値と同一。`bridge_core::Deal` の `FromStr` も 4 手完備なら同じ文字列を読む |

```rust
pub fn parse(input: &str) -> Result<PartialDeal, ParseError>;   // `-` = unknown hand
pub fn write(deal: &Deal, first: Seat) -> String;                // clockwise from `first`, ranks descending
```

規則: ランクは `AKQJT98765432` (import では `10` も `T` と読み、順序は問わない)。`write` は常に降順。スート内の重複と 13 枚超は `ParseError`。`-` が不明の手で、完備な `Deal` が要るときは `parse(..)?.complete()` を使う。

## 6. テスト

### 6.1 proptest ラウンドトリップ

| 戦略 | 生成 |
| --- | --- |
| `arb_deal()` | 52 枚をシャッフルして 4 × 13 に配る |
| `arb_auction()` | `Auction::is_legal` を満たすコールを完了までランダムに選ぶ (ディーラーとバルネラビリティもランダム) |
| `arb_play()` | 合法なフォロー (スートがあれば従う) をランダムに 0..=52 枚 |
| `arb_tags()` | 語彙 (Event/Site/Date/Scoring 等) から値を生成。`\"` と `\\` を含む文字列と Latin-1 文字を混ぜる |
| `arb_game()` | 上記を組み合わせ、`Contract`/`Declarer`/`Result` を `Auction::contract()` とプレイから導出 |

性質: `parse_strict(write(g, export)) == normalize(g)` (`normalize` はタグ順の正規化、`AP` 展開、`Suffix` → `Nag`、ランク降順)、`write(parse(write(g))) == write(g)` (冪等)、`view(parse(write(g))) == view(g)`。ケース数 10^4。

### 6.2 スナップショット

`tests/snapshots/` に手選びの 30 件 (`*.pbn`, `*.lin`) と期待出力 (`insta` の `.snap`、`Debug` 文字列でも可) を置く。必ず含める題材: `[Note]` の参照、`#`/`##` の継承、`AP`、`-` の手、複数行のブレースコメント、複数タグが 1 行にある import 形式、`OptimumResultTable`、LIN の 4 手目省略、8 名の Vugraph、クレーム付き、チャット混入。

### 6.3 fuzz

`cargo fuzz` ターゲット `pbn_parse_lenient` と `lin_parse_lenient` (`crates/bridge-format/fuzz/`、ワークスペース外)。基準はパニック 0。ナイトリーまたはローカルで `cargo +nightly fuzz run pbn_parse_lenient -- -max_total_time=600`。

### 6.4 コーパス パース率

`tests/corpus.rs` (`#[ignore]`、`BRIDGE_CORPUS_DIR` 未設定ならスキップ)。1 ゲームを「成功」と数える条件は `view()` が `Deal` を返し、存在する `Auction`/`Play` がタグと矛盾しないこと。閾値は PBN ≥ 99% (コーパス 1, 2, 4)、LIN ≥ 95% のボードが `Deal` と `Auction` を返すこと (コーパス 3)、パニック 0。

## 7. コーパス

| # | ソース | 形式 | URL | 用途・備考 |
| --- | --- | --- | --- | --- |
| 1 | computerbridge.se (世界選手権・NABC 決勝) | PBN (zip) | `https://www.computerbridge.se/finals-world-championship-in-pbn/` → 例 `…/Bermuda%20Bowl%202019%20Final.zip?ph=11c38132ee`, `…/Venice%20Cup%20Final%202019.zip?…`, `…/Spingold%202019%20Final.zip?…`, `…/Vanderbilt%202019%20Final.zip?…` | オークションとプレイ付きの実戦記録。CDN リンクは `?ph=` トークン付きなので URL と sha256 を固定し、切れたら再確認する (R7) |
| 2 | DDS テストハンド (`dds-bridge/dds` タグ v2.9.0) | Deal 文字列 + `TABLE`/`PAR`/`PLAY`/`TRACE` 行 | `https://raw.githubusercontent.com/dds-bridge/dds/v2.9.0/hands/list100.txt`, `https://raw.githubusercontent.com/dds-bridge/dds/v2.9.0/hands/masterDD.txt` (83,691 配牌) | タグ固定で不変。Deal 文字列パーサのテストと **DDS 差分テスト** (`10-dds.md`) |
| 3 | BBO Vugraph アーカイブ | LIN | `https://www.bridgebase.com/tools/vugraph_linfetch.php?id=<N>` (id 一覧は `https://www.bridgebase.com/vugraph_archives/vugraph_archives.php`、例 87143) | 実データの不規則性 (8 名、`qx\|o/c`、クレーム、チャット)。20 件の id を固定、取得は 1 req/s。未決: 20 件の id |
| 4 | PBN ホームページの例 | PBN | `https://www.tistis.nl/pbn/OptimumResultTable.pbn`, `https://www.tistis.nl/pbn/OptimumPlayTable.pbn` | 小さい。テーブルセクションの準拠 |
| 5 (未検証) | Vugraph Project (`sarantakos.com/bridge/vugraph.html`、TLS エラー)、Bridgetoernooi (`bridgetoernooi.com`、DNS 失敗) | PBN/LIN | | 1955〜2013 の大規模アーカイブ。到達できれば追加 |

形式ごとに 2 ソース以上を持ち (R7)、タグ固定の DDS ファイルを常設の安定ソースとする。ハンドレコードは事実データで各ソースに制限の記載はないが、コミットは決してしない。

### 7.1 `corpus/manifest.toml`

骨格に置いた雛形の形式 (sha256 は空のまま):

```toml
# corpus/manifest.toml: sources of test corpora. Data is fetched into corpus/data/ (git-ignored).

[[entry]]
name        = "dds-list100"
description = "DDS v2.9.0 test hands with double-dummy tables (100 deals); tag-pinned, immutable"
url         = "https://raw.githubusercontent.com/dds-bridge/dds/v2.9.0/hands/list100.txt"
sha256      = ""            # 未決: 初回 fetch 時に固定する
unpack      = "none"        # "none" | "zip"
dest        = "dds/list100.txt"
formats     = ["deal-string", "dd-table"]   # "pbn" | "lin" | "deal-string" | "dd-table"

[[entry]]
name        = "dds-masterdd"
description = "DDS v2.9.0 master double-dummy set (83,691 deals); used by the --ignored differential test"
url         = "https://raw.githubusercontent.com/dds-bridge/dds/v2.9.0/hands/masterDD.txt"
sha256      = ""
unpack      = "none"
dest        = "dds/masterDD.txt"
formats     = ["deal-string", "dd-table"]

[[entry]]
name        = "pbn-optimum-result-table"
description = "PBN 2.1 reference example with an OptimumResultTable section"
url         = "https://www.tistis.nl/pbn/OptimumResultTable.pbn"
sha256      = ""
unpack      = "none"
dest        = "pbn/OptimumResultTable.pbn"
formats     = ["pbn"]
```

computerbridge.se の zip エントリ (`unpack = "zip"`, `dest = "pbn/<name>/"`) と BBO の LIN エントリ (`rate_limit_ms = 1000`) はフェーズ 1.9 で追加する。

### 7.2 `cargo xtask corpus fetch`

1. `corpus/manifest.toml` を読み、出力先を `BRIDGE_CORPUS_DIR` (未設定なら `corpus/data/`) とする。
2. 各 `entry` について `dest` が存在し `.sha256` サイドカーが一致すればスキップ (`--force` で再取得、`--only <name>` で限定)。
3. `ureq` で取得 (同一ホストへは `rate_limit_ms` 以上の間隔)。一時ファイルに書き、`sha2` で sha256 を計算する。
4. `sha256` が空なら計算値を表示して終了コード 2 (manifest に貼る)。不一致なら一時ファイルを消して失敗する。ハッシュが一致しないデータは決してテストに使わない。
5. `unpack = "zip"` なら `zip` で `dest` に展開する。`.sha256` サイドカーと `corpus/data/.manifest-hash` (manifest の blake3) を書く。

テスト側は `tests/common/mod.rs` の `corpus_dir() -> Option<PathBuf>` で `BRIDGE_CORPUS_DIR` を読み、`None` なら `eprintln!` して即座に `return` する (ネットワークには触れない)。CI は `corpus/data` を `hashFiles('corpus/manifest.toml')` をキーにキャッシュし、ナイトリージョブだけが取得する (`11-testing.md` §10)。

## 8. モジュール配置

```
crates/bridge-format/
├── Cargo.toml
├── src/
│   ├── lib.rs             # re-exports, ParseError, crate docs, #![allow(dead_code, unused_variables)] (phase 0)
│   ├── warning.rs         # Warning, WarningKind
│   ├── deal_string.rs     # parse (4-hand deal string -> PartialDeal), write
│   ├── lin.rs             # lin::parse_lenient, LinBoard, to_game
│   └── pbn/
│       ├── mod.rs         # re-exports of the pbn types and functions
│       ├── model.rs       # PbnFile, Directive, Game, TagPair, TagValue, Section, Token, Comment
│       ├── parser.rs      # bytes -> text, directives, comments, game separation, tag pairs, sections, tokens (winnow)
│       ├── view.rs        # GameView, PartialDeal, ViewError, Game::view, play rotation, OptimumResultTable -> DdTable
│       └── writer.rs      # WriteOptions, write (export and verbatim)
├── tests/                 # phase 1: deal_string.rs  pbn_roundtrip.rs  pbn_snapshots.rs  lin.rs  corpus.rs  common/mod.rs
│   └── snapshots/         # *.pbn, *.lin, *.snap
├── benches/parse.rs       # phase 1: criterion, games/s for parse_lenient (no target; informational)
└── fuzz/                  # phase 1: cargo fuzz targets pbn_parse_lenient, lin_parse_lenient (outside the workspace)
```

`parser.rs` はレキサ (行分割、ディレクティブ、コメント、ゲーム区切り) とトークン文法を 1 ファイルに持つ。実装順はフェーズ 1 (`12-roadmap.md`): Deal 文字列 → PBN parser (lenient) + `Warning` → `GameView` → writer + proptest → コーパスと xtask → LIN。

## 9. 未決

- 未決: RBN 対応の要否 (v2 送り)。
- 未決: コーパス 3 (BBO Vugraph) の固定 20 件の id と、各エントリの sha256 (初回 fetch 時に確定、フェーズ 1.9)。
- 未決: コーパス 5 の到達可否 (到達できた時点で manifest に追加)。
- 未決: `Declarer`/`Result` の `^` 接頭辞の正確な意味 (PBN 2.1 §3 の再確認。ビューは剥がすだけなので実装に影響しない)。
