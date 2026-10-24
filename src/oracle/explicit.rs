//! Tier 3a of the oracle: an in-memory Kripke structure, Spot's automaton
//! for the negated candidate, and an emptiness check of their product.
//!
//! Each query runs `ltl2tgba -H` on `!(φ)` through [`process::run`], reads
//! the generalised Büchi automaton with the private `hoa` reader, and searches the
//! product on the fly from (Kripke state 0, initial automaton state). A
//! non-empty product gives a run of M that satisfies ¬φ, returned as the
//! lasso of a failing verdict.
//!
//! **Positions.** A product state (s, q) means "the run is at Kripke state
//! s and the automaton, in state q, is about to read L(s)". Its successors
//! are (s', q') for each Kripke successor s' and each edge q → q' whose
//! label holds in L(s). The automaton thus reads L(s₀) L(s₁) … from its
//! initial state, as HOA defines a run, and position i of the returned
//! lasso is L(sᵢ), as [`Lasso::satisfies`] reads it.
//!
//! On a RERS structure the positions are the model's actions (see
//! `docs/dev/rers-kripke.md`), so a formula with `X` is decided over those
//! positions. SPIN cannot do that, since its `X` counts its own steps.
//!
//! **Emptiness.** The search is Couvreur's SCC-based check (Couvreur 1999,
//! the algorithm behind Spot's `couvreur99`). It
//! handles several acceptance sets directly, so the automaton needs no
//! degeneralisation, which would multiply its states by the number of
//! sets, and it stops at the first SCC that is found to hold every set,
//! often before the whole product is built. Nested DFS needs a Büchi
//! automaton and so the degeneralised one. The depth-first search keeps an
//! explicit stack of frames rather than recursing, so a million-state
//! product cannot overflow the thread stack.

use std::collections::{HashMap, VecDeque};
use std::hash::BuildHasherDefault;
use std::process::Command;

use super::hoa::{self, Automaton, Marks};
use super::process::{self, Limits};
use super::{Backend, Lasso, Valuation, Verdict};
use crate::kripke::{Kripke, StateId};
use crate::ltl::Formula;
use crate::rers::KeyHasher;
use crate::search::Rng;

/// A [`Backend`] that checks candidates against one Kripke structure held
/// in memory. Only the Spot translation runs as a child process.
pub struct Explicit {
    kripke: Kripke,
    limits: Limits,
}

impl Explicit {
    /// A back end for `kripke`. `limits` apply to each `ltl2tgba` call.
    pub fn new(kripke: Kripke, limits: Limits) -> Self {
        Explicit { kripke, limits }
    }
}

impl Explicit {
    /// Spot's automaton for `!(formula)`.
    fn negation(&self, formula: &Formula) -> Result<Automaton, String> {
        let negated = format!("!({formula})\n");
        let mut command = Command::new("ltl2tgba");
        command.arg("-H");
        let output = process::run(&mut command, &negated, self.limits)
            .map_err(|error| format!("ltl2tgba: {error}"))?;
        if !output.status.success() {
            return Err(format!("ltl2tgba: {}", output.stderr.trim()));
        }
        hoa::parse(&output.stdout).map_err(|reason| format!("ltl2tgba output: {reason}"))
    }

    /// Up to `count` random runs of the structure, each a walk from state 0
    /// that picks a uniform successor until it revisits a state, within
    /// `max_length` steps. A walk that does not close in time is dropped.
    pub fn random_lassos(&self, count: usize, max_length: usize, rng: &mut Rng) -> Vec<Lasso> {
        let valuation =
            |s: StateId| -> Valuation { self.kripke.atom(s).into_iter().cloned().collect() };
        let mut out = Vec::new();
        for _ in 0..count {
            let mut seen: HashMap<StateId, usize> = HashMap::new();
            let mut path = vec![0];
            seen.insert(0, 0);
            while path.len() <= max_length {
                let successors = self.kripke.successors(*path.last().expect("non-empty"));
                if successors.is_empty() {
                    break;
                }
                let next = successors[rng.below(successors.len())];
                if let Some(&at) = seen.get(&next) {
                    let prefix = path[..at].iter().map(|&s| valuation(s)).collect();
                    let cycle = path[at..].iter().map(|&s| valuation(s)).collect();
                    out.push(Lasso::new(prefix, cycle).expect("the cycle has a state"));
                    break;
                }
                seen.insert(next, path.len());
                path.push(next);
            }
        }
        out
    }
}

