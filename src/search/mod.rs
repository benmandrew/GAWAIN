//! The weakening search: a genetic loop over single LTL formulas, scored by
//! the oracle and selected under NSGA-II, then an output gate that keeps
//! only true, maximal weakenings of the input.
//!
//! One generation breeds one offspring per population slot (crossover, then
//! mutation, then `simplify`), filters them (repeats, bloat, validity),
//! scores the survivors (status, syntactic and model-counting similarity),
//! pools them with their parents, regrades the pool's status against the
//! runs of M stored so far ([`grade`]), and keeps the population size by
//! PEREDUR's `nsga2-apportion` step. See `docs/dev/search.md`.
//!
//! [`random_search`] is the baseline without selection: the same breeding,
//! filters and scores, with each parent drawn uniformly from every
//! candidate kept so far, and one NSGA-II cut at the end.

pub mod grade;
pub mod nsga2;
mod operators;
mod rng;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

pub use grade::{GradeCounts, Grader, Status};
pub use operators::{crossover, monotone_rewrite, mutate, Direction};
pub use rng::Rng;

use crate::count::Counter;
use crate::ltl::{BinaryOp, Formula, Node, UnaryOp};
use crate::oracle::process::Limits;
use crate::oracle::{implications_each, simplifications, Backend, Oracle, Semantics, Verdict};

/// The chance that an offspring is mutated, PEREDUR's `mutation_rate`.
const MUTATION_RATE: f64 = 1.0;
/// The largest offspring kept, as a multiple of the input's size, PEREDUR's
/// bloat-cap default.
const BLOAT_RATIO: usize = 2;

/// The index of the status objective: 1 when M satisfies the candidate,
/// below 1 otherwise (see [`Status`]).
pub const STATUS: usize = 0;
/// The index of the syntactic-similarity objective.
pub const SIMILARITY: usize = 1;
/// The index of the model-counting similarity objective (see
/// [`crate::count`]).
pub const SEMANTIC: usize = 2;

/// The size of a run.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Generations to breed.
    pub generations: usize,
    /// Individuals in the population, at least 1.
    pub population: usize,
    /// How a candidate that does not hold is scored.
    pub status: Status,
    /// The chance that an offspring is crossed with a parent, in [0, 1].
    /// PEREDUR's `crossover_rate` default is 0.1.
    pub crossover_rate: f64,
    /// The chance that a mutation is a [`monotone_rewrite`], in [0, 1].
    /// PEREDUR's `p_monotone` default is 0.25.
    pub p_monotone: f64,
    /// Whether a candidate that holds but that φ is not proved to imply
    /// scores status 0, as one that fails ([`drop_valid`]).
    pub weakening: bool,
}

impl Params {
    /// A run of `generations` rounds of `population`, with the default
    /// status, PEREDUR's default rates and the weakening filter.
    pub fn new(generations: usize, population: usize) -> Self {
        Params {
            generations,
            population,
            status: Status::default(),
            crossover_rate: 0.1,
            p_monotone: 0.25,
            weakening: true,
        }
    }
}

/// A candidate with its objective vector, indexed by [`STATUS`],
/// [`SIMILARITY`] and [`SEMANTIC`].
#[derive(Clone, Debug)]
pub struct Scored {
    /// The candidate φ'.
    pub formula: Formula,
    /// Its objectives, higher is better.
    pub objectives: Vec<f64>,
}

impl Scored {
    /// Whether the oracle proved M ⊨ φ'. Any other verdict scores below 1,
    /// so it reads as false here.
    pub fn holds(&self) -> bool {
        self.objectives[STATUS] == 1.0
    }
}

