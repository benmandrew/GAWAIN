//! Model-counting similarity between a candidate φ' and the input φ, the
//! search's third objective, after PEREDUR's `semantic_similarity.cpp` and
//! `model_counter.cpp`.
//!
//! A formula's *count* is the number of lassos (u, v) over the valuations
//! of the atom universe, with |u| + |v| = [`BOUND`] and |v| ≥ 1, whose
//! infinite word u v^ω is in its language. A larger language accepts every
//! lasso a smaller one does, so the count is monotone under implication,
//! which PEREDUR's count of accepting paths in a transfer matrix is not,
//! and it sees liveness, which a count of finite prefixes does not. The
//! count runs a subset construction over u and the transition monoid over
//! v, on the `ltl2tgba -D` automaton. The universe is the atoms of φ and φ'
//! together, so the three counts below share it. The similarity is
//! PEREDUR's logarithmic metric over the counts of φ, φ' and `φ & φ'`: the
//! harmonic mean of `log shared / log count(φ)` and
//! `log shared / log count(φ')`, each clamped to [0, 1].
//! [`Counter::rejections`] gives a second, reporting score from the same
//! counts: the rejection strength `log count(¬φ') / log count(¬φ)`, which
//! separates near-⊤ candidates that the similarity scores alike.
//! [`Side::Rejected`] makes the similarity itself count from that side: the
//! same metric over ¬φ, ¬φ' and `¬φ & ¬φ'`. For a weakening of φ the
//! shared count is that of ¬φ', so the score is 2r/(1 + r) for the
//! rejection strength r, and ranks weakenings as r does. It still
//! penalises a candidate that drifts from φ, which r alone does not.
//!
//! Under LTLf ([`Semantics::Finite`]) the count is instead the number of
//! finite words over the universe, of each length from 1 to [`BOUND`], that
//! satisfy the formula. A larger LTLf language accepts every word a smaller
//! one does, so this count is monotone under LTLf implication, which the
//! lasso count is not. The automaton is the transition-based DFA of
//! `ltlf2dfa`, which accepts a word when its last transition is accepting,
//! so the words are counted by the state they reach, with no subset
//! construction. `ltlf2dfa` builds these automata hundreds of times faster
//! than `ltl2tgba -D` builds the lasso ones for the same formulas; see
//! "Model counts under LTLf" in `docs/dev/trace-sets.md`.
//!
//! Labels mentioning at most [`EXHAUSTIVE_ATOMS`] atoms are split into
//! letters by truth table in process, as in PEREDUR. A single label or
//! letter mentioning at most [`LABEL_TABLE_ATOMS`] atoms is counted by
//! truth table too, and a wider one by Ganak (submodule `ganak`). Counts are kept as base-2 logarithms, so no bound overflows.
//! All the translations of one [`Counter::similarities`] call go through
//! one exec. See "Model-counting similarity" in
//! `docs/dev/search.md`.

mod ganak;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{BuildHasherDefault, Hasher};
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ltl::{BinaryOp, Formula, UnaryOp};
use crate::oracle::hoa::{self, Guard, Marks, Raw};
use crate::oracle::process::{self, Limits};
use crate::oracle::Semantics;

/// The trace length, PEREDUR's `default_model_counting_bound`.
pub const BOUND: usize = 20;

/// The widest set of labels split into letters by truth table, in
/// mentioned atoms, PEREDUR's `k_exhaustive_count_max_atoms`.
pub const EXHAUSTIVE_ATOMS: usize = 16;

/// The widest single label or letter counted by truth table, in mentioned
/// atoms. Wider ones go to Ganak. A table over 20 atoms is 16,384 words,
/// which one label evaluates in about a millisecond, where one Ganak exec
/// costs tens; see "Slow implication batches" in `docs/dev/trace-sets.md`.
pub const LABEL_TABLE_ATOMS: usize = 20;

/// The most memory, in bytes, that one lasso count's transition monoid
/// may take, as estimated by its bookkeeping. A count that would pass it
/// fails, and its candidate scores 0, as on a translation timeout. The
/// monoid of an automaton can grow exponentially with its states: one
/// 85-state automaton of an MCC run reached 4.6 million relations at loop
/// length 7 and took the whole search past 16 GB. See "Bounded monoid" in
/// `docs/dev/search.md`.
pub const MONOID_BYTES: usize = 1 << 30;

/// The most work one lasso count may do, in steps: one per state of a
/// prefix subset stepped by a letter; per relation, 32 to intern it and
/// one per block id stepped by a letter; one per row of a block stepped;
/// one per word combined into a stepped row; and per relation's closure,
/// one per state, per word of its rows and per edge. A count that would
/// pass it fails, as one past [`MONOID_BYTES`] does. The count is of steps
/// and not of time, so a seeded search makes the same choices under any
/// load. A step took 4 to 19 ns on av2, 7 ns in the median, so the bound
/// is about 30 s, and 3.4 times the largest count of the replayed corpus;
/// see "Lasso count time" in `docs/dev/search.md`.
pub const COUNT_WORK: u64 = 1 << 32;

/// Which languages [`Counter::similarities`] compares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Side {
    /// φ, φ' and `φ & φ'`, PEREDUR's metric. When φ accepts nearly every
    /// lasso, the scores of its weakenings differ from ⊤'s by about 1e-11,
    /// near the noise of `f64`.
    #[default]
    Accepted,
    /// ¬φ, ¬φ' and `¬φ & ¬φ'`, which keeps that difference: ⊤ scores 0.
    Rejected,
}

/// What one lasso count may spend before it fails.
#[derive(Clone, Copy, Debug)]
struct Budget {
    /// The estimated bytes of the transition monoid.
    bytes: usize,
    /// Steps, as in [`COUNT_WORK`].
    work: u64,
}

impl Budget {
    const FULL: Budget = Budget {
        bytes: MONOID_BYTES,
        work: COUNT_WORK,
    };
}

/// What a [`Counter`] has spent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Counts {
    /// Candidates asked about, φ itself included.
    pub queries: u64,
    /// Candidates answered from the similarity cache.
    pub hits: u64,
    /// `ltl2tgba` or `ltlf2dfa` execs, batches and retries together.
    pub translations: u64,
    /// Formulas translated.
    pub formulas: u64,
    /// Ganak execs.
    pub ganak_calls: u64,
    /// Formulas with no count (a timeout or a tool error). Each candidate
    /// that needs one scores 0.
    pub failures: u64,
    /// Wall time inside [`Counter::similarities`].
    pub time: Duration,
    /// The part of `time` spent waiting on `ltl2tgba` and Ganak.
    pub tool_time: Duration,
    /// The part of `time` that the search waited for, after its oracle
    /// checks of the same candidates had finished.
    pub wait: Duration,
}

/// Scores candidates against one fixed φ, caching every count.
pub struct Counter {
    phi: Formula,
    semantics: Semantics,
    phi_atoms: BTreeSet<Arc<str>>,
    side: Side,
    limits: Limits,
    /// log2 counts by canonical formula and universe size; `None` when the
    /// formula could not be counted.
    logs: HashMap<(Formula, usize), Option<f64>>,
    /// Similarities by canonical candidate.
    similarities: HashMap<Formula, f64>,
    /// Ganak counts by label, over the label's own variables.
    ganak: HashMap<Guard, Option<f64>>,
    counts: Counts,
}

impl Counter {
    /// A counter for candidates against `phi`, counting lassos under
    /// [`Semantics::Infinite`] and finite words under
    /// [`Semantics::Finite`]. `limits` apply to each translation and Ganak
    /// exec.
    pub fn new(phi: &Formula, semantics: Semantics, limits: Limits) -> Self {
        Counter {
            phi: phi.canonical(),
            semantics,
            phi_atoms: phi.atoms(),
            side: Side::Accepted,
            limits,
            logs: HashMap::new(),
            similarities: HashMap::new(),
            ganak: HashMap::new(),
            counts: Counts::default(),
        }
    }

