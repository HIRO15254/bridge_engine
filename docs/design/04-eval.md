# 04. `bridge-eval` (L1): ハンド評価

本文書は `bridge-eval` の評価関数を確定する。全手の線形指標 (HCP、コントロール、各オナー枚数) はテーブルを使わずビット演算で求め、スート単位の非線形指標 (LTC、クイックトリック、オナー数) は `const fn` で構築した 8192 要素の `static` テーブルで引く。半単位の値は `Half(u8)`、分布点は `DistMethod` で流派を切り替え、Bergen の adjust-3 が負になるため戻り値は `i8` とする (D13)。依存は `bridge-core` のみ。

## 1. 責務と非責務

| 含む | 含まない |
| --- | --- |
| 純粋関数としての評価 (`Hand`/`Holding` → 数値) | フィットに依存するダミー点 (`dummy_points(hand, trump, method)` として後日追加、`distribution_points` には混ぜない) |
| `bridge-constraint` のサンプラーが使うスート別テーブル (`SUIT`) | 手の補正評価 (アップグレード/ダウングレード)。`SystemMeta` の宣言として L2 に置く |
| `DistMethod` / `LtcMethod` の宣言 (システム定義が前提を宣言する) | ビディング判断 |

全関数は `Send + Sync` な純粋関数で、内部状態を持たない。

## 2. `Half`: 半単位の値

```rust
/// value = halves / 2。内部 u8 の Ord/Eq 導出がそのまま正しい。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct Half(u8);

impl Half {
    pub const ZERO: Half = Half(0);
    pub const fn from_halves(n: u8) -> Half;     // Half(n)
    pub const fn from_whole(n: u8) -> Half;      // Half(n * 2)
    pub const fn halves(self) -> u8;
    pub const fn whole(self) -> u8;              // 切り捨て
    pub const fn is_half(self) -> bool;          // 奇数
    pub const fn as_f32(self) -> f32;            // halves * 0.5
    pub const fn add(self, other: Half) -> Half;  // 飽和加算
}
impl Add for Half; impl Sum for Half;
impl Display for Half;   // "2", "2.5" (ASCII。"½" はオプトインのフォーマッタのみ)
```

## 3. 全手の線形指標 (テーブル不要)

ランクごとの 4 スート分マスクと `popcount` だけで求める。全て `const fn` で 2〜3 ns。

```rust
pub const fn rank_mask(r: Rank) -> u64 { let b = 1u64 << r.index(); b | b << 13 | b << 26 | b << 39 }
pub const ACES: u64 = rank_mask(Rank::Ace);
pub const KINGS: u64 = rank_mask(Rank::King);
pub const QUEENS: u64 = rank_mask(Rank::Queen);
pub const JACKS: u64 = rank_mask(Rank::Jack);
pub const TENS: u64 = rank_mask(Rank::Ten);

pub const fn hcp(h: Hand) -> u8 {
    let b = h.bits();
    (4 * (b & ACES).count_ones() + 3 * (b & KINGS).count_ones() + 2 * (b & QUEENS).count_ones() + (b & JACKS).count_ones()) as u8
}
pub const fn controls(h: Hand) -> u8 { let b = h.bits(); (2 * (b & ACES).count_ones() + (b & KINGS).count_ones()) as u8 }
pub const fn aces(h: Hand) -> u8;    // (b & ACES).count_ones()
pub const fn kings(h: Hand) -> u8;
pub const fn queens(h: Hand) -> u8;
pub const fn jacks(h: Hand) -> u8;
pub const fn tens(h: Hand) -> u8;
pub const fn holding_hcp(h: Holding) -> u8 { SUIT.hcp[h.bits() as usize] }   // スート表の参照 (§4)
```

| 指標 | 式 | 全手の最大 | コスト |
| --- | --- | --- | --- |
| `hcp` | `4·A + 3·K + 2·Q + J` | 37 | 4 popcnt + 3 積和、2〜3 ns |
| `controls` | `2·A + K` | 12 | 2 popcnt |
| `aces` … `tens` | 1 popcnt | 4 | 1 ns |

## 4. スート別テーブル `SUIT` (`const` 構築)

```rust
pub struct SuitTables {
    pub hcp: [u8; 8192],      // サンプラーのバケット構築用 (holding_hcp と同値)
    pub losers2: [u8; 8192],  // Classic LTC × 2
    pub nltc2: [u8; 8192],    // New LTC × 2
    pub qt2: [u8; 8192],      // クイックトリック × 2
    pub honors5: [u8; 8192],  // A K Q J T の枚数
}
impl SuitTables { const fn build() -> SuitTables; }       // 非公開。while ループ、8192 × 5 回
pub static SUIT: SuitTables = SuitTables::build();        // 約 40 KB を .rodata に置く
```

添字は `Holding::bits()` そのもの (13 bit = 8192 通り)。`const fn` で構築して `static` に束縛する。`static` は `todo!()` にできないので、`SUIT` はフェーズ 0 の骨格の時点で完全に実装済みである (`tables.rs` の spot-check テスト付き)。`LazyLock` は毎アクセスに初期化チェックが入り 40 KB の計算を初回使用時に回すだけで利点がなく、`build.rs` は生成ファイルと式の二重化と `OUT_DIR` の配線を増やすだけなので採用しない。const 評価が遅すぎるテーブルが現れた場合のみ `build.rs` へ移す。