/// Scores `candidates` against the input `phi`: the oracle's status,
/// syntactic similarity and model-counting similarity. The counts run on a
/// second thread while the oracle checks the candidates one by one, so
/// their `ltl2tgba` exec overlaps the model checker's.
pub fn score<B: Backend>(
    candidates: Vec<Formula>,
    phi: &Formula,
    oracle: &mut Oracle<B>,
    counter: &mut Counter,
) -> Vec<Scored> {
    let (semantic, statuses, wait) = std::thread::scope(|scope| {
        let counting = scope.spawn(|| counter.similarities(&candidates));
        let statuses: Vec<f64> = candidates
            .iter()
            .map(|candidate| match oracle.check(candidate) {
                Verdict::Holds => 1.0,
                // Unknown scores as a failure. The oracle memoises it, so
                // a formula that comes back costs nothing again.
                Verdict::Fails(_) | Verdict::Unknown(_) => 0.0,
            })
            .collect();
        let checked = Instant::now();
        let semantic = counting.join().expect("the counting thread");
        (semantic, statuses, checked.elapsed())
    });
    counter.record_wait(wait);
    candidates
        .into_iter()
        .zip(statuses)
        .zip(semantic)
        .map(|((candidate, status), semantic)| {
            let similarity = candidate.syntactic_similarity(phi);
            Scored {
                formula: candidate,
                objectives: vec![status, similarity, semantic],
            }
        })
        .collect()
}

/// What a search spent, in candidates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    /// Candidates bred, φ included: 1 + G × P for either search.
    pub bred: usize,
    /// Of those, the candidates that passed [`filter`] and were scored.
    pub scored: usize,
}

/// Runs the search from `phi` and returns the final population, best first,
/// with the cost of grading the status and the candidates spent. Mutation
/// draws atoms from `phi`'s atoms only.
pub fn search<B: Backend>(
    phi: &Formula,
    oracle: &mut Oracle<B>,
    counter: &mut Counter,
    invariant: &Formula,
    params: Params,
    spot_limits: Limits,
    rng: &mut Rng,
) -> (Vec<Scored>, GradeCounts, Budget) {
    assert!(params.population >= 1, "the population needs a member");
    let atoms: Vec<Arc<str>> = phi.atoms().into_iter().collect();
    let mut grader = Grader::new(phi);
    // The stored lassos only grow, so the whole pool is regraded against
    // one set before it is ranked.
    let mut regrade = |pool: &mut Vec<Scored>, oracle: &Oracle<B>| {
        if params.status.graded() {
            grader.rescore(pool, oracle.lassos());
        }
    };
    let mut population = score(vec![phi.clone()], phi, oracle, counter);
    let mut budget = Budget { bred: 1, scored: 1 };
    regrade(&mut population, oracle);
    population = select(population, params.population, params.status);
    for _ in 0..params.generations {
        let parents: Vec<&Scored> = population.iter().collect();
        let offspring = filter(breed(&parents, &population, &atoms, &params, rng), phi);
        budget.bred += parents.len();
        budget.scored += offspring.len();
        let scored = score(offspring, phi, oracle, counter);
        population.extend(drop_valid(
            scored,
            invariant,
            params.weakening.then_some(phi),
            oracle.semantics(),
            spot_limits,
        ));
        regrade(&mut population, oracle);
        population = select(population, params.population, params.status);
    }
    (population, grader.counts(), budget)
}