impl Backend for Explicit {
    fn check(&mut self, formula: &Formula) -> Verdict {
        match self.negation(formula) {
            Ok(automaton) => match accepting_run(&self.kripke, &automaton) {
                Some(lasso) => Verdict::Fails(lasso),
                None => Verdict::Holds,
            },
            Err(reason) => Verdict::Unknown(reason),
        }
    }

    fn holding_states(&mut self, formula: &Formula) -> Option<(usize, usize)> {
        let automaton = self.negation(formula).ok()?;
        Some((
            holding_states(&self.kripke, &automaton),
            self.kripke.state_count(),
        ))
    }
}

/// The number of states s of `kripke` from which every run satisfies the
/// formula whose negation `automaton` accepts: those where (s, start) of
/// the full product reaches no accepting SCC. Tarjan's algorithm finishes
/// each SCC after every SCC it reaches, so badness is known for every
/// successor when an SCC is closed.
fn holding_states(kripke: &Kripke, automaton: &Automaton) -> usize {
    let ap_index: HashMap<&str, u32> = (automaton.aps.iter().enumerate())
        .map(|(i, ap)| (ap.as_str(), i as u32))
        .collect();
    let none = automaton.aps.len() as u32;
    let letters = (kripke.atoms().iter())
        .map(|atom| ap_index.get(&**atom).copied().unwrap_or(none))
        .collect();
    let product = Product {
        kripke,
        automaton,
        letters,
    };
    let all = automaton.all_marks;
    // index[v] is v's DFS number from 0; low, on_stack and scc by number.
    let mut index: HashMap<State, u32, BuildHasherDefault<KeyHasher>> = HashMap::default();
    let mut low: Vec<u32> = Vec::new();
    let mut on_stack: Vec<bool> = Vec::new();
    let mut scc: Vec<u32> = Vec::new();
    let mut bad: Vec<bool> = Vec::new();
    let mut stack: Vec<State> = Vec::new();
    let mut dfs: Vec<Frame> = Vec::new();
    let unset = u32::MAX;
    for s in 0..kripke.state_count() as StateId {
        let root = pack(s, automaton.start);
        if index.contains_key(&root) {
            continue;
        }
        let mut next = Some(root);
        loop {
            if let Some(v) = next.take() {
                let n = low.len() as u32;
                index.insert(v, n);
                low.push(n);
                on_stack.push(true);
                scc.push(unset);
                stack.push(v);
                dfs.push(product.frame(v));
            }
            let Some(frame) = dfs.last_mut() else { break };
            let v = index[&frame.state] as usize;
            if let Some((t, _)) = product.next(frame) {
                match index.get(&t) {
                    None => next = Some(t),
                    Some(&m) if on_stack[m as usize] => low[v] = low[v].min(m),
                    Some(_) => {}
                }
                continue;
            }
            let state = frame.state;
            dfs.pop();
            if let Some(parent) = dfs.last() {
                let p = index[&parent.state] as usize;
                low[p] = low[p].min(low[v]);
            }
            if low[v] != v as u32 {
                continue;
            }
            // v roots an SCC: its members are the stack from v up.
            let id = bad.len() as u32;
            let at = stack.iter().rposition(|&u| u == state).expect("on stack");
            let members = stack.split_off(at);
            for u in &members {
                let n = index[u] as usize;
                on_stack[n] = false;
                scc[n] = id;
            }
            let (mut marks, mut internal, mut reaches_bad) = (0, false, false);
            for &u in &members {
                let mut f = product.frame(u);
                while let Some((t, m)) = product.next(&mut f) {
                    let other = scc[index[&t] as usize];
                    if other == id {
                        internal = true;
                        marks |= m;
                    } else if bad[other as usize] {
                        reaches_bad = true;
                    }
                }
            }
            bad.push(reaches_bad || (internal && marks & all == all));
        }
    }
    (0..kripke.state_count() as StateId)
        .filter(|&s| !bad[scc[index[&pack(s, automaton.start)] as usize] as usize])
        .count()
}

/// A product state: the Kripke state in the low half, the automaton state
/// in the high half. The multiplicative [`KeyHasher`] only carries bits
/// upwards, so the part that varies most must be low.
type State = u64;

fn pack(s: StateId, q: u32) -> State {
    (q as u64) << 32 | s as u64
}

fn kripke_state(state: State) -> StateId {
    state as StateId
}

fn automaton_state(state: State) -> usize {
    (state >> 32) as usize
}

/// The index of a state whose SCC is finished and holds no accepting cycle.
const DEAD: u32 = 0;

