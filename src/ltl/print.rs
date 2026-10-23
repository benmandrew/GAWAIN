//! Printers for Spot, SPIN and nuXmv.
//!
//! The Spot printer is `Display`. It omits parentheses that Spot's precedence
//! makes redundant, and quotes any atom Spot would misread. The SPIN and
//! nuXmv printers parenthesise every non-atomic operand of a binary operator,
//! since their precedence tables differ from Spot's and from each other's.
//! They print atom names verbatim, so a name must already be a valid
//! identifier in the target language.

use std::fmt::{self, Write};

use super::{BinaryOp, Formula, Node, UnaryOp};

impl fmt::Display for Formula {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        spot(self, f)
    }
}

impl Formula {
    /// SPIN `ltl` syntax. SPIN has W, but it is expanded as for
    /// [`Formula::to_nuxmv`] so both back ends receive the same operator set.
    pub fn to_spin(&self) -> String {
        let mut out = String::new();
        explicit(&expand_weak_until(self), &SPIN, &mut out);
        out
    }

    /// nuXmv `LTLSPEC` syntax. nuXmv has no W, so `a W b` is printed as
    /// `(a U b) | (G a)`.
    pub fn to_nuxmv(&self) -> String {
        let mut out = String::new();
        explicit(&expand_weak_until(self), &NUXMV, &mut out);
        out
    }
}

fn expand_weak_until(formula: &Formula) -> Formula {
    formula.rewrite_post_order(|node| match node.node() {
        Node::Binary(BinaryOp::WeakUntil, lhs, rhs) => Some(Formula::binary(
            BinaryOp::Or,
            Formula::binary(BinaryOp::Until, lhs.clone(), rhs.clone()),
            Formula::unary(UnaryOp::Globally, lhs.clone()),
        )),
        _ => None,
    })
}

fn precedence(formula: &Formula) -> u8 {
    match formula.node() {
        Node::Binary(op, _, _) => match op {
            BinaryOp::Implies | BinaryOp::Iff => 1,
            BinaryOp::Or => 2,
            BinaryOp::And => 3,
            BinaryOp::Until | BinaryOp::WeakUntil | BinaryOp::Release => 4,
        },
        _ => 5,
    }
}

fn binary_op(formula: &Formula) -> Option<BinaryOp> {
    match formula.node() {
        Node::Binary(op, _, _) => Some(*op),
        _ => None,
    }
}

fn is_plain_spot_atom(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && !matches!(first, 'F' | 'G' | 'X')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !matches!(
            name,
            "true" | "false" | "TRUE" | "FALSE" | "U" | "W" | "R" | "V" | "M" | "xor"
        )
}

fn spot(formula: &Formula, out: &mut impl Write) -> fmt::Result {
    match formula.node() {
        Node::True => out.write_str("true"),
        Node::False => out.write_str("false"),
        Node::Atom(name) if is_plain_spot_atom(name) => out.write_str(name),
        Node::Atom(name) => write!(out, "\"{name}\""),
        Node::Unary(op, child) => {
            out.write_str(match op {
                UnaryOp::Not => "!",
                UnaryOp::Next => "X",
                UnaryOp::Eventually => "F",
                UnaryOp::Globally => "G",
            })?;
            if binary_op(child).is_some() {
                out.write_char('(')?;
                spot(child, out)?;
                out.write_char(')')
            } else {
                if *op != UnaryOp::Not {
                    out.write_char(' ')?;
                }
                spot(child, out)
            }
        }
        Node::Binary(op, lhs, rhs) => {
            let own = precedence(formula);
            let left_assoc = matches!(op, BinaryOp::And | BinaryOp::Or);
            let lhs_parens = if left_assoc {
                precedence(lhs) < own
            } else {
                precedence(lhs) <= own
            };
            // Right-associative levels hold two or three operators; a mixed
            // chain such as `a U b W c` parses, but reads badly.
            let rhs_parens = if left_assoc {
                precedence(rhs) <= own
            } else {
                precedence(rhs) < own || (precedence(rhs) == own && binary_op(rhs) != Some(*op))
            };
            spot_operand(lhs, lhs_parens, out)?;
            out.write_str(match op {
                BinaryOp::And => " & ",
                BinaryOp::Or => " | ",
                BinaryOp::Implies => " -> ",
                BinaryOp::Iff => " <-> ",
                BinaryOp::Until => " U ",
                BinaryOp::WeakUntil => " W ",
                BinaryOp::Release => " R ",
            })?;
            spot_operand(rhs, rhs_parens, out)
        }
    }
}

