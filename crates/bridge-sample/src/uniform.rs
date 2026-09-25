//! The v0 baseline: deal the unknown cards uniformly.

use bridge_core::{Card, Deal, Hand, Seat};

use crate::{PreparedProposal, Proposal, SampleContext, SampleError};

/// Fisher–Yates over the pool; every constraint is handled by the likelihood.
/// `log_prob` is the constant `−ln(|pool|! / Π_s needed(s)!)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct UniformProposal;

impl Proposal for UniformProposal {
    fn prepare<'c>(
        &self,
        ctx: &'c SampleContext<'c>,
    ) -> Result<Box<dyn PreparedProposal + Send + Sync + 'c>, SampleError> {
        // ln(|pool|! / Π needed(s)!) = ln|pool|! − Σ ln needed(s)!, negated.
        let mut log_prob = -ln_factorial(ctx.known.pool().len());
        for seat in Seat::ALL {
            log_prob += ln_factorial(ctx.known.needed(seat));
        }
        Ok(Box::new(PreparedUniform { ctx, log_prob }))
    }
}

struct PreparedUniform<'c> {
    ctx: &'c SampleContext<'c>,
    log_prob: f64,
}

impl PreparedProposal for PreparedUniform<'_> {
    fn propose(&self, rng: &mut dyn rand_core::Rng) -> Option<Deal> {
        let mut pool: Vec<Card> = self.ctx.known.pool().cards().collect();
        fisher_yates(&mut pool, rng);

        let mut hands = [Hand::EMPTY; 4];
        let mut offset = 0usize;
        for seat in Seat::ALL {
            let needed = self.ctx.known.needed(seat) as usize;
            let mut hand = self.ctx.known.known[seat.index() as usize];
            for &card in &pool[offset..offset + needed] {
                hand = hand.with(card);
            }
            hands[seat.index() as usize] = hand;
            offset += needed;
        }
        debug_assert_eq!(
            offset,
            pool.len(),
            "every pool card must be dealt to some seat"
        );
        Some(Deal::new(hands).expect(
            "known cards are pairwise disjoint (KnownCards::new) and the pool covers exactly \
             what each seat still needs, so the four hands always partition the deck",
        ))
    }

    fn log_prob(&self, deal: &Deal) -> f64 {
        self.log_prob
    }
}

/// Shuffles `cards` uniformly at random, drawing indices with [`bounded`].
fn fisher_yates(cards: &mut [Card], rng: &mut dyn rand_core::Rng) {
    for i in (1..cards.len()).rev() {
        let j = bounded(rng, (i + 1) as u64) as usize;
        cards.swap(i, j);
    }
}

/// A uniform integer in `0..n` (`n ≥ 1`) via Lemire's nearly-divisionless method, so that the
/// sample stream depends only on `rng.next_u64()` and never on `rand`'s own range-sampling
/// algorithm (which is free to change between versions).
fn bounded(rng: &mut dyn rand_core::Rng, n: u64) -> u64 {
    debug_assert!(n > 0, "bounded(rng, 0) has no valid output");
    let mut x = rng.next_u64();
    let mut wide = u128::from(x) * u128::from(n);
    let mut low = wide as u64;
    if low < n {
        // `threshold = 2^64 mod n`, computed without overflow as `(-n) mod n` in `u64`.
        let threshold = n.wrapping_neg() % n;
        while low < threshold {
            x = rng.next_u64();
            wide = u128::from(x) * u128::from(n);
            low = wide as u64;
        }
    }
    (wide >> 64) as u64
}

/// `ln(n!)` for `n ≤ 52` from a precomputed table (there are at most 52 cards to place).
fn ln_factorial(n: u8) -> f64 {
    LN_FACTORIAL[n as usize]
}

/// `ln(k!)` for `k = 0..=52`, i.e. `ln_gamma(k + 1)`.
#[rustfmt::skip]
#[allow(clippy::approx_constant)] // `ln_factorial(2) == LN_2` is a coincidence of the table, not an approximation.
static LN_FACTORIAL: [f64; 53] = [
    0.0, 0.0, 0.693147180559945, 1.791759469228055,
    3.1780538303479444, 4.787491742782047, 6.579251212010101, 8.525161361065415,
    10.60460290274525, 12.801827480081467, 15.104412573075514, 17.502307845873887,
    19.987214495661885, 22.55216385312342, 25.191221182738683, 27.89927138384089,
    30.671860106080672, 33.50507345013689, 36.39544520803305, 39.339884187199495,
    42.335616460753485, 45.38013889847691, 48.47118135183522, 51.60667556776438,
    54.78472939811232, 58.00360522298051, 61.26170176100201, 64.55753862700634,
    67.88974313718154, 71.257038967168, 74.65823634883016, 78.0922235533153,
    81.55795945611503, 85.05446701758152, 88.58082754219768, 92.1361756036871,
    95.7196945421432, 99.33061245478743, 102.96819861451381, 106.63176026064346,
    110.32063971475739, 114.03421178146169, 117.77188139974508, 121.53308151543864,
    125.3172711493569, 129.12393363912722, 132.9525750356163, 136.80272263732635,
    140.67392364823425, 144.56574394634487, 148.47776695177305, 152.40959258449735,
    156.3608363030788,
];