/// A depth-first frame: a state and how far its successors have been read.
struct Frame {
    state: State,
    edge: usize,
    successor: usize,
}

/// An SCC on the root stack: the DFS number of its first state, the
/// acceptance sets of the edges inside it, and those of the edge that
/// entered it.
struct Root {
    index: u32,
    marks: Marks,
    entry: Marks,
}

/// The product of a Kripke structure and an automaton.
struct Product<'a> {
    kripke: &'a Kripke,
    automaton: &'a Automaton,
    /// The letter of each Kripke atom: its AP index, or `aps.len()` when
    /// the automaton does not mention it.
    letters: Vec<u32>,
}

impl Product<'_> {
    fn letter(&self, s: StateId) -> u32 {
        match self.kripke.atom_index(s) {
            Some(atom) => self.letters[atom],
            None => self.automaton.aps.len() as u32,
        }
    }

    fn frame(&self, state: State) -> Frame {
        Frame {
            state,
            edge: 0,
            successor: 0,
        }
    }

    /// The next successor of `frame` and the marks of the edge to it.
    fn next(&self, frame: &mut Frame) -> Option<(State, Marks)> {
        let s = kripke_state(frame.state);
        let bit = 1 << self.letter(s);
        let edges = &self.automaton.edges[automaton_state(frame.state)];
        let successors = self.kripke.successors(s);
        while let Some(edge) = edges.get(frame.edge) {
            if edge.letters & bit != 0 {
                if let Some(&t) = successors.get(frame.successor) {
                    frame.successor += 1;
                    return Some((pack(t, edge.target), edge.marks));
                }
            }
            frame.edge += 1;
            frame.successor = 0;
        }
        None
    }

    fn valuation(&self, state: State) -> Valuation {
        self.kripke
            .atom(kripke_state(state))
            .into_iter()
            .cloned()
            .collect()
    }
}

/// A run of `kripke` that `automaton` accepts, if there is one.
fn accepting_run(kripke: &Kripke, automaton: &Automaton) -> Option<Lasso> {
    let ap_index: HashMap<&str, u32> = (automaton.aps.iter().enumerate())
        .map(|(i, ap)| (ap.as_str(), i as u32))
        .collect();
    let none = automaton.aps.len() as u32;
    let letters = (kripke.atoms().iter())
        .map(|atom| ap_index.get(&**atom).copied().unwrap_or(none))
        .collect();
    let product = Product {
        kripke,
        automaton,
        letters,
    };
    let all = automaton.all_marks;

    // DFS numbers count from 1, so DEAD never names a live state.
    let mut index: HashMap<State, u32, BuildHasherDefault<KeyHasher>> = HashMap::default();
    let mut count = 0;
    let mut roots: Vec<Root> = Vec::new();
    let mut live: Vec<State> = Vec::new();
    let mut dfs: Vec<Frame> = Vec::new();
    // The initial state enters as the target of an edge with no marks.
    let mut next = Some((pack(0, automaton.start), 0));

    loop {
        if let Some((t, marks)) = next.take() {
            match index.get(&t).copied() {
                None => {
                    count += 1;
                    index.insert(t, count);
                    roots.push(Root {
                        index: count,
                        marks: 0,
                        entry: marks,
                    });
                    live.push(t);
                    dfs.push(product.frame(t));
                }
                Some(DEAD) => {}
                Some(n) => {
                    // t is live, so this edge closes a cycle through every
                    // SCC whose root came after t's: merge them.
                    let mut merged = marks;
                    while roots.last().is_some_and(|r| r.index > n) {
                        let r = roots.pop().expect("checked");
                        merged |= r.marks | r.entry;
                    }
                    let top = roots.last_mut().expect("a live state has a root");
                    top.marks |= merged;
                    if top.marks & all == all {
                        let root = top.index;
                        return Some(lasso(&product, &index, &dfs, root));
                    }
                }
            }
        }
        let frame = dfs.last_mut()?;
        next = product.next(frame);
        if next.is_none() {
            // Every successor is done. If this state is a root, its SCC is
            // complete and holds no accepting cycle.
            let n = index[&frame.state];
            dfs.pop();
            if roots.last().is_some_and(|r| r.index == n) {
                roots.pop();
                while let Some(&v) = live.last() {
                    if index[&v] < n {
                        break;
                    }
                    index.insert(v, DEAD);
                    live.pop();
                }
            }
        }
    }
}

