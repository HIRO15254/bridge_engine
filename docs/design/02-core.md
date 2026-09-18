# 02. `bridge-core` (L0): コアドメイン型

本文書は `bridge-core` の全公開型とその完全なシグネチャ、ビット配置、オークション合法性・コントラクト導出・トリック勝者・バルネラビリティの規則、表示形式、エラー型、serde の扱い、`const fn` の制約を確定する。このクレートは依存ゼロ (`serde` optional、`thiserror` のみ) で、一度公開したら変えない層である。`Shape`/`ShapeClass`/`ShapeSet` はここに置く (D5)。

## 1. ビット配置と表示順

| 型 | 内部表現 | 配置 | 備考 |
| --- | --- | --- | --- |
| `Card(u8)` | `0..52` | `index = suit * 13 + rank` (♣2 = 0, ♠A = 51) | `bit() = 1 << index` |
| `Holding(u16)` | 13 bit | bit `rank` (Two = 0 … Ace = 12) | 最高札 = `15 − leading_zeros()`。数値そのものが 8192 要素テーブルの添字 |
| `Hand(u64)` | 52 bit | bit `card.index()`。♣ が bit `0..13`、♠ が `39..52` | `FULL = (1 << 52) − 1`。`complement` は必ず `FULL` でマスク |
| `Shape(u16)` | nibble × 4 | nibble `suit` (♣ が下位 4 bit、♠ が bit `12..16`) | 順序あり (スート固定) |
| `ShapeClass(u16)` | nibble × 4 | 降順ソート済み (nibble 3 が最長) | 順序なしパターン。13 枚で 39 種 |
| `ShapeSet([u64; 9])` | 576 bit | bit `i` ⇔ `SHAPES[i]`。bit `560..576` は常に 0 | 13 枚の順序付きシェイプ 560 個の集合 |
| `Bid(u8)` | `0..35` | `index = (level − 1) * 5 + strain` (1♣ = 0, 7NT = 34) | レベル比較が整数比較になる |
| `DdTable` | `[[u8; 4]; 5]` | `tricks[strain][declarer]` (`Strain` 順、`Seat` 順) | ダブルダミー表。`bridge-dds` が生成し `bridge-format` が読む純データ型 (§2.10) |
| `Seat`, `Suit`, `Rank`, `Strain`, `Vulnerability` | `#[repr(u8)]` | 宣言順 | `from_index` / `index` は `const fn` |

表示は PBN 準拠で **♠.♥.♦.♣** (`AKQ.234.AKQ.2345`)。内部のビット順 (♣ が下位) との変換は `fmt` モジュールの `Display`/`FromStr` だけが知る。`Shape` の表示は `5=4=3=1` (♠=♥=♦=♣)、`ShapeClass` は `5-4-3-1`。

## 2. 型定義

### 2.1 `Suit`, `Rank`, `Card` (`card.rs`)

```rust
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Suit { Clubs = 0, Diamonds = 1, Hearts = 2, Spades = 3 }

impl Suit {
    pub const ALL: [Suit; 4];
    pub const fn from_index(i: u8) -> Suit;      // i >= 4 は契約違反 (panic)
    pub const fn index(self) -> u8;
    pub const fn shift(self) -> u8;              // 13 * index
    pub const fn mask(self) -> u64;              // 0x1FFF << shift
    pub const fn symbol(self) -> char;           // ♣ ♦ ♥ ♠
    pub const fn letter(self) -> char;           // C D H S
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Rank { Two = 0, Three, Four, Five, Six, Seven, Eight, Nine, Ten = 8, Jack = 9, Queen = 10, King = 11, Ace = 12 }

impl Rank {
    pub const ALL: [Rank; 13];
    pub const fn from_index(i: u8) -> Rank;      // i >= 13 は契約違反 (panic)
    pub const fn index(self) -> u8;
    pub const fn hcp(self) -> u8;                // A=4, K=3, Q=2, J=1, else 0
    pub const fn to_char(self) -> char;          // 2..9 T J Q K A
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Card(u8);

impl Card {
    pub const fn new(suit: Suit, rank: Rank) -> Card;
    pub const fn from_index(i: u8) -> Option<Card>;   // i >= 52 は None
    pub const fn index(self) -> u8;
    pub const fn suit(self) -> Suit;                  // index / 13
    pub const fn rank(self) -> Rank;                  // index % 13
    pub const fn bit(self) -> u64;                    // 1 << index
}
// Debug は "SA" (スート文字 + ランク文字)。Display は fmt モジュール (§4)。
```

### 2.2 `Holding` (`holding.rs`)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Holding(u16);

impl Holding {
    pub const EMPTY: Holding;                          // 0
    pub const FULL: Holding;                           // 0x1FFF
    pub const fn from_bits(bits: u16) -> Option<Holding>;   // bit 13 以上が立っていれば None
    pub const fn bits(self) -> u16;
    pub const fn len(self) -> u8;                      // count_ones
    pub const fn is_empty(self) -> bool;
    pub const fn contains(self, rank: Rank) -> bool;
    pub const fn with(self, rank: Rank) -> Holding;
    pub const fn without(self, rank: Rank) -> Holding;
    pub const fn highest(self) -> Option<Rank>;        // 15 − leading_zeros
    pub const fn lowest(self) -> Option<Rank>;         // trailing_zeros
    pub const fn top_ranks(n: u8) -> Holding;          // 上位 n ランク (A, K, Q, …) のマスク。n は 13 にクランプ
    pub const fn union(self, other: Holding) -> Holding;
    pub const fn intersect(self, other: Holding) -> Holding;
    pub const fn difference(self, other: Holding) -> Holding;
    pub const fn complement(self) -> Holding;          // !bits & 0x1FFF
    pub const fn is_subset(self, other: Holding) -> bool;
    pub fn ranks(self) -> HoldingRanks;                // 高い順
    pub fn submasks(self) -> Submasks;                 // 全部分集合 (EMPTY と self を含む)。数値降順
}

