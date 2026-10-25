//! The graded status objective, an alternative to the binary status that
//! the ablation in `docs/dev/search.md` ("Graded status") did not adopt.
//!
//! A candidate the oracle proves scores 1. Under [`Status::Graded`] any
//! other candidate φ' scores k / (n + 1), where n is the number of stored
//! lassos that violate φ and k the number of those that φ' satisfies. φ
//! scores 0, and for a fixed set of lassos the score is monotone under
//! implication. The set grows during a run, so the whole pool is rescored
//! before each selection. [`Status::GradedFeasible`] keeps the grade but
//! ranks every proved candidate above every other in selection, and
//! [`Status::GradedReserve`] also keeps a share of the population for the
//! others. The default, [`Status::Binary`], scores every such candidate 0.

use std::time::{Duration, Instant};

use super::{Scored, STATUS};
use crate::ltl::Formula;
use crate::oracle::Lasso;

/// How the status objective scores a candidate the oracle does not prove.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// 0, whatever the verdict.
    #[default]
    Binary,
    /// The fraction of the stored lassos that violate φ which the
    /// candidate satisfies, kept below 1 ([`Grader`]). Kept for the
    /// ablation in `docs/dev/search.md`.
    Graded,
    /// The grade of [`Status::Graded`], with every proved candidate ranked
    /// above every other, by Deb's constrained domination
    /// ([`super::nsga2::constrained_dominates`]). The grade then orders
    /// only the candidates that are not proved. Kept for the ablation in
    /// `docs/dev/search.md`.
    GradedFeasible,
    /// The order of [`Status::GradedFeasible`], with ⌈population / d⌉ slots
    /// kept for candidates that are not proved, where d is the field, so φ
    /// and the candidates climbing the grade stay in the population. Kept
    /// for the ablation in `docs/dev/search.md`.
    GradedReserve(usize),
}

impl Status {
    /// Whether candidates are graded against the stored lassos.
    pub fn graded(self) -> bool {
        matches!(
            self,
            Status::Graded | Status::GradedFeasible | Status::GradedReserve(_)
        )
    }

    /// Whether selection ranks proved candidates above all others.
    pub fn feasible_first(self) -> bool {
        matches!(self, Status::GradedFeasible | Status::GradedReserve(_))
    }

    /// How many of `size` slots selection keeps for candidates that are not
    /// proved, when there are enough of them.
    pub fn reserved(self, size: usize) -> usize {
        match self {
            Status::GradedReserve(divisor) => size.div_ceil(divisor),
            _ => 0,
        }
    }
}

/// What rescoring cost over one run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GradeCounts {
    /// Calls to [`Grader::rescore`], one per selection.
    pub rescores: u64,
    /// Lasso evaluations of candidates, by [`Lasso::satisfies`].
    pub evaluations: u64,
    /// Lassos stored by the oracle at the last rescore.
    pub lassos: usize,
    /// Of those, the lassos that violate φ, the denominator less 1.
    pub refuting: usize,
    /// Wall time spent rescoring.
    pub time: Duration,
}

/// Grades candidates against the stored runs of M that violate φ.
#[derive(Debug)]
pub struct Grader {
    phi: Formula,
    /// Indices into the oracle's lassos of those that violate φ.
    refuting: Vec<usize>,
    /// How many of the oracle's lassos have been sorted into `refuting`.
    scanned: usize,
    counts: GradeCounts,
}

impl Grader {
    /// A grader for the input `phi`, with no lassos seen.
    pub fn new(phi: &Formula) -> Self {
        Grader {
            phi: phi.clone(),
            refuting: Vec::new(),
            scanned: 0,
            counts: GradeCounts::default(),
        }
    }

    /// The cost so far.
    pub fn counts(&self) -> GradeCounts {
        self.counts
    }