/// The lasso through the accepting SCC whose root has DFS number `root`:
/// the DFS path to the root, then a cycle in the SCC through every
/// acceptance set and back to the root.
fn lasso(
    product: &Product,
    index: &HashMap<State, u32, BuildHasherDefault<KeyHasher>>,
    dfs: &[Frame],
    root: u32,
) -> Lasso {
    // The SCC is every live state numbered from its root on.
    let member = |v: State| index.get(&v).is_some_and(|&n| n >= root);
    let at = dfs
        .iter()
        .position(|f| index[&f.state] == root)
        .expect("an open SCC's root is on the DFS stack");
    let start = dfs[at].state;
    let prefix: Vec<State> = dfs[..at].iter().map(|f| f.state).collect();

    let mut cycle = Vec::new();
    let mut missing = product.automaton.all_marks;
    let mut current = start;
    while missing != 0 {
        let (path, next, marks) = search(product, current, member, |v, m| {
            member(v) && m & missing != 0
        });
        cycle.extend(path);
        missing &= !marks;
        current = next;
    }
    // The last marked edge may already end at the root.
    if current != start || cycle.is_empty() {
        let (path, _, _) = search(product, current, member, |v, _| v == start);
        cycle.extend(path);
    }

    let valuations = |states: &[State]| states.iter().map(|&s| product.valuation(s)).collect();
    Lasso::new(valuations(&prefix), valuations(&cycle)).expect("the cycle has an edge")
}