/// `Holding::ranks` のイテレータ (Item = Rank、高い順)。
pub struct HoldingRanks { bits: u16 }
/// `Holding::submasks` のイテレータ (Item = Holding)。`sub = (sub − 1) & mask` の走査。サンプラーが使う。
pub struct Submasks { mask: u16, current: Option<u16> }

impl BitOr / BitAnd / Sub / Not for Holding   // union / intersect / difference / complement に委譲 (非 const)
```

`top_ranks(n)` の式: `n == 0 → 0`、それ以外は `(0x1FFF >> (13 − n)) << (13 − n)`。`top_ranks(5)` は AKQJT。

### 2.3 `Hand` (`hand.rs`)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Hand(u64);

impl Hand {
    pub const EMPTY: Hand;
    pub const FULL: Hand;                               // (1 << 52) − 1
    pub const fn from_bits(bits: u64) -> Option<Hand>;  // bits >> 52 != 0 なら None
    pub const fn bits(self) -> u64;
    pub const fn from_holdings(clubs: Holding, diamonds: Holding, hearts: Holding, spades: Holding) -> Hand;
    pub const fn holding(self, suit: Suit) -> Holding;
    pub const fn with_holding(self, suit: Suit, holding: Holding) -> Hand;
    pub const fn len(self) -> u8;
    pub const fn is_empty(self) -> bool;
    pub const fn shape(self) -> Shape;                  // 4 回の masked popcount
    pub const fn contains(self, card: Card) -> bool;
    pub const fn with(self, card: Card) -> Hand;
    pub const fn without(self, card: Card) -> Hand;
    pub const fn union(self, other: Hand) -> Hand;
    pub const fn intersect(self, other: Hand) -> Hand;
    pub const fn difference(self, other: Hand) -> Hand;
    pub const fn complement(self) -> Hand;              // !bits & FULL
    pub const fn is_disjoint(self, other: Hand) -> bool;
    pub const fn is_subset(self, other: Hand) -> bool;
    pub fn cards(self) -> HandCards;                    // 昇順 (♣ の低いランクから)
}

/// `Hand::cards` のイテレータ (Item = Card)。trailing_zeros で走査。
pub struct HandCards { bits: u64 }

impl BitOr / BitAnd / Sub / Not for Hand   // 非 const、inherent const fn に委譲
```

`Hand` は 13 枚である必要がない。プレイ途中の残り手、既出カード、サンプラーのプール (未知カード集合) も `Hand` で表す。13 枚検証は `Deal::new` のみが行う。

### 2.4 `Shape`, `ShapeClass`, `ShapeSet` (`shape.rs`)