fn spot_operand(formula: &Formula, parens: bool, out: &mut impl Write) -> fmt::Result {
    if parens {
        out.write_char('(')?;
        spot(formula, out)?;
        out.write_char(')')
    } else {
        spot(formula, out)
    }
}

struct Syntax {
    tt: &'static str,
    ff: &'static str,
    unary: fn(UnaryOp) -> &'static str,
    binary: fn(BinaryOp) -> &'static str,
}

const SPIN: Syntax = Syntax {
    tt: "true",
    ff: "false",
    unary: |op| match op {
        UnaryOp::Not => "!",
        UnaryOp::Next => "X",
        UnaryOp::Eventually => "<>",
        UnaryOp::Globally => "[]",
    },
    binary: |op| match op {
        BinaryOp::And => "&&",
        BinaryOp::Or => "||",
        BinaryOp::Implies => "->",
        BinaryOp::Iff => "<->",
        BinaryOp::Until => "U",
        BinaryOp::Release => "V",
        BinaryOp::WeakUntil => unreachable!("W is expanded before printing"),
    },
};

const NUXMV: Syntax = Syntax {
    tt: "TRUE",
    ff: "FALSE",
    unary: |op| match op {
        UnaryOp::Not => "!",
        UnaryOp::Next => "X",
        UnaryOp::Eventually => "F",
        UnaryOp::Globally => "G",
    },
    binary: |op| match op {
        BinaryOp::And => "&",
        BinaryOp::Or => "|",
        BinaryOp::Implies => "->",
        BinaryOp::Iff => "<->",
        BinaryOp::Until => "U",
        BinaryOp::Release => "V",
        BinaryOp::WeakUntil => unreachable!("W is expanded before printing"),
    },
};

fn explicit(formula: &Formula, syntax: &Syntax, out: &mut String) {
    match formula.node() {
        Node::True => out.push_str(syntax.tt),
        Node::False => out.push_str(syntax.ff),
        Node::Atom(name) => out.push_str(name),
        Node::Unary(op, child) => {
            let symbol = (syntax.unary)(*op);
            out.push_str(symbol);
            if binary_op(child).is_some() {
                out.push('(');
                explicit(child, syntax, out);
                out.push(')');
            } else {
                // `Xa` would lex as one identifier in both languages.
                if symbol.ends_with(|c: char| c.is_ascii_alphabetic()) {
                    out.push(' ');
                }
                explicit(child, syntax, out);
            }
        }
        Node::Binary(op, lhs, rhs) => {
            explicit_operand(lhs, syntax, out);
            out.push(' ');
            out.push_str((syntax.binary)(*op));
            out.push(' ');
            explicit_operand(rhs, syntax, out);
        }
    }
}

fn explicit_operand(formula: &Formula, syntax: &Syntax, out: &mut String) {
    if matches!(formula.node(), Node::Unary(..) | Node::Binary(..)) {
        out.push('(');
        explicit(formula, syntax, out);
        out.push(')');
    } else {
        explicit(formula, syntax, out);
    }
}

#[cfg(test)]
mod tests {
    use super::expand_weak_until;
    use crate::ltl::tests::{f, random_formulas};
    use crate::ltl::{Formula, Node, UnaryOp};