/// The random-mutation baseline: [`search`] with no selection during the
/// run. Each of G rounds breeds P offspring, the same way, from parents
/// drawn uniformly from the archive, the distinct candidates kept so far
/// (φ first). The offspring pass the same [`filter`], scores and
/// [`drop_valid`], and the new distinct ones join the archive. At the end
/// the archive is regraded once and cut to P by [`select`]. Both searches
/// breed 1 + G × P candidates; how many are scored depends on the filter.
pub fn random_search<B: Backend>(
    phi: &Formula,
    oracle: &mut Oracle<B>,
    counter: &mut Counter,
    invariant: &Formula,
    params: Params,
    spot_limits: Limits,
    rng: &mut Rng,
) -> (Vec<Scored>, GradeCounts, Budget) {
    assert!(params.population >= 1, "the population needs a member");
    let atoms: Vec<Arc<str>> = phi.atoms().into_iter().collect();
    let mut grader = Grader::new(phi);
    let mut archive = score(vec![phi.clone()], phi, oracle, counter);
    let mut seen: HashSet<Formula> = archive.iter().map(|s| s.formula.canonical()).collect();
    let mut budget = Budget { bred: 1, scored: 1 };
    for _ in 0..params.generations {
        let drawn = draw_parents(archive.len(), params.population, rng);
        let parents: Vec<&Scored> = drawn.into_iter().map(|i| &archive[i]).collect();
        let offspring = filter(breed(&parents, &archive, &atoms, &params, rng), phi);
        budget.bred += parents.len();
        budget.scored += offspring.len();
        let scored = score(offspring, phi, oracle, counter);
        let kept = drop_valid(
            scored,
            invariant,
            params.weakening.then_some(phi),
            oracle.semantics(),
            spot_limits,
        );
        archive.extend(
            kept.into_iter()
                .filter(|s| seen.insert(s.formula.canonical())),
        );
    }
    if params.status.graded() {
        grader.rescore(&mut archive, oracle.lassos());
    }
    let population = select(archive, params.population, params.status);
    (population, grader.counts(), budget)
}

/// `n` indices drawn uniformly, with replacement, from `[0, len)`.
fn draw_parents(len: usize, n: usize, rng: &mut Rng) -> Vec<usize> {
    (0..n).map(|_| rng.below(len)).collect()
}

/// One offspring per slot of `parents`. Each starts as its slot's parent,
/// is crossed with a partner drawn uniformly from `partners` at
/// `params.crossover_rate`, mutated at [`MUTATION_RATE`] with monotone
/// share `params.p_monotone`, then simplified.
/// Crossover and mutation interleave per slot, so this stays one pass.
fn breed(
    parents: &[&Scored],
    partners: &[Scored],
    atoms: &[Arc<str>],
    params: &Params,
    rng: &mut Rng,
) -> Vec<Formula> {
    let mut offspring = Vec::with_capacity(parents.len());
    for parent in parents {
        let mut child = parent.formula.clone();
        if rng.chance(params.crossover_rate) {
            let partner = rng.below(partners.len());
            child = crossover(&child, &partners[partner].formula, rng);
        }
        if rng.chance(MUTATION_RATE) {
            child = mutate(&child, atoms, params.p_monotone, rng);
        }
        offspring.push(child.simplify());
    }
    offspring
}

/// The filters before scoring, in PEREDUR's order: drop repeats up to
/// [`Formula::canonical`], then anything over `BLOAT_RATIO` times `phi`'s
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

/// Drops the candidates Spot proves valid modulo the encoding `invariant`
/// (that `invariant -> φ'` is valid, so `invariant & !φ'` is
/// unsatisfiable), in one `ltlfilt` exec, as PEREDUR's vacuity filter asks
/// a solver whether a guarantee is valid. The syntactic check in [`filter`]
/// misses most valid offspring: on the first runs every repair the gate
/// reported was valid, such as `G (ack -> X φ | ack)` and `a W (ψ | !a)`,
/// and a valid candidate holds on every model, so it crowds the front. With
/// no exclusive groups the invariant is `true` and this is plain validity.
/// A candidate with no answer is kept, so the verdict never depends on
/// machine load. Validity is under `semantics`, the model's.
///
/// M satisfies the invariant, so a candidate that fails on M is not valid
/// modulo it, and only the scored candidates that hold go to Spot. On
/// large LTLf formulas validity is the expensive question, and this asks
/// it of fewer formulas; see "Slow implication batches" in
/// `docs/dev/trace-sets.md`.
///
/// With `weakens` = φ, the same exec also asks whether φ implies each
/// candidate that holds, and one that φ is not proved to imply scores
/// status 0, as a candidate that fails on M does. The gate reports only
/// proved weakenings, so such a candidate can never be a repair, but it
/// still breeds. Here an unanswered pair demotes its candidate, as an
/// unknown model-checking verdict scores below 1.
pub fn drop_valid(
    scored: Vec<Scored>,
    invariant: &Formula,
    weakens: Option<&Formula>,
    semantics: Semantics,
    limits: Limits,
) -> Vec<Scored> {
    let holding: Vec<usize> = (0..scored.len()).filter(|&i| scored[i].holds()).collect();
    let width = 1 + usize::from(weakens.is_some());
    let pairs: Vec<(&Formula, &Formula)> = holding
        .iter()
        .flat_map(|&i| {
            let formula = &scored[i].formula;
            std::iter::once((invariant, formula)).chain(weakens.map(|phi| (phi, formula)))
        })
        .collect();
    let (proved, _) = implications_each(&pairs, semantics, limits);
    let mut valid = vec![false; scored.len()];
    let mut weakening = vec![true; scored.len()];
    for (&i, proved) in holding.iter().zip(proved.chunks(width)) {
        valid[i] = proved[0];
        weakening[i] = proved.get(1).copied().unwrap_or(true);
    }
    scored
        .into_iter()
        .enumerate()
        .filter(|&(i, _)| !valid[i])
        .map(|(i, mut s)| {
            if !weakening[i] {
                s.objectives[STATUS] = 0.0;
            }
            s
        })
        .collect()
}