```rust
/// 順序付きシェイプ。nibble i = Suit i の長さ。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shape(u16);

impl Shape {
    pub const fn new(c: u8, d: u8, h: u8, s: u8) -> Shape;     // debug_assert!(各 <= 13)
    pub const fn from_lens(lens: [u8; 4]) -> Shape;             // Suit 順 (♣ が先頭)
    pub const fn bits(self) -> u16;
    pub const fn len(self, suit: Suit) -> u8;                   // (bits >> (4 * suit)) & 0xF
    pub const fn lens(self) -> [u8; 4];                         // ♣ が先頭
    pub const fn total(self) -> u8;
    pub const fn class(self) -> ShapeClass;                     // 5 比較器のソーティングネットワーク (const 可)
    pub const fn index(self) -> u16;                            // debug_assert!(total() == 13)。shape_index(c, d, h)
    pub const fn from_index(i: u16) -> Shape;                   // SHAPES[i]
    pub const fn add(self, other: Shape) -> Shape;              // nibble 加算。各和 <= 13 の範囲で桁上がりなし
    pub const fn checked_sub(self, other: Shape) -> Option<Shape>;
    pub const fn longest(self) -> u8;
    pub const fn shortest(self) -> u8;
    pub const fn is_balanced(self) -> bool;                     // class().is_balanced()
}

/// {(c, d, h) : c + d + h <= 13} → 0..560 の全単射 (辞書順)。数値検証済み。
pub const fn shape_index(c: u8, d: u8, h: u8) -> u16 {
    const T3: [u16; 14] = [0, 105, 196, 274, 340, 395, 440, 476, 504, 525, 540, 550, 556, 559];
    let (c, d, h) = (c as u16, d as u16, h as u16);
    T3[c as usize] + d * (14 - c) - (d * d - d) / 2 + h
}
pub const SHAPES: [Shape; 560];      // const fn (入れ子 while) で構築。SHAPES[i].index() == i
pub const CLASS_OF: [u8; 560];       // SHAPES[i].class() の CLASSES 内添字

/// 順序なしパターン。nibble 降順、nibble 3 が最長。表示 "5-4-3-1"。導出 Ord は最長スート優先の順序。
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShapeClass(u16);
pub const CLASSES: [ShapeClass; 39]; // SHAPES を走査した初出順

impl ShapeClass {
    pub const C4333: ShapeClass; pub const C4432: ShapeClass; pub const C5332: ShapeClass;
    pub const C5422: ShapeClass; pub const C6322: ShapeClass; pub const C4441: ShapeClass;
    pub const C5431: ShapeClass; pub const C5521: ShapeClass; pub const C6331: ShapeClass;
    pub const C7222: ShapeClass;               // 名前付き定数は以上 10 個
    pub const fn new(lens: [u8; 4]) -> ShapeClass;   // 任意の順序で受け取り降順に正規化
    pub const fn bits(self) -> u16;
    pub const fn lens(self) -> [u8; 4];        // 降順
    pub const fn total(self) -> u8;
    pub const fn index(self) -> u8;            // CLASSES の線形探索 (39)。13 枚でなければ panic
    pub const fn from_index(i: u8) -> ShapeClass;
    pub const fn longest(self) -> u8;
    pub const fn shortest(self) -> u8;
    pub const fn is_balanced(self) -> bool;    // 4333 | 4432 | 5332
    pub const fn is_semi_balanced(self) -> bool;  // balanced | 5422 | 6322
}

/// 13 枚の順序付きシェイプの集合。bit i ⇔ SHAPES[i]。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShapeSet([u64; 9]);

impl ShapeSet {
    pub const EMPTY: ShapeSet;
    pub const ALL: ShapeSet;                   // 下位 560 bit: [u64::MAX; 8] + (1 << 48) − 1
    pub const BALANCED: ShapeSet;              // from_classes(&[C4333, C4432, C5332])。28 シェイプ
    pub const SEMI_BALANCED: ShapeSet;         // BALANCED ∪ from_classes(&[C5422, C6322])。52 シェイプ

    pub const fn from_words(words: [u64; 9]) -> Option<ShapeSet>;   // bit 560 以上が立っていれば None
    pub const fn words(self) -> [u64; 9];
    pub const fn contains(self, s: Shape) -> bool;        // debug_assert!(s.total() == 13)
    pub const fn insert(self, s: Shape) -> ShapeSet;
    pub const fn remove(self, s: Shape) -> ShapeSet;
    pub const fn union(self, o: ShapeSet) -> ShapeSet;
    pub const fn intersect(self, o: ShapeSet) -> ShapeSet;
    pub const fn difference(self, o: ShapeSet) -> ShapeSet;
    pub const fn complement(self) -> ShapeSet;            // ALL と XOR (単純な NOT は禁止)
    pub const fn is_empty(self) -> bool;
    pub const fn len(self) -> u16;                        // Σ count_ones
    pub const fn is_subset(self, o: ShapeSet) -> bool;
    pub const fn from_class(c: ShapeClass) -> ShapeSet;   // 560 ループ、CLASS_OF[i] == c.index()
    pub const fn from_classes(cs: &[ShapeClass]) -> ShapeSet;
    pub const fn from_suit_len(s: Suit, lo: u8, hi: u8) -> ShapeSet;
    pub const fn from_suit_lens(lens: [(u8, u8); 4]) -> ShapeSet;   // 4 つの from_suit_len の AND (直積)
    pub fn filter(pred: impl Fn(Shape) -> bool) -> ShapeSet;         // 汎用ビルダ (非 const)
    pub fn iter(self) -> ShapeSetIter;                               // trailing_zeros 走査

    // 射影 (要約)。過大近似であり satisfies には使わない。
    pub fn suit_len(self, s: Suit) -> Option<RangeInclusive<u8>>;   // 空集合なら None
    pub fn classes(self) -> u64;                                    // 39 bit のクラスマスク
    pub fn factor(self) -> Option<[RangeInclusive<u8>; 4]>;         // 自身が射影の直積に等しいときのみ Some
    pub fn min_hcp(self) -> u8;                                     // min over members of Σ MIN_HCP[len]
    pub fn max_hcp(self) -> u8;                                     // max over members of Σ MAX_HCP[len]
}

/// `ShapeSet::iter` のイテレータ (Item = Shape、index 昇順)。
pub struct ShapeSetIter { words: [u64; 9], word: usize }

pub const MAX_HCP: [u8; 14] = [0, 4, 7, 9, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10];
pub const MIN_HCP: [u8; 14] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 3, 6, 10];
```

数値 (検証済み): 順序付きシェイプ 560、クラス 39、バランス 28、セミバランス 52、`(l0, l1)` 対 105、13 枚の HCP 最大 37。`MAX_HCP[l]` はスート `l` 枚での最大 HCP (AKQJ で 10)、`MIN_HCP[l]` は最小 (10 枚なら J を含まざるを得ないので 1)。

### 2.5 `Seat`, `Side`, `Vulnerability` (`seat.rs`)