    /// This counter, comparing the languages of `side`.
    pub fn with_side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }

    /// What this counter has spent so far.
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// The log2 count of each formula over the atoms of φ and the formula,
    /// in one translation exec, or `None` where the count failed.
    pub fn log_counts(&mut self, formulas: &[Formula]) -> Vec<Option<f64>> {
        let entries: Vec<(Formula, usize)> = formulas
            .iter()
            .map(|f| (f.canonical(), self.universe(f)))
            .collect();
        let mut needed = Vec::new();
        for entry in &entries {
            if !self.logs.contains_key(entry) && !needed.contains(entry) {
                needed.push(entry.clone());
            }
        }
        self.count_all(needed);
        entries.iter().map(|entry| self.logs[entry]).collect()
    }

    /// Records that the search waited `wait` for this counter.
    pub(crate) fn record_wait(&mut self, wait: Duration) {
        self.counts.wait += wait;
    }

    /// The similarity of each candidate to φ, in [0, 1], 1 for φ itself. A
    /// candidate whose counts fail scores 0. The cache is keyed on
    /// [`Formula::canonical`] with no renaming, since the score is relative
    /// to the fixed φ.
    pub fn similarities(&mut self, candidates: &[Formula]) -> Vec<f64> {
        let start = Instant::now();
        let keys: Vec<Formula> = candidates.iter().map(Formula::canonical).collect();
        // Each formula to count once, with its universe.
        let mut needed: Vec<(Formula, usize)> = Vec::new();
        for key in &keys {
            self.counts.queries += 1;
            if *key == self.phi || self.similarities.contains_key(key) {
                self.counts.hits += 1;
                continue;
            }
            let universe = self.universe(key);
            for formula in self.terms(key) {
                let entry = (formula, universe);
                if !self.logs.contains_key(&entry) && !needed.contains(&entry) {
                    needed.push(entry);
                }
            }
        }
        self.count_all(needed);
        let scores = keys
            .iter()
            .map(|key| {
                if *key == self.phi {
                    return 1.0;
                }
                if let Some(&score) = self.similarities.get(key) {
                    return score;
                }
                let universe = self.universe(key);
                let [phi, candidate, shared] = self.terms(key).map(|f| self.logs[&(f, universe)]);
                let score = match (phi, candidate, shared) {
                    (Some(phi), Some(candidate), Some(shared)) => {
                        similarity_from_logs(phi, candidate, shared)
                    }
                    _ => 0.0,
                };
                self.similarities.insert(key.clone(), score);
                score
            })
            .collect();
        self.counts.time += start.elapsed();
        scores
    }

    /// The rejection strength of each candidate, a reporting score that
    /// the search does not use: [`rejection_from_logs`] over the counts of
    /// ¬φ and ¬φ', both over the universe of φ and the candidate. ⊤ scores
    /// 0 and φ 1. A candidate whose counts fail scores 0. The counts share
    /// the cache of [`Counter::similarities`].
    pub fn rejections(&mut self, candidates: &[Formula]) -> Vec<f64> {
        let start = Instant::now();
        let not = |f: &Formula| Formula::unary(UnaryOp::Not, f.clone()).canonical();
        let entries: Vec<[(Formula, usize); 2]> = candidates
            .iter()
            .map(|candidate| {
                let universe = self.universe(candidate);
                [(not(&self.phi), universe), (not(candidate), universe)]
            })
            .collect();
        let mut needed = Vec::new();
        for entry in entries.iter().flatten() {
            if !self.logs.contains_key(entry) && !needed.contains(entry) {
                needed.push(entry.clone());
            }
        }
        self.count_all(needed);
        let scores = entries
            .iter()
            .map(
                |[phi, candidate]| match (self.logs[phi], self.logs[candidate]) {
                    (Some(phi), Some(candidate)) => rejection_from_logs(phi, candidate),
                    _ => 0.0,
                },
            )
            .collect();
        self.counts.time += start.elapsed();
        scores
    }

    fn universe(&self, candidate: &Formula) -> usize {
        let mut atoms = candidate.atoms();
        atoms.extend(self.phi_atoms.iter().cloned());
        atoms.len()
    }

    /// The formulas whose counts give `candidate`'s similarity: φ, φ' and
    /// their conjunction, or under [`Side::Rejected`] their negations.
    fn terms(&self, candidate: &Formula) -> [Formula; 3] {
        let [phi, candidate] = match self.side {
            Side::Accepted => [self.phi.clone(), candidate.clone()],
            Side::Rejected => {
                [&self.phi, candidate].map(|f| Formula::unary(UnaryOp::Not, f.clone()).canonical())
            }
        };
        let shared = Formula::binary(BinaryOp::And, phi.clone(), candidate.clone()).canonical();
        [phi, candidate, shared]
    }

    /// Fills `logs` for every entry of `needed`: one translation exec for
    /// all, and if that fails, one exec per formula.
    fn count_all(&mut self, needed: Vec<(Formula, usize)>) {
        if needed.is_empty() {
            return;
        }
        let formulas: Vec<&Formula> = needed.iter().map(|(f, _)| f).collect();
        let automata = match self.translate(&formulas) {
            Ok(automata) => automata.into_iter().map(Ok).collect(),
            Err(_) => formulas
                .iter()
                .map(|&f| self.translate(&[f]).map(|mut a| a.remove(0)))
                .collect::<Vec<_>>(),
        };
        for ((formula, universe), automaton) in needed.into_iter().zip(automata) {
            let log = automaton.ok().and_then(|a| match self.semantics {
                Semantics::Infinite => self.log_count(&a, universe).ok(),
                Semantics::Finite => self.log_count_finite(&a, universe).ok(),
            });
            if log.is_none() {
                self.counts.failures += 1;
            }
            self.logs.insert((formula, universe), log);
        }
    }

    /// The automata of `formulas`, in order, from one exec: `ltl2tgba -D`
    /// for lassos, `ltlf2dfa` for finite words.
    fn translate(&mut self, formulas: &[&Formula]) -> Result<Vec<Raw>, String> {
        self.counts.translations += 1;
        self.counts.formulas += formulas.len() as u64;
        let mut input = String::new();
        for formula in formulas {
            let line = format!("{formula}\n");
            if line.matches('\n').count() != 1 {
                return Err(format!("cannot put {formula} on one line"));
            }
            input.push_str(&line);
        }
        let (tool, options) = match self.semantics {
            Semantics::Infinite => ("ltl2tgba", &["-D", "-H"][..]),
            Semantics::Finite => ("ltlf2dfa", &["-Ht"][..]),
        };
        let mut command = Command::new(tool);
        command.args(options).args(["-F", "-"]);
        let start = Instant::now();
        let output = process::run(&mut command, &input, self.limits);
        self.counts.tool_time += start.elapsed();
        let output = output.map_err(|e| format!("{tool}: {e}"))?;
        if !output.status.success() {
            return Err(format!("{tool}: {}", output.stderr.trim()));
        }
        let automata = output
            .stdout
            .split_inclusive("--END--")
            .filter(|doc| doc.contains("--BODY--"))
            .map(hoa::parse_raw)
            .collect::<Result<Vec<_>, _>>()?;
        if automata.len() != formulas.len() {
            return Err(format!(
                "{tool}: {} automata for {} formulas",
                automata.len(),
                formulas.len()
            ));
        }
        Ok(automata)
    }

    /// The log2 count of `automaton` over a universe of `universe` atoms:
    /// the pairs (u, v) with |u| + |v| = [`BOUND`] and |v| ≥ 1 such that
    /// the automaton accepts u v^ω. Pairs are counted, not words, so a
    /// word with several such pairs counts once per pair.
    ///
    /// The prefixes u go through a subset construction, one subset per u.
    /// The loops v go through the transition monoid: the relation of v
    /// links p to q when some run on v leads from p to q, carrying the
    /// acceptance sets seen on any such run. v^ω is accepted from p when p
    /// reaches, in the graph of that relation, a cycle that collects every
    /// set; an infinite run can take each edge of the cycle by a different
    /// run of v on each pass, so the union over runs is exact. Weights are
    /// rescaled to a maximum of 1 after each step, the scale kept as a
    /// logarithm, so no product overflows.
    fn log_count(&mut self, automaton: &Raw, universe: usize) -> Result<f64, String> {
        self.log_count_within(automaton, universe, Budget::FULL)
    }

    /// [`Counter::log_count`] within `budget`.
    fn log_count_within(
        &mut self,
        automaton: &Raw,
        universe: usize,
        budget: Budget,
    ) -> Result<f64, String> {
        let n = automaton.edges.len();
        let graph = edge_graph(automaton);
        let live = accepting_reach(&graph, automaton.all_marks);
        let start = automaton.start as usize;
        if !live[start] {
            return Ok(f64::NEG_INFINITY);
        }
        let letters = self.letters(automaton, &live, universe)?;
        // Ordered maps throughout, so the sums run in the same order on
        // every run.
        let mut prefixes: Vec<(BTreeMap<Vec<u32>, f64>, f64)> = Vec::with_capacity(BOUND);
        let mut weights = BTreeMap::from([(vec![start as u32], 1.0)]);
        let mut scale = 0.0;
        let mut work = 0;
        for _ in 0..BOUND {
            let mut next: BTreeMap<Vec<u32>, f64> = BTreeMap::new();
            for (subset, weight) in &weights {
                work += (subset.len() * letters.len()) as u64;
                if work > budget.work {
                    return Err(over_work(budget));
                }
                for letter in &letters {
                    let mut post: Vec<u32> = subset
                        .iter()
                        .flat_map(|&s| letter.step[s as usize].iter().map(|&(t, _)| t as u32))
                        .collect();
                    post.sort_unstable();
                    post.dedup();
                    if !post.is_empty() {
                        *next.entry(post).or_default() += weight * letter.models;
                    }
                }
            }
            let (next, top) = rescaled(next);
            prefixes.push((std::mem::replace(&mut weights, next), scale));
            scale += top;
        }
        let mut monoid = Monoid::new(n, automaton.all_marks, &letters, budget);
        monoid.work = work;
        // The loops of each length by relation id. The monoid is finite,
        // so the ids stop growing and each step is a sparse product over
        // a fixed table, in id order. A first pass builds the monoid alone,
        // so that a count past the budget fails before any closure is
        // computed. It visits the relations in the same order as the second
        // pass, so the same counts fail at the same point.
        let identity = monoid.identity();
        let empty_word = |identity: usize| {
            let mut loops = vec![0.0; identity + 1];
            loops[identity] = 1.0;
            loops
        };
        let mut loops = empty_word(identity);
        for length in 1..=BOUND {
            loops = monoid.step(&loops, &letters, length)?.0;
        }
        let mut loops = empty_word(identity);
        let mut scale = 0.0;
        let mut terms = Vec::with_capacity(BOUND);
        for length in 1..=BOUND {
            let top;
            (loops, top) = monoid.step(&loops, &letters, length)?;
            scale += top;
            // The loops by the states that accept their ω-power.
            let mut by_good = vec![0.0; monoid.goods.len()];
            for (relation, &weight) in loops.iter().enumerate() {
                if weight != 0.0 {
                    let good = monoid.good(relation);
                    if monoid.work > budget.work {
                        return Err(over_work(budget));
                    }
                    if good >= by_good.len() {
                        by_good.resize(good + 1, 0.0);
                    }
                    by_good[good] += weight;
                }
            }
            let (subsets, prefix_scale) = &prefixes[BOUND - length];
            let mut dot = 0.0;
            for (subset, prefix) in subsets {
                for (good, &weight) in by_good.iter().enumerate() {
                    let good = &monoid.goods[good];
                    if weight != 0.0 && subset.iter().any(|&s| has(good, s as usize)) {
                        dot += prefix * weight;
                    }
                }
            }
            if dot > 0.0 {
                terms.push(dot.log2() + prefix_scale + scale);
            }
        }
        let top = terms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if top == f64::NEG_INFINITY {
            return Ok(top);
        }
        Ok(top + terms.iter().map(|t| (t - top).exp2()).sum::<f64>().log2())
    }

    /// The log2 count of the finite words of length 1 to [`BOUND`] over a
    /// universe of `universe` atoms that the DFA `automaton` accepts, a
    /// word being accepted when its last transition is. The edges out of a
    /// state of a DFA hold on disjoint valuations, so the words reaching
    /// each state are counted by state, each edge weighted by its own
    /// models, with no subsets and no letter classes. Each label is
    /// counted over the atoms it mentions, so few go to Ganak.
    fn log_count_finite(&mut self, automaton: &Raw, universe: usize) -> Result<f64, String> {
        let mut edges = Vec::new();
        for (source, out) in automaton.edges.iter().enumerate() {
            for edge in out {
                let models = self.guard_models(&edge.guard, universe)?;
                let marked = edge.marks | automaton.state_marks[source] != 0;
                edges.push((source as u32, edge.target, models, marked));
            }
        }
        let mut weights = BTreeMap::from([(automaton.start, 1.0)]);
        let mut scale = 0.0;
        let mut terms = Vec::with_capacity(BOUND);
        for _ in 0..BOUND {
            let mut next: BTreeMap<u32, f64> = BTreeMap::new();
            let mut accepted = 0.0;
            for &(source, target, models, marked) in &edges {
                if let Some(weight) = weights.get(&source) {
                    *next.entry(target).or_default() += weight * models;
                    if marked {
                        accepted += weight * models;
                    }
                }
            }
            if accepted > 0.0 {
                terms.push(f64::log2(accepted) + scale);
            }
            let (next, top) = rescaled(next);
            scale += top;
            weights = next;
        }
        let top = terms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if top == f64::NEG_INFINITY {
            return Ok(top);
        }
        Ok(top + terms.iter().map(|t| (t - top).exp2()).sum::<f64>().log2())
    }

    /// The letters of `automaton` over the universe: the classes of
    /// valuations that enable the same edges between live states, each
    /// with its edges and its number of valuations. A class that enables
    /// no edge is left out, since no run reads it.
    fn letters(
        &mut self,
        automaton: &Raw,
        live: &[bool],
        universe: usize,
    ) -> Result<Vec<Letter>, String> {
        let mut labels: Vec<Guard> = Vec::new();
        let mut index: HashMap<&Guard, usize> = HashMap::new();
        let mut edges = Vec::new();
        for (source, out) in automaton.edges.iter().enumerate() {
            if !live[source] {
                continue;
            }
            for edge in out.iter().filter(|e| live[e.target as usize]) {
                let label = *index.entry(&edge.guard).or_insert_with(|| {
                    labels.push(edge.guard.clone());
                    labels.len() - 1
                });
                let marks = edge.marks | automaton.state_marks[source];
                edges.push((label, source, edge.target as usize, marks));
            }
        }
        let classes = self.classes(&labels, universe)?;
        Ok(classes
            .into_iter()
            .map(|(holds, models)| {
                let mut step = vec![Vec::new(); automaton.edges.len()];
                for &(label, source, target, marks) in &edges {
                    if holds.binary_search(&label).is_ok() {
                        step[source].push((target, marks));
                    }
                }
                Letter { models, step }
            })
            .collect())
    }

    /// The valuations of the universe split into classes by `labels`: each
    /// class is the indices of the labels that hold on it, with its number
    /// of valuations. Only non-empty classes on which some label holds are
    /// kept.
    fn classes(
        &mut self,
        labels: &[Guard],
        universe: usize,
    ) -> Result<Vec<(Vec<usize>, f64)>, String> {
        let (local, mentioned) = renumber_all(labels);
        if mentioned > universe {
            return Err(format!(
                "labels mention {mentioned} atoms, the universe has {universe}"
            ));
        }
        if mentioned <= EXHAUSTIVE_ATOMS {
            // One truth table per label over the atoms they mention, and
            // each class a set of rows.
            let free = 2f64.powi((universe - mentioned) as i32);
            let words = table_words(mentioned);
            let mut all = vec![!0; words];
            all[0] = table_mask(mentioned);
            let mut classes: Vec<(Vec<usize>, Vec<u64>)> = vec![(Vec::new(), all)];
            for (index, label) in local.iter().enumerate() {
                let table = evaluate(label, words);
                let mut refined = Vec::with_capacity(classes.len() * 2);
                for (targets, rows) in classes {
                    let with: Vec<u64> = rows.iter().zip(&table).map(|(r, t)| r & t).collect();
                    let without: Vec<u64> = rows.iter().zip(&table).map(|(r, t)| r & !t).collect();
                    if with.iter().any(|&w| w != 0) {
                        let mut more = targets.clone();
                        more.push(index);
                        refined.push((more, with));
                    }
                    if without.iter().any(|&w| w != 0) {
                        refined.push((targets, without));
                    }
                }
                classes = refined;
            }
            return Ok(classes
                .into_iter()
                .filter(|(targets, _)| !targets.is_empty())
                .map(|(targets, rows)| {
                    let ones: u64 = rows.iter().map(|w| w.count_ones() as u64).sum();
                    (targets, ones as f64 * free)
                })
                .collect());
        }
        // Too wide for one table: each class is a conjunction, counted by
        // `guard_models`, which sends what is still too wide to Ganak.
        let mut classes = vec![(Vec::new(), Guard::True, 2f64.powi(universe as i32))];
        for (index, label) in labels.iter().enumerate() {
            let mut refined = Vec::with_capacity(classes.len() * 2);
            for (targets, class, models) in classes {
                let with = Guard::And(Box::new(class.clone()), Box::new(label.clone()));
                let with_models = self.guard_models(&with, universe)?;
                if with_models > 0.0 {
                    let mut more = targets.clone();
                    more.push(index);
                    refined.push((more, with, with_models));
                }
                if with_models < models {
                    let not = Guard::Not(Box::new(label.clone()));
                    let without = Guard::And(Box::new(class), Box::new(not));
                    refined.push((targets, without, models - with_models));
                }
            }
            classes = refined;
        }
        Ok(classes
            .into_iter()
            .filter(|(targets, _, _)| !targets.is_empty())
            .map(|(targets, _, models)| (targets, models))
            .collect())
    }

    /// The valuations of the universe that satisfy `guard`: its models over
    /// the atoms it mentions, times 2 for each atom it does not.
    fn guard_models(&mut self, guard: &Guard, universe: usize) -> Result<f64, String> {
        let (local, mentioned) = renumber(guard);
        if mentioned > universe {
            return Err(format!(
                "a label mentions {mentioned} atoms, the universe has {universe}"
            ));
        }
        let free = 2f64.powi((universe - mentioned) as i32);
        if mentioned <= LABEL_TABLE_ATOMS {
            return Ok(truth_table_models(&local, mentioned) as f64 * free);
        }
        let models = match self.ganak.get(&local) {
            Some(&models) => models,
            None => {
                self.counts.ganak_calls += 1;
                let start = Instant::now();
                let models = ganak::count(&local, mentioned, self.limits).ok();
                self.counts.tool_time += start.elapsed();
                self.ganak.insert(local, models);
                models
            }
        };
        models
            .map(|m| m * free)
            .ok_or_else(|| "ganak gave no count".into())
    }
}

