//! Syntactic simplification, ported from PEREDUR's `simplify.cpp`.
//!
//! One post-order pass of local folds: boolean identities, double negation,
//! constants through every temporal operator, `G G φ` and `F F φ`, and the
//! self-joins `φ U φ`, `φ W φ`, `φ R φ`. Each fold is an equivalence.

use super::{BinaryOp, Formula, Node, UnaryOp};

impl Formula {
    /// An equivalent formula with the folds in the module header applied,
    /// bottom-up in one pass. The result is never larger than the input.
    pub fn simplify(&self) -> Formula {
        self.rewrite_post_order(simplify_node)
    }
}

fn is_negation_of(lhs: &Formula, rhs: &Formula) -> bool {
    let negates =
        |a: &Formula, b: &Formula| matches!(a.node(), Node::Unary(UnaryOp::Not, c) if c == b);
    negates(lhs, rhs) || negates(rhs, lhs)
}

// Folds as it negates, so a fold never leaves behind a fresh `!!` or `!true`
// that this single pass would not revisit.
fn negate(formula: &Formula) -> Formula {
    match formula.node() {
        Node::Unary(UnaryOp::Not, child) => child.clone(),
        Node::True => Formula::ff(),
        Node::False => Formula::tt(),
        _ => Formula::unary(UnaryOp::Not, formula.clone()),
    }
}

fn simplify_node(node: &Formula) -> Option<Formula> {
    match node.node() {
        Node::True | Node::False | Node::Atom(_) => None,
        Node::Unary(op, child) => simplify_unary(*op, child),
        Node::Binary(op, lhs, rhs) => simplify_binary(*op, lhs, rhs),
    }
}

fn simplify_unary(op: UnaryOp, child: &Formula) -> Option<Formula> {
    match op {
        UnaryOp::Not => match child.node() {
            Node::Unary(UnaryOp::Not, grandchild) => Some(grandchild.clone()),
            Node::True => Some(Formula::ff()),
            Node::False => Some(Formula::tt()),
            _ => None,
        },
        // A constant holds at every timepoint or at none.
        _ if child.is_true() || child.is_false() => Some(child.clone()),
        // X X φ is not X φ, so only F and G are idempotent.
        UnaryOp::Next => None,
        UnaryOp::Eventually | UnaryOp::Globally => match child.node() {
            Node::Unary(inner, _) if *inner == op => Some(child.clone()),
            _ => None,
        },
    }
}

fn simplify_binary(op: BinaryOp, lhs: &Formula, rhs: &Formula) -> Option<Formula> {
    match op {
        BinaryOp::And => {
            if lhs == rhs || rhs.is_true() {
                Some(lhs.clone())
            } else if lhs.is_true() {
                Some(rhs.clone())
            } else if lhs.is_false() || rhs.is_false() || is_negation_of(lhs, rhs) {
                Some(Formula::ff())
            } else {
                None
            }
        }
        BinaryOp::Or => {
            if lhs == rhs || rhs.is_false() {
                Some(lhs.clone())
            } else if lhs.is_true() || rhs.is_true() || is_negation_of(lhs, rhs) {
                Some(Formula::tt())
            } else if lhs.is_false() {
                Some(rhs.clone())
            } else {
                None
            }
        }
        BinaryOp::Implies => {
            if lhs == rhs || rhs.is_true() || lhs.is_false() {
                Some(Formula::tt())
            } else if lhs.is_true() {
                Some(rhs.clone())
            } else if rhs.is_false() {
                Some(negate(lhs))
            } else {
                None
            }
        }
        BinaryOp::Iff => {
            if lhs == rhs {
                Some(Formula::tt())
            } else if lhs.is_true() {
                Some(rhs.clone())
            } else if rhs.is_true() {
                Some(lhs.clone())
            } else if lhs.is_false() {
                Some(negate(rhs))
            } else if rhs.is_false() {
                Some(negate(lhs))
            } else if is_negation_of(lhs, rhs) {
                Some(Formula::ff())
            } else {
                None
            }
        }
        // Each constant case reads off the fixpoint expansion. W and R are the
        // two that do not simply annihilate: φ W false = G φ and
        // false R ψ = G ψ.
        BinaryOp::Until => {
            if rhs.is_true() || rhs.is_false() || lhs.is_false() {
                Some(rhs.clone())
            } else if lhs.is_true() {
                Some(Formula::unary(UnaryOp::Eventually, rhs.clone()))
            } else {
                self_join(lhs, rhs)
            }
        }
        BinaryOp::WeakUntil => {
            if rhs.is_true() || lhs.is_false() {
                Some(rhs.clone())
            } else if lhs.is_true() {
                Some(lhs.clone())
            } else if rhs.is_false() {
                Some(Formula::unary(UnaryOp::Globally, lhs.clone()))
            } else {
                self_join(lhs, rhs)
            }
        }
        BinaryOp::Release => {
            if rhs.is_true() || rhs.is_false() || lhs.is_true() {
                Some(rhs.clone())
            } else if lhs.is_false() {
                Some(Formula::unary(UnaryOp::Globally, rhs.clone()))
            } else {
                self_join(lhs, rhs)
            }
        }
    }
}

