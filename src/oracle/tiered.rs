//! The tiered oracle: the cheap tiers in front of a complete [`Backend`].
//!
//! [`Oracle::check`] tries, in order, and stops at the first answer:
//!
//! 0. a memo of earlier verdicts, keyed on [`Formula::canonical`];
//! 1. the stored runs of M ([`LassoCache::refutes`]), which can only refute;
//! 2. the known truths of M: the fingerprint prefilter
//!    ([`KnownTruths::may_imply`]), then one Spot exec ([`implications`]) for
//!    the truths that survive it, which can only prove;
//! 3. the back end.
//!
//! Every answer feeds the tiers in front of it. A failure's lasso goes into
//! the lasso cache, and a back-end truth into the known truths, from which it
//! evicts every stored truth Spot proves it implies. `Unknown` is never
//! memoised. See "Recommended oracle" in `docs/dev/oracle-performance.md` and
//! `docs/dev/oracle.md`.

use std::collections::HashMap;

use super::process::Limits;
use super::spot::implications;
use super::{Backend, KnownTruths, LassoCache, Verdict, Words};
use crate::ltl::Formula;

/// How many queries each tier answered, and what the slow tiers cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Calls to [`Oracle::check`].
    pub queries: u64,
    /// Queries answered by the memo (tier 0).
    pub memo_hits: u64,
    /// Queries refuted by a stored lasso (tier 1).
    pub lasso_hits: u64,
    /// Queries proved from a known truth (tier 2).
    pub implication_hits: u64,
    /// `ltlfilt` execs, for tier 2 and for evicting implied truths.
    pub spot_calls: u64,
    /// `ltlfilt` execs that answered "unknown", read as "nothing proved".
    pub spot_errors: u64,
    /// Queries that reached the back end (tier 3).
    pub backend_calls: u64,
    /// Back-end calls that answered [`Verdict::Unknown`].
    pub backend_unknowns: u64,
}

/// A model-checking oracle for one fixed model, with `B` as its last tier.
pub struct Oracle<B> {
    backend: B,
    memo: HashMap<Formula, Verdict>,
    lassos: LassoCache,
    truths: KnownTruths,
    spot_limits: Limits,
    counters: Counters,
}

impl<B: Backend> Oracle<B> {
    /// An oracle with empty caches. `words` must be sampled over the model's
    /// atoms; `spot_limits` bound each `ltlfilt` exec.
    pub fn new(backend: B, words: Words, spot_limits: Limits) -> Self {
        Oracle {
            backend,
            memo: HashMap::new(),
            lassos: LassoCache::new(),
            truths: KnownTruths::new(words),
            spot_limits,
            counters: Counters::default(),
        }
    }

    /// The counters so far.
    pub fn counters(&self) -> Counters {
        self.counters
    }

    /// Decides whether the model satisfies `formula`, cheapest tier first.
    pub fn check(&mut self, formula: &Formula) -> Verdict {
        self.counters.queries += 1;
        let key = formula.canonical();
        if let Some(verdict) = self.memo.get(&key) {
            self.counters.memo_hits += 1;
            return verdict.clone();
        }
        let verdict = if let Some(lasso) = self.lassos.refutes(formula) {
            self.counters.lasso_hits += 1;
            Verdict::Fails(lasso.clone())
        } else if self.implied_by_a_truth(formula) {
            self.counters.implication_hits += 1;
            Verdict::Holds
        } else {
            self.counters.backend_calls += 1;
            match self.backend.check(formula) {
                Verdict::Holds => {
                    self.learn_truth(formula);
                    Verdict::Holds
                }
                Verdict::Fails(lasso) => {
                    self.lassos.insert(lasso.clone());
                    Verdict::Fails(lasso)
                }
                Verdict::Unknown(reason) => {
                    self.counters.backend_unknowns += 1;
                    return Verdict::Unknown(reason);
                }
            }
        };
        self.memo.insert(key, verdict.clone());
        verdict
    }

    /// Tier 2: does Spot prove that some known truth implies `formula`?
    fn implied_by_a_truth(&mut self, formula: &Formula) -> bool {
        let survivors: Vec<Formula> = self.truths.may_imply(formula).cloned().collect();
        let pairs: Vec<_> = survivors.iter().map(|truth| (truth, formula)).collect();
        self.spot(&pairs).contains(&true)
    }

    /// Adds a back-end truth, and drops each stored truth it provably
    /// implies, so the store stays an antichain as far as Spot can tell.
    fn learn_truth(&mut self, truth: &Formula) {
        let implied: Vec<Formula> = self.truths.may_be_implied_by(truth).cloned().collect();
        let pairs: Vec<_> = implied.iter().map(|weaker| (truth, weaker)).collect();
        let proved = self.spot(&pairs);
        self.truths.insert(truth.clone());
        for (weaker, proved) in implied.iter().zip(proved) {
            if proved {
                self.truths.remove(weaker);
            }
        }
    }

