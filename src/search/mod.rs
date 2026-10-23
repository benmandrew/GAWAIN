//! The weakening search: a genetic loop over single LTL formulas, scored by
//! the oracle and selected under NSGA-II, then an output gate that keeps
//! only true, maximal weakenings of the input.
//!
//! One generation breeds one offspring per population slot (crossover, then
//! mutation, then `simplify`), filters them (repeats, bloat, validity),
//! scores the survivors, pools
//! them with their parents and keeps the population size by PEREDUR's
//! `nsga2-apportion` step. See `docs/dev/search.md`.

pub mod nsga2;
mod operators;
mod rng;

use std::collections::HashSet;
use std::sync::Arc;

pub use operators::{crossover, monotone_rewrite, mutate, Direction};
pub use rng::Rng;

use crate::ltl::{Formula, Node};
use crate::oracle::process::Limits;
use crate::oracle::{implications, Backend, Oracle, Verdict};

/// The chance that an offspring is crossed with a parent, PEREDUR's
/// `crossover_rate` default.
const CROSSOVER_RATE: f64 = 0.1;
/// The chance that an offspring is mutated, PEREDUR's `mutation_rate`.
const MUTATION_RATE: f64 = 1.0;
/// The largest offspring kept, as a multiple of the input's size, PEREDUR's
/// bloat-cap default.
const BLOAT_RATIO: usize = 2;

/// The index of the status objective: 1 when M satisfies the candidate.
pub const STATUS: usize = 0;
/// The index of the syntactic-similarity objective.
pub const SIMILARITY: usize = 1;

/// The size of a run.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Generations to breed.
    pub generations: usize,
    /// Individuals in the population, at least 1.
    pub population: usize,
}

/// A candidate with its objective vector, indexed by [`STATUS`] and
/// [`SIMILARITY`].
#[derive(Clone, Debug)]
pub struct Scored {
    /// The candidate φ'.
    pub formula: Formula,
    /// Its objectives, higher is better.
    pub objectives: Vec<f64>,
}

impl Scored {
    /// Whether the oracle proved M ⊨ φ'. An unknown verdict scores 0, so it
    /// reads as false here.
    pub fn holds(&self) -> bool {
        self.objectives[STATUS] == 1.0
    }
}

/// Scores `candidate` against the input `phi`. Model-counting similarity
/// to `phi` will join this list as a third objective.
pub fn score<B: Backend>(candidate: Formula, phi: &Formula, oracle: &mut Oracle<B>) -> Scored {
    let status = match oracle.check(&candidate) {
        Verdict::Holds => 1.0,
        // Unknown scores as a failure. The oracle never memoises it, so the
        // same formula is asked again if it comes back.
        Verdict::Fails(_) | Verdict::Unknown(_) => 0.0,
    };
    let similarity = candidate.syntactic_similarity(phi);
    Scored {
        formula: candidate,
        objectives: vec![status, similarity],
    }
}

/// Runs the search from `phi` and returns the final population, best first.
/// Mutation draws atoms from `phi`'s atoms only.
pub fn search<B: Backend>(
    phi: &Formula,
    oracle: &mut Oracle<B>,
    params: Params,
    spot_limits: Limits,
    rng: &mut Rng,
) -> Vec<Scored> {
    assert!(params.population >= 1, "the population needs a member");
    let atoms: Vec<Arc<str>> = phi.atoms().into_iter().collect();
    let seed = score(phi.clone(), phi, oracle);
    let mut population = select(vec![seed], params.population);
    for _ in 0..params.generations {
        let offspring = filter(breed(&population, &atoms, rng), phi);
        let offspring = drop_valid(offspring, spot_limits);
        let scored: Vec<Scored> = offspring
            .into_iter()
            .map(|candidate| score(candidate, phi, oracle))
            .collect();
        population.extend(scored);
        population = select(population, params.population);
    }
    population
}

/// One offspring per slot of the best-first `parents`. Each starts as its
/// slot's parent, is crossed with a parent drawn from the whole population
/// at [`CROSSOVER_RATE`], mutated at [`MUTATION_RATE`], then simplified.
/// Crossover and mutation interleave per slot, so this stays one pass.
fn breed(parents: &[Scored], atoms: &[Arc<str>], rng: &mut Rng) -> Vec<Formula> {
    let mut offspring = Vec::with_capacity(parents.len());
    for parent in parents {
        let mut child = parent.formula.clone();
        if rng.chance(CROSSOVER_RATE) {
            let partner = rng.below(parents.len());
            child = crossover(&child, &parents[partner].formula, rng);
        }
        if rng.chance(MUTATION_RATE) {
            child = mutate(&child, atoms, rng);
        }
        offspring.push(child.simplify());
    }
    offspring
}

