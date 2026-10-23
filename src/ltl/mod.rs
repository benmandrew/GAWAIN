//! LTL formulas over the TLSF operator set, ported from PEREDUR's `Formula`.
//!
//! A [`Formula`] is an immutable, reference-counted tree. Cloning is a
//! refcount bump, and every node caches its structural hash and size, so a
//! formula is a cheap `HashMap` key and `n_subformulae` is O(1). Equality and
//! ordering are structural; use [`Formula::canonical`] or
//! [`Formula::renamed`] to collapse spellings of one formula into one key.

mod canonical;
mod parse;
mod print;
mod similarity;
mod simplify;

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

pub use parse::ParseError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UnaryOp {
    Not,
    Next,
    Eventually,
    Globally,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BinaryOp {
    And,
    Or,
    Implies,
    Iff,
    Until,
    WeakUntil,
    Release,
}

impl UnaryOp {
    pub fn is_temporal(self) -> bool {
        self != UnaryOp::Not
    }
}

impl BinaryOp {
    pub fn is_temporal(self) -> bool {
        matches!(
            self,
            BinaryOp::Until | BinaryOp::WeakUntil | BinaryOp::Release
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Node {
    True,
    False,
    Atom(Arc<str>),
    Unary(UnaryOp, Formula),
    Binary(BinaryOp, Formula, Formula),
}

struct Inner {
    node: Node,
    hash: u64,
    size: usize,
}

#[derive(Clone)]
pub struct Formula(Arc<Inner>);

impl Formula {
    pub fn from_node(node: Node) -> Self {
        // DefaultHasher::new() uses fixed keys, so the cached hash is stable
        // within a build; children contribute their own cached hashes.
        let mut hasher = DefaultHasher::new();
        node.hash(&mut hasher);
        let size = match &node {
            Node::True | Node::False | Node::Atom(_) => 1,
            Node::Unary(_, child) => 1 + child.n_subformulae(),
            Node::Binary(_, lhs, rhs) => 1 + lhs.n_subformulae() + rhs.n_subformulae(),
        };
        Formula(Arc::new(Inner {
            node,
            hash: hasher.finish(),
            size,
        }))
    }

    pub fn tt() -> Self {
        Self::from_node(Node::True)
    }

    pub fn ff() -> Self {
        Self::from_node(Node::False)
    }

    pub fn constant(value: bool) -> Self {
        if value {
            Self::tt()
        } else {
            Self::ff()
        }
    }

    pub fn atom(name: impl AsRef<str>) -> Self {
        Self::from_node(Node::Atom(Arc::from(name.as_ref())))
    }

    pub fn unary(op: UnaryOp, child: Formula) -> Self {
        Self::from_node(Node::Unary(op, child))
    }

    pub fn binary(op: BinaryOp, lhs: Formula, rhs: Formula) -> Self {
        Self::from_node(Node::Binary(op, lhs, rhs))
    }

    pub fn negated(child: Formula) -> Self {
        Self::unary(UnaryOp::Not, child)
    }

    pub fn next(child: Formula) -> Self {
        Self::unary(UnaryOp::Next, child)
    }

    pub fn eventually(child: Formula) -> Self {
        Self::unary(UnaryOp::Eventually, child)
    }

    pub fn globally(child: Formula) -> Self {
        Self::unary(UnaryOp::Globally, child)
    }

    pub fn and(lhs: Formula, rhs: Formula) -> Self {
        Self::binary(BinaryOp::And, lhs, rhs)
    }

    pub fn or(lhs: Formula, rhs: Formula) -> Self {
        Self::binary(BinaryOp::Or, lhs, rhs)
    }

    pub fn implies(lhs: Formula, rhs: Formula) -> Self {
        Self::binary(BinaryOp::Implies, lhs, rhs)
    }

    pub fn iff(lhs: Formula, rhs: Formula) -> Self {
        Self::binary(BinaryOp::Iff, lhs, rhs)
    }

    pub fn until(lhs: Formula, rhs: Formula) -> Self {
        Self::binary(BinaryOp::Until, lhs, rhs)
    }

    pub fn weak_until(lhs: Formula, rhs: Formula) -> Self {
        Self::binary(BinaryOp::WeakUntil, lhs, rhs)
    }

    pub fn release(lhs: Formula, rhs: Formula) -> Self {
        Self::binary(BinaryOp::Release, lhs, rhs)
    }

    pub fn node(&self) -> &Node {
        &self.0.node
    }

    pub fn is_true(&self) -> bool {
        matches!(self.node(), Node::True)
    }

    pub fn is_false(&self) -> bool {
        matches!(self.node(), Node::False)
    }

    pub fn atom_name(&self) -> Option<&str> {
        match self.node() {
            Node::Atom(name) => Some(name),
            _ => None,
        }
    }

    pub fn unary_child(&self) -> Option<&Formula> {
        match self.node() {
            Node::Unary(_, child) => Some(child),
            _ => None,
        }
    }

    pub fn binary_children(&self) -> Option<(&Formula, &Formula)> {
        match self.node() {
            Node::Binary(_, lhs, rhs) => Some((lhs, rhs)),
            _ => None,
        }
    }

    /// Number of nodes in the tree, counting repeated subtrees once per
    /// occurrence: `a & a` has three.
    pub fn n_subformulae(&self) -> usize {
        self.0.size
    }

    pub fn is_propositional(&self) -> bool {
        match self.node() {
            Node::True | Node::False | Node::Atom(_) => true,
            Node::Unary(op, child) => !op.is_temporal() && child.is_propositional(),
            Node::Binary(op, lhs, rhs) => {
                !op.is_temporal() && lhs.is_propositional() && rhs.is_propositional()
            }
        }
    }

    pub fn atoms(&self) -> BTreeSet<&str> {
        let mut out = BTreeSet::new();
        for sub in self.subformulae() {
            if let Some(name) = sub.atom_name() {
                out.insert(name);
            }
        }
        out
    }

    /// Every subtree in post-order (left before right, root last), with
    /// repeats.
    pub fn subformulae(&self) -> Vec<&Formula> {
        fn walk<'a>(formula: &'a Formula, out: &mut Vec<&'a Formula>) {
            match formula.node() {
                Node::True | Node::False | Node::Atom(_) => {}
                Node::Unary(_, child) => walk(child, out),
                Node::Binary(_, lhs, rhs) => {
                    walk(lhs, out);
                    walk(rhs, out);
                }
            }
            out.push(formula);
        }
        let mut out = Vec::with_capacity(self.n_subformulae());
        walk(self, &mut out);
        out
    }

    /// Rebuilds the formula bottom-up. Each node is offered to `rewrite` after
    /// its children have been rewritten, and is replaced if it returns
    /// `Some`. Children are visited left before right, so a callback that
    /// draws from a seeded RNG sees a fixed draw order.
    pub fn rewrite_post_order<F>(&self, mut rewrite: F) -> Formula
    where
        F: FnMut(&Formula) -> Option<Formula>,
    {
        self.rewrite_with(&mut rewrite)
    }

    fn rewrite_with<F>(&self, rewrite: &mut F) -> Formula
    where
        F: FnMut(&Formula) -> Option<Formula>,
    {
        let rebuilt = match self.node() {
            Node::True | Node::False | Node::Atom(_) => self.clone(),
            Node::Unary(op, child) => {
                let new_child = child.rewrite_with(rewrite);
                if new_child.ptr_eq(child) {
                    self.clone()
                } else {
                    Formula::unary(*op, new_child)
                }
            }
            Node::Binary(op, lhs, rhs) => {
                let new_lhs = lhs.rewrite_with(rewrite);
                let new_rhs = rhs.rewrite_with(rewrite);
                if new_lhs.ptr_eq(lhs) && new_rhs.ptr_eq(rhs) {
                    self.clone()
                } else {
                    Formula::binary(*op, new_lhs, new_rhs)
                }
            }
        };
        rewrite(&rebuilt).unwrap_or(rebuilt)
    }

    /// Replaces every `a W b` with `(a U b) | G a`, for back ends without W.
    pub fn expand_weak_until(&self) -> Formula {
        self.rewrite_post_order(|f| match f.node() {
            Node::Binary(BinaryOp::WeakUntil, lhs, rhs) => Some(Formula::or(
                Formula::until(lhs.clone(), rhs.clone()),
                Formula::globally(lhs.clone()),
            )),
            _ => None,
        })
    }

    pub fn ptr_eq(&self, other: &Formula) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl PartialEq for Formula {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other)
            || (self.0.hash == other.0.hash
                && self.0.size == other.0.size
                && self.0.node == other.0.node)
    }
}

impl Eq for Formula {}

impl Hash for Formula {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.0.hash);
    }
}

