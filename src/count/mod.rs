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

use crate::ltl::{BinaryOp, Formula};
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
            limits,
            logs: HashMap::new(),
            similarities: HashMap::new(),
            ganak: HashMap::new(),
            counts: Counts::default(),
        }
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
            for formula in [self.phi.clone(), key.clone(), self.conjunction(key)] {
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
                let log = |f: Formula| self.logs[&(f, universe)];
                let score = match (
                    log(self.phi.clone()),
                    log(key.clone()),
                    log(self.conjunction(key)),
                ) {
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

    fn universe(&self, candidate: &Formula) -> usize {
        let mut atoms = candidate.atoms();
        atoms.extend(self.phi_atoms.iter().cloned());
        atoms.len()
    }

    fn conjunction(&self, candidate: &Formula) -> Formula {
        Formula::binary(BinaryOp::And, self.phi.clone(), candidate.clone()).canonical()
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
        for _ in 0..BOUND {
            let mut next: BTreeMap<Vec<u32>, f64> = BTreeMap::new();
            for (subset, weight) in &weights {
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
        let mut monoid = Monoid::new(n, automaton.all_marks, &letters);
        // The loops of each length by relation id. The monoid is finite,
        // so the ids stop growing and each step is a sparse product over
        // a fixed table, in id order.
        let identity = monoid.identity();
        let mut loops = vec![0.0; identity + 1];
        loops[identity] = 1.0;
        let mut scale = 0.0;
        let mut terms = Vec::with_capacity(BOUND);
        for length in 1..=BOUND {
            let mut next = vec![0.0; monoid.relations.len()];
            for (relation, &weight) in loops.iter().enumerate() {
                if weight == 0.0 {
                    continue;
                }
                for (letter, &to) in letters.iter().zip(monoid.successors(relation)) {
                    if to >= next.len() {
                        next.resize(to + 1, 0.0);
                    }
                    next[to] += weight * letter.models;
                }
            }
            let top = next.iter().copied().fold(0.0, f64::max);
            if top > 0.0 {
                next.iter_mut().for_each(|w| *w /= top);
                scale += top.log2();
            }
            loops = next;
            // The loops by the states that accept their ω-power.
            let mut by_good = vec![0.0; monoid.goods.len()];
            for (relation, &weight) in loops.iter().enumerate() {
                if weight != 0.0 {
                    let good = monoid.good(relation);
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
    let containment = |whole: f64| {
        if whole <= 0.0 {
            return if shared >= whole { 1.0 } else { 0.0 };
        }
        if shared <= 0.0 {
            return 0.0;
        }
        (shared / whole).clamp(0.0, 1.0)
    };
    let (first, second) = (containment(phi), containment(candidate));
    if first == 0.0 && second == 0.0 {
        return 0.0;
    }
    2.0 * first * second / (first + second)
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

/// The relations of the loops v met so far, interned. A relation has one
/// row per state, and a row is a stack of `planes` bitsets of `words`
/// 64-bit words: plane 0 holds q in row p when some run on v leads from p
/// to q, and plane 1 + i when some such run sees acceptance set i. A
/// letter is a relation of length 1.
///
/// Row p of v·a depends only on row p of v and on a, so rows are
/// interned on their own, a relation is its list of row ids, and the
/// step of a row by a letter is computed once and kept in a table. A
/// step of a relation is then one table lookup per state. Each
/// relation's set of accepting states, a bitset, is interned too.
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
    ids: HashMap<Rc<[u32]>, usize, BuildHasherDefault<WordHasher>>,
    relations: Vec<Rc<[u32]>>,
    steps: Vec<Vec<usize>>,
    good_of: Vec<Option<usize>>,
    good_ids: HashMap<Vec<u64>, usize>,
    goods: Vec<Vec<u64>>,
}

impl Monoid {
    fn new(n: usize, all_marks: Marks, letters: &[Letter]) -> Self {
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
            ids: HashMap::default(),
            relations: Vec::new(),
            steps: Vec::new(),
            good_of: Vec::new(),
            good_ids: HashMap::new(),
            goods: Vec::new(),
        }
    }

    fn intern_row(&mut self, row: &[u64]) -> u32 {
        if let Some(&id) = self.row_ids.get(row) {
            return id;
        }
        let row: Rc<[u64]> = row.into();
        let id = self.rows.len() as u32;
        self.rows.push(row.clone());
        self.row_ids.insert(row, id);
        self.row_steps
            .resize(self.rows.len() * self.letters.len(), u32::MAX);
        id
    }

    fn intern(&mut self, relation: &[u32]) -> usize {
        if let Some(&id) = self.ids.get(relation) {
            return id;
        }
        let relation: Rc<[u32]> = relation.into();
        self.relations.push(relation.clone());
        self.ids.insert(relation, self.relations.len() - 1);
        self.relations.len() - 1
    }

    /// The relation of the empty word.
    fn identity(&mut self) -> usize {
        let relation: Vec<u32> = (0..self.n)
            .map(|p| {
                let mut row = vec![0; self.planes * self.words];
                row[p / 64] |= 1 << (p % 64);
                self.intern_row(&row)
            })
            .collect();
        self.intern(&relation)
    }

    /// The relation of v followed by each letter, by letter index.
    fn successors(&mut self, relation: usize) -> &[usize] {
        if self.steps.len() <= relation {
            self.steps.resize(relation + 1, Vec::new());
        }
        if self.steps[relation].is_empty() {
            let from = self.relations[relation].clone();
            let mut next = vec![0; self.n];
            let out = (0..self.letters.len())
                .map(|letter| {
                    for (slot, &row) in next.iter_mut().zip(from.iter()) {
                        *slot = self.row_step(row, letter);
                    }
                    self.intern(&next)
                })
                .collect();
            self.steps[relation] = out;
        }
        &self.steps[relation]
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

    /// The words of plane `plane` in row `p` of `relation`.
    fn plane(&self, relation: usize, plane: usize, p: usize) -> &[u64] {
        let row = &self.rows[self.relations[relation][p] as usize];
        &row[plane * self.words..(plane + 1) * self.words]
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
        let (n, words) = (self.n, self.words);
        // reach[p]: the states reachable from p in one step or more.
        // Warshall's closure on bit rows: after round k, row p holds every
        // state reached by a path whose inner states are below k.
        let mut reach: Vec<Vec<u64>> = (0..n)
            .map(|p| self.plane(relation, 0, p).to_vec())
            .collect();
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
                    self.plane(relation, plane, t)
                        .iter()
                        .zip(&component)
                        .any(|(a, b)| a & b != 0)
                })
            };
            if (1..self.planes).all(inside) {
                on_cycle[s / 64] |= 1 << (s % 64);
            }
        }
        let mut states = on_cycle.clone();
        for (p, row) in reach.iter().enumerate() {
            if row.iter().zip(&on_cycle).any(|(a, b)| a & b != 0) {
                states[p / 64] |= 1 << (p % 64);
            }
        }
        let good = match self.good_ids.get(&states) {
            Some(&good) => good,
            None => {
                self.goods.push(states.clone());
                self.good_ids.insert(states, self.goods.len() - 1);
                self.goods.len() - 1
            }
        };
        self.good_of[relation] = Some(good);
        good
    }
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
}
