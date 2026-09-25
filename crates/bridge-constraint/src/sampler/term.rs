//! One prepared DNF term.
//!
//! Two shapes of term are prepared, chosen once in [`PreparedTerm::prepare`]:
//!
//! - the **unconstrained fast path** (§9): a term equal to `Atom::ANY` with no custom literal and
//!   no residual is drawn combinatorially (a partial Fisher-Yates shuffle of the pool), skipping
//!   the shape/HCP machinery entirely;
//! - the **general path** (§6-§8): every literal the exact scheme can express (shape, HCP,
//!   single-suit card requirements, `SuitQuality`, shape-only `DistPoints`/`TotalPoints`, and at
//!   most one additive per-suit feature) narrows a shape/HCP-bucketed enumeration; anything left
//!   over (`Custom`, a DNF residual, `DistMethod::BergenStarting`, or a second additive feature)
//!   is checked by the term's own [`DnfTerm::satisfies`] after each draw, with an acceptance rate
//!   estimated by a burn-in probe.

use core::ops::RangeInclusive;
use std::collections::HashMap;
use std::sync::Arc;

use bridge_core::{Card, Hand, Holding, Shape, Suit};
use bridge_eval::{DistMethod, LtcMethod, SUIT, holding_hcp, shape_points};

use super::rand_util::{SplitMix64, random_below};
use super::suit_table::{
    PAIR_HCP_MAX, PAIR_X_MAX, PairConv, SparseVec, SuitTable, full_suit, pack_key, unpack_key,
};
use crate::{Atom, CardRequirement, DnfTerm, Metric};

/// A DNF term prepared for a fixed pool and fixed part.
pub(crate) struct PreparedTerm {
    /// Size of the exact superset this term contributes (before any rejection check): the number
    /// of hands matching every literal the exact scheme captured. For a term needing rejection
    /// this over-counts the true satisfying set; [`PreparedTerm::alpha`] carries the correction.
    pub(crate) total: u64,
    /// Estimated acceptance rate of the literals the exact scheme could not capture (`None` when
    /// every literal was captured exactly, i.e. every draw is guaranteed to satisfy the term).
    pub(crate) alpha: Option<f64>,
    /// Probability that drawing from this term with up to `opts.max_tries` retries produces
    /// *some* accepted hand: `1 − (1 − alpha)^max_tries` when `alpha.is_some()`, `1.0` for an
    /// exact term (every draw is accepted, so retries never matter). Used as the weight
    /// `Sampler` gives this term in `log_prob`'s normalising constant, since a term whose
    /// `alpha·max_tries` is small enough to often exhaust its retries contributes less to the
    /// distribution `sample` actually returns than its raw `1/alpha` would suggest.
    pub(crate) s: f64,
    /// The source term (for the full `atom ∧ custom ∧ residual` check).
    pub(crate) term: DnfTerm,
    /// `Some` for the unconstrained fast path; `None` for the general path.
    any: Option<AnyTerm>,
    /// `Some` for the general path; `None` for the fast path. Exactly one of `any`/`general` is
    /// ever set.
    general: Option<GeneralTerm>,
}

/// The unconstrained fast path: `pool.len()` cards, `m` of them drawn per hand.
struct AnyTerm {
    cards: Vec<Card>,
    fixed: Hand,
    m: u8,
}

impl AnyTerm {
    fn draw<R: rand_core::Rng + ?Sized>(&self, rng: &mut R) -> Hand {
        let mut cards = self.cards.clone();
        let n = cards.len();
        let m = self.m as usize;
        for i in 0..m {
            let j = i + random_below(rng, (n - i) as u64) as usize;
            cards.swap(i, j);
        }
        let mut hand = self.fixed;
        for &c in &cards[..m] {
            hand = hand.with(c);
        }
        hand
    }
}

/// The general path's per-term state: per-suit tables, the pair convolutions used by some
/// feasible shape, and the feasible shapes themselves with their weights and HCP windows.
struct GeneralTerm {
    suits: [Arc<SuitTable>; 4],
    /// Pair convolutions for `(len_clubs, len_diamonds)` pairs used by some feasible shape.
    pair01: HashMap<(u8, u8), PairConv>,
    /// Pair convolutions for `(len_hearts, len_spades)` pairs used by some feasible shape.
    pair23: HashMap<(u8, u8), PairConv>,
    /// Feasible shapes with their weights and HCP windows.
    shapes: Vec<(Shape, u64, (u8, u8))>,
    cum: Vec<u64>,
    /// The additive-feature window, shared by every shape (it does not depend on the shape).
    x_window: (u8, u8),
}

