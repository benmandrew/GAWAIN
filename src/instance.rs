//! The degeneracy filter: whether a model M and a property φ make an
//! instance worth weakening. M must violate φ, or there is nothing to
//! weaken, and M must not satisfy ¬φ, or every weakening that holds is just
//! some truth of M to which φ contributes nothing. See "Evaluation without a
//! ground truth" in `PLAN.md` and "The degeneracy filter" in
//! `docs/dev/search.md`.
//!
//! [`initial`] finds the initial-state predicate of M over φ's atoms, for
//! the experimental initial-state triviality check ("Triviality at the
//! initial state" in `docs/dev/search.md`).

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::ltl::{BinaryOp, Formula, UnaryOp};
use crate::oracle::{Backend, Lasso, Oracle, Valuation, Verdict};

/// The most distinct initial letters [`initial`] collects before it gives
/// up. Each letter beyond those of the stored runs costs one query, and a
/// model whose free inputs start in every combination would need 2^k.
pub const INITIAL_LETTERS: usize = 16;

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

/// The initial-state predicate Init of `oracle`'s model over `atoms`: the
/// disjunction of the letters, projected onto `atoms`, that start some run
/// of M. It is exact, not an over-approximation: M satisfies Init, read at
/// position 0, and every disjunct starts a real run. The letters come first
/// from the runs the oracle has stored, then from refutations: while the
/// oracle refutes the disjunction so far, the refuting run starts with a
/// letter outside it, which joins. A model with one initial letter costs
/// one query after [`classify`]. Gives up with a reason when a query is
/// `Unknown` or more than [`INITIAL_LETTERS`] letters appear, since a
/// partial disjunction would be stronger than Init.
pub fn initial<B: Backend>(
    atoms: &BTreeSet<Arc<str>>,
    oracle: &mut Oracle<B>,
) -> Result<Formula, String> {
    let first = |lasso: &Lasso| -> Option<Valuation> {
        let at = lasso.prefix().first().or(lasso.cycle().first())?;
        Some(at.intersection(atoms).cloned().collect())
    };
    let mut letters: BTreeSet<Valuation> = oracle.lassos().iter().filter_map(first).collect();
    loop {
        if letters.len() > INITIAL_LETTERS {
            return Err(format!("more than {INITIAL_LETTERS} initial letters"));
        }
        let init = (letters.iter())
            .map(|letter| {
                (atoms.iter())
                    .map(|a| match letter.contains(a) {
                        true => Formula::atom(a),
                        false => Formula::unary(UnaryOp::Not, Formula::atom(a)),
                    })
                    .reduce(|l, r| Formula::binary(BinaryOp::And, l, r))
                    .unwrap_or_else(Formula::tt)
            })
            .reduce(|l, r| Formula::binary(BinaryOp::Or, l, r))
            .unwrap_or_else(Formula::ff);
        match oracle.check(&init) {
            Verdict::Holds => return Ok(init),
            Verdict::Fails(run) => {
                let letter = first(&run).ok_or("an empty run")?;
                if !letters.insert(letter) {
                    return Err(format!("a run starting in {init} refutes it"));
                }
            }
            Verdict::Unknown(reason) => return Err(format!("{init}: {reason}")),
        }
    }
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

    fn atoms(names: &[&str]) -> BTreeSet<Arc<str>> {
        names.iter().map(|&n| Arc::from(n)).collect()
    }

    #[test]
    fn the_initial_predicate_joins_the_first_letter_of_each_run() {
        // With no stored run, the empty disjunction is refuted once per
        // initial letter, then confirmed.
        let mut fresh = oracle();
        let found = initial(&atoms(&["a", "b"]), &mut fresh);
        assert_eq!(found, Ok(f("(!a & !b) | (a & !b)")));
        assert_eq!(fresh.counters().backend_calls, 3);
        // After classify both runs are stored, so one query confirms them.
        let mut oracle = oracle();
        classify(&f("G a"), &mut oracle);
        let calls = oracle.counters().backend_calls;
        assert_eq!(initial(&atoms(&["a"]), &mut oracle), Ok(f("!a | a")));
        assert_eq!(oracle.counters().backend_calls, calls + 1);
        assert_eq!(initial(&atoms(&["b"]), &mut oracle), Ok(f("!b")));
    }

    #[test]
    fn the_initial_predicate_gives_up_on_an_unknown() {
        let found = initial(&atoms(&["a", "u"]), &mut oracle());
        assert!(
            found
                .as_ref()
                .is_err_and(|e| e.contains("no verdict for u")),
            "{found:?}"
        );
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