impl PartialOrd for Formula {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Formula {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.ptr_eq(other) {
            Ordering::Equal
        } else {
            self.0.node.cmp(&other.0.node)
        }
    }
}

impl fmt::Debug for Formula {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Formula({self})")
    }
}

impl std::ops::Not for Formula {
    type Output = Formula;
    fn not(self) -> Formula {
        Formula::negated(self)
    }
}

impl std::ops::BitAnd for Formula {
    type Output = Formula;
    fn bitand(self, rhs: Formula) -> Formula {
        Formula::and(self, rhs)
    }
}

impl std::ops::BitOr for Formula {
    type Output = Formula;
    fn bitor(self, rhs: Formula) -> Formula {
        Formula::or(self, rhs)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn f(text: &str) -> Formula {
        text.parse()
            .unwrap_or_else(|e| panic!("parse {text:?}: {e}"))
    }

    /// Deterministic random formulas over a few atoms, for round-trip tests.
    pub(crate) fn random_formulas(count: usize, seed: u64) -> Vec<Formula> {
        struct Rng(u64);
        impl Rng {
            fn below(&mut self, bound: u64) -> u64 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                self.0 % bound
            }
        }
        fn gen(rng: &mut Rng, depth: u32) -> Formula {
            let atoms = ["a", "b", "req_1", "Grant", "U", "x y"];
            if depth == 0 || rng.below(4) == 0 {
                return match rng.below(8) {
                    0 => Formula::tt(),
                    1 => Formula::ff(),
                    n => Formula::atom(atoms[(n - 2) as usize]),
                };
            }
            if rng.below(3) == 0 {
                let op = [
                    UnaryOp::Not,
                    UnaryOp::Next,
                    UnaryOp::Eventually,
                    UnaryOp::Globally,
                ][rng.below(4) as usize];
                Formula::unary(op, gen(rng, depth - 1))
            } else {
                let op = [
                    BinaryOp::And,
                    BinaryOp::Or,
                    BinaryOp::Implies,
                    BinaryOp::Iff,
                    BinaryOp::Until,
                    BinaryOp::WeakUntil,
                    BinaryOp::Release,
                ][rng.below(7) as usize];
                let lhs = gen(rng, depth - 1);
                let rhs = gen(rng, depth - 1);
                Formula::binary(op, lhs, rhs)
            }
        }
        let mut rng = Rng(seed | 1);
        (0..count).map(|_| gen(&mut rng, 5)).collect()
    }