impl GeneralTerm {
    fn draw<R: rand_core::Rng + ?Sized>(&self, rng: &mut R, total: u64) -> Hand {
        // Step 1 (of §8.1, shape-level): pick a shape proportional to its weight.
        let target = random_below(rng, total);
        let shape_idx = self.cum.partition_point(|&c| c <= target);
        let (shape, weight, (hlo, hhi)) = self.shapes[shape_idx];
        let lens = shape.lens();
        let (l0, l1, l2, l3) = (lens[0], lens[1], lens[2], lens[3]);
        let pair01 = self
            .pair01
            .get(&(l0, l1))
            .expect("built for every feasible shape in `prepare`");
        let pair23 = self
            .pair23
            .get(&(l2, l3))
            .expect("built for every feasible shape in `prepare`");
        let (xlo, xhi) = self.x_window;

        // Step 3: pick `a` (clubs+diamonds combined hcp/x) proportional to
        // `n01(a) * BoxSum23(window - a)`.
        let mut remaining = random_below(rng, weight);
        let mut chosen_a = None;
        for &(key_a, n_a) in &pair01.p.0 {
            let (ah, ax) = unpack_key(key_a);
            let sub_a = pair23.box_sum(
                i64::from(hlo) - i64::from(ah),
                i64::from(hhi) - i64::from(ah),
                i64::from(xlo) - i64::from(ax),
                i64::from(xhi) - i64::from(ax),
            );
            if sub_a == 0 {
                continue;
            }
            let contrib = n_a * sub_a;
            if remaining < contrib {
                chosen_a = Some((ah, ax, sub_a));
                break;
            }
            remaining -= contrib;
        }
        let (ah, ax, sub_a) = chosen_a.expect("weight equals the sum of pair01's contributions");

        // Step 4: pick `b` (hearts+spades combined hcp/x) in the shifted window, proportional to
        // `n23(b)`.
        let h_lo_b = i64::from(hlo) - i64::from(ah);
        let h_hi_b = i64::from(hhi) - i64::from(ah);
        let x_lo_b = i64::from(xlo) - i64::from(ax);
        let x_hi_b = i64::from(xhi) - i64::from(ax);
        let mut remaining_b = random_below(rng, sub_a);
        let mut chosen_b = None;
        for &(key_b, n_b) in &pair23.p.0 {
            let (bh, bx) = unpack_key(key_b);
            if i64::from(bh) < h_lo_b
                || i64::from(bh) > h_hi_b
                || i64::from(bx) < x_lo_b
                || i64::from(bx) > x_hi_b
            {
                continue;
            }
            if remaining_b < n_b {
                chosen_b = Some((bh, bx));
                break;
            }
            remaining_b -= n_b;
        }
        let (bh, bx) = chosen_b.expect("sub_a equals the sum of the matching pair23 entries");

        // Steps 5-6: split each pair's combined (hcp, x) back into its two suits.
        let (h0, x0, h1, x1) = split_pair(
            rng,
            &self.suits[0].counts[l0 as usize],
            &self.suits[1].counts[l1 as usize],
            ah,
            ax,
        );
        let (h2, x2, h3, x3) = split_pair(
            rng,
            &self.suits[2].counts[l2 as usize],
            &self.suits[3].counts[l3 as usize],
            bh,
            bx,
        );

        // Step 6: a uniform holding from each suit's bucket.
        let sub0 = pick_holding(rng, &self.suits[0], l0, h0, x0);
        let sub1 = pick_holding(rng, &self.suits[1], l1, h1, x1);
        let sub2 = pick_holding(rng, &self.suits[2], l2, h2, x2);
        let sub3 = pick_holding(rng, &self.suits[3], l3, h3, x3);

        Hand::from_holdings(
            Holding::from_bits(sub0).expect("stored bits are a valid 13-bit holding"),
            Holding::from_bits(sub1).expect("stored bits are a valid 13-bit holding"),
            Holding::from_bits(sub2).expect("stored bits are a valid 13-bit holding"),
            Holding::from_bits(sub3).expect("stored bits are a valid 13-bit holding"),
        )
    }
}

fn pick_holding<R: rand_core::Rng + ?Sized>(
    rng: &mut R,
    table: &SuitTable,
    len: u8,
    hcp: u8,
    x: u8,
) -> u16 {
    let bucket = table.bucket(len, pack_key(hcp, x));
    bucket[random_below(rng, bucket.len() as u64) as usize]
}

