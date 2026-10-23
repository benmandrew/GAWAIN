//! The seeded random source of the search.
//!
//! PEREDUR's `RandomSource` rules carry over. Each draw is sequenced into its
//! own local, never two draws as arguments of one call. Rust evaluates
//! arguments left to right, so the rule is not needed for portability here,
//! but it keeps the draw order visible in the source, which is what a
//! determinism test pins. A probability of 0 or 1 costs no draw, so a branch
//! switched off does not shift the stream.

/// SplitMix64 with Lemire's bounded reduction. Not PEREDUR's mt19937: the
/// streams of the two tools are not meant to match, only each to reproduce.
#[derive(Clone, Debug)]
pub struct Rng(u64);

/// The resolution of [`Rng::chance`], as PEREDUR's `k_rate_granularity`.
const GRANULARITY: usize = 1_000_000;

impl Rng {
    /// A source seeded with `seed`. Equal seeds give equal streams.
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform index in `[0, bound)`. Panics on a zero bound.
    ///
    /// Lemire, "Fast Random Integer Generation in an Interval", ACM TOMACS
    /// 29(1), 2019: multiply into 128 bits, keep the high half, and reject
    /// the short low tail that would bias it.
    pub fn below(&mut self, bound: usize) -> usize {
        assert!(bound > 0, "a draw needs a non-empty range");
        let bound = bound as u64;
        let mut wide = u128::from(self.next_u64()) * u128::from(bound);
        if (wide as u64) < bound {
            let threshold = bound.wrapping_neg() % bound;
            while (wide as u64) < threshold {
                wide = u128::from(self.next_u64()) * u128::from(bound);
            }
        }
        (wide >> 64) as usize
    }

    /// A fair coin.
    pub fn coin(&mut self) -> bool {
        self.below(2) == 1
    }

    /// True with probability `p`. No draw when `p <= 0` or `p >= 1`.
    pub fn chance(&mut self, p: f64) -> bool {
        if p <= 0.0 {
            return false;
        }
        if p >= 1.0 {
            return true;
        }
        self.below(GRANULARITY) < (p * GRANULARITY as f64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_seeds_give_equal_streams() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        let mut c = Rng::new(43);
        let draws = |rng: &mut Rng| (0..100).map(|_| rng.below(1000)).collect::<Vec<_>>();
        let first = draws(&mut a);
        assert_eq!(first, draws(&mut b));
        assert_ne!(first, draws(&mut c));
    }

    #[test]
    fn draws_stay_in_range_and_cover_it() {
        let mut rng = Rng::new(1);
        let mut seen = [0usize; 7];
        for _ in 0..7000 {
            seen[rng.below(7)] += 1;
        }
        assert!(seen.iter().all(|&n| n > 800), "{seen:?}");
        assert_eq!(rng.below(1), 0);
    }

    #[test]
    fn certain_and_impossible_chances_cost_no_draw() {
        let mut rng = Rng::new(9);
        let mut reference = rng.clone();
        assert!(!rng.chance(0.0));
        assert!(rng.chance(1.0));
        assert_eq!(rng.below(1 << 20), reference.below(1 << 20));
    }
}
