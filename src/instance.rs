//! The degeneracy filter: whether a model M and a property φ make an
//! instance worth weakening. M must violate φ, or there is nothing to
//! weaken, and M must not satisfy ¬φ, or every weakening that holds is just
//! some truth of M to which φ contributes nothing. See "Evaluation without a
//! ground truth" in `PLAN.md` and "The degeneracy filter" in
//! `docs/dev/search.md`.

use crate::ltl::{Formula, UnaryOp};
use crate::oracle::{Backend, Oracle, Verdict};

/// What the two checks found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Instance {
    /// M ⊭ φ and M ⊭ ¬φ, as far as the checks could tell. Each entry of
    /// `unknown` is the reason one check gave no verdict, prefixed by the
    /// formula; such an instance is kept.
    Proper {
        /// The checks that gave no verdict.
        unknown: Vec<String>,
    },
    /// M ⊨ φ: there is nothing to weaken.
    Satisfied,
    /// M ⊨ ¬φ: φ fails on every run of M.
    Degenerate,
}

/// Checks φ and then ¬φ on `oracle`'s model. Both answers are memoised, and
/// the lasso that refutes φ (and the one that refutes ¬φ) seed the lasso
/// tier, so a search on the same oracle starts with them. ¬φ is not asked
/// when M ⊨ φ.
pub fn classify<B: Backend>(phi: &Formula, oracle: &mut Oracle<B>) -> Instance {
    let mut unknown = Vec::new();
    match oracle.check(phi) {
        Verdict::Holds => return Instance::Satisfied,
        Verdict::Fails(_) => {}
        Verdict::Unknown(reason) => unknown.push(format!("{phi}: {reason}")),
    }
    let negation = Formula::unary(UnaryOp::Not, phi.clone());
    match oracle.check(&negation) {
        Verdict::Holds => return Instance::Degenerate,
        Verdict::Fails(_) => {}
        Verdict::Unknown(reason) => unknown.push(format!("{negation}: {reason}")),
    }
    Instance::Proper { unknown }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::ltl::tests::f;
    use crate::oracle::process::Limits;
    use crate::oracle::{Lasso, Valuation, Words};

    /// A model with two runs, `a` forever and nothing forever, so `b` never
    /// holds. It answers `Unknown` for any formula that mentions `u`.
    struct TwoRuns;

    impl Backend for TwoRuns {
        fn check(&mut self, formula: &Formula) -> Verdict {
            if formula.atoms().iter().any(|a| &**a == "u") {
                return Verdict::Unknown("no verdict for u".into());
            }
            let a: Valuation = [Arc::from("a")].into_iter().collect();
            let runs = [
                Lasso::new(vec![], vec![a]).unwrap(),
                Lasso::new(vec![], vec![Valuation::new()]).unwrap(),
            ];
            match runs.into_iter().find(|run| !run.satisfies(formula)) {
                Some(run) => Verdict::Fails(run),
                None => Verdict::Holds,
            }
        }
    }

    fn oracle() -> Oracle<TwoRuns> {
        let limits = Limits {
            timeout: Duration::from_secs(10),
            memory_bytes: 1 << 30,
        };
        Oracle::new(TwoRuns, Words::sample(["a", "b", "u"], 5), limits)
    }

    #[test]
    fn a_property_that_fails_on_some_runs_only_is_proper() {
        let mut oracle = oracle();
        let phi = f("G a");
        assert_eq!(
            classify(&phi, &mut oracle),
            Instance::Proper { unknown: vec![] }
        );
        assert_eq!(oracle.counters().backend_calls, 2);
        // φ is memoised, and the run that refuted it refutes a stronger
        // formula without the back end.
        assert!(matches!(oracle.check(&phi), Verdict::Fails(_)));
        assert!(matches!(oracle.check(&f("G (a & b)")), Verdict::Fails(_)));
        let counters = oracle.counters();
        assert_eq!((counters.memo_hits, counters.lasso_hits), (1, 1));
        assert_eq!(counters.backend_calls, 2);
    }

    #[test]
    fn a_property_that_fails_on_every_run_is_degenerate() {
        assert_eq!(classify(&f("F b"), &mut oracle()), Instance::Degenerate);
    }

    #[test]
    fn a_property_that_holds_is_satisfied_without_asking_its_negation() {
        let mut oracle = oracle();
        assert_eq!(classify(&f("G !b"), &mut oracle), Instance::Satisfied);
        assert_eq!(oracle.counters().queries, 1);
    }

    #[test]
    fn an_unknown_verdict_keeps_the_instance_with_a_reason() {
        let Instance::Proper { unknown } = classify(&f("G u"), &mut oracle()) else {
            panic!("an unknown instance is kept");
        };
        assert_eq!(unknown.len(), 2, "{unknown:?}");
        assert!(unknown[0].contains("no verdict for u"), "{unknown:?}");
    }
}