/// The encoding invariant I of `groups` of mutually exclusive atoms:
/// `G` of `!(a & b)` over every pair within a group, or `true` when there
/// is no pair. A candidate that I implies is trivial on any model whose
/// atoms keep the groups exclusive, such as the values of one SMV variable.
pub fn invariant(groups: &[Vec<Arc<str>>]) -> Formula {
    let exclusions: Vec<Formula> = groups
        .iter()
        .flat_map(|group| {
            group.iter().enumerate().flat_map(move |(i, a)| {
                group[i + 1..].iter().map(move |b| {
                    let both = Formula::binary(BinaryOp::And, Formula::atom(a), Formula::atom(b));
                    Formula::unary(UnaryOp::Not, both)
                })
            })
        })
        .collect();
    match exclusions
        .into_iter()
        .reduce(|l, r| Formula::binary(BinaryOp::And, l, r))
    {
        Some(all) => Formula::unary(UnaryOp::Globally, all),
        None => Formula::tt(),
    }
}

/// PEREDUR's `nsga2-apportion` survivor step: drop repeats up to
/// [`Formula::canonical`], sort by the crowded-comparison order, then keep
/// the best `size` or replicate the distinct ones up to `size`. When
/// `status` ranks feasible candidates first, a candidate that holds is
/// feasible and every other is not, so every holder ranks above every other
/// candidate (see [`nsga2::constrained_dominates`]); otherwise all are
/// feasible. When the pool is cut, [`Status::reserved`] slots go to the
/// best candidates that do not hold, as far as there are any, and the
/// holders fill the rest; a pool short of either kind leaves its slots to
/// the other.
pub fn select(pool: Vec<Scored>, size: usize, status: Status) -> Vec<Scored> {
    let mut seen = HashSet::new();
    let distinct: Vec<Scored> = pool
        .into_iter()
        .filter(|s| seen.insert(s.formula.canonical()))
        .collect();
    let objectives: Vec<Vec<f64>> = distinct.iter().map(|s| s.objectives.clone()).collect();
    let feasible: Vec<bool> = (distinct.iter())
        .map(|s| !status.feasible_first() || s.holds())
        .collect();
    let (order, ranks) = nsga2::crowded_order(&objectives, &feasible);
    if order.len() >= size {
        // Fronts never mix the two kinds, so each kind's part of `order`
        // is its own crowded order.
        let (proved, unproved): (Vec<usize>, Vec<usize>) =
            order.iter().partition(|&&i| distinct[i].holds());
        let reserve = status.reserved(size);
        let kept: Vec<usize> = if reserve == 0 {
            order.into_iter().take(size).collect()
        } else {
            let others = reserve
                .min(unproved.len())
                .max(size.saturating_sub(proved.len()));
            let holders = size - others;
            (proved.into_iter().take(holders))
                .chain(unproved.into_iter().take(others))
                .collect()
        };
        return kept.into_iter().map(|i| distinct[i].clone()).collect();
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
    /// Its model-counting similarity to φ.
    pub semantic: f64,
    /// Where it is vacuous on M. A flag for the reader, never a reason to
    /// drop it.
    pub vacuity: Vacuity,
    /// An equivalent form of `formula` for the reader, from Spot's
    /// simplifier (see [`simplified`]). Nothing else reads it.
    pub simplified: String,
}

/// The subformula occurrences in which a repair is vacuous on M (see
/// [`vacuity`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vacuity {
    /// Each occurrence ψ whose witness M satisfies, in pre-order. Distinct
    /// occurrences with the same text appear once.
    pub subformulae: Vec<Formula>,
    /// Whether the oracle answered `Unknown` for some witness.
    pub unknown: bool,
}