```rust
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Seat { North = 0, East = 1, South = 2, West = 3 }

impl Seat {
    pub const ALL: [Seat; 4];
    pub const fn from_index(i: u8) -> Seat;
    pub const fn index(self) -> u8;
    pub const fn offset(self, n: u8) -> Seat;          // (index + n) % 4
    pub const fn next(self) -> Seat;                   // offset(1)
    pub const fn prev(self) -> Seat;                   // offset(3)
    pub const fn partner(self) -> Seat;                // offset(2)
    pub const fn lho(self) -> Seat;                    // next の別名
    pub const fn rho(self) -> Seat;                    // prev の別名
    pub const fn side(self) -> Side;                   // index % 2 == 0 → NS
    pub const fn dealer_of_board(n: u16) -> Seat;      // from_index(((n + 3) % 4) as u8)。ボード 1 → N
    pub const fn letter(self) -> char;                 // N E S W
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Side { NS = 0, EW = 1 }
impl Side {
    pub const fn seats(self) -> [Seat; 2];
    pub const fn other(self) -> Side;
    pub const fn contains(self, seat: Seat) -> bool;
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Vulnerability { None = 0, NS = 1, EW = 2, Both = 3 }

impl Vulnerability {
    pub const fn from_index(i: u8) -> Vulnerability;
    pub const fn index(self) -> u8;
    /// 16 ボード周期。ボード 0 は 16 と同じ扱い。
    pub const fn from_board_number(n: u16) -> Vulnerability {
        let i = ((n + 15) % 16) as u8;
        Vulnerability::from_index((i / 4 + i % 4) % 4)
    }
    pub const fn is_vulnerable_side(self, side: Side) -> bool;   // Both | (NS, NS) | (EW, EW)
    pub const fn is_vulnerable(self, seat: Seat) -> bool;        // is_vulnerable_side(seat.side())
}
```

`from_board_number` の検算: ボード 1〜16 で `None, NS, EW, Both, NS, EW, Both, None, EW, Both, None, NS, Both, None, NS, EW`。規則書の表と一致する。

### 2.6 `Strain`, `Bid`, `Call`, `Doubling`, `Contract` (`call.rs`)

```rust
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Strain { Clubs = 0, Diamonds = 1, Hearts = 2, Spades = 3, NoTrump = 4 }

impl Strain {
    pub const ALL: [Strain; 5];
    pub const fn from_index(i: u8) -> Strain;
    pub const fn index(self) -> u8;
    pub const fn suit(self) -> Option<Suit>;           // NoTrump → None
    pub const fn from_suit(s: Suit) -> Strain;
    pub const fn is_major(self) -> bool;               // Hearts | Spades
    pub const fn is_minor(self) -> bool;               // Clubs | Diamonds
}

/// 1♣ = 0 … 7NT = 34
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Bid(u8);

impl Bid {
    pub const fn new(level: u8, strain: Strain) -> Option<Bid>;   // level ∉ 1..=7 は None。(level − 1) * 5 + strain
    pub const fn from_index(i: u8) -> Option<Bid>;                 // i >= 35 は None
    pub const fn index(self) -> u8;
    pub const fn level(self) -> u8;                                // index / 5 + 1
    pub const fn strain(self) -> Strain;                           // index % 5
    pub const fn tricks_required(self) -> u8;                      // 6 + level
}
// Bid の Debug は Display に委譲 ("1C")。次のビッドは Bid::from_index(index + 1) で得る。

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Call { Pass, Double, Redouble, Bid(Bid) }

impl Call {
    /// 0 = Pass, 1 = Double, 2 = Redouble, 3 + bid.index() = Bid。トライの索引に使う (0..38)。
    pub const fn index(self) -> u8;
    pub const fn from_index(i: u8) -> Option<Call>;
    pub const fn bid(self) -> Option<Bid>;
    pub const fn is_bid(self) -> bool;
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Doubling { Undoubled, Doubled, Redoubled }

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Contract { pub bid: Bid, pub declarer: Seat, pub doubling: Doubling }

impl Contract {
    pub const fn leader(self) -> Seat;   // declarer.next() (オープニングリーダー)
    pub const fn dummy(self) -> Seat;    // declarer.partner()
}
```

### 2.7 `Auction` (`auction.rs`)

フィールドは非公開。全コンストラクタが合法性を検証するので、不正なオークションは存在しない。

```rust
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Auction { dealer: Seat, vulnerability: Vulnerability, calls: Vec<Call> }

impl Auction {
    pub fn new(dealer: Seat, vulnerability: Vulnerability) -> Auction;
    pub fn from_calls(dealer: Seat, vulnerability: Vulnerability, calls: impl IntoIterator<Item = Call>) -> Result<Auction, AuctionError>;
    pub fn push(&mut self, call: Call) -> Result<(), AuctionError>;
    pub fn with(&self, call: Call) -> Result<Auction, AuctionError>;   // clone + push
    pub fn dealer(&self) -> Seat;
    pub fn vulnerability(&self) -> Vulnerability;
    pub fn calls(&self) -> &[Call];
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn seat_at(&self, index: usize) -> Seat;            // dealer.offset((index % 4) as u8)
    pub fn next_seat(&self) -> Seat;                        // seat_at(len)
    pub fn calls_by(&self, seat: Seat) -> CallsBy<'_>;      // (index, call) をその席のものだけ順に
    pub fn position_of(&self, seat: Seat) -> u8;            // seat がオープンする位置 1..=4 (1 = ディーラー)。#SEAT 条件用
    pub fn legal_calls(&self) -> LegalCalls<'_>;            // Call::index 順 (Pass, X, XX, 1C..7NT)
    pub fn last_bid(&self) -> Option<(usize, Bid)>;
    pub fn last_non_pass(&self) -> Option<(usize, Call)>;
    pub fn leading_passes(&self) -> usize;                  // 先頭の連続パス数 0..=3 (パスアウトは 4)。#SEAT 条件用 (D17)
    pub fn is_legal(&self, call: Call) -> bool;
    pub fn is_complete(&self) -> bool;
    pub fn is_passed_out(&self) -> bool;                    // complete かつ last_bid == None
    pub fn contract(&self) -> Option<Contract>;             // 未完了またはパスアウトなら None
}

/// `Auction::calls_by` のイテレータ (Item = (usize, Call))。
pub struct CallsBy<'a> { auction: &'a Auction, seat: Seat, next: usize }
/// `Auction::legal_calls` のイテレータ (Item = Call)。`next` は次に試す Call::index。
pub struct LegalCalls<'a> { auction: &'a Auction, next: u8 }
```