/// The filters before scoring, in PEREDUR's order: drop repeats up to
/// [`Formula::canonical`], then anything over [`BLOAT_RATIO`] times `phi`'s
/// size, then anything `simplify` folds to `true`, the trivial weakening.
/// A dropped candidate costs no oracle call.
pub fn filter(offspring: Vec<Formula>, phi: &Formula) -> Vec<Formula> {
    let cap = BLOAT_RATIO * phi.n_subformulae();
    let mut seen = HashSet::new();
    offspring
        .into_iter()
        .filter(|candidate| seen.insert(candidate.canonical()))
        .filter(|candidate| candidate.n_subformulae() <= cap)
        .filter(|candidate| !matches!(candidate.simplify().node(), Node::True))
        .collect()
}

/// Drops the candidates Spot proves valid, in one `ltlfilt` exec, as
/// PEREDUR's vacuity filter asks a solver whether a guarantee is valid. The
/// syntactic check in [`filter`] misses most valid offspring: on the first
/// runs every repair the gate reported was valid, such as
/// `G (ack -> X φ | ack)` and `a W (ψ | !a)`, and a valid candidate holds on
/// every model, so it crowds the front. A batch with no answer drops
/// nothing, so the verdict never depends on machine load.
pub fn drop_valid(offspring: Vec<Formula>, limits: Limits) -> Vec<Formula> {
    let tt = Formula::tt();
    let pairs: Vec<(&Formula, &Formula)> = offspring.iter().map(|c| (&tt, c)).collect();
    let Ok(valid) = implications(&pairs, limits) else {
        return offspring;
    };
    offspring
        .into_iter()
        .zip(valid)
        .filter(|(_, valid)| !valid)
        .map(|(candidate, _)| candidate)
        .collect()
}

/// PEREDUR's `nsga2-apportion` survivor step: drop repeats up to
/// [`Formula::canonical`], sort by the crowded-comparison order, then keep
/// the best `size` or replicate the distinct ones up to `size`.
pub fn select(pool: Vec<Scored>, size: usize) -> Vec<Scored> {
    let mut seen = HashSet::new();
    let distinct: Vec<Scored> = pool
        .into_iter()
        .filter(|s| seen.insert(s.formula.canonical()))
        .collect();
    let objectives: Vec<Vec<f64>> = distinct.iter().map(|s| s.objectives.clone()).collect();
    let (order, ranks) = nsga2::crowded_order(&objectives);
    if order.len() >= size {
        return order
            .into_iter()
            .take(size)
            .map(|i| distinct[i].clone())
            .collect();
    }
    let sorted_ranks: Vec<usize> = order.iter().map(|&i| ranks[i]).collect();
    let copies = nsga2::apportion(&sorted_ranks, size);
    order
        .iter()
        .zip(copies)
        .flat_map(|(&i, n)| std::iter::repeat_n(distinct[i].clone(), n))
        .collect()
}

/// A reported weakening.
#[derive(Clone, Debug, PartialEq)]
pub struct Repair {
    /// φ', with M ⊨ φ' and φ ⇒ φ' both proved.
    pub formula: Formula,
    /// Its syntactic similarity to φ.
    pub similarity: f64,
}

/// What the output gate kept, and why any `ltlfilt` batch gave no answer.
#[derive(Debug, Default)]
pub struct Gate {
    /// The maximal repairs, most similar to φ first.
    pub repairs: Vec<Repair>,
    /// One reason per Spot batch that failed. A failed weakening batch drops
    /// every candidate; a failed maximality batch drops none.
    pub errors: Vec<String>,
}