    #[test]
    fn equality_is_structural_not_by_pointer() {
        assert_eq!(
            f("a & G b"),
            Formula::and(Formula::atom("a"), Formula::globally(Formula::atom("b")))
        );
        assert_ne!(f("a & b"), f("b & a"));
    }

    #[test]
    fn hash_agrees_with_equality() {
        use std::collections::HashSet;
        let set: HashSet<Formula> = [f("a U b"), f("a U b"), f("b U a")].into_iter().collect();
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn ordering_is_total_and_consistent() {
        let mut items = random_formulas(200, 7);
        items.sort();
        for pair in items.windows(2) {
            assert_ne!(pair[0].cmp(&pair[1]), Ordering::Greater);
            assert_eq!(pair[0] == pair[1], pair[0].cmp(&pair[1]) == Ordering::Equal);
        }
    }

    #[test]
    fn n_subformulae_counts_repeats() {
        assert_eq!(f("a").n_subformulae(), 1);
        assert_eq!(f("a & a").n_subformulae(), 3);
        assert_eq!(f("G (a -> F b)").n_subformulae(), 5);
    }

    #[test]
    fn subformulae_are_post_order() {
        let formula = f("!a U b");
        let rendered: Vec<String> = formula
            .subformulae()
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(rendered, ["a", "!a", "b", "!a U b"]);
    }

    #[test]
    fn is_propositional_rejects_temporal_operators() {
        assert!(f("a & !(b -> c) <-> true").is_propositional());
        assert!(!f("a & X b").is_propositional());
        assert!(!f("a W b").is_propositional());
    }

    #[test]
    fn atoms_collects_names_but_not_constants() {
        let formula = f("G (req -> F grant) & true");
        assert_eq!(
            formula.atoms().into_iter().collect::<Vec<_>>(),
            ["grant", "req"]
        );
    }

    #[test]
    fn rewrite_post_order_visits_children_first() {
        let mut seen = Vec::new();
        let _ = f("(a & b) | c").rewrite_post_order(|node| {
            seen.push(node.to_string());
            None
        });
        assert_eq!(seen, ["a", "b", "a & b", "c", "a & b | c"]);
    }

    #[test]
    fn rewrite_post_order_replaces_and_shares_untouched_subtrees() {
        let formula = f("G a & F b");
        let rewritten = formula.rewrite_post_order(|node| match node.atom_name() {
            Some("b") => Some(Formula::atom("c")),
            _ => None,
        });
        assert_eq!(rewritten, f("G a & F c"));
        let (old_lhs, _) = formula.binary_children().unwrap();
        let (new_lhs, _) = rewritten.binary_children().unwrap();
        assert!(old_lhs.ptr_eq(new_lhs));

        let unchanged = formula.rewrite_post_order(|_| None);
        assert!(unchanged.ptr_eq(&formula));
    }

    #[test]
    fn rewrite_sees_rewritten_children() {
        let rewritten = f("!!a").rewrite_post_order(|node| match node.atom_name() {
            Some("a") => Some(Formula::tt()),
            _ => None,
        });
        assert_eq!(rewritten, f("!!true"));
    }

    #[test]
    fn expand_weak_until_removes_every_w() {
        assert_eq!(f("a W b").expand_weak_until(), f("(a U b) | G a"));
        assert_eq!(
            f("(a W b) W c").expand_weak_until(),
            f("(((a U b) | G a) U c) | G ((a U b) | G a)")
        );
    }

    #[test]
    fn operator_overloads_build_nodes() {
        let a = Formula::atom("a");
        let b = Formula::atom("b");
        assert_eq!(!a.clone() & b.clone() | a, f("!a & b | a"));
    }
}