    fn identifier_atoms(formula: &Formula) -> Formula {
        formula.rewrite_post_order(|node| match node.node() {
            Node::Atom(name) => match &**name {
                "U" => Some(Formula::atom("u")),
                "x y" => Some(Formula::atom("x_y")),
                "Grant" => Some(Formula::atom("grant")),
                _ => None,
            },
            _ => None,
        })
    }

    #[test]
    fn expand_weak_until_removes_every_w() {
        assert_eq!(expand_weak_until(&f("a W b")), f("(a U b) | G a"));
        assert_eq!(
            expand_weak_until(&f("(a W b) W c")),
            f("(((a U b) | G a) U c) | G ((a U b) | G a)")
        );
    }

    #[test]
    fn spot_drops_redundant_parentheses() {
        let cases = [
            ("a & (b U c)", "a & b U c"),
            ("(a & b) | c", "a & b | c"),
            ("a & (b | c)", "a & (b | c)"),
            ("(a & b) & c", "a & b & c"),
            ("a & (b & c)", "a & (b & c)"),
            ("a U (b U c)", "a U b U c"),
            ("(a U b) U c", "(a U b) U c"),
            ("a U (b W c)", "a U (b W c)"),
            ("a -> (b <-> c)", "a -> (b <-> c)"),
            ("(a -> b) -> c", "(a -> b) -> c"),
            ("G(F(a))", "G F a"),
            ("!(a & b)", "!(a & b)"),
            ("!(!a)", "!!a"),
            ("X(a U b)", "X(a U b)"),
            ("(F a) U b", "F a U b"),
            ("1 W 0", "true W false"),
        ];
        for (input, expected) in cases {
            assert_eq!(f(input).to_string(), expected, "{input}");
        }
    }

    #[test]
    fn spot_quotes_atoms_it_would_misread() {
        for name in ["Grant", "Fail", "X", "U", "true", "a b", "xor", ""] {
            let formula = Formula::unary(UnaryOp::Eventually, Formula::atom(name));
            assert_eq!(formula.to_string(), format!("F \"{name}\""));
            assert_eq!(f(&formula.to_string()), formula);
        }
        assert_eq!(Formula::atom("req_1").to_string(), "req_1");
        assert_eq!(Formula::atom("Uab").to_string(), "Uab");
    }

    #[test]
    fn spot_round_trip() {
        for formula in random_formulas(2000, 1) {
            let text = formula.to_string();
            assert_eq!(f(&text), formula, "{text}");
        }
    }

    #[test]
    fn spin_syntax() {
        assert_eq!(f("G (req -> F grant)").to_spin(), "[](req -> (<>grant))");
        assert_eq!(
            f("!a & X b | c R d").to_spin(),
            "((!a) && (X b)) || (c V d)"
        );
        assert_eq!(f("a W b").to_spin(), "(a U b) || ([]a)");
        assert_eq!(f("true U false").to_spin(), "true U false");
        assert_eq!(f("X G a").to_spin(), "X []a");
    }

    #[test]
    fn nuxmv_syntax() {
        assert_eq!(f("G (req -> F grant)").to_nuxmv(), "G(req -> (F grant))");
        assert_eq!(f("!a & X b | c R d").to_nuxmv(), "((!a) & (X b)) | (c V d)");
        assert_eq!(f("a W b").to_nuxmv(), "(a U b) | (G a)");
        assert_eq!(f("true <-> false").to_nuxmv(), "TRUE <-> FALSE");
        assert_eq!(f("!G F a").to_nuxmv(), "!G F a");
    }

    #[test]
    fn spin_and_nuxmv_round_trip_through_the_parser() {
        for formula in random_formulas(2000, 2) {
            let formula = identifier_atoms(&formula);
            let expected = expand_weak_until(&formula);
            assert_eq!(f(&formula.to_spin()), expected, "{}", formula.to_spin());
            assert_eq!(f(&formula.to_nuxmv()), expected, "{}", formula.to_nuxmv());
        }
    }
}