`position_of(seat)` は `((seat.index() + 4 − dealer.index()) % 4) + 1`。`leading_passes()` が `k` のとき `position_of(seat_at(k)) == k + 1` で、`LookupKey.opener_pos` (`06-system.md`) はこれを使う。

### 2.8 `Deal`, `Board` (`deal.rs`)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Deal { hands: [Hand; 4] }   // 添字 = Seat

impl Deal {
    /// O(1) 検証: 各 len() == 13 かつ 4 手の OR == FULL (⇒ 互いに素)
    pub fn new(hands: [Hand; 4]) -> Result<Deal, DealError>;
    pub const fn hand(&self, seat: Seat) -> Hand;
    pub const fn hands(&self) -> [Hand; 4];
    pub fn owner(&self, card: Card) -> Seat;
}

/// 番号付きボード。フィールドは公開 (純データ)。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Board { pub number: u16, pub dealer: Seat, pub vulnerability: Vulnerability, pub deal: Deal }

impl Board {
    pub fn new(number: u16, deal: Deal) -> Board;   // dealer = Seat::dealer_of_board(n), vul = Vulnerability::from_board_number(n)
    pub fn with_conditions(number: u16, dealer: Seat, vulnerability: Vulnerability, deal: Deal) -> Board;   // PBN の上書き用
}
```

`Deal::random` は `bridge-core` に置かない (依存ゼロを保つ)。乱数配牌は `bridge-sample` の `UniformProposal` が提供する。PBN の `Deal` 先頭席の正規化は `bridge-format` が `Deal::new` に渡す配列の並べ替えで行い、`Deal` に回転 API は持たない。

### 2.9 `PlayHistory`, `Trick`, `Tricks` (`play.rs`)

```rust
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct PlayHistory { trump: Strain, leader: Seat, cards: Vec<Card> }   // 打たれた順。席は index から導出

impl PlayHistory {
    pub fn new(trump: Strain, leader: Seat) -> PlayHistory;
    pub fn trump(&self) -> Strain;
    pub fn leader(&self) -> Seat;                                   // オープニングリーダー
    pub fn cards(&self) -> &[Card];                                 // 枚数は cards().len()、完了は == 52
    pub fn is_legal(&self, card: Card, remaining: Hand) -> bool;    // remaining に含まれ、未使用、フォロー可能なら従う
    pub fn play(&mut self, card: Card, remaining: Hand) -> Result<(), PlayError>;
    pub fn seat_at(&self, index: usize) -> Seat;                    // trick_leader(index / 4).offset((index % 4) as u8)
    pub fn next_to_play(&self) -> Seat;                             // seat_at(cards.len())
    pub fn trick_leader(&self, t: usize) -> Seat;                   // t == 0 → leader、それ以外 trick_winner(t − 1)
    pub fn trick_winner(&self, t: usize) -> Option<Seat>;           // トリック t が未完了なら None
    pub fn current_trick(&self) -> &[Card];                         // 進行中トリックのカード (0..4 枚)
    pub fn led_suit(&self) -> Option<Suit>;                         // 進行中トリックのリードスート
    pub fn tricks(&self) -> Tricks<'_>;                             // 完了トリックと進行中トリックを順に
    pub fn played(&self) -> Hand;                                   // 既出カード全体
    pub fn played_by(&self, seat: Seat) -> Hand;
    pub fn tricks_won(&self, side: Side) -> u8;                     // 完了トリックのみ
}

/// 1 トリックのビュー (完了または進行中)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Trick { pub leader: Seat, pub cards: [Option<Card>; 4], pub winner: Option<Seat> }   // winner は 4 枚揃ったときだけ Some

/// `PlayHistory::tricks` のイテレータ (Item = Trick)。
pub struct Tricks<'a> { history: &'a PlayHistory, next: usize }
```

`seat_at` はリーダーを辿るので O(index / 4)。13 トリック × 約 10 演算で無視できる。席は保存しない (仕様 §3)。プロファイルで問題になった場合のみ `leaders: Vec<Seat>` を派生データとして持ち、`PartialEq` から除外する。

### 2.10 `DdTable` (`dd_table.rs`)

```rust
/// (strain, declarer) ごとのダブルダミートリック数。純データ型。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DdTable { tricks: [[u8; 4]; 5] }   // tricks[strain][declarer]、Strain 順 (♣ … NT)、Seat 順 (N … W)