/// What the output gate kept, and why any `ltlfilt` batch gave no answer.
#[derive(Debug, Default)]
pub struct Gate {
    /// The maximal repairs, most similar to φ first.
    pub repairs: Vec<Repair>,
    /// One reason per Spot batch that failed. A failed weakening batch drops
    /// every candidate; a failed maximality batch drops none; a failed
    /// simplification batch leaves every repair unsimplified.
    pub errors: Vec<String>,
    /// What the vacuity flags cost.
    pub vacuity: VacuityCounts,
}

/// The cost of the gate's vacuity flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VacuityCounts {
    /// Witnesses sent to the oracle.
    pub queries: u64,
    /// Witnesses the oracle answered `Unknown`.
    pub unknowns: u64,
}

/// Where `candidate` is vacuous on M (Beer, Ben-David, Eisner and Rodeh;
/// Kupferman and Vardi): each subformula occurrence ψ whose vacuity
/// witness, the candidate with ψ replaced by the constant that makes it
/// stronger, M satisfies. A witness that folds to `true` holds with no
/// query. Every witness is asked; an unknown verdict is counted and shows
/// nothing about its ψ.
pub fn vacuity<B: Backend>(
    candidate: &Formula,
    oracle: &mut Oracle<B>,
    counts: &mut VacuityCounts,
) -> Vacuity {
    let mut found = Vacuity::default();
    for (subformula, witness) in operators::vacuity_witnesses(candidate) {
        let holds = matches!(witness.node(), Node::True) || {
            counts.queries += 1;
            match oracle.check(&witness) {
                Verdict::Holds => true,
                Verdict::Fails(_) => false,
                Verdict::Unknown(_) => {
                    counts.unknowns += 1;
                    found.unknown = true;
                    false
                }
            }
        };
        if holds && !found.subformulae.contains(&subformula) {
            found.subformulae.push(subformula);
        }
    }
    found
}

