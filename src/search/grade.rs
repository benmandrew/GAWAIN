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

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{Scored, STATUS};
use crate::ltl::Formula;
use crate::oracle::{Lasso, Packed, Program};

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

/// Grades candidates against the runs of M that violate φ, from
/// [`Oracle::lassos`](crate::oracle::Oracle::lassos). For a model checker
/// those are the lassos stored by tier 1, a list that grows; for a trace
/// set they are the positive traces, a fixed list (see "Graded status on
/// trace sets" in `docs/dev/trace-sets.md`).
///
/// Runs of at most 64 positions are packed over φ's atoms and read by the
/// trace sets' evaluator (`oracle::packed`), which is about 20 times
/// faster here ("Graded status on trace sets"). Longer runs, and any
/// candidate with an atom outside φ's, which mutation never makes, are
/// read by [`Lasso::satisfies`], with the same semantics.
#[derive(Debug)]
pub struct Grader {
    phi: Formula,
    /// φ's atoms, in the order runs are packed over.
    atoms: Vec<Arc<str>>,
    /// The oracle's runs that violate φ: each one's index in the oracle's
    /// list, and its packed form when it fits.
    refuting: Vec<(usize, Option<Packed>)>,
    /// How many of the oracle's runs have been sorted into `refuting`.
    scanned: usize,
    counts: GradeCounts,
}

/// Whether the run `lassos[i]`, packed as `packed`, satisfies `formula`,
/// compiled as `program` when its atoms are all packed.
fn satisfies(
    (i, packed): &(usize, Option<Packed>),
    program: Option<&Program>,
    formula: &Formula,
    lassos: &[Lasso],
    stack: &mut Vec<u64>,
) -> bool {
    match (packed, program) {
        (Some(packed), Some(program)) => program.satisfies(packed, stack),
        _ => lassos[*i].satisfies(formula),
    }
}

impl Grader {
    /// A grader for the input `phi`, with no lassos seen.
    pub fn new(phi: &Formula) -> Self {
        Grader {
            phi: phi.clone(),
            atoms: phi.atoms().into_iter().collect(),
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
        let mut stack = Vec::new();
        let phi = Program::new(&self.phi, &self.atoms);
        for (i, lasso) in lassos.iter().enumerate().skip(self.scanned) {
            let run = (i, Packed::new(lasso, &self.atoms));
            if !satisfies(&run, Some(&phi), &self.phi, lassos, &mut stack) {
                self.refuting.push(run);
            }
        }
        self.scanned = lassos.len();
        let denominator = (self.refuting.len() + 1) as f64;
        for scored in pool.iter_mut().filter(|s| !s.holds()) {
            let formula = &scored.formula;
            let packable = formula.atoms().iter().all(|a| self.atoms.contains(a));
            let program = packable.then(|| Program::new(formula, &self.atoms));
            let satisfied = (self.refuting.iter())
                .filter(|run| satisfies(run, program.as_ref(), formula, lassos, &mut stack))
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
    use crate::oracle::traces::TraceSet;
    use crate::oracle::{Oracle, Valuation};
    use crate::search::Rng;

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

    #[test]
    fn a_trace_set_grades_against_its_positives_under_ltlf() {
        // The first positive satisfies φ = G a under LTLf and the other
        // three violate it. The last has a at 69 positions and then none,
        // 70 in all, too long to pack.
        let long = ["10"; 69].join(" ") + " 00";
        let text = format!("semantics finite\natoms a b\n+ 10 10\n+ 00 11\n+ 01\n+ {long}\n");
        let set = TraceSet::parse(&text).unwrap();
        // memo_only stores no lasso, so the grader reads the positives.
        let oracle = Oracle::memo_only(set.clone());
        assert_eq!(oracle.lassos(), set.positives());
        let mut grader = Grader::new(&f("G a"));
        let mut pool: Vec<Scored> = ["G a", "F a", "G (a | X false)", "G F a", "true"]
            .iter()
            .map(|t| scored(t, 0.0))
            .collect();
        grader.rescore(&mut pool, oracle.lassos());
        // n = 3. F a holds on the second and fourth. X false holds only
        // at a word's last position (weak next), so G (a | X false) holds
        // on the third and fourth. G F a needs a at the last position: the
        // second only. true holds on all three and still scores below 1.
        assert_eq!(statuses(&pool), [0.0, 0.5, 0.5, 0.25, 0.75]);
        let counts = grader.counts();
        assert_eq!((counts.lassos, counts.refuting), (4, 3));
    }

    #[test]
    fn the_score_is_monotone_under_ltlf_implication() {
        // Each formula implies the next under LTLf, where weak X b holds at
        // a word's last position, so on any finite words the scores rise.
        // φ mentions both atoms, so every run is packed.
        let chain = ["G (a & b)", "G (a | X b)", "a | X b", "F (a | X b)", "true"];
        let mut rng = Rng::new(7);
        for _ in 0..50 {
            let words: Vec<Lasso> = (0..20)
                .map(|_| {
                    let length = 1 + rng.below(6);
                    let mut word = Vec::new();
                    for _ in 0..length {
                        let v: Valuation = ["a", "b"]
                            .into_iter()
                            .filter(|_| rng.coin())
                            .map(Arc::from)
                            .collect();
                        word.push(v);
                    }
                    Lasso::finite(word).unwrap()
                })
                .collect();
            let mut grader = Grader::new(&f(chain[0]));
            let mut pool: Vec<Scored> = chain.iter().map(|t| scored(t, 0.0)).collect();
            grader.rescore(&mut pool, &words);
            let s = statuses(&pool);
            assert!(s.windows(2).all(|w| w[0] <= w[1]), "{s:?}");
            assert!(s[0] == 0.0 && s[4] < 1.0, "{s:?}");
        }
    }
}
