//! Tier 1 of the oracle: evaluating a formula on a stored lasso.
//!
//! A lasso `prefix · cycle^ω` has only `prefix.len() + cycle.len()` distinct
//! positions, because position `i` past the end of the cycle behaves as
//! position `i - cycle.len()`. So the truth of each subformula is one bit per
//! distinct position, computed bottom-up. The temporal operators are
//! fixpoints of their one-step unfolding; on the cycle the least fixpoint
//! (`U`, `F`) starts from false and the greatest (`W`, `R`, `G`) from true.

use crate::ltl::{BinaryOp, Formula, Node, UnaryOp};

use super::Lasso;

impl Lasso {
    /// Does the infinite word `prefix · cycle^ω` satisfy `formula` from its
    /// first position? An atom holds at a position iff its name is in that
    /// position's valuation. Takes time linear in the formula size times the
    /// lasso length.
    pub fn satisfies(&self, formula: &Formula) -> bool {
        self.truth(formula)[0]
    }

    fn len(&self) -> usize {
        self.prefix.len() + self.cycle.len()
    }

    /// The position after `i`: the last cycle position loops back.
    fn succ(&self, i: usize) -> usize {
        if i + 1 < self.len() {
            i + 1
        } else {
            self.prefix.len()
        }
    }

    /// The truth of `formula` at every distinct position.
    fn truth(&self, formula: &Formula) -> Vec<bool> {
        let n = self.len();
        match formula.node() {
            Node::True => vec![true; n],
            Node::False => vec![false; n],
            Node::Atom(name) => self
                .prefix
                .iter()
                .chain(&self.cycle)
                .map(|valuation| valuation.contains(name))
                .collect(),
            Node::Unary(op, child) => {
                let x = self.truth(child);
                match op {
                    UnaryOp::Not => x.iter().map(|b| !b).collect(),
                    UnaryOp::Next => (0..n).map(|i| x[self.succ(i)]).collect(),
                    UnaryOp::Eventually => self.fixpoint(false, |i, next| x[i] || next),
                    UnaryOp::Globally => self.fixpoint(true, |i, next| x[i] && next),
                }
            }
            Node::Binary(op, lhs, rhs) => {
                let (l, r) = (self.truth(lhs), self.truth(rhs));
                match op {
                    BinaryOp::And => zip(&l, &r, |a, b| a && b),
                    BinaryOp::Or => zip(&l, &r, |a, b| a || b),
                    BinaryOp::Implies => zip(&l, &r, |a, b| !a || b),
                    BinaryOp::Iff => zip(&l, &r, |a, b| a == b),
                    BinaryOp::Until => self.fixpoint(false, |i, next| r[i] || (l[i] && next)),
                    BinaryOp::WeakUntil => self.fixpoint(true, |i, next| r[i] || (l[i] && next)),
                    BinaryOp::Release => self.fixpoint(true, |i, next| r[i] && (l[i] || next)),
                }
            }
        }
    }

    /// Solves `v[i] = step(i, v[succ(i)])` for a monotone `step`, as the
    /// least fixpoint when `init` is false and the greatest when it is true.
    ///
    /// The first backward pass over the cycle reads `init` at the loop-back,
    /// which makes the cycle's first position exact: every position it can
    /// reach is in that one pass. The second pass then reads that exact
    /// value, and the prefix needs only one pass.
    fn fixpoint(&self, init: bool, step: impl Fn(usize, bool) -> bool) -> Vec<bool> {
        let n = self.len();
        let start = self.prefix.len();
        let mut v = vec![init; n];
        for _ in 0..2 {
            for i in (start..n).rev() {
                v[i] = step(i, v[self.succ(i)]);
            }
        }
        for i in (0..start).rev() {
            v[i] = step(i, v[i + 1]);
        }
        v
    }
}

fn zip(l: &[bool], r: &[bool], op: impl Fn(bool, bool) -> bool) -> Vec<bool> {
    l.iter().zip(r).map(|(&a, &b)| op(a, b)).collect()
}

/// The runs of the model seen so far, each violating some earlier candidate.
/// A candidate that fails on any of them is false on the model.
#[derive(Debug, Default)]
pub struct LassoCache {
    lassos: Vec<Lasso>,
}

