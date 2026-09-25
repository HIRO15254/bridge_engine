//! Differential tests: every table-driven metric against an independent, loop-based
//! implementation, plus hand-written spot checks transcribed from `docs/design/04-eval.md`.

use bridge_core::{Card, Hand, Holding, Rank, Shape, Suit};
use bridge_eval::{
    DistMethod, Half, LtcMethod, SUIT, aces, controls, distribution_points, hcp, holding_hcp,
    honors, jacks, kings, losers, losers_with, queens, quick_tricks, shape_points, suit_quality,
    tens, top_honors, total_points,
};

// ---------------------------------------------------------------------------------------------
// A tiny, dependency-free PRNG (splitmix64) for pseudo-random hands.
// ---------------------------------------------------------------------------------------------

struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        SplitMix64(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..bound` (slightly biased for non-power-of-two bounds, irrelevant here).
    fn next_bound(&mut self, bound: u32) -> u32 {
        (self.next_u64() % bound as u64) as u32
    }
}

/// A pseudo-random 13-card hand via Fisher-Yates over the 52 cards.
fn random_hand(rng: &mut SplitMix64) -> Hand {
    let mut cards: Vec<u8> = (0..52).collect();
    for i in (1..cards.len()).rev() {
        let j = rng.next_bound((i + 1) as u32) as usize;
        cards.swap(i, j);
    }
    let mut bits = 0u64;
    for &c in &cards[..13] {
        bits |= Card::from_index(c).unwrap().bit();
    }
    Hand::from_bits(bits).unwrap()
}

// ---------------------------------------------------------------------------------------------
// Independent, loop-based per-suit reference implementations (§4.1 of the design doc).
// ---------------------------------------------------------------------------------------------

fn naive_hcp(h: Holding) -> u8 {
    Rank::ALL
        .into_iter()
        .filter(|&r| h.contains(r))
        .map(Rank::hcp)
        .sum()
}

fn naive_losers2(h: Holding) -> u8 {
    let len = h.len();
    if len == 0 {
        return 0;
    }
    let a = h.contains(Rank::Ace);
    let k = h.contains(Rank::King);
    let q = h.contains(Rank::Queen);
    let base = len.min(3);
    let mut losers = base;
    if a {
        losers -= 1;
    }
    if k && len >= 2 {
        losers -= 1;
    }
    if q && len >= 3 {
        losers -= 1;
    }
    losers * 2
}

fn naive_nltc2(h: Holding) -> u8 {
    let len = h.len();
    let a = h.contains(Rank::Ace);
    let k = h.contains(Rank::King);
    let q = h.contains(Rank::Queen);
    let mut halves = 0u8;
    if len >= 1 && !a {
        halves += 3;
    }
    if len >= 2 && !k {
        halves += 2;
    }
    if len >= 3 && !q {
        halves += 1;
    }
    halves
}

fn naive_qt2(h: Holding) -> u8 {
    let len = h.len();
    let a = h.contains(Rank::Ace);
    let k = h.contains(Rank::King);
    let q = h.contains(Rank::Queen);
    if a && k {
        4
    } else if a && q {
        3
    } else if a || (k && q) {
        2
    } else if k && len >= 2 {
        1
    } else {
        0
    }
}

fn naive_honors5(h: Holding) -> u8 {
    [Rank::Ace, Rank::King, Rank::Queen, Rank::Jack, Rank::Ten]
        .into_iter()
        .filter(|&r| h.contains(r))
        .count() as u8
}

/// Whole-hand HCP, computed by summing `Rank::hcp()` over every held card (independent of the
/// bit-mask/table paths in `bridge-eval`).
fn naive_hand_hcp(hand: Hand) -> u8 {
    Suit::ALL
        .into_iter()
        .map(|s| naive_hcp(hand.holding(s)))
        .sum()
}

fn naive_hand_controls(hand: Hand) -> u8 {
    Suit::ALL
        .into_iter()
        .map(|s| {
            let h = hand.holding(s);
            2 * h.contains(Rank::Ace) as u8 + h.contains(Rank::King) as u8
        })
        .sum()
}

fn naive_hand_losers(hand: Hand, method: LtcMethod) -> Half {
    let mut halves = 0u8;
    for s in Suit::ALL {
        halves += match method {
            LtcMethod::Classic => naive_losers2(hand.holding(s)),
            LtcMethod::New => naive_nltc2(hand.holding(s)),
        };
    }
    Half::from_halves(halves)
}

fn naive_hand_quick_tricks(hand: Hand) -> Half {
    let halves: u8 = Suit::ALL
        .into_iter()
        .map(|s| naive_qt2(hand.holding(s)))
        .sum();
    Half::from_halves(halves)
}

fn naive_honor_count(hand: Hand, rank: Rank) -> u8 {
    Suit::ALL
        .into_iter()
        .filter(|&s| hand.holding(s).contains(rank))
        .count() as u8
}

fn naive_top_honors(h: Holding, n: u8) -> u8 {
    Holding::top_ranks(n)
        .ranks()
        .filter(|&r| h.contains(r))
        .count() as u8
}

fn naive_suit_quality(h: Holding) -> u8 {
    naive_honors5(h)
}

/// Independent per-suit distribution-point helpers, mirroring §5 of the design doc but written
/// against `Shape::len` directly rather than the production `dist.rs` code path.
fn naive_short_suit(shape: Shape, void: u8, singleton: u8, doubleton: u8) -> i8 {
    Suit::ALL
        .into_iter()
        .map(|s| match shape.len(s) {
            0 => void as i8,
            1 => singleton as i8,
            2 => doubleton as i8,
            _ => 0,
        })
        .sum()
}

fn naive_long_suit(shape: Shape) -> i8 {
    Suit::ALL
        .into_iter()
        .map(|s| {
            let len = shape.len(s);
            if len > 4 { (len - 4) as i8 } else { 0 }
        })
        .sum()
}

fn naive_adjust3(hand: Hand) -> i8 {
    let plus =
        naive_honor_count(hand, Rank::Ace) as i16 + naive_honor_count(hand, Rank::Ten) as i16;
    let minus =
        naive_honor_count(hand, Rank::Queen) as i16 + naive_honor_count(hand, Rank::Jack) as i16;
    let delta = plus - minus;
    if delta >= 3 {
        1
    } else if delta <= -3 {
        -1
    } else {
        0
    }
}

fn naive_bergen_starting(hand: Hand) -> i8 {
    let quality_suits = Suit::ALL
        .into_iter()
        .filter(|&s| {
            let h = hand.holding(s);
            h.len() >= 4 && naive_honors5(h) >= 3
        })
        .count() as i8;
    naive_long_suit(hand.shape()) + quality_suits + naive_adjust3(hand)
}

const METHODS: [DistMethod; 4] = [
    DistMethod::GOREN_321,
    DistMethod::DUMMY_531,
    DistMethod::LongSuit,
    DistMethod::BergenStarting,
];

fn naive_distribution_points(hand: Hand, method: DistMethod) -> i8 {
    match method {
        DistMethod::ShortSuit {
            void,
            singleton,
            doubleton,
        } => naive_short_suit(hand.shape(), void, singleton, doubleton),
        DistMethod::LongSuit => naive_long_suit(hand.shape()),
        DistMethod::BergenStarting => naive_bergen_starting(hand),
    }
}

// ---------------------------------------------------------------------------------------------
// (a) SUIT table: all 8192 holdings against the naive per-suit implementations.
// ---------------------------------------------------------------------------------------------

#[test]
fn suit_table_matches_naive_for_all_holdings() {
    for bits in 0u16..8192 {
        let h = Holding::from_bits(bits).unwrap();
        let i = bits as usize;
        assert_eq!(SUIT.hcp[i], naive_hcp(h), "hcp mismatch at {bits:#06b}");
        assert_eq!(
            SUIT.losers2[i],
            naive_losers2(h),
            "losers2 mismatch at {bits:#06b}"
        );
        assert_eq!(
            SUIT.nltc2[i],
            naive_nltc2(h),
            "nltc2 mismatch at {bits:#06b}"
        );
        assert_eq!(SUIT.qt2[i], naive_qt2(h), "qt2 mismatch at {bits:#06b}");
        assert_eq!(
            SUIT.honors5[i],
            naive_honors5(h),
            "honors5 mismatch at {bits:#06b}"
        );
        assert_eq!(holding_hcp(h), naive_hcp(h));
    }
}

// ---------------------------------------------------------------------------------------------
// (b) Hand-level metrics against naive per-suit sums, on pseudo-random hands.
// ---------------------------------------------------------------------------------------------

#[test]
fn hand_metrics_match_naive_on_random_hands() {
    let mut rng = SplitMix64::new(0xC0FF_EE12_3456_789A);
    for _ in 0..4000 {
        let hand = random_hand(&mut rng);

        assert_eq!(hcp(hand), naive_hand_hcp(hand));
        assert_eq!(controls(hand), naive_hand_controls(hand));
        assert_eq!(
            losers_with(hand, LtcMethod::Classic),
            naive_hand_losers(hand, LtcMethod::Classic)
        );
        assert_eq!(
            losers_with(hand, LtcMethod::New),
            naive_hand_losers(hand, LtcMethod::New)
        );
        assert_eq!(losers(hand), naive_hand_losers(hand, LtcMethod::Classic));
        assert_eq!(quick_tricks(hand), naive_hand_quick_tricks(hand));
        assert_eq!(aces(hand), naive_honor_count(hand, Rank::Ace));
        assert_eq!(kings(hand), naive_honor_count(hand, Rank::King));
        assert_eq!(queens(hand), naive_honor_count(hand, Rank::Queen));
        assert_eq!(jacks(hand), naive_honor_count(hand, Rank::Jack));
        assert_eq!(tens(hand), naive_honor_count(hand, Rank::Ten));

        for suit in Suit::ALL {
            let holding = hand.holding(suit);
            assert_eq!(honors(holding), naive_honors5(holding));
            assert_eq!(suit_quality(holding), naive_suit_quality(holding));
            for n in 0..=5 {
                assert_eq!(top_honors(holding, n), naive_top_honors(holding, n));
            }
        }

        for method in METHODS {
            assert_eq!(
                distribution_points(hand, method),
                naive_distribution_points(hand, method)
            );
        }
    }
}

/// Hand-written spot checks transcribed from the design doc (§4.2).
#[test]
fn holding_spot_checks_from_design_doc() {
    let akx = Holding::EMPTY
        .with(Rank::Ace)
        .with(Rank::King)
        .with(Rank::Two);
    assert_eq!(SUIT.losers2[akx.bits() as usize], 2); // 1.0 loser
    assert_eq!(SUIT.nltc2[akx.bits() as usize], 1); // Q missing: 0.5
    assert_eq!(SUIT.qt2[akx.bits() as usize], 4); // AK
    assert_eq!(SUIT.honors5[akx.bits() as usize], 2);

    let aqx = Holding::EMPTY
        .with(Rank::Ace)
        .with(Rank::Queen)
        .with(Rank::Two);
    assert_eq!(SUIT.losers2[aqx.bits() as usize], 2); // 1.0 loser
    assert_eq!(SUIT.nltc2[aqx.bits() as usize], 2); // K missing: 1.0
    assert_eq!(SUIT.qt2[aqx.bits() as usize], 3); // AQ = 1.5
    assert_eq!(SUIT.honors5[aqx.bits() as usize], 2);

    let qxx = Holding::EMPTY
        .with(Rank::Queen)
        .with(Rank::Two)
        .with(Rank::Three);
    assert_eq!(SUIT.losers2[qxx.bits() as usize], 4); // 2.0 losers
    assert_eq!(SUIT.nltc2[qxx.bits() as usize], 5); // A + K missing: 1.5 + 1.0 = 2.5
    assert_eq!(SUIT.qt2[qxx.bits() as usize], 0);
    assert_eq!(SUIT.honors5[qxx.bits() as usize], 1);

    let kx = Holding::EMPTY.with(Rank::King).with(Rank::Two);
    assert_eq!(SUIT.losers2[kx.bits() as usize], 2); // 1.0 loser
    assert_eq!(SUIT.nltc2[kx.bits() as usize], 3); // A missing: 1.5
    assert_eq!(SUIT.qt2[kx.bits() as usize], 1); // Kx = 0.5
    assert_eq!(SUIT.honors5[kx.bits() as usize], 1);

    let k_singleton = Holding::EMPTY.with(Rank::King);
    assert_eq!(SUIT.losers2[k_singleton.bits() as usize], 2); // 1.0 loser (K doesn't count, len < 2)
    assert_eq!(SUIT.nltc2[k_singleton.bits() as usize], 3); // A missing: 1.5
    assert_eq!(SUIT.qt2[k_singleton.bits() as usize], 0);
    assert_eq!(SUIT.honors5[k_singleton.bits() as usize], 1);

    let x = Holding::EMPTY.with(Rank::Two);
    assert_eq!(SUIT.losers2[x.bits() as usize], 2); // 1.0 loser
    assert_eq!(SUIT.nltc2[x.bits() as usize], 3); // A missing: 1.5
    assert_eq!(SUIT.qt2[x.bits() as usize], 0);
    assert_eq!(SUIT.honors5[x.bits() as usize], 0);

    let void = Holding::EMPTY;
    assert_eq!(SUIT.losers2[void.bits() as usize], 0);
    assert_eq!(SUIT.nltc2[void.bits() as usize], 0);
    assert_eq!(SUIT.qt2[void.bits() as usize], 0);
    assert_eq!(SUIT.honors5[void.bits() as usize], 0);

    let akqjt = Holding::top_ranks(5);
    assert_eq!(SUIT.losers2[akqjt.bits() as usize], 0);
    assert_eq!(SUIT.nltc2[akqjt.bits() as usize], 0);
    assert_eq!(SUIT.qt2[akqjt.bits() as usize], 4); // AK = 2.0
    assert_eq!(SUIT.honors5[akqjt.bits() as usize], 5);
}

// ---------------------------------------------------------------------------------------------
// (c) `distribution_points` spot checks for every method, including a negative Bergen value.
// ---------------------------------------------------------------------------------------------

fn hand_of_shape(clubs: Holding, diamonds: Holding, hearts: Holding, spades: Holding) -> Hand {
    Hand::from_holdings(clubs, diamonds, hearts, spades)
}

#[test]
fn distribution_points_spot_checks() {
    // 13-0-0-0: one 13-card suit, everything else void.
    let mono = hand_of_shape(
        Holding::EMPTY,
        Holding::EMPTY,
        Holding::EMPTY,
        Holding::FULL,
    );
    assert_eq!(distribution_points(mono, DistMethod::GOREN_321), 9); // 3 voids * 3
    assert_eq!(distribution_points(mono, DistMethod::DUMMY_531), 15); // 3 voids * 5
    assert_eq!(distribution_points(mono, DistMethod::LongSuit), 9); // 13 - 4

    // Balanced 4-3-3-3: no short-suit or long-suit points at all.
    let flat = hand_of_shape(
        Holding::from_bits(0b111).unwrap(),  // clubs: 3
        Holding::from_bits(0b111).unwrap(),  // diamonds: 3
        Holding::from_bits(0b111).unwrap(),  // hearts: 3
        Holding::from_bits(0b1111).unwrap(), // spades: 4
    );
    assert_eq!(distribution_points(flat, DistMethod::GOREN_321), 0);
    assert_eq!(distribution_points(flat, DistMethod::DUMMY_531), 0);
    assert_eq!(distribution_points(flat, DistMethod::LongSuit), 0);
    assert_eq!(distribution_points(flat, DistMethod::BergenStarting), 0);

    // A single doubleton, rest flat-ish: 4=4=3=2.
    let one_doubleton = hand_of_shape(
        Holding::from_bits(0b11).unwrap(),   // clubs: 2 (doubleton)
        Holding::from_bits(0b111).unwrap(),  // diamonds: 3
        Holding::from_bits(0b1111).unwrap(), // hearts: 4
        Holding::from_bits(0b1111).unwrap(), // spades: 4
    );
    assert_eq!(distribution_points(one_doubleton, DistMethod::GOREN_321), 1);
    assert_eq!(distribution_points(one_doubleton, DistMethod::DUMMY_531), 1);
    assert_eq!(distribution_points(one_doubleton, DistMethod::LongSuit), 0);

    // Bergen: a 4432 hand with no quality suit and a very negative adjust3 (many Q/J, no A/T)
    // must come out negative: LongSuit = 0, quality_suits = 0, adjust3 = -1.
    // Spades: K Q J x (len 4, honors5 = 3 -> would be a quality suit, so avoid that: use Q J x x)
    let negative_bergen = hand_of_shape(
        Holding::from_bits(0b1111).unwrap(), // clubs: 4 low, no honours
        Holding::EMPTY
            .with(Rank::Queen)
            .with(Rank::Four)
            .with(Rank::Five), // diamonds: Qxx (3)
        Holding::EMPTY
            .with(Rank::Jack)
            .with(Rank::Four)
            .with(Rank::Five), // hearts: Jxx (3)
        Holding::EMPTY
            .with(Rank::Queen)
            .with(Rank::Jack)
            .with(Rank::Four), // spades: QJx (3)
    );
    assert_eq!(negative_bergen.len(), 13);
    assert_eq!(naive_honor_count(negative_bergen, Rank::Ace), 0);
    assert_eq!(naive_honor_count(negative_bergen, Rank::Ten), 0);
    assert_eq!(naive_honor_count(negative_bergen, Rank::Queen), 2);
    assert_eq!(naive_honor_count(negative_bergen, Rank::Jack), 2);
    // adjust3 = (0 + 0) - (2 + 2) = -4 <= -3 -> -1
    let bergen = distribution_points(negative_bergen, DistMethod::BergenStarting);
    assert_eq!(bergen, -1);
    assert_eq!(
        bergen,
        naive_distribution_points(negative_bergen, DistMethod::BergenStarting)
    );
    assert!(bergen < 0);
}

// ---------------------------------------------------------------------------------------------
// (d) `shape_points(hand.shape(), m) == Some(distribution_points(hand, m))` for shape-only
// methods, on random hands and exhaustively over all 560 shapes.
// ---------------------------------------------------------------------------------------------

const SHAPE_ONLY_METHODS: [DistMethod; 3] = [
    DistMethod::GOREN_321,
    DistMethod::DUMMY_531,
    DistMethod::LongSuit,
];

#[test]
fn shape_points_matches_distribution_points_on_random_hands() {
    let mut rng = SplitMix64::new(0x1357_9BDF_2468_ACE0);
    for _ in 0..2000 {
        let hand = random_hand(&mut rng);
        for method in SHAPE_ONLY_METHODS {
            assert_eq!(
                shape_points(hand.shape(), method),
                Some(distribution_points(hand, method))
            );
        }
        assert_eq!(shape_points(hand.shape(), DistMethod::BergenStarting), None);
    }
}

#[test]
fn shape_points_matches_naive_on_all_560_shapes() {
    for index in 0u16..560 {
        let shape = Shape::from_index(index);
        for method in SHAPE_ONLY_METHODS {
            assert_eq!(
                shape_points(shape, method),
                Some(match method {
                    DistMethod::ShortSuit {
                        void,
                        singleton,
                        doubleton,
                    } => naive_short_suit(shape, void, singleton, doubleton),
                    DistMethod::LongSuit => naive_long_suit(shape),
                    DistMethod::BergenStarting => unreachable!(),
                })
            );
        }
        assert_eq!(shape_points(shape, DistMethod::BergenStarting), None);
        assert!(DistMethod::GOREN_321.is_shape_only());
        assert!(DistMethod::LongSuit.is_shape_only());
        assert!(!DistMethod::BergenStarting.is_shape_only());
    }
}

// ---------------------------------------------------------------------------------------------
// (e) `total_points` saturation.
// ---------------------------------------------------------------------------------------------

#[test]
fn total_points_saturates_at_zero() {
    // A hand with hcp = 0 and a very negative Bergen adjust3, no long suits: total must clamp
    // to 0, not wrap or go negative.
    let hand = hand_of_shape(
        Holding::from_bits(0b1111).unwrap(), // clubs: 4 low cards, no honours
        Holding::EMPTY
            .with(Rank::Queen)
            .with(Rank::Four)
            .with(Rank::Five), // diamonds: Qxx
        Holding::EMPTY
            .with(Rank::Jack)
            .with(Rank::Four)
            .with(Rank::Five), // hearts: Jxx
        Holding::EMPTY
            .with(Rank::Queen)
            .with(Rank::Jack)
            .with(Rank::Four), // spades: QJx
    );
    assert_eq!(hcp(hand), 6); // diamonds Q=2, hearts J=1, spades Q+J=3
    let dp = distribution_points(hand, DistMethod::BergenStarting);
    let total = total_points(hand, DistMethod::BergenStarting);
    let expected = (hcp(hand) as i16 + dp as i16).max(0) as u8;
    assert_eq!(total, expected);

    // hcp = 0 and dp = 0 (flat balanced hand) => total = 0.
    let flat = hand_of_shape(
        Holding::from_bits(0b111).unwrap(),
        Holding::from_bits(0b111).unwrap(),
        Holding::from_bits(0b111).unwrap(),
        Holding::from_bits(0b1111).unwrap(),
    );
    assert_eq!(hcp(flat), 0);
    assert_eq!(distribution_points(flat, DistMethod::LongSuit), 0);
    assert_eq!(total_points(flat, DistMethod::LongSuit), 0);
}

#[test]
fn total_points_matches_random_hands() {
    let mut rng = SplitMix64::new(0xDEAD_BEEF_CAFE_F00D);
    for _ in 0..2000 {
        let hand = random_hand(&mut rng);
        for method in METHODS {
            let expected =
                (hcp(hand) as i16 + distribution_points(hand, method) as i16).max(0) as u8;
            assert_eq!(total_points(hand, method), expected);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// (f) `Half` arithmetic and `Display`.
// ---------------------------------------------------------------------------------------------

#[test]
fn half_arithmetic_and_display() {
    assert_eq!(Half::ZERO.halves(), 0);
    assert_eq!(Half::ZERO.whole(), 0);
    assert!(!Half::ZERO.is_half());
    assert_eq!(Half::ZERO.as_f32(), 0.0);
    assert_eq!(format!("{}", Half::ZERO), "0");

    let two_and_half = Half::from_halves(5);
    assert_eq!(two_and_half.whole(), 2);
    assert!(two_and_half.is_half());
    assert_eq!(two_and_half.as_f32(), 2.5);
    assert_eq!(format!("{two_and_half}"), "2.5");

    let three = Half::from_whole(3);
    assert_eq!(three.halves(), 6);
    assert_eq!(three.whole(), 3);
    assert!(!three.is_half());
    assert_eq!(format!("{three}"), "3");

    // Addition, `Add` and `Sum`.
    assert_eq!(
        Half::from_halves(3).add(Half::from_halves(4)),
        Half::from_halves(7)
    );
    assert_eq!(
        Half::from_halves(3) + Half::from_halves(4),
        Half::from_halves(7)
    );
    let summed: Half = [
        Half::from_halves(1),
        Half::from_halves(2),
        Half::from_halves(3),
    ]
    .into_iter()
    .sum();
    assert_eq!(summed, Half::from_halves(6));

    // Saturating add at the u8 boundary.
    let near_max = Half::from_halves(250);
    assert_eq!(near_max.add(Half::from_halves(10)).halves(), 255);

    // Ord matches the numeric order of halves.
    assert!(Half::from_halves(3) < Half::from_halves(4));
    assert!(Half::from_halves(4) > Half::from_halves(3));
    assert_eq!(Half::from_halves(4), Half::from_halves(4));
}