    /// One `ltlfilt` exec over `pairs`, with "unknown" read as nothing
    /// proved. No exec for an empty batch.
    fn spot(&mut self, pairs: &[(&Formula, &Formula)]) -> Vec<bool> {
        if pairs.is_empty() {
            return Vec::new();
        }
        self.counters.spot_calls += 1;
        implications(pairs, self.spot_limits).unwrap_or_else(|_| {
            self.counters.spot_errors += 1;
            vec![false; pairs.len()]
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::oracle::{Lasso, Valuation};

    fn f(text: &str) -> Formula {
        text.parse().unwrap()
    }

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

    /// The model whose only run is `a` at the start and nothing after, as a
    /// back end that counts its calls. It answers `Unknown` for any formula
    /// that mentions `u`.
    struct Fake {
        calls: usize,
    }

    impl Backend for Fake {
        fn check(&mut self, formula: &Formula) -> Verdict {
            self.calls += 1;
            let text = formula.to_string();
            if text.split(|c: char| !c.is_alphanumeric()).any(|w| w == "u") {
                return Verdict::Unknown("no verdict for u".into());
            }
            let run = lasso(&["a"], &[""]);
            if run.satisfies(formula) {
                Verdict::Holds
            } else {
                Verdict::Fails(run)
            }
        }
    }

    fn oracle() -> Oracle<Fake> {
        let limits = Limits {
            timeout: Duration::from_secs(10),
            memory_bytes: 1 << 30,
        };
        Oracle::new(Fake { calls: 0 }, Words::sample(["a", "b"], 7), limits)
    }

    #[test]
    fn a_repeat_is_answered_by_the_memo_up_to_canonical_form() {
        let mut oracle = oracle();
        assert_eq!(oracle.check(&f("a & !b")), Verdict::Holds);
        assert_eq!(oracle.check(&f("!b & a")), Verdict::Holds);
        assert_eq!(oracle.check(&f("!!(a & !b)")), Verdict::Holds);
        assert!(matches!(oracle.check(&f("G a")), Verdict::Fails(_)));
        assert!(matches!(oracle.check(&f("G a")), Verdict::Fails(_)));
        assert_eq!(oracle.backend.calls, 2);
        let counters = oracle.counters();
        assert_eq!(counters.queries, 5);
        assert_eq!(counters.memo_hits, 3);
        assert_eq!(counters.backend_calls, 2);
    }

    #[test]
    fn unknown_is_never_memoised() {
        let mut oracle = oracle();
        for _ in 0..3 {
            assert!(matches!(oracle.check(&f("F u")), Verdict::Unknown(_)));
        }
        assert_eq!(oracle.backend.calls, 3);
        let counters = oracle.counters();
        assert_eq!(counters.memo_hits, 0);
        assert_eq!(counters.backend_unknowns, 3);
    }

    #[test]
    fn a_failure_refutes_a_weaker_failing_candidate_without_the_back_end() {
        let mut oracle = oracle();
        let Verdict::Fails(run) = oracle.check(&f("G a")) else {
            panic!("G a fails on the model");
        };
        // G (a | b) is weaker than G a and fails on the same run.
        assert_eq!(oracle.check(&f("G (a | b)")), Verdict::Fails(run.clone()));
        assert_eq!(oracle.check(&f("F b")), Verdict::Fails(run));
        assert_eq!(oracle.backend.calls, 1);
        let counters = oracle.counters();
        assert_eq!(counters.lasso_hits, 2);
        assert_eq!(counters.backend_calls, 1);
        // A lasso never refutes a truth, which still reaches the back end.
        assert_eq!(oracle.check(&f("F !a")), Verdict::Holds);
        assert_eq!(oracle.backend.calls, 2);
    }

    #[test]
    fn a_truth_the_prefilter_rules_out_never_reaches_spot() {
        let mut oracle = oracle();
        // No stored truth yet, so no Spot call.
        assert_eq!(oracle.check(&f("a")), Verdict::Holds);
        // The fingerprint shows `a` does not imply `F G !b`, so this goes
        // to the back end with no Spot call either.
        assert_eq!(oracle.check(&f("F G !b")), Verdict::Holds);
        let counters = oracle.counters();
        assert_eq!(counters.spot_calls, 0);
        assert_eq!(counters.implication_hits, 0);
        assert_eq!(counters.backend_calls, 2);
    }

    /// Every stored truth: each one may imply `true`.
    fn truths(oracle: &Oracle<Fake>) -> Vec<Formula> {
        oracle.truths.may_imply(&Formula::tt()).cloned().collect()
    }

    #[test]
    fn spot_proves_from_a_truth_and_evicts_weaker_truths() {
        let on_path = std::env::var_os("PATH")
            .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("ltlfilt").is_file()));
        if !on_path {
            eprintln!("skipping spot_proves_from_a_truth_and_evicts_weaker_truths: no ltlfilt");
            return;
        }
        let mut oracle = oracle();
        assert_eq!(oracle.check(&f("a | b")), Verdict::Holds);
        // The fingerprint rules out `a | b` => `a & !b`, so the back end
        // answers; Spot then proves the new truth implies the old one.
        assert_eq!(oracle.check(&f("a & !b")), Verdict::Holds);
        assert_eq!(truths(&oracle), [f("a & !b")]);
        assert_eq!(oracle.counters().spot_calls, 1);
        // Spot proves this one from the stored truth.
        assert_eq!(oracle.check(&f("a | X b")), Verdict::Holds);
        let counters = oracle.counters();
        assert_eq!(counters.implication_hits, 1);
        assert_eq!(counters.spot_calls, 2);
        assert_eq!(counters.backend_calls, 2);
        assert_eq!(oracle.backend.calls, 2);
        // A truth Spot cannot prove implies the stored one does not evict it.
        assert_eq!(oracle.check(&f("F !a")), Verdict::Holds);
        assert_eq!(truths(&oracle), [f("a & !b"), f("F !a")]);
    }
}