/// The output gate. From `population`, keeps the distinct candidates that M
/// satisfies, that `phi` implies and that Spot does not prove valid (one
/// batch; if it fails, no weakening is proved and every candidate drops),
/// then only the maximal
/// ones: a candidate implied by another survivor is dropped (one more batch).
/// Of two equivalent survivors the one more similar to φ stays, then the
/// smaller in formula order. "Not proved" never counts as proved.
pub fn gate(phi: &Formula, population: &[Scored], limits: Limits) -> Gate {
    let mut gate = Gate::default();
    let mut seen = HashSet::new();
    let holding: Vec<&Scored> = population
        .iter()
        .filter(|s| s.holds() && seen.insert(s.formula.canonical()))
        .collect();
    // Pair 2k asks whether φ implies candidate k, pair 2k + 1 whether
    // candidate k is valid, which the search's filter may have let through
    // on a failed batch.
    let tt = Formula::tt();
    let pairs: Vec<(&Formula, &Formula)> = holding
        .iter()
        .flat_map(|s| [(phi, &s.formula), (&tt, &s.formula)])
        .collect();
    let proved = implications(&pairs, limits).unwrap_or_else(|e| {
        gate.errors.push(e);
        vec![false; pairs.len()]
    });
    let mut candidates: Vec<Repair> = holding
        .iter()
        .zip(proved.chunks(2))
        .filter(|(_, proved)| proved[0] && !proved[1])
        .map(|(s, _)| Repair {
            formula: s.formula.clone(),
            similarity: s.objectives[SIMILARITY],
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.similarity
            .total_cmp(&a.similarity)
            .then_with(|| a.formula.cmp(&b.formula))
    });
    // pairs[k] asks whether candidate i implies candidate j, for every i ≠ j.
    let n = candidates.len();
    let index: Vec<(usize, usize)> = (0..n)
        .flat_map(|i| (0..n).filter(move |&j| j != i).map(move |j| (i, j)))
        .collect();
    let pairs: Vec<(&Formula, &Formula)> = index
        .iter()
        .map(|&(i, j)| (&candidates[i].formula, &candidates[j].formula))
        .collect();
    let proved = implications(&pairs, limits).unwrap_or_else(|e| {
        gate.errors.push(e);
        vec![false; pairs.len()]
    });
    let mut implies = vec![vec![false; n]; n];
    for (&(i, j), proved) in index.iter().zip(proved) {
        implies[i][j] = proved;
    }
    // j is dropped when a strictly stronger survivor implies it, or an
    // equivalent one that sorts before it.
    let dropped: Vec<bool> = (0..n)
        .map(|j| (0..n).any(|i| implies[i][j] && (i < j || !implies[j][i])))
        .collect();
    gate.repairs = candidates
        .into_iter()
        .zip(dropped)
        .filter(|(_, dropped)| !dropped)
        .map(|(repair, _)| repair)
        .collect();
    gate
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ltl::tests::f;

    fn scored(text: &str, status: f64, similarity: f64) -> Scored {
        Scored {
            formula: f(text),
            objectives: vec![status, similarity],
        }
    }

    #[test]
    fn filter_drops_repeats_bloat_and_tautologies() {
        let phi = f("G (a -> X b)"); // size 5, cap 10
        let kept = filter(
            vec![
                f("G (a -> F b)"),
                f("G (a -> F b)"),
                f("G (!!a -> F b)"),
                f("G (a -> X b) | (a & b & F G (a | b))"),
                f("G (a | !a)"),
                f("a -> a"),
                f("G a"),
            ],
            &phi,
        );
        assert_eq!(kept, [f("G (a -> F b)"), f("G a")]);
    }

    #[test]
    fn select_replicates_distinct_survivors_by_rank() {
        let pool = vec![
            scored("a", 0.0, 1.0),
            scored("b", 1.0, 0.2),
            scored("!!a", 0.0, 1.0),
            scored("c", 0.0, 0.1),
        ];
        let next = select(pool, 7);
        let names: Vec<String> = next.iter().map(|s| s.formula.to_string()).collect();
        // Front 0 is {a, b} (the repeat !!a is dropped), front 1 is {c}.
        // Spare 4 by weights 1, 1, 1/2: 1.6, 1.6, 0.8.
        assert_eq!(names, ["a", "a", "a", "b", "b", "c", "c"]);
    }

    #[test]
    fn select_truncates_a_large_pool_in_crowded_order() {
        let pool = vec![
            scored("a", 0.0, 0.5),
            scored("b", 1.0, 0.2),
            scored("c", 0.0, 0.9),
        ];
        let next = select(pool, 2);
        let names: Vec<String> = next.iter().map(|s| s.formula.to_string()).collect();
        assert_eq!(names, ["b", "c"]);
    }

    #[test]
    fn holds_reads_the_status_objective() {
        assert!(scored("a", 1.0, 0.0).holds());
        assert!(!scored("a", 0.0, 1.0).holds());
    }
}
