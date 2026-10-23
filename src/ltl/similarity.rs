//! Syntactic similarity by shared subformulae, ported from PEREDUR's
//! `similarity.cpp`.

use std::collections::HashMap;

use super::Formula;

impl Formula {
    /// Subtrees the two formulas have in common, matched as multisets: a
    /// subtree occurring twice on one side and once on the other counts once.
    fn shared_subformulae(&self, other: &Formula) -> usize {
        let mut counts: HashMap<&Formula, usize> = HashMap::new();
        for sub in self.subformulae() {
            *counts.entry(sub).or_default() += 1;
        }
        let mut shared = 0;
        for sub in other.subformulae() {
            if let Some(count) = counts.get_mut(sub) {
                if *count > 0 {
                    *count -= 1;
                    shared += 1;
                }
            }
        }
        shared
    }

    /// The harmonic mean of the fraction of each side's subtrees that the
    /// other shares, in [0, 1]. Subtrees are matched as multisets.
    ///
    /// The harmonic mean rather than the arithmetic one because a small
    /// formula contained in a large one has a trivial ratio of 1 on its own
    /// side, which would hold the arithmetic mean at 0.5 or above however
    /// large the other side grew.
    pub fn syntactic_similarity(&self, other: &Formula) -> f64 {
        let shared = self.shared_subformulae(other) as f64;
        if shared == 0.0 {
            return 0.0;
        }
        let first = shared / self.n_subformulae() as f64;
        let second = shared / other.n_subformulae() as f64;
        2.0 * first * second / (first + second)
    }
}

#[cfg(test)]
mod tests {
    use crate::ltl::tests::{f, random_formulas};

    #[test]
    fn identical_formulas_score_one() {
        for formula in random_formulas(200, 4) {
            assert_eq!(formula.syntactic_similarity(&formula), 1.0);
        }
    }

    #[test]
    fn disjoint_formulas_score_zero() {
        assert_eq!(f("a & b").syntactic_similarity(&f("G c")), 0.0);
    }

    #[test]
    fn shared_counts_with_multiplicity() {
        assert_eq!(f("a & a").shared_subformulae(&f("a")), 1);
        assert_eq!(f("a & a").shared_subformulae(&f("a | a")), 2);
        assert_eq!(f("G (a -> b)").shared_subformulae(&f("F (a -> b)")), 3);
    }

    #[test]
    fn harmonic_mean_of_both_ratios() {
        // shared = 3 of 4 and 3 of 4.
        assert!((f("G (a -> b)").syntactic_similarity(&f("F (a -> b)")) - 0.75).abs() < 1e-12);
        // a is 1 of 1 on the left and 1 of 3 on the right: 2 * 1 * (1/3) / (4/3).
        assert!((f("a").syntactic_similarity(&f("a & b")) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn similarity_is_symmetric_and_bounded() {
        let formulas = random_formulas(60, 5);
        for x in &formulas {
            for y in &formulas {
                let s = x.syntactic_similarity(y);
                assert!((0.0..=1.0).contains(&s));
                assert_eq!(s, y.syntactic_similarity(x));
            }
        }
    }
}
