//! Cache keys, ported from PEREDUR's `canonical.cpp` and `formula_key.cpp`.
//!
//! A cache that stores a verdict or a count, both invariant under a bijection
//! on atoms, may key on [`Formula::renamed`]. One that stores a formula or an
//! automaton must key on [`Formula::canonical`], which keeps atom names.

use std::collections::HashMap;
use std::sync::Arc;

use super::{BinaryOp, Formula, Node, UnaryOp};

impl Formula {
    /// A semantics-preserving normal form for use as a cache key. `&` and `|`
    /// chains are flattened, sorted, deduplicated and rebuilt left-leaning;
    /// `<->` operands are sorted; `!!φ` becomes `φ`.
    ///
    /// Not a simplifier: no constant is folded and no tautology recognised,
    /// both of which belong to [`Formula::simplify`]. It is meant for keys
    /// only, since re-associating a stored formula changes what
    /// `rewrite_post_order` walks, and with it a seeded run's draw order.
    pub fn canonical(&self) -> Formula {
        match self.node() {
            Node::True | Node::False | Node::Atom(_) => self.clone(),
            Node::Unary(op, child) => {
                let inner = child.canonical();
                if let (UnaryOp::Not, Node::Unary(UnaryOp::Not, grandchild)) = (op, inner.node()) {
                    return grandchild.clone();
                }
                Formula::unary(*op, inner)
            }
            Node::Binary(op, lhs, rhs) => {
                let lhs = lhs.canonical();
                let rhs = rhs.canonical();
                match op {
                    BinaryOp::And | BinaryOp::Or => {
                        let mut operands = Vec::new();
                        flatten(&lhs, *op, &mut operands);
                        flatten(&rhs, *op, &mut operands);
                        operands.sort();
                        operands.dedup();
                        let mut operands = operands.into_iter();
                        let first = operands.next().expect("a chain has operands");
                        operands.fold(first, |acc, next| Formula::binary(*op, acc, next))
                    }
                    // Commutative but not idempotent (`a <-> a` is true, not
                    // `a`), so ordered but neither flattened nor deduplicated.
                    BinaryOp::Iff if rhs < lhs => Formula::iff(rhs, lhs),
                    _ => Formula::binary(*op, lhs, rhs),
                }
            }
        }
    }

    /// [`Formula::canonical`] with atoms renamed to `p0`, `p1`, … in order of
    /// first appearance, so formulas differing only in atom names share a key.
    pub fn renamed(&self) -> Formula {
        let canonical = self.canonical();
        let mut names: HashMap<Arc<str>, Formula> = HashMap::new();
        for sub in canonical.subformulae() {
            if let Node::Atom(name) = sub.node() {
                let next = names.len();
                names
                    .entry(name.clone())
                    .or_insert_with(|| Formula::atom(format!("p{next}")));
            }
        }
        canonical.rewrite_post_order(|node| match node.node() {
            Node::Atom(name) => names.get(name).cloned(),
            _ => None,
        })
    }
}

fn flatten(formula: &Formula, op: BinaryOp, out: &mut Vec<Formula>) {
    match formula.node() {
        Node::Binary(inner, lhs, rhs) if *inner == op => {
            flatten(lhs, op, out);
            flatten(rhs, op, out);
        }
        _ => out.push(formula.clone()),
    }
}

#[cfg(test)]
mod tests {
    use crate::ltl::tests::{f, random_formulas};

    #[test]
    fn commutative_chains_collapse() {
        let key = f("a & (b & c)").canonical();
        assert_eq!(f("(c & b) & a").canonical(), key);
        assert_eq!(f("!!a & b & c & b").canonical(), key);
        assert_eq!(key.to_spot(), "a & b & c");
        assert_eq!(f("b | a | b").canonical(), f("a | b"));
    }

    #[test]
    fn iff_is_ordered_but_not_deduplicated() {
        assert_eq!(f("b <-> a").canonical(), f("a <-> b"));
        assert_eq!(f("a <-> a").canonical(), f("a <-> a"));
    }

    #[test]
    fn non_commutative_operators_keep_their_order() {
        assert_ne!(f("a U b").canonical(), f("b U a").canonical());
        assert_ne!(f("a -> b").canonical(), f("b -> a").canonical());
        assert_eq!(f("X X a").canonical(), f("X X a"));
        assert_eq!(f("G G a").canonical(), f("G G a"));
    }

    #[test]
    fn canonical_folds_no_constants() {
        assert_eq!(f("a & true").canonical(), f("true & a"));
        assert_eq!(f("G false").canonical(), f("G false"));
    }

    #[test]
    fn canonical_is_idempotent_and_reparses() {
        for formula in random_formulas(1000, 6) {
            let key = formula.canonical();
            assert_eq!(key.canonical(), key, "{formula}");
            assert_eq!(f(&key.to_spot()), key, "{formula}");
        }
    }

    #[test]
    fn renamed_forgets_atom_names_but_not_constants() {
        assert_eq!(f("G (req -> F grant)").renamed(), f("G (p0 -> F p1)"));
        assert_eq!(
            f("G (x -> F y)").renamed(),
            f("G (req -> F grant)").renamed()
        );
        assert_eq!(f("b & !a").renamed(), f("a & !b").renamed());
        assert_eq!(f("a U true").renamed(), f("p0 U true"));
        assert_ne!(f("a U b").renamed(), f("a U a").renamed());
    }
}
