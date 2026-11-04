//! The evaluator of the trace-set back end and of the grader
//! (`search::grade`): [`Lasso::satisfies`] over runs of at
//! most 64 positions, one machine word per subformula.
//!
//! A run is packed once, as one `u64` per atom with bit `i` set where the
//! atom holds at position `i`. A formula is compiled once per check into a
//! post-order program over the trace set's atom indices, then run on each
//! packed run with a stack of words. The boolean operators are one word
//! operation each; `X` is a shift; `F`, `G`, `U`, `W` and `R` are the same
//! backward fixpoint passes as `Lasso::fixpoint`, bit by bit. Nothing is
//! allocated per run beyond the reused stack, where `Lasso::satisfies`
//! allocates a vector per subformula and looks every atom up in a set per
//! position. The semantics are the same, and a unit test checks the two
//! agree on random formulas and runs.

use std::sync::Arc;

use super::Lasso;
use crate::ltl::{BinaryOp, Formula, Node, UnaryOp};

/// The longest run this module packs.
pub(super) const MAX_LEN: usize = 64;

/// A run packed into words.
#[derive(Clone, Debug)]
pub(crate) struct Packed {
    len: usize,
    /// The cycle's first position, or `len` for a finite word.
    start: usize,
    /// One word per atom of the trace set, in its order.
    atoms: Vec<u64>,
}

impl Packed {
    /// Packs `run` over `atoms`, or `None` when it is longer than
    /// [`MAX_LEN`].
    pub(crate) fn new(run: &Lasso, atoms: &[Arc<str>]) -> Option<Self> {
        let positions = [run.prefix(), run.cycle()].concat();
        if positions.len() > MAX_LEN {
            return None;
        }
        let words = atoms
            .iter()
            .map(|atom| {
                positions
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.contains(atom))
                    .fold(0u64, |w, (i, _)| w | 1 << i)
            })
            .collect();
        Some(Packed {
            len: positions.len(),
            start: run.prefix().len(),
            atoms: words,
        })
    }

    /// A run of `len` positions, 1 to [`MAX_LEN`], already in words: bit
    /// `i` of `atoms[k]` is atom `k` at position `i`. The cycle starts at
    /// `start`, and `start == len` makes a finite word.
    pub(crate) fn from_words(len: usize, start: usize, atoms: Vec<u64>) -> Self {
        assert!((1..=MAX_LEN).contains(&len) && start <= len);
        Packed { len, start, atoms }
    }

    fn mask(&self) -> u64 {
        u64::MAX >> (64 - self.len)
    }

    fn finite(&self) -> bool {
        self.start == self.len
    }

    /// Bit `i` is `x` at the position after `i`; past the end of a finite
    /// word it is true, as weak next is.
    fn next(&self, x: u64) -> u64 {
        let last = self.len - 1;
        let wrap = if self.finite() {
            true
        } else {
            x >> self.start & 1 == 1
        };
        (x >> 1) | (u64::from(wrap) << last)
    }

    /// `Lasso::fixpoint` on words: `v[i] = step(l[i], r[i], v[succ(i)])`,
    /// least when `init` is false, greatest when it is true.
    fn fixpoint(&self, init: bool, l: u64, r: u64, step: fn(bool, bool, bool) -> bool) -> u64 {
        let bit = |w: u64, i: usize| w >> i & 1 == 1;
        let mut v = if init { self.mask() } else { 0 };
        let set = |v: &mut u64, i: usize, b: bool| {
            *v = (*v & !(1 << i)) | (u64::from(b) << i);
        };
        let n = self.len;
        for _ in 0..2 {
            for i in (self.start..n).rev() {
                let next = bit(v, if i + 1 < n { i + 1 } else { self.start });
                let b = step(bit(l, i), bit(r, i), next);
                set(&mut v, i, b);
            }
        }
        let mut next = if self.finite() {
            init
        } else {
            bit(v, self.start)
        };
        for i in (0..self.start).rev() {
            next = step(bit(l, i), bit(r, i), next);
            set(&mut v, i, next);
        }
        v
    }
}

/// One step of a compiled formula.
#[derive(Clone, Copy, Debug)]
enum Op {
    True,
    False,
    /// An atom by its index in the trace set, or `None` for an atom the set
    /// does not trace, which is false everywhere.
    Atom(Option<usize>),
    Unary(UnaryOp),
    Binary(BinaryOp),
}

/// A formula compiled against a trace set's atoms.
pub(crate) struct Program {
    ops: Vec<Op>,
}

impl Program {
    /// Compiles `formula`, reading atoms by their index in `atoms`.
    pub(crate) fn new(formula: &Formula, atoms: &[Arc<str>]) -> Self {
        fn walk(f: &Formula, atoms: &[Arc<str>], ops: &mut Vec<Op>) {
            match f.node() {
                Node::True => ops.push(Op::True),
                Node::False => ops.push(Op::False),
                Node::Atom(name) => ops.push(Op::Atom(atoms.iter().position(|a| a == name))),
                Node::Unary(op, child) => {
                    walk(child, atoms, ops);
                    ops.push(Op::Unary(*op));
                }
                Node::Binary(op, lhs, rhs) => {
                    walk(lhs, atoms, ops);
                    walk(rhs, atoms, ops);
                    ops.push(Op::Binary(*op));
                }
            }
        }
        let mut ops = Vec::with_capacity(formula.n_subformulae());
        walk(formula, atoms, &mut ops);
        Program { ops }
    }