/// A breadth-first search through the states where `inside` holds, from
/// `from`, for an edge u → v with `wanted(v, marks)`. Returns the states
/// from `from` to u, then v and the edge's marks. The caller guarantees
/// that such an edge is reachable.
fn search(
    product: &Product,
    from: State,
    inside: impl Fn(State) -> bool,
    wanted: impl Fn(State, Marks) -> bool,
) -> (Vec<State>, State, Marks) {
    let mut parent: HashMap<State, State, BuildHasherDefault<KeyHasher>> = HashMap::default();
    let mut queue = VecDeque::from([from]);
    parent.insert(from, from);
    while let Some(u) = queue.pop_front() {
        let mut frame = product.frame(u);
        while let Some((v, marks)) = product.next(&mut frame) {
            if wanted(v, marks) {
                let mut path = vec![u];
                let mut w = u;
                while w != from {
                    w = parent[&w];
                    path.push(w);
                }
                path.reverse();
                return (path, v, marks);
            }
            if inside(v) && !parent.contains_key(&v) {
                parent.insert(v, u);
                queue.push_back(v);
            }
        }
    }
    unreachable!("an accepting SCC has the edge the search wants")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A structure from `(atom, successors)` per state, over atoms a, b, c.
    fn kripke(states: &[(Option<usize>, &[StateId])]) -> Kripke {
        let mut k = Kripke::new(vec!["a".into(), "b".into(), "c".into()]);
        for (atom, successors) in states {
            k.push_state(*atom, successors);
        }
        k
    }

    fn automaton(body: &str, aps: &str, acceptance: &str) -> Automaton {
        let states = body.matches("State:").count();
        let text = format!(
            "HOA: v1\nStates: {states}\nStart: 0\nAP: {aps}\nAcceptance: {acceptance}\n--BODY--\n{body}--END--\n"
        );
        hoa::parse(&text).unwrap()
    }

    fn word(states: &[&str]) -> Vec<Valuation> {
        (states.iter())
            .map(|s| s.chars().map(|c| c.to_string().into()).collect())
            .collect()
    }

    /// The automaton for `a`: the first letter has a, then anything.
    const FIRST_A: &str = "State: 0\n[0] 1\nState: 1\n[t] 1\n";
    /// The automaton for `!a`.
    const FIRST_NOT_A: &str = "State: 0\n[!0] 1\nState: 1\n[t] 1\n";

    #[test]
    fn the_automaton_reads_the_label_of_the_current_state() {
        // a, then nothing forever. If the product read the label of the
        // successor instead, these two verdicts would swap.
        let k = kripke(&[(Some(0), &[1]), (None, &[1])]);
        let not_a = automaton(FIRST_NOT_A, "1 \"a\"", "0 t");
        assert_eq!(accepting_run(&k, &not_a), None);
        let a = automaton(FIRST_A, "1 \"a\"", "0 t");
        let lasso = accepting_run(&k, &a).unwrap();
        assert_eq!(lasso.prefix(), word(&["a"]));
        assert_eq!(lasso.cycle(), word(&[""]));
        assert!(!lasso.satisfies(&"!a".parse().unwrap()));
    }

    #[test]
    fn holding_states_counts_the_states_a_formula_holds_from() {
        // a, then b forever: state 0 is a, state 1 is b with a self-loop.
        let k = kripke(&[(Some(0), &[1]), (Some(1), &[1])]);
        // !a: holds from neither... `a` holds from state 0 only.
        let not_a = automaton(FIRST_NOT_A, "1 \"a\"", "0 t");
        assert_eq!(holding_states(&k, &not_a), 1);
        // G !a as the negation of F a: F a holds from state 0 only.
        let never_a = automaton("State: 0 {0}\n[!0] 0\n", "1 \"a\"", "1 Inf(0)");
        assert_eq!(holding_states(&k, &never_a), 1);
        // The negation of `true` accepts nothing, so it holds from both.
        let empty = automaton("State: 0\n", "1 \"a\"", "1 Inf(0)");
        assert_eq!(holding_states(&k, &empty), 2);
        // b loops back to a: F a now holds from both.
        let k = kripke(&[(Some(0), &[1]), (Some(1), &[0])]);
        assert_eq!(holding_states(&k, &never_a), 2);
    }

    #[test]
    fn a_random_lasso_is_a_run_of_the_structure() {
        // a, then b and c in either order forever.
        let k = kripke(&[(Some(0), &[1, 2]), (Some(1), &[2, 1]), (Some(2), &[1])]);
        let limits = Limits { timeout: std::time::Duration::from_secs(1), memory_bytes: 1 << 30 };
        let backend = Explicit::new(k, limits);
        let runs = backend.random_lassos(20, 10, &mut Rng::new(1));
        assert_eq!(runs.len(), 20);
        for run in &runs {
            assert_eq!(run.prefix().first(), Some(&word(&["a"])[0]));
            assert!(run.satisfies(&"a & X G (b | c)".parse().unwrap()), "{run:?}");
        }
    }

    /// `G F a & G F b` with one acceptance set per conjunct.
    const INF_A_INF_B: &str = "State: 0\n[!0&!1] 0\n[0&!1] 0 {0}\n[!0&1] 0 {1}\n[0&1] 0 {0 1}\n";

    #[test]
    fn an_accepting_cycle_needs_every_set_in_one_scc() {
        // c, then a loop through a and b. c is not an AP of the automaton.
        let k = kripke(&[(Some(2), &[1]), (Some(0), &[1, 2]), (Some(1), &[2])]);
        let inf = automaton(INF_A_INF_B, "2 \"a\" \"b\"", "2 Inf(0)&Inf(1)");
        // Here a and b are in different SCCs, the b one a sink.
        assert_eq!(accepting_run(&k, &inf), None);

        // Now b leads back to a, so {a, b} is one SCC with both sets.
        let k = kripke(&[(Some(2), &[1]), (Some(0), &[1, 2]), (Some(1), &[2, 1])]);
        let lasso = accepting_run(&k, &inf).unwrap();
        assert_eq!(lasso.prefix(), word(&["c"]));
        let formula = "G F a & G F b".parse().unwrap();
        assert!(lasso.satisfies(&formula), "{lasso:?}");
        assert!(lasso.cycle().len() >= 2, "{lasso:?}");
    }

    #[test]
    fn a_long_chain_does_not_overflow_the_stack() {
        // A million states with nothing, then a sink where a holds.
        let n: StateId = 1_000_000;
        let mut states: Vec<(Option<usize>, Vec<StateId>)> =
            (0..n).map(|s| (None, vec![s + 1])).collect();
        states.push((Some(0), vec![n]));
        let mut k = Kripke::new(vec!["a".into()]);
        for (atom, successors) in &states {
            k.push_state(*atom, successors);
        }
        // G !a, as a Büchi automaton: never a.
        let never_a = automaton("State: 0 {0}\n[!0] 0\n", "1 \"a\"", "1 Inf(0)");
        assert_eq!(accepting_run(&k, &never_a), None);
        // F a: some a, then anything.
        let eventually_a = automaton(
            "State: 0\n[!0] 0\n[0] 1\nState: 1 {0}\n[t] 1\n",
            "1 \"a\"",
            "1 Inf(0)",
        );
        let lasso = accepting_run(&k, &eventually_a).unwrap();
        // The automaton leaves its first state while reading the a, so the
        // a appears once in the prefix and then forever in the cycle.
        assert_eq!(lasso.prefix().len(), n as usize + 1);
        assert_eq!(lasso.cycle(), word(&["a"]));
    }
}