/// PEREDUR's logarithmic metric from the log2 counts of φ, φ' and
/// `φ & φ'` (`-inf` for a count of 0). Two empty languages score 1, and
/// one empty against one non-empty 0. A count of at most 1 has no growth
/// rate, so its term falls back to membership.
pub fn similarity_from_logs(phi: f64, candidate: f64, shared: f64) -> f64 {
    let empty = |log: f64| log == f64::NEG_INFINITY;
    if empty(phi) && empty(candidate) {
        return 1.0;
    }
    if empty(phi) || empty(candidate) {
        return 0.0;
    }
    let (first, second) = (containment(shared, phi), containment(shared, candidate));
    if first == 0.0 && second == 0.0 {
        return 0.0;
    }
    2.0 * first * second / (first + second)
}

/// The rejection strength from the log2 counts of ¬φ and ¬φ' (`-inf` for a
/// count of 0): `log count(¬φ') / log count(¬φ)`, the log share of φ's
/// violating lassos that φ' still rejects, clamped to [0, 1]. φ ⇒ φ' puts
/// L(¬φ') inside L(¬φ), so the share needs no intersection. The boundaries
/// are those of [`similarity_from_logs`]: two empty negations (φ and φ'
/// both ⊤) score 1, one empty against one non-empty 0, and a count of ¬φ
/// of at most 1 falls back to membership.
pub fn rejection_from_logs(phi: f64, candidate: f64) -> f64 {
    let empty = |log: f64| log == f64::NEG_INFINITY;
    if empty(phi) && empty(candidate) {
        return 1.0;
    }
    if empty(phi) || empty(candidate) {
        return 0.0;
    }
    containment(candidate, phi)
}