### 4.1 スート別の式

`l = len`、`A/K/Q` は所持ビット、`[cond]` は 0/1。8192 通りを素朴実装と突き合わせて検証する。

| 指標 | スート別の式 | スート最大 | 全手最大 |
| --- | --- | --- | --- |
| Classic LTC (`losers2 / 2`) | `l == 0 → 0; else min(l, 3) − A − [K && l >= 2] − [Q && l >= 3]` | 3 | 12 |
| New LTC ×2 (`nltc2`) | `[l >= 1 && !A]·3 + [l >= 2 && !K]·2 + [l >= 3 && !Q]·1` (A 欠け 1.5、K 欠け 1.0、Q 欠け 0.5) | 6 | 24 halves |
| クイックトリック ×2 (`qt2`) | `AK → 4, AQ → 3, A → 2, KQ → 2, Kx (l >= 2) → 1, else 0` | 4 | 16 halves |
| コントロール | `2·A + K` (テーブル不要) | 3 | 12 |
| `honors5` | `popcount(holding ∩ top_ranks(5))` | 5 | 13 (13 枚の手) |
| `suit_quality(h)` | `= honors5[h]` (「トップ 5 のうち 3 枚」判定に使う) | 5 | |
| `top_honors(h, n)` | `popcount(h ∩ top_ranks(n))` | n | |

### 4.2 検算例

| ホールディング | Classic LTC | New LTC | QT | honors5 |
| --- | --- | --- | --- | --- |
| `AKx` | `3 − 1 − 1 − 0 = 1` | Q 欠け 0.5 | 2 | 2 |
| `AQx` | `3 − 1 − 0 − 1 = 1` | K 欠け 1.0 | 1.5 | 2 |
| `Qxx` | `3 − 0 − 0 − 1 = 2` | A 欠け 1.5 + K 欠け 1.0 = 2.5 | 0 | 1 |
| `Kx` | `2 − 0 − 1 = 1` | A 欠け 1.5 | 0.5 | 1 |
| `K` (シングルトン) | `1 − 0 − 0 = 1` (K は `l >= 2` でないので数えない) | A 欠け 1.5 | 0 | 1 |
| `x` | 1 | 1.5 | 0 | 0 |
| ボイド | 0 | 0 | 0 | 0 |
| `AKQJT` | 0 | 0 | 2 | 5 |

### 4.3 公開関数

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LtcMethod { Classic, New }

pub fn losers(h: Hand) -> Half;                         // Σ SUIT.losers2 (Classic)
pub fn losers_with(h: Hand, m: LtcMethod) -> Half;      // Classic → losers2、New → nltc2
pub fn quick_tricks(h: Hand) -> Half;                   // Σ SUIT.qt2
pub const fn honors(h: Holding) -> u8;                  // SUIT.honors5[h] (A K Q J T の枚数)
pub const fn suit_quality(h: Holding) -> u8;            // = honors(h)
pub const fn top_honors(h: Holding, n: u8) -> u8;       // popcount(h ∩ top_ranks(n))
```

全手の値は 4 回のテーブル参照 (L1 キャッシュ) の和で 4〜8 ns。スート単位の LTC / QT は `SUIT.losers2[h]` 等を直接引く (専用の公開関数は置かない)。`LtcMethod` は `DistMethod` と同じ `dist.rs` にある。

## 5. 分布点 `DistMethod`

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DistMethod {
    /// Σ_s [l = 0]·void + [l = 1]·singleton + [l = 2]·doubleton
    ShortSuit { void: u8, singleton: u8, doubleton: u8 },
    /// Σ_s max(0, l − 4)
    LongSuit,
    /// Bergen "Starting Points": LongSuit + Σ_s [l >= 4 && honors5 >= 3] + adjust3
    /// adjust3 = +1 if (aces + tens) − (queens + jacks) >= 3, −1 if <= −3, else 0
    BergenStarting,
}

impl DistMethod {
    pub const GOREN_321: DistMethod = DistMethod::ShortSuit { void: 3, singleton: 2, doubleton: 1 };
    pub const DUMMY_531: DistMethod = DistMethod::ShortSuit { void: 5, singleton: 3, doubleton: 1 };
    /// Shape だけの関数なら true (ShortSuit, LongSuit)。サンプラーが利用する。
    pub const fn is_shape_only(self) -> bool;
}

pub fn distribution_points(h: Hand, m: DistMethod) -> i8;      // Bergen adjust-3 が −1 になり得るため i8
pub fn shape_points(s: Shape, m: DistMethod) -> Option<i8>;    // is_shape_only のときのみ Some
pub fn total_points(h: Hand, m: DistMethod) -> u8;             // (hcp as i8 + distribution_points).max(0) as u8
```