/// Splits a pair's combined `(hcp, x) = (target_h, target_x)` back into its two suits, weighted
/// by `counts_a[key_a] * counts_b[key_b]` over every pair that sums to the target (design §8.1
/// steps 5-6, generalised to two dimensions).
fn split_pair<R: rand_core::Rng + ?Sized>(
    rng: &mut R,
    counts_a: &SparseVec,
    counts_b: &SparseVec,
    target_h: u8,
    target_x: u8,
) -> (u8, u8, u8, u8) {
    let mut total = 0u64;
    for &(key_a, n_a) in &counts_a.0 {
        let (ha, xa) = unpack_key(key_a);
        if ha > target_h || xa > target_x {
            continue;
        }
        if let Some(n_b) = find_count(counts_b, pack_key(target_h - ha, target_x - xa)) {
            total += n_a * n_b;
        }
    }
    let mut r = random_below(rng, total);
    for &(key_a, n_a) in &counts_a.0 {
        let (ha, xa) = unpack_key(key_a);
        if ha > target_h || xa > target_x {
            continue;
        }
        let hb = target_h - ha;
        let xb = target_x - xa;
        if let Some(n_b) = find_count(counts_b, pack_key(hb, xb)) {
            let w = n_a * n_b;
            if r < w {
                return (ha, xa, hb, xb);
            }
            r -= w;
        }
    }
    unreachable!("split_pair: total did not match the sum of its own contributions")
}

fn find_count(counts: &SparseVec, key: u16) -> Option<u64> {
    counts.0.iter().find(|&&(k, _)| k == key).map(|&(_, n)| n)
}

impl PairConv {
    /// Convolves two suits' `(len_a, len_b)` count vectors: for every pair of entries, the
    /// combined `(hcp, x)` accumulates `n_a * n_b`. Also builds the 2-D prefix sums used by
    /// [`PairConv::box_sum`].
    ///
    /// `with_x` is whether the term carries an additive feature at all (K=2); when it does not
    /// (K=1), every entry's `x` is 0 (`SuitTable`'s key was packed with `x = 0` throughout), so
    /// the `x` axis needs only 1 slot instead of the generous `PAIR_X_MAX + 1` upper bound - a
    /// term.rs caller with no additive feature builds `height * 1` arrays here instead of
    /// `height * (PAIR_X_MAX + 1)`, and the nested loops below are `O(height)` instead of
    /// `O(height * (PAIR_X_MAX + 1))`.
    fn build(a: &SuitTable, b: &SuitTable, len_a: u8, len_b: u8, with_x: bool) -> PairConv {
        let width = if with_x { PAIR_X_MAX as usize + 1 } else { 1 };
        let height = PAIR_HCP_MAX as usize + 1;
        let mut acc = vec![0u64; height * width];
        for &(key_a, n_a) in &a.counts[len_a as usize].0 {
            let (ha, xa) = unpack_key(key_a);
            for &(key_b, n_b) in &b.counts[len_b as usize].0 {
                let (hb, xb) = unpack_key(key_b);
                let h = ha as usize + hb as usize;
                let x = xa as usize + xb as usize;
                if h >= height || x >= width {
                    // Both suits together cannot exceed the generous PAIR_HCP_MAX/PAIR_X_MAX
                    // bounds in practice; skip defensively rather than panic.
                    continue;
                }
                acc[h * width + x] += n_a * n_b;
            }
        }

        let mut p = Vec::new();
        for h in 0..height {
            for x in 0..width {
                let n = acc[h * width + x];
                if n > 0 {
                    p.push((pack_key(h as u8, x as u8), n));
                }
            }
        }

        let mut prefix = vec![0u64; height * width];
        for h in 0..height {
            let mut row = 0u64;
            for x in 0..width {
                row += acc[h * width + x];
                let up = if h == 0 {
                    0
                } else {
                    prefix[(h - 1) * width + x]
                };
                prefix[h * width + x] = up + row;
            }
        }

        PairConv {
            p: SparseVec(p),
            prefix: prefix.into_boxed_slice(),
            width,
        }
    }

    /// `Σ_{h' <= hcp_hi, x' <= x_hi} count`, `0` when either bound is negative.
    fn cdf(&self, hcp_hi: i64, x_hi: i64) -> u64 {
        if hcp_hi < 0 || x_hi < 0 {
            return 0;
        }
        let hcp_hi = hcp_hi.min(i64::from(PAIR_HCP_MAX)) as usize;
        let x_hi = x_hi.min(self.width as i64 - 1) as usize;
        self.prefix[hcp_hi * self.width + x_hi]
    }

    /// The count of entries with `hcp` in `[hcp_lo, hcp_hi]` and `x` in `[x_lo, x_hi]` (both
    /// inclusive; either window may extend outside the achievable domain, or be empty, and this
    /// clamps rather than panicking).
    pub(crate) fn box_sum(&self, hcp_lo: i64, hcp_hi: i64, x_lo: i64, x_hi: i64) -> u64 {
        if hcp_hi < hcp_lo || x_hi < x_lo || hcp_hi < 0 || x_hi < 0 {
            return 0;
        }
        let a = i128::from(self.cdf(hcp_hi, x_hi));
        let b = i128::from(self.cdf(hcp_lo - 1, x_hi));
        let c = i128::from(self.cdf(hcp_hi, x_lo - 1));
        let d = i128::from(self.cdf(hcp_lo - 1, x_lo - 1));
        (a - b - c + d).max(0) as u64
    }
}