/// One term of PEREDUR's logarithmic metric, `log shared / log whole`
/// clamped to [0, 1], from non-empty log2 counts. A whole of at most one
/// model has no growth rate, so the term is membership.
fn containment(shared: f64, whole: f64) -> f64 {
    if whole <= 0.0 {
        return if shared >= whole { 1.0 } else { 0.0 };
    }
    if shared <= 0.0 {
        return 0.0;
    }
    (shared / whole).clamp(0.0, 1.0)
}

/// One letter of an automaton: a class of valuations and, per state, the
/// targets it reaches on them with the acceptance sets of each edge.
struct Letter {
    models: f64,
    step: Vec<Vec<(usize, Marks)>>,
}

/// A cheap multiplicative hash, 8 bytes at a time, for interning
/// relations; the default SipHash dominated the loop count.
#[derive(Default)]
struct WordHasher(u64);

impl Hasher for WordHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for &word in words {
            self.write_u64(u64::from_le_bytes(word));
        }
        for &byte in rest {
            self.write_u64(byte as u64);
        }
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

/// Whether bit `i` of the bitset `set` is on.
fn has(set: &[u64], i: usize) -> bool {
    set[i / 64] >> (i % 64) & 1 == 1
}

/// The relations of the loops v met so far, interned. A relation has one
/// row per state, and a row is a stack of `planes` bitsets of `words`
/// 64-bit words: plane 0 holds q in row p when some run on v leads from p
/// to q, and plane 1 + i when some such run sees acceptance set i. A
/// letter is a relation of length 1.
///
/// Row p of v·a depends only on row p of v and on a, so rows are
/// interned on their own and the step of a row by a letter is computed
/// once and kept in a table. The same holds for a block of [`BLOCK`]
/// consecutive rows, so blocks of row ids are interned too, with a table
/// of their steps, and a relation is its list of block ids. A step of a
/// relation is then one table lookup per block. Each relation's set of
/// accepting states, a bitset, is interned too.
///
/// `bytes` estimates the memory the monoid held when a relation was its
/// list of row ids, so that a count can stop before a monoid that grows
/// without bound exhausts the process. Blocks take less, but the estimate
/// is kept so that the same counts stop; see "Lasso count time" in
/// `docs/dev/search.md`.
struct Monoid {
    n: usize,
    words: usize,
    planes: usize,
    letters: Vec<Vec<u64>>,
    row_ids: HashMap<Rc<[u64]>, u32, BuildHasherDefault<WordHasher>>,
    rows: Vec<Rc<[u64]>>,
    /// The id of row r stepped by letter a, at `r * letters + a`, or
    /// `u32::MAX` before it is computed.
    row_steps: Vec<u32>,
    /// Blocks of row ids, the last block of a relation padded with
    /// [`PAD`].
    blocks: Lists,
    /// The id of block b stepped by letter a, at `b * letters + a`, or
    /// `u32::MAX` before it is computed.
    block_steps: Vec<u32>,
    /// Relations as lists of block ids.
    relations: Lists,
    /// The id of relation r followed by letter a, at `r * letters + a`, or
    /// `u32::MAX` before it is computed.
    steps: Vec<u32>,
    good_of: Vec<Option<usize>>,
    good_ids: HashMap<Vec<u64>, usize>,
    goods: Vec<Vec<u64>>,
    components: Components,
    bytes: usize,
    /// Steps so far, as in [`COUNT_WORK`].
    work: u64,
    budget: Budget,
}

/// The states in one block of a relation.
const BLOCK: usize = 8;

/// The row id that pads the last block of a relation past the last state.
const PAD: u32 = u32::MAX;

/// Lists of `width` ids, interned and stored end to end, so that a list
/// costs no allocation of its own. The table is open addressing with
/// linear probing, and each slot keeps the high half of its list's hash,
/// which settles most probes without reading the list.
struct Lists {
    width: usize,
    data: Vec<u32>,
    /// Per slot, 0 when empty, else the high 32 bits of the hash above the
    /// id plus one.
    slots: Vec<u64>,
}

impl Lists {
    fn new(width: usize) -> Self {
        Lists {
            width,
            data: Vec::new(),
            slots: vec![0; 16],
        }
    }

    fn len(&self) -> usize {
        self.data.len() / self.width
    }

    fn get(&self, id: u32) -> &[u32] {
        let at = id as usize * self.width;
        &self.data[at..at + self.width]
    }

    fn hash(list: &[u32]) -> u64 {
        let mut hasher = WordHasher::default();
        for pair in list.chunks(2) {
            hasher.write_u64(pair[0] as u64 | (*pair.get(1).unwrap_or(&0) as u64) << 32);
        }
        hasher.finish()
    }