#[cfg(test)]
mod tests {
    use bridge_bidding::Interpretation;
    use bridge_constraint::{HandConstraint, KnownCards};
    use bridge_core::Seat;

    use super::*;
    use crate::rng_for;

    fn empty_interpretation() -> Interpretation {
        Interpretation {
            seats: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            per_call: Vec::new(),
            divergence: None,
        }
    }

    const NO_CONSTRAINTS: [HandConstraint; 4] = [
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
    ];

    /// The 52 cards split into 4 chunks of 13, in `Seat::ALL` order.
    fn deck_in_seat_chunks() -> [Hand; 4] {
        let cards: Vec<Card> = Hand::FULL.cards().collect();
        let mut hands = [Hand::EMPTY; 4];
        for (hand, chunk) in hands.iter_mut().zip(cards.chunks(13)) {
            for &card in chunk {
                *hand = hand.with(card);
            }
        }
        hands
    }

    #[test]
    fn propose_gives_valid_deals_respecting_known_cards() {
        // North and the dummy (East) are fully known; South and West split the other 26 cards.
        let hands = deck_in_seat_chunks();
        let known = KnownCards::from_viewer(Seat::North, hands[0]).with_dummy(Seat::East, hands[1]);
        let interpretation = empty_interpretation();
        let ctx = SampleContext {
            known,
            interpretation: &interpretation,
            play_constraints: &NO_CONSTRAINTS,
            play_soft: None,
            bidding: None,
        };

        let prepared = UniformProposal
            .prepare(&ctx)
            .expect("uniform proposal always prepares");
        let mut rng = rng_for(7, 0);
        let pool = known.pool();
        for _ in 0..200 {
            let deal = prepared
                .propose(&mut rng)
                .expect("uniform proposal never rejects");

            // The known hands are exactly reproduced.
            assert_eq!(deal.hand(Seat::North), hands[0]);
            assert_eq!(deal.hand(Seat::East), hands[1]);

            // The unknown seats draw only from the pool, 13 cards each (`Deal::new` already
            // guarantees the four hands partition the deck; this also pins them to the pool).
            assert!(deal.hand(Seat::South).is_subset(pool));
            assert!(deal.hand(Seat::West).is_subset(pool));
            assert_eq!(deal.hand(Seat::South).len(), 13);
            assert_eq!(deal.hand(Seat::West).len(), 13);
        }
    }

    #[test]
    fn log_prob_is_constant_and_matches_the_multinomial_formula() {
        // 11 known cards per seat, 2 unknown each: log_prob = −ln(8! / (2!)^4) = −ln 2520.
        let full_hands = deck_in_seat_chunks();
        let mut known = [Hand::EMPTY; 4];
        for (seat, hand) in Seat::ALL.into_iter().zip(full_hands) {
            let mut eleven = Hand::EMPTY;
            for card in hand.cards().take(11) {
                eleven = eleven.with(card);
            }
            known[seat.index() as usize] = eleven;
        }
        let known = KnownCards::new(known).expect("11 disjoint cards per seat");
        assert_eq!(known.pool().len(), 8);
        for seat in Seat::ALL {
            assert_eq!(known.needed(seat), 2);
        }

        let interpretation = empty_interpretation();
        let ctx = SampleContext {
            known,
            interpretation: &interpretation,
            play_constraints: &NO_CONSTRAINTS,
            play_soft: None,
            bidding: None,
        };

        let prepared = UniformProposal
            .prepare(&ctx)
            .expect("uniform proposal always prepares");
        let expected = -2520f64.ln();

        let mut rng = rng_for(11, 0);
        for _ in 0..20 {
            let deal = prepared
                .propose(&mut rng)
                .expect("uniform proposal never rejects");
            let log_prob = prepared.log_prob(&deal);
            assert!(
                (log_prob - expected).abs() < 1e-9,
                "log_prob = {log_prob}, expected {expected}"
            );
        }
    }

    #[test]
    fn ln_factorial_matches_small_values() {
        assert_eq!(ln_factorial(0), 0.0);
        assert_eq!(ln_factorial(1), 0.0);
        assert!((ln_factorial(2) - 2f64.ln()).abs() < 1e-12);
        assert!((ln_factorial(5) - 120f64.ln()).abs() < 1e-9);
    }

    #[test]
    fn bounded_stays_in_range() {
        use crate::rng_for;
        let mut rng = rng_for(42, 0);
        for _ in 0..10_000 {
            let v = bounded(&mut rng, 7);
            assert!(v < 7);
        }
    }

    #[test]
    fn bounded_one_is_always_zero() {
        use crate::rng_for;
        let mut rng = rng_for(1, 1);
        for _ in 0..100 {
            assert_eq!(bounded(&mut rng, 1), 0);
        }
    }
}