| 方式 | 式 | Shape のみ | 最小 | 最大 (13 枚) |
| --- | --- | --- | --- | --- |
| `ShortSuit{3,2,1}` (`GOREN_321`) | `3·void + 2·singleton + 1·doubleton` | ○ | 0 | 9 (13-0-0-0) |
| `ShortSuit{5,3,1}` (`DUMMY_531`) | `5·void + 3·singleton + 1·doubleton` | ○ | 0 | 15 |
| `LongSuit` | `Σ max(0, l − 4)` | ○ | 0 | 9 (13-0-0-0) |
| `BergenStarting` | `LongSuit + 品質スート数 + adjust3` | × (オナーに依存) | −1 | 9 + 1 + 1 = 11 (品質スートは長さ 4 以上に限られ、13-0-0-0 では 1 つ) |

仕様の `distribution_points -> u8` は Bergen の adjust-3 を表現できないため `i8` に変える (D13)。`total_points` は 0 で飽和させて `u8` を返す。

`is_shape_only()` の意味: 値が `Hand::shape()` だけで決まる。サンプラーは `ShortSuit`/`LongSuit` の `DistPoints`/`TotalPoints` 要件をシェイプの絞り込みと HCP 窓のシフトで厳密に扱い、`BergenStarting` は追加特徴 (K=3) または棄却で扱う (`05-constraint.md` §7)。

## 6. 制約側が使う指標の値域

`bridge-constraint` の `Metric` は次の値域でクランプする。全て本クレートの定数から導く。

| `Metric` | 単位 | 値域 | 由来 |
| --- | --- | --- | --- |
| `Controls` | 1 | `0..=12` | `2·4 + 4` |
| `Losers(Classic)` | half | `0..=24` | スート最大 3 × 4 |
| `Losers(New)` | half | `0..=24` | スート最大 6 × 4 |
| `QuickTricks` | half | `0..=16` | スート最大 4 × 4 |
| `DistPoints(m)` | 1 | `0..=40` (方式によらない固定上限。負は 0 にクランプ) | `Metric::max` |
| `TotalPoints(m)` | 1 | `0..=77` (`37 + 40`) | `Metric::max` |
| `SuitQuality(s)` | 1 | `0..=5` | `honors5` |

`Metric::max` は方式別の最大値 (上表の 9 / 15 / 11) ではなく `DistMethod` に依らない上限 40 を返す。方式別の実際の最大はサンプラーが `shape_points` の走査で自然に得るので、上限は範囲のクランプにしか使わない。

サンプラーの鍵の詰め込み (`key = hcp | (x << 6)`) では追加特徴 `x` のスート別最大が 6 以下、全手で `4·x_max <= 24 < 64` なので、詰めた加算が成分ごとの加算に一致する。

## 7. 性能目標と測定

| 操作 | 目標 | 試算 | 測定 |
| --- | --- | --- | --- |
| `hcp(hand)` | < 10 ns (仕様 §9) | 2〜3 ns | `criterion` (`benches/eval.rs`) |
| `controls(hand)` | < 10 ns | 2〜3 ns | 同上 |
| `losers(hand)`, `quick_tricks(hand)` | < 20 ns | 4〜8 ns (4 回の L1 ロード) | 同上 |
| `distribution_points(hand, m)` | < 20 ns | `shape()` 4 popcnt + 4 分岐 | 同上 |
| `SUIT` のサイズ | | 5 × 8192 = 40,960 バイト | `size_of::<SuitTables>()` の静的アサート |
| コンパイル時間 | const 評価が `long_running_const_eval` に掛からない | 8192 × 5 ループ (上限の約 2×10^6 終端子を大きく下回る) | CI |

テスト: 全 8192 ホールディングで `SUIT` の各列を素朴実装 (ランクを 1 枚ずつ数える) と完全一致させる差分テスト。全手の `hcp` は `Σ holding_hcp` と一致。`total_points` の飽和。`shape_points` を 560 シェイプで総当たりし `distribution_points` と一致させる (shape-only 方式)。

## 8. モジュール構成

```
bridge-eval/src/
├── lib.rs        // 再エクスポート (bridge_core::{Hand, Holding, Shape} も)、#![forbid(unsafe_code)]
├── half.rs       // Half
├── metrics.rs    // rank_mask, ACES.., hcp, holding_hcp, controls, aces..tens, losers, losers_with, quick_tricks, honors, top_honors, suit_quality
├── tables.rs     // SuitTables, SUIT
└── dist.rs       // DistMethod, LtcMethod, distribution_points, shape_points, total_points
```

feature: `default = ["std"]`, `std`, `serde` (`bridge-core/serde` を伝播)。依存は `bridge-core` と `serde` (optional) のみで、`thiserror` は使わない (エラー型が無い)。

## 9. 未決

- 未決: `dummy_points(hand, trump: Suit, method)` (フィット確定後の短スート点、トランプ長の上限付き) の式はフェーズ 5 のプレイ側実装まで保留する。
- 未決: `Half` の `Display` で `"½"` を出すオプトイン API の形 (フォーマッタ型か `fmt` フラグか) は UI 要件が出てから決める。