    /// The slot of the hash `hash` for `list`: the one holding it, or the
    /// empty one where it belongs.
    fn find(&self, list: &[u32], hash: u64) -> usize {
        let mask = self.slots.len() - 1;
        let tag = hash >> 32;
        let mut i = (hash >> 32) as usize & mask;
        loop {
            let slot = self.slots[i];
            if slot == 0 || (slot >> 32 == tag && self.get((slot as u32) - 1) == list) {
                return i;
            }
            i = (i + 1) & mask;
        }
    }

    /// The id of `list`, and whether it is new.
    fn intern(&mut self, list: &[u32]) -> (u32, bool) {
        let hash = Self::hash(list);
        let i = self.find(list, hash);
        if self.slots[i] != 0 {
            return ((self.slots[i] as u32) - 1, false);
        }
        let id = self.len() as u32;
        self.data.extend_from_slice(list);
        self.slots[i] = (hash >> 32) << 32 | (id as u64 + 1);
        if 2 * self.len() > self.slots.len() {
            let grown = vec![0; 2 * self.slots.len()];
            let old = std::mem::replace(&mut self.slots, grown);
            for slot in old.into_iter().filter(|&slot| slot != 0) {
                let list = self.get((slot as u32) - 1);
                let i = self.find(list, Self::hash(list));
                self.slots[i] = slot;
            }
        }
        (id, true)
    }
}

/// The estimated bytes per interned row or relation besides its payload:
/// the allocation header, the pointers in the list and in the hash map,
/// and the per-relation slots of `steps`, `good_of` and the loop weights.
const ENTRY_BYTES: usize = 160;

impl Monoid {
    fn new(n: usize, all_marks: Marks, letters: &[Letter], budget: Budget) -> Self {
        let words = n.div_ceil(64).max(1);
        let planes = 1 + all_marks.count_ones() as usize;
        let letters = letters
            .iter()
            .map(|letter| {
                let mut table = vec![0; n * planes * words];
                for (p, out) in letter.step.iter().enumerate() {
                    for &(q, marks) in out {
                        let seen = (0..planes - 1).filter(|i| marks >> i & 1 == 1);
                        for plane in std::iter::once(0).chain(seen.map(|i| 1 + i)) {
                            table[(p * planes + plane) * words + q / 64] |= 1 << (q % 64);
                        }
                    }
                }
                table
            })
            .collect();
        Monoid {
            n,
            words,
            planes,
            letters,
            row_ids: HashMap::default(),
            rows: Vec::new(),
            row_steps: Vec::new(),
            blocks: Lists::new(BLOCK),
            block_steps: Vec::new(),
            relations: Lists::new(n.div_ceil(BLOCK)),
            steps: Vec::new(),
            good_of: Vec::new(),
            good_ids: HashMap::new(),
            goods: Vec::new(),
            components: Components::default(),
            bytes: 0,
            work: 0,
            budget,
        }
    }

    fn intern_row(&mut self, row: &[u64]) -> u32 {
        if let Some(&id) = self.row_ids.get(row) {
            return id;
        }
        self.bytes += 8 * row.len() + 4 * self.letters.len() + ENTRY_BYTES;
        let row: Rc<[u64]> = row.into();
        let id = self.rows.len() as u32;
        self.rows.push(row.clone());
        self.row_ids.insert(row, id);
        self.row_steps
            .resize(self.rows.len() * self.letters.len(), u32::MAX);
        id
    }

    fn intern_block(&mut self, block: &[u32]) -> u32 {
        let (id, new) = self.blocks.intern(block);
        if new {
            self.block_steps
                .resize(self.blocks.len() * self.letters.len(), u32::MAX);
        }
        id
    }

    fn intern(&mut self, relation: &[u32]) -> usize {
        let (id, new) = self.relations.intern(relation);
        if new {
            self.bytes += 4 * self.n + 8 * self.letters.len() + ENTRY_BYTES;
            self.work += 32;
            self.steps
                .resize(self.relations.len() * self.letters.len(), u32::MAX);
        }
        id as usize
    }

    /// The relation of the empty word.
    fn identity(&mut self) -> usize {
        let mut rows: Vec<u32> = (0..self.n)
            .map(|p| {
                let mut row = vec![0; self.planes * self.words];
                row[p / 64] |= 1 << (p % 64);
                self.intern_row(&row)
            })
            .collect();
        rows.resize(self.n.next_multiple_of(BLOCK), PAD);
        let relation: Vec<u32> = rows
            .chunks(BLOCK)
            .map(|block| self.intern_block(block))
            .collect();
        self.intern(&relation)
    }

    /// The loops of length `length` by relation id, from those of length
    /// `length - 1`, rescaled to a maximum of 1, and the log2 of the scale.
    /// The relations are stepped in id order, and the step fails once the
    /// monoid passes its budget.
    fn step(
        &mut self,
        loops: &[f64],
        letters: &[Letter],
        length: usize,
    ) -> Result<(Vec<f64>, f64), String> {
        let mut next = vec![0.0; self.relations.len()];
        for (relation, &weight) in loops.iter().enumerate() {
            if weight == 0.0 {
                continue;
            }
            for (letter, &to) in letters.iter().zip(self.successors(relation)) {
                let to = to as usize;
                if to >= next.len() {
                    next.resize(to + 1, 0.0);
                }
                next[to] += weight * letter.models;
            }
            if self.bytes > self.budget.bytes {
                return Err(format!(
                    "the transition monoid passed {} MiB at {} relations, loop length {length}",
                    self.budget.bytes >> 20,
                    self.relations.len()
                ));
            }
            if self.work > self.budget.work {
                return Err(over_work(self.budget));
            }
        }
        let top = next.iter().copied().fold(0.0, f64::max);
        if top == 0.0 {
            return Ok((next, 0.0));
        }
        next.iter_mut().for_each(|w| *w /= top);
        Ok((next, top.log2()))
    }

    /// The relation of v followed by each letter, by letter index.
    fn successors(&mut self, relation: usize) -> &[u32] {
        let letters = self.letters.len();
        let at = relation * letters;
        if letters > 0 && self.steps[at] == u32::MAX {
            let from = self.relations.get(relation as u32).to_vec();
            self.work += (letters * from.len()) as u64;
            let mut next = vec![0; from.len()];
            for letter in 0..letters {
                for (slot, &block) in next.iter_mut().zip(&from) {
                    *slot = self.block_step(block, letter);
                }
                let to = self.intern(&next);
                self.steps[at + letter] = to as u32;
            }
        }
        &self.steps[at..at + letters]
    }

    /// The id of block `block` of some v, stepped by the letter `index`.
    fn block_step(&mut self, block: u32, index: usize) -> u32 {
        let slot = block as usize * self.letters.len() + index;
        if self.block_steps[slot] != u32::MAX {
            return self.block_steps[slot];
        }
        let rows = self.blocks.get(block).to_vec();
        self.work += BLOCK as u64;
        let next: Vec<u32> = rows
            .iter()
            .map(|&row| {
                if row == PAD {
                    PAD
                } else {
                    self.row_step(row, index)
                }
            })
            .collect();
        let id = self.intern_block(&next);
        self.block_steps[slot] = id;
        id
    }

    /// The id of row `row` of some v, stepped by the letter `index`: a
    /// run sees set i when its part on v does or its last edge does.
    fn row_step(&mut self, row: u32, index: usize) -> u32 {
        let slot = row as usize * self.letters.len() + index;
        if self.row_steps[slot] != u32::MAX {
            return self.row_steps[slot];
        }
        let (words, stride) = (self.words, self.planes * self.words);
        let (from, letter) = (&self.rows[row as usize], &self.letters[index]);
        let mut out = vec![0; stride];
        for (w, &word) in from[..words].iter().enumerate() {
            let mut rest = word;
            while rest != 0 {
                // Every plane of the letter's row q, for each q reached on v.
                let q = w * 64 + rest.trailing_zeros() as usize;
                rest &= rest - 1;
                let by = &letter[q * stride..(q + 1) * stride];
                out.iter_mut().zip(by).for_each(|(a, b)| *a |= b);
                self.work += stride as u64;
            }
        }
        for plane in 1..self.planes {
            for w in 0..words {
                let mut rest = from[plane * words + w];
                while rest != 0 {
                    // Plane 0 of the letter's row q, for each q reached
                    // seeing the set.
                    let q = w * 64 + rest.trailing_zeros() as usize;
                    rest &= rest - 1;
                    let by = &letter[q * stride..q * stride + words];
                    self.work += words as u64;
                    out[plane * words..(plane + 1) * words]
                        .iter_mut()
                        .zip(by)
                        .for_each(|(a, b)| *a |= b);
                }
            }
        }
        let id = self.intern_row(&out);
        self.row_steps[slot] = id;
        id
    }