impl LassoCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores a run of the model.
    pub fn insert(&mut self, lasso: Lasso) {
        self.lassos.push(lasso);
    }

    /// The first stored run that violates `formula`, if any.
    pub fn refutes(&self, formula: &Formula) -> Option<&Lasso> {
        self.lassos.iter().find(|lasso| !lasso.satisfies(formula))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::Valuation;

    /// A lasso from strings of atom letters: `"ab"` is a position where `a`
    /// and `b` hold, `""` one where nothing does.
    fn lasso(prefix: &[&str], cycle: &[&str]) -> Lasso {
        let positions = |letters: &[&str]| -> Vec<Valuation> {
            letters
                .iter()
                .map(|s| s.chars().map(|c| c.to_string().into()).collect())
                .collect()
        };
        Lasso::new(positions(prefix), positions(cycle)).unwrap()
    }

    fn holds(l: &Lasso, text: &str) -> bool {
        l.satisfies(&text.parse().unwrap())
    }

    #[test]
    fn constants_atoms_and_boolean_operators() {
        let l = lasso(&["a"], &[""]);
        assert!(holds(&l, "true"));
        assert!(!holds(&l, "false"));
        assert!(holds(&l, "a"));
        assert!(!holds(&l, "b"));
        assert!(holds(&l, "!b"));
        assert!(!holds(&l, "a & b"));
        assert!(holds(&l, "a | b"));
        assert!(!holds(&l, "a -> b"));
        assert!(holds(&l, "b -> a"));
        assert!(!holds(&l, "a <-> b"));
        assert!(holds(&l, "b <-> c"));
    }

    #[test]
    fn next_crosses_into_and_around_the_cycle() {
        // Positions: a, then cycle (b, c) forever.
        let l = lasso(&["a"], &["b", "c"]);
        assert!(holds(&l, "X b"));
        assert!(holds(&l, "X X c"));
        // The last cycle position loops back to the first.
        assert!(holds(&l, "X X X b"));
        assert!(!holds(&l, "X X X a"));
        // A one-position cycle is its own successor.
        let l = lasso(&[], &["a"]);
        assert!(holds(&l, "X X X a"));
    }

    #[test]
    fn eventually_and_globally() {
        let l = lasso(&["", ""], &["", "a"]);
        assert!(holds(&l, "F a"));
        assert!(!holds(&l, "G a"));
        assert!(holds(&l, "G F a"));
        assert!(!holds(&l, "F G a"));
        assert!(holds(&l, "F G (a | X a)"));
        let l = lasso(&["a"], &[""]);
        assert!(holds(&l, "F a"));
        assert!(!holds(&l, "G F a"));
        assert!(holds(&l, "F G !a"));
    }

    #[test]
    fn until_with_its_witness_only_in_the_cycle() {
        // b first holds at the cycle's last position.
        let l = lasso(&["a", "a"], &["a", "a", "b"]);
        assert!(holds(&l, "a U b"));
        assert!(holds(&l, "G (a U b)"));
        // A gap before the witness breaks it.
        let l = lasso(&["a"], &["", "b"]);
        assert!(!holds(&l, "a U b"));
        // a forever but no b: strong until fails, weak until holds.
        let l = lasso(&["a"], &["a"]);
        assert!(!holds(&l, "a U b"));
        assert!(holds(&l, "a W b"));
        assert!(!holds(&l, "X a U b"));
    }

    #[test]
    fn weak_until() {
        assert!(holds(&lasso(&["a"], &["b"]), "a W b"));
        assert!(!holds(&lasso(&["a"], &["", "b"]), "a W b"));
        assert!(holds(&lasso(&[], &["a", "a"]), "a W b"));
        assert!(holds(&lasso(&["", ""], &["a"]), "F (a W b)"));
    }

    #[test]
    fn release_with_its_releaser_only_in_the_cycle() {
        // b holds throughout until a and b hold together, in the cycle.
        let l = lasso(&["b"], &["b", "ab", ""]);
        assert!(holds(&l, "a R b"));
        assert!(!holds(&l, "G (a R b)"));
        // b forever with no releaser: release holds.
        assert!(holds(&lasso(&["b"], &["b"]), "a R b"));
        // b stops before a releases it.
        assert!(!holds(&lasso(&["b"], &["", "ab"]), "a R b"));
        // b stops in the cycle with no a ever.
        assert!(!holds(&lasso(&["b", "b"], &["b", ""]), "a R b"));
    }

    #[test]
    fn long_lasso() {
        let mut prefix = vec![""; 200];
        prefix[0] = "a";
        let mut cycle = vec![""; 300];
        cycle[299] = "b";
        let l = lasso(&prefix, &cycle);
        assert!(holds(&l, "G F b"));
        assert!(!holds(&l, "F G !b"));
        assert!(holds(&l, "a & X G !a"));
        assert!(holds(&l, "!b U b"));
    }

    #[test]
    fn cache_returns_the_first_refuting_lasso() {
        let mut cache = LassoCache::new();
        let f: Formula = "G F a".parse().unwrap();
        assert!(cache.refutes(&f).is_none());
        let good = lasso(&[], &["a"]);
        let bad = lasso(&["a"], &[""]);
        let worse = lasso(&[], &[""]);
        cache.insert(good.clone());
        assert!(cache.refutes(&f).is_none());
        cache.insert(bad.clone());
        cache.insert(worse);
        assert_eq!(cache.refutes(&f), Some(&bad));
        assert_eq!(cache.refutes(&"F a".parse().unwrap()), cache.lassos.get(2));
        assert!(cache.refutes(&"true".parse().unwrap()).is_none());
    }
}
