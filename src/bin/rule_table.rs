//! `rule_table`: prints every outcome of [`monotone_outcomes`] for each
//! formula read from stdin, one per line, so `bench/check_lean_orders.py`
//! can compare them with the Lean transcription (`outs` in
//! `proofs/Gawain/Rules.lean`) and `bench/operator_orders.py` can read the
//! rules from the code itself.
//!
//! ```text
//! rule_table < FORMULAS
//! ```
//!
//! Each input prints four tab-separated rows, `SEXP DIR SET OUTCOMES`: the
//! formula as an S-expression, `w` or `s`, `peredur` or `ordered`, and the
//! outcomes as S-expressions joined by ` ; `, in the order
//! [`monotone_outcomes`] lists them. The drawn literal is the atom `c`.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use gawain::ltl::{BinaryOp, Formula, Node, UnaryOp};
use gawain::search::{monotone_outcomes, Direction, RuleSet};

fn sexp(formula: &Formula) -> String {
    match formula.node() {
        Node::True => "tt".into(),
        Node::False => "ff".into(),
        Node::Atom(name) => name.to_string(),
        Node::Unary(op, child) => {
            let op = match op {
                UnaryOp::Not => "not",
                UnaryOp::Next => "X",
                UnaryOp::Eventually => "F",
                UnaryOp::Globally => "G",
            };
            format!("({op} {})", sexp(child))
        }
        Node::Binary(op, lhs, rhs) => {
            let op = match op {
                BinaryOp::And => "and",
                BinaryOp::Or => "or",
                BinaryOp::Implies => "imp",
                BinaryOp::Iff => "iff",
                BinaryOp::Until => "U",
                BinaryOp::WeakUntil => "W",
                BinaryOp::Release => "R",
            };
            format!("({op} {} {})", sexp(lhs), sexp(rhs))
        }
    }
}

fn main() -> ExitCode {
    let literal = Formula::atom("c");
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    for (number, line) in io::stdin().lock().lines().enumerate() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                eprintln!("rule_table: {e}");
                return ExitCode::FAILURE;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let formula: Formula = match line.parse() {
            Ok(formula) => formula,
            Err(e) => {
                eprintln!("rule_table: line {}: {e}", number + 1);
                return ExitCode::FAILURE;
            }
        };
        for (dir, direction) in [("w", Direction::Weaken), ("s", Direction::Strengthen)] {
            for (name, set) in [("peredur", RuleSet::Peredur), ("ordered", RuleSet::Ordered)] {
                let outcomes: Vec<String> = monotone_outcomes(&formula, direction, set, &literal)
                    .iter()
                    .map(sexp)
                    .collect();
                let row = format!(
                    "{}\t{dir}\t{name}\t{}",
                    sexp(&formula),
                    outcomes.join(" ; ")
                );
                if writeln!(out, "{row}").is_err() {
                    return ExitCode::FAILURE;
                }
            }
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sexp_names_every_operator() {
        let formula: Formula = "!(a -> X b) | (F a U G b) & (true <-> false) W (a R b)"
            .parse()
            .unwrap();
        assert_eq!(
            sexp(&formula),
            "(or (not (imp a (X b))) (and (U (F a) (G b)) (W (iff tt ff) (R a b))))"
        );
    }
}