/// Per-suit card and suit-quality filters, applied exactly while enumerating that suit (§6.2).
#[derive(Default)]
struct SuitFilter {
    cards: Vec<CardRequirement>,
    quality: Option<RangeInclusive<u8>>,
}

impl SuitFilter {
    fn is_trivial(&self) -> bool {
        self.cards.is_empty() && self.quality.is_none()
    }

    fn matches(&self, suit: Suit, h: Holding) -> bool {
        for req in &self.cards {
            let mask_h = req.mask.holding(suit);
            if !req.count.contains(&h.intersect(mask_h).len()) {
                return false;
            }
        }
        if let Some(range) = &self.quality {
            if !range.contains(&SUIT.honors5[h.bits() as usize]) {
                return false;
            }
        }
        true
    }
}

/// The one additive per-suit feature a term's key may carry (§6.1).
enum Additive {
    Controls,
    Losers(LtcMethod),
    QuickTricks,
    /// A multi-suit `CardRequirement`'s mask; its per-suit popcount is the feature.
    Cards(Hand),
}

impl Additive {
    fn suit_value(&self, suit: Suit, h: Holding) -> u8 {
        match self {
            Additive::Controls => suit_controls(h),
            Additive::Losers(LtcMethod::Classic) => SUIT.losers2[h.bits() as usize],
            Additive::Losers(LtcMethod::New) => SUIT.nltc2[h.bits() as usize],
            Additive::QuickTricks => SUIT.qt2[h.bits() as usize],
            Additive::Cards(mask) => h.intersect(mask.holding(suit)).len(),
        }
    }
}

/// Controls of one suit's holding: A = 2, K = 1 (Ace is bit 12, King is bit 11).
fn suit_controls(h: Holding) -> u8 {
    let bits = h.bits();
    2 * ((bits >> 12) & 1) as u8 + ((bits >> 11) & 1) as u8
}

/// How an atom's literals were routed (§6.2): per-suit filters, at most one additive feature, the
/// shape-only `DistPoints`/`TotalPoints` filters and shifts, and whether anything is left over
/// for the full `DnfTerm::satisfies` check.
struct Classified {
    suit_filters: [SuitFilter; 4],
    additive: Option<(Additive, RangeInclusive<u8>)>,
    /// `true` when some literal could not be captured exactly (an extra additive candidate beyond
    /// the one slot, or a `DistMethod::BergenStarting`-based requirement).
    needs_full_check: bool,
    dist_shape_filters: Vec<(DistMethod, RangeInclusive<u8>)>,
    total_shape_shifts: Vec<(DistMethod, RangeInclusive<u8>)>,
}

fn classify(atom: &Atom, opts: &super::SampleOptions) -> Classified {
    let mut suit_filters: [SuitFilter; 4] = Default::default();
    let mut needs_full_check = false;
    let mut additive_candidates: Vec<(Additive, RangeInclusive<u8>)> = Vec::new();
    let mut dist_shape_filters = Vec::new();
    let mut total_shape_shifts = Vec::new();

    for req in &atom.cards {
        match req.single_suit() {
            Some(suit) => suit_filters[suit.index() as usize].cards.push(req.clone()),
            None => additive_candidates.push((Additive::Cards(req.mask), req.count.clone())),
        }
    }
    for req in &atom.eval {
        match req.metric {
            Metric::Controls => additive_candidates.push((Additive::Controls, req.range.clone())),
            Metric::Losers(method) => {
                additive_candidates.push((Additive::Losers(method), req.range.clone()));
            }
            Metric::QuickTricks => {
                additive_candidates.push((Additive::QuickTricks, req.range.clone()));
            }
            Metric::DistPoints(method) => {
                if method.is_shape_only() {
                    dist_shape_filters.push((method, req.range.clone()));
                } else {
                    needs_full_check = true;
                }
            }
            Metric::TotalPoints(method) => {
                if method.is_shape_only() {
                    total_shape_shifts.push((method, req.range.clone()));
                } else {
                    needs_full_check = true;
                }
            }
            Metric::SuitQuality(suit) => {
                suit_filters[suit.index() as usize].quality = Some(req.range.clone());
            }
        }
    }

    let additive = if opts.extra_features >= 1 && !additive_candidates.is_empty() {
        Some(additive_candidates.remove(0))
    } else {
        None
    };
    if !additive_candidates.is_empty() {
        needs_full_check = true;
    }

    Classified {
        suit_filters,
        additive,
        needs_full_check,
        dist_shape_filters,
        total_shape_shifts,
    }
}

fn suit_lens(hand: Hand) -> [u8; 4] {
    [
        hand.holding(Suit::Clubs).len(),
        hand.holding(Suit::Diamonds).len(),
        hand.holding(Suit::Hearts).len(),
        hand.holding(Suit::Spades).len(),
    ]
}

