//! Maximality demotion, PEREDUR's `maximality_demotion`
//! (`src/genetic/demotion.cpp`): a candidate that holds moves one front
//! down when another candidate of the pool that holds strictly implies it.
//!
//! A weaker repair costs nothing on similarity and the same on status, so
//! without demotion it sits on the first front beside the repair it
//! weakens. GAWAIN's semantic objective is monotone under implication and
//! already ranks the stronger one at least as high, so the rule is measured
//! as an arm (`search_bench --demotion on`) and is off by default.
//!
//! Each direction ψ ⇒ χ between two holders first meets the fingerprint
//! test of [`Fingerprint::may_imply`], which refutes most non-implications
//! with no exec. Spot decides the rest in batches, and every answer is
//! memoised for the rest of the run, since the population changes by a few
//! candidates a generation. "Not proved" never demotes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::Scored;
use crate::ltl::Formula;
use crate::oracle::process::Limits;
use crate::oracle::{implications_each, Fingerprint, Semantics, Words};

/// The pairs in one `ltlfilt` exec, so one exec stays well under the
/// search's per-exec time limit.
const BATCH: usize = 128;
/// The seed of the fingerprint words. They only decide which pairs go to
/// Spot, never a verdict, so any fixed seed does.
const WORDS_SEED: u64 = 0;

/// What demotion did over a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DemotionCounts {
    /// Candidates demoted, summed over the generations.
    pub demoted: u64,
    /// Implication pairs sent to Spot.
    pub pairs: u64,
}

/// The fingerprints and Spot verdicts of one run.
pub struct Demoter {
    words: Words,
    prints: HashMap<Formula, Fingerprint>,
    proved: HashMap<(Formula, Formula), bool>,
    semantics: Semantics,
    limits: Limits,
    counts: DemotionCounts,
}

impl Demoter {
    /// A demoter with no verdicts yet, over the formula atoms `atoms`.
    /// Every Spot exec runs under `semantics` and `limits`.
    pub fn new(atoms: &[Arc<str>], semantics: Semantics, limits: Limits) -> Self {
        Demoter {
            words: Words::sample(atoms.iter().cloned(), WORDS_SEED),
            prints: HashMap::new(),
            proved: HashMap::new(),
            semantics,
            limits,
            counts: DemotionCounts::default(),
        }
    }

    /// What it did so far.
    pub fn counts(&self) -> DemotionCounts {
        self.counts
    }

    /// For each member of `pool`, whether it holds and another member that
    /// holds strictly implies it. Members equal up to
    /// [`Formula::canonical`] never demote each other.
    pub fn demoted(&mut self, pool: &[Scored]) -> Vec<bool> {
        let holders: Vec<usize> = (0..pool.len()).filter(|&i| pool[i].holds()).collect();
        let keys: Vec<Formula> = holders
            .iter()
            .map(|&i| pool[i].formula.canonical())
            .collect();
        for key in &keys {
            if !self.prints.contains_key(key) {
                let print = self.words.fingerprint(key);
                self.prints.insert(key.clone(), print);
            }
        }
        let may = |a: &Formula, b: &Formula| a != b && self.prints[a].may_imply(&self.prints[b]);
        let mut asked = HashSet::new();
        let mut ask: Vec<(Formula, Formula)> = Vec::new();
        for a in &keys {
            for b in &keys {
                let pair = (a.clone(), b.clone());
                if may(a, b) && !self.proved.contains_key(&pair) && asked.insert(pair.clone()) {
                    ask.push(pair);
                }
            }
        }
        for chunk in ask.chunks(BATCH) {
            let pairs: Vec<(&Formula, &Formula)> = chunk.iter().map(|(a, b)| (a, b)).collect();
            let (proved, _) = implications_each(&pairs, self.semantics, self.limits);
            for (pair, proved) in chunk.iter().zip(proved) {
                self.proved.insert(pair.clone(), proved);
            }
        }
        self.counts.pairs += ask.len() as u64;
        let implies = |a: &Formula, b: &Formula| {
            may(a, b) && self.proved.get(&(a.clone(), b.clone())) == Some(&true)
        };
        let mut demoted = vec![false; pool.len()];
        for (j, b) in keys.iter().enumerate() {
            if keys.iter().any(|a| implies(a, b) && !implies(b, a)) {
                demoted[holders[j]] = true;
            }
        }
        self.counts.demoted += demoted.iter().filter(|&&d| d).count() as u64;
        demoted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ltl::tests::f;
    use std::time::Duration;

    #[test]
    fn a_pool_the_fingerprints_separate_asks_spot_nothing() {
        // F a and F b are incomparable, and a sampled word shows it both
        // ways; `a` does not hold, so it is never compared. No pair reaches
        // Spot, so this runs without it. `tests/search.rs` covers the
        // Spot half.
        let limits = Limits {
            timeout: Duration::from_secs(1),
            memory_bytes: 1 << 20,
        };
        let atoms: Vec<Arc<str>> = ["a", "b"].into_iter().map(Arc::from).collect();
        let mut demoter = Demoter::new(&atoms, Semantics::Infinite, limits);
        let scored = |text: &str, status: f64| Scored {
            formula: f(text),
            objectives: vec![status, 0.5, 0.5],
        };
        let pool = vec![scored("F a", 1.0), scored("F b", 1.0), scored("a", 0.0)];
        assert_eq!(demoter.demoted(&pool), [false, false, false]);
        assert_eq!(demoter.counts(), DemotionCounts::default());
    }
}