    /// The id, in `goods`, of the states from which v^ω is accepted: those
    /// that reach, in the graph of plane 0, a strongly connected component
    /// with an edge inside it on every other plane. An infinite run can
    /// take each edge of the component by a different run of v on each
    /// pass, so the union over runs in each plane is exact.
    fn good(&mut self, relation: usize) -> usize {
        if self.good_of.len() <= relation {
            self.good_of.resize(relation + 1, None);
        }
        if let Some(good) = self.good_of[relation] {
            return good;
        }
        let (ids, buffer) = (&mut self.components.ids, &mut self.components.rows);
        ids.clear();
        buffer.clear();
        for &block in self.relations.get(relation as u32) {
            for &row in self.blocks.get(block).iter().take_while(|&&row| row != PAD) {
                ids.push(row);
                buffer.extend_from_slice(&self.rows[row as usize][..self.words]);
            }
        }
        let states = self
            .components
            .accepting(self.n, self.words, self.planes, &self.rows);
        let good = match self.good_ids.get(states) {
            Some(&good) => good,
            None => {
                self.goods.push(states.to_vec());
                self.good_ids.insert(states.to_vec(), self.goods.len() - 1);
                self.goods.len() - 1
            }
        };
        self.work += self.components.work;
        self.good_of[relation] = Some(good);
        good
    }
}

/// The buffers of [`Components::accepting`], kept between calls, since the
/// monoid asks once per relation.
#[derive(Default)]
struct Components {
    /// The row id of each state, in the table passed to `accepting`.
    ids: Vec<u32>,
    /// Plane 0 of the rows of the graph, one after another.
    rows: Vec<u64>,
    index: Vec<u32>,
    low: Vec<u32>,
    on_stack: Vec<u64>,
    stack: Vec<u32>,
    /// The depth-first path: a state, the word of its row being scanned
    /// and the bits of that word not yet followed.
    path: Vec<(u32, u32, u64)>,
    component: Vec<u64>,
    members: Vec<u32>,
    states: Vec<u64>,
    /// The steps of the last call: a state visited, a word scanned or an
    /// edge followed.
    work: u64,
}

impl Components {
    /// The states from which a run can visit, infinitely often, every
    /// acceptance set, in the graph of `n` states whose row p is
    /// `table[ids[p]]`, with its plane 0 copied into `rows`: `planes`
    /// bitsets of `words` words, plane 0 the successors of p, and plane
    /// 1 + i those reached by an edge in set i, a subset of plane 0.
    /// Those are the states that reach, in plane 0, a strongly connected
    /// component with a cycle and an edge inside it on every other plane.
    ///
    /// Tarjan's algorithm closes each component after every component it
    /// reaches, so whether a component reaches an accepting one is known
    /// from the states already marked. The cost is linear in the states
    /// and the edges, where a transitive closure costs n³/64 word
    /// operations; see "Lasso count time" in `docs/dev/search.md`.
    fn accepting<R: AsRef<[u64]>>(
        &mut self,
        n: usize,
        words: usize,
        planes: usize,
        table: &[R],
    ) -> &[u64] {
        const NONE: u32 = u32::MAX;
        let Components {
            ids,
            rows,
            index,
            low,
            on_stack,
            stack,
            path,
            component,
            members,
            states,
            work,
        } = self;
        *work = (n * (words + 1)) as u64;
        let rows = &rows[..n * words];
        let succ = |p: usize| &rows[p * words..(p + 1) * words];
        let meets = |set: &[u64], other: &[u64]| set.iter().zip(other).any(|(a, b)| a & b != 0);
        index.clear();
        index.resize(n, NONE);
        low.clear();
        low.resize(n, 0);
        for set in [&mut *on_stack, &mut *component, &mut *states] {
            set.clear();
            set.resize(words, 0);
        }
        stack.clear();
        path.clear();
        let mut next_index = 0u32;
        for root in 0..n {
            if index[root] != NONE {
                continue;
            }
            macro_rules! visit {
                ($u:expr) => {{
                    let u: usize = $u;
                    index[u] = next_index;
                    low[u] = next_index;
                    next_index += 1;
                    stack.push(u as u32);
                    on_stack[u / 64] |= 1 << (u % 64);
                    path.push((u as u32, 0, succ(u)[0]));
                }};
            }
            visit!(root);
            while let Some((v, w, rest)) = path.last_mut() {
                let v = *v as usize;
                if *rest != 0 {
                    let u = *w as usize * 64 + rest.trailing_zeros() as usize;
                    *rest &= *rest - 1;
                    *work += 1;
                    if index[u] == NONE {
                        visit!(u);
                    } else if has(on_stack, u) {
                        low[v] = low[v].min(index[u]);
                    }
                    continue;
                }
                *w += 1;
                if (*w as usize) < words {
                    *rest = succ(v)[*w as usize];
                    continue;
                }
                // Every successor of v is done: v closes a component when
                // it is the first of it visited.
                path.pop();
                if let Some(&(parent, _, _)) = path.last() {
                    let parent = parent as usize;
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] != index[v] {
                    continue;
                }
                members.clear();
                loop {
                    let t = stack.pop().expect("v is on the stack") as usize;
                    on_stack[t / 64] &= !(1 << (t % 64));
                    component[t / 64] |= 1 << (t % 64);
                    members.push(t as u32);
                    if t == v {
                        break;
                    }
                }
                let cyclic = members.len() > 1 || has(succ(v), v);
                let accepting = cyclic
                    && (1..planes).all(|plane| {
                        members.iter().any(|&t| {
                            let row = table[ids[t as usize] as usize].as_ref();
                            meets(&row[plane * words..(plane + 1) * words], component)
                        })
                    });
                if accepting || members.iter().any(|&t| meets(succ(t as usize), states)) {
                    states
                        .iter_mut()
                        .zip(&*component)
                        .for_each(|(a, b)| *a |= b);
                }
                for &t in &*members {
                    component[t as usize / 64] = 0;
                }
            }
        }
        states
    }
}

/// The error of a count past its work budget.
fn over_work(budget: Budget) -> String {
    format!("the lasso count passed {} steps", budget.work)
}

/// `weights` divided by their maximum, and the log2 of that maximum (0
/// when every weight is 0).
fn rescaled<K: Ord>(mut weights: BTreeMap<K, f64>) -> (BTreeMap<K, f64>, f64) {
    let top = weights.values().copied().fold(0.0, f64::max);
    if top == 0.0 {
        return (weights, 0.0);
    }
    weights.values_mut().for_each(|w| *w /= top);
    (weights, top.log2())
}

/// The edges of `automaton` as a graph, each with its own acceptance sets
/// and those of its source state. Spot prints no edge whose label is
/// false.
fn edge_graph(automaton: &Raw) -> Vec<Vec<(usize, Marks)>> {
    automaton
        .edges
        .iter()
        .zip(&automaton.state_marks)
        .map(|(out, &marks)| {
            out.iter()
                .map(|e| (e.target as usize, e.marks | marks))
                .collect()
        })
        .collect()
}

/// The nodes of `graph` that reach a cycle collecting every set of
/// `all_marks`. A node is on such a cycle when it reaches itself and its
/// strongly connected component, the nodes it reaches that reach it back,
/// carries every set on an edge inside.
fn accepting_reach(graph: &[Vec<(usize, Marks)>], all_marks: Marks) -> Vec<bool> {
    let n = graph.len();
    // reach[s][t]: t is reachable from s in one step or more.
    let mut reach = vec![vec![false; n]; n];
    for (s, row) in reach.iter_mut().enumerate() {
        let mut stack: Vec<usize> = graph[s].iter().map(|&(t, _)| t).collect();
        while let Some(t) = stack.pop() {
            if !row[t] {
                row[t] = true;
                stack.extend(graph[t].iter().map(|&(u, _)| u));
            }
        }
    }
    let on_cycle: Vec<bool> = (0..n)
        .map(|s| {
            if !reach[s][s] {
                return false;
            }
            let member = |t: usize| reach[s][t] && reach[t][s];
            let mut marks = 0;
            for t in (0..n).filter(|&t| member(t)) {
                for &(u, m) in &graph[t] {
                    if member(u) {
                        marks |= m;
                    }
                }
            }
            marks & all_marks == all_marks
        })
        .collect();
    (0..n)
        .map(|s| on_cycle[s] || (0..n).any(|t| reach[s][t] && on_cycle[t]))
        .collect()
}