/// The output gate. From `population`, keeps the distinct candidates that M
/// satisfies, that `phi` implies and that Spot does not prove valid modulo
/// the encoding `invariant` (one batch under `limits`; a candidate whose
/// pair Spot cannot decide drops). Then it keeps only the maximal ones: a
/// candidate implied by another survivor is dropped (one more batch, under
/// `pruning`, since an undecided pair only keeps a candidate). Of two
/// equivalent survivors the one more similar to φ stays, then the smaller
/// in formula order. "Not proved" never counts as proved. Last, it flags
/// where each repair is [`vacuity`]-vacuous on M, through `oracle`, and
/// gives each its [`simplified`] form, in one more batch under `limits`.
/// Every implication is decided under the oracle's [`Semantics`].
pub fn gate<B: Backend>(
    phi: &Formula,
    population: &[Scored],
    invariant: &Formula,
    oracle: &mut Oracle<B>,
    limits: Limits,
    pruning: Limits,
) -> Gate {
    let mut gate = Gate::default();
    let semantics = oracle.semantics();
    let mut seen = HashSet::new();
    let holding: Vec<&Scored> = population
        .iter()
        .filter(|s| s.holds() && seen.insert(s.formula.canonical()))
        .collect();
    // Pair 2k asks whether φ implies candidate k, pair 2k + 1 whether
    // candidate k is valid modulo the invariant, which the search's filter
    // may have let through on a failed batch.
    let pairs: Vec<(&Formula, &Formula)> = holding
        .iter()
        .flat_map(|s| [(phi, &s.formula), (invariant, &s.formula)])
        .collect();
    let (proved, error) = implications_each(&pairs, semantics, limits);
    gate.errors.extend(error);
    let mut candidates: Vec<Repair> = holding
        .iter()
        .zip(proved.chunks(2))
        .filter(|(_, proved)| proved[0] && !proved[1])
        .map(|(s, _)| Repair {
            formula: s.formula.clone(),
            similarity: s.objectives[SIMILARITY],
            semantic: s.objectives[SEMANTIC],
            vacuity: Vacuity::default(),
            simplified: String::new(),
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
    let (proved, error) = implications_each(&pairs, semantics, pruning);
    gate.errors.extend(error);
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
    for repair in &mut gate.repairs {
        repair.vacuity = vacuity(&repair.formula, oracle, &mut gate.vacuity);
    }
    // Spot's rewrite keeps equivalence over infinite words only, so under
    // LTLf every repair stands as it is.
    let formulas: Vec<&Formula> = gate.repairs.iter().map(|r| &r.formula).collect();
    let texts = match semantics {
        Semantics::Infinite => simplifications(&formulas, limits).unwrap_or_else(|error| {
            gate.errors.push(error);
            Vec::new()
        }),
        Semantics::Finite => Vec::new(),
    };
    for (i, repair) in gate.repairs.iter_mut().enumerate() {
        repair.simplified = simplified(&repair.formula, texts.get(i).map(String::as_str));
    }
    gate
}

/// How a repair reads once simplified, from `spot`, Spot's text for it.
/// Text that parses is printed as the rest of the output prints formulas,
/// but only when it has fewer nodes, since Spot also spells `a -> b` as the
/// larger `!a | b`; text that does not parse is kept as Spot wrote it. With
/// no shorter text the repair is its own simplified form.
pub fn simplified(formula: &Formula, spot: Option<&str>) -> String {
    match spot.map(|text| (text, text.parse::<Formula>())) {
        Some((_, Ok(shorter))) if shorter.n_subformulae() < formula.n_subformulae() => {
            shorter.to_string()
        }
        Some((text, Err(_))) => text.to_owned(),
        _ => formula.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ltl::tests::f;
    use crate::oracle::{Lasso, Valuation, Words};

    /// A model whose only run alternates `x` and `y` forever, so `x & y`
    /// never holds, as two values of one state variable never do.
    struct Alternating;

    impl Backend for Alternating {
        fn check(&mut self, formula: &Formula) -> Verdict {
            let at = |name: &str| -> Valuation { [Arc::from(name)].into_iter().collect() };
            let run = Lasso::new(vec![], vec![at("x"), at("y")]).unwrap();
            if run.satisfies(formula) {
                Verdict::Holds
            } else {
                Verdict::Fails(run)
            }
        }
    }

    #[test]
    fn an_antecedent_unsatisfiable_on_the_model_is_vacuous() {
        let limits = Limits {
            timeout: std::time::Duration::from_secs(10),
            memory_bytes: 1 << 30,
        };
        let mut oracle = Oracle::new(Alternating, Words::sample(["x", "y", "z"], 3), limits);
        let mut counts = VacuityCounts::default();
        let candidate = f("G (x & y -> X z)");
        assert_eq!(oracle.check(&candidate), Verdict::Holds);
        // G X z, G (y -> X z) and G (x -> X z) fail; G !(x & y), the
        // witness for both X z and z, holds.
        let found = vacuity(&candidate, &mut oracle, &mut counts);
        assert_eq!(found.subformulae, [f("X z"), f("z")]);
        assert!(!found.unknown);
        assert_eq!(counts.queries, 5);
        // The control holds on the run and every witness fails.
        let control = f("G (x -> X y)");
        assert_eq!(oracle.check(&control), Verdict::Holds);
        assert_eq!(
            vacuity(&control, &mut oracle, &mut counts),
            Vacuity::default()
        );
        assert_eq!(counts.unknowns, 0);
    }

    #[test]
    fn a_simplified_repair_is_shorter_and_in_gawain_syntax() {
        let repair = f("G (a -> !a)");
        assert_eq!(simplified(&repair, None), repair.to_string());
        assert_eq!(simplified(&repair, Some("G!a")), f("G !a").to_string());
        let implication = f("G (a -> b)");
        assert_eq!(
            simplified(&implication, Some("G(!a | b)")),
            implication.to_string()
        );
        assert_eq!(simplified(&repair, Some("G {a}")), "G {a}");
    }

    #[test]
    fn the_invariant_excludes_pairs_within_each_group() {
        let group = |names: &[&str]| names.iter().map(|&n| Arc::from(n)).collect::<Vec<_>>();
        assert_eq!(invariant(&[]), Formula::tt());
        assert_eq!(invariant(&[group(&["a"])]), Formula::tt());
        assert_eq!(
            invariant(&[group(&["a", "b", "c"]), group(&["p", "q"])]),
            f("G (!(a & b) & !(a & c) & !(b & c) & !(p & q))")
        );
    }

    fn scored(text: &str, status: f64, similarity: f64) -> Scored {
        Scored {
            formula: f(text),
            objectives: vec![status, similarity, similarity],
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
        let next = select(pool, 7, Status::Binary);
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
        let next = select(pool, 2, Status::Binary);
        let names: Vec<String> = next.iter().map(|s| s.formula.to_string()).collect();
        assert_eq!(names, ["b", "c"]);
    }

    #[test]
    fn feasible_first_selection_keeps_holders_over_graded_failures() {
        // Graded failures at 0.5 and 0.8 are not dominated by the holder b
        // under plain NSGA-II, so they take the two slots ahead of it.
        let pool = || {
            vec![
                scored("a", 0.5, 0.9),
                scored("b", 1.0, 0.2),
                scored("c", 0.8, 0.6),
                scored("d", 1.0, 0.1),
            ]
        };
        let names = |next: Vec<Scored>| -> Vec<String> {
            next.iter().map(|s| s.formula.to_string()).collect()
        };
        assert_eq!(names(select(pool(), 2, Status::Binary)), ["a", "b"]);
        assert_eq!(names(select(pool(), 2, Status::GradedFeasible)), ["b", "d"]);
        assert_eq!(
            names(select(pool(), 3, Status::GradedFeasible)),
            ["b", "d", "a"]
        );
    }

    #[test]
    fn a_reserve_keeps_failures_beside_the_holders() {
        // Four holders and three failures; φ is the failure with both
        // similarities at 1.
        let pool = || {
            vec![
                scored("h1", 1.0, 0.4),
                scored("h2", 1.0, 0.3),
                scored("h3", 1.0, 0.2),
                scored("h4", 1.0, 0.1),
                scored("phi", 0.0, 1.0),
                scored("c", 0.5, 0.8),
                scored("e", 0.2, 0.5),
            ]
        };
        let names = |next: Vec<Scored>| -> Vec<String> {
            next.iter().map(|s| s.formula.to_string()).collect()
        };
        // Without a reserve the holders take every slot.
        assert_eq!(
            names(select(pool(), 4, Status::GradedFeasible)),
            ["h1", "h2", "h3", "h4"]
        );
        // ⌈4 / 2⌉ = 2 slots go to the failures on their own front 0, φ and
        // c; e is dominated by c.
        assert_eq!(
            names(select(pool(), 4, Status::GradedReserve(2))),
            ["h1", "h2", "phi", "c"]
        );
        // ⌈6 / 4⌉ = 2, so four holders.
        assert_eq!(
            names(select(pool(), 6, Status::GradedReserve(4))),
            ["h1", "h2", "h3", "h4", "phi", "c"]
        );
    }

    #[test]
    fn a_short_kind_leaves_its_slots_to_the_other() {
        let names = |next: Vec<Scored>| -> Vec<String> {
            next.iter().map(|s| s.formula.to_string()).collect()
        };
        // One failure for a reserve of two: the holders take the spare slot.
        let few_failures = vec![
            scored("h1", 1.0, 0.4),
            scored("h2", 1.0, 0.3),
            scored("h3", 1.0, 0.2),
            scored("h4", 1.0, 0.1),
            scored("phi", 0.0, 1.0),
        ];
        assert_eq!(
            names(select(few_failures, 4, Status::GradedReserve(2))),
            ["h1", "h2", "h3", "phi"]
        );
        // One holder for a reserve of one in four: the failures take three.
        let few_holders = vec![
            scored("h1", 1.0, 0.4),
            scored("phi", 0.0, 1.0),
            scored("c", 0.5, 0.8),
            scored("d", 0.8, 0.3),
            scored("e", 0.2, 0.5),
        ];
        assert_eq!(
            names(select(few_holders, 4, Status::GradedReserve(4))),
            ["h1", "phi", "d", "c"]
        );
    }

    #[test]
    fn parents_are_drawn_uniformly_and_reproducibly() {
        let mut rng = Rng::new(5);
        let drawn = draw_parents(6, 60_000, &mut rng);
        let mut seen = [0usize; 6];
        for &i in &drawn {
            seen[i] += 1;
        }
        // 10,000 expected each; the standard deviation is about 91.
        assert!(
            seen.iter().all(|&n| (9_500..10_500).contains(&n)),
            "{seen:?}"
        );
        assert_eq!(drawn, draw_parents(6, 60_000, &mut Rng::new(5)));
        assert_eq!(draw_parents(1, 3, &mut rng), [0, 0, 0]);
    }

    #[test]
    fn breeding_spends_one_candidate_per_slot() {
        // A round of either search breeds one offspring per parent, so G
        // rounds of P and φ give 1 + G × P bred candidates.
        let atoms: Vec<Arc<str>> = ["a", "b"].into_iter().map(Arc::from).collect();
        let archive = vec![scored("G (a -> X b)", 0.0, 1.0), scored("F b", 1.0, 0.2)];
        let mut rng = Rng::new(3);
        for population in [1, 4, 9] {
            let drawn = draw_parents(archive.len(), population, &mut rng);
            let parents: Vec<&Scored> = drawn.iter().map(|&i| &archive[i]).collect();
            let offspring = breed(&parents, &archive, &atoms, &Params::new(1, 1), &mut rng);
            assert_eq!(offspring.len(), population);
            assert!(filter(offspring, &archive[0].formula).len() <= population);
        }
    }

    #[test]
    fn crossover_rate_zero_never_crosses() {
        // At rate 0 the crossover draw always fails and takes no partner
        // or graft draws, so breeding is the parent's mutation alone: the
        // same stream as mutating each parent in turn after one chance draw.
        let atoms: Vec<Arc<str>> = ["a", "b"].into_iter().map(Arc::from).collect();
        let archive = vec![scored("G (a -> X b)", 0.0, 1.0), scored("F b", 1.0, 0.2)];
        let parents: Vec<&Scored> = vec![&archive[0], &archive[1], &archive[0]];
        let params = Params {
            crossover_rate: 0.0,
            ..Params::new(1, 3)
        };
        for seed in 0..50 {
            let bred = breed(&parents, &archive, &atoms, &params, &mut Rng::new(seed));
            let mut rng = Rng::new(seed);
            let mutated: Vec<Formula> = parents
                .iter()
                .map(|p| {
                    assert!(!rng.chance(0.0));
                    assert!(rng.chance(MUTATION_RATE));
                    mutate(&p.formula, &atoms, params.p_monotone, &mut rng).simplify()
                })
                .collect();
            assert_eq!(bred, mutated);
        }
    }

    #[test]
    fn holds_reads_the_status_objective() {
        assert!(scored("a", 1.0, 0.0).holds());
        assert!(!scored("a", 0.0, 1.0).holds());
    }
}
