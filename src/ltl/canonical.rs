//! Canonical cache keys, ported from PEREDUR's `canonical.cpp`.
//!
//! PEREDUR also keys some caches on a form with atoms renamed. That is unsound
//! here, where atoms name model variables, so only [`Formula::canonical`] is
//! ported.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::{BinaryOp, Formula, Node, UnaryOp};

/// Canonical forms already built, keyed on subtree identity, for
/// [`Formula::canonical_shared`]. Each key holds its subtree alive, so no
/// other subtree can take a freed address while the memo lives.
#[derive(Default)]
pub struct CanonicalMemo(HashMap<ById, Formula>);

/// A subtree compared and hashed by its address, not its structure.
struct ById(Formula);

impl PartialEq for ById {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}

impl Eq for ById {}

impl Hash for ById {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0 .0).hash(state);
    }
}

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
        self.canonical_with(&mut |child| child.canonical())
    }

    /// [`Formula::canonical`], equal to it on every input, except that a
    /// subtree already put through `memo` (the same node, not merely an
    /// equal one) gets the same canonical node back. Formulas that share
    /// most of their tree, such as the outcomes of one rewrite at each of
    /// its sites, then share most of their canonical forms too, where
    /// [`Formula::canonical`] builds each in full. A canonical form is its
    /// own canonical form, so each result is also stored as its own key: a
    /// formula built from earlier results, such as `φ & φ'` from the keys
    /// of φ and φ', rebuilds only the nodes above them.
    pub fn canonical_shared(&self, memo: &mut CanonicalMemo) -> Formula {
        if let Node::True | Node::False | Node::Atom(_) = self.node() {
            return self.clone();
        }
        let id = ById(self.clone());
        if let Some(key) = memo.0.get(&id) {
            return key.clone();
        }
        let key = self.canonical_with(&mut |child| child.canonical_shared(memo));
        memo.0.insert(id, key.clone());
        memo.0.insert(ById(key.clone()), key.clone());
        key
    }

    /// One step of the canonical form, with `recurse` giving the children's.
    fn canonical_with(&self, recurse: &mut impl FnMut(&Formula) -> Formula) -> Formula {
        match self.node() {
            Node::True | Node::False | Node::Atom(_) => self.clone(),
            Node::Unary(op, child) => {
                let inner = recurse(child);
                if let (UnaryOp::Not, Node::Unary(UnaryOp::Not, grandchild)) = (op, inner.node()) {
                    return grandchild.clone();
                }
                Formula::unary(*op, inner)
            }
            // The whole chain at once: its operands are put through `recurse`
            // and the chain is rebuilt once. Recursing into each link instead
            // builds a sorted copy of every prefix, O(n²) nodes for n
            // operands, which a memo would keep. An operand's canonical form
            // can itself be a chain of `op`, as `!!(a & b)` is, so it is
            // flattened again. Either way the operands are the same, so the
            // result is too.
            Node::Binary(op @ (BinaryOp::And | BinaryOp::Or), _, _) => {
                let mut links = Vec::new();
                flatten(self, *op, &mut links);
                let mut operands = Vec::new();
                for link in &links {
                    flatten(&recurse(link), *op, &mut operands);
                }
                operands.sort();
                operands.dedup();
                let mut operands = operands.into_iter();
                let first = operands.next().expect("a chain has operands");
                operands.fold(first, |acc, next| Formula::binary(*op, acc, next))
            }
            Node::Binary(op, lhs, rhs) => {
                let lhs = recurse(lhs);
                let rhs = recurse(rhs);
                match op {
                    // Commutative but not idempotent (`a <-> a` is true, not
                    // `a`), so ordered but neither flattened nor deduplicated.
                    BinaryOp::Iff if rhs < lhs => Formula::binary(BinaryOp::Iff, rhs, lhs),
                    _ => Formula::binary(*op, lhs, rhs),
                }
            }
        }
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
    use super::{flatten, CanonicalMemo};
    use crate::ltl::tests::{f, random_formulas};
    use crate::ltl::{BinaryOp, Formula, Node, UnaryOp};

    /// The canonical form as first ported, which recursed into each link of
    /// a chain. The whole-chain form must give exactly the same result.
    fn per_link(formula: &Formula) -> Formula {
        match formula.node() {
            Node::True | Node::False | Node::Atom(_) => formula.clone(),
            Node::Unary(op, child) => {
                let inner = per_link(child);
                if let (UnaryOp::Not, Node::Unary(UnaryOp::Not, grandchild)) = (op, inner.node()) {
                    return grandchild.clone();
                }
                Formula::unary(*op, inner)
            }
            Node::Binary(op, lhs, rhs) => {
                let (lhs, rhs) = (per_link(lhs), per_link(rhs));
                match op {
                    BinaryOp::And | BinaryOp::Or => {
                        let mut operands = Vec::new();
                        flatten(&lhs, *op, &mut operands);
                        flatten(&rhs, *op, &mut operands);
                        operands.sort();
                        operands.dedup();
                        let mut operands = operands.into_iter();
                        let first = operands.next().unwrap();
                        operands.fold(first, |acc, next| Formula::binary(*op, acc, next))
                    }
                    BinaryOp::Iff if rhs < lhs => Formula::binary(BinaryOp::Iff, rhs, lhs),
                    _ => Formula::binary(*op, lhs, rhs),
                }
            }
        }
    }

    /// Asserts that every form of the canonical form agrees on `g`, and
    /// prints the same.
    fn agree(g: &Formula, memo: &mut CanonicalMemo) {
        let old = per_link(g);
        assert_eq!(g.canonical(), old, "{g}");
        assert_eq!(g.canonical().to_string(), old.to_string(), "{g}");
        let shared = g.canonical_shared(memo);
        assert_eq!(shared, old, "{g}");
        assert_eq!(shared.to_string(), old.to_string(), "{g}");
    }

    #[test]
    fn whole_chains_equal_the_per_link_form() {
        let mut memo = CanonicalMemo::default();
        let formulas = random_formulas(1000, 7);
        for formula in &formulas {
            agree(formula, &mut memo);
        }
        let not_not = |g: Formula| Formula::unary(UnaryOp::Not, Formula::unary(UnaryOp::Not, g));
        for op in [BinaryOp::And, BinaryOp::Or] {
            // Sixty operands with repeats, leaning left, right and both ways,
            // with same-op chains nested under `!!` and under the other op.
            let operands: Vec<Formula> = formulas[..60]
                .iter()
                .chain(&formulas[..10])
                .cloned()
                .collect();
            let left = operands
                .iter()
                .cloned()
                .reduce(|acc, g| Formula::binary(op, acc, g))
                .unwrap();
            let right = operands
                .iter()
                .cloned()
                .rev()
                .reduce(|acc, g| Formula::binary(op, g, acc))
                .unwrap();
            let other = match op {
                BinaryOp::And => BinaryOp::Or,
                _ => BinaryOp::And,
            };
            let nested = Formula::binary(
                op,
                Formula::binary(op, not_not(left.clone()), formulas[3].clone()),
                Formula::binary(
                    op,
                    not_not(not_not(right.clone())),
                    Formula::binary(other, left.clone(), not_not(right.clone())),
                ),
            );
            let balanced = Formula::binary(op, left.clone(), right.clone());
            for g in [&left, &right, &nested, &balanced, &not_not(nested.clone())] {
                agree(g, &mut memo);
            }
        }
    }

    #[test]
    fn commutative_chains_collapse() {
        let key = f("a & (b & c)").canonical();
        assert_eq!(f("(c & b) & a").canonical(), key);
        assert_eq!(f("!!a & b & c & b").canonical(), key);
        assert_eq!(key.to_string(), "a & b & c");
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
            assert_eq!(f(&key.to_string()), key, "{formula}");
        }
    }

    #[test]
    fn shared_equals_canonical_on_shared_subtrees_and_chains() {
        let mut memo = CanonicalMemo::default();
        let formulas = random_formulas(1000, 6);
        for (i, formula) in formulas.iter().enumerate() {
            // Built from earlier formulas, so the memo already holds some of
            // the subtrees, and the chains nest `&` in `|` and in `&`.
            let other = &formulas[i / 2];
            let chain = Formula::binary(
                BinaryOp::And,
                Formula::binary(BinaryOp::Or, formula.clone(), other.clone()),
                Formula::binary(BinaryOp::And, other.clone(), formula.clone()),
            );
            let not_not = Formula::unary(UnaryOp::Not, Formula::unary(UnaryOp::Not, chain.clone()));
            for g in [formula, &chain, &not_not] {
                let shared = g.canonical_shared(&mut memo);
                assert_eq!(shared, g.canonical(), "{g}");
                assert_eq!(shared.to_string(), g.canonical().to_string(), "{g}");
                // Built from results, which the memo holds as their own keys.
                let built =
                    Formula::binary(BinaryOp::Or, shared, other.canonical_shared(&mut memo));
                assert_eq!(
                    built.canonical_shared(&mut memo),
                    built.canonical(),
                    "{built}"
                );
            }
        }
        for text in [
            "a & (b & c)",
            "!!a & b & c & b",
            "b <-> a",
            "(c | b) & (a | b) & X !!d",
        ] {
            let g = f(text);
            assert_eq!(g.canonical_shared(&mut memo), g.canonical(), "{g}");
        }
    }

    #[test]
    fn outcomes_sharing_a_subtree_share_its_canonical_form() {
        let common = f("G (c | (b | a)) U (d & !!e)");
        let one = Formula::binary(BinaryOp::And, common.clone(), f("x"));
        let two = Formula::binary(BinaryOp::Or, f("y"), common.clone());
        let mut memo = CanonicalMemo::default();
        // An atom sorts before a compound node, so `common` ends up on the
        // right of both chains.
        let rhs = |g: Formula| match g.node() {
            Node::Binary(_, _, rhs) => rhs.clone(),
            _ => unreachable!("{g} is a chain"),
        };
        let a = rhs(one.canonical_shared(&mut memo));
        let b = rhs(two.canonical_shared(&mut memo));
        assert_eq!(a, common.canonical());
        assert!(a.ptr_eq(&b));
        assert!(!rhs(one.canonical()).ptr_eq(&rhs(two.canonical())));
    }
}