/// `guard` with the APs it mentions renumbered `0..m` in index order, and
/// m. A count over the mentioned atoms does not depend on which they are.
fn renumber(guard: &Guard) -> (Guard, usize) {
    let (mut local, mentioned) = renumber_all(std::slice::from_ref(guard));
    (local.remove(0), mentioned)
}

/// [`renumber`] for several guards at once, with one numbering for all.
fn renumber_all(guards: &[Guard]) -> (Vec<Guard>, usize) {
    fn collect(guard: &Guard, out: &mut BTreeSet<usize>) {
        match guard {
            Guard::True | Guard::False => {}
            Guard::Ap(i) => {
                out.insert(*i);
            }
            Guard::Not(g) => collect(g, out),
            Guard::And(g, h) | Guard::Or(g, h) => {
                collect(g, out);
                collect(h, out);
            }
        }
    }
    fn map(guard: &Guard, index: &HashMap<usize, usize>) -> Guard {
        match guard {
            Guard::True => Guard::True,
            Guard::False => Guard::False,
            Guard::Ap(i) => Guard::Ap(index[i]),
            Guard::Not(g) => Guard::Not(Box::new(map(g, index))),
            Guard::And(g, h) => Guard::And(Box::new(map(g, index)), Box::new(map(h, index))),
            Guard::Or(g, h) => Guard::Or(Box::new(map(g, index)), Box::new(map(h, index))),
        }
    }
    let mut mentioned = BTreeSet::new();
    guards.iter().for_each(|g| collect(g, &mut mentioned));
    let index: HashMap<usize, usize> = mentioned.iter().enumerate().map(|(n, &i)| (i, n)).collect();
    let local = guards.iter().map(|g| map(g, &index)).collect();
    (local, mentioned.len())
}

/// The models of `guard` over APs `0..n`, `n` at most
/// [`LABEL_TABLE_ATOMS`], by a truth table 64 assignments to a word: bit `j`
/// of the table is the value under assignment `j`, whose bit `i` is AP `i`.
fn truth_table_models(guard: &Guard, n: usize) -> u64 {
    let table = evaluate(guard, table_words(n));
    let live = table_mask(n);
    table.iter().map(|w| (w & live).count_ones() as u64).sum()
}

/// The 64-bit words in a truth table over `n` APs.
fn table_words(n: usize) -> usize {
    if n > 6 {
        1 << (n - 6)
    } else {
        1
    }
}

/// The rows of each word that are assignments over `n` APs: all 64, or the
/// low 2^n of one word when n < 6.
fn table_mask(n: usize) -> u64 {
    if n < 6 {
        (1u64 << (1 << n)) - 1
    } else {
        !0
    }
}

