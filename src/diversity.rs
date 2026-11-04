//! The behavioural distance between repairs, for the diversity measure of
//! `bench/diversity_stats.py` ("Diversity" in `docs/dev/evaluation.md`).
//!
//! The distance follows PEREDUR's paper. A fixed sample of random traces
//! over the atoms is drawn once per instance, and each formula's satisfying
//! set is one bit per trace. The distance between formulas a and b is
//! |a xor b| / |a or b|, the share of the traces that either satisfies on
//! which they disagree, and 0 when neither satisfies any.
//!
//! The sampler is PEREDUR's `sample_words` (`src/fingerprint/lasso.cpp`)
//! with its separation settings: a stem of 0 to 8 positions and a loop of 1
//! to 8, both uniform, and each atom true at each position by a fair coin.
//! Under LTLf a word is finite, so the sampler draws the same two lengths
//! and makes one finite word of their sum: lengths 1 to 16, with the same
//! distribution as a lasso's number of distinct positions. The stream is
//! [`Rng`], not PEREDUR's mt19937, so the traces are reproducible but not
//! PEREDUR's.
//!
//! Each trace is packed once into words, and a formula is compiled once and
//! run on every trace by the trace-set back end's evaluator
//! (`oracle::packed`), so nothing is allocated per trace.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::ltl::Formula;
use crate::oracle::{Packed, Program, Semantics};
use crate::search::Rng;

/// The longest stem, and the longest loop, of a sampled trace.
pub const MAX_STEP: usize = 8;

/// A fixed sample of traces over a set of atoms.
pub struct Traces {
    atoms: Vec<Arc<str>>,
    runs: Vec<Packed>,
}

impl Traces {
    /// Draws `count` traces over `atoms` under `semantics`. The traces
    /// depend only on the sorted set of atoms, the semantics, `count` and
    /// `seed`.
    pub fn sample(
        atoms: BTreeSet<Arc<str>>,
        semantics: Semantics,
        seed: u64,
        count: usize,
    ) -> Self {
        let atoms: Vec<Arc<str>> = atoms.into_iter().collect();
        let mut rng = Rng::new(seed);
        let runs = (0..count)
            .map(|_| {
                let stem = rng.below(MAX_STEP + 1);
                let len = stem + 1 + rng.below(MAX_STEP);
                let mut words = vec![0u64; atoms.len()];
                for position in 0..len {
                    for word in &mut words {
                        if rng.coin() {
                            *word |= 1 << position;
                        }
                    }
                }
                let start = match semantics {
                    Semantics::Infinite => stem,
                    Semantics::Finite => len,
                };
                Packed::from_words(len, start, words)
            })
            .collect();
        Traces { atoms, runs }
    }

    /// The number of traces.
    pub fn len(&self) -> usize {
        self.runs.len()
    }

    /// Whether the sample is empty.
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// The traces that satisfy `formula`, as a bitset of
    /// `len().div_ceil(64)` words. An atom outside the sample's atoms is
    /// false everywhere.
    pub fn satisfying(&self, formula: &Formula) -> Vec<u64> {
        let program = Program::new(formula, &self.atoms);
        let mut stack = Vec::with_capacity(formula.n_subformulae());
        let mut bits = vec![0u64; self.runs.len().div_ceil(64)];
        for (i, run) in self.runs.iter().enumerate() {
            if program.satisfies(run, &mut stack) {
                bits[i / 64] |= 1 << (i % 64);
            }
        }
        bits
    }
}

/// |a xor b| / |a or b| over two bitsets of one sample, and 0 when both
/// are empty.
pub fn distance(a: &[u64], b: &[u64]) -> f64 {
    let (mut xor, mut or) = (0u32, 0u32);
    for (x, y) in a.iter().zip(b) {
        xor += (x ^ y).count_ones();
        or += (x | y).count_ones();
    }
    if or == 0 {
        0.0
    } else {
        f64::from(xor) / f64::from(or)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(text: &str) -> Formula {
        text.parse().unwrap()
    }

    fn traces(semantics: Semantics) -> Traces {
        let atoms = ["a", "b"].iter().map(|&a| a.into()).collect();
        Traces::sample(atoms, semantics, 7, 4096)
    }

    fn d(t: &Traces, x: &str, y: &str) -> f64 {
        distance(&t.satisfying(&f(x)), &t.satisfying(&f(y)))
    }

    #[test]
    fn identical_and_equivalent_formulas_are_at_distance_zero() {
        for s in [Semantics::Infinite, Semantics::Finite] {
            let t = traces(s);
            assert_eq!(d(&t, "G F a", "G F a"), 0.0);
            assert_eq!(d(&t, "!(a U b)", "!b W (!a & !b)"), 0.0);
        }
    }

    #[test]
    fn a_formula_and_its_negation_are_at_distance_one() {
        for s in [Semantics::Infinite, Semantics::Finite] {
            let t = traces(s);
            assert_eq!(d(&t, "a U b", "!(a U b)"), 1.0);
        }
    }

    #[test]
    fn a_monotone_chain_has_ordered_distances() {
        let t = traces(Semantics::Infinite);
        // G a ⇒ a ⇒ F a: the satisfying sets nest, so the distance from G a
        // grows along the chain, and is the share of the larger set that
        // the smaller misses.
        let (g, a, fa) = (
            t.satisfying(&f("G a")),
            t.satisfying(&f("a")),
            t.satisfying(&f("F a")),
        );
        assert!(g.iter().zip(&a).all(|(x, y)| x & !y == 0));
        let (ga, gf) = (distance(&g, &a), distance(&g, &fa));
        assert!(0.0 < ga && ga < gf && gf < 1.0);
        // About half the traces satisfy a, and nearly all F a.
        let ones = |b: &[u64]| b.iter().map(|w| w.count_ones()).sum::<u32>() as f64;
        assert!((ones(&a) / t.len() as f64 - 0.5).abs() < 0.05);
        assert!((distance(&a, &fa) - (1.0 - ones(&a) / ones(&fa))).abs() < 1e-12);
    }

    #[test]
    fn finite_words_read_ltlf() {
        // Weak X makes `X false` hold at the last position of a finite
        // word, so F X false holds on every finite word and on no lasso.
        assert_eq!(d(&traces(Semantics::Finite), "F X false", "true"), 0.0);
        assert_eq!(d(&traces(Semantics::Infinite), "F X false", "false"), 0.0);
    }

    #[test]
    fn the_sample_is_fixed_by_its_seed() {
        let t = traces(Semantics::Infinite);
        assert_eq!(
            t.satisfying(&f("a U b")),
            traces(Semantics::Infinite).satisfying(&f("a U b"))
        );
        let u = Traces::sample(
            t.atoms.iter().cloned().collect(),
            Semantics::Infinite,
            8,
            4096,
        );
        assert_ne!(t.satisfying(&f("a U b")), u.satisfying(&f("a U b")));
    }

    #[test]
    fn neither_satisfying_any_is_distance_zero() {
        assert_eq!(distance(&[0, 0], &[0, 0]), 0.0);
        assert_eq!(distance(&[0b1010], &[0b0110]), 2.0 / 3.0);
    }
}