fn self_join(lhs: &Formula, rhs: &Formula) -> Option<Formula> {
    (lhs == rhs).then(|| lhs.clone())
}

#[cfg(test)]
mod tests {
    use crate::ltl::tests::{f, random_formulas};

    fn check(cases: &[(&str, &str)]) {
        for (input, expected) in cases {
            assert_eq!(f(input).simplify(), f(expected), "{input}");
        }
    }

    #[test]
    fn negation_folds() {
        check(&[
            ("!!a", "a"),
            ("!true", "false"),
            ("!false", "true"),
            ("!!!a", "!a"),
        ]);
    }

    #[test]
    fn conjunction_folds() {
        check(&[
            ("a & a", "a"),
            ("true & a", "a"),
            ("a & true", "a"),
            ("a & false", "false"),
            ("false & a", "false"),
            ("a & !a", "false"),
            ("!a & a", "false"),
            ("a & b", "a & b"),
        ]);
    }

    #[test]
    fn disjunction_folds() {
        check(&[
            ("a | a", "a"),
            ("a | true", "true"),
            ("true | a", "true"),
            ("a | !a", "true"),
            ("a | false", "a"),
            ("false | a", "a"),
            ("a | b", "a | b"),
        ]);
    }

    #[test]
    fn implication_folds() {
        check(&[
            ("a -> a", "true"),
            ("a -> true", "true"),
            ("true -> a", "a"),
            ("false -> a", "true"),
            ("a -> false", "!a"),
            ("!a -> false", "a"),
            ("a -> b", "a -> b"),
        ]);
    }

    #[test]
    fn equivalence_folds() {
        check(&[
            ("a <-> a", "true"),
            ("true <-> a", "a"),
            ("a <-> true", "a"),
            ("false <-> a", "!a"),
            ("a <-> false", "!a"),
            ("a <-> !a", "false"),
            ("a <-> b", "a <-> b"),
        ]);
    }

    #[test]
    fn temporal_unary_folds() {
        check(&[
            ("X true", "true"),
            ("F false", "false"),
            ("G true", "true"),
            ("G G a", "G a"),
            ("F F a", "F a"),
            ("G G G a", "G a"),
            ("X X a", "X X a"),
            ("G F a", "G F a"),
        ]);
    }

    #[test]
    fn temporal_binary_folds() {
        check(&[
            ("a U true", "true"),
            ("a U false", "false"),
            ("false U a", "a"),
            ("true U a", "F a"),
            ("a W true", "true"),
            ("true W a", "true"),
            ("false W a", "a"),
            ("a W false", "G a"),
            ("a R true", "true"),
            ("a R false", "false"),
            ("true R a", "a"),
            ("false R a", "G a"),
            ("a U a", "a"),
            ("a W a", "a"),
            ("a R a", "a"),
            ("a U b", "a U b"),
        ]);
    }

    #[test]
    fn folds_cascade_bottom_up() {
        check(&[
            ("G (a & !a) | b", "b"),
            ("(F F a) U (F a)", "F a"),
            ("X (true U false) -> c", "true"),
            ("!(a -> false)", "a"),
        ]);
    }

    #[test]
    fn simplify_never_grows_a_formula() {
        for formula in random_formulas(2000, 3) {
            let simplified = formula.simplify();
            assert!(
                simplified.n_subformulae() <= formula.n_subformulae(),
                "{formula}"
            );
        }
    }
}
