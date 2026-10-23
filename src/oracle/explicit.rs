//! Tier 3a of the oracle: an in-memory Kripke structure, Spot's automaton
//! for the negated candidate, and an emptiness check of their product.
//!
//! Each query runs `ltl2tgba -H` on `!(φ)` through [`process::run`], reads
//! the generalised Büchi automaton with [`super::hoa`], and searches the
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

impl Backend for Explicit {
    fn check(&mut self, formula: &Formula) -> Verdict {
        let negated = format!("!({formula})\n");
        let mut command = Command::new("ltl2tgba");
        command.arg("-H");
        let output = match process::run(&mut command, &negated, self.limits) {
            Ok(output) => output,
            Err(error) => return Verdict::Unknown(format!("ltl2tgba: {error}")),
        };
        if !output.status.success() {
            return Verdict::Unknown(format!("ltl2tgba: {}", output.stderr.trim()));
        }
        match hoa::parse(&output.stdout) {
            Ok(automaton) => match accepting_run(&self.kripke, &automaton) {
                Some(lasso) => Verdict::Fails(lasso),
                None => Verdict::Holds,
            },
            Err(reason) => Verdict::Unknown(format!("ltl2tgba output: {reason}")),
        }
    }
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