    /// Sets the status of every member of `pool` that does not hold to
    /// k / (n + 1), against `lassos`, the oracle's stored runs, which must
    /// extend the list given to the previous call.
    pub fn rescore(&mut self, pool: &mut [Scored], lassos: &[Lasso]) {
        let start = Instant::now();
        for (i, lasso) in lassos.iter().enumerate().skip(self.scanned) {
            if !lasso.satisfies(&self.phi) {
                self.refuting.push(i);
            }
        }
        self.scanned = lassos.len();
        let denominator = (self.refuting.len() + 1) as f64;
        for scored in pool.iter_mut().filter(|s| !s.holds()) {
            let satisfied = self
                .refuting
                .iter()
                .filter(|&&i| lassos[i].satisfies(&scored.formula))
                .count();
            self.counts.evaluations += self.refuting.len() as u64;
            scored.objectives[STATUS] = satisfied as f64 / denominator;
        }
        self.counts.rescores += 1;
        self.counts.lassos = lassos.len();
        self.counts.refuting = self.refuting.len();
        self.counts.time += start.elapsed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ltl::tests::f;
    use crate::oracle::Valuation;

    /// A lasso from strings of atom letters, as in `lasso.rs`.
    fn lasso(prefix: &[&str], cycle: &[&str]) -> Lasso {
        let positions = |letters: &[&str]| -> Vec<Valuation> {
            letters
                .iter()
                .map(|s| s.chars().map(|c| c.to_string().into()).collect())
                .collect()
        };
        Lasso::new(positions(prefix), positions(cycle)).unwrap()
    }

    fn scored(text: &str, status: f64) -> Scored {
        Scored {
            formula: f(text),
            objectives: vec![status, 0.0, 0.0],
        }
    }

    fn statuses(pool: &[Scored]) -> Vec<f64> {
        pool.iter().map(|s| s.objectives[STATUS]).collect()
    }

    #[test]
    fn the_reserve_rounds_up() {
        assert_eq!(Status::GradedReserve(4).reserved(40), 10);
        assert_eq!(Status::GradedReserve(4).reserved(41), 11);
        assert_eq!(Status::GradedReserve(2).reserved(1), 1);
        assert_eq!(Status::GradedFeasible.reserved(40), 0);
        assert!(Status::GradedReserve(4).feasible_first());
        assert!(Status::GradedReserve(4).graded());
    }

    #[test]
    fn the_score_counts_the_lassos_that_violate_phi() {
        let phi = f("G a");
        let mut grader = Grader::new(&phi);
        // `a` forever satisfies φ, so it is not counted; the other three
        // violate φ.
        let lassos = [
            lasso(&[], &["a"]),
            lasso(&["a"], &[""]),
            lasso(&[""], &["a"]),
            lasso(&[], &[""]),
        ];
        let mut pool = vec![
            scored("G a", 0.0),
            scored("F a", 0.0),
            scored("F G a", 0.0),
            scored("true", 0.0),
            scored("b", 1.0),
        ];
        grader.rescore(&mut pool, &lassos);
        // n = 3: φ gets 0; F a holds on two, F G a on one, true on all
        // three but still below 1; a holder keeps its 1.
        assert_eq!(statuses(&pool), [0.0, 0.5, 0.25, 0.75, 1.0]);
        let counts = grader.counts();
        assert_eq!((counts.lassos, counts.refuting), (4, 3));
        assert_eq!(counts.evaluations, 12);
    }

    #[test]
    fn a_new_lasso_rescores_the_whole_pool() {
        let phi = f("G a");
        let mut grader = Grader::new(&phi);
        let mut lassos = vec![lasso(&[""], &["a"])];
        let mut pool = vec![scored("F G a", 0.0), scored("F a", 0.0)];
        grader.rescore(&mut pool, &lassos);
        assert_eq!(statuses(&pool), [0.5, 0.5]);
        lassos.push(lasso(&["a"], &[""]));
        grader.rescore(&mut pool, &lassos);
        assert_eq!(statuses(&pool), [1.0 / 3.0, 2.0 / 3.0]);
        assert_eq!(grader.counts().rescores, 2);
    }

    #[test]
    fn the_score_is_monotone_under_implication() {
        // Each formula implies the next, so on any lassos the scores rise.
        let chain = ["G a", "G (a | b)", "G F (a | b)", "F (a | b)", "true"];
        let lassos = [
            lasso(&["b"], &["a"]),
            lasso(&[""], &["b", ""]),
            lasso(&["a"], &[""]),
            lasso(&[], &[""]),
            lasso(&["ab", "b"], &["a"]),
        ];
        let mut grader = Grader::new(&f(chain[0]));
        let mut pool: Vec<Scored> = chain.iter().map(|t| scored(t, 0.0)).collect();
        grader.rescore(&mut pool, &lassos);
        let s = statuses(&pool);
        assert!(s.windows(2).all(|w| w[0] <= w[1]), "{s:?}");
        assert_eq!(s[0], 0.0);
        assert!(s[4] < 1.0);
    }
}