fn shape_weight(pair01: &PairConv, pair23: &PairConv, hlo: u8, hhi: u8, xlo: u8, xhi: u8) -> u64 {
    let mut weight = 0u64;
    for &(key, n) in &pair01.p.0 {
        let (ah, ax) = unpack_key(key);
        let sub = pair23.box_sum(
            i64::from(hlo) - i64::from(ah),
            i64::from(hhi) - i64::from(ah),
            i64::from(xlo) - i64::from(ax),
            i64::from(xhi) - i64::from(ax),
        );
        weight += n * sub;
    }
    weight
}

/// `C(n, k)`, `0` when `k > n`. `C(52, 13) ≈ 6.35×10^11` fits comfortably in `u64` (D13).
fn binomial(n: u64, k: u64) -> u64 {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut result: u128 = 1;
    for i in 0..k {
        result = result * u128::from(n - i) / u128::from(i + 1);
    }
    result as u64
}

/// Folds one more fixed-width value into a running accumulator (`SplitMix64`'s own mixing step,
/// same constants as [`super::rand_util::SplitMix64`]).
///
/// [`burn_in_seed`] uses this instead of `core::hash::Hash` + `DefaultHasher`: the derived `Hash`
/// impl of a `Vec`/slice writes its length via `Hasher::write_usize`, whose byte width is the
/// *host's* pointer width, so on wasm32 (32-bit `usize`) it feeds a different byte stream into
/// the hasher than on a 64-bit host for the exact same logical `atom.cards`/`atom.eval`, and
/// `DefaultHasher::finish` (SipHash) is sensitive to that. That silently broke D12 (a result must
/// not depend on the build target) for any atom with a `cards` or `eval` literal, since
/// `burn_in_seed`'s result feeds `PreparedTerm::prepare` (via the burn-in probe's own RNG seed),
/// which in turn feeds `Sampler::prepare`'s output. `mix` only ever combines values already
/// widened to `u64` by the caller, so no host-word-size ever enters the computation.
fn mix(acc: u64, v: u64) -> u64 {
    let mut z = acc ^ v;
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn mix_range(acc: u64, r: &RangeInclusive<u8>) -> u64 {
    mix(mix(acc, u64::from(*r.start())), u64::from(*r.end()))
}

fn mix_dist_method(acc: u64, m: DistMethod) -> u64 {
    match m {
        DistMethod::ShortSuit {
            void,
            singleton,
            doubleton,
        } => {
            let acc = mix(acc, 0);
            let acc = mix(acc, u64::from(void));
            let acc = mix(acc, u64::from(singleton));
            mix(acc, u64::from(doubleton))
        }
        DistMethod::LongSuit => mix(acc, 1),
        DistMethod::BergenStarting => mix(acc, 2),
    }
}

fn mix_metric(acc: u64, m: Metric) -> u64 {
    match m {
        Metric::Controls => mix(acc, 0),
        Metric::Losers(method) => mix(mix(acc, 1), method as u64),
        Metric::QuickTricks => mix(acc, 2),
        Metric::DistPoints(method) => mix_dist_method(mix(acc, 3), method),
        Metric::TotalPoints(method) => mix_dist_method(mix(acc, 4), method),
        Metric::SuitQuality(suit) => mix(mix(acc, 5), suit as u64),
    }
}

/// A deterministic seed for the burn-in probe, derived from the atom and the pool/fixed split so
/// that [`PreparedTerm::prepare`] stays a pure function of its arguments (and, per D12, the same
/// function regardless of target: see [`mix`]'s doc comment for why `Hash` + `DefaultHasher`
/// cannot be used here).
fn burn_in_seed(atom: &Atom, pool: Hand, fixed: Hand) -> u64 {
    let mut acc = 0u64;
    for word in atom.shapes.words() {
        acc = mix(acc, word);
    }
    acc = mix_range(acc, &atom.hcp);
    acc = mix(acc, atom.cards.len() as u64);
    for req in &atom.cards {
        acc = mix(acc, req.mask.bits());
        acc = mix_range(acc, &req.count);
    }
    acc = mix(acc, atom.eval.len() as u64);
    for req in &atom.eval {
        acc = mix_metric(acc, req.metric);
        acc = mix_range(acc, &req.range);
    }
    acc = mix(acc, pool.bits());
    mix(acc, fixed.bits())
}

impl PreparedTerm {
    pub(crate) fn prepare(
        term: DnfTerm,
        pool: Hand,
        fixed: Hand,
        opts: &super::SampleOptions,
    ) -> PreparedTerm {
        // Unconstrained fast path (§9): drawn combinatorially instead of via the shape/HCP
        // machinery.
        if term.is_exact() && term.atom == Atom::ANY {
            let m = 13u8.saturating_sub(fixed.len());
            let total = binomial(u64::from(pool.len()), u64::from(m));
            let cards: Vec<Card> = pool.cards().collect();
            return PreparedTerm {
                total,
                alpha: None,
                s: 1.0,
                term,
                any: Some(AnyTerm { cards, fixed, m }),
                general: None,
            };
        }

        let classified = classify(&term.atom, opts);
        let needs_full_check =
            classified.needs_full_check || !term.custom.is_empty() || term.residual.is_some();

        // Per-suit tables: shared `FULL_SUIT` for the common "full pool, no filter, no feature"
        // case, otherwise a fresh enumeration.
        let mut suits: [Option<Arc<SuitTable>>; 4] = [None, None, None, None];
        for (i, suit) in Suit::ALL.into_iter().enumerate() {
            let p = pool.holding(suit);
            let f = fixed.holding(suit);
            let filter = &classified.suit_filters[i];
            let additive = &classified.additive;
            let trivial = filter.is_trivial() && additive.is_none();
            let table = if p == Holding::FULL && f == Holding::EMPTY && trivial {
                full_suit()
            } else {
                let filter_fn = |h: Holding| filter.matches(suit, h);
                let key_fn = |h: Holding| -> u16 {
                    let hcp = holding_hcp(h);
                    match additive {
                        Some((feature, _)) => pack_key(hcp, feature.suit_value(suit, h)),
                        None => pack_key(hcp, 0),
                    }
                };
                Arc::new(SuitTable::build(p, f, &filter_fn, &key_fn))
            };
            suits[i] = Some(table);
        }
        let suits: [Arc<SuitTable>; 4] = suits.map(|s| s.expect("every suit was assigned above"));

        let x_window: (u8, u8) = match &classified.additive {
            Some((_, range)) => (*range.start(), *range.end()),
            None => (0, 0),
        };

        let fixed_lens = suit_lens(fixed);
        let pool_lens = suit_lens(pool);

        let mut pair01: HashMap<(u8, u8), PairConv> = HashMap::new();
        let mut pair23: HashMap<(u8, u8), PairConv> = HashMap::new();
        let mut shapes: Vec<(Shape, u64, (u8, u8))> = Vec::new();

        for shape in term.atom.shapes.iter() {
            let lens = shape.lens();
            let feasible =
                (0..4).all(|i| lens[i] >= fixed_lens[i] && lens[i] - fixed_lens[i] <= pool_lens[i]);
            if !feasible {
                continue;
            }

            let dist_ok = classified.dist_shape_filters.iter().all(|(method, range)| {
                let d = shape_points(shape, *method)
                    .expect("classify only routes shape-only methods here")
                    .max(0) as u8;
                range.contains(&d)
            });
            if !dist_ok {
                continue;
            }

            let mut lo = i32::from(*term.atom.hcp.start());
            let mut hi = i32::from(*term.atom.hcp.end());
            for (method, range) in &classified.total_shape_shifts {
                let d = i32::from(
                    shape_points(shape, *method)
                        .expect("classify only routes shape-only methods here"),
                );
                lo = lo.max(i32::from(*range.start()) - d);
                hi = hi.min(i32::from(*range.end()) - d);
            }
            if lo > hi {
                continue;
            }
            let lo = lo.clamp(0, 37) as u8;
            let hi = hi.clamp(0, 37) as u8;
            if lo > hi {
                continue;
            }

            let (l0, l1, l2, l3) = (lens[0], lens[1], lens[2], lens[3]);
            let with_x = classified.additive.is_some();
            let p01 = pair01
                .entry((l0, l1))
                .or_insert_with(|| PairConv::build(&suits[0], &suits[1], l0, l1, with_x));
            let p23 = pair23
                .entry((l2, l3))
                .or_insert_with(|| PairConv::build(&suits[2], &suits[3], l2, l3, with_x));

            let weight = shape_weight(p01, p23, lo, hi, x_window.0, x_window.1);
            if weight > 0 {
                shapes.push((shape, weight, (lo, hi)));
            }
        }

        let mut cum = Vec::with_capacity(shapes.len());
        let mut running = 0u64;
        for &(_, w, _) in &shapes {
            running += w;
            cum.push(running);
        }
        let total = running;

        let mut prepared = PreparedTerm {
            total,
            alpha: None,
            s: 1.0,
            term,
            any: None,
            general: Some(GeneralTerm {
                suits,
                pair01,
                pair23,
                shapes,
                cum,
                x_window,
            }),
        };

        if needs_full_check {
            let alpha = if total == 0 {
                // The exact superset is already empty, so this term never contributes a sample
                // (`Sampler::sample` only ever picks a term with `total > 0`, and
                // `Sampler::log_prob` skips terms with `term.total == 0` before looking at
                // `alpha`); the exact value is unobservable and unused.
                0.0
            } else {
                let seed = burn_in_seed(&prepared.term.atom, pool, fixed);
                let mut rng = SplitMix64::new(seed);
                let burn_in = opts.burn_in.max(1);
                let mut hits = 0u32;
                for _ in 0..burn_in {
                    let hand = prepared.draw(&mut rng);
                    if prepared.term.satisfies(hand) {
                        hits += 1;
                    }
                }
                // Jeffreys estimate (never exactly zero, even when the burn-in probe finds no
                // hit): the true acceptance rate of a narrow rejection literal can be well below
                // `1/burn_in`, in which case a plain `hits/burn_in` estimate is `0.0`. `sample`
                // does not skip such a term (it can still find an accepting hand within
                // `max_tries`), so a stored `0.0` would make `log_prob` divide by an estimate of
                // zero. `Sampler::any_definitely_satisfiable` deliberately does not read `alpha`
                // at all, for the same reason (see its doc comment).
                (f64::from(hits) + 0.5) / (f64::from(burn_in) + 1.0)
            };
            // `s`: the probability that up to `opts.max_tries` retries within this term produce
            // *some* accepted hand. `sample_deals`-style callers redraw on `None`, so the
            // distribution `Sampler::sample` actually returns is conditioned on success; `log_prob`
            // must weight each term by `s`, not treat every term as if `max_tries` were infinite
            // (see `PreparedTerm::s`'s doc comment).
            let max_tries = f64::from(opts.max_tries.max(1));
            prepared.s = 1.0 - (1.0 - alpha).powf(max_tries);
            prepared.alpha = Some(alpha);
        }

        prepared
    }

    /// Draws one hand from this term (before residual checks).
    pub(crate) fn draw<R: rand_core::Rng + ?Sized>(&self, rng: &mut R) -> Hand {
        if let Some(any) = &self.any {
            return any.draw(rng);
        }
        self.general
            .as_ref()
            .expect("prepare always sets exactly one of `any`/`general`")
            .draw(rng, self.total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suit_table_hcp_only(pool: Holding) -> SuitTable {
        SuitTable::build(pool, Holding::EMPTY, &|_| true, &|h| {
            u16::from(holding_hcp(h))
        })
    }

    /// `PairConv::build`'s `with_x: false` path (`width == 1`, used whenever a term carries no
    /// additive feature) must give exactly the same `box_sum` results a real K=1 caller can ever
    /// observe as the always-full-width (`with_x: true`) path would - every such caller only ever
    /// queries with `x_lo == x_hi == 0` (`GeneralTerm`'s `x_window` is `(0, 0)` with no additive
    /// feature), matching the fact that a K=1 `SuitTable`'s key packs `x = 0` throughout, so the
    /// reduced-width table drops no reachable entry.
    #[test]
    fn narrow_width_matches_the_full_width_reference_at_x_0() {
        let a = suit_table_hcp_only(Holding::from_bits(0b0111_1110_0000).unwrap());
        let b = suit_table_hcp_only(Holding::from_bits(0b0000_0001_1111).unwrap());

        for len_a in 0..=7u8 {
            for len_b in 0..=5u8 {
                let narrow = PairConv::build(&a, &b, len_a, len_b, false);
                let full = PairConv::build(&a, &b, len_a, len_b, true);
                assert_eq!(narrow.width, 1);
                assert_eq!(full.width, PAIR_X_MAX as usize + 1);
                // The sparse (key, count) list itself must match exactly: every real entry has
                // x = 0 in both tables, so nothing the full-width build finds is dropped.
                assert_eq!(narrow.p, full.p);
                for hlo in 0..=PAIR_HCP_MAX {
                    for hhi in hlo..=PAIR_HCP_MAX {
                        assert_eq!(
                            narrow.box_sum(i64::from(hlo), i64::from(hhi), 0, 0),
                            full.box_sum(i64::from(hlo), i64::from(hhi), 0, 0),
                            "len_a={len_a} len_b={len_b} hlo={hlo} hhi={hhi}"
                        );
                    }
                }
            }
        }
    }

    /// A term with an additive feature (K=2, `with_x: true`) must still convolve and query
    /// correctly across the full `x` axis, not just at `x = 0`: build one suit table with a
    /// synthetic per-holding `x` feature (card count, like `Additive::Cards`) and check `box_sum`
    /// against a brute-force count over every pair of holdings.
    #[test]
    fn with_x_true_matches_brute_force_over_the_full_x_axis() {
        let key_with_len_feature =
            |h: Holding| pack_key(holding_hcp(h), h.len().min(PAIR_X_MAX / 2));
        let a = SuitTable::build(
            Holding::from_bits(0b0111_1110_0000).unwrap(),
            Holding::EMPTY,
            &|_| true,
            &key_with_len_feature,
        );
        let b = SuitTable::build(
            Holding::from_bits(0b0000_0001_1111).unwrap(),
            Holding::EMPTY,
            &|_| true,
            &key_with_len_feature,
        );

        for len_a in 0..=7u8 {
            for len_b in 0..=5u8 {
                let pair = PairConv::build(&a, &b, len_a, len_b, true);
                assert_eq!(pair.width, PAIR_X_MAX as usize + 1);

                for hlo in [0u8, 3, 10] {
                    for hhi in [hlo, hlo + 2, PAIR_HCP_MAX] {
                        for xlo in [0u8, 2] {
                            for xhi in [xlo, xlo + 3, PAIR_X_MAX] {
                                let mut expected = 0u64;
                                for &(key_a, n_a) in &a.counts[len_a as usize].0 {
                                    let (ha, xa) = unpack_key(key_a);
                                    for &(key_b, n_b) in &b.counts[len_b as usize].0 {
                                        let (hb, xb) = unpack_key(key_b);
                                        let h = ha + hb;
                                        let x = xa + xb;
                                        if (hlo..=hhi).contains(&h) && (xlo..=xhi).contains(&x) {
                                            expected += n_a * n_b;
                                        }
                                    }
                                }
                                assert_eq!(
                                    pair.box_sum(
                                        i64::from(hlo),
                                        i64::from(hhi),
                                        i64::from(xlo),
                                        i64::from(xhi)
                                    ),
                                    expected,
                                    "len_a={len_a} len_b={len_b} hlo={hlo} hhi={hhi} xlo={xlo} xhi={xhi}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// `burn_in_seed` must be a pure, fixed-width function of its arguments (D12): pinning its
    /// output for a specific atom/pool/fixed catches any accidental reintroduction of
    /// `core::hash::Hash` + `DefaultHasher` (whose `Vec`/slice impl hashes each length through
    /// `Hasher::write_usize`, i.e. through the host's own pointer width, and so would give this
    /// exact atom a different seed on a 32-bit target like wasm32 than on a 64-bit host, even
    /// though `atom`/`pool`/`fixed` are logically identical there).
    #[test]
    fn burn_in_seed_is_pinned_for_a_fixed_atom() {
        let atom = Atom {
            cards: vec![
                CardRequirement {
                    mask: Hand::from_bits(0b11).unwrap(), // ♣A, ♣K: single-suit, not additive.
                    count: 1..=2,
                },
                CardRequirement {
                    // ♣A + ♦A: spans two suits, so this one competes for the additive-feature
                    // slot (see `classify`).
                    mask: Hand::from_bits(0b1 | (0b1 << 13)).unwrap(),
                    count: 1..=2,
                },
            ],
            ..Atom::ANY
        };
        let seed = burn_in_seed(&atom, Hand::FULL, Hand::EMPTY);
        assert_eq!(seed, 0x53c8_d139_227b_d64e);

        // Changing any single fixed-width input changes the seed (the mix isn't accidentally
        // discarding a field, e.g. by mixing in a `Vec::len()` that std would coalesce to the
        // same value some other way).
        let mut other = atom.clone();
        other.cards[0].count = 1..=1;
        assert_ne!(burn_in_seed(&other, Hand::FULL, Hand::EMPTY), seed);
        assert_ne!(
            burn_in_seed(&atom, Hand::FULL, Hand::from_bits(0b100).unwrap()),
            seed
        );
    }

    /// End-to-end pin: for an atom whose two multi-suit `cards` literals force `needs_full_check`
    /// (only one fits the additive-feature budget), `PreparedTerm::prepare`'s burn-in-derived
    /// `alpha` is a deterministic function of `burn_in_seed`'s output. Pinning it here means any
    /// future change to `burn_in_seed` (including reverting to a `Hash`-based one) that alters
    /// the seed for this atom would change this literal, not just some internal, untested detail.
    #[test]
    fn rejection_term_alpha_is_pinned_for_a_fixed_atom() {
        let atom = Atom {
            cards: vec![
                CardRequirement {
                    mask: Hand::from_bits(0b1 | (0b1 << 13)).unwrap(), // ♣A + ♦A
                    count: 1..=2,
                },
                CardRequirement {
                    mask: Hand::from_bits((0b1 << 26) | (0b1 << 39)).unwrap(), // ♥A + ♠A
                    count: 1..=2,
                },
            ],
            ..Atom::ANY
        };
        let term = DnfTerm {
            atom,
            custom: Vec::new(),
            residual: None,
        };
        let opts = super::super::SampleOptions::default();
        let prepared = PreparedTerm::prepare(term, Hand::FULL, Hand::EMPTY, &opts);
        let alpha = prepared
            .alpha
            .expect("two multi-suit card literals force needs_full_check");
        assert!((0.0..=1.0).contains(&alpha));
        assert!(
            (alpha - 0.429_961_089_494_163_45).abs() < 1e-12,
            "alpha = {alpha}, expected a value pinned to burn_in_seed's current output"
        );
    }
}