fn evaluate(guard: &Guard, words: usize) -> Vec<u64> {
    const PATTERNS: [u64; 6] = [
        0xAAAA_AAAA_AAAA_AAAA,
        0xCCCC_CCCC_CCCC_CCCC,
        0xF0F0_F0F0_F0F0_F0F0,
        0xFF00_FF00_FF00_FF00,
        0xFFFF_0000_FFFF_0000,
        0xFFFF_FFFF_0000_0000,
    ];
    match guard {
        Guard::True => vec![!0; words],
        Guard::False => vec![0; words],
        Guard::Ap(i) if *i < 6 => vec![PATTERNS[*i]; words],
        Guard::Ap(i) => (0..words)
            .map(|w| if (w >> (i - 6)) & 1 == 1 { !0 } else { 0 })
            .collect(),
        Guard::Not(g) => evaluate(g, words).into_iter().map(|w| !w).collect(),
        Guard::And(g, h) => evaluate(g, words)
            .into_iter()
            .zip(evaluate(h, words))
            .map(|(a, b)| a & b)
            .collect(),
        Guard::Or(g, h) => evaluate(g, words)
            .into_iter()
            .zip(evaluate(h, words))
            .map(|(a, b)| a | b)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::hoa::RawEdge;

    fn ap(i: usize) -> Box<Guard> {
        Box::new(Guard::Ap(i))
    }

    #[test]
    fn truth_tables_count_models() {
        assert_eq!(truth_table_models(&Guard::True, 0), 1);
        assert_eq!(truth_table_models(&Guard::False, 3), 0);
        assert_eq!(truth_table_models(&Guard::Ap(0), 1), 1);
        let a_or_not_b = Guard::Or(ap(0), Box::new(Guard::Not(ap(1))));
        assert_eq!(truth_table_models(&a_or_not_b, 2), 3);
        // a7 & !a10 over 12 atoms: a quarter of 4096.
        let wide = Guard::And(ap(7), Box::new(Guard::Not(ap(10))));
        assert_eq!(truth_table_models(&wide, 12), 1024);
        // The disjunction of 16 atoms misses one assignment.
        let any = (1..16).fold(Guard::Ap(0), |g, i| Guard::Or(Box::new(g), ap(i)));
        assert_eq!(truth_table_models(&any, 16), 65535);
    }

    #[test]
    fn renumbering_keeps_index_order() {
        let guard = Guard::And(ap(5), Box::new(Guard::Not(ap(2))));
        let (local, n) = renumber(&guard);
        assert_eq!(n, 2);
        assert_eq!(local, Guard::And(ap(1), Box::new(Guard::Not(ap(0)))));
    }

    #[test]
    fn lassos_count_once_and_only_when_accepted() {
        let edge = |guard: Guard, target: u32, marks: Marks| RawEdge {
            guard,
            target,
            marks,
        };
        let automaton = |state_marks: Vec<Marks>, edges: Vec<Vec<RawEdge>>| Raw {
            aps: vec!["a".into()],
            start: 0,
            all_marks: 1,
            state_marks,
            edges,
        };
        let limits = Limits {
            timeout: Duration::from_secs(1),
            memory_bytes: 1 << 30,
        };
        let mut counter = Counter::new(&Formula::tt(), Semantics::Infinite, limits);
        let mut count = |a: &Raw, universe: usize| counter.log_count(a, universe).unwrap();
        let k = BOUND as f64;
        let close = |found: f64, expected: f64| {
            assert!(
                (found - expected).abs() < 1e-9,
                "{found} against {expected}"
            );
        };
        // Every lasso is accepted: k splits of 2^k pairs each. Two runs read
        // every word, but each pair counts once.
        let twice = automaton(
            vec![0, 1, 1],
            vec![
                vec![edge(Guard::True, 1, 0), edge(Guard::True, 2, 0)],
                vec![edge(Guard::True, 1, 0)],
                vec![edge(Guard::True, 2, 0)],
            ],
        );
        close(count(&twice, 1), k + k.log2());
        // a leads to a state with no accepting cycle, so only the lassos
        // whose word starts with !a count: half of each split.
        let dead_end = automaton(
            vec![0, 1, 1],
            vec![
                vec![edge(Guard::Ap(0), 1, 0), edge(Guard::Not(ap(0)), 2, 0)],
                vec![],
                vec![edge(Guard::True, 2, 0)],
            ],
        );
        close(count(&dead_end, 1), k - 1.0 + k.log2());
        // A cycle without the acceptance set is not live; with it on the
        // edge, it is.
        let unmarked = automaton(vec![0], vec![vec![edge(Guard::True, 0, 0)]]);
        assert_eq!(count(&unmarked, 1), f64::NEG_INFINITY);
        let marked = automaton(vec![0], vec![vec![edge(Guard::True, 0, 1)]]);
        close(count(&marked, 1), k + k.log2());
        // G F a: the loop v must hold an a, so a split with |v| = j has
        // 2^(k-j) (2^j - 1) pairs, k 2^k - 2^k + 1 in all.
        let infinitely = automaton(
            vec![0],
            vec![vec![
                edge(Guard::Ap(0), 0, 1),
                edge(Guard::Not(ap(0)), 0, 0),
            ]],
        );
        let top = 2f64.powi(BOUND as i32);
        close(count(&infinitely, 1), (k * top - top + 1.0).log2());
        // Far past f64's range the logarithm is still finite: k 2^(100 k).
        close(count(&marked, 100), 100.0 * k + k.log2());
        // A monoid past its budget is a failed count, not a wrong one.
        let within = |bytes, work| Budget { bytes, work };
        let error = counter
            .log_count_within(&infinitely, 1, within(ENTRY_BYTES, COUNT_WORK))
            .unwrap_err();
        assert!(error.contains("transition monoid"), "{error}");
        assert!(counter
            .log_count_within(&infinitely, 1, within(4096, COUNT_WORK))
            .is_ok());
        // So is a count past its work budget.
        let error = counter
            .log_count_within(&infinitely, 1, within(MONOID_BYTES, 10))
            .unwrap_err();
        assert!(error.contains("steps"), "{error}");
        assert!(counter
            .log_count_within(&infinitely, 1, within(MONOID_BYTES, 1000))
            .is_ok());
    }

    #[test]
    fn finite_words_count_when_their_last_transition_accepts() {
        let edge = |guard: Guard, target: u32, marks: Marks| RawEdge {
            guard,
            target,
            marks,
        };
        let automaton = |edges: Vec<Vec<RawEdge>>| Raw {
            aps: vec!["a".into()],
            start: 0,
            all_marks: 1,
            state_marks: vec![0; edges.len()],
            edges,
        };
        let limits = Limits {
            timeout: Duration::from_secs(1),
            memory_bytes: 1 << 30,
        };
        let mut counter = Counter::new(&Formula::tt(), Semantics::Finite, limits);
        let mut count = |a: &Raw, universe: usize| counter.log_count_finite(a, universe).unwrap();
        let sum = |terms: &dyn Fn(i32) -> f64| (1..=BOUND as i32).map(terms).sum::<f64>().log2();
        let close = |found: f64, expected: f64| {
            assert!(
                (found - expected).abs() < 1e-9,
                "{found} against {expected}"
            );
        };
        // ltlf2dfa's automaton of X a: every word of length 1, and of the
        // longer ones those whose second letter is a.
        let next = automaton(vec![
            vec![edge(Guard::True, 1, 1)],
            vec![edge(Guard::Ap(0), 2, 1)],
            vec![edge(Guard::True, 2, 1)],
        ]);
        close(
            count(&next, 1),
            (2.0 + (2..=BOUND as i32).map(|n| 2f64.powi(n - 1)).sum::<f64>()).log2(),
        );
        // An unmarked last step rejects: X[!] true has every word but those
        // of length 1.
        let strong = automaton(vec![
            vec![edge(Guard::True, 1, 0)],
            vec![edge(Guard::True, 1, 1)],
        ]);
        close(
            count(&strong, 1),
            sum(&|n| if n == 1 { 0.0 } else { 2f64.powi(n) }),
        );
        // A free atom doubles each letter, and disjoint edges add up.
        let split = automaton(vec![vec![
            edge(Guard::Ap(0), 0, 1),
            edge(Guard::Not(ap(0)), 0, 1),
        ]]);
        close(count(&split, 2), sum(&|n| 4f64.powi(n)));
        let none = automaton(vec![vec![edge(Guard::True, 0, 0)]]);
        assert_eq!(count(&none, 1), f64::NEG_INFINITY);
    }

    /// The on bits of the bitset `set`, in order.
    fn bits(set: &[u64]) -> impl Iterator<Item = usize> + '_ {
        set.iter().enumerate().flat_map(|(w, &word)| {
            let mut rest = word;
            std::iter::from_fn(move || {
                if rest == 0 {
                    return None;
                }
                let b = rest.trailing_zeros() as usize;
                rest &= rest - 1;
                Some(w * 64 + b)
            })
        })
    }

    /// [`Components::accepting`] by Warshall's closure, as the count first
    /// computed it.
    fn accepting_by_closure(rows: &[Vec<u64>], words: usize, planes: usize) -> Vec<u64> {
        let n = rows.len();
        let mut reach: Vec<Vec<u64>> = rows.iter().map(|r| r[..words].to_vec()).collect();
        for k in 0..n {
            let through = reach[k].clone();
            for row in reach.iter_mut().filter(|row| has(row, k)) {
                row.iter_mut().zip(&through).for_each(|(a, b)| *a |= b);
            }
        }
        let mut on_cycle = vec![0u64; words];
        for s in (0..n).filter(|&s| has(&reach[s], s)) {
            let mut component = vec![0u64; words];
            for t in bits(&reach[s]).filter(|&t| has(&reach[t], s)) {
                component[t / 64] |= 1 << (t % 64);
            }
            let inside = |plane: usize| {
                bits(&component).any(|t| {
                    rows[t][plane * words..(plane + 1) * words]
                        .iter()
                        .zip(&component)
                        .any(|(a, b)| a & b != 0)
                })
            };
            if (1..planes).all(inside) {
                on_cycle[s / 64] |= 1 << (s % 64);
            }
        }
        let mut states = on_cycle.clone();
        for (p, row) in reach.iter().enumerate() {
            if row.iter().zip(&on_cycle).any(|(a, b)| a & b != 0) {
                states[p / 64] |= 1 << (p % 64);
            }
        }
        states
    }

    #[test]
    fn components_find_the_states_of_a_closure() {
        // Random graphs of 1 to 150 states, sparse and dense, with up to
        // two acceptance planes, each plane a subset of plane 0.
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for case in 0..600 {
            let n = 1 + (random() % 150) as usize;
            let words = n.div_ceil(64);
            let planes = 1 + case % 3;
            let density = [2, 8, 64, 400][case % 4];
            let rows: Vec<Vec<u64>> = (0..n)
                .map(|_| {
                    let mut row = vec![0u64; planes * words];
                    for q in 0..n {
                        if random() % 1000 < density {
                            row[q / 64] |= 1 << (q % 64);
                            for plane in 1..planes {
                                if random() % 2 == 0 {
                                    row[plane * words + q / 64] |= 1 << (q % 64);
                                }
                            }
                        }
                    }
                    row
                })
                .collect();
            let mut components = Components {
                ids: (0..n as u32).collect(),
                rows: rows.iter().flat_map(|r| r[..words].to_vec()).collect(),
                ..Components::default()
            };
            assert_eq!(
                components.accepting(n, words, planes, &rows),
                accepting_by_closure(&rows, words, planes),
                "case {case}: {n} states, {planes} planes"
            );
        }
    }

    #[test]
    fn similarity_follows_peredurs_boundaries() {
        let ninf = f64::NEG_INFINITY;
        assert_eq!(similarity_from_logs(ninf, ninf, ninf), 1.0);
        assert_eq!(similarity_from_logs(3.0, ninf, ninf), 0.0);
        assert_eq!(similarity_from_logs(4.0, 4.0, 4.0), 1.0);
        // φ ⇒ φ': shared is φ's count, so the score is 2r / (1 + r) for
        // r = log count(φ) / log count(φ').
        let r: f64 = 10.0 / 20.0;
        assert!((similarity_from_logs(10.0, 20.0, 10.0) - 2.0 * r / (1.0 + r)).abs() < 1e-12);
        // Disjoint languages share nothing.
        assert_eq!(similarity_from_logs(10.0, 20.0, ninf), 0.0);
        // A count of 1 against a larger shared count is contained.
        assert_eq!(similarity_from_logs(0.0, 0.0, 0.0), 1.0);
    }

    #[test]
    fn rejection_is_the_log_share_of_the_negation() {
        let ninf = f64::NEG_INFINITY;
        // ⊤ rejects nothing; φ and ⊤ against ⊤ both reject nothing.
        assert_eq!(rejection_from_logs(10.0, ninf), 0.0);
        assert_eq!(rejection_from_logs(ninf, ninf), 1.0);
        assert_eq!(rejection_from_logs(ninf, 3.0), 0.0);
        assert_eq!(rejection_from_logs(10.0, 10.0), 1.0);
        assert_eq!(rejection_from_logs(10.0, 4.0), 0.4);
        // A larger negation than φ's is clamped.
        assert_eq!(rejection_from_logs(10.0, 12.0), 1.0);
        // One violating lasso: membership.
        assert_eq!(rejection_from_logs(0.0, 0.0), 1.0);
        assert_eq!(rejection_from_logs(5.0, 0.0), 0.0);
    }
}