    /// Whether `run` satisfies the formula from its first position.
    /// `stack` is scratch space, reused across calls.
    pub(crate) fn satisfies(&self, run: &Packed, stack: &mut Vec<u64>) -> bool {
        stack.clear();
        let mask = run.mask();
        for op in &self.ops {
            let word = match *op {
                Op::True => mask,
                Op::False => 0,
                Op::Atom(index) => index.map_or(0, |k| run.atoms[k]),
                Op::Unary(op) => {
                    let x = stack.pop().expect("an operand");
                    match op {
                        UnaryOp::Not => !x & mask,
                        UnaryOp::Next => run.next(x),
                        UnaryOp::Eventually => run.fixpoint(false, 0, x, |_, r, next| r || next),
                        UnaryOp::Globally => run.fixpoint(true, 0, x, |_, r, next| r && next),
                    }
                }
                Op::Binary(op) => {
                    let r = stack.pop().expect("an operand");
                    let l = stack.pop().expect("an operand");
                    match op {
                        BinaryOp::And => l & r,
                        BinaryOp::Or => l | r,
                        BinaryOp::Implies => (!l | r) & mask,
                        BinaryOp::Iff => !(l ^ r) & mask,
                        BinaryOp::Until => run.fixpoint(false, l, r, |l, r, n| r || (l && n)),
                        BinaryOp::WeakUntil => run.fixpoint(true, l, r, |l, r, n| r || (l && n)),
                        BinaryOp::Release => run.fixpoint(true, l, r, |l, r, n| r && (l || n)),
                    }
                }
            };
            stack.push(word);
        }
        stack.pop().expect("a result") & 1 == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::Valuation;
    use crate::search::Rng;

    const ATOMS: [&str; 3] = ["a", "b", "c"];
    const UNARY: [UnaryOp; 4] = [
        UnaryOp::Not,
        UnaryOp::Next,
        UnaryOp::Eventually,
        UnaryOp::Globally,
    ];
    const BINARY: [BinaryOp; 7] = [
        BinaryOp::And,
        BinaryOp::Or,
        BinaryOp::Implies,
        BinaryOp::Iff,
        BinaryOp::Until,
        BinaryOp::WeakUntil,
        BinaryOp::Release,
    ];

    fn formula(rng: &mut Rng, depth: usize) -> Formula {
        if depth == 0 || rng.below(4) == 0 {
            return match rng.below(8) {
                0 => Formula::tt(),
                1 => Formula::ff(),
                // `d` is not traced, so it is false everywhere.
                2 => Formula::atom("d"),
                _ => Formula::atom(ATOMS[rng.below(3)]),
            };
        }
        if rng.below(3) == 0 {
            Formula::unary(UNARY[rng.below(4)], formula(rng, depth - 1))
        } else {
            let lhs = formula(rng, depth - 1);
            Formula::binary(BINARY[rng.below(7)], lhs, formula(rng, depth - 1))
        }
    }

    fn positions(rng: &mut Rng, n: usize) -> Vec<Valuation> {
        (0..n)
            .map(|_| {
                ATOMS
                    .iter()
                    .filter(|_| rng.coin())
                    .map(|&a| a.into())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn agrees_with_the_lasso_evaluator() {
        let atoms: Vec<Arc<str>> = ATOMS.iter().map(|&a| a.into()).collect();
        let mut rng = Rng::new(11);
        let mut stack = Vec::new();
        let mut held = 0;
        for case in 0..4000 {
            // Lengths up to 64 in both shapes, with the edge cases often.
            let n = match rng.below(4) {
                0 => 1,
                1 => MAX_LEN,
                _ => 1 + rng.below(MAX_LEN),
            };
            let run = if case % 2 == 0 {
                Lasso::finite(positions(&mut rng, n)).unwrap()
            } else {
                let cycle = 1 + rng.below(n);
                let prefix = positions(&mut rng, n - cycle);
                Lasso::new(prefix, positions(&mut rng, cycle)).unwrap()
            };
            let f = formula(&mut rng, 5);
            let packed = Packed::new(&run, &atoms).unwrap();
            let ours = Program::new(&f, &atoms).satisfies(&packed, &mut stack);
            held += usize::from(ours);
            assert_eq!(ours, run.satisfies(&f), "{f} on {run:?}");
        }
        assert!(held > 800 && held < 3200, "{held} of 4000 hold");
    }

    #[test]
    fn a_long_run_is_not_packed() {
        let atoms: Vec<Arc<str>> = vec!["a".into()];
        let run = Lasso::finite(vec![Valuation::new(); MAX_LEN + 1]).unwrap();
        assert!(Packed::new(&run, &atoms).is_none());
    }
}