impl DdTable {
    pub const fn new(tricks: [[u8; 4]; 5]) -> DdTable;
    pub const fn tricks(&self, strain: Strain, declarer: Seat) -> u8;
    pub const fn as_array(&self) -> [[u8; 4]; 5];
    pub fn best_for(&self, declarer: Seat) -> (Strain, u8);   // 最多トリックのストレイン。同点は上位ストレイン (ビッド順で後)
}
```

`bridge-dds` (`CalcDDtable`) が生成し、`bridge-format` が PBN の `OptimumResultTable` セクションから読み、上位アプリが消費する。三者が共通に見える依存ゼロの層はここしかないので `bridge-core` に置き、`bridge-dds` とファサード `bridge::dd` が `pub use bridge_core::DdTable` で再エクスポートする (`03-format.md` §2.2、`10-dds.md` §9)。DDS のストレイン順 (S H D C NT) との軸変換は `bridge-dds::convert` の責務で、この型は常に core 順である。

## 3. 規則

### 3.1 オークションの合法性 (規則書 17〜19 条、全て O(1))

`n = calls.len()`。`last_bid()` と `last_non_pass()` の 2 つの検索で判定する。

1. `is_complete()` ⇔ `n >= 4 && calls[n−3..]` が全て `Pass`。パスアウト (4 パス) と `1C P P P` を共に覆い、`P P P` 単独は未完了。
2. 完了していれば何も合法でない。
3. `Pass` は常に合法。
4. `Bid(b)` は `last_bid()` が `None` か `b > last` のとき合法。
5. `Double` は `last_non_pass() == Some((i, Bid(_)))` かつ `seat_at(i).side() != next_seat().side()` のとき合法。
6. `Redouble` は `last_non_pass() == Some((i, Double))` かつ `seat_at(i).side() != next_seat().side()` のとき合法。

`legal_calls()` は上記を `Pass`、`Double`、`Redouble`、`Bid(last + 1 ..= 7NT)` の順に列挙する。

### 3.2 コントラクトの導出

1. 未完了、または `last_bid() == None` なら `None` (`is_passed_out()` で区別)。
2. `(i, bid) = last_bid()`、`side = seat_at(i).side()`。
3. `doubling` は `last_non_pass()` から: `Double → Doubled`、`Redouble → Redoubled`、`Bid → Undoubled`。
4. `declarer = seat_at(j)`、`j` は `calls[j] == Bid(b')` かつ `b'.strain() == bid.strain()` かつ `seat_at(j).side() == side` を満たす最小の添字。

### 3.3 トリックの勝者

トリック `t` のカードを `c[0..4]`、`led = c[0].suit()` とする。

```
key(card) = (if trump.suit() == Some(card.suit()) { 32 } else if card.suit() == led { 16 } else { 0 }) + card.rank().index()
winner_offset = argmax_j key(c[j])           // カードは相異なるので同点はない
trick_winner(t) = trick_leader(t).offset(winner_offset)
```

NT では `trump.suit() == None` なので 32 は付かない。オフスートは `0..12 < 16` なので決して勝たない。

### 3.4 プレイの合法性

`is_legal(card, remaining)`: (1) `remaining.contains(card)`、(2) `!played().contains(card)`、(3) `led_suit()` が `Some(s)` で `remaining.holding(s)` が空でなければ `card.suit() == s`、(4) 未完了 (`len < 52`)。違反はそれぞれ `PlayError::{NotHeld, AlreadyPlayed, Revoke{led}, Complete}`。

### 3.5 ディーラーとバルネラビリティ

| 項目 | 式 | 例 |
| --- | --- | --- |
| ディーラー | `Seat::from_index(((n + 3) % 4) as u8)` | ボード 1 → N、2 → E、3 → S、4 → W |
| バルネラビリティ | `i = (n + 15) % 16`、`index = (i / 4 + i % 4) % 4` | ボード 1 → None、2 → NS、3 → EW、4 → Both、5 → NS |

## 4. 表示形式 (`fmt` モジュール、`Display` / `FromStr`)

| 型 | `Display` | `FromStr` が受理するもの (lenient) |
| --- | --- | --- |
| `Suit` | `C D H S` | `♣♦♥♠`、小文字も可 |
| `Rank` | `2..9 T J Q K A` | `10`、小文字も可 |
| `Card` | `SA` (スート先行、PBN Play / LIN 形式) | `SA` と `AS` の両方 |
| `Holding` | ランク降順 `AKQ`、ボイドは `""` | 任意の順序、`-` はボイド |
| `Hand` | PBN 順 **S.H.D.C**: `AKQ.234.AKQ.2345`、ボイドは空フィールド | `-` でボイド、ランク順は任意、**13 枚未満を受理** (部分手のため)、重複は `DuplicateCard` |
| `Deal` | `N:AKQ.234.AKQ.2345 <E> <S> <W>` 指定席から時計回り | 任意の先頭席 (`N:`/`E:`/`S:`/`W:`)、`Deal::new` で検証 |
| `Shape` | `5=4=3=1` (**S=H=D=C** 表示順、`=` は順序固定の慣例) | 同形式 |
| `ShapeClass` | `5-4-3-1` | 同形式、順序は任意 (ソートして正規化) |
| `Seat` | `N E S W` | 小文字、`North` 等 |
| `Strain` | `C D H S NT` | `N`、小文字 |
| `Bid` | `1C` … `7NT` | `7N`、小文字 |
| `Call` | `Pass`, `X`, `XX`, `1C` (PBN) | `P`, `D`, `R`, `Dbl`, `Rdbl`、小文字 |
| `Contract` | `4SX` (宣言者は PBN 同様に別タグで出力) | `4SXX`, `3NT`, `4HX` |
| `Vulnerability` | `None NS EW All` (PBN) | `Love`, `Both`, `-`, `NONE`、小文字 |
| `Auction` | コールを空白区切りでディーラーから順に (`1C Pass 1H Pass`)。PBN の 4 コール改行は `bridge-format` の責務 | `FromStr` は提供しない (ディーラーと vul が要るため `Auction::from_calls` を使う) |

保存順は ♣ 先 (ビット配置)、テキストは ♠ 先 (PBN)。この二重の順序を知るのは `fmt` だけで、他のモジュールへスート順を漏らさない。`Debug` は `Card` 以外は `Display` に委譲する。

## 5. エラー型 (`thiserror` 2)

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("unknown suit symbol {0:?}")] Suit(char),
    #[error("unknown rank symbol {0:?}")] Rank(char),
    #[error("unknown seat {0:?}")] Seat(char),
    #[error("expected 4 suits separated by '.', found {0}")] SuitCount(usize),
    #[error("expected 4 hands, found {0}")] HandCount(usize),
    #[error("duplicate card {0}")] DuplicateCard(Card),
    #[error("invalid bid {0:?}")] Bid(String),
    #[error("invalid call {0:?}")] Call(String),
    #[error("invalid vulnerability {0:?}")] Vulnerability(String),
    #[error("invalid shape {0:?}")] Shape(String),
    #[error("empty input")] Empty,
}
// Contract::from_str の失敗は Bid (レベル・ストレイン部) か Call (ダブル部) で報告する。専用 variant は無い。

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DealError {
    #[error("{seat} holds {count} cards, expected 13")] HandSize { seat: Seat, count: u8 },
    #[error("card {0} is held by two seats")] Duplicate(Card),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuctionError {
    #[error("call {call} is illegal at position {index}")] IllegalCall { call: Call, index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlayError {
    #[error("card {0} already played")] AlreadyPlayed(Card),
    #[error("card {0} is not held")] NotHeld(Card),
    #[error("must follow suit {led}")] Revoke { led: Suit },
    #[error("play is complete")] Complete,
}
```

エラーメッセージのために `Card`, `Seat`, `Call`, `Suit` は `Display` を持つ。`ParseError` は `error.rs`、他は各モジュールで定義し、`lib.rs` から再エクスポートする。

## 6. serde の扱い

| 型 | 方法 | human-readable | binary |
| --- | --- | --- | --- |
| `Suit`, `Rank`, `Seat`, `Side`, `Strain`, `Call`, `Doubling`, `Contract`, `Vulnerability`, `DdTable` | `#[cfg_attr(feature = "serde", derive(...))]` | 列挙名 / 構造体 | 同左 |
| `Card`, `Holding`, `Hand`, `Shape`, `Bid` | 手動 impl (`serde_impls.rs`) | `Display` 文字列 (`"SA"`, `"AKQ"`, `"AKQ.234.AKQ.2345"`, `"5=4=3=1"`, `"1C"`) | 生の整数 (`u8`/`u16`/`u64`) |
| `Deal` | 手動 impl | `Display` 文字列 (`"N:… … … …"`) | `[u64; 4]` |
| `ShapeSet` | 手動 impl | `factor()` が `Some` なら 4 つの範囲、クラスの和なら クラス名の列、それ以外は 140 桁の hex ビットマップ | `[u64; 9]` |
| `Auction` | 手動 impl。非公開のミラー構造体 `AuctionRepr { dealer, vulnerability, calls }` を経由し、`Deserialize` は `Auction::from_calls` で再検証 | `{dealer, vulnerability, calls}` | 同左 |
| `ShapeClass` | derive (降順 nibble の `u16`) | | |
| `Board`, `PlayHistory`, `Trick` | 骨格では serde 未対応 | | |

`Deserialize` は必ず検証する: `Card < 52`、`Hand` の bit `>= 52` なし、`Holding` の bit `>= 13` なし、`Deal::new`、`Auction::from_calls` (不正なオークションは構築できない)、`ShapeSet` の bit `>= 560` なし。`is_human_readable()` で分岐する。

`ShapeClass` は `bridge-system` の `BalancedDef { balanced: Vec<ShapeClass>, .. }` が serde 派生で保持するため、骨格の時点で `#[cfg_attr(feature = "serde", derive(...))]` を付けてある。`PlayHistory` の serde は `bridge-format` の `GameView` を直列化する要件が出た時点で `Auction` と同じミラー構造体方式で追加する。

## 7. `const fn` の制約 (stable、edition 2024)

コードの形を決める制約なので列挙する。

| 制約 | 対処 |
| --- | --- |
| `const fn` からトレイト呼び出し不可 (`Iterator`/`for`、`From`/`Into`、演算子トレイト、ユーザー型の `PartialEq`、`Ord::max`) | `while` ループと `match`、`.0` フィールド比較、手書きの `max`/`min` ヘルパ |
| `?` 不可 (`Try` トレイト) | 明示的な `match`。`Option::unwrap`/`expect` と `panic!("literal")` は const 可 |
| heap 不可 (`Vec` 等) | `SHAPES`, `CLASSES`, `CLASS_OF`、評価テーブルは固定長配列を `const fn` で構築し `const`/`static` に束縛 |
| `const trait impl` は unstable | 演算子 (`BitOr` 等) は非 const で inherent const fn に委譲。`#[derive(Default)]` は const でないので `EMPTY` 定数を用意 |
| `long_running_const_eval` (deny-by-default、約 2×10^6 終端子) | 最大ループは `from_class` 系 560×560 と評価テーブル 8192×5 で十分下回る。超えたら `#[allow]` ではなく `const` を分割 |
| edition 2024 で `gen` が予約語 | `rand` 0.10 (`random_range`、`rand::rng()`) を使う。`r#gen` は書かない |
| edition 2024 の RPIT は in-scope lifetime を全て捕捉 | `calls_by(&self, ..) -> impl Iterator` は `self` を暗黙に借用 (意図通り)。名前付きイテレータ型 (`CallsBy` 等) を公開して安定させる |
| `u16::count_ones` / `leading_zeros` / `trailing_zeros`、スライス添字、`as` キャスト | 全て const 可 |

## 8. モジュール構成 (`lib.rs`)

```rust
#![forbid(unsafe_code)]
#![warn(missing_docs)]
// Phase 0 skeleton: the public surface is final, most bodies are `todo!()`.
// Remove these allows as the bodies are implemented.
#![allow(dead_code, unused_variables)]

mod auction;   // Auction, AuctionError, CallsBy, LegalCalls
mod call;      // Bid, Call, Contract, Doubling, Strain
mod card;      // Card, Rank, Suit
mod dd_table;  // DdTable
mod deal;      // Board, Deal, DealError
mod error;     // ParseError
pub mod fmt;   // Display / FromStr の実装と PBN 順の変換
mod hand;      // Hand, HandCards
mod holding;   // Holding, HoldingRanks, Submasks
mod play;      // PlayError, PlayHistory, Trick, Tricks
mod seat;      // Seat, Side, Vulnerability
#[cfg(feature = "serde")]
mod serde_impls;
mod shape;     // CLASSES, CLASS_OF, MAX_HCP, MIN_HCP, SHAPES, Shape, ShapeClass, ShapeSet, ShapeSetIter, shape_index
```

`fmt` だけを `pub mod` にするのは、モジュール doc に `Display`/`FromStr` の受理形式表を置き、`bridge-format` から参照できるようにするため。`lib.rs` は `pub use auction::{Auction, AuctionError, CallsBy, LegalCalls}` で名前付きイテレータも再エクスポートする。フェーズ 0 の `#![allow(dead_code, unused_variables)]` は `todo!()` 本体の未使用引数を黙らせるための暫定で、本体の実装とともに外す (`12-roadmap.md` §1)。`SHAPES` / `CLASSES` / `CLASS_OF` と `ShapeSet` の定数は `const` 評価されるため `todo!()` にできず、骨格の時点で完全に実装済みである。

## 9. 設計上の落とし穴と対処

1. **複集合の補集合はマスクする**: `ShapeSet::complement` は `ALL` と XOR、`Hand::complement` は `FULL` と AND。`from_bits` は範囲外の bit を拒否し、`Deserialize` も検証する。
2. **二つのスート順**: 保存は ♣ 先、テキストは ♠ 先。`fmt` に閉じ込める。`Shape` は `=`、`ShapeClass` は `-` で表示し、取り違えを目で分かるようにする。
3. **`suit_len` 射影は過大近似**: 直積でない `ShapeSet` では `suit_len` は緩い要約に過ぎない。`satisfies` はビット集合を使う。`factor()` が `Some` のときのみ射影が厳密。
4. **シェイプ依存の HCP 上下界**: `max_hcp()`/`min_hcp()` は「6-6 で 25+ HCP」(最大 20) のような矛盾を無料で検出する。絶対上限 37 だけでは何も捕まらない。
5. **`Auction` の不変条件**: `calls` は非公開、全コンストラクタが検証、serde は `from_calls` を経由する。
6. **`is_complete` の端**: `P P P` 未完了、`P P P P` 完了、`1C P P P` 完了。「`n >= 4` かつ末尾 3 つがパス」の単一規則で全て覆う。
7. **NT のトリック勝者**: `trump.suit() == None` は決して一致しない。優先順位はトランプ > リードスート > その他、同じ群ではランク。
8. **`Hand::from_str` は部分手を受理**: 残り手や確定カード部分を表すため。13 枚検証は `Deal::new` だけ。
9. **`Shape::index` は 13 枚限定**: `debug_assert!(total() == 13)`。サンプラーはバケットを元の長さで索引するので `ShapeSet::contains` に部分シェイプは渡らない。
10. **`Bid` と `Call` の整数化**: `Bid::index` は 0..35、`Call::index` は 0..38。トライやテーブルの添字に直接使う。
11. **`Copy` の徹底**: `Deal` (32 バイト) と `Board` も `Copy`。`Auction`/`PlayHistory` は `Vec` を持つので `Clone`。
12. **`Debug` と `Display` の分離**: `Card` の `Debug` だけは `SA` 形式を独自実装 (既存の骨格に合わせる)。他は `Display` へ委譲。

## 10. テストの要点

詳細は `11-testing.md`。本クレート固有のもの:

- シェイプテーブル: `SHAPES.len() == 560`、`CLASSES.len() == 39`、`BALANCED.len() == 28`、`SEMI_BALANCED.len() == 52`、`shape_index` が 0..560 の全単射、`SHAPES[i].index() == i`。
- `MAX_HCP`/`MIN_HCP` を全 8192 ホールディングの総当たりで検算。
- オークション: proptest でランダムな合法オークションのラウンドトリップ (`from_calls(calls()) == self`)、不正コールが `IllegalCall` になる、`contract()` の宣言者が規則通り。
- `Vulnerability::from_board_number` をボード 1〜32 の表と突き合わせ。
- `Display`/`FromStr` ラウンドトリップ (proptest)、部分手の受理、重複の拒否。
- `PlayHistory`: 4 枚ごとの勝者、リボーク検出、`played_by` の整合。

## 11. 決定済み事項

骨格で確定したため未決は無い。`Trick` は `winner: Option<Seat>` を保持する (4 枚揃ったときだけ `Some`)。`Auction::legal_calls` は名前付きイテレータ `LegalCalls<'_>` を返す。
