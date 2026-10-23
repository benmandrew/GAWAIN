//! LTL formulas over the TLSF operator set, ported from PEREDUR's `Formula`.
//!
//! A [`Formula`] is an immutable tree of shared nodes. Cloning one is a
//! refcount bump, and every node caches its structural hash and size, so a
//! formula is a cheap `HashMap` key. Equality and ordering are structural;
//! [`Formula::canonical`] collapses spellings of one formula into one key.
//! See `docs/dev/ltl.md` for the design reasons.

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

/// A unary operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UnaryOp {
    /// Negation, `!φ`.
    Not,
    /// Next, `X φ`.
    Next,
    /// Eventually, `F φ`.
    Eventually,
    /// Globally, `G φ`.
    Globally,
}

/// A binary operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BinaryOp {
    /// Conjunction, `φ & ψ`.
    And,
    /// Disjunction, `φ | ψ`.
    Or,
    /// Implication, `φ -> ψ`.
    Implies,
    /// Equivalence, `φ <-> ψ`.
    Iff,
    /// Strong until, `φ U ψ`.
    Until,
    /// Weak until, `φ W ψ`.
    WeakUntil,
    /// Release, `φ R ψ`.
    Release,
}

/// The root of a formula, with its children.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Node {
    /// The constant true.
    True,
    /// The constant false.
    False,
    /// An atomic proposition.
    Atom(Arc<str>),
    /// A unary operator applied to one formula.
    Unary(UnaryOp, Formula),
    /// A binary operator applied to two formulas.
    Binary(BinaryOp, Formula, Formula),
}

struct Inner {
    node: Node,
    hash: u64,
    size: usize,
}

/// An LTL formula. Parse one from Spot syntax with [`str::parse`], and print
/// it with `Display` (Spot), [`Formula::to_spin`] or [`Formula::to_nuxmv`].
#[derive(Clone)]
pub struct Formula(Arc<Inner>);

impl Formula {
    fn from_node(node: Node) -> Self {
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

    /// The constant true.
    pub fn tt() -> Self {
        Self::from_node(Node::True)
    }

    /// The constant false.
    pub fn ff() -> Self {
        Self::from_node(Node::False)
    }

    /// An atomic proposition. Any name is allowed; the printers quote or pass
    /// it through as each back end requires.
    pub fn atom(name: impl AsRef<str>) -> Self {
        Self::from_node(Node::Atom(Arc::from(name.as_ref())))
    }

    /// `op child`.
    pub fn unary(op: UnaryOp, child: Formula) -> Self {
        Self::from_node(Node::Unary(op, child))
    }

    /// `lhs op rhs`.
    pub fn binary(op: BinaryOp, lhs: Formula, rhs: Formula) -> Self {
        Self::from_node(Node::Binary(op, lhs, rhs))
    }

    /// The root node.
    pub fn node(&self) -> &Node {
        &self.0.node
    }

    fn is_true(&self) -> bool {
        matches!(self.node(), Node::True)
    }

    fn is_false(&self) -> bool {
        matches!(self.node(), Node::False)
    }

    /// Number of nodes in the tree, counting a repeated subtree once per
    /// occurrence: `a & a` has three. O(1).
    pub fn n_subformulae(&self) -> usize {
        self.0.size
    }

    /// The atoms the formula mentions, in sorted order.
    pub fn atoms(&self) -> BTreeSet<Arc<str>> {
        self.subformulae()
            .into_iter()
            .filter_map(|sub| match sub.node() {
                Node::Atom(name) => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    /// Every subtree in post-order (left before right, root last), with
    /// repeats.
    fn subformulae(&self) -> Vec<&Formula> {
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
    /// draws from a seeded RNG sees a fixed draw order. Untouched subtrees are
    /// shared with the input, not copied.
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

    fn ptr_eq(&self, other: &Formula) -> bool {
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
        let built = Formula::binary(
            BinaryOp::And,
            Formula::atom("a"),
            Formula::unary(UnaryOp::Globally, Formula::atom("b")),
        );
        assert_eq!(f("a & G b"), built);
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
    fn atoms_are_sorted_and_distinct() {
        let atoms: Vec<String> = f("G (b -> F a) & X b & true")
            .atoms()
            .iter()
            .map(|a| a.to_string())
            .collect();
        assert_eq!(atoms, ["a", "b"]);
        assert!(f("G true").atoms().is_empty());
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
    fn rewrite_post_order_visits_children_first() {
        let mut seen = Vec::new();
        let _ = f("(a & b) | c").rewrite_post_order(|node| {
            seen.push(node.to_string());
            None
        });
        assert_eq!(seen, ["a", "b", "a & b", "c", "a & b | c"]);
    }

    fn replace_atom(
        name: &'static str,
        with: &'static str,
    ) -> impl FnMut(&Formula) -> Option<Formula> {
        move |node| match node.node() {
            Node::Atom(atom) if &**atom == name => Some(f(with)),
            _ => None,
        }
    }

    #[test]
    fn rewrite_post_order_replaces_and_shares_untouched_subtrees() {
        let formula = f("G a & F b");
        let rewritten = formula.rewrite_post_order(replace_atom("b", "c"));
        assert_eq!(rewritten, f("G a & F c"));
        let (Node::Binary(_, old_lhs, _), Node::Binary(_, new_lhs, _)) =
            (formula.node(), rewritten.node())
        else {
            panic!("expected conjunctions");
        };
        assert!(old_lhs.ptr_eq(new_lhs));

        let unchanged = formula.rewrite_post_order(|_| None);
        assert!(unchanged.ptr_eq(&formula));
    }

    #[test]
    fn rewrite_sees_rewritten_children() {
        let rewritten = f("!!a").rewrite_post_order(replace_atom("a", "true"));
        assert_eq!(rewritten, f("!!true"));
    }
}
